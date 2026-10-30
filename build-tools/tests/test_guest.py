#!/usr/bin/env python3
"""The build-tools add-on in the guest: GNU make (parallel jobs and its jobserver, $(shell),
recursive make, remade makefiles, output sync, a recipe that is a script without #!, errors, a
write error at exit), ninja (depfiles, pools, a regenerated manifest, its tools, failures, under
make's jobserver) and pkgconf (as pkg-config: versions, flags, Requires, static linking, the
system's directories, its search path). With the python-extension add-on (PYEXT_DIR, default
../python-extension/out) also the Python headers as builds find them; with the llvm add-on
(LLVM_DIR, default ../llvm/out) make and ninja driving clang and wasm-ld (freestanding code: that
add-on has no C library yet). Run by tests/run.sh.

--upstream: the three's own test suites in the guest — ninja_test and pkgconf's test-runner and
API tests (TEST=1 build.sh puts them in build/tests), GNU make's run_make_tests.pl (perl, from the
tools image), whose failures must be those tests/upstream-known.txt lists.

The guest is the packaged runtime (COLLABO_RUNTIME, default dist/runtime/collabo-core-<platform>)
with out/build-tools.cpio (BT_DIR: another folder with build-tools.cpio).
"""
import functools
import hashlib
import http.server
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time

T = os.path.dirname(os.path.abspath(__file__))
A = os.path.dirname(T)
ROOT = os.path.dirname(os.path.dirname(A))
PLATFORM = {("Linux", "x86_64"): "linux-x64", ("Linux", "aarch64"): "linux-arm64",
            ("Darwin", "arm64"): "darwin-arm64", ("Darwin", "x86_64"): "darwin-x64"}.get(
                (platform.system(), platform.machine()), "linux-x64")
RT = os.environ.get("COLLABO_RUNTIME") or os.path.join(ROOT, "dist/runtime/collabo-core-" + PLATFORM)
IMAGES = os.path.join(RT, "app/images")
ENGINE = os.path.join(RT, "bin/collabo-core-engine")
DIRS = {"build-tools": os.environ.get("BT_DIR") or os.path.join(A, "out"),
        "python-extension": os.environ.get("PYEXT_DIR") or os.path.join(A, "../python-extension/out"),
        "llvm": os.environ.get("LLVM_DIR") or os.path.join(A, "../llvm/out")}
UPSTREAM = "--upstream" in sys.argv[1:]

fails = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else "\n     " + str(detail)[-3000:]), flush=True)
    if not ok:
        fails.append(name)


def have(name):
    return os.path.exists(os.path.join(DIRS[name], name + ".cpio"))


def addon_dir():
    """One --addon-dir with each add-on there is: its image and description."""
    d = tempfile.mkdtemp(prefix="build-tools-addons-")
    for name, src in DIRS.items():
        if not have(name):
            continue
        os.symlink(os.path.abspath(os.path.join(src, name + ".cpio")), os.path.join(d, name + ".cpio"))
        desc = os.path.join(src, name + ".json")
        if not os.path.exists(desc):
            desc = os.path.join(A, "addon.json") if name == "build-tools" else os.path.join(A, "..", name, "addon.json")
        shutil.copyfile(desc, os.path.join(d, name + ".json"))
    return d


ADDONS = addon_dir()


def guest(work, command, addons=("build-tools",), python=False, tools=False, mounts=(), timeout=600, loopback=False):
    """Runs `command` (sh) in a fresh guest with the add-ons, `work` at /work (the cwd); returns
    (status, output). `loopback`: the network, to this machine's loopback only (192.0.2.1 there)."""
    argv = [ENGINE, "exec", "--kernel", f"{IMAGES}/vmlinux.wasm", "--initramfs", f"{IMAGES}/initramfs.cpio",
            "--addon-dir", ADDONS, "--mount", f"{work}:/work", "--cwd", "/work"]
    argv += ["--allow-loopback"] if loopback else ["--no-network"]
    if python:
        argv += ["--python-image", f"{IMAGES}/python.cpio"]
    if tools:
        argv += ["--tools-image", f"{IMAGES}/tools.cpio"]
    for name in addons:
        argv += ["--addon", name]
    for m in mounts:
        argv += ["--mount", m]
    argv += ["--", "/bin/sh", "-c", command]
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as e:
        return -1, f"timed out after {timeout} s: {e.stdout or ''}"
    # the command's output is stdout; stderr is the kernel's boot messages, then the command's
    err = p.stderr
    mark = err.find("init: starting /bin/sh")
    if mark >= 0:
        err = err[err.find("\n", mark) + 1:]
    return p.returncode, p.stdout + err


