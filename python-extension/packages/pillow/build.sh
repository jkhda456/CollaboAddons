#!/usr/bin/env bash
# Pillow 12.3 for the guest, with the C libraries its own wheels carry, static, in build/pillow/deps:
#
#   libjpeg-turbo  JPEG (no SIMD: its SIMD is x86/Arm/... assembly)
#   openjpeg       JPEG 2000
#   libtiff        TIFF (with JPEG, zlib, LZMA and WebP compression)
#   libwebp        WebP, animated WebP (webpmux, webpdemux)
#   freetype       ImageFont: TrueType/OpenType fonts (with zlib; without PNG glyphs, HarfBuzz, Brotli)
#   lcms2          ImageCms: colour management
#
#   zlib, liblzma  PNG, TIFF's Deflate and LZMA (the python step's versions: the interpreter's own
#                  copies are neither exported nor position-independent)
#
# Every one static and position-independent, in the modules that use it (a module is a shared
# library: tools/wasm-pyext-cc). Not built: libimagequant (GPL-3:
# ImageQuant quantizing; Pillow's own quantizers remain), raqm/HarfBuzz/FriBiDi (complex text
# layout; ImageFont's basic layout remains, enough for Hangul and Latin), libavif and XCB.
# Pillow itself is built by its setup.py on the host CPython answering for the guest's
# (build/host/bin/python3-guest), platform guessing off so that only these libraries are seen.
# Run by ../../build.sh (packages/README.md); result: build/pillow/site.
set -euo pipefail
source "$PYEXT/tools/lib.sh"
HERE="$PYEXT/packages/pillow"
SITE="$BP/site"
D="$BP/deps"
mkdir -p "$BP"

# ── the libraries ───────────────────────────────────────────────────────────────────────────────
# patches/<library>/*.patch: changes to one of them (patches/pillow: to Pillow)
if stage pillow-deps "$D/.done" $(lock zlib xz jpegturbo openjpeg libwebp tiff freetype lcms2) "$HERE/build.sh" \
     "$(patchset "$HERE/patches/freetype" "$BP/patches-freetype.sum")"; then
  rm -rf "$BP/deps-src" "$BP/deps-build" "$D"; mkdir -p "$BP/deps-src" "$BP/deps-build" "$D"
  LOG="$BP/deps.log"; : > "$LOG"

  zlib_pic "$(unpack zlib "$BP/deps-src")" "$D" "$LOG"

  S="$(unpack xz "$BP/deps-src")"     # liblzma alone, with distro's patch (the signal mask musl has)
  patch -d "$S" -p1 --quiet < "$ROOT/third_party/distro/distro/xz/wasm-sigmask.patch"
  log_run "$LOG" configure_cross "$S" "$D" --disable-nls --disable-threads --disable-sandbox --disable-doc \
    --disable-scripts --disable-xz --disable-xzdec --disable-lzmadec --disable-lzmainfo --disable-lzma-links
  log_run "$LOG" make -C "$S/src/liblzma" -j"$JOBS" install

  S="$(unpack jpegturbo "$BP/deps-src")"
  log_run "$LOG" cmake_cross "$S" "$BP/deps-build/jpeg" "$D" -DENABLE_SHARED=OFF -DENABLE_STATIC=ON \
    -DWITH_SIMD=OFF -DWITH_TURBOJPEG=OFF -DWITH_TOOLS=OFF -DWITH_TESTS=OFF
  log_run "$LOG" "$NINJA" -C "$BP/deps-build/jpeg" -j"$JOBS" install

  S="$(unpack openjpeg "$BP/deps-src")"
  log_run "$LOG" cmake_cross "$S" "$BP/deps-build/openjpeg" "$D" -DBUILD_STATIC_LIBS=ON -DBUILD_CODEC=OFF \
    -DBUILD_TESTING=OFF -DBUILD_DOC=OFF
  log_run "$LOG" "$NINJA" -C "$BP/deps-build/openjpeg" -j"$JOBS" install

  S="$(unpack libwebp "$BP/deps-src")"
  log_run "$LOG" cmake_cross "$S" "$BP/deps-build/webp" "$D" -DWEBP_ENABLE_SIMD=OFF -DWEBP_BUILD_ANIM_UTILS=OFF \
    -DWEBP_BUILD_CWEBP=OFF -DWEBP_BUILD_DWEBP=OFF -DWEBP_BUILD_GIF2WEBP=OFF -DWEBP_BUILD_IMG2WEBP=OFF \
    -DWEBP_BUILD_VWEBP=OFF -DWEBP_BUILD_WEBPINFO=OFF -DWEBP_BUILD_WEBPMUX=OFF -DWEBP_BUILD_EXTRAS=OFF \
    -DWEBP_BUILD_LIBWEBPMUX=ON -DWEBP_USE_THREAD=ON
  log_run "$LOG" "$NINJA" -C "$BP/deps-build/webp" -j"$JOBS" install
  # libwebp needs libsharpyuv, which Pillow does not link (a shared libwebp brings it along)
  llvm-ar-19 qcL "$D/lib/libwebp.a" "$D/lib/libsharpyuv.a" && llvm-ar-19 s "$D/lib/libwebp.a"

  S="$(unpack tiff "$BP/deps-src")"
  log_run "$LOG" cmake_cross "$S" "$BP/deps-build/tiff" "$D" -Dtiff-tools=OFF -Dtiff-tests=OFF \
    -Dtiff-contrib=OFF -Dtiff-docs=OFF -Dtiff-deprecated=OFF -Dcxx=OFF \
    -Djpeg=ON -Dzlib=ON -Dlzma=ON -Dwebp=ON -Dzstd=OFF -Djbig=OFF -Dlerc=OFF -Dlibdeflate=OFF \
    -DJPEG_INCLUDE_DIR="$D/include" -DJPEG_LIBRARY="$D/lib/libjpeg.a" \
    -DZLIB_INCLUDE_DIR="$D/include" -DZLIB_LIBRARY="$D/lib/libz.a" \
    -DLIBLZMA_INCLUDE_DIR="$D/include" -DLIBLZMA_LIBRARY="$D/lib/liblzma.a"
  log_run "$LOG" "$NINJA" -C "$BP/deps-build/tiff" -j"$JOBS" install
  # and libtiff needs libwebp and liblzma, which Pillow does not link into _imaging (a shared
  # libtiff would bring them along)
  llvm-ar-19 qcL "$D/lib/libtiff.a" "$D/lib/libwebp.a" "$D/lib/liblzma.a" && llvm-ar-19 s "$D/lib/libtiff.a"

  S="$(unpack freetype "$BP/deps-src")"
  apply_patches "$S" "$HERE/patches/freetype"
  log_run "$LOG" cmake_cross "$S" "$BP/deps-build/freetype" "$D" -DFT_DISABLE_HARFBUZZ=ON -DFT_DISABLE_PNG=ON \
    -DFT_DISABLE_BROTLI=ON -DFT_DISABLE_BZIP2=ON -DFT_REQUIRE_ZLIB=ON \
    -DZLIB_INCLUDE_DIR="$D/include" -DZLIB_LIBRARY="$D/lib/libz.a"
  log_run "$LOG" "$NINJA" -C "$BP/deps-build/freetype" -j"$JOBS" install
  # and FreeType needs zlib, which Pillow does not link into _imagingft
  llvm-ar-19 qcL "$D/lib/libfreetype.a" "$D/lib/libz.a" && llvm-ar-19 s "$D/lib/libfreetype.a"

  S="$(unpack lcms2 "$BP/deps-src")"
  log_run "$LOG" configure_cross "$S" "$D" --without-jpeg --without-tiff --without-zlib
  log_run "$LOG" make -C "$S/src" -j"$JOBS" install
  log_run "$LOG" make -C "$S" install-data   # lcms2.h

  touch "$D/.done"
  redo pillow
