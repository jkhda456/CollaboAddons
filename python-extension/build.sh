#!/usr/bin/env bash
# The python-extension add-on: Python packages with compiled extension modules, built for the guest
# as shared libraries its CPython loads with dlopen (WebAssembly side modules, which the C
# library's dlopen and the engine's dylink imports load: addons/mod), their Python halves beside
# them in site-packages — numpy, Pillow, LIEF, lxml, cffi (and pycparser), cryptography, rpds-py and
# unicorn so far, and mcp (the MCP SDK, pure Python, with what it needs); one folder each in packages/ (packages/README.md: what a package's build.sh does, and how to add
# one). Packed as out/python-extension.cpio, an overlay over python.cpio: the packages in
# /usr/lib/python3.13/site-packages, their console scripts in /usr/bin, and wheels of some of them
# (cffi's versions, pycparser, unicorn's versions) in /usr/share/python-wheels, where /etc/pip.conf
# has pip look for what a build's isolated environment or a package's pin needs, etc/collabo-config-site.pth (CONFIG_SITE for the
# configure scripts Python's builds run), and the interpreter's _ctypes, which the python step
# leaves out, in /usr/lib/python3.13/lib-dynload. README.md has the details.
#
# Needs the userspace step (musl with dlopen, compiler-rt, wasm-cc -shared: userspace/), the python
# step (an interpreter linked for dynamic loading, its headers in python/stage, the host CPython
# python/host), the guest's Rust toolchain (python/build-rust-toolchain.sh: its wasm-ld, and rustc
# for the packages in Rust), cmake and ninja (./build.sh tools). The modules link against the
# interpreter's exports only (the Python API and the C library): a CPython 3.13 that loads
# extension modules runs them, rebuilt or not.
#
#   addons/python-extension/build.sh [PACKAGE...]   the packages to put in (default: all, in PACKAGES order)
#   FORCE=stage[,stage]    redo those stages: libcxx host rust-std <package> <package>-src
#                          fpcast-<package> install ctypes wheels dev cpio (the other versions'
#                          of cffi and unicorn: cffi-1.17.1 ..., unicorn-1.0.3 unicorn-1.0.3-src ...)
#   JOBS=N
set -euo pipefail
PYEXT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$PYEXT/../.." && pwd)"
U="$ROOT/userspace"
P="$ROOT/python"
B="$PYEXT/build"
DL="$B/dl"
JOBS="${JOBS:-$(nproc)}"
FORCE=",${FORCE:-},"
CMAKE="$ROOT/.tools/cmake/bin/cmake"
NINJA="$ROOT/.tools/cmake/bin/ninja"
HOSTPY="$P/host/bin/python3.13"
export WASM_LD="$ROOT/.tools/rust-wasm/bin/wasm-ld"
export PATH="$ROOT/.tools/cmake/bin:$U/tools:$PATH"
export PYEXT ROOT B DL JOBS FORCE CMAKE NINJA HOSTPY
# shellcheck source=tools/lib.sh
source "$PYEXT/tools/lib.sh"

