"""Shared by the typst add-on's checks: the guest (the packaged runtime with out/typst.cpio) and
the reference (the official typst 0.15.1 binary, fetched as sources.lock names it).

COLLABO_RUNTIME (default dist/runtime/collabo-core-<platform>) is the runtime; TYPST_ENGINE and
TYPST_KERNEL pick another engine binary or kernel, TYPST_CPIO another image, TYPST_REF another
reference binary.
"""
import atexit
import hashlib
import os
import struct
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
import zlib

T = os.path.dirname(os.path.abspath(__file__))
A = os.path.dirname(T)
ROOT = os.path.dirname(os.path.dirname(A))
B = os.path.join(A, "build")
PLATFORM = {("Linux", "x86_64"): "linux-x64", ("Linux", "aarch64"): "linux-arm64",
            ("Darwin", "arm64"): "darwin-arm64", ("Darwin", "x86_64"): "darwin-x64"}.get(
                (platform.system(), platform.machine()), "linux-x64")
RT = os.environ.get("COLLABO_RUNTIME") or os.path.join(ROOT, "dist/runtime/collabo-core-" + PLATFORM)
ENGINE = os.environ.get("TYPST_ENGINE") or os.path.join(RT, "bin/collabo-core-engine")
KERNEL = os.environ.get("TYPST_KERNEL") or os.path.join(RT, "app/images/vmlinux.wasm")
INITRAMFS = os.path.join(RT, "app/images/initramfs.cpio")
CPIO = os.environ.get("TYPST_CPIO") or os.path.join(A, "out/typst.cpio")
FONTS = os.path.join(B, "fonts/nanum")              # what the image has in /usr/share/fonts/nanum
PACKAGES = os.path.join(A, "share/typst/packages")  # … in /usr/share/typst/packages
UNIVERSE = os.path.join(B, "packages")              # … in /usr/share/typst/packages/preview
EPOCH = "1760000000"                                # SOURCE_DATE_EPOCH on both sides

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


def lock(name):
    """(url, sha256) of a sources.lock entry."""
    with open(os.path.join(A, "sources.lock")) as f:
        for line in f:
            parts = line.split()
            if len(parts) == 3 and parts[0] == name:
                return parts[1], parts[2]
    sys.exit(f"{name} is not in sources.lock")


def reference():
    """The official typst binary for this machine, downloaded (once) and checked."""
    if os.environ.get("TYPST_REF"):
        return os.environ["TYPST_REF"]
    arch = {"aarch64": "aarch64", "arm64": "aarch64", "x86_64": "x86_64", "AMD64": "x86_64"}[platform.machine()]
    url, sha = lock(f"typst-ref-{arch}")
    archive = os.path.join(B, "dl", os.path.basename(url))
    binary = os.path.join(B, "ref", os.path.basename(url).removesuffix(".tar.xz"), "typst")
    if os.path.exists(binary):
        return binary
    os.makedirs(os.path.dirname(archive), exist_ok=True)
    if not os.path.exists(archive):
        print(f"fetching {url}", flush=True)
        urllib.request.urlretrieve(url, archive + ".part")
        os.rename(archive + ".part", archive)
    with open(archive, "rb") as f:
        if hashlib.sha256(f.read()).hexdigest() != sha:
            os.remove(archive)
            sys.exit(f"checksum mismatch: {archive} (deleted)")
    with tarfile.open(archive) as tar:
        tar.extractall(os.path.join(B, "ref"), filter="data")
    return binary


