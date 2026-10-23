#!/usr/bin/env bash
# The imagemagick add-on: ImageMagick 7 (the clone ../org/ImageMagick at IM_REV) for the guest, one
# static wasm32 Linux program with its delegate libraries, packed as out/imagemagick.cpio (an
# overlay):
#
#   /usr/bin/magick                      ImageMagick (Q16 HDRI), and convert, identify, mogrify,
#                                        composite, compare, montage, stream, conjure as links to it
#   /etc/ImageMagick-7                   its configuration (policy, delegates, colors, fonts: type.xml)
#   /usr/share/ImageMagick-7             its messages (locale)
#   /usr/share/fonts/dejavu, …/nanum     fonts: DejaVu (Latin, Greek, Cyrillic), Nanum Gothic and
#                                        Myeongjo (Korean)
#   /usr/share/doc/imagemagick           the agents' guide (AGENTS.md)
#   /etc/codex/skills/imagemagick        the guide as a skill, for Codex (its system skills),
#   /root/.agent/skills/imagemagick      agent (the agent add-on: the guest user's skills) and
#   /root/.claude/skills/imagemagick     Claude Code (the guest user's skills; the node add-on runs it)
#
# Stages (each skipped when its output is up to date, i.e. newer than what it is made from):
#   src       ImageMagick at IM_REV exported from the clone into build/src, a git repository whose
#             first commit is the pristine export, with patches/imagemagick/*.patch applied
#   deps      the delegate libraries sources.lock names, static, in build/<target>/deps
#   magick    ImageMagick configured and built for <target>, installed into build/<target>/root
#   fonts     the fonts sources.lock names
#   cpio      out/imagemagick.cpio, out/imagemagick.json, out/licenses
#
# <target> is wasm (the guest's program); with REF=1 also native: the same sources for this
# machine (clang-19, no FMA contraction), the reference tests/ compares the guest's output with.
#
# Needs: ./build.sh userspace (musl sysroot, compiler-rt) and tools (cmake, ninja), the clone at
# ../org/ImageMagick (beside CollaboCore; never modified), network access for the first downloads.
#
#   FORCE=stage[,stage]   redo those stages: src deps magick fonts cpio (deps/magick: both targets)
#   JOBS=N                parallel jobs (default: the number of CPUs)
#   REF=1                 also build the native reference (build/native)
set -euo pipefail
A="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$A/../.." && pwd)"
U="$ROOT/userspace"
B="$A/build"
OUT="$A/out"
DL="$B/dl"
IM_SRC="${IM_SRC:-$(cd "$ROOT/.." && pwd)/org/ImageMagick}"
IM_REV=a0c4bf729a26a5c2feb28b0365b47a8db736078a   # tag 7.1.2-34
IM_VERSION=7.1.2-34
JOBS="${JOBS:-$(nproc)}"
FORCE=",${FORCE:-},"
CMAKE="$ROOT/.tools/cmake/bin/cmake"
NINJA="$ROOT/.tools/cmake/bin/ninja"
# What every guest program is linked with (as the other C add-ons): the engine's shared memory,
# up to 4 GiB of it, and an 8 MiB stack.
LINK_FLAGS="-Wl,--import-memory -Wl,--max-memory=4294967296 -Wl,--shared-memory -Wl,--export-table -Wl,-z,stack-size=8388608"

# stage NAME OUTPUT [INPUT...]: true when the stage must run (forced, the output missing, or an
# input newer than it).
stage() {
  local name=$1 out=$2 src; shift 2
  if [[ "$FORCE" != *",$name,"* && -e "$out" ]]; then
    for src in "$@"; do
      if [[ "$src" -nt "$out" ]]; then echo "== $name: $(basename "$src") changed"; return 0; fi
    done
    echo "== $name: up to date"; return 1
  fi
  echo "== $name"; return 0
}
redo() { FORCE="$FORCE$1,"; }   # NAME: that stage must run (what it depends on was redone)

# log_run LOG COMMAND...: run it with its output in LOG; on failure show the end of LOG and stop.
log_run() {
  local log=$1; shift
  "$@" >> "$log" 2>&1 || { tail -40 "$log"; echo "FAILED: $* (log: $log)" >&2; exit 1; }
}

# fetch NAME: download (once) the file sources.lock names into build/dl and check its sha256;
# prints its path. A file another add-on downloaded already is taken from there.
fetch() {
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
    echo "  fetch $(basename "$file")" >&2
    curl -fL --retry 3 -s -S -o "$file.part" "$url" && mv "$file.part" "$file"
  fi
  echo "$sum  $file" | sha256sum -c --quiet - >&2 || { echo "checksum mismatch: $file (deleted)" >&2; rm -f "$file"; exit 1; }
  echo "$file"
}

