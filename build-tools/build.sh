#!/usr/bin/env bash
# The build-tools add-on: the programs that drive a build, for the guest — GNU make 4.4.1, ninja
# 1.13.2, pkgconf 3.0.10 (as pkg-config too) and CMake 4.4.5 (cmake, ctest) — packed as
# out/build-tools.cpio (an overlay: /usr/bin/make, gmake, ninja, pkgconf, pkg-config, cmake, ctest,
# patchelf (tools/patchelf, for WebAssembly files); /usr/share/cmake-4.4, /usr/share/config.site
# (etc/config.site)). They start their commands with posix_spawn, so they need no fork(). The
# compilers are not here: the llvm add-on has them. README.md has the details.
#
# Needs the userspace step (musl, compiler-rt: userspace/sysroot), the python step (OpenSSL for
# the guest, python/deps: CMake's curl) and cmake and ninja (./build.sh tools). Sources are the
# release archives in sources.lock, unpacked into build/src/<name> with patches/<name>/*.patch
# applied: a git repository whose first commit is the pristine source, so `git diff` there shows
# (and regenerates) a patch.
#
#   addons/build-tools/build.sh
#   FORCE=stage[,stage]    redo those stages: src-make src-ninja src-pkgconf src-cmake
#                          src-googletest libcxx make ninja pkgconf cmake cpio
#   TEST=1                 also the upstream test programs for tests/run.sh --upstream: ninja_test
#                          (with googletest) and pkgconf's test-runner, in build/tests; CMake's
#                          own tests, in build/cmake
#   JOBS=N
set -euo pipefail
A="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$A/../.." && pwd)"
U="$ROOT/userspace"
B="$A/build"
DL="$B/dl"
S="$B/src"
STAGE="$B/stage"
JOBS="${JOBS:-$(nproc)}"
FORCE=",${FORCE:-},"
TEST="${TEST:-0}"
CMAKE="$ROOT/.tools/cmake/bin/cmake"
NINJA="$ROOT/.tools/cmake/bin/ninja"
CC_W="$A/tools/wasm-bt-cc"
CXX_W="$A/tools/wasm-bt-c++"
HOST_TRIPLE="$(uname -m)-pc-linux-gnu"

