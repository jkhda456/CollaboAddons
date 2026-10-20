#!/usr/bin/env python3
"""The packages' own test suites, in the guest: numpy's (every subpackage's tests/), Pillow's
(Tests/), lxml's (lxml.tests, lxml.html.tests), cffi's (its backend's, and those of its tests that
need no C compiler), cryptography's (tests/, with its vectors) and libffi's (its testsuite's C
programs, built here with the add-on's compiler against packages/cffi's libffi), from their
sources, with pytest and hypothesis from tests/upstream.lock, against the interpreter the add-on
puts in place; and those of rpds-py and of the two packages of mcp's that use it, jsonschema (with the
JSON Schema Test Suite) and referencing (with its suite). Every failure must be a known one (tests/upstream-known.txt:
what cannot work in the guest, each with its reason); any other is a finding. The checks of
tests/run.sh compare with the wheels; these cover what the packages test themselves — it is how
the guest's traps of mistyped methods were found (tools/fpcast-scan.py now finds them at build).

    python3 tests/upstream.py [numpy|pillow|lxml|cffi|cryptography|libffi|rpds-py|jsonschema|referencing ...]
                                                                       (tests/run.sh --upstream)

Same guest as tests/test_guest.py (COLLABO_RUNTIME, PYEXT_CPIO, PYEXT_ENGINE). A suite takes a few
minutes; the first run downloads the wheels into build/upstream.
"""
import fnmatch
import glob
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tarfile
import time
import urllib.request
import zipfile
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import test_guest as tg  # noqa: E402  (the guest, the runtime, the add-on's image)