def write(work, files):
    for name, text in files.items():
        path = os.path.join(work, name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(text)
        os.chmod(path, 0o755 if name.endswith(".sh") else 0o644)


# Builds run in /work, the shared folder: its times are the host's, which make and ninja go by
# (touch there sets them: engine/src/fs.rs OP_SETATTR; the guest's clock is the host's to the
# microsecond: kernel/patches/wasm-wall-clock-sync.patch, so make sees nothing "in the future").
IN_WORK = "cd /work && "


def versions(work):
    rc, out = guest(work, "make --version | head -1; gmake --version | head -1; ninja --version; pkgconf --version; "
                          "pkg-config --version; ls -l /usr/bin/gmake /usr/bin/pkg-config")
    check("make 4.4.1, ninja 1.13.2, pkgconf 3.0.10 (gmake, pkg-config)", rc == 0
          and out.count("GNU Make 4.4.1") == 2 and "\n1.13.2\n" in out and out.count("3.0.10") == 2
          and "gmake -> make" in out and "pkg-config -> pkgconf" in out, out)


def make_checks(work):
    write(work, {
        "Makefile": """NOW := $(shell echo shell-ran)
JOBS := j1 j2 j3 j4
all: $(JOBS) sub
\t@echo "all: $(NOW) $(GEN)"
j%:
\t@sleep 1; echo $@ > $@
sub:
\t@$(MAKE) -s -C sub
.PHONY: all sub
include gen.mk
gen.mk:
\techo 'GEN := generated' > $@
""",
        "sub/Makefile": "all:\n\t@echo \"sub: level $(MAKELEVEL) flags $(MAKEFLAGS)\"\n",
        "sync.mk": "all: a b\na:\n\t@echo a1; sleep 1; echo a2\nb:\n\t@sleep 0.5; echo b1; sleep 1; echo b2\n",
        "script.mk": "all:\n\t./noshebang.sh one two\n",
        "noshebang.sh": "echo \"script got $1 $2\"\n",
        "fail.mk": "all:\n\t@echo before; false\n\t@echo never\n",
        "stamp.mk": "out: in\n\t@cp in out; echo rebuilt\n",
    })
    rc, out = guest(work, IN_WORK + "s=$(date +%s); make -j4; echo \"rc $? secs $(( $(date +%s) - s ))\"; make -q j1; echo \"q $?\"")
    secs = re.search(r"rc 0 secs (\d+)", out)
    check("make -j4: four 1 s jobs at once, the makefile remade and read again", secs is not None
          and int(secs.group(1)) <= 3 and "echo 'GEN := generated' > gen.mk" in out
          and "all: shell-ran generated" in out and "q 0" in out, out)
    check("make: recursive $(MAKE) with the jobserver (a fifo)", re.search(r"sub: level 1 flags .*-j4 .*--jobserver-auth=fifo:", out)
          is not None, out)
    rc, out = guest(work, IN_WORK + "make -j2 -Otarget -f sync.mk")
    check("make -Otarget: each target's output together", rc == 0 and re.search(r"a1\na2\nb1\nb2|b1\nb2\na1\na2", out) is not None, out)
    rc, out = guest(work, IN_WORK + "make -f script.mk")
    check("make: a recipe that runs a script without #! (spawned, then by the shell)", rc == 0 and "script got one two" in out, out)
    rc, out = guest(work, IN_WORK + "make -f fail.mk; echo \"rc $?\"")
    check("make: a failing recipe stops it, status 2", "before" in out and "never" not in out
          and re.search(r"fail\.mk:2: all\] Error 1", out) is not None and "rc 2" in out, out)
    rc, out = guest(work, IN_WORK + "make -v > /dev/full; echo \"rc $?\"")
    check("make: a write error on stdout at exit: status 1, no trap (patches/make/0002)", "write error: stdout" in out
          and "rc 1" in out and "trapped" not in out, out)
    rc, out = guest(work, IN_WORK + "echo 1 > in; make -f stamp.mk; make -f stamp.mk; sleep 1; touch in; make -f stamp.mk")
    check("make: up to date, then rebuilt after touch", out.count("rebuilt") == 2 and "'out' is up to date" in out, out)
    rc, out = guest(work, IN_WORK + "for i in 1 2 3 4 5 6 7 8; do rm -f gen.mk j1 j2 j3 j4; make -j4 2>&1; done")
    check("make on the shared folder: no clock skew, nothing in the future", out.count("all: shell-ran generated") == 8
          and "skew" not in out and "in the future" not in out, out)


def ninja_checks(work):
    write(work, {
        "build.ninja": """rule cat
  command = cat $in > $out && echo "$out: $in $hdr" > $out.d
  depfile = $out.d
  deps = gcc
rule slow
  command = sleep 1 && echo $out > $out
  pool = one
pool one
  depth = 1
rule fast
  command = sleep 1 && echo $out > $out
rule regen
  command = sh gen.sh > $out
  generator = 1
rule fail
  command = echo failing; false
build build.ninja: regen gen.sh
build x.txt: cat in.txt
  hdr = dep.h
build s1: slow
build s2: slow
build f1: fast
build f2: fast
build f3: fast
build serial: phony s1 s2
build parallel: phony f1 f2 f3
build bad: fail
default x.txt
""",
        "in.txt": "hello\n",
        "dep.h": "1\n",
        "jobs.mk": "all:\n\t+ninja -j8 parallel\n",
    })
    with open(os.path.join(work, "build.ninja")) as f:
        manifest = f.read()
    write(work, {"gen.sh": "cat <<'EOF'\n" + manifest + "EOF\necho 'build extra: phony x.txt'\n"})
    rc, out = guest(work, IN_WORK + """
        ninja; echo "rc $?"; ninja; ninja -t deps x.txt
        sleep 1; touch dep.h; ninja -d explain x.txt 2>&1 | grep -o 'most recent input dep.h'
        s=$(date +%s); ninja serial >/dev/null; echo "serial $(( $(date +%s) - s ))"
        s=$(date +%s); ninja -j3 parallel >/dev/null; echo "parallel $(( $(date +%s) - s ))"
        ninja -t targets all | grep -c ': '; ninja -t compdb cat | grep -c '"command"'
        ninja -n -t clean | head -1; ninja bad; echo "bad rc $?"
        sleep 1; touch gen.sh; ninja extra; grep -c 'build extra' build.ninja
        rm -f f1 f2 f3; s=$(date +%s); make -j3 -f jobs.mk >/dev/null; echo "under make -j3: $(( $(date +%s) - s ))"
    """)
    check("ninja: builds, then nothing to do", "rc 0" in out and "ninja: no work to do." in out, out)
    check("ninja: deps from a depfile (deps = gcc), rebuilt after the header changed",
          re.search(r"x\.txt: #deps 2, deps mtime \d+ \(VALID\)\n\s+in\.txt\n\s+dep\.h", out) is not None
          and "most recent input dep.h" in out, out)
    serial = re.search(r"serial (\d+)", out)
    parallel = re.search(r"parallel (\d+)", out)
    check("ninja: a pool of depth 1 runs one at a time, -j3 three at once", serial is not None and parallel is not None
          and int(serial.group(1)) >= 2 and int(parallel.group(1)) <= 2, out)
    check("ninja -t targets, -t compdb, -t clean", re.search(r"\n1[0-9]\n1\n", out) is not None
          and "Cleaning..." in out, out)
    check("ninja: a failing command, status 1", "FAILED: [code=1] bad" in out and "failing" in out and "bad rc 1" in out, out)
    check("ninja: the manifest regenerated (generator rule) and read again", re.search(r"\n1\n", out.split("bad rc 1")[-1])
          is not None and "under make" in out, out)
    under = re.search(r"under make -j3: (\d+)", out)
    check("ninja under make -j3: a jobserver client, three 1 s jobs at once", under is not None and int(under.group(1)) <= 2, out)


def pkgconf_checks(work):
    write(work, {
        "pc/foo.pc": """prefix=/opt/foo
libdir=${prefix}/lib
includedir=${prefix}/include
answer=42
Name: foo
Description: test library
Version: 1.4.2
Requires: bar >= 2.0
Requires.private: baz
Libs: -L${libdir} -lfoo
Libs.private: -lm
Cflags: -I${includedir} -DFOO=1
""",
        "pc/bar.pc": "Name: bar\nDescription: b\nVersion: 2.1\nLibs: -L/usr/lib -lbar\nCflags: -I/usr/include -I/opt/bar/include\n",
        "pc/baz.pc": "Name: baz\nDescription: z\nVersion: 0.9\nLibs: -lbaz\n",
    })
    rc, out = guest(work, """
        export PKG_CONFIG_PATH=/work/pc
        pkg-config --modversion foo; pkg-config --cflags foo; pkg-config --libs foo; pkg-config --static --libs foo
        pkg-config --variable=answer foo; pkg-config --exists 'foo >= 1.4' && echo ge-ok
        pkg-config --exists 'foo >= 2' || echo lt-ok; pkg-config --atleast-version=1.4.2 foo && echo atleast-ok
        pkg-config --print-errors --exists nosuch 2>&1; echo "missing rc $?"
        pkg-config --variable=pc_path pkg-config
        PKG_CONFIG_ALLOW_SYSTEM_CFLAGS=1 pkg-config --cflags bar
    """)
    lines = out.splitlines()
    check("pkg-config: version, cflags, libs (with Requires; /usr/include and -L/usr/lib left out)",
          lines[:3] == ["1.4.2", "-I/opt/foo/include -DFOO=1 -I/opt/bar/include", "-L/opt/foo/lib -lfoo -lbar"], out)
    check("pkg-config --static: Libs.private and Requires.private", len(lines) > 3
          and lines[3] == "-L/opt/foo/lib -lfoo -lm -lbar -lbaz", out)
    check("pkg-config: variables and version constraints", "42" in lines and "ge-ok" in out and "lt-ok" in out
          and "atleast-ok" in out, out)
    check("pkg-config: a missing package", "Package 'nosuch' not found" in out or "nosuch" in out and "missing rc 1" in out, out)
    check("pkg-config: the guest's search path", "/usr/lib/pkgconfig:/usr/share/pkgconfig" in out, out)
    check("pkg-config: PKG_CONFIG_ALLOW_SYSTEM_CFLAGS keeps -I/usr/include", "-I/usr/include -I/opt/bar/include" in out, out)


def guest_tools_checks(work):
    """patchelf for WebAssembly files (meson-python's run path fixes), config.site (configure)."""
    rc, out = guest(work, """patchelf --version; patchelf --print-rpath /usr/bin/cmake; echo "rpath rc $?"
        patchelf --set-rpath '$ORIGIN/lib' /usr/bin/ninja; echo "set rc $?"
        printf 'not wasm' > /tmp/x; patchelf --print-rpath /tmp/x; echo "other rc $?"
        ls -l /usr/local/share/config.site; (. /usr/share/config.site; echo "build_alias=$build_alias fork=$ac_cv_func_fork")""")
    check("patchelf: WebAssembly files have no run path (printed empty, set as a no-op); others refused",
          out.startswith("patchelf 0.18.0") and "rpath rc 0" in out and "set rc 0" in out
          and "not a WebAssembly file" in out and "other rc 1" in out, out)
    check("config.site: the build machine configure cannot guess (wasm32-unknown-linux-musl), no fork",
          "/usr/local/share/config.site -> /usr/share/config.site" in out
          and "build_alias=wasm32-unknown-linux-musl fork=no" in out, out)


def serve(directory):
    """An HTTP server on this machine's loopback, for the guest (192.0.2.1 is the host there)."""
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *args):
            pass
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=directory))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server.server_address[1]