stage() { # name output...: true when the stage must run
  local name="$1"; shift
  if [[ "$FORCE" != *",$name,"* ]]; then
    local f ok=1
    for f in "$@"; do [[ -e "$f" ]] || ok=0; done
    if [[ $ok == 1 ]]; then echo "== $name: up to date"; return 1; fi
  fi
  echo "== $name"; return 0
}
redo() { FORCE="$FORCE$1,"; } # name: the stage must run (its inputs changed)
log_run() { # log command...
  local log="$1"; shift
  "$@" >> "$log" 2>&1 || { tail -40 "$log"; echo "failed: $* (log: $log)" >&2; exit 1; }
}
fetch() { # name: the download sources.lock names, into build/dl (once; another add-on's copy when
          # it has one), checked; prints its path
  local line url sum file other
  line="$(grep -E "^$1 " "$A/sources.lock")" || { echo "$1 is not in sources.lock" >&2; exit 1; }
  read -r _ url sum file <<<"$line"
  file="$DL/${file:-$(basename "$url")}"
  mkdir -p "$DL"
  if [[ ! -f "$file" ]]; then
    for other in "$A"/../*/build/dl/"$(basename "$file")"; do
      [[ -f "$other" ]] && { ln "$other" "$file" 2>/dev/null || cp "$other" "$file"; break; }
    done
  fi
  if [[ ! -f "$file" ]]; then
    echo "== fetch $(basename "$file")" >&2
    curl -fL --retry 3 -s -S -o "$file.part" "$url" && mv "$file.part" "$file"
  fi
  echo "$sum  $file" | sha256sum -c --quiet - >&2 || { echo "checksum mismatch: $file (deleted)" >&2; rm -f "$file"; exit 1; }
  echo "$file"
}

[[ -f "$U/sysroot/lib/crt1.o" ]] || { echo "missing userspace/sysroot; run: ./build.sh userspace" >&2; exit 1; }
[[ -x "$CMAKE" && -x "$NINJA" ]] || { echo "missing .tools/cmake; run: ./build.sh tools" >&2; exit 1; }
chmod +x "$A"/tools/wasm-*
mkdir -p "$B" "$A/out"

# ── src-<name>: each source unpacked, with its patches ──────────────────────────────────────────
# A patch newer than the unpacked source redoes it.
unpack() { # name
  local name="$1" d="$S/$1" p
  rm -rf "$d"; mkdir -p "$d"
  # the archive's own times: what autoconf and automake made is newer than what it is made from
  tar -xf "$(fetch "$name")" -C "$d" --strip-components=1
  git -C "$d" init -q && git -C "$d" add -A -f
  git -C "$d" -c user.name=build -c user.email=build@localhost commit -q -m pristine
  for p in "$A"/patches/"$name"/*.patch; do
    [[ -f "$p" ]] || continue
    echo "  patch $name/$(basename "$p")"
    git -C "$d" apply --whitespace=nowarn "$p"
  done
  touch "$d/.stamp"
}
SOURCES=(make ninja pkgconf cmake)
[[ "$TEST" == 1 ]] && SOURCES+=(googletest)
for name in "${SOURCES[@]}"; do
  [[ -f "$S/$name/.stamp" && -d "$A/patches/$name" \
     && -n "$(find "$A/patches/$name" -name '*.patch' -newer "$S/$name/.stamp" -print -quit)" ]] && redo "src-$name"
  if stage "src-$name" "$S/$name/.stamp"; then
    unpack "$name"
    case "$name" in googletest) redo ninja ;; *) redo "$name" ;; esac
  fi
done

# ── libcxx: libc++, libc++abi and libunwind for wasm32, with C++ exceptions ────────────────────
# ninja, googletest and CMake are C++, CMake with exceptions (throw and catch in its own code):
# LLVM 19.1.7's runtimes as the python-extension add-on builds them (not PIC: these are
# programs), -fwasm-exceptions, WebAssembly exception handling, which the engine runs (wasm-cc
# rewrites the linked program to the standard exnref encoding); libunwind's Unwind-wasm.c is the
# unwinder (the engine unwinds; it only calls the personality routine), merged into libc++abi.a.
# LIBUNWIND_HIDE_SYMBOLS: without it, LLVM 19's libunwind marks its API __declspec(dllexport) on
# anything that is not ELF or Mach-O.
LIBCXX="$B/libcxx-install"
WASM_FLAGS="--target=wasm32-unknown-unknown -matomics -mbulk-memory -mllvm -wasm-enable-sjlj -fwasm-exceptions -D__linux__=1 -D__linux=1 -D__unix__=1 -D__unix=1 -D__gnu_linux__=1 -Wno-unused-command-line-argument -Wno-override-module --sysroot=$U/sysroot"
[[ "$(cat "$LIBCXX/flags" 2>/dev/null)" == "$WASM_FLAGS" ]] || redo libcxx
if stage libcxx "$LIBCXX/lib/libc++.a" "$B/rt-src/libcxx/LICENSE.TXT"; then
  RT_SRC="$B/rt-src"
  RT_DIRS=(runtimes cmake llvm/cmake llvm/utils/llvm-lit libcxx libcxxabi libunwind)
  rm -rf "$RT_SRC" "$B/libcxx" "$LIBCXX"; mkdir -p "$RT_SRC"
  tar -xJmf "$(fetch llvm)" -C "$RT_SRC" --strip-components=1 "${RT_DIRS[@]/#/llvm-project-19.1.7.src/}"
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
  echo "$WASM_FLAGS" > "$LIBCXX/flags"
  rm -rf "$B/libcxx"   # the objects; the install is what the build uses
  redo ninja; redo cmake
fi

# ── make: GNU make, configured for the guest ─────────────────────────────────────────────────────
# Its jobs, $(shell) and the commands it runs start with posix_spawn (configure finds it: make's
# USE_POSIX_SPAWN), the shell /bin/sh; no Guile, no `load` (no dlopen for programs), no NLS.
# Built out of the source tree, which tests/run.sh --upstream then takes its tests/ from.
if stage make "$STAGE/usr/bin/make"; then
  MB="$B/make"; LOG="$B/make.log"; : > "$LOG"
  rm -rf "$MB"; mkdir -p "$MB"
  (cd "$MB" && log_run "$LOG" env CONFIG_SITE="$ROOT/python/config.site" "$S/make/configure" \
     --host=wasm32-unknown-linux-musl --build="$HOST_TRIPLE" \
     CC="$CC_W" AR=llvm-ar-19 RANLIB=llvm-ranlib-19 CFLAGS="-O2" \
     --prefix=/usr --disable-nls --without-guile --without-customs --disable-load \
     --disable-dependency-tracking)
  grep -q '^#define USE_POSIX_SPAWN 1' "$MB/src/config.h" \
    || { echo "make's configure did not take posix_spawn: it would vfork" >&2; exit 1; }
  log_run "$LOG" make -C "$MB" -j"$JOBS"
  install -D -m 755 "$MB/make" "$STAGE/usr/bin/make"
  redo cpio
fi

# ── ninja: ninja, with CMake ─────────────────────────────────────────────────────────────────────
# Its commands start with posix_spawn (subprocess-posix.cc), waited for with ppoll. Not `ninja -t
# browse` (it forks a Python web server): HAVE_FORK=0 leaves it out. With TEST=1 also ninja_test,
# on googletest (build/src/googletest, not downloaded by CMake).
NB="$B/ninja"
if [[ "$TEST" == 1 && ! -f "$B/tests/ninja_test" ]]; then redo ninja; fi
if stage ninja "$STAGE/usr/bin/ninja"; then
  LOG="$B/ninja.log"; : > "$LOG"
  rm -rf "$NB"
  test_args=(-DBUILD_TESTING=OFF)
  [[ "$TEST" == 1 ]] && test_args=(-DBUILD_TESTING=ON -DFETCHCONTENT_FULLY_DISCONNECTED=ON
                                   -DFETCHCONTENT_SOURCE_DIR_GOOGLETEST="$S/googletest" -DINSTALL_GTEST=OFF)
  log_run "$LOG" "$CMAKE" -DCMAKE_MAKE_PROGRAM="$NINJA" -S "$S/ninja" -B "$NB" -G Ninja --log-level=WARNING \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR=wasm32 \
    -DCMAKE_C_COMPILER="$CC_W" -DCMAKE_CXX_COMPILER="$CXX_W" \
    -DCMAKE_AR=/usr/bin/llvm-ar-19 -DCMAKE_RANLIB=/usr/bin/llvm-ranlib-19 \
    -DCMAKE_SKIP_RPATH=ON -DHAVE_FORK=0 -DRE2C=RE2C-NOTFOUND "${test_args[@]}"
  log_run "$LOG" "$NINJA" -C "$NB" -j"$JOBS" ninja $([[ "$TEST" == 1 ]] && echo ninja_test)
  install -D -m 755 "$NB/ninja" "$STAGE/usr/bin/ninja"
  [[ "$TEST" == 1 ]] && install -D -m 755 "$NB/ninja_test" "$B/tests/ninja_test"
  redo cpio
fi

# ── pkgconf: pkgconf, configured for the guest's layout ──────────────────────────────────────────
# Static (no libpkgconf.so); its search path /usr/lib/pkgconfig:/usr/share/pkgconfig, /usr/lib and
# /usr/include the system's (left out of --libs and --cflags, as pkg-config does). As `pkg-config`
# it behaves as pkg-config. With TEST=1 also its test-runner, the tools its tests run and its
# API tests (test-api-*).
PB="$B/pkgconf"
if [[ "$TEST" == 1 && ! -f "$B/tests/pkgconf/test-runner" ]]; then redo pkgconf; fi
if stage pkgconf "$STAGE/usr/bin/pkgconf"; then
  LOG="$B/pkgconf.log"; : > "$LOG"
  rm -rf "$PB"; mkdir -p "$PB"
  (cd "$PB" && log_run "$LOG" env CONFIG_SITE="$ROOT/python/config.site" "$S/pkgconf/configure" \
     --host=wasm32-unknown-linux-musl --build="$HOST_TRIPLE" \
     CC="$CC_W" AR=llvm-ar-19 RANLIB=llvm-ranlib-19 CFLAGS="-O2" \
     --prefix=/usr --disable-shared --enable-static --disable-dependency-tracking \
     --with-pkg-config-dir=/usr/lib/pkgconfig:/usr/share/pkgconfig \
     --with-system-libdir=/usr/lib --with-system-includedir=/usr/include)
  log_run "$LOG" make -C "$PB" -j"$JOBS" pkgconf
  install -D -m 755 "$PB/pkgconf" "$STAGE/usr/bin/pkgconf"
  if [[ "$TEST" == 1 ]]; then
    api="$(make -C "$PB" -pn 2>/dev/null | sed -n 's/^API_TEST_PROGS = //p' | head -1)"
    [[ -n "$api" ]] || { echo "pkgconf's Makefile names no API_TEST_PROGS" >&2; exit 1; }
    log_run "$LOG" make -C "$PB" -j"$JOBS" test-runner bomtool spdxtool pccritic $api
    rm -rf "$B/tests/pkgconf"; mkdir -p "$B/tests/pkgconf"
    for prog in test-runner pkgconf bomtool spdxtool pccritic $api; do
      install -m 755 "$PB/$prog" "$B/tests/pkgconf/$prog"
    done
  fi
  redo cpio
fi

# ── cmake: CMake, cross-built with this machine's CMake ─────────────────────────────────────────
# Every process it starts (execute_process, the builds of try_compile and cmake --build, the
# compiler checks, ctest's tests) goes through its libuv (cmUVProcessChain), whose uv_spawn forks:
# patches/cmake/0001 has it spawn with posix_spawn, as libuv does on macOS; 0002 the same for
# KWSys' Process (exec_program()); 0003 leaves libuv's io_uring out (no io_uring in the guest's
# kernel, no mmap for its rings: libuv uses epoll, as without it); 0004 starts libuv's threads
# through a start routine of the type pthread_create calls (its cast of another traps on wasm);
# 0005 gives its Linux platform no run path on the guest (WebAssembly has none).
# Its own copies of what
# it uses (libuv, curl, libarchive, expat, jsoncpp, zlib, zstd, xz, bzip2, nghttp2, rhash), curl on
# python/deps' OpenSSL with the guest's CA bundle; not ccmake (curses) or cmake-gui (Qt). What it
# looks for (headers, libraries) only in the guest's sysroot and python/deps, not this machine's
# /usr. With
# TEST=1 also CMake's own unit tests (Tests/CMakeLib: testUVProcessChain and the rest).
CB="$B/cmake"
if [[ "$TEST" == 1 && ! -f "$CB/Tests/CMakeLib/CMakeLibTests" ]]; then redo cmake; fi
if stage cmake "$B/cmake-install/usr/bin/cmake"; then
  [[ -f "$ROOT/python/deps/lib/libssl.a" ]] || { echo "missing OpenSSL for the guest; build the python step first (./build.sh python)" >&2; exit 1; }
  LOG="$B/cmake.log"; : > "$LOG"
  rm -rf "$CB" "$B/cmake-install"
  log_run "$LOG" "$CMAKE" -S "$S/cmake" -B "$CB" -G Ninja -DCMAKE_MAKE_PROGRAM="$NINJA" --log-level=WARNING \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR=wasm32 \
    -DCMAKE_C_COMPILER="$CC_W" -DCMAKE_CXX_COMPILER="$CXX_W" \
    -DCMAKE_AR=/usr/bin/llvm-ar-19 -DCMAKE_RANLIB=/usr/bin/llvm-ranlib-19 -DCMAKE_SKIP_RPATH=ON \
    "-DCMAKE_FIND_ROOT_PATH=$U/sysroot;$ROOT/python/deps" -DCMAKE_FIND_ROOT_PATH_MODE_PROGRAM=NEVER \
    -DCMAKE_FIND_ROOT_PATH_MODE_LIBRARY=ONLY -DCMAKE_FIND_ROOT_PATH_MODE_INCLUDE=ONLY \
    -DCMAKE_FIND_ROOT_PATH_MODE_PACKAGE=ONLY \
    -DCMAKE_INSTALL_PREFIX=/usr -DBUILD_TESTING="$([[ "$TEST" == 1 ]] && echo ON || echo OFF)" \
    -DBUILD_CursesDialog=OFF -DBUILD_QtDialog=OFF \
    -DCMAKE_USE_OPENSSL=ON -DOPENSSL_ROOT_DIR="$ROOT/python/deps" -DOPENSSL_USE_STATIC_LIBS=ON \
    -DCURL_CA_BUNDLE=/etc/ssl/certs/ca-certificates.crt -DCURL_CA_PATH=none
  log_run "$LOG" "$NINJA" -C "$CB" -j"$JOBS"
  log_run "$LOG" env DESTDIR="$B/cmake-install" "$CMAKE" --install "$CB"
  redo cpio
fi

# ── cpio: the overlay ───────────────────────────────────────────────────────────────────────────
CPIO="$A/out/build-tools.cpio"
[[ -f "$CPIO" && ( "$A/tools/patchelf" -nt "$CPIO" || "$A/etc/config.site" -nt "$CPIO" ) ]] && redo cpio
if stage cpio "$CPIO" "$B/cmake-install/usr/bin/cmake"; then
  L="$A/out/licenses"
  rm -rf "$L"; mkdir -p "$L"
  cp "$S/make/COPYING" "$L/addon-build-tools-make.txt"
  cp "$S/ninja/COPYING" "$L/addon-build-tools-ninja.txt"
  cp "$S/pkgconf/COPYING" "$L/addon-build-tools-pkgconf.txt"
  cp "$B/rt-src/libcxx/LICENSE.TXT" "$L/addon-build-tools-libcxx.txt"
  CI="$B/cmake-install/usr"
  cp "$S/cmake/LICENSE.rst" "$L/addon-build-tools-cmake.txt"
  for lib in cmbzip2/LICENSE cmcppdap/LICENSE cmcurl/COPYING cmexpat/COPYING cmjsoncpp/LICENSE \
             cmlibarchive/COPYING cmliblzma/COPYING cmlibrhash/COPYING cmlibuv/LICENSE cmnghttp2/COPYING \
             cmzlib/Copyright.txt cmzstd/LICENSE; do
    cp "$S/cmake/Utilities/$lib" "$L/addon-build-tools-cmake-${lib%%/*}.txt"
  done
  OPENSSL_LICENSE="$(ls "$ROOT"/python/build-deps/openssl-*/LICENSE.txt 2>/dev/null | head -1)"
  [[ -z "$OPENSSL_LICENSE" ]] || cp "$OPENSSL_LICENSE" "$L/addon-build-tools-cmake-openssl.txt"
  # cmake and ctest, CMake's modules, templates and help (cmake --help-*); not cpack, nor the
  # editors' and shells' files
  python3 "$U/mkinitramfs.py" --overlay -o "$CPIO" --tree "usr/bin=$STAGE/usr/bin" \
    --link usr/bin/gmake=make --link usr/bin/pkg-config=pkgconf --file "usr/bin/patchelf=$A/tools/patchelf" \
    --file "usr/share/config.site=$A/etc/config.site" --link usr/local/share/config.site=/usr/share/config.site \
    --file "usr/bin/cmake=$CI/bin/cmake" --file "usr/bin/ctest=$CI/bin/ctest" \
    --tree "usr/share/cmake-4.4=$CI/share/cmake-4.4"
fi
ls -l "$CPIO"