# unpack NAME DIR: a fresh copy of the source in DIR/<its top directory>, every file with the same
# mtime (so that no generated file looks older than its input); prints that directory.
unpack() {
  local file top
  file="$(fetch "$1")"
  mkdir -p "$2"
  top="$(python3 -I -c 'import sys, tarfile; print(tarfile.open(sys.argv[1]).next().name.split("/")[0])' "$file")"
  # (Python unpacks: this machine has no bzip2 for tar -j)
  rm -rf "${2:?}/$top"; python3 -I -c 'import sys, tarfile; tarfile.open(sys.argv[1]).extractall(sys.argv[2], filter="tar")' "$file" "$2"
  find "$2/$top" -exec touch -h -d "@$(date +%s)" {} +
  echo "$2/$top"
}

# apply_patches DIR PATCHDIR: every PATCHDIR/*.patch on DIR, in order.
apply_patches() {
  local p
  for p in "$2"/*.patch; do
    [[ -f "$p" ]] || continue
    patch -d "$1" -p1 --forward --no-backup-if-mismatch --quiet < "$p" || { echo "patch failed: $p" >&2; exit 1; }
  done
}

# record FILE TEXT: FILE holds TEXT, rewritten only when it changes; prints FILE (a stage's input for
# what is no file: the patches a stage applies, by checksum, so that a removed one counts too).
record() {
  mkdir -p "$(dirname "$1")"
  [[ -f "$1" && "$(cat "$1")" == "$2" ]] || printf '%s\n' "$2" > "$1"
  echo "$1"
}
patchset() { record "$2" "$(cd "$1" && sha256sum -- *.patch 2>/dev/null)"; }

[[ -f "$U/sysroot/lib/crt1.o" ]] || { echo "missing userspace/sysroot; run: ./build.sh userspace" >&2; exit 1; }
[[ -x "$CMAKE" && -x "$NINJA" ]] || { echo "missing .tools/cmake; run: ./build.sh tools" >&2; exit 1; }
[[ -d "$IM_SRC/.git" ]] || { echo "missing the clone $IM_SRC (git clone https://github.com/ImageMagick/ImageMagick)" >&2; exit 1; }
mkdir -p "$B" "$OUT"

# ── src: ImageMagick, exported and patched ───────────────────────────────────────────────────────
S="$B/src"
if stage src "$S/.stamp" "$(record "$B/recipes/src" "$IM_REV")" "$(patchset "$A/patches/imagemagick" "$B/patches-imagemagick.sum")" \
     "$A/patches/imagemagick"/*.patch; then
  git -C "$IM_SRC" cat-file -e "$IM_REV^{commit}" 2>/dev/null \
    || { echo "the clone has no commit $IM_REV (git -C $IM_SRC fetch --tags)" >&2; exit 1; }
  rm -rf "$S"; mkdir -p "$S"
  git -C "$IM_SRC" archive "$IM_REV" | tar -xm -C "$S"
  git -C "$S" init -q && git -C "$S" add -A -f
  git -C "$S" -c user.name=build -c user.email=build@localhost commit -q -m pristine
  for p in "$A/patches/imagemagick"/*.patch; do
    [[ -f "$p" ]] || continue
    echo "  patch $(basename "$p")"
    git -C "$S" apply --whitespace=nowarn "$p"
  done
  # one time for every file: configure and the Makefile.in must not look older than their sources
  # (there is no autoconf/automake here to remake them)
  find "$S" -path "$S/.git" -prune -o -exec touch -h -d "@$(date +%s)" {} +
  touch "$S/.stamp"
  redo magick
fi

# ── llvm: the parts of LLVM 19.1.7 (clang-19's release) the build takes: the OpenMP runtime
# (libomp, with patches/openmp), and libc++ with libc++abi and libunwind (the C++ libraries:
# libheif, libde265; libomp's headers) ───────────────────────────────────────────────────────────
LLVM_SRC="$B/llvm-src"
LLVM_DIRS=(openmp runtimes cmake llvm/cmake llvm/utils/llvm-lit libcxx libcxxabi libunwind third-party)
if stage llvm "$LLVM_SRC/.done" "$(record "$B/locks/llvm" "$(grep -E '^llvm ' "$A/sources.lock") ${LLVM_DIRS[*]}")" \
     "$(patchset "$A/patches/openmp" "$B/patches-openmp.sum")"; then
  rm -rf "$LLVM_SRC"; mkdir -p "$LLVM_SRC"
  tar -xJmf "$(fetch llvm)" -C "$LLVM_SRC" --strip-components=1 \
    "${LLVM_DIRS[@]/#/llvm-project-19.1.7.src/}"
  apply_patches "$LLVM_SRC/openmp" "$A/patches/openmp"
  touch "$LLVM_SRC/.done"
  redo deps-wasm; redo deps-native
fi

# ── the targets ──────────────────────────────────────────────────────────────────────────────────
# target_env TARGET: sets CC, CFLAGS, the configure and cmake arguments for it, and T (its folder).
target_env() {
  T="$B/$1"
  case "$1" in
    wasm)
      CC="$A/tools/wasm-cc"; CXX="$A/tools/wasm-c++"; CFLAGS="-O2"; LDFLAGS="$LINK_FLAGS"
      HOST_ARGS=(--host=wasm32-unknown-linux-musl --build="$(uname -m)-pc-linux-gnu")
      CONFIG_SITE_FILE="$ROOT/python/config.site"
      CMAKE_ARGS=(-DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR=wasm32)
      FIND_ROOTS="$T/deps" ;;
    native)
      # no fused multiply-add (the guest has none): the same arithmetic as the guest's
      CC=clang-19; CXX=clang++-19; CFLAGS="-O2 -ffp-contract=off"; LDFLAGS="-fuse-ld=lld-19"
      HOST_ARGS=(); CONFIG_SITE_FILE=/dev/null; CMAKE_ARGS=()
      FIND_ROOTS="$T/deps;/" ;;   # the C library's libm is this machine's
  esac
}

# cmake_lib SOURCE BUILD [ARG...]: a CMake project for the target, static, installed into $D.
cmake_lib() {
  local src=$1 build=$2; shift 2
  "$CMAKE" -S "$src" -B "$build" -G Ninja -Wno-dev --log-level=WARNING -DCMAKE_MAKE_PROGRAM="$NINJA" \
    -DCMAKE_BUILD_TYPE=Release "${CMAKE_ARGS[@]}" -DCMAKE_C_COMPILER="$CC" -DCMAKE_CXX_COMPILER="$CXX" \
    -DCMAKE_AR=/usr/bin/llvm-ar-19 -DCMAKE_RANLIB=/usr/bin/llvm-ranlib-19 \
    -DCMAKE_C_FLAGS_RELEASE=-DNDEBUG -DCMAKE_C_FLAGS="$CFLAGS" -DCMAKE_CXX_FLAGS_RELEASE=-DNDEBUG -DCMAKE_CXX_FLAGS="$CFLAGS" \
    -DCMAKE_EXE_LINKER_FLAGS="$LDFLAGS" \
    -DCMAKE_INSTALL_PREFIX="$D" -DCMAKE_INSTALL_LIBDIR=lib -DBUILD_SHARED_LIBS=OFF \
    -DCMAKE_POSITION_INDEPENDENT_CODE=OFF -DCMAKE_PREFIX_PATH="$D" -DCMAKE_SKIP_RPATH=ON \
    -DCMAKE_FIND_ROOT_PATH="$FIND_ROOTS" -DCMAKE_FIND_ROOT_PATH_MODE_PROGRAM=NEVER \
    -DCMAKE_FIND_ROOT_PATH_MODE_LIBRARY=ONLY -DCMAKE_FIND_ROOT_PATH_MODE_INCLUDE=ONLY \
    -DCMAKE_FIND_ROOT_PATH_MODE_PACKAGE=ONLY "$@"
  "$NINJA" -C "$build" -j"$JOBS" install
}

# configure_lib SOURCE [ARG...]: an autoconf configure for the target (in SOURCE), static, into $D.
configure_lib() {
  local src=$1; shift
  (cd "$src" && CONFIG_SITE="$CONFIG_SITE_FILE" ./configure "${HOST_ARGS[@]}" --prefix="$D" \
     --disable-shared --enable-static CC="$CC" AR=llvm-ar-19 RANLIB=llvm-ranlib-19 \
     CFLAGS="$CFLAGS" LDFLAGS="$LDFLAGS" PKG_CONFIG_LIBDIR="$D/lib/pkgconfig" "$@")
}

# ── deps: the delegate libraries ─────────────────────────────────────────────────────────────────
# zlib, liblzma (TIFF, MIFF and XZ compression), libpng, libjpeg-turbo (no SIMD: assembly for
# x86/Arm/…), libtiff (with JPEG, Deflate, LZMA and WebP compression), libwebp (with its mux and
# demux: animated WebP), OpenJPEG (JPEG 2000), FreeType (text: TrueType/OpenType fonts; with zlib,
# without PNG glyphs, HarfBuzz, Brotli), Little CMS (ICC profiles, -profile, -intent), libxml2
# (SVG through ImageMagick's own renderer (MSVG), XMP), libheif (HEIC and AVIF) with libde265 (HEVC
# decoding) and libaom (AV1, generic C: decoding and encoding). Their pkg-config files are what
# ImageMagick's configure reads (pkg-config --static). First the runtimes: the C++ libraries
# build on libc++.
DEPS=(zlib xz libpng jpegturbo libwebp tiff openjpeg freetype lcms2 libxml2 libde265 libaom libheif)
# The flags of the guest's clang for the LLVM runtimes, which build with clang-19 itself (as the nmap
# and jdk add-ons build libc++): wasm-cc's, with the sysroot.
WASM_RT_FLAGS="--target=wasm32-unknown-unknown -matomics -mbulk-memory -mllvm -wasm-enable-sjlj -D__linux__=1 -D__linux=1 -D__unix__=1 -D__unix=1 -D__gnu_linux__=1 -Wno-unused-command-line-argument -Wno-override-module --sysroot=$U/sysroot"
build_deps() {
  target_env "$1"
  D="$T/deps"
  local lock_inputs=() name
  for name in "${DEPS[@]}"; do lock_inputs+=("$(record "$B/locks/$name" "$(grep -E "^$name " "$A/sources.lock")")"); done
  stage "deps-$1" "$D/.done" "$(record "$B/recipes/deps-$1" "$(declare -f target_env build_deps cmake_lib configure_lib build_runtimes)")" "${lock_inputs[@]}" "$LLVM_SRC/.done" \
    "$(patchset "$A/patches/freetype" "$B/patches-freetype.sum")" || return 0
  rm -rf "$T/deps-src" "$T/deps-build" "$D"; mkdir -p "$T/deps-src" "$T/deps-build" "$D/lib/pkgconfig" "$D/include"
  local LOG="$T/deps.log" S; : > "$LOG"
  build_runtimes "$1"

  S="$(unpack zlib "$T/deps-src")"
  # zlib's own configure knows no cross compilers by --host; CHOST names the tools
  (cd "$S" && log_run "$LOG" env CC="$CC" CFLAGS="$CFLAGS" AR=llvm-ar-19 RANLIB=llvm-ranlib-19 \
     ./configure --static --prefix="$D")
  log_run "$LOG" make -C "$S" -j"$JOBS" libz.a
  log_run "$LOG" make -C "$S" install

  S="$(unpack xz "$T/deps-src")"     # liblzma alone, with distro's patch (the signal mask musl has)
  patch -d "$S" -p1 --quiet < "$ROOT/third_party/distro/distro/xz/wasm-sigmask.patch"
  log_run "$LOG" configure_lib "$S" --disable-nls --disable-threads --disable-sandbox --disable-doc \
    --disable-scripts --disable-xz --disable-xzdec --disable-lzmadec --disable-lzmainfo --disable-lzma-links
  log_run "$LOG" make -C "$S/src/liblzma" -j"$JOBS" install

  S="$(unpack libpng "$T/deps-src")"
  log_run "$LOG" cmake_lib "$S" "$T/deps-build/png" -DPNG_SHARED=OFF -DPNG_STATIC=ON -DPNG_TESTS=OFF \
    -DPNG_TOOLS=OFF -DPNG_HARDWARE_OPTIMIZATIONS=OFF -DPNG_FRAMEWORK=OFF -DZLIB_ROOT="$D"

  S="$(unpack jpegturbo "$T/deps-src")"
  log_run "$LOG" cmake_lib "$S" "$T/deps-build/jpeg" -DENABLE_SHARED=OFF -DENABLE_STATIC=ON \
    -DWITH_SIMD=OFF -DWITH_TURBOJPEG=OFF -DWITH_TOOLS=OFF -DWITH_TESTS=OFF

  S="$(unpack libwebp "$T/deps-src")"
  log_run "$LOG" cmake_lib "$S" "$T/deps-build/webp" -DWEBP_ENABLE_SIMD=OFF -DWEBP_BUILD_ANIM_UTILS=OFF \
    -DWEBP_BUILD_CWEBP=OFF -DWEBP_BUILD_DWEBP=OFF -DWEBP_BUILD_GIF2WEBP=OFF -DWEBP_BUILD_IMG2WEBP=OFF \
    -DWEBP_BUILD_VWEBP=OFF -DWEBP_BUILD_WEBPINFO=OFF -DWEBP_BUILD_WEBPMUX=OFF -DWEBP_BUILD_EXTRAS=OFF \
    -DWEBP_BUILD_LIBWEBPMUX=ON -DWEBP_USE_THREAD=ON

  S="$(unpack tiff "$T/deps-src")"
  log_run "$LOG" cmake_lib "$S" "$T/deps-build/tiff" -Dtiff-tools=OFF -Dtiff-tests=OFF \
    -Dtiff-contrib=OFF -Dtiff-docs=OFF -Dtiff-deprecated=OFF -Dtiff-cxx=OFF \
    -Djpeg=ON -Dzlib=ON -Dlzma=ON -Dwebp=ON -Dzstd=OFF -Djbig=OFF -Dlerc=OFF -Dlibdeflate=OFF \
    -DJPEG_INCLUDE_DIR="$D/include" -DJPEG_LIBRARY="$D/lib/libjpeg.a" \
    -DZLIB_INCLUDE_DIR="$D/include" -DZLIB_LIBRARY="$D/lib/libz.a" \
    -DLIBLZMA_INCLUDE_DIR="$D/include" -DLIBLZMA_LIBRARY="$D/lib/liblzma.a" -DWebP_DIR="$D/share/WebP/cmake"

  S="$(unpack openjpeg "$T/deps-src")"
  log_run "$LOG" cmake_lib "$S" "$T/deps-build/openjpeg" -DBUILD_STATIC_LIBS=ON -DBUILD_CODEC=OFF \
    -DBUILD_TESTING=OFF -DBUILD_DOC=OFF

  S="$(unpack freetype "$T/deps-src")"
  [[ "$1" == wasm ]] && apply_patches "$S" "$A/patches/freetype"
  log_run "$LOG" cmake_lib "$S" "$T/deps-build/freetype" -DFT_DISABLE_HARFBUZZ=ON -DFT_DISABLE_PNG=ON \
    -DFT_DISABLE_BROTLI=ON -DFT_DISABLE_BZIP2=ON -DFT_REQUIRE_ZLIB=ON \
    -DZLIB_INCLUDE_DIR="$D/include" -DZLIB_LIBRARY="$D/lib/libz.a"

  S="$(unpack lcms2 "$T/deps-src")"
  log_run "$LOG" configure_lib "$S" --without-jpeg --without-tiff --without-zlib
  log_run "$LOG" make -C "$S/src" -j"$JOBS" install
  log_run "$LOG" make -C "$S" install-data   # lcms2.h, lcms2.pc

  S="$(unpack libxml2 "$T/deps-src")"
  log_run "$LOG" cmake_lib "$S" "$T/deps-build/libxml2" -DLIBXML2_WITH_PYTHON=OFF -DLIBXML2_WITH_ICU=OFF \
    -DLIBXML2_WITH_ZLIB=ON -DLIBXML2_WITH_LZMA=OFF -DLIBXML2_WITH_MODULES=OFF -DLIBXML2_WITH_PROGRAMS=OFF \
    -DLIBXML2_WITH_TESTS=OFF -DLIBXML2_WITH_READLINE=OFF -DLIBXML2_WITH_HISTORY=OFF -DLIBXML2_WITH_ICONV=ON \
    -DLIBXML2_WITH_THREADS=ON -DLIBXML2_WITH_HTTP=OFF -DLIBXML2_WITH_DOCS=OFF \
    -DZLIB_INCLUDE_DIR="$D/include" -DZLIB_LIBRARY="$D/lib/libz.a"

  S="$(unpack libde265 "$T/deps-src")"
  log_run "$LOG" cmake_lib "$S" "$T/deps-build/libde265" -DENABLE_SDL=OFF -DENABLE_DECODER=OFF \
    -DENABLE_ENCODER=OFF -DDISABLE_SSE=ON
  S="$(unpack libaom "$T/deps-src")"
  log_run "$LOG" cmake_lib "$S" "$T/deps-build/aom" -DAOM_TARGET_CPU=generic -DCONFIG_RUNTIME_CPU_DETECT=0 \
    -DENABLE_DOCS=0 -DENABLE_APPS=0 -DENABLE_EXAMPLES=0 -DENABLE_TESTS=0 -DENABLE_TOOLS=0 -DENABLE_TESTDATA=0 \
    -DCONFIG_MULTITHREAD=1 -DCONFIG_TUNE_VMAF=0 -DCONFIG_TUNE_BUTTERAUGLI=0
  S="$(unpack libheif "$T/deps-src")"
  log_run "$LOG" cmake_lib "$S" "$T/deps-build/libheif" -DENABLE_PLUGIN_LOADING=OFF -DWITH_LIBDE265=ON \
    -DWITH_X265=OFF -DWITH_AOM_DECODER=ON -DWITH_AOM_ENCODER=ON -DWITH_DAV1D=OFF -DWITH_RAV1E=OFF \
    -DWITH_SvtEnc=OFF -DWITH_KVAZAAR=OFF -DWITH_X264=OFF -DWITH_OpenH264_DECODER=OFF \
    -DWITH_JPEG_DECODER=OFF -DWITH_JPEG_ENCODER=OFF -DWITH_OpenJPEG_DECODER=OFF \
    -DWITH_OpenJPEG_ENCODER=OFF -DWITH_FFMPEG_DECODER=OFF -DWITH_OPENJPH_ENCODER=OFF \
    -DWITH_UNCOMPRESSED_CODEC=OFF -DWITH_LIBSHARPYUV=OFF -DWITH_EXAMPLES=OFF -DWITH_GDK_PIXBUF=OFF \
    -DBUILD_TESTING=OFF -DBUILD_DOCUMENTATION=OFF -DWITH_FUZZERS=OFF
  if [[ "$1" == wasm ]]; then
    # its pkg-config file names the C++ library of the machine it was configured on (-lstdc++)
    sed -i 's/-lstdc++//g; s/-lc++//g' "$D/lib/pkgconfig/libheif.pc"
  fi

  touch "$D/.done"
  redo "magick-$1"
}

# build_runtimes TARGET: for wasm libc++ (natively the C++ library is this machine's), and libomp,
# the OpenMP runtime (static, without its tool interfaces OMPT/OMPD, hwloc or ITT), into $D.
build_runtimes() {
  local cxxflags=""
  local args=(-DCMAKE_MAKE_PROGRAM="$NINJA" -G Ninja --log-level=WARNING -DCMAKE_BUILD_TYPE=Release
              -DCMAKE_AR=/usr/bin/llvm-ar-19 -DCMAKE_RANLIB=/usr/bin/llvm-ranlib-19
              -DCMAKE_C_COMPILER=/usr/bin/clang-19 -DCMAKE_CXX_COMPILER=/usr/bin/clang++-19)
  if [[ "$1" == wasm ]]; then
    args+=(-DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR=wasm32
           -DCMAKE_C_COMPILER_TARGET=wasm32-unknown-unknown -DCMAKE_CXX_COMPILER_TARGET=wasm32-unknown-unknown
           -DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY)
    # with C++ exceptions (libheif turns exceptions, out of memory among them, into errors), as
    # WebAssembly exception handling; libunwind's Unwind-wasm.c is the unwinder, merged into
    # libc++abi.a (as the python-extension add-on builds it, without -fPIC)
    log_run "$LOG" "$CMAKE" "${args[@]}" -S "$LLVM_SRC/runtimes" -B "$T/deps-build/libcxx" \
      -DCMAKE_INSTALL_PREFIX="$D" "-DCMAKE_C_FLAGS=$WASM_RT_FLAGS -fwasm-exceptions" \
      "-DCMAKE_CXX_FLAGS=$WASM_RT_FLAGS -fwasm-exceptions" \
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
    log_run "$LOG" "$NINJA" -C "$T/deps-build/libcxx" -j"$JOBS" install
    # libunwind's register save/restore are assembly for real CPUs: empty, non-wasm objects here
    local lib
    for lib in "$D"/lib/libc++abi.a "$D"/lib/libunwind.a; do
      [[ -f "$lib" ]] && llvm-ar-19 d "$lib" UnwindRegistersRestore.S.o UnwindRegistersSave.S.o 2>/dev/null || true
    done
    args+=("-DCMAKE_C_FLAGS=$WASM_RT_FLAGS" "-DCMAKE_ASM_FLAGS=$WASM_RT_FLAGS"
           "-DCMAKE_CXX_FLAGS=$WASM_RT_FLAGS -nostdinc++ -isystem $D/include/c++/v1" -DLIBOMP_ARCH=wasm32)
  else
    args+=("-DCMAKE_C_FLAGS=$CFLAGS" "-DCMAKE_CXX_FLAGS=$CFLAGS")
  fi
  log_run "$LOG" "$CMAKE" "${args[@]}" -S "$LLVM_SRC/openmp" -B "$T/deps-build/openmp" \
    -DCMAKE_INSTALL_PREFIX="$D" -DOPENMP_STANDALONE_BUILD=ON -DLIBOMP_ENABLE_SHARED=OFF \
    -DLIBOMP_OMPT_SUPPORT=OFF -DLIBOMP_OMPD_SUPPORT=OFF -DLIBOMP_USE_HWLOC=OFF -DLIBOMP_USE_ITT_NOTIFY=OFF \
    -DLIBOMP_FORTRAN_MODULES=OFF -DLIBOMP_INSTALL_ALIASES=OFF -DOPENMP_ENABLE_LIBOMPTARGET=OFF \
    -DOPENMP_ENABLE_OMPT_TOOLS=OFF -DOPENMP_ENABLE_TESTING=OFF
  log_run "$LOG" "$NINJA" -C "$T/deps-build/openmp" -j"$JOBS" install
}

# ── magick: ImageMagick for the target ───────────────────────────────────────────────────────────
# Q16 with HDRI (ImageMagick 7's default), every coder built in (no modules), OpenMP (libomp: as
# many threads as the guest has CPUs), no X11, Perl, Magick++ or OpenCL. (configure's OpenMP check
# links without the runtime: its answer is given.) Delegates not built here are off explicitly, so that
# nothing of this machine's is ever picked up.
build_magick() {
  target_env "$1"
  D="$T/deps"
  local IMB="$T/im" ROOTDIR="$T/root"
  stage "magick-$1" "$ROOTDIR/.done" "$S/.stamp" "$D/.done" \
    "$(record "$B/recipes/magick-$1" "$(declare -f target_env build_magick)")" "$A/tools/wasm-cc" || return 0
  rm -rf "$IMB" "$ROOTDIR"; mkdir -p "$IMB"
  local LOG="$T/magick.log"; : > "$LOG"
  (cd "$IMB" && log_run "$LOG" env CONFIG_SITE="$CONFIG_SITE_FILE" PKG_CONFIG="pkg-config --static" \
     ac_cv_prog_c_openmp=-fopenmp ac_cv_prog_cxx_openmp=-fopenmp \
     PKG_CONFIG_LIBDIR="$D/lib/pkgconfig" PKG_CONFIG_PATH= \
     "$S/configure" "${HOST_ARGS[@]}" --prefix=/usr --sysconfdir=/etc --localstatedir=/var \
     CC="$CC" CXX="$CXX" AR=llvm-ar-19 RANLIB=llvm-ranlib-19 CFLAGS="$CFLAGS" CXXFLAGS="$CFLAGS" \
     LDFLAGS="$LDFLAGS -L$D/lib" CPPFLAGS="-I$D/include" \
     --disable-shared --enable-static --without-modules --enable-openmp --disable-opencl \
     --disable-docs --disable-dpc --without-x --without-perl --without-magick-plus-plus \
     --with-quantum-depth=16 --enable-hdri --with-security-policy=open \
     --with-zlib --with-lzma --with-png --with-jpeg --with-tiff --with-webp --with-openjp2 \
     --with-freetype --with-lcms --with-xml --with-dejavu-font-dir=/usr/share/fonts/dejavu \
     --without-bzlib --without-zstd --without-zip --without-djvu --without-fftw --without-fontconfig \
     --with-heic --without-raqm --without-jbig --without-jxl --without-lqr --without-openexr \
     --without-pango --without-raw --without-rsvg --without-wmf --without-gvc --without-dps \
     --without-fpx --without-flif --without-uhdr --without-gslib --without-autotrace --without-dmr \
     --without-jemalloc --without-tcmalloc --without-umem --without-mtmalloc)
  sed -n '/^ImageMagick is configured/,$p' "$LOG" | sed -n '/Delegate Library/,/^$/p' | head -50
  log_run "$LOG" make -C "$IMB" -j"$JOBS"
  # and the programs of its own checks (make check's: tests/run.sh runs them, in the guest)
  log_run "$LOG" make -C "$IMB" -j"$JOBS" tests/validate tests/drawtest tests/wandtest
  chmod +x "$IMB"/tests/{validate,drawtest,wandtest} "$IMB/utilities/magick"
  log_run "$LOG" make -C "$IMB" install DESTDIR="$ROOTDIR"
  touch "$ROOTDIR/.done"
  [[ "$1" == wasm ]] && redo cpio
  return 0
}

build_deps wasm
build_magick wasm
if [[ -n "${REF:-}" ]]; then
  build_deps native
  build_magick native
fi

# ── fonts ────────────────────────────────────────────────────────────────────────────────────────
F="$B/fonts"
if stage fonts "$F/.done" "$(record "$B/locks/fonts" "$(grep -E '^(dejavu|nanum-)' "$A/sources.lock")")"; then
  rm -rf "$F"; mkdir -p "$F/dejavu" "$F/nanum" "$F/src"
  dejavu="$(unpack dejavu "$F/src")"
  cp "$dejavu"/ttf/*.ttf "$F/dejavu/"
  cp "$dejavu/LICENSE" "$F/dejavu/LICENSE"
  for font in gothic-regular gothic-bold myeongjo-regular myeongjo-bold; do
    cp "$(fetch "nanum-$font")" "$F/nanum/"
  done
  cp "$(fetch nanum-ofl)" "$F/nanum/OFL.txt"
  rm -rf "$F/src"
  touch "$F/.done"
  redo cpio
fi

# ── config: ImageMagick's configuration files, as the guest has them ─────────────────────────────
# Its own (colors, delegates, log, mime, quantization tables, thresholds, the locale's messages,
# configure.xml for -list configure), the add-on's policy.xml (etc/ImageMagick-7) and type.xml
# (tools/type-xml.py: the fonts by name). config_dir TARGET DIR FONTDIR: all of them in DIR, the
# fonts as at FONTDIR. The native reference reads its own with MAGICK_CONFIGURE_PATH.
config_dir() {
  local root="$B/$1/root" dir=$2 f
  rm -rf "$dir"; mkdir -p "$dir"
  for f in colors delegates log mime quantization-table thresholds; do cp "$root/etc/ImageMagick-7/$f.xml" "$dir/"; done
  cp "$root"/usr/share/ImageMagick-7/*.xml "$dir/"
  cp "$root"/usr/lib/ImageMagick-*/config-*/configure.xml "$dir/"
  cp "$A/etc/ImageMagick-7/policy.xml" "$dir/"
  cp "$S/config/sRGB.icm" "$dir/"   # for -profile: the sRGB profile ImageMagick's sources carry
  python3 "$A/tools/type-xml.py" "$3" "$dir/type.xml"
}