def ref(args, cwd, env=None, stdin=None, timeout=300):
    """Runs the reference typst with the guest's fonts and packages: (status, stdout, stderr)."""
    e = dict(os.environ, SOURCE_DATE_EPOCH=EPOCH, TYPST_PACKAGE_PATH=PACKAGES, NO_COLOR="1",
             TYPST_PACKAGE_CACHE_PATH=UNIVERSE,
             TYPST_IGNORE_SYSTEM_FONTS="true", TYPST_FONT_PATHS=FONTS, **(env or {}))
    p = subprocess.run([reference()] + args, cwd=cwd, env=e, capture_output=True, text=True,
                       input=stdin, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def addon_dir():
    d = tempfile.mkdtemp(prefix="typst-addon-")
    atexit.register(shutil.rmtree, d, True)
    os.symlink(os.path.abspath(CPIO), os.path.join(d, "typst.cpio"))
    shutil.copyfile(os.path.join(A, "addon.json"), os.path.join(d, "typst.json"))
    return d


_ADDONS = None


def guest(work, command, timeout=600, engine_args=(), cpus=None):
    """Runs `command` (sh) in a fresh guest with the add-on, in /work (the host folder `work`);
    returns (status, output): the command's stdout, then its stderr."""
    global _ADDONS
    if _ADDONS is None:
        _ADDONS = addon_dir()
    argv = [ENGINE, "exec", "--kernel", KERNEL, "--initramfs", INITRAMFS, "--addon-dir", _ADDONS,
            "--addon", "typst", "--mount", f"{work}:/work", "--cwd", "/work", *engine_args]
    if cpus:
        argv += ["--cpus", str(cpus)]
    argv += ["--", "/bin/sh", "-c", f"export SOURCE_DATE_EPOCH={EPOCH} NO_COLOR=1; {command}"]
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout, errors="replace")
    except subprocess.TimeoutExpired as e:
        out = e.stdout.decode(errors="replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        return -1, f"timed out after {timeout} s: {out}"
    # stderr is the kernel's boot messages, then the command's
    err = p.stderr
    mark = err.find("init: starting /bin/sh")
    if mark >= 0:
        err = err[err.find("\n", mark) + 1:]
    return p.returncode, p.stdout + err


def workdir(prefix):
    d = tempfile.mkdtemp(prefix=prefix)
    os.chmod(d, 0o777)
    return d


def present():
    missing = [f for f in (ENGINE, KERNEL, INITRAMFS, CPIO, FONTS) if not os.path.exists(f)]
    for f in missing:
        print(f"missing {f}" + (" (run addons/typst/build.sh)" if f in (CPIO, FONTS) else ""), file=sys.stderr)
    return not missing


def same_files(a, b, names):
    """The names whose files differ between folders a and b (or are missing)."""
    bad = []
    for n in names:
        pa, pb = os.path.join(a, n), os.path.join(b, n)
        if not (os.path.exists(pa) and os.path.exists(pb)):
            bad.append(f"{n} (missing {'reference' if not os.path.exists(pa) else 'guest'})")
            continue
        with open(pa, "rb") as fa, open(pb, "rb") as fb:
            if fa.read() != fb.read():
                bad.append(n)
    return bad


class Timer:
    def __enter__(self):
        self.start = time.monotonic()
        return self

    def __exit__(self, *exc):
        self.seconds = time.monotonic() - self.start


def png_pixels(path):
    """(width, height, channels, pixel bytes) of an 8-bit, non-interlaced PNG (what typst writes)."""
    data = open(path, "rb").read()
    i, idat = 8, b""
    while i < len(data):
        n, kind = struct.unpack(">I4s", data[i:i + 8])
        body = data[i + 8:i + 8 + n]
        if kind == b"IHDR":
            width, height, depth, ctype = struct.unpack(">IIBB", body[:10])
            channels = {6: 4, 2: 3, 0: 1, 4: 2}[ctype]
        elif kind == b"IDAT":
            idat += body
        i += 12 + n
    raw, stride = zlib.decompress(idat), width * channels
    rows, prev = [], bytearray(stride)
    for y in range(height):
        f = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for x in range(stride):
            a = line[x - channels] if x >= channels else 0
            b, c = prev[x], (prev[x - channels] if x >= channels else 0)
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
        rows.append(bytes(line))
        prev = line
    return width, height, channels, b"".join(rows)


def png_difference(a, b):
    """(pixels that differ, all pixels, the largest channel difference) of two PNGs."""
    wa, ha, ca, pa = png_pixels(a)
    wb, hb, cb, pb = png_pixels(b)
    if (wa, ha, ca) != (wb, hb, cb):
        return wa * ha, wa * ha, 255
    pixels = sum(1 for i in range(0, len(pa), ca) if pa[i:i + ca] != pb[i:i + ca])
    return pixels, wa * ha, max((abs(x - y) for x, y in zip(pa, pb)), default=0)