T = tg.T
A = tg.A
U = os.path.join(A, "build/upstream")
SITE = "/usr/lib/python3.13/site-packages"
# numpy's core test modules import ctypes at the top, which the guest's Python does not have (no
# _ctypes: libffi has no port for its ABI): with this stand-in they are collected, and only their
# tests that use ctypes fail (upstream-known.txt). For the tests alone; nothing of it is in the image.
CTYPES_STUB = '''"""A stand-in for ctypes for numpy's test suite in the guest (tests/upstream.py)."""
class _Unavailable:
    def __init__(self, *args, **kwargs):
        raise NotImplementedError("ctypes is not available in the guest")


def __getattr__(name):
    if name.startswith("__"):
        raise AttributeError(name)
    return type(name, (_Unavailable,), {})
'''
# cffi's tests read the sizes of C's types from ctypes, and look for the C library with
# ctypes.util.find_library: the sizes here (wasm32), and no library file — the C library and libm
# are the program itself in the guest (the interpreter exports them: dlopen(NULL)), which
# CFFI_GUEST makes of the libraries "c", "m", "dl", ... the tests ask for (musl has them all in one).
CTYPES_SIZES_STUB = '''"""A stand-in for ctypes for cffi's test suite in the guest (tests/upstream.py): the sizes
of C's types there (wasm32) and find_library; nothing can be called through it."""
from . import util  # noqa: F401


class _CType:
    _size = 0


for _name, _size in [("c_char", 1), ("c_byte", 1), ("c_ubyte", 1), ("c_bool", 1), ("c_short", 2), ("c_ushort", 2),
                     ("c_int", 4), ("c_uint", 4), ("c_long", 4), ("c_ulong", 4), ("c_longlong", 8), ("c_ulonglong", 8),
                     ("c_float", 4), ("c_double", 8), ("c_longdouble", 16), ("c_void_p", 4), ("c_char_p", 4),
                     ("c_wchar", 4), ("c_wchar_p", 4), ("c_size_t", 4), ("c_ssize_t", 4)]:
    globals()[_name] = type(_name, (_CType,), {"_size": _size})


def sizeof(t):
    return t._size
'''
CTYPES_UTIL_STUB = '''def find_library(name):
    return None
'''
CFFI_GUEST = '''"""cffi's test suite in the guest (tests/upstream.py): the C library's libraries are the program."""
import pytest


def _guest(name):
    return None if name in ("c", "m", "dl", "pthread", "rt") else name


def pytest_configure(config):
    import cffi.api
    load = cffi.api._load_backend_lib
    cffi.api._load_backend_lib = lambda backend, name, flags: load(backend, _guest(name), flags)


@pytest.fixture(autouse=True)
def _guest_libraries(request):
    mod = getattr(request, "module", None)
    if mod is not None and hasattr(mod, "find_and_load_library") and hasattr(mod, "load_library"):
        load = mod.load_library
        mod.find_and_load_library = lambda name, flags=mod.RTLD_NOW: load(_guest(name), flags)
    yield
'''
# pytest that goes on after a test ends the interpreter — a trap, which here is what a call through
# a pointer of another type is (cffi's tests declare functions wrongly on purpose: getenv() as
# returning void; natively the result is just ignored) — from the test after it, the one that
# ended it reported as FAILED "the interpreter ended"; the rest as pytest reports them.
RESILIENT_PYTEST = '''"""pytest, on after a test that ends the interpreter (tests/upstream.py)."""
import re, subprocess, sys, time

start = time.time()
# resilient-pytest.py [PYTEST-OPTION ...] -- PATH ...
cut = sys.argv.index("--")
opts, paths = sys.argv[1:cut], sys.argv[cut + 1:]
common = [sys.executable, "-m", "pytest", "-p", "no:cacheprovider", "-o", "addopts="] + opts
out = subprocess.run(common + ["--collect-only", "-q"] + paths, capture_output=True, text=True).stdout
ids = [line for line in out.splitlines() if "::" in line]
counts = dict(failed=0, passed=0, skipped=0, errors=0, xfailed=0, xpassed=0)
kinds = {"PASSED": "passed", "FAILED": "failed", "SKIPPED": "skipped", "ERROR": "errors", "XFAIL": "xfailed",
         "XPASS": "xpassed"}
remaining = ids
while remaining:
    p = subprocess.run(common + ["-v", "--tb=no", "-rfE"] + remaining, capture_output=True, text=True)
    seen = set()
    for line in p.stdout.splitlines():
        m = re.match(r"(\\S+::\\S+) (PASSED|FAILED|SKIPPED|ERROR|XFAIL|XPASS)\\b", line)
        if m:
            seen.add(m.group(1))
            counts[kinds[m.group(2)]] += 1
        elif line.startswith(("FAILED ", "ERROR ")):
            print(line, flush=True)
    if re.search(r"\\d+ (passed|failed|error)", p.stdout.splitlines()[-1] if p.stdout.strip() else ""):
        break
    # the interpreter ended: at the first test that has no result
    rest = [i for i in remaining if i not in seen]
    if not rest:
        break
    print(f"FAILED {rest[0]} - the interpreter ended", flush=True)
    counts["failed"] += 1
    remaining = rest[1:]
print(", ".join(f"{v} {k}" for k, v in counts.items() if v or k in ("failed", "passed")) + f" in {time.time() - start:.2f}s")
'''
# cffi's TestOwnLib (and so TestFFI, which mixes it in) compiles testownlib.c with cc first: the
# guest has no compiler, so it gets the library prepare() built from the same source with the
# add-on's compiler (a shared library, as cc -shared would make it).
CFFI_CC = '''#!/bin/sh
# cffi's tests compile testownlib.c with cc: this guest has no compiler, so the one built for it
while [ $# -gt 0 ]; do [ "$1" = -o ] && out=$2; shift; done
exec cp /upstream/cffi-testownlib.so "$out"
'''
# libffi's testsuite: one program per test (built by prepare()), each run; reported as pytest
# reports. A test passes when it exits 0 (CHECK aborts).
LIBFFI_RUNNER = '''cd /upstream/libffi && passed=0 failed=0 start=$(date +%s)
for t in bin/*; do
  n=${t#bin/}
  (cd /tmp && timeout 300 /upstream/libffi/$t > /dev/null 2>&1); rc=$?
  if [ $rc != 0 ]; then echo "FAILED ${n%.*}/${n##*.} - exit $rc"; failed=$((failed + 1)); else passed=$((passed + 1)); fi
done
echo "$failed failed, $passed passed in $(( $(date +%s) - start ))s"
'''
SUITES = {
    # numpy's tests, copied into the installed package, so that they import it as upstream intends
    "numpy": ("numpy", f"""
        for d in /upstream/numpy/numpy/*/tests /upstream/numpy/numpy/tests; do
            rel=${{d#/upstream/numpy/}}; mkdir -p {SITE}/$(dirname $rel); cp -r $d {SITE}/$rel; done
        cp /upstream/numpy/numpy/conftest.py {SITE}/numpy/
        cd /tmp && COLUMNS=300 PYTHONPATH=/upstream/stub:/upstream/site python3 -m pytest -p no:cacheprovider -q --tb=no -rfE \\
            --continue-on-collection-errors --rootdir={SITE} -W ignore::PendingDeprecationWarning \\
            -o addopts="" --pyargs numpy"""),
    "pillow": ("pillow", """
        cp -r /upstream/pillow /tmp/pillow && cd /tmp/pillow && PYTHONPATH=/upstream/site \\
            COLUMNS=300 python3 -m pytest -p no:cacheprovider -q --tb=no -rfE --continue-on-collection-errors -o addopts="" Tests"""),
    # lxml's tests, copied into the installed package as numpy's (they import their package's
    # modules), its doc/ where their doctests look for it (four folders up: /usr/lib/python3.13/doc)
    "lxml": ("lxml", f"""
        cp -r /upstream/lxml/lxml/tests {SITE}/lxml/tests; cp -r /upstream/lxml/lxml/html/tests {SITE}/lxml/html/tests
        cp -r /upstream/lxml/doc /usr/lib/python3.13/doc
        mkdir /tmp/lxml && cp -r /upstream/lxml/doc /upstream/lxml/samples /tmp/lxml/   # (as from the sdist's top, as test.py runs)
        cd /tmp/lxml && python3 /upstream/lxml-runner.py"""),
    # cffi's backend tests (src/c/test_c.py) and those of its testing/ that need no C compiler (the
    # rest compile modules: verify(), the recompiler, setuptools; the guest has no compiler)
    "cffi": ("cffi", """
        cp -r /upstream/cffi /tmp/cffi && cd /tmp/cffi && PATH=/upstream/bin-cffi:$PATH \\
            PYTHONPATH=/upstream/stub-cffi:/upstream/site COLUMNS=300 \\
            python3 /upstream/resilient-pytest.py -p cffi_guest -- src/c/test_c.py testing/cffi0/test_ffi_backend.py testing/cffi0/test_cdata.py \\
            testing/cffi0/test_model.py testing/cffi0/test_parsing.py testing/cffi0/test_platform.py \\
            testing/cffi0/test_version.py testing/cffi1/test_ffi_obj.py testing/cffi1/test_realize_c_type.py \\
            testing/cffi1/test_unicode_literals.py testing/cffi1/test_dlopen.py testing/cffi1/test_commontypes.py \\
            testing/cffi1/test_cffi_binary.py"""),
    # cryptography's tests/ (not its benchmarks), against the installed package, with its vectors
    "cryptography": ("cryptography", """
        cp -r /upstream/cryptography /tmp/cryptography && cd /tmp/cryptography && PYTHONPATH=/upstream/site \\
            COLUMNS=300 python3 -m pytest -p no:cacheprovider -q --tb=no -rfE --continue-on-collection-errors \\
            -o addopts="" --ignore=tests/bench tests"""),
    "libffi": ("libffi", LIBFFI_RUNNER),
    # rpds-py's tests/, against the installed package
    "rpds-py": ("rpds-py", """
        cp -r /upstream/rpds-py /tmp/rpds && cd /tmp/rpds && PYTHONPATH=/upstream/site COLUMNS=300 \\
            python3 -m pytest -p no:cacheprovider -q --tb=no -rfE -o addopts="" tests"""),
    # jsonschema's tests (its wheel has them: jsonschema.tests), with the JSON Schema Test Suite its sdist carries
    "jsonschema": ("jsonschema", """
        cd /tmp && JSON_SCHEMA_TEST_SUITE=/upstream/jsonschema/json PYTHONPATH=/upstream/site COLUMNS=300 \\
            python3 -m pytest -p no:cacheprovider -q --tb=no -rfE -o addopts="" --pyargs jsonschema.tests"""),
    # referencing's tests, copied into the installed package as numpy's, with the referencing suite
    "referencing": ("referencing", f"""
        cp -r /upstream/referencing/referencing/tests {SITE}/referencing/tests
        cd /tmp && REFERENCING_SUITE=/upstream/referencing/suite PYTHONPATH=/upstream/site COLUMNS=300 \\
            python3 -m pytest -p no:cacheprovider -q --tb=no -rfE -o addopts="" --pyargs referencing.tests"""),
}


