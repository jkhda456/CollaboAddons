"""Make tests/expected/<check>.out: what each tests/checks/<check>.py prints with the packages' own
wheels from PyPI, on this machine, with the host CPython 3.13 (python/host) — what the guest's
build of the same versions must print too (tests/test_guest.py compares).

    python3 addons/python-extension/tests/reference.py [CHECK...]

The wheels are the manylinux ones for this machine's CPU (CPython 3.13, or the stable ABI; pure
Python ones as they are), checked against PyPI's sha256, unpacked into build/reference. Versions:
the add-on's out/ (the .dist-info in its image's site-packages), so the reference follows what was
built.

Unicorn's other versions, the wheels the add-on carries for pip (build/unicorn/wheels): the
unicorn check with each one's own wheel too, in a site of its own (build/reference/unicorn-<version>),
into expected/unicorn_check-<version>.out. Before 2.1 the binding imports distutils and
pkg_resources, which Python 3.12 and newer setuptools have not (the add-on's builds have
patches-<line>/0005 instead): here they get the two functions it calls of them
(build/reference/unicorn-shim), which only say where its library is.

The host CPython (python/build-host.sh) has no ssl and hashlib's OpenSSL: this machine has no
OpenSSL headers. The checks that import them (mcp's: httpx2, pyjwt) get the two modules built for it
here, against the OpenSSL the python step builds the guest's from (its source, python/sources.lock),
static, in build/reference/ssl — for the reference alone; nothing of it is in the image.
"""
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import urllib.request
import zipfile

T = os.path.dirname(os.path.abspath(__file__))
A = os.path.dirname(T)
ROOT = os.path.dirname(os.path.dirname(A))
HOSTPY = os.path.join(ROOT, "python/host/bin/python3.13")
REF = os.path.join(A, "build/reference")
SSL = os.path.join(REF, "ssl")
PACKAGES = {"numpy_check": ["numpy"], "pillow_check": ["numpy", "pillow"], "lief_check": ["lief"], "lxml_check": ["lxml"],
            "cffi_check": ["pycparser", "cffi"], "cryptography_check": ["pycparser", "cffi", "cryptography"],
            "rpds_check": ["rpds-py"], "unicorn_check": ["unicorn"]}
# mcp's: what it needs of the Python image's (python/packages.lock), the add-on's compiled packages,
# and its wheels (packages/mcp)
PACKAGES["mcp_check"] = ["pydantic", "pydantic-core", "annotated-types", "typing-inspection", "typing-extensions", "anyio",
                         "sniffio", "idna", "httpx2", "httpcore2", "h11", "truststore", "pycparser", "cffi", "cryptography",
                         "rpds-py", "mcp", "mcp-types", "jsonschema", "jsonschema-specifications", "referencing", "attrs",
                         "opentelemetry-api", "pyjwt", "python-multipart", "sse-starlette", "starlette", "uvicorn", "click"]
NEEDS_SSL = {"mcp_check"}


