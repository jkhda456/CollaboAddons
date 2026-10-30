#!/usr/bin/env python3
"""The llvm add-on in the guest: guest.sh runs every step in one machine (tests/ at /work), this
script checks what each printed; pyext.sh builds a C extension module the way setuptools does
and imports it (with the Python image and the interpreter's headers). Run by tests/run.sh.

The guest is the packaged runtime (COLLABO_RUNTIME, default dist/runtime/collabo-core-<platform>)
with out/llvm.cpio (LLVM_CPIO); LLVM_ENGINE picks another engine.
"""
import math
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
PLATFORM = {("Linux", "x86_64"): "linux-x64", ("Linux", "aarch64"): "linux-arm64",
            ("Darwin", "arm64"): "darwin-arm64", ("Darwin", "x86_64"): "darwin-x64"}.get(
                (platform.system(), platform.machine()), "linux-x64")
RT = os.environ.get("COLLABO_RUNTIME") or os.path.join(ROOT, "dist/runtime/collabo-core-" + PLATFORM)
ENGINE = os.environ.get("LLVM_ENGINE") or os.path.join(RT, "bin/collabo-core-engine")
CPIO = os.environ.get("LLVM_CPIO") or os.path.join(A, "out/llvm.cpio")
# the guest interpreter's headers (python/build.sh's stage), for the extension module check
PY_INCLUDE = os.environ.get("PY_INCLUDE") or os.path.join(ROOT, "python/stage/usr/include/python3.13")

big = 0xffffffffffffffff * 0x10001
# what each step prints, line for line (None: checked by its own test below)
EXPECTED = {
    "hello": ["hello, world"],
    "cfeatures": [
        "longjmp: 42",
        "thread: 207, main's tls still 1",
        "math: %.6f %.6f" % (math.sqrt(2.0) * math.cos(0.5), math.pow(2.0, 10.5)),
        "long double: 0.33333333333333333333",
        "__int128: %x%016x" % (big >> 64, big & 0xffffffffffffffff),
        "malloc 64 MiB: 1",
        "qsort: 1 3 5 9",
        "file: written 123",
    ],
    "cxx_hello": ["hello from c++"],
    "cxx_features": [
        "static constructor",
        "unwound frame 0",
        "unwound frame 1",
        "caught my error code 7",
        "rethrowing",
        "out_of_range",
        "exception_ptr: from a thread",
        "dynamic_cast: 5",
        "typeid: 1",
        "threads: 10 4",
        "a=3 b=2 c=1",
        "variant 3",
        "filesystem: 5 1",
        "format: 42     x 3.142",
    ],
    "shared": [
        "lib_add: 45",                       # 1 + 2 + 42 (40, and its constructor's 2)
        "lib_setjmp: 9",                     # longjmp in the library, with the program's C library
        "lib_calls: 1",
        "lib_fn: 42",
        "hello from lib.so, main",
        "missing: not found",
        "libxx: made by a constructor 20",   # libc++ in the library, its constructors run
        "libxx: made by a constructor caught too big: 5",   # thrown and caught in the library
    ],
    "ld_options": ["lib_add: 45", "lib_setjmp: 9"],
}

fails = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else "\n     " + str(detail)[-4000:].replace("\n", "\n     ")), flush=True)
    if not ok:
        fails.append(name)


def addon_dir():
    d = tempfile.mkdtemp(prefix="llvm-addon-")
    os.symlink(os.path.abspath(CPIO), os.path.join(d, "llvm.cpio"))
    shutil.copyfile(os.path.join(A, "addon.json"), os.path.join(d, "llvm.json"))
    return d


def guest(command, timeout=3600, python=None, mounts=()):
    """Runs `command` (sh) in a fresh guest with the add-on and tests/ at /work (with the Python
    image `python`, and mounts: (host, guest, read-only)); returns (status, stdout, stderr after the
    boot messages)."""
    more = ["--python-image", python] if python else []
    for host, path, ro in mounts:
        more += ["--mount", f"{host}:{path}" + (":ro" if ro else "")]
    argv = [ENGINE, "exec", "--kernel", f"{RT}/app/images/vmlinux.wasm", "--initramfs", f"{RT}/app/images/initramfs.cpio",
            *more, "--addon-dir", ADDONS, "--addon", "llvm", "--no-network", "--mount", f"{T}:/work", "--cwd", "/work",
            "--", "/bin/sh", "-c", command]
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as e:
        return -1, f"timed out after {timeout} s: {e.stdout or ''}", ""
    err = p.stderr
    mark = err.find("init: starting /bin/sh")
    if mark >= 0:
        err = err[err.find("\n", mark) + 1:]
    return p.returncode, p.stdout, err


