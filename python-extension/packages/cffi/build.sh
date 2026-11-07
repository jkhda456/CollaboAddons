#!/usr/bin/env bash
# cffi 2.1 for the guest: its C half, _cffi_backend, on libffi 3.8 with a backend for this
# platform (patches/libffi), static and position-independent in the module.
#
# libffi calls a function, and makes a callback, of any C type; WebAssembly checks the type of
# every indirect call, so each needs a function of that very type, which nothing can have ready
# for every type and WebAssembly cannot make at run time but by instantiating a module. libffi's
# own wasm32 port makes them in JavaScript (Emscripten); patches/libffi/0001 adds one for a Linux
# guest whose C library loads side modules (src/wasm/ffi_dlopen.c): each call type and each
# callback is a small side module it writes and loads with dlopen, which the engine instantiates
# in every thread of the process alike (addons/mod/0019, 0020). The C ABI is clang's for
# WebAssembly. patches/cffi/0001 has cffi take callbacks from libffi there, as on Emscripten
# (it would allocate executable memory itself).
#
# Also cffi for the host CPython (build/cffi/host-site: cffi, a host _cffi_backend on a host
# build of the same libffi, pycparser), for the packages whose build runs cffi (cryptography's
# _openssl.c).
#
# And cffi's earlier minor lines, built the same way, for pip in the guest (build/cffi/wheels:
# ../../build.sh packs them as wheels, with the installed version, into /usr/share/python-wheels,
# where /etc/pip.conf's find-links points). pip builds a package in an isolated environment with the
# cffi its pyproject.toml asks for — zstandard's `cffi~=1.17` on Python < 3.14 — and would build
# it from PyPI's sdist, which cannot work here: without patches/cffi, cffi allocates its callbacks
# in executable memory (mmap, which the guest's C library does not declare) and calls variadic
# functions as fixed ones (a trap here). One version per minor line, its last release, back to
# 1.17, the first that builds for CPython 3.13 (1.16 calls _PyErr_WriteUnraisableMsg, gone in 3.13).
# Run by ../../build.sh (packages/README.md); result: build/cffi/site, build/cffi/wheels.
set -euo pipefail
source "$PYEXT/tools/lib.sh"
HERE="$PYEXT/packages/cffi"
SITE="$BP/site"
D="$BP/deps"          # libffi for the guest
HD="$BP/host-deps"    # libffi for the host
mkdir -p "$BP"

# ── cffi's source (patches/cffi) ────────────────────────────────────────────────────────────────
if stage cffi-src "$BP/src/.stamp" "$(lock cffi)" "$(patchset "$HERE/patches/cffi" "$BP/patches-cffi.sum")"; then
  rm -rf "$BP/src"
  S="$(unpack cffi "$BP/src")"
  apply_patches "$S" "$HERE/patches/cffi"
  touch "$BP/src/.stamp"
  redo cffi-host
  redo cffi
fi
S="$(echo "$BP"/src/cffi-*/)"; S="${S%/}"

# ── libffi, for the guest and for the host ──────────────────────────────────────────────────────
if stage cffi-libffi "$D/.done" "$(lock libffi)" "$(patchset "$HERE/patches/libffi" "$BP/patches-libffi.sum")" "$HERE/build.sh"; then
  rm -rf "$BP/deps-src" "$D" "$HD"; mkdir -p "$BP/deps-src/host" "$D" "$HD"
  LOG="$BP/deps.log"; : > "$LOG"
  L="$(unpack libffi "$BP/deps-src")"
  apply_patches "$L" "$HERE/patches/libffi"
  log_run "$LOG" configure_cross "$L" "$D" --disable-docs --disable-multi-os-directory
  log_run "$LOG" make -C "$L" -j"$JOBS" install
  L="$(unpack libffi "$BP/deps-src/host")"
  (cd "$L" && log_run "$LOG" ./configure --prefix="$HD" --disable-shared --enable-static --with-pic \
     --disable-docs --disable-multi-os-directory CC=clang-19 CFLAGS=-O2)
  log_run "$LOG" make -C "$L" -j"$JOBS" install
  touch "$D/.done"
  redo cffi-host
  redo cffi
fi

# pkg-config for setup.py, answering for libffi alone: from PREFIX (and nothing of this machine's)
pkgconfig() {
  cat > "$BP/pkg-config-$2" <<EOF
#!/bin/sh
case "\$1" in
  --cflags-only-I) echo "-I$1/include" ;;
  --libs-only-L) echo "-L$1/lib" ;;
  --libs-only-l) echo "-lffi" ;;
  *) echo ;;