def host_ssl():
    """_ssl and _hashlib for the host CPython, in build/reference/ssl (see above); that path."""
    out = SSL
    if os.path.exists(os.path.join(out, ".done")):
        return out
    line = next(l.split() for l in open(os.path.join(ROOT, "python/sources.lock")) if l.startswith("openssl "))
    tarball = os.path.join(ROOT, "python/src", line[1].rsplit("/", 1)[1])
    if not os.path.exists(tarball):
        sys.exit(f"{tarball} missing: the python step downloads it (python/build.sh)")
    if hashlib.sha256(open(tarball, "rb").read()).hexdigest() != line[2]:
        sys.exit(f"{tarball}: checksum mismatch")
    print("== OpenSSL, _ssl and _hashlib for the host CPython (once)")
    work = os.path.join(REF, "ssl-build")
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    subprocess.run(["tar", "-xzf", tarball, "-C", work], check=True)
    src = os.path.join(work, os.listdir(work)[0])
    target = {("Linux", "x86_64"): "linux-x86_64", ("Linux", "aarch64"): "linux-aarch64",
              ("Darwin", "arm64"): "darwin64-arm64-cc", ("Darwin", "x86_64"): "darwin64-x86_64-cc"}[
                  (platform.system(), platform.machine())]
    prefix = os.path.join(work, "prefix")
    log = open(os.path.join(work, "build.log"), "w")
    for argv in (["perl", "./Configure", target, "CC=clang-19", "-fPIC", "no-shared", "no-tests", "no-apps", "no-docs",
                  f"--prefix={prefix}", "--libdir=lib"],
                 ["make", f"-j{os.cpu_count()}", "build_libs"], ["make", "install_dev"]):
        if subprocess.run(argv, cwd=src, stdout=log, stderr=subprocess.STDOUT).returncode:
            sys.exit(f"{' '.join(argv)} failed: {log.name}")
    py = os.path.join(ROOT, "python/src/Python-3.13.14")
    suffix = subprocess.run([HOSTPY, "-c", "import sysconfig; print(sysconfig.get_config_var('EXT_SUFFIX'))"],
                            capture_output=True, text=True, check=True).stdout.strip()
    os.makedirs(out, exist_ok=True)
    for module, source in (("_ssl", "_ssl.c"), ("_hashlib", "_hashopenssl.c")):
        argv = ["clang-19", "-shared", "-fPIC", "-O2", "-DPy_BUILD_CORE_MODULE", f"-I{ROOT}/python/build-host",
                f"-I{py}/Include", f"-I{py}/Include/internal", f"-I{prefix}/include", os.path.join(py, "Modules", source),
                f"{prefix}/lib/libssl.a", f"{prefix}/lib/libcrypto.a", "-o", os.path.join(out, module + suffix)]
        if subprocess.run(argv, stdout=log, stderr=subprocess.STDOUT).returncode:
            sys.exit(f"{module} does not build: {log.name}")
    shutil.rmtree(work)
    open(os.path.join(out, ".done"), "w").close()
    return out


def built_versions():
    """{name: version}: the add-on's packages (its build's site-packages), and the Python image's
    (python/packages.lock); names normalized as PyPI's (rpds-py, not rpds_py)."""
    found = {}
    for line in open(os.path.join(ROOT, "python/packages.lock")):
        f = line.split()
        if f and f[0] in ("pure", "native"):
            found[f[1]] = f[2]
    site = os.path.join(A, "build/site")
    for d in os.listdir(site):
        m = re.fullmatch(r"(.+)-([^-]+)\.dist-info", d)
        if m:
            found[re.sub(r"[-_.]+", "-", m.group(1)).lower()] = m.group(2)
    return found


def wheel(name, version):
    """The PyPI wheel of name==version for CPython 3.13 on this machine: cp313's own, else the
    stable ABI's (abi3 of CPython 3.13 or older; not the free-threaded abi3t), else one for any
    Python 3 on this machine (unicorn's before 2.1: py2.py3-none, a library ctypes loads), else a
    pure one."""
    arch = platform.machine()
    meta = json.load(urllib.request.urlopen(f"https://pypi.org/pypi/{name}/{version}/json"))
    files = [u for u in meta["urls"] if u["filename"].endswith(".whl")]
    native = [u for u in files if "manylinux" in u["filename"] and u["filename"].endswith(f"_{arch}.whl")]
    for u in native:
        if "-cp313-cp313-" in u["filename"]:
            return u
    for u in native:
        m = re.search(r"-cp3(\d+)-abi3-", u["filename"])
        if m and int(m.group(1)) <= 13:
            return u
    for u in native:
        if re.search(r"-(py3|py2\.py3)-none-", u["filename"]):
            return u
    for u in files:
        if u["filename"].endswith("-none-any.whl"):
            return u
    sys.exit(f"no {name} {version} wheel for CPython 3.13 on {arch}")


