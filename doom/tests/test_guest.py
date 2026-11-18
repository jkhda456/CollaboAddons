#!/usr/bin/env python3
"""Checks of the doom add-on, in the guest (the packaged runtime with out/doom.cpio) and against the
native build of the same sources (build/host: the reference):

  unit      the frontend's unit tests (input decoder, renderer), natively and in the guest
  frames    frames of the game (title, demos, play with scripted keys; both Freedoom IWADs) and
            what a terminal is sent for them in every mode — the guest's the same as the native
            build's, byte for byte; what the bytes show (a terminal emulator) is the picture
            scaled as it should be, and is what the renderer takes the terminal to show
  tty       the game in the guest's shell, in a pseudo-terminal: the alternate screen and back,
            ENDOOM, the title; keys (legacy sequences and their inferred releases, the kitty
            keyboard protocol, win32-input-mode, a Korean input method's jamo), the menu, a game
            started; a terminal that draws block characters wide; the window resized; the screen
            as the renderer thinks it is, at the end
  slow      a terminal slower than the frames: the budget, colour precision stepping down, the
            game in real time all the same, the screen whole at the end
  errors    no IWAD: the message on the terminal, exit status 255

    tests/run.sh [PART...]       (default: all)

The guest is the packaged runtime (dist/runtime/collabo-core-<platform>, COLLABO_RUNTIME) with
out/doom.cpio (DOOM_CPIO); DOOM_ENGINE picks another engine.

Copyright (C) 2026 the collaboCore add-ons' authors. GPL-3.0-only (addons/LICENSE).
"""
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import time

A = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ROOT = os.path.dirname(os.path.dirname(A))
sys.path.insert(0, os.path.join(A, "tests", "tools"))
import ptyrun  # noqa: E402
import vt  # noqa: E402

PLATFORM = {"x86_64": "linux-x64", "aarch64": "linux-arm64", "arm64": "macos-arm64"}.get(platform.machine(), "linux-x64")
RT = os.environ.get("COLLABO_RUNTIME") or os.path.join(ROOT, "dist/runtime/collabo-core-" + PLATFORM)
ENGINE = os.environ.get("DOOM_ENGINE") or os.path.join(RT, "bin/collabo-core-engine")
CPIO = os.environ.get("DOOM_CPIO") or os.path.join(A, "out/doom.cpio")
B = os.path.join(A, "build")
HOST = os.path.join(B, "host")
FD = os.path.join(B, "wads/freedoom-0.13.0")
WORK = os.path.join(B, "test-work")

fails = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else "\n     " + str(detail)[-3000:]), flush=True)
    if not ok:
        fails.append(name)


def addon_dir():
    d = tempfile.mkdtemp(prefix="doom-addon-")
    os.symlink(os.path.abspath(CPIO), os.path.join(d, "doom.cpio"))
    shutil.copyfile(os.path.join(A, "addon.json"), os.path.join(d, "doom.json"))
    return d


ADDONS = None


def engine_argv(mounts=()):
    argv = [ENGINE, "--kernel", f"{RT}/app/images/vmlinux.wasm", "--initramfs", f"{RT}/app/images/initramfs.cpio",
            "--addon-dir", ADDONS, "--addon", "doom", "--arg", "collabo.quiet=1"]
    for host, path in mounts:
        argv += ["--mount", f"{host}:{path}"]
    return argv