# ── cpio ─────────────────────────────────────────────────────────────────────────────────────────
newest() { find "$@" -type f -printf '%T@ %p\n' 2>/dev/null | sort -n | tail -1 | cut -d' ' -f2-; }
if [[ -n "${REF:-}" ]] && stage config-native "$B/native/config/.done" "$B/native/root/.done" "$F/.done" \
     "$A/tools/type-xml.py" "$(newest "$A/etc")"; then
  config_dir native "$B/native/config" "$F"
  touch "$B/native/config/.done"
fi
R="$B/wasm/root"
CPIO="$OUT/imagemagick.cpio"
if stage cpio "$CPIO" "$R/.done" "$F/.done" "$A/addon.json" "$A/tools/type-xml.py" "$(newest "$A/share" "$A/etc")"; then
  ST="$B/stage"
  rm -rf "$ST"; mkdir -p "$ST/usr/share/ImageMagick-7" "$ST/usr/lib"
  config_dir wasm "$ST/etc/ImageMagick-7" /usr/share/fonts
  # (where the program looks for each: the locale's in the share folder, configure.xml in the lib's)
  mv "$ST"/etc/ImageMagick-7/{english,francais,locale}.xml "$ST/usr/share/ImageMagick-7/"
  cp -r "$R"/usr/lib/ImageMagick-* "$ST/usr/lib/" && rm "$ST/etc/ImageMagick-7/configure.xml"
  # the tools as links to magick, but those of X11 (animate, display, import: not built)
  links=()
  for l in "$R"/usr/bin/*; do
    case "$(basename "$l")" in animate|display|import) continue ;; esac
    if [[ -L "$l" ]]; then links+=(--link "usr/bin/$(basename "$l")=$(readlink "$l")"); fi
  done
  python3 "$U/mkinitramfs.py" --overlay -o "$CPIO" --file usr/bin/magick="$R/usr/bin/magick" "${links[@]}" \
    --tree etc/ImageMagick-7="$ST/etc/ImageMagick-7" \
    --tree usr/share/ImageMagick-7="$ST/usr/share/ImageMagick-7" \
    --tree usr/lib="$ST/usr/lib" \
    --tree usr/share/fonts/dejavu="$F/dejavu" \
    --tree usr/share/fonts/nanum="$F/nanum" \
    --tree usr/share/doc/imagemagick="$A/share/doc" \
    --tree etc/codex/skills/imagemagick="$A/share/skill" \
    --tree root/.claude/skills/imagemagick="$A/share/skill" \
    --tree root/.agent/skills/imagemagick="$A/share/skill"
  cp "$A/addon.json" "$OUT/imagemagick.json"
  rm -rf "$OUT/licenses"; mkdir -p "$OUT/licenses"
  cp "$S/LICENSE" "$OUT/licenses/addon-imagemagick-ImageMagick-LICENSE.txt"
  cp "$S/NOTICE" "$OUT/licenses/addon-imagemagick-ImageMagick-NOTICE.txt"
  for f in 'zlib-*/LICENSE' 'xz-*/COPYING' 'xz-*/COPYING.0BSD' 'libpng-*/LICENSE' 'libjpeg-turbo-*/LICENSE.md' \
           'libwebp-*/COPYING' 'tiff-*/LICENSE.md' 'openjpeg-*/LICENSE' 'freetype-*/LICENSE.TXT' \
           'freetype-*/docs/FTL.TXT' 'lcms2-*/LICENSE' 'libxml2-*/Copyright' 'libheif-*/COPYING' \
           'libde265-*/COPYING' 'libaom-*/LICENSE' 'libaom-*/PATENTS'; do
    path="$(echo "$B"/wasm/deps-src/$f)"
    [[ -f "$path" ]] || { echo "no $f in $B/wasm/deps-src" >&2; rm -f "$CPIO"; exit 1; }
    cp "$path" "$OUT/licenses/addon-imagemagick-${f%%-\**}-$(basename "$path")"
  done
  cp "$LLVM_SRC/openmp/LICENSE.TXT" "$OUT/licenses/addon-imagemagick-libomp-LICENSE.TXT"
  cp "$LLVM_SRC/libcxx/LICENSE.TXT" "$OUT/licenses/addon-imagemagick-libcxx-LICENSE.TXT"
  # libheif and libde265 are LGPL-3.0 (their COPYING says so without the text, which is the
  # LGPL's on top of the GPL's: this machine's copies). Linked statically: their sources and this
  # build are what relinking with other versions needs.
  for l in LGPL-3 GPL-3; do
    [[ -f "/usr/share/common-licenses/$l" ]] || { echo "no /usr/share/common-licenses/$l (the $l text)" >&2; rm -f "$CPIO"; exit 1; }
    cp "/usr/share/common-licenses/$l" "$OUT/licenses/addon-imagemagick-$l.0.txt"
  done
  cp "$F/dejavu/LICENSE" "$OUT/licenses/addon-imagemagick-dejavu-fonts-LICENSE.txt"
  cp "$F/nanum/OFL.txt" "$OUT/licenses/addon-imagemagick-nanum-fonts-OFL.txt"
fi
ls -l "$CPIO" | awk '{print "  " $NF, $5}'