def main():
    for f in (ENGINE, CPIO):
        if not os.path.exists(f):
            print(f"missing {f}", file=sys.stderr)
            return 2
    global ADDONS
    ADDONS = addon_dir()

    start = time.time()
    rc, out, err = guest("/work/guest.sh")
    print(f"  (guest: {time.time() - start:.1f} s)")
    steps = {}
    for chunk in out.replace("\r", "").split("### ")[1:]:
        head, _, body = chunk.partition("\n")
        if head.startswith("status "):
            steps[last] = (int(head.split()[1]), steps[last][1])
        else:
            last = head.strip()
            steps[last] = (None, body.rstrip("\n").split("\n") if body.strip() else [])
    if not steps:
        check("the guest ran guest.sh", False, f"status {rc}\n{out}\n{err}")
        return 1

    def step(name):
        status, lines = steps.get(name, (None, ["(did not run)"]))
        return status, [line.rstrip() for line in lines]

    status, lines = step("tools")
    check("tools: every name is there, clang/LLVM/LLD 24",
          status == 0 and not any(l.startswith("missing") for l in lines)
          and any(l.startswith("clang version 24.") for l in lines)
          and any(l.startswith("LLVM version 24.") for l in lines)
          and any(l.startswith("LLD 24.") for l in lines), "\n".join(lines))

    for name, want in EXPECTED.items():
        status, lines = step(name)
        check(f"{name}: prints what it should", status == 0 and lines == want,
              f"status {status}\n" + "\n".join(lines) + "\n--- wanted:\n" + "\n".join(want))

    status, lines = step("archive")
    text = "\n".join(lines)
    check("archive: ar, ranlib, nm, a program linked with it, strip, objdump, readobj, cc -r",
          status == 0 and "T a_value" in text and "T b_value" in text and lines.count("archive: 42") == 2
          and "Format: WASM" in text and text.count("T a_value") == 2, text)

    status, lines = step("probes")
    want = ["wasm32-unknown-linux-musl", "compiler-rt: found", "#define __PIC__ 2", "#define __linux__ 1",
            "#define __wasm32__ 1", "#define __wasm_exception_handling__ 1", "6*7", "stdin: built and ran", "LLD 24"]
    check("probes: -dumpmachine, -print-libgcc-file-name, the predefined macros, cpp, cc -x c -, "
          "cc -Wl,--version, c++ -v",
          status == 0 and lines[:len(want)] == want and len(lines) > len(want)
          and lines[len(want)].startswith("clang version 24."), "\n".join(lines))

    # a Python extension module, built as setuptools builds one (the interpreter's build variables:
    # CC is cc, LDSHARED cc -shared), against the interpreter's headers (which its image leaves out)
    py_image = f"{RT}/app/images/python.cpio"
    if os.path.exists(py_image) and os.path.isdir(PY_INCLUDE):
        rc, out, err2 = guest("/work/pyext.sh", python=py_image,
                              mounts=[(PY_INCLUDE, "/usr/include/python3.13", True)])
        want = ["add: 42", "error: failed with [1, 'x']", "jump: jumped 3", "loader: ExtensionFileLoader True"]
        lines = [l.rstrip() for l in out.replace("\r", "").splitlines()]
        check("pyext: a C extension module built with Python's CC/LDSHARED, imported", rc == 0 and lines == want,
              f"status {rc}\n{out}{err2[-2000:]}")
    else:
        print(f"skip pyext: needs {py_image} and the interpreter's headers ({PY_INCLUDE})")

    print(f"{'FAILED: ' + ', '.join(fails) if fails else 'all passed'}")
    if fails:
        print("     guest stderr:\n     " + err[-3000:].replace("\n", "\n     "))
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
