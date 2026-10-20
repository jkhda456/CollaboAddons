#!/usr/bin/env python3
"""The python-extension add-on in the guest: the packages' extension modules load from their
shared libraries in site-packages (dlopen); each tests/checks/<check>.py prints, in the guest,
exactly what it prints with the packages' own wheels from PyPI (tests/expected/<check>.out, from
tests/reference.py); pip sees the packages; their console scripts run (mcp's says what it lacks,
its cli extra, as when pip installs mcp); the modules in threads
(a thread that exists before a module is loaded must load it too); the known differences from the
wheels (no raqm text shaping, no floating-point exception flags from the hardware) are what they
are. Run by tests/run.sh.

The guest is the packaged runtime (COLLABO_RUNTIME, default dist/runtime/collabo-core-<platform>)
with its python.cpio and out/python-extension.cpio (PYEXT_CPIO); PYEXT_ENGINE picks another engine.
"""
import difflib
import shlex
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
ENGINE = os.environ.get("PYEXT_ENGINE") or os.path.join(RT, "bin/collabo-core-engine")
CPIO = os.environ.get("PYEXT_CPIO") or os.path.join(A, "out/python-extension.cpio")
CHECKS = ["numpy_check", "pillow_check", "lief_check", "lxml_check", "cffi_check", "cryptography_check", "rpds_check",
          "mcp_check"]
# the extension modules each package has, as shared libraries in site-packages
MODULES = {
    "numpy": ["numpy._core._multiarray_umath", "numpy.linalg._umath_linalg", "numpy.linalg.lapack_lite",
              "numpy.fft._pocketfft_umath", "numpy.random._common", "numpy.random.bit_generator",
              "numpy.random._bounded_integers", "numpy.random._mt19937", "numpy.random._philox",
              "numpy.random._pcg64", "numpy.random._sfc64", "numpy.random._generator", "numpy.random.mtrand",
              "numpy._core._multiarray_tests", "numpy._core._umath_tests", "numpy._core._rational_tests",
              "numpy._core._struct_ufunc_tests", "numpy._core._operand_flag_tests", "numpy._core._simd"],
    "pillow": ["PIL._imaging", "PIL._imagingft", "PIL._imagingcms", "PIL._webp", "PIL._imagingmath",
               "PIL._imagingmorph", "PIL._imagingtk"],
    "lief": ["lief._lief"],
    "lxml": ["lxml.etree", "lxml.objectify", "lxml._elementpath", "lxml.builder", "lxml.sax", "lxml.html.diff",
             "lxml.html._difflib"],
    "cffi": ["_cffi_backend"],
    "cryptography": ["cryptography.hazmat.bindings._rust"],
    "rpds-py": ["rpds.rpds"],
}
VERSIONS = {"numpy": "2.5.4", "pillow": "12.3.0", "lief": "1.0.0", "lxml": "6.1.3", "pycparser": "3.11", "cffi": "2.1.1",
            "cryptography": "50.0.2", "rpds-py": "2026.9.1", "mcp": "2.3.0", "mcp-types": "2.3.0", "jsonschema": "4.26.0",
            "jsonschema-specifications": "2025.9.1", "referencing": "0.37.0", "attrs": "26.1.0", "opentelemetry-api": "1.45.1",
            "pyjwt": "2.15.1", "python-multipart": "0.0.32", "sse-starlette": "3.5.0", "starlette": "1.9.0", "uvicorn": "0.54.0",
            "click": "8.5.0"}

fails = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else "\n     " + str(detail)[-4000:].replace("\n", "\n     ")), flush=True)
    if not ok:
        fails.append(name)


def addon_dir():
    d = tempfile.mkdtemp(prefix="pyext-addon-")
    os.symlink(os.path.abspath(CPIO), os.path.join(d, "python-extension.cpio"))
    shutil.copyfile(os.path.join(A, "addon.json"), os.path.join(d, "python-extension.json"))
    return d


ADDONS = addon_dir()