def cmake_checks(work):
    write(work, {
        "cm/CMakeLists.txt": """cmake_minimum_required(VERSION 3.20)
project(demo NONE)
execute_process(COMMAND sh -c "echo out; echo err >&2; exit 3"
                OUTPUT_VARIABLE out ERROR_VARIABLE err RESULT_VARIABLE rc)
message(STATUS "exec: [${out}] [${err}] ${rc}")
execute_process(COMMAND pwd WORKING_DIRECTORY /usr/bin OUTPUT_VARIABLE cwd OUTPUT_STRIP_TRAILING_WHITESPACE)
message(STATUS "cwd: ${cwd}")
execute_process(COMMAND echo hello pipeline COMMAND tr a-z A-Z COMMAND rev OUTPUT_VARIABLE piped
                OUTPUT_STRIP_TRAILING_WHITESPACE)
message(STATUS "pipe: ${piped}")
execute_process(COMMAND no-such-program RESULT_VARIABLE missing)
message(STATUS "missing: ${missing}")
execute_process(COMMAND sleep 30 TIMEOUT 1 RESULT_VARIABLE slow)
message(STATUS "timeout: ${slow}")
execute_process(COMMAND sh -c "echo $FOO" OUTPUT_VARIABLE env OUTPUT_STRIP_TRAILING_WHITESPACE)
message(STATUS "env: ${env}")
add_custom_command(OUTPUT out.txt COMMAND ${CMAKE_COMMAND} -E copy ${CMAKE_SOURCE_DIR}/in.txt out.txt
                   COMMAND ${CMAKE_COMMAND} -E echo copied DEPENDS ${CMAKE_SOURCE_DIR}/in.txt)
add_custom_target(copy ALL DEPENDS out.txt)
enable_testing()
add_test(NAME passes COMMAND sh -c "exit 0")
add_test(NAME fails COMMAND sh -c "exit 1")
set_tests_properties(fails PROPERTIES WILL_FAIL TRUE)
""",
        "cm/in.txt": "one\n",
    })
    rc, out = guest(work, "cmake --version | head -1; cmake -E capabilities")
    caps = re.search(r"^\{.*\}$", out, re.M)
    gens = [g["name"] for g in json.loads(caps.group(0))["generators"]] if caps else []
    check("cmake 4.4.5, its generators (Ninja, Unix Makefiles)", "cmake version 4.4.5" in out
          and "Ninja" in gens and "Unix Makefiles" in gens, out)
    rc, out = guest(work, """cd cm
        FOO=from-env cmake -S . -B b -G Ninja 2>&1; echo "configure rc $?"
        cmake --build b; echo "build rc $?"; cmake --build b | tail -1
        sleep 1; echo two > in.txt; cmake --build b | grep -c copied; cat b/out.txt
        cd b && ctest 2>&1 | tail -3
    """)
    check("cmake: execute_process's output, errors and status (libuv's uv_spawn: posix_spawn)",
          "-- exec: [out\n] [err\n] 3" in out and "configure rc 0" in out, out)
    check("cmake: execute_process in a WORKING_DIRECTORY, a pipeline of three, the environment",
          "-- cwd: /usr/bin" in out and "-- pipe: ENILEPIP OLLEH" in out and "-- env: from-env" in out, out)
    check("cmake: a missing program, a TIMEOUT", "-- missing: no such file or directory" in out.lower()
          and "-- timeout: Process terminated due to timeout" in out, out)
    check("cmake -G Ninja, cmake --build: built, up to date, rebuilt after its input changed", "build rc 0" in out
          and "ninja: no work to do." in out and re.search(r"\n1\ntwo\n", out) is not None, out)
    check("ctest: a test that passes, one that fails as it should", "100% tests passed out of 2" in out, out)
    rc, out = guest(work, """cd cm && rm -rf m && cmake -S . -B m -G 'Unix Makefiles' >/dev/null && cmake --build m -j2
        echo "rc $?"; cmake --build m | grep -c copied""")
    check("cmake -G 'Unix Makefiles' with make", "rc 0" in out and "Built target copy" in out
          and "rc 0\n0\n" in out, out)

    www = tempfile.mkdtemp(prefix="build-tools-www-")
    payload = os.urandom(200000)
    with open(os.path.join(www, "blob.bin"), "wb") as f:
        f.write(payload)
    port = serve(www)
    digest = hashlib.sha256(payload).hexdigest()
    write(work, {"t/get.cmake": f"""file(DOWNLOAD http://192.0.2.1:{port}/blob.bin blob.bin EXPECTED_HASH SHA256={digest} STATUS st)
message("download: ${{st}}")
file(SIZE blob.bin n)
message("size: ${{n}}")
"""})
    rc, out = guest(work, """cd t
        printf 'a\\n' > a.txt; mkdir -p d; printf 'b\\n' > d/b.txt
        cmake -E tar czf x.tar.gz a.txt d && rm -rf a.txt d && cmake -E tar xzf x.tar.gz && cat a.txt d/b.txt
        cmake -E tar cJf x.tar.xz a.txt && cmake -E tar tf x.tar.xz
        cmake -E tar cjf x.tar.bz2 a.txt && cmake -E tar tf x.tar.bz2
        cmake -E tar cf x.tar.zst --zstd a.txt && cmake -E tar tf x.tar.zst
        cmake -E sha256sum a.txt; cmake -E env A=1 B=2 sh -c 'echo $A$B'
        cmake -E make_directory p/q && cmake -E touch p/q/r && ls p/q
        cmake -P get.cmake
    """, loopback=True)
    check("cmake -E tar: gzip, xz, bzip2 and zstd archives made and read back (libarchive)", "a\nb\n" in out
          and out.count("a.txt\n") >= 4, out)
    check("cmake -E sha256sum, env, make_directory, touch",
          "87428fc522803d31065e7bce3cf03fe475096631e5e07bbd7a0fde60c4cf25c7  a.txt" in out
          and "\n12\n" in out and "\nr\n" in out, out)
    check("cmake file(DOWNLOAD) over HTTP, its hash checked (curl)", 'download: 0;"No error"' in out
          and "size: 200000" in out, out)
    shutil.rmtree(www, ignore_errors=True)