# Suites whose pytest is known to trap at its exit, after its summary (every result already printed):
# jsonschema's. The trap is in the interpreter's last collection (Py_FinalizeEx, finalize_modules,
# gc_collect_main: a GC slot called as another type), only with pytest's assertion rewriting and
# only for some of its test modules (test_cli, test_types, test_validators: those that import
# Draft202012Validator by name); not rpds-py's (its objects are not GC-tracked, so the collector
# calls none of its functions). Not found yet; see docs/note.md (2026-10-20).
EXIT_TRAP_KNOWN = {"jsonschema"}


def lock_file(name):
    """The download of NAME: the build's (sources.lock, in build/dl), or the tests' (upstream.lock)."""
    for line in open(os.path.join(A, "sources.lock")):
        f = line.split()
        if f and f[0] == name:
            return os.path.join(A, "build/dl", f[3] if len(f) > 3 else f[1].rsplit("/", 1)[1])
    for line in open(os.path.join(T, "upstream.lock")):
        f = line.split()
        if f and f[0] == name:
            return os.path.join(U, "dl", f[1].rsplit("/", 1)[1])
    sys.exit(f"{name} is in neither sources.lock nor tests/upstream.lock")


# lxml's tests are written for its own runner (test.py): each module's test_suite() gathers its
# TestCases and doctests (pytest would take those functions, and the abstract base classes, for
# tests). This runs them so, and reports as pytest does: FAILED/ERROR lines, then the counts.
LXML_RUNNER = '''"""lxml's own tests as its test.py runs them, reported as pytest reports (tests/upstream.py)."""
import importlib, os, sys, time, unittest

start = time.time()
counts = dict(passed=0, failed=0, skipped=0, errors=0)
for package in ("lxml.tests", "lxml.html.tests"):
    folder = os.path.dirname(importlib.import_module(package).__file__)
    for f in sorted(os.listdir(folder)):
        if not (f.startswith("test_") and f.endswith(".py")):
            continue
        path = package.replace(".", "/") + "/" + f
        try:
            suite = importlib.import_module(package + "." + f[:-3]).test_suite()
        except Exception as e:
            print(f"ERROR {path} - {type(e).__name__}: {e}", flush=True)
            counts["errors"] += 1
            continue
        result = unittest.TestResult()
        suite.run(result)
        for kind, cases in (("FAILED", result.failures), ("ERROR", result.errors)):
            for case, text in cases:
                name = case.id()
                where = path + "::" + "::".join(name.split(".")[-2:]) if name.startswith(package) else path + "::" + name
                last = [line for line in text.strip().splitlines() if line.strip()][-1]
                print(f"{kind} {where} - {last[:200]}", flush=True)
        counts["failed"] += len(result.failures)
        counts["errors"] += len(result.errors)
        counts["skipped"] += len(result.skipped)
        counts["passed"] += result.testsRun - len(result.failures) - len(result.errors) - len(result.skipped)
print(f"{counts['failed']} failed, {counts['passed']} passed, {counts['skipped']} skipped, {counts['errors']} errors "
      f"in {time.time() - start:.2f}s")
'''


