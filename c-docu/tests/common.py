"""Shared by the c-docu add-on's checks: the guest (the packaged runtime with out/c-docu.cpio),
docu built natively (build/host, `cargo build`), the fixtures, and LibreOffice when there is one.

COLLABO_RUNTIME (default dist/runtime/collabo-core-<platform>) is the runtime; DOCU_ENGINE and
DOCU_KERNEL pick another engine binary or kernel, DOCU_CPIO another image, DOCU_HOST another
native docu, DOCU_SOFFICE LibreOffice's soffice (default: on PATH or ~/.local/bin).
"""
import atexit
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import zipfile

T = os.path.dirname(os.path.abspath(__file__))
A = os.path.dirname(T)
ROOT = os.path.dirname(os.path.dirname(A))
B = os.path.join(A, "build")
PLATFORM = {("Linux", "x86_64"): "linux-x64", ("Linux", "aarch64"): "linux-arm64",
            ("Darwin", "arm64"): "darwin-arm64", ("Darwin", "x86_64"): "darwin-x64"}.get(
                (platform.system(), platform.machine()), "linux-x64")
RT = os.environ.get("COLLABO_RUNTIME") or os.path.join(ROOT, "dist/runtime/collabo-core-" + PLATFORM)
ENGINE = os.environ.get("DOCU_ENGINE") or os.path.join(RT, "bin/collabo-core-engine")
KERNEL = os.environ.get("DOCU_KERNEL") or os.path.join(RT, "app/images/vmlinux.wasm")
INITRAMFS = os.path.join(RT, "app/images/initramfs.cpio")
CPIO = os.environ.get("DOCU_CPIO") or os.path.join(A, "out/c-docu.cpio")
FONTS = os.path.join(B, "fonts")                      # what the image has in /usr/share/docu/fonts
HOST = os.environ.get("DOCU_HOST") or next(
    (p for p in (os.path.join(B, "host/release/docu"), os.path.join(B, "host/debug/docu")) if os.path.exists(p)),
    os.path.join(B, "host/debug/docu"))

fails = []
skips = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else "\n     " + str(detail)[-3000:]), flush=True)
    if not ok:
        fails.append(name)
    return ok


def skip(name, why):
    print(f"SKIP {name}: {why}", flush=True)
    skips.append(name)


def summary():
    print(f"\n{len(fails)} failed" + (": " + ", ".join(fails) if fails else "")
          + (f", {len(skips)} skipped" if skips else ""))
    return 1 if fails else 0


def workdir(prefix):
    d = tempfile.mkdtemp(prefix=prefix)
    os.chmod(d, 0o777)
    atexit.register(shutil.rmtree, d, True)
    return d


def png(path, w, h, rgb):
    """A plain PNG of one color."""
    import struct
    import zlib
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xffffffff)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def emf(path, rgb=(0x20, 0xA0, 0x40)):
    """A small EMF: a rectangle filled with one color, 100 x 100 device pixels (26.46 mm)."""
    import struct
    recs = []
    recs.append(struct.pack("<II", 39, 24) + struct.pack("<IIII", 1, 0, rgb[0] | rgb[1] << 8 | rgb[2] << 16, 0))
    recs.append(struct.pack("<II", 37, 12) + struct.pack("<I", 1))
    recs.append(struct.pack("<II", 43, 24) + struct.pack("<4i", 10, 10, 90, 90))
    recs.append(struct.pack("<II", 14, 20) + struct.pack("<III", 0, 16, 20))
    body = b"".join(recs)
    size = 108 + len(body)
    header = struct.pack("<II4i4iIIIIHHIIIIi", 1, 108, 0, 0, 99, 99, 0, 0, 2646, 2646, 0x464D4520, 0x10000, size, len(recs) + 1, 2, 0, 0, 0, 0, 1024, 768)
    header += struct.pack("<iiIIIii", 271, 203, 0, 0, 0, 271000, 203000)
    with open(path, "wb") as f:
        f.write(header + body)


def png_pixels(path):
    """(width, height, [(r, g, b)] rows) of an 8-bit PNG."""
    import struct
    import zlib
    data = open(path, "rb").read()
    i, idat = 8, b""
    while i < len(data):
        n, kind = struct.unpack(">I4s", data[i:i + 8])
        body = data[i + 8:i + 8 + n]
        if kind == b"IHDR":
            w, h, _, ct = struct.unpack(">IIBB", body[:10])
        elif kind == b"IDAT":
            idat += body
        i += 12 + n
    ch = {6: 4, 2: 3, 0: 1, 4: 2}[ct]
    raw, st = zlib.decompress(idat), w * ch
    rows, prev = [], bytearray(st)
    for y in range(h):
        f = raw[y * (st + 1)]
        line = bytearray(raw[y * (st + 1) + 1:(y + 1) * (st + 1)])
        for x in range(st):
            a = line[x - ch] if x >= ch else 0
            b, c = prev[x], (prev[x - ch] if x >= ch else 0)
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append([tuple(line[x * ch:x * ch + 3]) if ch >= 3 else (line[x * ch],) * 3 for x in range(w)])
        prev = line
    return w, h, rows