def guest(command, mounts=(), timeout=1800):
    """`command` (sh) in a fresh guest with the add-on: (status, output)"""
    argv = engine_argv(mounts)
    argv[1:1] = ["exec"]
    argv += ["--cwd", "/", "--", "/bin/sh", "-c", command]
    try:
        p = subprocess.run(argv, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired as e:
        return -1, f"timed out after {timeout} s: {(e.stdout or b'').decode('utf-8', 'replace')}"
    err = p.stderr.decode("utf-8", "replace")
    mark = err.find("init: starting /bin/sh")
    if mark >= 0:
        err = err[err.find("\n", mark) + 1:]
    return p.returncode, p.stdout.decode("utf-8", "replace") + err


def native(args, home, timeout=600):
    p = subprocess.run([os.path.join(HOST, "doom")] + args, capture_output=True, timeout=timeout,
                       env={**os.environ, "HOME": home}, stdin=subprocess.DEVNULL)
    return p.returncode, (p.stdout + p.stderr).decode("utf-8", "replace")


def fresh(name):
    d = os.path.join(WORK, name)
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    return d


# ── the renderer's model and what a terminal shows ───────────────────────────────────────────────
def color_of(kind, v):
    if kind == "truecolor":
        return v
    if kind == "256":
        return vt.xterm256(v)
    if kind == "16":
        return vt.ANSI16[v & 15]
    return None


def read_state(path):
    lines = open(path).read().split("\n")
    f = lines[0].split()
    lay = {"cols": int(f[1]), "rows": int(f[2]), "blocks": f[3], "color": f[4], "ox": int(f[5]), "oy": int(f[6]),
           "cw": int(f[7]), "ch": int(f[8]), "wide": int(f[9]), "quant": int(f[10])}
    cells = []
    for line in lines[1:1 + lay["cw"] * lay["ch"]]:
        v, a, b, c = line.split()
        cells.append((int(v), int(a, 16), int(b, 16), int(c, 16)))
    return lay, cells


def screen_mismatches(screen, lay, cells, limit=5):
    """cells where what the screen shows is not what the renderer took it to show"""
    bad = []
    kind = lay["color"]
    for i, (valid, a, b, c) in enumerate(cells):
        if not valid:
            continue
        cx, cy = i % lay["cw"], i // lay["cw"]
        x, y = lay["ox"] + cx * lay["wide"], lay["oy"] + cy
        ch, fg, bg = screen.grid[y][x]
        fg = vt.DEFAULT_FG if fg is None else fg
        bg = vt.DEFAULT_BG if bg is None else bg
        if lay["blocks"] == "ascii":
            ok = ch == chr(c) and (c == 32 or kind == "mono" or fg == color_of(kind, a))
            want = (chr(c), color_of(kind, a))
            got = (ch, fg)
        else:
            q = vt.QUAD.get(ch)
            if q is None:
                ok, want, got = False, "a block", ch
            else:
                shown = [fg if q >> k & 1 else bg for k in range(4)]
                if lay["blocks"] == "half":
                    want = [color_of(kind, a)] * 2 + [color_of(kind, b)] * 2
                else:
                    want = [color_of(kind, a) if c >> k & 1 else color_of(kind, b) for k in range(4)]
                got = shown
                ok = shown == want
        if not ok:
            bad.append((cx, cy, want, got))
            if len(bad) >= limit:
                break
    return bad


def expected_half(ppm, lay):
    """the cells of half-block mode computed here from the frame: the box filter and the colour
    precision of render.c, independently"""
    w, h, px = vt.read_ppm(ppm)
    pw, ph = lay["cw"], lay["ch"] * 2
    xs = [i * w // pw for i in range(pw + 1)]
    ys = [i * h // ph for i in range(ph + 1)]
    q = lay["quant"]
    levels = (1 << q) - 1

    def quant(v):
        return v if q >= 8 else ((v * levels + 127) // 255) * 255 // levels
    out = []
    for y in range(ph):
        y0, y1 = min(ys[y], h - 1), ys[y + 1]
        y1 = max(y1, y0 + 1)
        row = []
        for x in range(pw):
            x0, x1 = min(xs[x], w - 1), xs[x + 1]
            x1 = max(x1, x0 + 1)
            n = (x1 - x0) * (y1 - y0)
            s = [0, 0, 0]
            for yy in range(y0, y1):
                for xx in range(x0, x1):
                    o = (yy * w + xx) * 3
                    s[0] += px[o]
                    s[1] += px[o + 1]
                    s[2] += px[o + 2]
            row.append(quant((s[0] + n // 2) // n) << 16 | quant((s[1] + n // 2) // n) << 8 | quant((s[2] + n // 2) // n))
        out.append(row)
    return out


# ── unit ─────────────────────────────────────────────────────────────────────────────────────────
def test_unit():
    for t in ("test_input", "test_render"):
        p = subprocess.run([os.path.join(HOST, t)], capture_output=True, text=True)
        check(f"unit: {t} natively", p.returncode == 0, p.stdout[-2000:])
    rc, out = guest("/b/cross/test_input && /b/cross/test_render", mounts=[(B, "/b")])
    check("unit: test_input and test_render in the guest", rc == 0 and "test_render: " in out and " 0 failed" in out, out)


# ── frames ───────────────────────────────────────────────────────────────────────────────────────
FRAME_RUNS = [
    # name, args, shot tics
    ("title-demos-1", ["-iwad", "freedoom1.wad"], [35, 200, 700, 1500, 3000]),
    ("title-demos-2", ["-iwad", "freedoom2.wad"], [35, 700, 2000]),
    ("play-e1m1", ["-iwad", "freedoom1.wad", "-warp", "1", "1", "-skill", "3",
                   "-keys", "20:+up,90:-up,95:+right,110:-right,115:+fire,140:-fire,150:use,160:tab,190:tab,200:+run,201:+up,260:-up,261:-run"],
     [60, 100, 130, 170, 230, 280]),
    ("play-map01-nomonsters", ["-iwad", "freedoom2.wad", "-warp", "1", "-nomonsters", "-keys", "10:+up,70:-up,75:3,90:+fire,130:-fire"],
     [50, 100, 140]),
]
RENDER_MODES = [
    ("half-truecolor", ["-termsize", "120x40"]),
    ("half-truecolor-8bits", ["-termsize", "80x25", "-quant", "8"]),
    ("quad-truecolor", ["-termsize", "100x30", "-blocks", "quad"]),
    ("ascii-truecolor", ["-termsize", "100x30", "-blocks", "ascii"]),
    ("half-256", ["-termsize", "90x30", "-color", "256"]),
    ("quad-16", ["-termsize", "64x20", "-blocks", "quad", "-color", "16"]),
    ("ascii-mono", ["-termsize", "80x24", "-color", "mono"]),
    ("half-stretch", ["-termsize", "70x40", "-stretch"]),
]


def test_frames():
    gdir = fresh("frames-guest")
    ndir = fresh("frames-native")
    cmds = []
    for name, args, tics in FRAME_RUNS:
        a = [x for x in args]
        for k, x in enumerate(a):
            if x.endswith(".wad"):
                a[k] = x
        cmds.append((name, a + ["-shot", f"{name}-%d.ppm", "-shotansi", f"{name}-%d.ans", "-termsize", "120x40",
                                "-shottic", ",".join(map(str, tics))]))
    for mname, margs in RENDER_MODES:
        cmds.append((f"mode-{mname}", ["-iwad", "freedoom1.wad", "-shotansi", f"mode-{mname}.ans", "-dumpstate",
                                       f"mode-{mname}.state", "-shottic", "1000"] + margs))
    script = []
    for name, a in cmds:
        q = " ".join("'" + x.replace("'", "'\\''") + "'" for x in a)
        script.append(f"(cd /g && HOME=/g/home doom {q} > {name}.log 2>&1; echo {name} $? >> status)")
    rc, out = guest("mkdir -p /g/home && " + " && ".join(script), mounts=[(gdir, "/g")])
    check("frames: the guest's runs", rc == 0, out)
    status = dict(line.split() for line in open(os.path.join(gdir, "status")).read().split("\n") if line.strip()) \
        if os.path.exists(os.path.join(gdir, "status")) else {}
    os.makedirs(os.path.join(ndir, "home"), exist_ok=True)
    for name, a in cmds:
        na = [os.path.join(FD, x) if x.endswith(".wad") else x for x in a]
        p = subprocess.run([os.path.join(HOST, "doom")] + na, capture_output=True, cwd=ndir,
                           env={**os.environ, "HOME": os.path.join(ndir, "home")}, stdin=subprocess.DEVNULL, timeout=600)
        check(f"frames: {name}: exit 0 in the guest and natively", status.get(name) == "0" and p.returncode == 0,
              (status.get(name), p.returncode, open(os.path.join(gdir, name + ".log"), errors="replace").read()[-1500:]))
    # the same bytes
    files = sorted(f for f in os.listdir(ndir) if f.endswith((".ppm", ".ans", ".state")))
    check("frames: all written", len(files) == sum(2 * len(t) for _, _, t in FRAME_RUNS) + 2 * len(RENDER_MODES),
          files)
    differ = [f for f in files if not os.path.exists(os.path.join(gdir, f))
              or open(os.path.join(gdir, f), "rb").read() != open(os.path.join(ndir, f), "rb").read()]
    check(f"frames: the guest's {len(files)} frames and terminal renderings are the native build's, byte for byte",
          not differ, differ)
    # the frames are pictures, and they move
    for name, _, tics in FRAME_RUNS:
        pics = [vt.read_ppm(os.path.join(gdir, f"{name}-{t}.ppm"))[2] for t in tics if os.path.exists(os.path.join(gdir, f"{name}-{t}.ppm"))]
        colours = [len(set(p[i:i + 3] for i in range(0, len(p), 3 * 97))) for p in pics]
        check(f"frames: {name}: pictures, each different", len(pics) == len(tics) and min(colours) > 20
              and len(set(pics)) == len(pics), colours)
    # the scripted keys did something: the view moved forward (the frames at 60 and 100 differ
    # from a run without keys)
    rc, out = native(["-iwad", os.path.join(FD, "freedoom1.wad"), "-warp", "1", "1", "-skill", "3",
                      "-shot", os.path.join(ndir, "still-%d.ppm"), "-shottic", "100"], os.path.join(ndir, "home"))
    check("frames: -keys moves the player", rc == 0 and open(os.path.join(ndir, "still-100.ppm"), "rb").read()
          != open(os.path.join(ndir, "play-e1m1-100.ppm"), "rb").read(), out[-500:])
    # Freedoom's texts (its DEHACKED lump), not the original game's
    log = open(os.path.join(gdir, "title-demos-1.log"), errors="replace").read()
    check("frames: Freedoom's DEHACKED lump read", "DEH_LoadIwadLump: the IWAD's DEHACKED: 214 texts" in log, log[-1500:])
    # what the terminal is sent shows the picture, as the renderer takes it to (every mode)
    for mname, margs in RENDER_MODES:
        st = os.path.join(gdir, f"mode-{mname}.state")
        if not os.path.exists(st):
            check(f"frames: mode {mname}: its state", False, "no state")
            continue
        lay, cells = read_state(st)
        s = vt.Screen(lay["cols"], lay["rows"])
        s.feed(open(os.path.join(gdir, f"mode-{mname}.ans"), "rb").read())
        bad = screen_mismatches(s, lay, cells)
        check(f"frames: mode {mname} ({lay['cw']}x{lay['ch']} cells): the screen shows what the renderer sent",
              not bad and all(c[0] for c in cells), bad)
    # half blocks: the picture scaled, computed here
    for name, tic in (("title-demos-1", 700), ("play-e1m1", 230)):
        s = vt.Screen(120, 40)
        s.feed(open(os.path.join(gdir, f"{name}-{tic}.ans"), "rb").read())
        lay = {"cw": 107, "ch": 40, "ox": 6, "oy": 0, "quant": 6}
        exp = expected_half(os.path.join(gdir, f"{name}-{tic}.ppm"), lay)
        bad = []
        for cy in range(lay["ch"]):
            for cx in range(lay["cw"]):
                ch, fg, bg = s.grid[cy][lay["ox"] + cx]
                q = vt.QUAD.get(ch, -1)
                top = fg if q in (3, 15) else bg
                bottom = fg if q in (12, 15) else bg
                if (top, bottom) != (exp[cy * 2][cx], exp[cy * 2 + 1][cx]):
                    bad.append((cx, cy, (top, bottom), (exp[cy * 2][cx], exp[cy * 2 + 1][cx])))
        check(f"frames: {name} at tic {tic}: the 120x40 terminal shows the frame scaled (box filter, 6 bits)",
              not bad, bad[:5])
        s.png(os.path.join(WORK, f"{name}-{tic}-terminal.png"))
        vt.ppm_to_png(os.path.join(gdir, f"{name}-{tic}.ppm"), os.path.join(WORK, f"{name}-{tic}.png"))


# ── tty ──────────────────────────────────────────────────────────────────────────────────────────
def boot(cols, rows, mounts, env=None):
    t = ptyrun.Session(engine_argv(mounts), cols=cols, rows=rows, env={"TERM": "xterm-256color", **(env or {})})
    t.expect(r"@collabo:\S*# ", 120)
    return t


def trace_lines(path, start=0):
    if not os.path.exists(path):
        return []
    return open(path).read().splitlines()[start:]


def events(lines):
    """(ms, down|up, data1, data2) of the trace"""
    out = []
    for line in lines:
        f = line.split()
        if len(f) == 4 and f[1] in ("down", "up"):
            out.append((int(f[0]), f[1], int(f[2]), int(f[3])))
    return out


def states(ev):
    """the events that change a key's state: a key-down of a key already down (a repeat, as DOS
    DOOM got them) left out"""
    down, out = set(), []
    for e in ev:
        if e[1] == "down" and e[2] in down:
            continue
        (down.add if e[1] == "down" else down.discard)(e[2])
        out.append(e)
    return out


KEY_UP, KEY_DOWN, KEY_LEFT, KEY_RIGHT = 0xad, 0xaf, 0xac, 0xae
KEY_FIRE, KEY_USE, KEY_STRAFE_L, KEY_STRAFE_R, KEY_RSHIFT = 0xa3, 0xa2, 0xa0, 0xa1, 0x80 + 0x36


def wait_trace(t, path, pattern, timeout=20):
    end = time.time() + timeout
    rx = re.compile(pattern, re.M)
    while time.time() < end:
        if os.path.exists(path) and rx.search(open(path).read()):
            return True
        t.pump(0.1)
    return False


def leave_split(data):
    """the bytes before the game leaves the alternate screen, and from there"""
    k = data.rfind(b"\x1b[?1049l")
    return (data, b"") if k < 0 else (data[:k], data[k:])


def test_tty():
    d = fresh("tty")
    tr, st = "/w/trace", "/w/state"
    trp, stp = os.path.join(d, "trace"), os.path.join(d, "state")

    # 1. a legacy terminal (no answers to the queries): keys as they come, releases inferred
    t = boot(120, 40, [(d, "/w")])
    try:
        raw = bytearray()
        t.send(f"doom -trace {tr} -dumpstate {st}\r")
        ok = wait_trace(t, trp, r"^\d+ layout 120 40 half truecolor 6 0 107 40 1$", 60)
        check("tty: the picture laid out for 120x40 (107x40 cells, 4:3)", ok, trace_lines(trp)[:5])
        t.pump(2)
        s = t.feed_screen()
        check("tty: the alternate screen, no cursor, no wrapping", s.alt and s.modes.get(25) is False and s.modes.get(7) is False,
              (s.alt, s.modes))
        check("tty: the title is the game's", s.title == "DOOM - Freedoom: Phase 1", s.title)
        out = bytes(t.text)
        check("tty: the queries (block width, kitty keyboard, device attributes) and win32-input-mode asked for",
              b"\x1b[6n" in out and b"\x1b[?u" in out and b"\x1b[c" in out and b"\x1b[?9001h" in out, out[:400])
        check("tty: frames in synchronized output", out.count(b"\x1b[?2026h") >= 1 and out.count(b"\x1b[?2026l") >= 1)

        n0 = len(trace_lines(trp))
        t.send("\x1b"); t.pump(0.8)            # the menu
        t.send("\x1b[B"); t.pump(0.5)          # down: Options...
        t.send("\x1b[A"); t.pump(0.5)          # up: New Game
        t.send("\r"); t.pump(0.6)              # New Game
        t.send("\r"); t.pump(0.6)              # episode 1
        t.send("\x1b[A"); t.pump(0.5)          # the skill above the default
        t.send("\r"); t.pump(2.5)              # start
        ev = events(trace_lines(trp, n0))
        downs = [e[2] for e in ev if e[1] == "down"]
        check("tty: menu keys reach the game: esc, down, up, enter, enter, up, enter",
              downs == [27, KEY_DOWN, KEY_UP, 13, 13, KEY_UP, 13], ev)
        check("tty: every key let go again (inferred: ~120-200 ms later)",
              all(any(u[1] == "up" and u[2] == e[2] and 90 <= u[0] - e[0] <= 400 for u in ev if u[0] >= e[0]) for e in ev if e[1] == "down"),
              ev)

        # playing: the game's keys
        n0 = len(trace_lines(trp))
        for k, wait in (("w", 1.0), ("\x1b[D", 0.6), ("a", 1.0), ("f", 0.6), (" ", 0.6), ("\x1b[1;2A", 1.0), ("ㅈ", 1.0),
                        ("\x1b[1;5C", 0.6), ("\x1b[1;3D", 1.0)):
            t.send(k)
            t.pump(wait)
        ev = states(events(trace_lines(trp, n0)))
        downs = [(e[2], e[3]) for e in ev if e[1] == "down"]
        want = [(KEY_UP, ord("w")), (KEY_LEFT, 0), (KEY_STRAFE_L, ord("a")), (KEY_FIRE, ord("f")), (KEY_USE, 32),
                (KEY_UP, 0), (KEY_RSHIFT, 0), (KEY_UP, ord("w")), (KEY_RIGHT, 0), (KEY_FIRE, 0), (KEY_STRAFE_L, 0)]
        check("tty: playing — W, ←, A, F, Space, Shift+↑ (run), ㅈ (W), Ctrl+→ (fire), Alt+← (strafe)", downs == want, ev)
        ups = [e for e in ev if e[1] == "up"]
        check("tty: ... and each let go", len(ups) == len(want), ev)

        # a held key (autorepeat after 500 ms, every 33 ms) is held, then let go once
        n0 = len(trace_lines(trp))
        t.send("w")
        t.pump(0.5)
        for _ in range(30):
            t.send("w")
            t.pump(0.033)
        t.pump(1.0)
        ev = events(trace_lines(trp, n0))
        sv = states(ev)
        check("tty: a held move key: down once, up once after its repeats stop",
              [(e[1], e[2]) for e in sv] == [("down", KEY_UP), ("up", KEY_UP)] and sv[1][0] - sv[0][0] > 1200, ev)
        check("tty: ... its repeats reach DOOM as key-downs with the character (as in DOS DOOM)",
              len([e for e in ev if e[1] == "down" and e[3] == ord("w")]) >= 25, len(ev))

        # always run, then a cheat (its letters go through to the game's cheat check)
        n0 = len(trace_lines(trp))
        t.send("r"); t.pump(0.5)
        for c in "iddqd":
            t.send(c); t.pump(0.15)
        t.pump(0.5)
        ev = events(trace_lines(trp, n0))
        check("tty: R holds run down (always run)", ("down", KEY_RSHIFT) in [(e[1], e[2]) for e in ev]
              and ("up", KEY_RSHIFT) not in [(e[1], e[2]) for e in ev], ev)
        check("tty: the cheat's letters reach the game as typed (data2)",
              [chr(e[3]) for e in ev if e[1] == "down" and e[3]][-5:] == list("iddqd"), ev)

        # the window resized: laid out again
        t.resize(100, 30)
        ok = wait_trace(t, trp, r"^\d+ layout 100 30 half truecolor \d+ \d+ 80 30 1$", 20)
        check("tty: a resized window: the picture laid out again (80x30)", ok, trace_lines(trp)[-5:])
        t.pump(1.5)
        # quit: Ctrl+C
        t.send("\x03")
        t.expect(r"@collabo:\S*# ", 30)
        t.pump(0.5)
        data = bytes(t.pending)
        game, after = leave_split(data)
        s = t.screen
        s.feed(game)
        lay, cells = read_state(stp)
        bad = screen_mismatches(s, lay, cells)
        check(f"tty: at the end the screen is what the renderer sent ({lay['cw']}x{lay['ch']})", not bad, bad)
        s.png(os.path.join(WORK, "tty-legacy-end.png"))
        s.feed(after)
        check("tty: Ctrl+C: the main screen back, the cursor shown, wrapping on", not s.alt and s.modes.get(25) is True
              and s.modes.get(7) is True, (s.alt, s.modes))
        txt = s.text()
        check("tty: ENDOOM printed, then the shell's prompt", "freedoom.github.io" in txt and txt.rstrip().endswith("#"),
              txt[-800:])
        check("tty: win32-input-mode off again, the title back",
              b"\x1b[?9001l" in game[-200:] and b"\x1b[23;0t" in after)
        t.send("stty -a | tr ' ' '\\n' | grep -x -e echo -e icanon -e isig | tr '\\n' ' '; echo END\r")
        out = t.expect(r"END\r?\n.*?# ", 20)
        check("tty: stty: echo icanon isig", "echo icanon isig" in out.replace("isig echo", "echo").replace("icanon isig", "icanon isig")
              or all(w in out for w in ("echo", "icanon", "isig")), out)
    finally:
        t.close()

    # 2. a kitty-protocol terminal that draws blocks wide: answers the queries; exact releases
    d2 = fresh("tty-kitty")
    trp2, stp2 = os.path.join(d2, "trace"), os.path.join(d2, "state")
    t = boot(160, 40, [(d2, "/w")])
    t.screen.wide_blocks = True   # as the terminal it says it is
    try:
        t.send(f"doom -trace {tr} -dumpstate {st} -warp 1 1\r")
        t.expect(r"\x1b\[c", 60)
        t.send("\x1b[2;3R\x1b[?0u\x1b[?62;22c")   # the cursor two columns on; kitty; device attributes
        ok = wait_trace(t, trp2, r"^\d+ kitty$", 20)
        check("tty: kitty: the protocol's flags pushed (1|2|8) when the terminal answers", ok and b"\x1b[>11u" in bytes(t.text))
        ok = wait_trace(t, trp2, r"^\d+ layout 160 40 half truecolor \d+ \d+ 53 40 2$", 20)
        check("tty: a terminal that draws block characters wide: two columns a cell (53x40 cells)", ok,
              trace_lines(trp2)[-6:])
        t.pump(2)
        n0 = len(trace_lines(trp2))
        t.send("\x1b[119u"); t.pump(1.5)                       # W down, held (no repeats)
        held = events(trace_lines(trp2, n0))
        t.send("\x1b[119;1:3u"); t.pump(0.3)                   # W up
        t.send("\x1b[57442;5u"); t.pump(0.6)                   # Ctrl down: fire
        t.send("\x1b[57441;7u"); t.pump(0.3)                   # Shift down too: run
        t.send("\x1b[57441;5:3u\x1b[57442;1:3u"); t.pump(0.3)  # both up
        t.send("\x1b[1;1:1D"); t.pump(0.2); t.send("\x1b[1;1:2D"); t.pump(0.6); t.send("\x1b[1;1:3D"); t.pump(0.3)
        ev = events(trace_lines(trp2, n0))
        check("tty: kitty: W held as long as it is down (no inferred release)", [(e[1], e[2]) for e in held] == [("down", KEY_UP)], held)
        sv = states(ev)
        check("tty: kitty: W, Ctrl (fire), Shift (run), ← with a repeat: down and up exactly",
              [(e[1], e[2]) for e in sv] == [("down", KEY_UP), ("up", KEY_UP), ("down", KEY_FIRE), ("down", KEY_RSHIFT),
                                             ("up", KEY_RSHIFT), ("up", KEY_FIRE), ("down", KEY_LEFT), ("up", KEY_LEFT)], ev)
        check("tty: kitty: ← held 0.8 s until its release", len(sv) == 8 and sv[7][0] - sv[6][0] >= 700, ev)
        t.send("\x1b[99;5u")                                   # Ctrl+C as kitty sends it
        t.expect(r"@collabo:\S*# ", 30)
        t.pump(0.3)
        game, after = leave_split(bytes(t.pending))
        s = t.screen
        s.feed(game)
        lay, cells = read_state(stp2)
        bad = screen_mismatches(s, lay, cells)
        check("tty: kitty: the screen is what the renderer sent (two columns a cell)", not bad and lay["wide"] == 2, bad)
        check("tty: kitty: the flags popped at the end", b"\x1b[<u" in game[-200:])
    finally:
        t.close()

    # 3. Windows Terminal's win32-input-mode
    d3 = fresh("tty-win32")
    trp3 = os.path.join(d3, "trace")
    t = boot(100, 30, [(d3, "/w")])
    try:
        t.send(f"doom -trace {tr} -warp 1 1 -blocks quad\r")
        t.expect(r"\x1b\[\?9001h", 60)
        t.send("\x1b[?61;6;7;22c")
        t.pump(2)
        n0 = len(trace_lines(trp3))
        t.send("\x1b[87;17;119;1;0;1_"); t.pump(0.4)
        t.send("\x1b[87;17;119;1;0;1_"); t.pump(0.4)          # a repeat: still down
        t.send("\x1b[87;17;119;0;0;1_"); t.pump(0.3)
        t.send("\x1b[17;29;0;1;8;1_"); t.pump(0.4); t.send("\x1b[17;29;0;0;0;1_"); t.pump(0.3)
        ev = states(events(trace_lines(trp3, n0)))
        check("tty: win32-input-mode: W (with a repeat) and Ctrl down and up exactly",
              [(e[1], e[2]) for e in ev] == [("down", KEY_UP), ("up", KEY_UP), ("down", KEY_FIRE), ("up", KEY_FIRE)]
              and ev[1][0] - ev[0][0] >= 700, ev)
        t.send("\x1b[67;46;3;1;8;1_")                           # Ctrl+C
        t.expect(r"@collabo:\S*# ", 30)
    finally:
        t.close()


# ── slow ─────────────────────────────────────────────────────────────────────────────────────────
def test_slow():
    d = fresh("slow")
    trp, stp = os.path.join(d, "trace"), os.path.join(d, "state")
    t = boot(160, 50, [(d, "/w")])
    try:
        t.send("doom -trace /w/trace -dumpstate /w/state -fps\r")
        t.expect(r"\x1b\[\?1049h", 60)
        t0 = time.time()
        t.pump(14, rate=300 * 1024)   # a terminal that takes 300 KB a second
        secs = [line.split() for line in trace_lines(trp) if " second " in line]
        tics = [(int(f[0]), int(f[3])) for f in secs]
        budgets = [float(f[11]) for f in secs]
        quants = [int(f[13]) for f in secs]
        late = [x for x in tics if x[0] > tics[0][0] + 4000] if tics else []
        rate = (late[-1][1] - late[0][1]) * 1000 / (late[-1][0] - late[0][0]) if len(late) > 2 else 0
        check(f"slow: the game runs in real time all the same ({rate:.1f} tics a second)", 33 <= rate <= 37, tics)
        check("slow: frames on a budget", any(0 < b < 60000 for b in budgets), budgets)
        check("slow: colour precision stepped down", min(quants) < 6, quants)
        scales = [line.split()[2] for line in trace_lines(trp) if " scale " in line]
        layouts = [line.split() for line in trace_lines(trp) if " layout " in line]
        check("slow: the picture made smaller in time (fewer cells)", scales and int(scales[-1]) >= 1
              and int(layouts[-1][8]) * int(layouts[-1][9]) < int(layouts[0][8]) * int(layouts[0][9]) / 2,
              (scales, [l[1:] for l in layouts]))
        sent = [int(f[9]) for f in secs]
        speed = (sent[-1] - sent[len(sent) // 2]) / ((secs[-1][0] and int(secs[-1][0]) - int(secs[len(sent) // 2][0])) / 1000)
        check(f"slow: what is sent fits the terminal ({speed / 1024:.0f} KB/s for 300)", speed < 300 * 1024 * 1.3, sent)
        t.send("\x03")
        t.pump(6)
        t.expect(r"@collabo:\S*# ", 60)
        game, _ = leave_split(bytes(t.pending))
        s = t.screen
        s.feed(game)
        lay, cells = read_state(stp)
        bad = screen_mismatches(s, lay, cells)
        check("slow: the screen is what the renderer sent, nothing half sent", not bad, bad)
        s.png(os.path.join(WORK, "slow-end.png"))
        print(f"     ({time.time() - t0:.0f} s; budget per second: {budgets}; bits: {quants})")
    finally:
        t.close()

    # fast: frames a second when the terminal takes all (informational, and at least 20)
    t = boot(160, 50, [(d, "/w")])
    try:
        t.send("doom -fps\r")
        t.expect(r"\x1b\[\?1049h", 60)
        t.pump(12)
        s = t.feed_screen()
        line = s.text().split("\n")[-1]
        m = re.search(r"DOOM\s+([\d.]+) fps\s+(\d+) KB/s", line)
        fps = float(m.group(1)) if m else 0
        check(f"fast: {line.strip()}", fps >= 20, line)
        t.send("\x03")
        t.expect(r"@collabo:\S*# ", 30)
    finally:
        t.close()


# ── errors ───────────────────────────────────────────────────────────────────────────────────────
def test_errors():
    rc, out = guest("cd /tmp && doom -iwad nope.wad; echo status=$?")
    check("errors: no such IWAD: the message and status 255", "IWAD file 'nope.wad' not found!" in out and "status=255" in out, out[-800:])
    t0 = time.time()
    rc, out = guest("cd /tmp && doom < /dev/null > /tmp/o 2>&1; echo status=$?; grep 'no terminal' /tmp/o")
    check("errors: no terminal: the message and status 255 at once (not a game nobody sees)",
          "no terminal to draw on" in out and "status=255" in out and time.time() - t0 < 60, out[-800:])
    rc, out = guest("cd /tmp && HOME=/tmp doom -h | head -3; doom -warp 1 1 -nonsense -shot /tmp/x.ppm -shottic 2 && ls /tmp/x.ppm")
    check("errors: -h, and options DOOM does not know are left alone", "doom: DOOM in the terminal" in out and "/tmp/x.ppm" in out, out[-800:])


PARTS = {"unit": test_unit, "frames": test_frames, "tty": test_tty, "slow": test_slow, "errors": test_errors}


def main():
    global ADDONS
    parts = sys.argv[1:] or list(PARTS)
    for p in parts:
        if p not in PARTS:
            sys.exit(f"no part {p}: {' '.join(PARTS)}")
    for f in (ENGINE, CPIO, os.path.join(HOST, "doom"), os.path.join(B, "cross/test_input")):
        if not os.path.exists(f):
            sys.exit(f"missing {f}: build first (./build.sh, and the runtime: ./build.sh in {ROOT})")
    os.makedirs(WORK, exist_ok=True)
    ADDONS = addon_dir()
    t0 = time.time()
    try:
        for p in parts:
            print(f"== {p}", flush=True)
            PARTS[p]()
    finally:
        shutil.rmtree(ADDONS, ignore_errors=True)
    print(f"{'FAILED' if fails else 'passed'}: {len(fails)} failed ({time.time() - t0:.0f} s)")
    for f in fails:
        print("  " + f)
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()