def prepare():
    os.makedirs(os.path.join(U, "dl"), exist_ok=True)
    site = os.path.join(U, "site")
    stamp = os.path.join(site, ".done")
    if not os.path.exists(stamp) or os.path.getmtime(stamp) < os.path.getmtime(os.path.join(T, "upstream.lock")):
        shutil.rmtree(site, ignore_errors=True)
        for line in open(os.path.join(T, "upstream.lock")):
            if not line.strip() or line.startswith("#"):
                continue
            _, url, sha = line.split()
            path = os.path.join(U, "dl", url.rsplit("/", 1)[1])
            if not os.path.exists(path):
                print(f"== fetch {os.path.basename(path)}")
                data = urllib.request.urlopen(url).read()
                if hashlib.sha256(data).hexdigest() != sha:
                    sys.exit(f"{path}: checksum mismatch")
                open(path, "wb").write(data)
            if path.endswith(".whl"):
                zipfile.ZipFile(path).extractall(site)
        open(os.path.join(site, ".done"), "w").close()
    # the suites, from the sources the add-on was built from
    for name, member, dest in (("numpy", re.compile(r"[^/]+/(numpy/(.*/)?(tests/.*|conftest\.py))$"), "numpy"),
                               ("pillow", re.compile(r"[^/]+/((Tests/.*)|conftest\.py|pyproject\.toml)$"), "pillow"),
                               ("lxml", re.compile(r"[^/]+/(?:src/)?((?:lxml/(?:html/)?tests|doc|samples)/.*)$"), "lxml"),
                               ("cffi", re.compile(r"[^/]+/(.*)$"), "cffi"),
                               ("cryptography", re.compile(r"[^/]+/((tests/.*)|pyproject\.toml)$"), "cryptography"),
                               ("rpds-py", re.compile(r"[^/]+/(tests/.*)$"), "rpds-py"),
                               ("jsonschema-sdist", re.compile(r"[^/]+/(json/.*)$"), "jsonschema"),
                               ("referencing-sdist", re.compile(r"[^/]+/((referencing/tests|suite)/.*)$"), "referencing")):
        out = os.path.join(U, dest)
        stamp = os.path.join(out, ".done")
        src = lock_file(name)
        if os.path.exists(stamp) and os.path.getmtime(stamp) > os.path.getmtime(src):
            continue
        shutil.rmtree(out, ignore_errors=True)
        with tarfile.open(src) as tar:
            for m in tar.getmembers():
                g = member.match(m.name)
                if g and m.isfile():
                    m.name = g.group(1)
                    tar.extract(m, out, filter="data")
        open(stamp, "w").close()
    with open(os.path.join(U, "lxml-runner.py"), "w") as f:
        f.write(LXML_RUNNER)
    with open(os.path.join(U, "resilient-pytest.py"), "w") as f:
        f.write(RESILIENT_PYTEST)
    os.makedirs(os.path.join(U, "stub/ctypes"), exist_ok=True)
    with open(os.path.join(U, "stub/ctypes/__init__.py"), "w") as f:
        f.write(CTYPES_STUB)
    os.makedirs(os.path.join(U, "stub-cffi/ctypes"), exist_ok=True)
    for path, text in (("stub-cffi/ctypes/__init__.py", CTYPES_SIZES_STUB), ("stub-cffi/ctypes/util.py", CTYPES_UTIL_STUB),
                       ("stub-cffi/cffi_guest.py", CFFI_GUEST), ("bin-cffi/cc", CFFI_CC)):
        os.makedirs(os.path.dirname(os.path.join(U, path)), exist_ok=True)
        with open(os.path.join(U, path), "w") as f:
            f.write(text)
    os.chmod(os.path.join(U, "bin-cffi/cc"), 0o755)
    build_cffi_testownlib()
    build_libffi_tests()
    subprocess.run(["chmod", "-R", "a+rX", U], check=True)