def fixtures(d):
    """The test documents in folder d: report.docx, book.xlsx, deck.pptx (made by fixtures.py),
    report.doc, book.xls, deck.ppt (tests/legacy: LibreOffice's saves of those) and pic.png."""
    subprocess.run([sys.executable, "-I", os.path.join(T, "fixtures.py"), d], check=True, capture_output=True)
    for n in os.listdir(os.path.join(T, "legacy")):
        shutil.copyfile(os.path.join(T, "legacy", n), os.path.join(d, n))
    png(os.path.join(d, "pic.png"), 40, 20, (40, 90, 200))


def host(args, cwd, timeout=300):
    """Runs the native docu: (status, stdout + stderr)."""
    env = dict(os.environ, DOCU_FONT_PATH=FONTS, DOCU_SYSTEM_FONTS="0")
    p = subprocess.run([HOST] + args, cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout + p.stderr


def addon_dir():
    d = tempfile.mkdtemp(prefix="docu-addon-")
    atexit.register(shutil.rmtree, d, True)
    os.symlink(os.path.abspath(CPIO), os.path.join(d, "c-docu.cpio"))
    shutil.copyfile(os.path.join(A, "addon.json"), os.path.join(d, "c-docu.json"))
    return d


_ADDONS = None


def guest(work, command, timeout=900):
    """Runs `command` (sh) in a fresh guest with the add-on, in /work (the host folder `work`);
    returns (status, output): the command's stdout, then its stderr."""
    global _ADDONS
    if _ADDONS is None:
        _ADDONS = addon_dir()
    argv = [ENGINE, "exec", "--kernel", KERNEL, "--initramfs", INITRAMFS, "--addon-dir", _ADDONS,
            "--addon", "c-docu", "--mount", f"{work}:/work", "--cwd", "/work",
            "--", "/bin/sh", "-c", command]
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout, errors="replace")
    except subprocess.TimeoutExpired as e:
        out = e.stdout.decode(errors="replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        return -1, f"timed out after {timeout} s: {out}"
    err = p.stderr
    mark = err.find("init: starting /bin/sh")
    if mark >= 0:
        err = err[err.find("\n", mark) + 1:]
    return p.returncode, p.stdout + err


def present():
    missing = [f for f in (ENGINE, KERNEL, INITRAMFS, CPIO) if not os.path.exists(f)]
    for f in missing:
        print(f"missing {f}" + (" (run addons/c-docu/build.sh)" if f == CPIO else ""), file=sys.stderr)
    return not missing


def soffice():
    """LibreOffice's soffice, or None."""
    for p in (os.environ.get("DOCU_SOFFICE"), shutil.which("soffice"), shutil.which("libreoffice"),
              os.path.expanduser("~/.local/bin/soffice")):
        if p and os.path.exists(p):
            return p
    return None


def lo_convert(path, to, outdir):
    """LibreOffice's conversion of a file (to: txt, csv, pdf, ...): the output's path or None."""
    so = soffice()
    if not so:
        return None
    profile = tempfile.mkdtemp(prefix="docu-lo-")
    atexit.register(shutil.rmtree, profile, True)
    subprocess.run([so, f"-env:UserInstallation=file://{profile}", "--headless", "--convert-to", to,
                    "--outdir", outdir, path], capture_output=True, timeout=180)
    base = os.path.splitext(os.path.basename(path))[0]
    ext = to.split(":")[0]
    out = os.path.join(outdir, f"{base}.{ext}")
    return out if os.path.exists(out) else None


def zip_entries(path):
    """{name: (compressed bytes as stored, crc)} of a zip package."""
    out = {}
    with open(path, "rb") as f:
        data = f.read()
    with zipfile.ZipFile(path) as z:
        for info in z.infolist():
            start = info.header_offset + 30 + len(info.filename.encode()) + len(info.extra)
            # The local header's own extra field may differ from the central one's.
            import struct
            n, m = struct.unpack("<HH", data[info.header_offset + 26:info.header_offset + 30])
            start = info.header_offset + 30 + n + m
            out[info.filename] = (data[start:start + info.compress_size], info.CRC)
    return out
