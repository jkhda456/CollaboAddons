#!/usr/bin/env bash
# numpy 2.5 for the guest: its own meson build (the meson numpy vendors), cross-compiling to wasm32
# with the add-on's compilers. Run by ../../build.sh (packages/README.md); result: build/numpy/site.
#
#   no BLAS/LAPACK library: numpy's own fallbacks (lapack_lite, its C matrix products), as numpy
#     builds where none is found (-Dallow-noblas). numpy.linalg works, at C-loop speed
#   no SIMD (-Dcpu-baseline=none -Dcpu-dispatch=none): numpy's dispatch knows no wasm32 features
#   long double is IEEE binary128 on wasm32 (software): the cross file says so, since numpy's
#     meson would otherwise run a program to find out
#   installed with the runtime, python-runtime and tests tags, but without the tests/ folders:
#     the test extension modules stay (_multiarray_tests, _umath_tests, ... — numpy's wheels have
#     them too; its own suite needs them, tests/upstream.py), the suite's files do not; no C
#     headers and libnpymath.a (nothing in the guest could compile against them: no dlopen)
set -euo pipefail
source "$PYEXT/tools/lib.sh"
HERE="$PYEXT/packages/numpy"
SITE="$BP/site"
mkdir -p "$BP"

if stage numpy-src "$BP/src/.stamp" "$(lock numpy)" "$(patchset "$HERE/patches" "$BP/patches.sum")"; then
  rm -rf "$BP/src"
  S="$(unpack numpy "$BP/src")"
  apply_patches "$S" "$HERE/patches"
  touch "$BP/src/.stamp"
  redo numpy
fi
S="$(echo "$BP"/src/numpy-*/)"; S="${S%/}"

if stage numpy "$SITE/.done" "$BP/src/.stamp" "$HERE/build.sh" "$PYEXT/tools/wasm-pyext-cc" "$ROOT/userspace/bin/wasm-cc" "$B/libcxx-install/lib/libc++.a"; then
  rm -rf "$BP/build" "$BP/destdir" "$SITE"
  cat > "$BP/cross.ini" <<EOF
[binaries]
c = '$PYEXT/tools/wasm-pyext-cc'
cpp = '$PYEXT/tools/wasm-pyext-c++'
ar = 'llvm-ar-19'
ranlib = 'llvm-ranlib-19'
strip = 'llvm-strip-19'
python = '$B/host/bin/python3-guest'
cython = '$B/host/bin/cython'
pkg-config = '/bin/false'

[host_machine]
system = 'linux'
cpu_family = 'wasm32'
cpu = 'wasm32'
endian = 'little'

[properties]
longdouble_format = 'IEEE_QUAD_LE'
EOF
  LOG="$BP/build.log"; : > "$LOG"
  MESON=("$HOSTPY" "$S/vendored-meson/meson/meson.py")
  log_run "$LOG" "${MESON[@]}" setup "$BP/build" "$S" --cross-file "$BP/cross.ini" \
    --prefix=/usr --buildtype=release -Db_ndebug=if-release \
    -Dpython.purelibdir=lib/python3.13/site-packages -Dpython.platlibdir=lib/python3.13/site-packages \
    -Dblas=none -Dlapack=none -Dallow-noblas=true -Dcpu-baseline=none -Dcpu-dispatch=none \
    -Ddisable-svml=true -Ddisable-intel-sort=true
  log_run "$LOG" "$NINJA" -C "$BP/build" -j"$JOBS"
  log_run "$LOG" "${MESON[@]}" install -C "$BP/build" --destdir "$BP/destdir" --tags runtime,python-runtime,tests
  mv "$BP/destdir/usr/lib/python3.13/site-packages" "$SITE"
  find "$SITE" -type d -name tests -prune -exec rm -rf {} +
  # numpy.show_config() lists the compilers by path: this machine's, not the guest's
  sed -i -e "s#$PYEXT/tools/#/usr/libexec/collabo/#g" -e "s#$ROOT#/usr/src/collabo-core#g" "$SITE/numpy/__config__.py"
  "$HOSTPY" "$PYEXT/tools/dist-info.py" "$S" "$SITE"
  touch "$SITE/.done"
fi