def build_cffi_testownlib():
    """The library cffi's TestOwnLib compiles (testing/cffi0/test_ownlib.py's SOURCE), built here."""
    import ast
    test = os.path.join(U, "cffi/testing/cffi0/test_ownlib.py")
    out = os.path.join(U, "cffi-testownlib.so")
    if os.path.exists(out) and os.path.getmtime(out) > os.path.getmtime(test):
        return
    tree = ast.parse(open(test).read())
    source = next(n.value.value for n in tree.body if isinstance(n, ast.Assign)
                  and any(getattr(t, "id", None) == "SOURCE" for t in n.targets))
    c = os.path.join(U, "cffi-testownlib.c")
    with open(c, "w") as f:
        f.write(source)
    env = dict(os.environ, WASM_LD=os.path.join(tg.ROOT, ".tools/rust-wasm/bin/wasm-ld"))
    subprocess.run([os.path.join(A, "tools/wasm-pyext-cc"), "-O2", "-shared", "-fPIC", c, "-o", out], env=env, check=True)


def build_libffi_tests():
    """libffi's testsuite (its C programs: libffi.call, .closures, .complex, .bhaible, .threads)
    built for the guest against packages/cffi's libffi, into build/upstream/libffi/bin. Not
    libffi.go (Go closures) and libffi.vector (vector types): this port has neither; nor
    overread.c, which maps memory."""
    srcs = sorted(glob.glob(os.path.join(A, "build/cffi/deps-src/libffi-*")))
    lib = os.path.join(A, "build/cffi/deps/lib/libffi.a")
    if not srcs or not os.path.exists(lib):
        sys.exit("no libffi build: build the add-on's cffi first (build.sh cffi)")
    src = srcs[-1]
    out = os.path.join(U, "libffi")
    stamp = os.path.join(out, ".done")
    if os.path.exists(stamp) and os.path.getmtime(stamp) > os.path.getmtime(lib):
        return
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(os.path.join(out, "bin"))
    build = os.path.join(src, "wasm32-unknown-linux-musl")
    tests = [t for t in sorted(glob.glob(os.path.join(src, "testsuite/libffi.*/*.c")) +
                               glob.glob(os.path.join(src, "testsuite/libffi.*/*.cc")))
             if os.path.basename(os.path.dirname(t)) in ("libffi.call", "libffi.closures", "libffi.complex",
                                                          "libffi.bhaible", "libffi.threads")
             and os.path.basename(t) not in ("testcases.c", "overread.c")]
    env = dict(os.environ, WASM_LD=os.path.join(tg.ROOT, ".tools/rust-wasm/bin/wasm-ld"))
    print(f"== libffi's testsuite: {len(tests)} programs")

    def one(t):
        name = os.path.basename(os.path.dirname(t)) + "." + os.path.splitext(os.path.basename(t))[0]
        cc = os.path.join(A, "tools/wasm-pyext-c++" if t.endswith(".cc") else "tools/wasm-pyext-cc")
        flags = ["-O2", "-w", f"-I{build}", f"-I{build}/include", f"-I{os.path.dirname(t)}",
                 f"-I{src}/testsuite/libffi.call", f"-I{src}/include"]
        r = subprocess.run([cc, *flags, t, lib, "-lm", "-rdynamic", "-o", os.path.join(out, "bin", name)],
                           env=env, capture_output=True, text=True)
        if r.returncode:
            sys.exit(f"{t}: does not build\n{r.stderr[-2000:]}")
        os.chmod(os.path.join(out, "bin", name), 0o755)

    with ThreadPoolExecutor(os.cpu_count()) as ex:
        list(ex.map(one, tests))
    open(stamp, "w").close()