def install(name, version, dest=None):
    dest = dest or os.path.join(REF, "site")
    # (a site of its own, for a version of a package beside the one in the shared site: its marker)
    marker = os.path.join(REF, f"{name}-{version}.installed" if dest == os.path.join(REF, "site")
                          else f"{os.path.basename(dest)}-site.installed")
    if os.path.exists(marker):
        return dest
    u = wheel(name, version)
    os.makedirs(os.path.join(REF, "dl"), exist_ok=True)
    path = os.path.join(REF, "dl", u["filename"])
    if not os.path.exists(path):
        print(f"== fetch {u['filename']}")
        data = urllib.request.urlopen(u["url"]).read()
        if hashlib.sha256(data).hexdigest() != u["digests"]["sha256"]:
            sys.exit(f"{u['filename']}: checksum mismatch")
        open(path, "wb").write(data)
    zipfile.ZipFile(path).extractall(dest)
    open(marker, "w").close()
    return dest


UNICORN_SHIM = {"distutils/__init__.py": "",
                "distutils/sysconfig.py": "import sysconfig\n\n\ndef get_python_lib(*args, **kwargs):\n"
                                          "    return sysconfig.get_path('purelib')\n",
                "pkg_resources.py": "import importlib\nimport os\n\n\ndef resource_filename(package, name):\n"
                                    "    return os.path.join(os.path.dirname(importlib.import_module(package).__file__), name)\n"}


def unicorn_versions():
    """The versions of unicorn the add-on carries as wheels besides the installed one."""
    wheels = os.path.join(A, "build/unicorn/wheels")
    return sorted(d.split("-", 1)[1] for d in os.listdir(wheels) if os.path.exists(os.path.join(wheels, d, ".done"))) \
        if os.path.isdir(wheels) else []


def run_check(check, path):
    env = {"PATH": "/usr/bin:/bin", "PYTHONPATH": path, "PYTHONNOUSERSITE": "1", "LC_ALL": "C.UTF-8"}
    return subprocess.run([HOSTPY, "-s", os.path.join(T, "checks", check + ".py")], cwd=os.path.join(T, "samples"),
                          env=env, capture_output=True, text=True)


def main():
    versions = built_versions()
    checks = sys.argv[1:] or sorted(PACKAGES)
    os.makedirs(os.path.join(T, "expected"), exist_ok=True)
    for check in checks:
        for name in PACKAGES[check]:
            site = install(name, versions[name])
        path = site + (os.pathsep + host_ssl() if check in NEEDS_SSL else "")
        p = run_check(check, path)
        if p.returncode != 0 or not p.stdout.rstrip().endswith("done"):
            sys.exit(f"{check} failed with the official wheels:\n{p.stdout[-2000:]}{p.stderr[-3000:]}")
        with open(os.path.join(T, "expected", check + ".out"), "w") as f:
            f.write(p.stdout)
        print(f"  expected/{check}.out: {len(p.stdout.splitlines())} lines ({', '.join(f'{n} {versions[n]}' for n in PACKAGES[check])})")
    if "unicorn_check" in checks:
        shim = os.path.join(REF, "unicorn-shim")
        for rel, text in UNICORN_SHIM.items():
            os.makedirs(os.path.dirname(os.path.join(shim, rel)), exist_ok=True)
            open(os.path.join(shim, rel), "w").write(text)
        for version in unicorn_versions():
            site = install("unicorn", version, os.path.join(REF, f"unicorn-{version}"))
            p = run_check("unicorn_check", site + os.pathsep + shim)
            if p.returncode != 0 or not p.stdout.rstrip().endswith("done"):
                sys.exit(f"unicorn_check failed with unicorn {version}'s wheel:\n{p.stdout[-2000:]}{p.stderr[-3000:]}")
            with open(os.path.join(T, "expected", f"unicorn_check-{version}.out"), "w") as f:
                f.write(p.stdout)
            print(f"  expected/unicorn_check-{version}.out: {len(p.stdout.splitlines())} lines (its wheel)")


if __name__ == "__main__":
    main()