PACKAGES=(numpy pillow lief lxml pycparser cffi cryptography rpds-py mcp unicorn)
RUST=(cryptography rpds-py)   # those with Rust in them (and so Rust's standard library: its license)
# those carried as wheels too, for pip in the guest (stage wheels): cffi and unicorn, whose sdists
# from PyPI cannot build here (each with versions of its earlier minor lines, for what pins them),
# and pycparser, which cffi needs (so pip has both without the network)
WHEELS=(pycparser cffi unicorn)
[[ $# -gt 0 ]] && PACKAGES=("$@")
for name in "${PACKAGES[@]}"; do
  [[ -f "$PYEXT/packages/$name/build.sh" ]] || { echo "no package $name (packages/$name/build.sh)" >&2; exit 1; }
done

chmod +x "$U/bin/wasm-cc" "$PYEXT/tools/wasm-pyext-cc" "$PYEXT/tools/wasm-pyext-c++"
for f in "$U/sysroot/lib/libc.a" "$U/sysroot/lib/libc.exports" "$P/stage/usr/include/python3.13/pyconfig.h" \
         "$P/build-wasm/Makefile" "$HOSTPY" "$WASM_LD" "$CMAKE" "$NINJA"; do
  [[ -e "$f" ]] || { echo "missing $f: build the userspace and python steps first (./build.sh tools userspace python)" >&2; exit 1; }
done
grep -q '^DYNLOADFILE=.*dynload_shlib' "$P/build-wasm/Makefile" \
  || { echo "the python step's interpreter loads no extension modules (no dlopen): rebuild it (addons/mod)" >&2; exit 1; }

# ── libcxx: libc++, libc++abi and libunwind for wasm32, with C++ exceptions, PIC ─────────────────
# The other add-ons' libc++ has no exceptions; nanobind (LIEF's bindings) cannot do without them.
# Position-independent: it goes into each C++ module (a shared library), statically.
# -fwasm-exceptions: WebAssembly exception handling, which the engine runs (wasm-cc rewrites the
# linked program to the standard exnref encoding); libunwind's Unwind-wasm.c is the unwinder (the
# engine unwinds; it only calls the personality routine), merged into libc++abi.a.
# LIBUNWIND_HIDE_SYMBOLS: without it, LLVM 19's libunwind marks its API __declspec(dllexport) on
# anything that is not ELF or Mach-O.
LIBCXX="$B/libcxx-install"
WASM_FLAGS="--target=wasm32-unknown-unknown -matomics -mbulk-memory -mllvm -wasm-enable-sjlj -fwasm-exceptions -fPIC -D__linux__=1 -D__linux=1 -D__unix__=1 -D__unix=1 -D__gnu_linux__=1 -Wno-unused-command-line-argument -Wno-override-module --sysroot=$U/sysroot"
if stage libcxx "$LIBCXX/lib/libc++.a" "$(lock llvm)" "$(record "$B/libcxx.flags" "$WASM_FLAGS")"; then
  RT_SRC="$B/rt-src"
  rm -rf "$RT_SRC" "$B/libcxx" "$LIBCXX"; mkdir -p "$RT_SRC"
  tar -xJmf "$(fetch llvm)" -C "$RT_SRC" --strip-components=1 \
    llvm-project-19.1.7.src/{runtimes,cmake,llvm/cmake,llvm/utils/llvm-lit,libcxx,libcxxabi,libunwind}
  LOG="$B/libcxx.log"; : > "$LOG"
  log_run "$LOG" "$CMAKE" -DCMAKE_MAKE_PROGRAM="$NINJA" -S "$RT_SRC/runtimes" -B "$B/libcxx" -G Ninja \
    --log-level=WARNING -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$LIBCXX" \
    -DCMAKE_SYSTEM_NAME=Linux -DCMAKE_C_COMPILER=/usr/bin/clang-19 -DCMAKE_CXX_COMPILER=/usr/bin/clang++-19 \
    -DCMAKE_AR=/usr/bin/llvm-ar-19 -DCMAKE_RANLIB=/usr/bin/llvm-ranlib-19 \
    -DCMAKE_C_COMPILER_TARGET=wasm32-unknown-unknown -DCMAKE_CXX_COMPILER_TARGET=wasm32-unknown-unknown \
    "-DCMAKE_C_FLAGS=$WASM_FLAGS" "-DCMAKE_CXX_FLAGS=$WASM_FLAGS" -DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY \
    -DLLVM_ENABLE_RUNTIMES="libunwind;libcxxabi;libcxx" -DLLVM_INCLUDE_TESTS=OFF -DLLVM_INCLUDE_DOCS=OFF \
    -DLIBUNWIND_ENABLE_SHARED=OFF -DLIBUNWIND_ENABLE_STATIC=ON -DLIBUNWIND_INCLUDE_TESTS=OFF \
    -DLIBUNWIND_ENABLE_THREADS=ON -DLIBUNWIND_USE_COMPILER_RT=OFF -DLIBUNWIND_HIDE_SYMBOLS=ON \
    -DLIBCXX_INCLUDE_BENCHMARKS=OFF -DLIBCXX_INCLUDE_TESTS=OFF -DLIBCXXABI_INCLUDE_TESTS=OFF \
    -DLIBCXXABI_ENABLE_SHARED=OFF -DLIBCXXABI_ENABLE_STATIC=ON -DLIBCXXABI_ENABLE_EXCEPTIONS=ON \
    -DLIBCXXABI_USE_LLVM_UNWINDER=ON -DLIBCXXABI_ENABLE_STATIC_UNWINDER=ON \
    -DLIBCXXABI_HAS_PTHREAD_API=ON -DLIBCXXABI_HAS_CXA_THREAD_ATEXIT_IMPL=OFF \
    -DLIBCXX_ENABLE_SHARED=OFF -DLIBCXX_ENABLE_STATIC=ON -DLIBCXX_ENABLE_EXCEPTIONS=ON \
    -DLIBCXX_ENABLE_FILESYSTEM=ON -DLIBCXX_HAS_MUSL_LIBC=ON -DLIBCXX_CXX_ABI=libcxxabi \
    -DLIBCXX_ENABLE_STATIC_ABI_LIBRARY=ON
  log_run "$LOG" "$NINJA" -C "$B/libcxx" -j"$JOBS" install
  # libunwind's register save/restore are assembly for real CPUs: empty, non-wasm objects here
  for lib in "$LIBCXX"/lib/*.a; do
    if llvm-ar-19 t "$lib" | grep -q '\.S\.o$'; then
      llvm-ar-19 d "$lib" UnwindRegistersRestore.S.o UnwindRegistersSave.S.o
    fi
  done
  rm -rf "$B/libcxx"   # the objects; the install is what the builds use
fi

# ── host: the build tools that run on this machine, and the guest's Python as they see it ──────
#   bin/python3-guest   the host CPython 3.13 answering for the guest's: sysconfig gives the guest
#                       interpreter's build variables (python/build-wasm's _sysconfigdata, with
#                       its headers here, python/stage/usr/include/python3.13), so meson's and
#                       CMake's Python lookups find the guest's pyconfig.h, not the host's
#   bin/cython          Cython (its pure-Python wheel) on the host CPython
#   site/               Cython, setuptools and pybind11 (Pillow's setup.py), for both
H="$B/host"
if stage host "$H/.done" $(lock cython setuptools pybind11) "$PYEXT/build.sh" "$P/build-wasm/libpython3.13.a"; then
  rm -rf "$H"; mkdir -p "$H/bin" "$H/site" "$H/sysconfig"
  for wheel in cython setuptools pybind11; do
    "$HOSTPY" -I -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' "$(fetch $wheel)" "$H/site"
  done
  SYSCONFIG="$(ls "$P"/build-wasm/build/lib.*/_sysconfigdata_*.py)"
  NAME="$(basename "$SYSCONFIG" .py)"
  sed -e "s#'/usr/include/python3.13'#'$P/stage/usr/include/python3.13'#g" "$SYSCONFIG" > "$H/sysconfig/$NAME.py"
  grep -q "'INCLUDEPY': '$P/stage" "$H/sysconfig/$NAME.py" || { echo "INCLUDEPY not rewritten in $NAME.py" >&2; exit 1; }
  cat > "$H/bin/python3-guest" <<EOF
#!/bin/sh
# The host CPython 3.13 answering for the guest's (addons/python-extension/build.sh, stage host).
_PYTHON_SYSCONFIGDATA_NAME=$NAME _PYTHON_HOST_PLATFORM=linux-wasm32 \\
PYTHONPATH="$H/sysconfig:$H/site\${PYTHONPATH:+:\$PYTHONPATH}" exec "$HOSTPY" "\$@"
EOF
  cat > "$H/bin/cython" <<EOF
#!/bin/sh
PYTHONPATH="$H/site" exec "$HOSTPY" -m cython "\$@"
EOF
  chmod +x "$H/bin/python3-guest" "$H/bin/cython"
  "$H/bin/cython" --version
  "$H/bin/python3-guest" -c 'import sysconfig; v = sysconfig.get_config_var; print(" guest:", v("HOST_GNU_TYPE"), v("EXT_SUFFIX"), v("SIZEOF_VOID_P"), v("INCLUDEPY"))'
  touch "$H/.done"
fi
export PATH="$H/bin:$PATH"

# ── <package>: each packages/<name>/build.sh, into build/<name>/site ───────────────────────────
# Every compile is recorded in build/<name>/compdb (tools/wasm-pyext-cc) for the stage after.
for name in "${PACKAGES[@]}"; do
  BP="$B/$name" PYEXT_COMPDB="$B/$name/compdb" bash "$PYEXT/packages/$name/build.sh"
done

# ── fpcast-<package>: no function the interpreter will call as a type it does not have ─────────
# WebAssembly checks the type of every indirect call, and the guest's CPython calls a method
# table's functions, getters, setters and type slots as the types it declares them: a method
# written with fewer parameters traps there, where it works natively (tools/fpcast-scan.py). The
# casts found must be fixed (patches/), or, reviewed as never called, listed in
# packages/<name>/fpcasts.allow (one source path per line, as the scan prints it after the source
# folder, with the reason in a comment).
for name in "${PACKAGES[@]}"; do
  allow="$PYEXT/packages/$name/fpcasts.allow"
  # (and the other versions a package builds for wheels: build/<name>/wheels/, compiled alike)
  if stage "fpcast-$name" "$B/$name/fpcast.ok" "$B/$name/site/.done" $(ls "$B/$name"/wheels/*/.done 2>/dev/null) \
       "$PYEXT/tools/fpcast-scan.py" $( [[ -f "$allow" ]] && echo "$allow"); then
    if [[ ! -d "$B/$name/compdb" && -z "$(find "$B/$name/site" -name '*.so' -print -quit)" ]]; then
      echo "  pure Python: nothing to scan"; touch "$B/$name/fpcast.ok"; continue
    fi
    # (Rust and no C: PyO3 declares each function as CPython calls it, and nothing was compiled to scan)
    if [[ ! -d "$B/$name/compdb" && " ${RUST[*]} " == *" $name "* ]]; then
      echo "  Rust, no C: nothing to scan"; touch "$B/$name/fpcast.ok"; continue
    fi
    [[ -d "$B/$name/compdb" ]] || { echo "no compile records in $B/$name/compdb: FORCE=$name to rebuild it" >&2; exit 1; }
    rc=0; found="$(python3 "$PYEXT/tools/fpcast-scan.py" "$B/$name/compdb" 2>"$B/$name/fpcast.log")" || rc=$?
    [[ $rc -le 1 ]] || { cat "$B/$name/fpcast.log" >&2; echo "error: tools/fpcast-scan.py failed" >&2; exit 1; }
    if [[ -f "$allow" ]]; then
      found="$(grep -v -F -f <(sed -e 's/#.*//' -e 's/[[:space:]]*$//' -e '/^$/d' "$allow") <<<"$found" || true)"
    fi
    if [[ -n "$found" ]]; then
      echo "$found"; echo "error: $name: functions the interpreter would call as another type (they trap in the guest)" >&2; exit 1
    fi
    tail -1 "$B/$name/fpcast.log"
    touch "$B/$name/fpcast.ok"
  fi
done

# ── install: the packages into one site-packages ────────────────────────────────────────────────
SITE="$B/site"
INSTALLED="$B/site.installed"
dones=(); for name in "${PACKAGES[@]}"; do dones+=("$B/$name/site/.done"); done
# (the list of packages is an input too: a build of fewer packages installs fewer)
[[ "$(cat "$B/packages.txt" 2>/dev/null)" == "${PACKAGES[*]}" ]] || { echo "${PACKAGES[*]}" > "$B/packages.txt"; redo install; }
if stage install "$INSTALLED" "${dones[@]}" "$PYEXT/tools/install-tree.py" "$P/stage/usr/bin/python3.13"; then
  "$HOSTPY" "$PYEXT/tools/install-tree.py" "$P/stage/usr/bin/python3.13" "$SITE" "$B/scripts" "${dones[@]%/.done}"
  touch "$INSTALLED"
fi

# ── ctypes: the interpreter's _ctypes, in lib-dynload ───────────────────────────────────────────
# The python step builds no _ctypes (there was no libffi for this platform); packages/cffi's libffi
# has a backend for it. CPython's Modules/_ctypes (the python step's source), built as its Makefile
# builds them on Linux — a core module (Py_BUILD_CORE_MODULE: the interpreter's internal headers,
# whose functions it exports) with libffi's three features its configure would find — into
# /usr/lib/python3.13/lib-dynload, where the interpreter looks for its own modules; ctypes' Python
# half is in its standard library already. CTYPES_MAX_ARGCOUNT 1000, as on Emscripten: a
# WebAssembly function has at most 1000 parameters, and libffi's backend refuses more.
# _ctypes_test, the module CPython's test_ctypes calls, is built beside it for the tests
# (build/ctypes/test), not installed. Compiles scanned as the packages' are (fpcast-scan.py), the
# modules checked to link as install checks its own.
CT="$B/ctypes"
if stage ctypes "$CT/.done" "$B/cffi/deps/.done" "$PYEXT/build.sh" "$PYEXT/tools/wasm-pyext-cc" "$ROOT/userspace/bin/wasm-cc" \
     "$PYEXT/tools/fpcast-scan.py" "$PYEXT/tools/install-tree.py" "$P/stage/usr/include/python3.13/pyconfig.h" \
     "$P/stage/usr/bin/python3.13"; then
  [[ -f "$B/cffi/deps/.done" ]] || { echo "ctypes needs packages/cffi's libffi: build cffi too" >&2; exit 1; }
  rm -rf "$CT"; mkdir -p "$CT/obj" "$CT/lib-dynload" "$CT/test"
  SRC="$P/src/Python-3.13.14/Modules/_ctypes" INC="$P/stage/usr/include/python3.13"
  LOG="$CT/build.log"; : > "$LOG"
  flags=(-O2 -DHAVE_FFI_PREP_CIF_VAR=1 -DHAVE_FFI_PREP_CLOSURE_LOC=1 -DHAVE_FFI_CLOSURE_ALLOC=1 -DCTYPES_MAX_ARGCOUNT=1000
         -I"$INC" -I"$INC/internal" -I"$B/cffi/deps/include")
  for f in _ctypes callbacks callproc stgdict cfield; do
    log_run "$LOG" env PYEXT_COMPDB="$CT/compdb" "$PYEXT/tools/wasm-pyext-cc" "${flags[@]}" -c "$SRC/$f.c" -o "$CT/obj/$f.o"
  done
  log_run "$LOG" "$PYEXT/tools/wasm-pyext-cc" -shared -o "$CT/lib-dynload/_ctypes.cpython-313.so" \
    "$CT"/obj/{_ctypes,callbacks,callproc,stgdict,cfield}.o "$B/cffi/deps/lib/libffi.a"
  log_run "$LOG" env PYEXT_COMPDB="$CT/compdb" "$PYEXT/tools/wasm-pyext-cc" -O2 -I"$INC" -c "$SRC/_ctypes_test.c" -o "$CT/obj/_ctypes_test.o"
  log_run "$LOG" "$PYEXT/tools/wasm-pyext-cc" -shared -o "$CT/test/_ctypes_test.cpython-313.so" "$CT/obj/_ctypes_test.o"
  rc=0; found="$(python3 "$PYEXT/tools/fpcast-scan.py" "$CT/compdb" 2>"$CT/fpcast.log")" || rc=$?
  [[ $rc -le 1 ]] || { cat "$CT/fpcast.log" >&2; echo "error: tools/fpcast-scan.py failed" >&2; exit 1; }
  [[ -z "$found" ]] || { echo "$found"; echo "error: _ctypes: functions the interpreter would call as another type" >&2; exit 1; }
  tail -1 "$CT/fpcast.log"
  "$HOSTPY" "$PYEXT/tools/install-tree.py" --check "$P/stage/usr/bin/python3.13" "$CT"/lib-dynload/*.so "$CT"/test/*.so
  touch "$CT/.done"
  redo cpio
fi

# ── wheels: what pip in the guest finds in /usr/share/python-wheels ──────────────────────────────
# etc/pip.conf (/etc/pip.conf) has pip's find-links there, so a build's isolated environment, a
# virtual environment or `pip install cffi==2.0.0` takes these where PyPI has only sources (none of
# PyPI's wheels is for this platform, linux_wasm): for each WHEELS package, its installed version
# (build/<name>/site) and the others its build.sh leaves in build/<name>/wheels/. pip takes, for a
# requirement, the highest version that matches it; of one version, a wheel before an sdist — and,
# for unicorn, wheels alone (pip.conf's only-binary): pwntools' `unicorn>=2.0.1,!=2.1.3,!=2.1.4`
# would pick PyPI's 2.1.2 sdist over the 2.0.1.post1 wheel otherwise. The modules and libraries are
# checked to link as install's are (tools/make-wheels.py).
WHL="$B/wheels"
trees=(); tree_dones=()
for name in "${WHEELS[@]}"; do
  [[ " ${PACKAGES[*]} " == *" $name "* ]] || continue
  for t in "$B/$name/site" "$B/$name"/wheels/*; do
    [[ -f "$t/.done" ]] && { trees+=("$t"); tree_dones+=("$t/.done"); }
  done
done
[[ "$(cat "$B/wheels.txt" 2>/dev/null)" == "${trees[*]}" ]] || { echo "${trees[*]}" > "$B/wheels.txt"; redo wheels; }
if stage wheels "$B/wheels.done" "${tree_dones[@]}" "$PYEXT/tools/make-wheels.py" "$PYEXT/tools/install-tree.py" \
     "$P/stage/usr/bin/python3.13"; then
  "$HOSTPY" "$PYEXT/tools/make-wheels.py" "$P/stage/usr/bin/python3.13" "$WHL" "${trees[@]}"
  touch "$B/wheels.done"
  redo cpio
fi

# ── dev: what a build of an extension module in the guest reads ────────────────────────────────
# The interpreter's headers, in /usr/include/python3.13 (sysconfig's INCLUDEPY), and its
# pkg-config file (python3.pc, python-3.13.pc), as a distribution's python3-dev has them; numpy's
# are in its package already (numpy.get_include()). Not the -embed ones: there is no libpython to
# link a program with. Libs.private names the interpreter's own static libraries (python/deps,
# linked into it, at their build paths): a module links none of them.
DEV="$B/dev"
if stage dev "$DEV/.done" "$P/stage/usr/include/python3.13/pyconfig.h" "$P/stage/usr/lib/pkgconfig/python-3.13.pc"; then
  rm -rf "$DEV"; mkdir -p "$DEV/include" "$DEV/pkgconfig"
  cp -a "$P/stage/usr/include/python3.13" "$DEV/include/python3.13"
  sed '/^Libs.private:/d' "$P/stage/usr/lib/pkgconfig/python-3.13.pc" > "$DEV/pkgconfig/python-3.13.pc"
  if grep -rlq "$ROOT" "$DEV"; then
    echo "the Python headers or python-3.13.pc name build paths:" >&2; grep -rl "$ROOT" "$DEV" >&2; exit 1
  fi
  touch "$DEV/.done"
  redo cpio
fi

# ── cpio: the overlay ───────────────────────────────────────────────────────────────────────────
CPIO="$PYEXT/out/python-extension.cpio"
if stage cpio "$CPIO" "$INSTALLED" "$CT/.done" "$B/wheels.done" "$PYEXT/etc/pip.conf" "$PYEXT/etc/collabo-config-site.pth" \
     "$DEV/.done"; then
  rm -rf "$PYEXT/out/licenses"; mkdir -p "$PYEXT/out/licenses"
  for info in "$SITE"/*.dist-info; do
    pkg="$(basename "$info" | cut -d- -f1)"
    for f in $(cd "$info" && find licenses -type f 2>/dev/null | sort); do
      cp "$info/$f" "$PYEXT/out/licenses/addon-python-extension-$pkg-$(basename "$f")"
    done
  done
  "$HOSTPY" -I -c 'import sys, tarfile; sys.stdout.buffer.write(tarfile.open(sys.argv[1]).extractfile("llvm-project-19.1.7.src/libcxx/LICENSE.TXT").read())' \
    "$(fetch llvm)" > "$PYEXT/out/licenses/addon-python-extension-libcxx.txt"
  for name in "${RUST[@]}"; do
    if [[ " ${PACKAGES[*]} " == *" $name "* ]]; then
      cp "$ROOT/.tools/rust-wasm/COPYRIGHT-library.html" "$PYEXT/out/licenses/addon-python-extension-rust-std-COPYRIGHT-library.html"
      break
    fi
  done
  cp "$P/src/Python-3.13.14/LICENSE" "$PYEXT/out/licenses/addon-python-extension-cpython-headers.txt"
  python3 "$U/mkinitramfs.py" --overlay -o "$CPIO" \
    --tree usr/bin="$B/scripts" \
    --tree usr/lib/python3.13/site-packages="$SITE" \
    --file usr/lib/python3.13/site-packages/collabo-config-site.pth="$PYEXT/etc/collabo-config-site.pth" \
    --tree usr/share/python-wheels="$WHL" \
    --tree usr/lib/python3.13/lib-dynload="$CT/lib-dynload" \
    --file etc/pip.conf="$PYEXT/etc/pip.conf" \
    --tree usr/include/python3.13="$DEV/include/python3.13" \
    --file usr/lib/pkgconfig/python-3.13.pc="$DEV/pkgconfig/python-3.13.pc" \
    --link usr/lib/pkgconfig/python3.pc=python-3.13.pc
fi
ls -l "$CPIO"