def known():
    patterns = []
    for line in open(os.path.join(T, "upstream-known.txt")):
        line = line.split("#", 1)[0].strip()
        if line:
            suite, pattern = line.split(None, 1)
            patterns.append((suite, pattern))
    return patterns


def main():
    suites = sys.argv[1:] or list(SUITES)
    prepare()
    patterns = known()
    bad = 0
    for suite in suites:
        _, script = SUITES[suite]
        start = time.time()
        rc, out, err = tg.guest(script, timeout=3600, mount=(U, "/upstream"))
        summary = next((l for l in reversed(out.splitlines()) if re.search(r"\d+ (passed|failed)", l)), "(no summary)")
        failures = sorted({re.sub(r"^.*?site-packages/", "", l.split(" - ")[0].split(None, 1)[1])
                           for l in out.splitlines() if l.startswith(("FAILED ", "ERROR "))})
        with open(os.path.join(U, f"{suite}.out"), "w") as f:
            f.write(out)
        unknown = [f for f in failures if not any(s == suite and fnmatch.fnmatch(f, p) for s, p in patterns)]
        # (each of libffi's tests is a program of its own, and cffi's run on after a test that ends
        # the interpreter: there one that traps is one failed test)
        trapped = suite not in ("libffi", "cffi") and ("trapped and was ended" in err or "nothing caught" in err)
        if trapped and suite in EXIT_TRAP_KNOWN and summary != "(no summary)":
            print(f"  (known: {suite}'s pytest trapped at its exit, after its summary: EXIT_TRAP_KNOWN)")
            trapped = False
        print(f"{suite}: {summary.strip('= ')} ({time.time() - start:.0f} s); {len(failures)} failures, "
              f"{len(failures) - len(unknown)} of them known")
        for f in unknown:
            print(f"  UNKNOWN {f}")
        if trapped or "(no summary)" == summary:
            print("  the interpreter ended:\n    " + "\n    ".join(err.strip().splitlines()[-25:]))
        tg.check(f"{suite}'s own tests: no failure but the known ones", not unknown and not trapped and summary != "(no summary)")
        bad += bool(unknown or trapped)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
