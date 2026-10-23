"""Shared by the imagemagick add-on's checks: the guest (the packaged runtime with
out/imagemagick.cpio) and the reference (the same ImageMagick built for this machine:
`REF=1 build.sh`, build/native).

COLLABO_RUNTIME (default dist/runtime/collabo-core-<platform>) is the runtime; IM_ENGINE and
IM_KERNEL pick another engine binary or kernel, IM_CPIO another image, IM_CPUS the guest's CPUs.
"""
import atexit
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import time

T = os.path.dirname(os.path.abspath(__file__))
A = os.path.dirname(T)
ROOT = os.path.dirname(os.path.dirname(A))
B = os.path.join(A, "build")
PLATFORM = {("Linux", "x86_64"): "linux-x64", ("Linux", "aarch64"): "linux-arm64",
            ("Darwin", "arm64"): "darwin-arm64", ("Darwin", "x86_64"): "darwin-x64"}.get(
                (platform.system(), platform.machine()), "linux-x64")
RT = os.environ.get("COLLABO_RUNTIME") or os.path.join(ROOT, "dist/runtime/collabo-core-" + PLATFORM)
ENGINE = os.environ.get("IM_ENGINE") or os.path.join(RT, "bin/collabo-core-engine")
KERNEL = os.environ.get("IM_KERNEL") or os.path.join(RT, "app/images/vmlinux.wasm")
INITRAMFS = os.path.join(RT, "app/images/initramfs.cpio")
CPIO = os.environ.get("IM_CPIO") or os.path.join(A, "out/imagemagick.cpio")
CPUS = os.environ.get("IM_CPUS")
NATIVE = os.path.join(B, "native")              # the reference: root/usr/bin/magick, config/
MAGICK = os.path.join(NATIVE, "root/usr/bin/magick")
CONFIG = os.path.join(NATIVE, "config")

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


def addon_dir():
    d = tempfile.mkdtemp(prefix="imagemagick-addon-")
    atexit.register(shutil.rmtree, d, True)
    os.symlink(os.path.abspath(CPIO), os.path.join(d, "imagemagick.cpio"))
    shutil.copyfile(os.path.join(A, "addon.json"), os.path.join(d, "imagemagick.json"))
    return d


_ADDONS = None


def guest(command, mounts=(), cwd="/work", timeout=900, env=None, cpus=None, stdin=None):
    """Runs `command` (sh) in a fresh guest with the add-on; mounts are (host, guest) folders.
    Returns (status, output): the command's stdout, then its stderr."""
    global _ADDONS
    if _ADDONS is None:
        _ADDONS = addon_dir()
    argv = [ENGINE, "exec", "--kernel", KERNEL, "--initramfs", INITRAMFS, "--addon-dir", _ADDONS,
            "--addon", "imagemagick", "--cwd", cwd]
    for host, path in mounts:
        argv += ["--mount", f"{host}:{path}"]
    cpus = cpus or CPUS
    if cpus:
        argv += ["--cpus", str(cpus)]
    exports = "".join(f"export {k}='{v}'; " for k, v in (env or {}).items())
    argv += ["--", "/bin/sh", "-c", exports + command]
    try:
        p = subprocess.run(argv, capture_output=True, timeout=timeout, input=stdin)
    except subprocess.TimeoutExpired as e:
        out = e.stdout.decode(errors="replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        return -1, f"timed out after {timeout} s: {out}"
    # stderr is the kernel's boot messages, then the command's
    err = p.stderr.decode(errors="replace")
    mark = err.find("init: starting /bin/sh")
    if mark >= 0:
        err = err[err.find("\n", mark) + 1:]
    return p.returncode, p.stdout.decode(errors="replace") + err


def native(command, cwd, timeout=900, env=None):
    """Runs `command` (sh) with the reference ImageMagick first on PATH: (status, output)."""
    e = dict(os.environ, MAGICK_CONFIGURE_PATH=CONFIG,
             PATH=os.path.dirname(MAGICK) + os.pathsep + os.environ["PATH"], **(env or {}))
    try:
        p = subprocess.run(["/bin/sh", "-c", command], cwd=cwd, env=e, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return -1, f"timed out after {timeout} s"
    return p.returncode, p.stdout.decode(errors="replace") + p.stderr.decode(errors="replace")


def workdir(prefix):
    d = tempfile.mkdtemp(prefix=prefix)
    os.chmod(d, 0o777)
    atexit.register(shutil.rmtree, d, True)
    return d


def present(need_native=True):
    files = [ENGINE, KERNEL, INITRAMFS, CPIO] + ([MAGICK, CONFIG] if need_native else [])
    missing = [f for f in files if not os.path.exists(f)]
    for f in missing:
        hint = (" (run: REF=1 addons/imagemagick/build.sh)" if f in (MAGICK, CONFIG)
                else " (run: addons/imagemagick/build.sh)" if f == CPIO else "")
        print(f"missing {f}{hint}", file=sys.stderr)
    return not missing


class Timer:
    def __enter__(self):
        self.start = time.monotonic()
        return self

    def __exit__(self, *exc):
        self.seconds = time.monotonic() - self.start
