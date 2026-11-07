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
          "mcp_check", "unicorn_check"]
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
    "ctypes": ["_ctypes"],         # the interpreter's own, in lib-dynload (build.sh, stage ctypes)
}
VERSIONS = {"numpy": "2.5.4", "pillow": "12.3.0", "lief": "1.0.0", "lxml": "6.1.3", "pycparser": "3.11", "cffi": "2.1.1",
            "cryptography": "50.0.2", "rpds-py": "2026.9.1", "mcp": "2.3.0", "mcp-types": "2.3.0", "jsonschema": "4.26.0",
            "jsonschema-specifications": "2025.9.1", "referencing": "0.37.0", "attrs": "26.1.0", "opentelemetry-api": "1.45.1",
            "pyjwt": "2.15.1", "python-multipart": "0.0.32", "sse-starlette": "3.5.0", "starlette": "1.9.0", "uvicorn": "0.54.0",
            "click": "8.5.0", "unicorn": "2.1.4"}
# unicorn's versions the add-on carries as wheels (packages/unicorn: one per minor line), each checked
# as installed by pip against its own PyPI wheel's output (expected/unicorn_check-<version>.out;
# the installed one's is expected/unicorn_check.out)
UNICORN_WHEELS = ["1.0.3", "2.0.1.post1", "2.1.4"]

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

    # the interpreter's headers and python3.pc, where sysconfig says a build finds them
    rc, out, err = guest("""
        inc=$(python3 -c 'import sysconfig; print(sysconfig.get_paths()["include"])'); echo "include $inc"
        ls "$inc"/Python.h "$inc"/pyconfig.h "$inc"/cpython/object.h >/dev/null && echo headers-ok
        grep -E '^(Version|Cflags):' /usr/lib/pkgconfig/python3.pc; grep -c 'Libs.private\\|/opt/' /usr/lib/pkgconfig/python3.pc
        ls /usr/lib/pkgconfig/python3-embed.pc 2>&1 | grep -c 'No such file'
        python3 -c 'import sysconfig as s; print("compilers", s.get_config_var("CC"), s.get_config_var("CXX"), s.get_config_var("LDSHARED").split()[:2], s.get_config_var("LDCXXSHARED").split()[:2])'
    """)
    check("the interpreter's headers in /usr/include/python3.13 (sysconfig), python3.pc without build paths, no -embed",
          "include /usr/include/python3.13" in out and "headers-ok" in out and "Version: 3.13" in out
          and "Cflags: -I${includedir}/python3.13" in out and "\n0\n1\ncompilers" in out, out + err)
    check("sysconfig's compilers are the guest's cc and c++ (CXX, LDCXXSHARED: python/pack-stdlib.sh)",
          "compilers cc c++ ['cc', '-shared'] ['c++', '-shared']" in out, out + err)

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

    # ctypes (the add-on's _ctypes in lib-dynload): calls, a variadic one (its fixed arguments
    # declared: WebAssembly passes the rest in memory), callbacks, structures and unions by value,
    # pointers, errno, the interpreter's own API, and its limits here (pointers 32-bit; at most 1000
    # arguments, as on Emscripten)
    rc, out, err = guest(r"""python3 - <<'PY'
import ctypes, _ctypes
from ctypes import *
libc = CDLL(None)
print("ctypes", _ctypes.__file__, sizeof(c_long), sizeof(c_void_p), sizeof(c_longdouble), _ctypes.CTYPES_MAX_ARGCOUNT)
print("strlen", libc.strlen(b"hello"))
libc.snprintf.argtypes = [c_char_p, c_size_t, c_char_p]
buf = create_string_buffer(64)
print("snprintf", libc.snprintf(buf, 64, b"%d %s %.3f %lld", 42, b"x", c_double(3.14159), c_longlong(1 << 40)), buf.value)
libc.qsort.restype = None
CMP = CFUNCTYPE(c_int, POINTER(c_int), POINTER(c_int))
arr = (c_int * 6)(5, 1, 4, 2, 3, 0)
libc.qsort(arr, 6, sizeof(c_int), CMP(lambda a, b: a[0] - b[0]))
print("qsort", list(arr))
class Div(Structure): _fields_ = [("quot", c_long), ("rem", c_long)]
libc.ldiv.restype = Div; libc.ldiv.argtypes = [c_long, c_long]
d = libc.ldiv(17, 5); print("ldiv", d.quot, d.rem)
class U(Union): _fields_ = [("i", c_uint32), ("b", c_ubyte * 4)]
print("union", list(U(0x01020304).b))
p = pointer(c_int(7)); p.contents.value += 1; print("pointer", p[0], cast(p, c_void_p).value != 0)
libc.strtol.argtypes = [c_char_p, POINTER(c_char_p), c_int]; end = c_char_p()
print("strtol", libc.strtol(b"0x1fz", byref(end), 16), end.value)
lc = CDLL(None, use_errno=True); lc.close.argtypes = [c_int]
print("errno", lc.close(-1), get_errno())
f = pythonapi.PyLong_FromLong; f.restype = py_object; f.argtypes = [c_long]; print("pythonapi", f(1234))
print("callback", CFUNCTYPE(c_double, c_double, c_int)(lambda x, n: x ** n)(2.0, 10))
PY""")
    for line in ("ctypes /usr/lib/python3.13/lib-dynload/_ctypes.cpython-313.so 4 4 16 1000", "strlen 5",
                 "snprintf 24 b'42 x 3.142 1099511627776'", "qsort [0, 1, 2, 3, 4, 5]", "ldiv 3 2", "union [4, 3, 2, 1]",
                 "pointer 8 True", "strtol 31 b'z'", "errno -1 9", "pythonapi 1234", "callback 1024.0"):
        check(f"ctypes: {line}", line in out, out + err)

    # the wheels pip finds (/etc/pip.conf): cffi's versions and pycparser, without the network — each
    # requirement gets its highest matching version, which calls, varargs and callbacks work with;
    # and a build's isolated environment takes them (zstandard's `cffi~=1.17`)
    rc, out, err = guest(r"""
        python3 -m pip config list 2>/dev/null; ls /usr/share/python-wheels
        for req in 'cffi~=1.17' 'cffi>=2.0,<2.1' 'cffi>=1.12'; do
          rm -rf /tmp/w
          python3 -m pip install -q --no-index --target /tmp/w "$req" > /tmp/pip.log 2>&1 || { echo "pip failed: $req"; tail -3 /tmp/pip.log; }
          PYTHONPATH=/tmp/w python3 - "$req" <<'PY'
import sys, cffi, _cffi_backend, importlib.metadata as md
ffi = cffi.FFI()
ffi.cdef("size_t strlen(const char *); int snprintf(char *, size_t, const char *, ...);"
         "void qsort(void *, size_t, size_t, int (*)(const void *, const void *));")
C = ffi.dlopen(None)
buf = ffi.new("char[32]")
C.snprintf(buf, 32, b"%d-%s", ffi.cast("int", 42), ffi.new("char[]", b"x"))
@ffi.callback("int(const void *, const void *)")
def cmp(a, b):
    return ffi.cast("int *", a)[0] - ffi.cast("int *", b)[0]
arr = ffi.new("int[]", [5, 1, 4])
C.qsort(arr, 3, ffi.sizeof("int"), cmp)
eps = [e.name for e in md.distribution("cffi").entry_points if e.group == "distutils.setup_keywords"]
print("wheel", sys.argv[1], cffi.__version__, _cffi_backend.__file__.startswith("/tmp/w/"), C.strlen(b"hello"),
      ffi.string(buf).decode(), list(arr), eps, md.version("pycparser"))
PY
        done
        python3 - <<'PY'
import os
backend = '''import os, zipfile
def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    import cffi, _cffi_backend
    name = os.path.basename(os.getcwd())
    with open("/tmp/report.txt", "a") as f:
        f.write(f"isolated {name} cffi {cffi.__version__} {_cffi_backend.__version__} {cffi.__file__.startswith('/tmp/pip-build-env-')}\\n")
    whl = f"{name}-1.0-py3-none-any.whl"
    info = f"{name}-1.0.dist-info"
    with zipfile.ZipFile(os.path.join(wheel_directory, whl), "w") as z:
        z.writestr(f"{info}/METADATA", f"Metadata-Version: 2.1\\nName: {name}\\nVersion: 1.0\\n")
        z.writestr(f"{info}/WHEEL", "Wheel-Version: 1.0\\nGenerator: test\\nRoot-Is-Purelib: true\\nTag: py3-none-any\\n")
        z.writestr(f"{info}/RECORD", "")
    return whl
'''
for name, req in (("needs_cffi1", "cffi~=1.17"), ("needs_cffi", "cffi>=1.12")):
    os.makedirs(f"/tmp/{name}")
    open(f"/tmp/{name}/backend.py", "w").write(backend)
    open(f"/tmp/{name}/pyproject.toml", "w").write(
        f'[build-system]\nrequires = ["{req}"]\nbuild-backend = "backend"\nbackend-path = ["."]\n')
PY
        for p in needs_cffi1 needs_cffi; do
          python3 -m pip wheel -q --no-index -w /tmp/out /tmp/$p > /tmp/pip.log 2>&1 || { echo "pip wheel failed: $p"; tail -5 /tmp/pip.log; }
        done
        sort -u /tmp/report.txt
    """)
    for line in ("global.find-links='/usr/share/python-wheels'",
                 "cffi-1.17.1-cp313-cp313-linux_wasm.whl\ncffi-2.0.0-cp313-cp313-linux_wasm.whl\n"
                 "cffi-2.1.1-cp313-cp313-linux_wasm.whl\npycparser-3.11-py3-none-any.whl\n"):
        check(f"pip finds the add-on's wheels: {line.split(chr(10))[0]}", line in out, out + err)
    for req, version in (("cffi~=1.17", "1.17.1"), ("cffi>=2.0,<2.1", "2.0.0"), ("cffi>=1.12", "2.1.1")):
        want = f"wheel {req} {version} True 5 42-x [1, 4, 5] ['cffi_modules'] 3.11"
        check(f"pip install '{req}' without the index: cffi {version} from its wheel, calls, varargs, callbacks",
              want in out, out + err)
    for name, version in (("needs_cffi1", "1.17.1"), ("needs_cffi", "2.1.1")):
        check(f"a build's isolated environment takes cffi {version} from the wheels ({name})",
              f"isolated {name} cffi {version} {version} True" in out, out + err)

    # unicorn's versions (pip.conf: only-binary): a requirement gets the highest of the wheels'
    # versions it allows (pwntools' pin: 2.0.1.post1), not an sdist of a higher one an index has (one
    # is put in a second find-links here, as PyPI's 2.1.2 is); a version no wheel has is an error that
    # names those there are; each version, installed by pip, prints what its own wheel prints
    missing = [v for v in UNICORN_WHEELS[:-1] if not os.path.exists(os.path.join(T, "expected", f"unicorn_check-{v}.out"))]
    check("unicorn's other versions have their expected output (tests/reference.py)", not missing, missing)
    probe = ("import importlib.metadata as md, unicorn; "
             "print(md.version('unicorn'), unicorn.__file__.startswith('/tmp/u/'), unicorn.uc_version()[:2])")
    lines = ["python3 -m pip config list 2>/dev/null", "ls /usr/share/python-wheels | grep unicorn",
             "mkdir -p /tmp/index && tar -czf /tmp/index/unicorn-2.1.2.tar.gz -C /tmp index"]
    for req in ("unicorn>=2.0.1,!=2.1.3,!=2.1.4", "unicorn<2", "unicorn>=2"):
        lines.append(f"rm -rf /tmp/u; python3 -m pip install -q --no-index --find-links /tmp/index --target /tmp/u '{req}' "
                     f"> /tmp/pip.log 2>&1 || {{ echo 'pip failed: {req}'; tail -3 /tmp/pip.log; }}; "
                     f"echo \"req {req} $(PYTHONPATH=/tmp/u python3 -c {shlex.quote(probe)})\"")
    lines.append("python3 -m pip install --no-index --target /tmp/u2 'unicorn==2.1.2' 2>&1 | grep -o 'from versions: [^)]*'")
    for v in UNICORN_WHEELS:
        lines.append(f"rm -rf /tmp/u; python3 -m pip install -q --no-index --target /tmp/u 'unicorn=={v}' > /tmp/pip.log 2>&1 "
                     f"|| tail -3 /tmp/pip.log; echo '### {v}'; PYTHONPATH=/tmp/u python3 ../checks/unicorn_check.py; "
                     f"echo \"### status $?\"")
    rc, out, err = guest("\n".join(lines))
    for line in ("global.only-binary='unicorn'", "unicorn-1.0.3-cp313-cp313-linux_wasm.whl\n"
                 "unicorn-2.0.1.post1-cp313-cp313-linux_wasm.whl\nunicorn-2.1.4-cp313-cp313-linux_wasm.whl\n"):
        check(f"pip finds unicorn's wheels, from wheels only: {line.split(chr(10))[0]}", line in out, out + err)
    for req, version in (("unicorn>=2.0.1,!=2.1.3,!=2.1.4", "2.0.1.post1 True (2, 0)"), ("unicorn<2", "1.0.3 True (1, 0)"),
                         ("unicorn>=2", "2.1.4 True (2, 1)")):
        check(f"pip install '{req}' without the index, an sdist of 2.1.2 beside: unicorn {version.split()[0]}",
              f"req {req} {version}" in out, out + err)
    check("pip install 'unicorn==2.1.2': no such wheel, the versions there are named",
          "from versions: 1.0.3, 2.0.1.post1, 2.1.4" in out, out + err)
    parts = {}
    for chunk in out.split("### ")[1:]:
        head, _, body = chunk.partition("\n")
        if head.startswith("status"):
            parts[last] = (parts[last][0], head.split()[1])
        else:
            last = head.strip()
            parts[last] = (body, None)
    for v in UNICORN_WHEELS:
        name = "unicorn_check.out" if v == UNICORN_WHEELS[-1] else f"unicorn_check-{v}.out"
        want = open(os.path.join(T, "expected", name)).read() if os.path.exists(os.path.join(T, "expected", name)) else ""
        got, status = parts.get(v, ("", None))
        diff = "".join(list(difflib.unified_diff(want.splitlines(True), got.splitlines(True), "wheel", "guest", n=1))[:60])
        check(f"unicorn {v} from its wheel (pip): unicorn_check as with its own wheel ({len(want.splitlines())} lines)",
              status == "0" and got == want, (diff or f"status {status}") + ("\n" + err[-2000:] if status != "0" else ""))

    # CONFIG_SITE (site-packages/collabo-config-site.pth): what a build Python starts runs configure
    # with, when build-tools' config.site is there (not in this guest: made here), and never over a
    # CONFIG_SITE already set
    rc, out, err = guest("""
        python3 -c 'import os; print("config-site", os.environ.get("CONFIG_SITE"))'
        mkdir -p /usr/share && echo 'build_alias=wasm32-unknown-linux-musl' > /usr/share/config.site
        python3 -c 'import subprocess; subprocess.run(["sh", "-c", "echo config-site-child $CONFIG_SITE"])'
        CONFIG_SITE=/other python3 -c 'import os; print("config-site-kept", os.environ["CONFIG_SITE"])'
    """)
    check("CONFIG_SITE for configure scripts Python starts: unset without config.site, set with it, a set one kept",
          "config-site None\nconfig-site-child /usr/share/config.site\nconfig-site-kept /other\n" in out, out + err)

    print(f"{len(fails)} failed" if fails else "all passed")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
