#!/usr/bin/env bash
# Build the llvm add-on: the LLVM toolchain as wasm32 Linux programs, and what it needs to build the
# guest's own programs and shared libraries there, packed as out/llvm.cpio (an overlay):
#
#   /usr/bin           clang, opt, llc, llvm-mc, lld, llvm-ar, llvm-readobj, llvm-objdump, llvm-nm,
#                      llvm-dwarfdump, llvm-symbolizer, llvm-objcopy and their other names (clang++,
#                      ld.lld, wasm-ld, llvm-ranlib, llvm-strip, ...; ar, ranlib, nm, strip, objcopy,
#                      objdump, readelf, addr2line); cc, c++, cpp and ld (scripts, guest/)
#   /usr/include       the C library's headers (musl, with Linux's uapi), libc++'s (c++/v1)
#   /usr/lib           libc.a, crt1.o, libc.exports (the userspace step's), libc++.a, libc++abi.a,
#                      libunwind.a
#   /usr/lib/clang/24  clang's own headers, compiler-rt's builtins (and the copy -shared links)
#
# Stages (each skipped when its output is up to date):
#   src       LLVM, Clang, LLD and the C++ runtimes at LLVM_REV, with patches/*.patch applied
#   native    from src, for this machine: the tablegens the cross build runs, and clang, lld and
#             llvm-ar for WebAssembly, the compiler of every stage after it
#   runtimes  libunwind, libc++abi and libc++ for the guest: C++ exceptions, position-independent
#   cross     the tools, cross-compiled for wasm32 (Linux, musl)
#   cpio      strip, the sysroot, the scripts, pack out/llvm.cpio
#
# Needs: ./build.sh tools userspace (cmake/ninja, the musl sysroot, compiler-rt), clang-19 and lld-19
# on the host (they build the native stage), and the llvm-project clone at ../org/llvm-project with
# LLVM_REV in it (a shallow fetch of that one commit is enough; nothing is checked out).
#
#   FORCE=src,native,runtimes,cross,cpio  re-run the named stages
#   JOBS=N, NINJA_FLAGS="-k 0"            passed to the native and cross builds' ninja
#
#   ./build.sh dist [ARCHIVE]             builds nothing: packs the code and out/ (not build/) into
#                                         ARCHIVE (default dist/llvm-addon-YYYYMMDD.tar.gz)
set -euo pipefail

A="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$A/../.." && pwd)"
LLVM_SRC="$ROOT/../org/llvm-project"
U="$ROOT/userspace"
TOOLS="$ROOT/.tools"
B="$A/build"
OUT="$A/out"
JOBS="${JOBS:-$(nproc)}"

CMAKE="$TOOLS/cmake/bin/cmake"
# Ninja ships in the same bin/ as cmake inside the cmake bundle.
NINJA="$(dirname "$CMAKE")/ninja"
[[ -x "$NINJA" ]] || NINJA="$(find "$TOOLS" -name ninja -perm /111 -maxdepth 5 2>/dev/null | head -1)"

# Wrapper: always inject the ninja path so cmake doesn't have to find it on PATH.
cmake() { "$CMAKE" -DCMAKE_MAKE_PROGRAM="$NINJA" "$@"; }

SYSROOT="$U/sysroot"
RT_LIB="$U/out/rt/lib/wasm32-unknown-unknown/libclang_rt.builtins.a"

LLVM_REV=4350d12c88cd947c3fb2b5e0bd4b22804e98a586
LLVM_MAJOR=24

WASM_TARGET=wasm32-unknown-unknown
WASM_TRIPLE_LINUX=wasm32-unknown-linux-musl

LLVM_TARGETS="WebAssembly;X86;AArch64;ARM;RISCV"
LLVM_PROJECTS="clang;lld"

# Tools to install into /usr/bin in the guest
LLVM_TOOLS="clang;opt;llc;llvm-mc;lld;llvm-ar;llvm-readobj;llvm-objdump;llvm-nm;llvm-dwarfdump;llvm-symbolizer;llvm-objcopy"

FORCE=",${FORCE:-},"