fi

# ── Pillow ──────────────────────────────────────────────────────────────────────────────────────
if stage pillow-src "$BP/src/.stamp" "$(lock pillow)" "$(patchset "$HERE/patches/pillow" "$BP/patches-pillow.sum")"; then
  rm -rf "$BP/src"
  S="$(unpack pillow "$BP/src")"
  apply_patches "$S" "$HERE/patches/pillow"
  touch "$BP/src/.stamp"
  redo pillow
fi
S="$(echo "$BP"/src/pillow-*/)"; S="${S%/}"

if stage pillow "$SITE/.done" "$BP/src/.stamp" "$D/.done" "$HERE/build.sh" "$PYEXT/tools/wasm-pyext-cc" "$ROOT/userspace/bin/wasm-cc"; then
  rm -rf "$S/build" "$SITE"
  LOG="$BP/build.log"; : > "$LOG"
  (cd "$S" && log_run "$LOG" env CC="$PYEXT/tools/wasm-pyext-cc" LDSHARED="$PYEXT/tools/wasm-pyext-cc -shared" \
     CFLAGS="-O2" PKG_CONFIG=/bin/false MAX_CONCURRENCY="$JOBS" \
     ZLIB_ROOT="$D" JPEG_ROOT="$D" JPEG2K_ROOT="$D" TIFF_ROOT="$D" WEBP_ROOT="$D" FREETYPE_ROOT="$D" LCMS_ROOT="$D" \
     "$B/host/bin/python3-guest" setup.py build_clib build_ext --disable-platform-guessing \
       --enable-zlib --enable-jpeg --enable-jpeg2000 --enable-tiff --enable-webp --enable-freetype --enable-lcms \
       --disable-imagequant --disable-raqm --disable-xcb --disable-avif \
     build)
  grep -A20 "PIL SETUP SUMMARY" "$LOG" | sed -n '1,20p'
  mkdir -p "$SITE"
  cp -r "$S"/build/lib.*/PIL "$SITE/PIL"
  # the libraries' licenses beside Pillow's, as licenses/<library>-<file>
  licenses=()
  for f in 'libjpeg-turbo-*/LICENSE.md' 'openjpeg-*/LICENSE' 'tiff-*/LICENSE.md' 'libwebp-*/COPYING' \
           'freetype-*/LICENSE.TXT' 'freetype-*/docs/FTL.TXT' 'lcms2-*/LICENSE' 'zlib-*/LICENSE' 'xz-*/COPYING' 'xz-*/COPYING.0BSD'; do
    path="$(echo "$BP"/deps-src/$f)"
    [[ -f "$path" ]] || { echo "no $f in $BP/deps-src" >&2; exit 1; }
    licenses+=(--license "${f%%-\**}-$(basename "$f")=$path")
  done
  "$HOSTPY" "$PYEXT/tools/dist-info.py" "$S" "$SITE" "${licenses[@]}"
  touch "$SITE/.done"
fi