def python_checks(work):
    """With python-extension: the Python headers where sysconfig and pkg-config say they are."""
    rc, out = guest(work, """
        pkg-config --cflags python3; pkg-config --modversion python3; pkg-config --libs python3
        python3 -c 'import sysconfig; print(sysconfig.get_paths()["include"]); print(sysconfig.get_config_var("EXT_SUFFIX"))'
        ls /usr/include/python3.13/Python.h /usr/include/python3.13/pyconfig.h
        python3 -c 'import numpy; print(numpy.get_include())'; ls $(python3 -c 'import numpy; print(numpy.get_include())')/numpy/arrayobject.h
        echo "pc build paths: $(grep -c 'collaboCore\\|/opt/' /usr/lib/pkgconfig/python-3.13.pc)"
    """, addons=("build-tools", "python-extension"), python=True)
    check("python-extension: pkg-config python3 gives -I/usr/include/python3.13, version 3.13", "-I/usr/include/python3.13" in out
          and "\n3.13\n" in out, out)
    check("python-extension: Python.h and pyconfig.h where sysconfig says, EXT_SUFFIX .cpython-313.so",
          "/usr/include/python3.13\n.cpython-313.so" in out and "No such file" not in out, out)
    check("python-extension: numpy's headers (numpy.get_include())", "numpy/arrayobject.h" in out and "No such file" not in out, out)
    check("python-extension: python-3.13.pc names no build machine's path", "pc build paths: 0" in out, out)