esac
EOF
  chmod +x "$BP/pkg-config-$2"
  echo "$BP/pkg-config-$2"
}

# ── cffi for the host CPython: what packages' builds run ────────────────────────────────────────
HS="$BP/host-site"
if stage cffi-host "$HS/.done" "$BP/src/.stamp" "$D/.done" "$(lock pycparser)"; then
  rm -rf "$S/build" "$HS"; mkdir -p "$HS"
  LOG="$BP/host.log"; : > "$LOG"
  (cd "$S" && log_run "$LOG" env CC=clang-19 LDSHARED="clang-19 -shared" PKG_CONFIG="$(pkgconfig "$HD" host)" \
     PYTHONPATH="$B/host/site" "$HOSTPY" setup.py build_ext build)
  cp -r "$S"/build/lib.*/. "$HS/"
  "$HOSTPY" -I -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' "$(fetch pycparser)" "$HS"
  PYTHONPATH="$HS" "$HOSTPY" -c 'import cffi, _cffi_backend; print("  host cffi", cffi.__version__, _cffi_backend.__file__.rsplit("/", 1)[-1])'
  rm -rf "$S/build"
  touch "$HS/.done"
fi

# ── cffi ────────────────────────────────────────────────────────────────────────────────────────
if stage cffi "$SITE/.done" "$BP/src/.stamp" "$D/.done" "$HERE/build.sh" "$PYEXT/tools/wasm-pyext-cc" "$ROOT/userspace/bin/wasm-cc"; then
  rm -rf "$S/build" "$SITE"
  LOG="$BP/build.log"; : > "$LOG"
  (cd "$S" && log_run "$LOG" env CC="$PYEXT/tools/wasm-pyext-cc" LDSHARED="$PYEXT/tools/wasm-pyext-cc -shared" \
     CFLAGS="-O2" PKG_CONFIG="$(pkgconfig "$D" guest)" "$B/host/bin/python3-guest" setup.py build_ext build)
  mkdir -p "$SITE"
  cp -r "$S"/build/lib.*/. "$SITE/"
  "$HOSTPY" "$PYEXT/tools/dist-info.py" "$S" "$SITE" --license "libffi-LICENSE=$(echo "$BP"/deps-src/libffi-*/LICENSE)"
  touch "$SITE/.done"
fi

# ── cffi's earlier minor lines, for wheels: build/cffi/wheels/cffi-<version> ────────────────────
# Each as the installed one, from its sdist with patches/cffi (which applies to 1.17 and 2.0 as it
# is), on the same libffi. Its source stays (src-<version>): the fpcast stage compiles it again.
OTHER=(cffi-1.17.1 cffi-2.0.0)
for name in "${OTHER[@]}"; do
  W="$BP/wheels/$name"
  if stage "$name" "$W/.done" "$(lock "$name")" "$BP/patches-cffi.sum" "$D/.done" "$HERE/build.sh" \
       "$PYEXT/tools/dist-info.py" "$PYEXT/tools/wasm-pyext-cc" "$ROOT/userspace/bin/wasm-cc"; then
    rm -rf "$BP/src-$name" "$W"
    VS="$(unpack "$name" "$BP/src-$name")"
    apply_patches "$VS" "$HERE/patches/cffi"
    LOG="$BP/build-$name.log"; : > "$LOG"
    (cd "$VS" && log_run "$LOG" env CC="$PYEXT/tools/wasm-pyext-cc" LDSHARED="$PYEXT/tools/wasm-pyext-cc -shared" \
       CFLAGS="-O2" PKG_CONFIG="$(pkgconfig "$D" guest)" "$B/host/bin/python3-guest" setup.py build_ext build)
    mkdir -p "$W"
    cp -r "$VS"/build/lib.*/. "$W/"
    "$HOSTPY" "$PYEXT/tools/dist-info.py" "$VS" "$W" --license "libffi-LICENSE=$(echo "$BP"/deps-src/libffi-*/LICENSE)"
    rm -rf "$VS/build"
    touch "$W/.done"
  fi
done
# (a version taken out of OTHER leaves no wheel behind, nor compile records of a source now gone)
for W in "$BP"/wheels/*/; do
  name="$(basename "$W")"
  [[ -d "$W" && " ${OTHER[*]} " != *" $name "* ]] || continue
  echo "  $name: no longer built, removed"
  rm -rf "$W" "$BP/src-$name"
  { grep -lF "\"$BP/src-$name/" "$BP"/compdb/*.json 2>/dev/null || true; } | xargs -r rm -f
done