# ── dist: the code and out/ as one .tar.gz, without build/ ───────────────────
# The archive holds llvm/ (build.sh, README.md, addon.json, cmake/, guest/, patches/, tests/, out/
# with the image and its licenses/) and the add-ons' LICENSE beside it: unpacked in addons/, it is a
# built add-on again. build/ (the source export, the native and cross trees: gigabytes) stays; it
# is what the stages below remake. Needs none of their prerequisites.
if [[ "${1:-}" == dist ]]; then
  N="$(basename "$A")"
  ARCHIVE="${2:-$A/dist/$N-addon-$(date +%Y%m%d).tar.gz}"
  [[ -f "$OUT/llvm.cpio" ]] || { echo "missing: $OUT/llvm.cpio (run ./build.sh first)" >&2; exit 1; }
  # out/ as it is: say so when the code it was built from changed since
  for f in "$A"/guest/* "$A"/patches/* "$A"/cmake/*; do
    if [[ "$f" -nt "$OUT/llvm.cpio" ]]; then
      echo "warning: ${f#"$A"/} is newer than out/llvm.cpio (./build.sh repacks it)" >&2
    fi
  done
  mkdir -p "$(dirname "$ARCHIVE")"
  ARCHIVE="$(cd "$(dirname "$ARCHIVE")" && pwd)/$(basename "$ARCHIVE")"
  lic=(); [[ -f "$A/../LICENSE" ]] && lic=(LICENSE)
  echo "== packing $ARCHIVE"
  # written beside it and renamed: an interrupted run leaves no half archive under its name
  tar -C "$A/.." -czf "$ARCHIVE.part" --sort=name --owner=0 --group=0 --numeric-owner \
    --exclude="$N/build" --exclude="$N/dist" --exclude=__pycache__ --exclude="$(basename "$ARCHIVE").part" \
    "${lic[@]}" "$N"
  mv "$ARCHIVE.part" "$ARCHIVE"
  ls -lh "$ARCHIVE"
  exit 0
fi

# ── prerequisite checks ────────────────────────────────────────────────────────
check() {
  [[ -e "$1" ]] || { echo "missing: $1" >&2; echo "  $2" >&2; exit 1; }
}
check "$SYSROOT/lib/libc.a"        "run: ./build.sh userspace"
check "$SYSROOT/lib/libc.exports"  "run: ./build.sh userspace (with dlopen: FORCE=musl bash userspace/build.sh)"
check "$RT_LIB"                    "run: ./build.sh userspace"
check "$CMAKE"                     ".tools/cmake missing; run: ./build.sh tools"
check "$NINJA"                     ".tools/cmake (ninja) missing; run: ./build.sh tools"
git -C "$LLVM_SRC" cat-file -e "$LLVM_REV^{commit}" 2>/dev/null || {
  echo "missing: llvm-project $LLVM_REV in $LLVM_SRC" >&2
  echo "  git init $LLVM_SRC && git -C $LLVM_SRC fetch --depth 1 https://github.com/llvm/llvm-project.git $LLVM_REV" >&2
  exit 1
}
for t in clang-19 clang++-19 ld.lld-19; do
  command -v "$t" >/dev/null 2>&1 || { echo "missing: $t (apt install clang-19 lld-19)" >&2; exit 1; }
done

# ── stage helpers ──────────────────────────────────────────────────────────────
stage() {
  local name=$1 out=$2; shift 2
  if [[ "$FORCE" == *",$name,"* || ! -e "$out" ]]; then
    echo "==== stage $name"; return 0
  fi
  for src in "$@"; do
    [[ "$src" -nt "$out" ]] && { echo "==== stage $name (source changed)"; return 0; }
  done
  echo "==== stage $name: up to date"; return 1
}

# record FILE TEXT: FILE holds TEXT, rewritten only when it changes; prints FILE (a stage's input for
# what is no file: its flags, the list of what it exports).
record() {
  mkdir -p "$(dirname "$1")"
  [[ -f "$1" && "$(cat "$1")" == "$2" ]] || printf '%s\n' "$2" > "$1"
  echo "$1"
}

log_file() { echo "  (log: $1)"; }

mkdir -p "$B" "$OUT"

# ── stage src: the LLVM/Clang/LLD and runtimes sources, with patches/ applied ─
# Exported from the clone at a pinned commit (tests left out), so the clone itself is never
# modified. patches/*.patch port what this platform lacks (no fork, ...); see README.md.
# libc/: LLVM and libc++ include llvm-libc's shared headers; libunwind, libcxxabi, libcxx and
# runtimes/: the guest's C++ runtime (stage runtimes), of the same revision as its clang.
SRC="$B/src"
SRC_PATHS="llvm clang lld cmake third-party libc runtimes libunwind libcxxabi libcxx"
SRC_EXCLUDE="libc/test libc/docs libc/benchmarks libc/fuzzing llvm/test clang/test lld/test llvm/unittests
clang/unittests lld/unittests libcxx/test libcxxabi/test libunwind/test"
if stage src "$SRC/.stamp" "$A"/patches/*.patch "$(record "$B/src.paths" "$LLVM_REV $SRC_PATHS $SRC_EXCLUDE")"; then
  rm -rf "$SRC"; mkdir -p "$SRC"
  excl=(); for p in $SRC_EXCLUDE; do excl+=(":(exclude)$p"); done
  # shellcheck disable=SC2086
  git -C "$LLVM_SRC" archive "$LLVM_REV" $SRC_PATHS "${excl[@]}" | tar -xm -C "$SRC"
  for p in "$A"/patches/*.patch; do
    echo "  patch $(basename "$p")"
    patch -d "$SRC" -p1 --no-backup-if-mismatch -s < "$p"
  done
  touch "$SRC/.stamp"
fi

# ── stage native: tablegens, clang, lld, llvm-ar for this machine ─────────────
# The host's clang-19 cannot be the add-on's compiler: it writes WebAssembly exceptions (C++'s, and
# setjmp's) in the withdrawn "legacy" encoding, which the engine (wasmtime) does not run, and trunk's
# libc++ supports only the two newest clang releases. So clang of LLVM_REV is built for this machine
# first (WebAssembly only) and compiles everything else: -mllvm -wasm-use-legacy-eh=false gives the
# standard exnref encoding, as the guest's clang is told to (guest/cc).
NATIVE="$B/native"
NATIVE_TBLGEN="$NATIVE/bin/llvm-tblgen"
CLANG_TBLGEN="$NATIVE/bin/clang-tblgen"
if stage native "$NATIVE/.stamp" "$SRC/.stamp"; then
  LOG="$B/native.log"
  [[ -f "$NATIVE/build.ninja" ]] || cmake -S "$SRC/llvm" -B "$NATIVE" -G Ninja \
    --log-level=WARNING \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_COMPILER=/usr/bin/clang-19 \
    -DCMAKE_CXX_COMPILER=/usr/bin/clang++-19 \
    -DLLVM_USE_LINKER=lld-19 \
    -DLLVM_ENABLE_PROJECTS="$LLVM_PROJECTS" \
    -DLLVM_TARGETS_TO_BUILD=WebAssembly \
    -DLLVM_DEFAULT_TARGET_TRIPLE="$WASM_TARGET" \
    -DLLVM_PARALLEL_LINK_JOBS=2 \
    -DLLVM_INCLUDE_TESTS=OFF \
    -DLLVM_INCLUDE_DOCS=OFF \
    -DLLVM_INCLUDE_EXAMPLES=OFF \
    -DLLVM_INCLUDE_BENCHMARKS=OFF \
    -DLLVM_BUILD_LLVM_DYLIB=OFF \
    -DLLVM_LINK_LLVM_DYLIB=OFF \
    -DLLVM_ENABLE_ASSERTIONS=OFF \
    -DLLVM_ENABLE_ZLIB=OFF \
    -DLLVM_ENABLE_ZSTD=OFF \
    -DLLVM_ENABLE_LIBXML2=OFF \
    -DLLVM_ENABLE_LIBEDIT=OFF \
    -DLLVM_ENABLE_CURL=OFF \
    -DCLANG_ENABLE_STATIC_ANALYZER=OFF \
    > "$B/native-configure.log" 2>&1 || { tail -30 "$B/native-configure.log"; log_file "$B/native-configure.log"; exit 1; }
  # shellcheck disable=SC2086
  "$NINJA" -C "$NATIVE" -j"$JOBS" ${NINJA_FLAGS:-} llvm-tblgen clang-tblgen clang lld llvm-ar llvm-ranlib llvm-objdump llvm-objcopy \
    > "$LOG" 2>&1 || { tail -30 "$LOG"; log_file "$LOG"; exit 1; }
  touch "$NATIVE/.stamp"
fi

# The flags userspace/bin/wasm-cc compiles with (atomics + bulk memory: wasm-ld refuses
# --shared-memory without them in every object; setjmp/longjmp as wasm exceptions; the Linux
# macros), in the standard exception encoding.
WASM_FLAGS="--target=$WASM_TARGET -matomics -mbulk-memory -mllvm -wasm-enable-sjlj -mllvm -wasm-use-legacy-eh=false -D__linux__=1 -D__linux=1 -D__unix__=1 -D__unix=1 -D__gnu_linux__=1 -Wno-unused-command-line-argument -Wno-override-module --sysroot=$SYSROOT"

# no_legacy_eh FILE...: fails when one holds an instruction of the legacy exception encoding (try,
# catch, catch_all, delegate, rethrow): the engine could not compile a program linked with it.
no_legacy_eh() {
  local f bad
  for f; do
    bad="$("$NATIVE/bin/llvm-objdump" -d --no-show-raw-insn "$f" 2>/dev/null \
      | grep -E $'^\\s+[0-9a-f]+:\\s+(try|catch|catch_all|delegate|rethrow|end_try)\\b' | head -3 || true)"
    [[ -z "$bad" ]] || { echo "legacy exception instructions in $f:" >&2; echo "$bad" >&2; return 1; }
  done
}

# ── stage runtimes: libunwind, libc++abi, libc++ for the guest ───────────────
# With C++ exceptions (-fwasm-exceptions: WebAssembly exception handling; the engine unwinds,
# libunwind's Unwind-wasm.c only calls the personality routine) and position-independent: the
# same archives go into the tools (stage cross, built -fno-exceptions), into the guest's programs
# and into its shared libraries (cc -shared), each of which gets its own copy, statically.
# LIBUNWIND_HIDE_SYMBOLS: the unwinder stays each program's and library's own.
RTI="$B/runtimes-install"
RT_FLAGS="$WASM_FLAGS -fPIC -fwasm-exceptions"
if stage runtimes "$RTI/lib/libc++.a" "$NATIVE/.stamp" "$(record "$B/runtimes.flags" "$RT_FLAGS")"; then
  LOG="$B/runtimes.log"; : > "$LOG"
  rm -rf "$B/runtimes" "$RTI"
  # try_compile builds static libraries here, so link checks always pass: the options naming what
  # musl lacks (__cxa_thread_atexit_impl) are given explicitly.
  cmake -S "$SRC/runtimes" -B "$B/runtimes" -G Ninja \
    --log-level=WARNING \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$RTI" \
    -DCMAKE_SYSTEM_NAME=Linux \
    -DCMAKE_SYSTEM_PROCESSOR=wasm32 \
    -DCMAKE_C_COMPILER="$NATIVE/bin/clang" \
    -DCMAKE_CXX_COMPILER="$NATIVE/bin/clang++" \
    -DCMAKE_AR="$NATIVE/bin/llvm-ar" \
    -DCMAKE_RANLIB="$NATIVE/bin/llvm-ranlib" \
    -DCMAKE_C_COMPILER_TARGET="$WASM_TARGET" \
    -DCMAKE_CXX_COMPILER_TARGET="$WASM_TARGET" \
    "-DCMAKE_C_FLAGS=$RT_FLAGS" \
    "-DCMAKE_CXX_FLAGS=$RT_FLAGS" \
    -DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY \
    -DLLVM_ENABLE_RUNTIMES="libunwind;libcxxabi;libcxx" \
    -DLLVM_INCLUDE_TESTS=OFF \
    -DLLVM_INCLUDE_DOCS=OFF \
    -DLIBUNWIND_ENABLE_SHARED=OFF \
    -DLIBUNWIND_ENABLE_STATIC=ON \
    -DLIBUNWIND_INCLUDE_TESTS=OFF \
    -DLIBUNWIND_ENABLE_THREADS=ON \
    -DLIBUNWIND_USE_COMPILER_RT=OFF \
    -DLIBUNWIND_HIDE_SYMBOLS=ON \
    -DLIBCXXABI_ENABLE_SHARED=OFF \
    -DLIBCXXABI_ENABLE_STATIC=ON \
    -DLIBCXXABI_INCLUDE_TESTS=OFF \
    -DLIBCXXABI_ENABLE_EXCEPTIONS=ON \
    -DLIBCXXABI_USE_LLVM_UNWINDER=ON \
    -DLIBCXXABI_ENABLE_STATIC_UNWINDER=ON \
    -DLIBCXXABI_STATICALLY_LINK_UNWINDER_IN_STATIC_LIBRARY=OFF \
    -DLIBCXXABI_HAS_PTHREAD_API=ON \
    -DLIBCXXABI_HAS_CXA_THREAD_ATEXIT_IMPL=OFF \
    -DLIBCXX_ENABLE_SHARED=OFF \
    -DLIBCXX_ENABLE_STATIC=ON \
    -DLIBCXX_INCLUDE_TESTS=OFF \
    -DLIBCXX_INCLUDE_BENCHMARKS=OFF \
    -DLIBCXX_ENABLE_EXCEPTIONS=ON \
    -DLIBCXX_ENABLE_FILESYSTEM=ON \
    -DLIBCXX_HAS_MUSL_LIBC=ON \
    -DLIBCXX_CXX_ABI=libcxxabi \
    -DLIBCXX_STATICALLY_LINK_ABI_IN_STATIC_LIBRARY=OFF \
    >> "$LOG" 2>&1 || { tail -30 "$LOG"; log_file "$LOG"; exit 1; }
  "$NINJA" -C "$B/runtimes" -j"$JOBS" install >> "$LOG" 2>&1 || { tail -30 "$LOG"; log_file "$LOG"; exit 1; }
  # libunwind's register save/restore are assembly for real CPUs: empty, non-wasm objects here
  for lib in "$RTI"/lib/*.a; do
    if "$NATIVE/bin/llvm-ar" t "$lib" | grep -q '\.S\.o$'; then
      "$NATIVE/bin/llvm-ar" d "$lib" $("$NATIVE/bin/llvm-ar" t "$lib" | grep '\.S\.o$')
    fi
  done
  no_legacy_eh "$RTI"/lib/*.a
  rm -rf "$B/runtimes"   # the objects; the install is what the next stages use
fi

# ── stage cross: LLVM tools for wasm32 ────────────────────────────────────────
CROSS_BUILD="$B/cross"
# clang is the largest indicator that the cross build completed
CROSS_CLANG="$CROSS_BUILD/bin/clang"

WASM_LINK_FLAGS="--target=$WASM_TARGET -nostdlib -Wl,--table-base=3 -Wl,--import-memory -Wl,--max-memory=4294967296 -Wl,--shared-memory -Wl,--export-table -Wl,-z,stack-size=8388608 -L$SYSROOT/lib"
LIBCXX_FLAGS="-nostdinc++ -isystem $RTI/include/c++/v1"
LIBCXX_LINK="-L$RTI/lib -lc++ -lc++abi -lunwind"

# What the cross build is configured with; a change reconfigures the existing tree (and ninja then
# rebuilds what it changes). LLVM_FORCE_VC_*: the source export sits in collaboCore's work tree, whose
# git repository LLVM would otherwise take for its own (`clang --version` named collaboCore's).
CROSS_CONFIG=(
  -DCMAKE_TOOLCHAIN_FILE="$A/cmake/wasm32-toolchain.cmake"
  -DWASM_SYSROOT="$SYSROOT"
  -DWASM_BIN="$NATIVE/bin"
  -DCMAKE_BUILD_TYPE=Release
  -DCMAKE_INSTALL_PREFIX="$CROSS_BUILD/install"
  "-DCMAKE_CXX_FLAGS=$WASM_FLAGS -fno-exceptions $LIBCXX_FLAGS"
  "-DCMAKE_C_FLAGS=$WASM_FLAGS"
  "-DCMAKE_EXE_LINKER_FLAGS=$WASM_LINK_FLAGS $SYSROOT/lib/crt1.o"
  "-DCMAKE_CXX_STANDARD_LIBRARIES=$LIBCXX_LINK -lc $RT_LIB"
  "-DCMAKE_C_STANDARD_LIBRARIES=$LIBCXX_LINK -lc $RT_LIB"
  -DLLVM_PARALLEL_LINK_JOBS=1
  -DLLVM_ENABLE_PROJECTS="$LLVM_PROJECTS"
  -DLLVM_TARGETS_TO_BUILD="$LLVM_TARGETS"
  -DLLVM_DEFAULT_TARGET_TRIPLE="$WASM_TRIPLE_LINUX"
  -DLLVM_TABLEGEN="$NATIVE_TBLGEN"
  -DCLANG_TABLEGEN="$CLANG_TBLGEN"
  -DLLVM_NATIVE_TOOL_DIR="$NATIVE/bin"
  -DLLVM_BUILD_TOOLS=ON
  -DLLVM_BUILD_LLVM_DYLIB=OFF
  -DLLVM_LINK_LLVM_DYLIB=OFF
  -DLLVM_ENABLE_ZLIB=OFF
  -DLLVM_ENABLE_ZSTD=OFF
  -DLLVM_ENABLE_LIBXML2=OFF
  -DLLVM_ENABLE_LIBEDIT=OFF
  -DLLVM_ENABLE_CURL=OFF
  -DLLVM_ENABLE_PIC=OFF
  -DCMAKE_SKIP_RPATH=ON
  -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON
  -DLLVM_INCLUDE_TESTS=OFF
  -DLLVM_INCLUDE_DOCS=OFF
  -DLLVM_INCLUDE_EXAMPLES=OFF
  -DLLVM_INCLUDE_BENCHMARKS=OFF
  -DLLVM_BUILD_RUNTIME=OFF
  -DCLANG_ENABLE_STATIC_ANALYZER=OFF
  -DCLANG_BUILD_TOOLS=ON
  -DCLANG_TOOL_CLANG_FORMAT_BUILD=OFF
  -DCLANG_TOOL_CLANG_CHECK_BUILD=OFF
  -DCLANG_TOOL_CLANG_DIFF_BUILD=OFF
  -DCLANG_TOOL_CLANG_EXTDEF_MAPPING_BUILD=OFF
  -DCLANG_TOOL_CLANG_IMPORT_TEST_BUILD=OFF
  -DCLANG_TOOL_CLANG_REFACTOR_BUILD=OFF
  -DCLANG_TOOL_CLANG_REPL_BUILD=OFF
  -DCLANG_TOOL_CLANG_SCAN_DEPS_BUILD=OFF
  -DLLVM_ENABLE_ASSERTIONS=OFF
  -DLLVM_OPTIMIZED_TABLEGEN=ON
  -DLLVM_HOST_TRIPLE="$WASM_TRIPLE_LINUX"
  -DBUILD_SHARED_LIBS=OFF
  -DLLVM_TOOL_LTO_BUILD=OFF
  -DLLVM_TOOL_LLVM_LTO_BUILD=OFF
  -DLLVM_TOOL_LLVM_LTO2_BUILD=OFF
  -DLLVM_TOOL_LLVM_DRIVER_BUILD=OFF
  -DLLVM_FORCE_VC_REPOSITORY=https://github.com/llvm/llvm-project.git
  -DLLVM_FORCE_VC_REVISION="$LLVM_REV"
)
CROSS_RECORD="$(record "$B/cross.config" "${CROSS_CONFIG[*]}")"

if stage cross "$CROSS_BUILD/.stamp" "$SRC/.stamp" "$RTI/lib/libc++.a" "$CROSS_RECORD"; then
  LOG="$B/cross.log"
  # Configured when there is no tree yet or its configuration changed; otherwise ninja rebuilds
  # incrementally (after a patch, say).
  if [[ ! -f "$CROSS_BUILD/build.ninja" || "$CROSS_RECORD" -nt "$CROSS_BUILD/build.ninja" ]]; then
    cmake -S "$SRC/llvm" -B "$CROSS_BUILD" -G Ninja --log-level=WARNING "${CROSS_CONFIG[@]}" \
      > "$B/cross-configure.log" 2>&1 || { tail -30 "$B/cross-configure.log"; log_file "$B/cross-configure.log"; exit 1; }
    touch "$CROSS_BUILD/build.ninja"
  fi
  # shellcheck disable=SC2086
  "$NINJA" -C "$CROSS_BUILD" -j"$JOBS" ${NINJA_FLAGS:-} ${LLVM_TOOLS//;/ } \
    > "$LOG" 2>&1 || { tail -30 "$LOG"; log_file "$LOG"; exit 1; }
  touch "$CROSS_BUILD/.stamp"
fi

# ── stage cpio: the tools, the sysroot, the scripts → out/llvm.cpio ───────────
CPIO_OUT="$OUT/llvm.cpio"
RES="usr/lib/clang/$LLVM_MAJOR"

if stage cpio "$CPIO_OUT" "$CROSS_BUILD/.stamp" "$A"/guest/* "$SYSROOT/lib/libc.a" "$SYSROOT/lib/libc.exports" "$RT_LIB"; then
  STAGE="$B/stage"
  rm -rf "$STAGE"; mkdir -p "$STAGE/usr/bin" "$STAGE/usr/include" "$STAGE/usr/lib" "$STAGE/$RES/lib/$WASM_TARGET"

  for name in ${LLVM_TOOLS//;/ }; do
    src="$CROSS_BUILD/bin/$name"
    [[ -f "$src" ]] || { echo "missing tool binary: $src" >&2; exit 1; }
    echo "  strip: $name"
    "$NATIVE/bin/llvm-objcopy" --strip-all "$src" "$STAGE/usr/bin/$name"
    chmod 755 "$STAGE/usr/bin/$name"
  done
  no_legacy_eh "$STAGE/usr/bin/clang"

  # Their other names (each tool tells what to be by the name it is run as), and the names build
  # systems call the binutils by. `ld` is guest/ld (lld run as `ld` would be the ELF linker).
  for l in clang++=clang ld.lld=lld wasm-ld=lld llvm-ranlib=llvm-ar llvm-readelf=llvm-readobj \
           llvm-strip=llvm-objcopy llvm-addr2line=llvm-symbolizer \
           ar=llvm-ar ranlib=llvm-ar nm=llvm-nm strip=llvm-objcopy objcopy=llvm-objcopy \
           objdump=llvm-objdump readelf=llvm-readobj addr2line=llvm-symbolizer; do
    ln -s "${l#*=}" "$STAGE/usr/bin/${l%%=*}"
  done
  # cc, c++ (cc by another name), cpp, ld: what builds the guest's own programs and libraries
  install -m 755 "$A/guest/cc" "$A/guest/cpp" "$A/guest/ld" "$STAGE/usr/bin/"
  ln -s cc "$STAGE/usr/bin/c++"

  # The sysroot is the guest's own /usr: the C library as the userspace step built it (with
  # dlopen, libc.exports), libc++, clang's headers and compiler-rt in clang's resource directory.
  cp -r "$SYSROOT/include/." "$STAGE/usr/include/"
  cp -r "$RTI/include/c++" "$STAGE/usr/include/"
  cp "$SYSROOT"/lib/*.o "$SYSROOT"/lib/*.a "$SYSROOT/lib/libc.exports" "$STAGE/usr/lib/"
  cp "$RTI"/lib/libc++.a "$RTI"/lib/libc++experimental.a "$RTI"/lib/libc++abi.a "$RTI"/lib/libunwind.a "$STAGE/usr/lib/"
  cp -r "$CROSS_BUILD/lib/clang/$LLVM_MAJOR/include" "$STAGE/$RES/"
  cp "$RT_LIB" "$STAGE/$RES/lib/$WASM_TARGET/"
  # compiler-rt for a shared library, without its __c_longjmp: a library must import the program's
  # tag (the one the C library's longjmp throws), not define one its setjmp would wait for in vain
  cp "$RT_LIB" "$STAGE/$RES/lib/$WASM_TARGET/libclang_rt.builtins-shared.a"
  "$NATIVE/bin/llvm-ar" d "$STAGE/$RES/lib/$WASM_TARGET/libclang_rt.builtins-shared.a" __c_longjmp.S.o
  chmod -R u+w,go+rX "$STAGE/usr"

  echo "== packing cpio"
  python3 "$U/mkinitramfs.py" --overlay -o "$CPIO_OUT" --tree "usr=$STAGE/usr"
  rm -rf "$STAGE"   # the image holds it all

  mkdir -p "$OUT/licenses"
  rm -f "$OUT"/licenses/*
  # LLVM, Clang, LLD, libc++, libc++abi, libunwind, compiler-rt: Apache-2.0 with LLVM-exception;
  # musl (libc.a, its headers): MIT; Linux's uapi headers: GPL-2.0 WITH Linux-syscall-note.
  for p in llvm clang lld libcxx libcxxabi libunwind; do
    cp "$SRC/$p/LICENSE.TXT" "$OUT/licenses/$p-LICENSE.txt"
  done
  cp "$U/src/llvm/compiler-rt/LICENSE.TXT" "$OUT/licenses/compiler-rt-LICENSE.txt"
  cp "$U/src/musl/COPYRIGHT" "$OUT/licenses/musl-COPYRIGHT.txt"
  cat "$ROOT/kernel/linux/COPYING" "$ROOT/kernel/linux/LICENSES/exceptions/Linux-syscall-note" \
    > "$OUT/licenses/linux-uapi-headers-LICENSE.txt"

  ls -lh "$CPIO_OUT"
fi