def llvm_checks(work):
    """With llvm: make and ninja drive clang (depfiles from -MD) and wasm-ld."""
    write(work, {
        "c/sq.h": "int sq(int);\n",
        "c/sq.c": '#include "sq.h"\nint sq(int x) { return x * x; }\n',
        "c/main.c": '#include "sq.h"\nint area(int a) { return sq(a) + 1; }\n',
        "c/Makefile": """CFLAGS = --target=wasm32 -O2 -nostdlib -MMD -MP
lib.wasm: sq.o main.o
\twasm-ld --no-entry --export=area --export=sq -o $@ $^
%.o: %.c
\tclang $(CFLAGS) -c $< -o $@
-include sq.d main.d
""",
        "c/build.ninja": """rule cc
  command = clang --target=wasm32 -O2 -nostdlib -MD -MF $out.d -c $in -o $out
  depfile = $out.d
  deps = gcc
rule ld
  command = wasm-ld --no-entry --export=area -o $out $in
build sq.o: cc sq.c
build main.o: cc main.c
build lib.wasm: ld sq.o main.o
""",
    })
    rc, out = guest(work, IN_WORK + """cd c
        make -j2 && llvm-nm lib.wasm | grep -E ' T (area|sq)$'; make; sleep 1; touch sq.h; make | grep -c clang
        make clean 2>/dev/null; rm -f *.o *.d lib.wasm
        ninja && ninja -t deps main.o | grep -c sq.h; sleep 1; touch sq.h; ninja -d explain 2>&1 | grep -c 'clang'
    """, addons=("build-tools", "llvm"))
    check("llvm: make -j2 builds a wasm library with clang and wasm-ld, -MMD depfiles", " T area" in out
          and " T sq" in out and "'lib.wasm' is up to date" in out and re.search(r"up to date\.\n2\n", out) is not None, out)
    check("llvm: ninja with clang's depfiles (deps = gcc), rebuilt after the header changed",
          re.search(r"lib\.wasm sq\.o main\.o\n1\n2\n", out) is not None, out)

    # a CMake project in C and C++ with the guest's cc and c++: the compilers found and checked,
    # try_compile's checks, a static library (ar), a shared one loaded with dlopen (its exception
    # caught in it), a program that exports to it (ENABLE_EXPORTS: -rdynamic), threads; no run
    # path (patches/cmake/0005), so Ninja installs it as it is
    write(work, {
        "cx/CMakeLists.txt": """cmake_minimum_required(VERSION 3.20)
project(demo C CXX)
set(CMAKE_CXX_STANDARD 20)
find_package(Threads REQUIRED)
include(CheckSymbolExists)
include(CheckCXXSourceCompiles)
check_symbol_exists(posix_spawn "spawn.h" HAVE_POSIX_SPAWN)
check_cxx_source_compiles("#include <format>\\nint main() { return std::format(\\"{}\\", 1).size() - 1; }" HAVE_STD_FORMAT)
include(CheckTypeSize)
check_type_size("long" SIZEOF_LONG)
message(STATUS "checks: posix_spawn=${HAVE_POSIX_SPAWN} format=${HAVE_STD_FORMAT} long=${SIZEOF_LONG}")
add_library(sq STATIC sq.c)
add_library(plug SHARED plug.cpp)
add_executable(app app.cpp)
target_link_libraries(app PRIVATE sq Threads::Threads ${CMAKE_DL_LIBS})
set_target_properties(app PROPERTIES ENABLE_EXPORTS ON)
target_compile_definitions(app PRIVATE PLUG="$<TARGET_FILE_NAME:plug>")
enable_testing()
add_test(NAME runs COMMAND app WORKING_DIRECTORY $<TARGET_FILE_DIR:plug>)
set_tests_properties(runs PROPERTIES PASS_REGULAR_EXPRESSION "sq=49 thread=7 plug=from-plugin caught=boom")
install(TARGETS app sq plug)
""",
        "cx/sq.c": "int sq(int x) { return x * x; }\n",
        "cx/plug.cpp": """#include <stdexcept>
#include <string>
extern "C" const char *plug_name(void) {
  static std::string s;
  try { throw std::runtime_error("from-plugin"); } catch (const std::exception &e) { s = e.what(); }
  return s.c_str();
}
""",
        "cx/app.cpp": """#include <cstdio>
#include <dlfcn.h>
#include <stdexcept>
#include <string>
#include <thread>
extern "C" int sq(int);
int main(int, char **argv) {
  int t = 0;
  std::thread th([&] { t = 7; });
  th.join();
  std::string dir = argv[0];
  dir = dir.substr(0, dir.rfind('/') + 1);
  void *h = dlopen((dir + PLUG).c_str(), RTLD_NOW);
  if (!h) h = dlopen((dir + "../lib/" + PLUG).c_str(), RTLD_NOW);
  const char *name = "none";
  if (h) { auto f = (const char *(*)(void))dlsym(h, "plug_name"); if (f) name = f(); }
  else std::printf("dlopen: %s\\n", dlerror());
  const char *caught = "no";
  try { throw std::logic_error("boom"); } catch (const std::exception &e) { caught = e.what(); }
  std::printf("sq=%d thread=%d plug=%s caught=%s\\n", sq(7), t, name, caught);
}
""",
    })
    rc, out = guest(work, IN_WORK + """cd cx && rm -rf b m inst
        cmake -S . -B b -G Ninja -DCMAKE_BUILD_TYPE=Release 2>&1 | grep -E 'compiler identification|checks:|Error'
        cmake --build b > /dev/null; echo "ninja build rc $?"; ./b/app; (cd b && ctest 2>&1 | grep 'tests passed')
        DESTDIR=$PWD/inst cmake --install b > /dev/null; echo "install rc $?"; ./inst/usr/local/bin/app
        cmake -S . -B m -G 'Unix Makefiles' > /dev/null && cmake --build m -j2 > /dev/null; echo "make build rc $?"; ./m/app
    """, addons=("build-tools", "llvm"))
    check("llvm + cmake: C and C++ compilers (Clang 24) and try_compile's checks", out.count("identification is Clang 24") == 2
          and "checks: posix_spawn=1 format=1 long=4" in out and "Error" not in out, out)
    check("llvm + cmake -G Ninja: a static and a shared library, a program that loads one (exceptions, threads), "
          "ctest, install", "ninja build rc 0" in out and out.count("sq=49 thread=7 plug=from-plugin caught=boom") == 3
          and "100% tests passed" in out and "install rc 0" in out, out)
    check("llvm + cmake -G 'Unix Makefiles' with make", "make build rc 0\nsq=49 thread=7 plug=from-plugin caught=boom" in out, out)