def guest(command, timeout=900, mount=None):
    """Runs `command` (sh) in a fresh guest with the add-on, tests/ at /work (and mount = (host
    directory, guest path) too), in /work/samples; returns (status, stdout, stderr after the boot
    messages)."""
    more = ["--mount", f"{mount[0]}:{mount[1]}"] if mount else []
    argv = [ENGINE, "exec", "--kernel", f"{RT}/app/images/vmlinux.wasm", "--initramfs", f"{RT}/app/images/initramfs.cpio",
            "--python-image", f"{RT}/app/images/python.cpio", "--addon-dir", ADDONS, "--addon", "python-extension",
            "--mount", f"{T}:/work", *more, "--cwd", "/work/samples", "--", "/bin/sh", "-c", command]
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
    for f in (ENGINE, CPIO, f"{RT}/app/images/python.cpio"):
        if not os.path.exists(f):
            print(f"missing {f}", file=sys.stderr)
            return 2
    missing = [c for c in CHECKS if not os.path.exists(os.path.join(T, "expected", c + ".out"))]
    if missing:
        print(f"no expected output for {missing}: run python3 {os.path.relpath(T, os.getcwd())}/reference.py", file=sys.stderr)
        return 2

    # every module from its shared library: imported with the ExtensionFileLoader (dlopen), not
    # built into the interpreter; the first import in a machine compiles the libraries (the
    # engine's cache keeps those of a megabyte or more on disk for the next machines)
    names = [n for names in MODULES.values() for n in names]
    probe = ("import importlib, sys\n"
             f"for name in {names!r}:\n"
             "    try:\n"
             "        m = importlib.import_module(name)\n"
             "    except Exception as e:\n"
             "        print('error', name, repr(e)); continue\n"
             "    print(name, type(m.__loader__).__name__, m.__file__.endswith(('.cpython-313.so', '.abi3.so')), name in sys.builtin_module_names)\n")
    start = time.time()
    rc, out, err = guest(f"python3 -c {shlex.quote(probe)}")
    print(f"  (guest boot and the import of {len(names)} extension modules: {time.time() - start:.1f} s)")
    loaded = {line.split()[0]: line.split()[1:] for line in out.splitlines() if len(line.split()) == 4}
    failed = {line.split()[1]: line for line in out.splitlines() if line.startswith("error ")}
    for package, mods in MODULES.items():
        # PIL._imagingtk loads, then wants tkinter, which the guest's Python has not (no display):
        # an ImportError, as with the wheels on a Python without Tk
        bad = [m for m in mods if loaded.get(m) != ["ExtensionFileLoader", "True", "False"]
               and not (m == "PIL._imagingtk" and "tkinter" in failed.get(m, ""))]
        check(f"{package}: its {len(mods)} extension modules load from their shared libraries", not bad,
              f"{bad}\n{out}{err}")

    # each check against the wheels' output
    script = "; ".join(f"echo '### {c}'; python3 ../checks/{c}.py; echo \"### status $?\"" for c in CHECKS)
    rc, out, err = guest(script)
    parts = {}
    for chunk in out.split("### ")[1:]:
        head, _, body = chunk.partition("\n")
        if head.startswith("status"):
            parts[last] = (parts[last][0], head.split()[1])
        else:
            last = head.strip()
            parts[last] = (body, None)
    for c in CHECKS:
        got, status = parts.get(c, ("", None))
        want = open(os.path.join(T, "expected", c + ".out")).read()
        diff = "".join(list(difflib.unified_diff(want.splitlines(True), got.splitlines(True), "wheels", "guest", n=1))[:60])
        check(f"{c}: as with the wheels ({len(want.splitlines())} lines)", status == "0" and got == want,
              (diff or f"status {status}") + ("\n" + err[-2000:] if status != "0" else ""))

    # pip, the console scripts, threads, what differs from the wheels
    rc, out, err = guest("""
        python3 -m pip list --format=freeze 2>/dev/null | grep -iE '^(""" + "|".join(VERSIONS) + """)=='
        python3 -m pip check 2>&1 | tail -1
        python3 -m pip show -f pillow 2>/dev/null | grep -c 'PIL/'
        numpy-config --version; f2py -v
        uvicorn --version; mcp 2>&1; echo "mcp status $?"
        python3 - <<'PY'
import concurrent.futures as cf, threading
# a thread that exists before the modules are loaded, and uses them after
ready, done = threading.Event(), []
def early():
    ready.wait()
    import numpy as np, lief
    from PIL import Image
    from lxml import etree
    from cryptography.hazmat.primitives import hashes
    d = hashes.Hash(hashes.SHA256()); d.update(b"abc")
    # a callback the main thread made (libffi: a function module loaded there), called here
    cmp_ptr = ffi.cast("int(*)(const void *, const void *)", compare)
    a = ffi.new("int[]", [3, 1, 2])
    C.qsort(a, 3, ffi.sizeof("int"), cmp_ptr)
    done.append((int(np.arange(10).sum()), Image.new("L", (4, 4)).size, lief.is_elf("elf-x86_64-static"),
                 etree.fromstring(b"<a><b/></a>")[0].tag, d.finalize().hex()[:8], list(a), cmp_ptr(a, a + 1)))
before = threading.Thread(target=early, daemon=True); before.start()
try:
    import numpy as np, lief
    from PIL import Image, features
    from lxml import etree
    import cffi
    ffi = cffi.FFI()
    ffi.cdef("void qsort(void *, size_t, size_t, int (*)(const void *, const void *));")
    C = ffi.dlopen(None)
    compare = ffi.callback("int(const void *, const void *)",
                           lambda x, y: ffi.cast("int *", x)[0] - ffi.cast("int *", y)[0])
finally:
    ready.set()
before.join(60)
print("early", done)
# one stylesheet for every thread (lxml lets threads share it)
count = etree.XSLT(etree.XML(b'<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">'
                             b'<xsl:template match="/"><n><xsl:value-of select="count(//i)"/></n></xsl:template></xsl:stylesheet>'))
from cryptography.hazmat.primitives.asymmetric import ed25519
def work(i):
    m = np.random.default_rng(i).random((60, 60))
    im = Image.fromarray((m * 255).astype("uint8")).resize((120, 120), Image.LANCZOS)
    b = lief.ELF.parse("elf-x86_64-static")
    doc = etree.fromstring(("<r>" + "<i/>" * (i + 1) * 50 + "</r>").encode())
    k = ed25519.Ed25519PrivateKey.from_private_bytes(bytes([i]) * 32)
    twice = ffi.callback("int(int)", lambda v: 2 * v)
    return (round(float(np.linalg.det(m @ m.T + np.eye(60))), 3) > 0, im.size, len(b.sections),
            int(count(doc).getroot().text) == (i + 1) * 50, k.public_key().verify(k.sign(b"m"), b"m"), twice(i) == 2 * i)
with cf.ThreadPoolExecutor(4) as ex:
    print("threads", sorted(set(ex.map(work, range(8)))))
print("features", features.check("raqm"), features.check("libimagequant"), features.check("freetype2"), features.check("webp"), features.check("avif"))
with np.errstate(all="raise"):
    try:
        np.float64(1.0) / np.float64(0.0)
        print("fpe none")
    except FloatingPointError:
        print("fpe raised")
print("longdouble", np.finfo(np.longdouble).bits, np.dtype(np.intp).itemsize)
PY""")
    listed = {line.split("==")[0].lower().replace("_", "-").replace(".", "-"): line.split("==")[1]
              for line in out.splitlines() if "==" in line}
    check("pip sees the packages", all(listed.get(n) == v for n, v in VERSIONS.items()), out + err)
    check("pip check: nothing broken", "No broken requirements found." in out, out + err)
    check("numpy's console scripts (numpy-config, f2py)", out.splitlines().count(VERSIONS["numpy"]) == 2, out + err)
    check("uvicorn's console script", f"Running uvicorn {VERSIONS['uvicorn']} with CPython 3.13" in out, out + err)
    # mcp's own needs its `cli` extra (typer), which `pip install mcp` leaves out too
    check("mcp's console script without its cli extra, as pip installs it",
          "Error: typer is required. Install with 'pip install mcp[cli]'\nmcp status 1" in out, out + err)
    check("numpy, Pillow, LIEF, lxml, cryptography and cffi callbacks in four threads",
          "threads [(True, (120, 120), 10, True, None, True)]" in out, out + err)
    check("a thread that existed before the modules were loaded uses them, and a callback made by another",
          "early [(45, (4, 4), True, 'b', 'ba7816bf', [1, 2, 3], -1)]" in out, out + err)
    check("Pillow without raqm, libimagequant and AVIF, with FreeType and WebP", "features False False True True False" in out, out + err)
    # wasm has no floating-point exception flags: numpy cannot tell a division by zero happened
    # (as in Pyodide); recorded, not a failure
    print("  (np.errstate(all='raise') on 1.0/0.0: " + ("raises" if "fpe raised" in out else "no FloatingPointError: wasm has no FP exception flags") + ")")
    check("long double is binary128, pointers 32-bit", "longdouble 128 4" in out, out + err)

    print(f"{len(fails)} failed" if fails else "all passed")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
