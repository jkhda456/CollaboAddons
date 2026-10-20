#!/usr/bin/env bash
# lxml 6.1 for the guest, with the libraries its own wheels carry, static, in build/lxml/deps — the
# versions and configure options of lxml's own static build (buildlibxml.py, which its wheels are
# made with):
#
#   libiconv 1.19   the character encodings libxml2 converts (GNU's, as in the wheels: not the C
#                   library's iconv, which knows fewer)
#   zlib            gzip'ed documents, read and written (Pillow's build of the python step's version)
#   libxml2 2.14.6  with iconv and zlib, without lzma (which lxml's build leaves out)
#   libxslt 1.1.43  and libexslt, without crypto; with the security fixes lxml backports
#                   (libxslt-1.1.43-backport1.patch, in lxml's sdist, applied as lxml applies it)
#
# Each static and position-independent, in the modules that use them (a module is a shared
# library: tools/wasm-pyext-cc). lxml itself is built by its setup.py on the host CPython
# answering for the guest's (build/host/bin/python3-guest) from the C its sdist carries (made by
# Cython: no Cython here), against those libraries (XML2_CONFIG, XSLT_CONFIG).
# Run by ../../build.sh (packages/README.md); result: build/lxml/site.
set -euo pipefail
source "$PYEXT/tools/lib.sh"
HERE="$PYEXT/packages/lxml"
SITE="$BP/site"
D="$BP/deps"
mkdir -p "$BP"

# ── lxml's source (its libxslt patch is in it) ──────────────────────────────────────────────────
# patches/lxml/*.patch: changes to lxml (patches/<library>: to one of the libraries)
patches() { [[ -d "$HERE/patches/$1" ]] && patchset "$HERE/patches/$1" "$BP/patches-$1.sum" || true; }
if stage lxml-src "$BP/src/.stamp" "$(lock lxml)" $(patches lxml); then
  rm -rf "$BP/src"
  S="$(unpack lxml "$BP/src")"
  apply_patches "$S" "$HERE/patches/lxml"
  touch "$BP/src/.stamp"
  redo lxml-deps
  redo lxml
fi
S="$(echo "$BP"/src/lxml-*/)"; S="${S%/}"

# ── the libraries ───────────────────────────────────────────────────────────────────────────────
if stage lxml-deps "$D/.done" $(lock zlib libiconv libxml2 libxslt) "$BP/src/.stamp" "$HERE/build.sh" \
     $(patches libxml2) $(patches libxslt); then
  rm -rf "$BP/deps-src" "$D"; mkdir -p "$BP/deps-src" "$D"
  LOG="$BP/deps.log"; : > "$LOG"

  zlib_pic "$(unpack zlib "$BP/deps-src")" "$D" "$LOG"

  L="$(unpack libiconv "$BP/deps-src")"
  log_run "$LOG" configure_cross "$L" "$D" --disable-nls
  # the library and its header, as its install-lib does (not the iconv program)
  log_run "$LOG" make -C "$L" lib/localcharset.h
  log_run "$LOG" make -C "$L/lib" -j"$JOBS" install
  install -m 644 "$L/include/iconv.h.inst" "$D/include/iconv.h"

  L="$(unpack libxml2 "$BP/deps-src")"
  apply_patches "$L" "$HERE/patches/libxml2"
  log_run "$LOG" configure_cross "$L" "$D" --without-python --with-iconv="$D" --with-zlib="$D" --without-lzma \
    --enable-rebuild-docs=no --without-debug --without-programs --without-modules
  log_run "$LOG" make -C "$L" -j"$JOBS" install
  # and libxml2 needs libiconv, which lxml does not link (a shared libxml2 brings it along)
  llvm-ar-19 qcL "$D/lib/libxml2.a" "$D/lib/libiconv.a" && llvm-ar-19 s "$D/lib/libxml2.a"

  L="$(unpack libxslt "$BP/deps-src")"
  (cd "$S" && log_run "$LOG" "$HOSTPY" -I -c 'import sys; sys.path.insert(0, "."); from patch_lxml_deplibs import apply_patch_file; apply_patch_file(sys.argv[1], sys.argv[2])' \
     libxslt-1.1.43-backport1.patch "$L")
  apply_patches "$L" "$HERE/patches/libxslt"
  log_run "$LOG" configure_cross "$L" "$D" --without-python --with-libxml-prefix="$D" --without-crypto
  log_run "$LOG" make -C "$L" -j"$JOBS" install

  touch "$D/.done"
  redo lxml
fi

# ── lxml ────────────────────────────────────────────────────────────────────────────────────────
if stage lxml "$SITE/.done" "$BP/src/.stamp" "$D/.done" "$HERE/build.sh" "$PYEXT/tools/wasm-pyext-cc" "$ROOT/userspace/bin/wasm-cc"; then
  rm -rf "$S/build" "$SITE"
  LOG="$BP/build.log"; : > "$LOG"
  (cd "$S" && log_run "$LOG" env CC="$PYEXT/tools/wasm-pyext-cc" LDSHARED="$PYEXT/tools/wasm-pyext-cc -shared" \
     CFLAGS="-O2" PKG_CONFIG=/bin/false XML2_CONFIG="$D/bin/xml2-config" XSLT_CONFIG="$D/bin/xslt-config" \
     "$B/host/bin/python3-guest" setup.py build_ext -j"$JOBS" build)
  grep -E "^Building (against|without)" "$LOG" | sort -u
  mkdir -p "$SITE"
  cp -r "$S"/build/lib.*/lxml "$SITE/lxml"
  # the libraries' licenses beside lxml's, as licenses/<library>-<file>
  licenses=()
  for f in 'libxml2-*/Copyright' 'libxslt-*/Copyright' 'libiconv-*/COPYING.LIB' 'zlib-*/LICENSE'; do
    path="$(echo "$BP"/deps-src/$f)"
    [[ -f "$path" ]] || { echo "no $f in $BP/deps-src" >&2; exit 1; }
    licenses+=(--license "${f%%-\**}-$(basename "$f")=$path")
  done
  "$HOSTPY" "$PYEXT/tools/dist-info.py" "$S" "$SITE" "${licenses[@]}"
  touch "$SITE/.done"
fi