KNOWN = {}   # tests/upstream-known.txt: make's scripts (with how many tests), CMake's unit tests ("-")
for _line in open(os.path.join(T, "upstream-known.txt")):
    _line = _line.split("#")[0].strip()
    if _line:
        KNOWN[_line.split()[0]] = _line.split()[1]


def upstream(work):
    tests = os.path.join(A, "build/tests")
    if not os.path.exists(os.path.join(tests, "ninja_test")):
        check("upstream: build/tests (TEST=1 addons/build-tools/build.sh)", False, "missing " + tests)
        return
    ro = [f"{tests}:/t:ro", f"{A}/build/src/pkgconf:/pkgconf:ro", f"{A}/build/src/make:/make:ro"]
    rc, out = guest(work, "mkdir /tmp/n && cd /tmp/n && /t/ninja_test 2>&1 | tail -3", mounts=ro)
    m = re.search(r"\[  PASSED  \] (\d+) tests", out)
    check(f"upstream: ninja_test ({m.group(1) if m else '?'} tests)", m is not None and "FAILED" not in out, out)

    rc, out = guest(work, """cp -a /pkgconf /tmp/p && cd /tmp/p
        for d in t/*/; do /t/pkgconf/test-runner --test-fixtures=/tmp/p/tests --tool-dir=/t/pkgconf $d > /tmp/o 2>&1
          echo "$d rc=$? pass=$(grep -c ^PASS /tmp/o) fail=$(grep -c ^FAIL /tmp/o) skip=$(grep -c ^SKIP /tmp/o)"; grep ^FAIL /tmp/o; done
        for p in /t/pkgconf/test-api-*; do $p > /tmp/o 2>&1 || { echo "$p rc=$?"; tail -3 /tmp/o; }; echo "api $(grep -c ': PASS' /tmp/o)"; done
    """, mounts=ro)
    runs = re.findall(r"rc=(\d+) pass=(\d+) fail=(\d+) skip=(\d+)", out)
    api = [int(n) for n in re.findall(r"^api (\d+)$", out, re.M)]
    passed = sum(int(p) for _, p, _, _ in runs)
    check(f"upstream: pkgconf's test-runner, {len(runs)} groups ({passed} tests) and {len(api)} API test programs "
          f"({sum(api)} tests)", len(runs) >= 14 and all(rc == "0" and f == "0" for rc, _, f, _ in runs)
          and len(api) >= 16 and all(api) and "rc=" not in out.split("api")[-1], out)

    # CMake's own unit tests (Tests/CMakeLib, KWSys'), by the guest's ctest in the build tree,
    # mounted where it was built (its CTestTestfile.cmake names absolute paths). Not KWSys'
    # DynamicLoader test: the programs here are static and load no libraries.
    cb, cs = os.path.join(A, "build/cmake"), os.path.join(A, "build/src/cmake")
    if os.path.exists(os.path.join(cb, "Tests/CMakeLib/CTestTestfile.cmake")):
        rc, out = guest(work, f"""ctest --test-dir {cb} -R '^(CMakeLib|kwsys)\\.' -E testDynamicLoader -j2 --output-on-failure > /tmp/ct.log 2>&1
            grep -E 'Test +#[0-9]+: +(CMakeLib.testUVProcessChain|kwsys.testProcess-[0-9]+) ' /tmp/ct.log; tail -40 /tmp/ct.log""",
                        mounts=[f"{cb}:{cb}", f"{cs}:{cs}:ro"], timeout=1800)
        m = re.search(r"tests passed(?:, (\d+) tests? failed)? out of (\d+)", out)
        spawns = sorted(set(re.findall(r"Test +#\d+: +(CMakeLib\.testUVProcessChain|kwsys\.testProcess-\d+) \.+ +Passed", out)))
        failed = re.findall(r"^\s+\d+ - (\S+) \(", out.split("The following tests FAILED:")[-1], re.M) \
            if "The following tests FAILED:" in out else []
        unexpected = [n for n in failed if KNOWN.get(n) != "-"]
        total = int(m.group(2)) if m else 0
        check(f"upstream: CMake's CMakeLib and KWSys unit tests ({total - len(failed)} of {total} pass, "
              f"testUVProcessChain and {sum(n.startswith('kwsys') for n in spawns)} kwsys.testProcess-* among them; failing only as known: "
              f"{', '.join(failed) or 'none'})", m is not None and total > 60 and not unexpected
              and "CMakeLib.testUVProcessChain" in spawns, f"unexpected: {unexpected}\n{out}")
    else:
        check("upstream: CMake's unit tests (TEST=1 addons/build-tools/build.sh builds them)", False, "missing " + cb)

    # GNU make's suite: perl's test driver, its tests copied where they can write
    rc, out = guest(work, """cp -a /make/tests /tmp/tests && cp /t/make-config-flags.pm /tmp/tests/config-flags.pm && cd /tmp/tests
        perl ./run_make_tests.pl -srcdir /make -make /usr/bin/make > /tmp/log 2>&1; cat /tmp/log
        for f in $(find work -name '*.diff*'); do echo "## $f"; head -20 $f; done
    """, tools=True, mounts=ro, timeout=1800)
    known = {name: int(count) for name, count in KNOWN.items() if "/" in name}
    # the log: one block per script, "category/name ..... " then its errors, ok or FAILED (n/m passed)
    blocks = re.split(r"^(?=[a-z_]+/[A-Za-z_0-9-]+ \.\.\.)", out.split("## ")[0], flags=re.M)
    passed, failed = 0, {}
    for block in blocks:
        m = re.match(r"([a-z_]+/[A-Za-z_0-9-]+) \.+", block)
        if not m:
            continue
        ok = re.search(r"ok\s+\((\d+) passed\)", block)
        bad = re.search(r"FAILED \((\d+)/(\d+) passed\)", block)
        if bad:
            failed[m.group(1)] = int(bad.group(2)) - int(bad.group(1))
        elif ok:
            passed += 1
        elif "N/A" not in block:
            failed[m.group(1)] = -1   # died
    unexpected = {n: c for n, c in failed.items() if n not in known or c < 0 or c > known[n]}
    check(f"upstream: GNU make's run_make_tests.pl ({passed} scripts pass; failing only as known: "
          f"{', '.join(f'{n} {c}' for n, c in sorted(failed.items())) or 'none'})", passed > 90 and not unexpected
          and "trapped" not in out, f"unexpected: {unexpected}\n{out}")


def main():
    for f in (ENGINE, f"{IMAGES}/vmlinux.wasm", os.path.join(DIRS["build-tools"], "build-tools.cpio")):
        if not os.path.exists(f):
            print(f"missing {f}", file=sys.stderr)
            return 2
    work = tempfile.mkdtemp(prefix="build-tools-test-")
    os.chmod(work, 0o777)
    versions(work)
    make_checks(work)
    ninja_checks(work)
    pkgconf_checks(work)
    cmake_checks(work)
    guest_tools_checks(work)
    if have("python-extension"):
        python_checks(work)
    else:
        print("SKIP python-extension: no " + os.path.join(DIRS["python-extension"], "python-extension.cpio"))
    if have("llvm"):
        llvm_checks(work)
    else:
        print("SKIP llvm: no " + os.path.join(DIRS["llvm"], "llvm.cpio"))
    if UPSTREAM:
        shutil.copyfile(os.path.join(A, "build/make/tests/config-flags.pm"), os.path.join(A, "build/tests/make-config-flags.pm"))
        upstream(work)
    shutil.rmtree(work, ignore_errors=True)
    shutil.rmtree(ADDONS, ignore_errors=True)
    print(f"{len(fails)} failed" if fails else "all passed")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
