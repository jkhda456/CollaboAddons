#!/usr/bin/env bash
# LIEF 1.0 for the guest: the library to parse, modify and abstract ELF, PE, Mach-O, COFF and the
# Android formats (DEX, OAT, VDEX, ART), and its Python bindings (nanobind, the copy LIEF carries)
# as the built-in module lief._lief. Its own CMake build with the add-on's compilers; the libraries
# LIEF vendors (spdlog, mbedtls, utfcpp, frozen, ...) come from its third-party/ folder.
#
#   the bindings need C++ exceptions (nanobind turns Python errors into them and back): the add-on's
#     libc++ has them; LIEF's core is built without, as LIEF builds it by default
#   from a tag archive, with no git: the version (1.0.0) and the tag's commit are told to CMake,
#     so that lief.__version__ and lief.__commit__ say what the wheels on PyPI say
#   off: the C API, examples, tests; the extended (closed-source) features cannot be built anyway
#   CMake packages are looked for outside the cross-build roots too: nanobind's config is in the
#     build tree (FetchContent of LIEF's copy), which those roots would hide
#
# Run by ../../build.sh (packages/README.md); result: build/lief/site.
set -euo pipefail
source "$PYEXT/tools/lib.sh"
HERE="$PYEXT/packages/lief"
SITE="$BP/site"
VERSION=1.0.0
COMMIT=d05b3499b   # tag 1.0.0's commit, as LIEF's own wheels name it (lief.__version__ 1.0.0-d05b3499b)
mkdir -p "$BP"

if stage lief-src "$BP/src/.stamp" "$(lock lief)" "$(patchset "$HERE/patches" "$BP/patches.sum")"; then
  rm -rf "$BP/src"
  S="$(unpack lief "$BP/src")"
  apply_patches "$S" "$HERE/patches"
  touch "$BP/src/.stamp"
  redo lief
fi
S="$(echo "$BP"/src/LIEF-*/)"; S="${S%/}"

if stage lief "$SITE/.done" "$BP/src/.stamp" "$HERE/build.sh" "$PYEXT/tools/wasm-pyext-cc" "$ROOT/userspace/bin/wasm-cc" "$B/libcxx-install/lib/libc++.a"; then
  rm -rf "$BP/build" "$SITE"
  LOG="$BP/build.log"; : > "$LOG"
  export LIEF_VERSION_ENV="$VERSION" LIEF_COMMIT="$COMMIT"
  log_run "$LOG" cmake_cross "$S" "$BP/build" "$BP/install" \
    -DLIEF_PYTHON_API=ON -DLIEF_C_API=OFF -DLIEF_EXAMPLES=OFF -DLIEF_TESTS=OFF -DLIEF_USE_CCACHE=OFF \
    -DLIEF_INSTALL=OFF -DCMAKE_FIND_ROOT_PATH_MODE_PACKAGE=BOTH \
    -DPython_EXECUTABLE="$B/host/bin/python3-guest" -DPython_INCLUDE_DIR="$ROOT/python/stage/usr/include/python3.13"
  # the bindings' translation units are large (nanobind templates): fewer at a time
  log_run "$LOG" "$NINJA" -C "$BP/build" -j"$(( JOBS > 4 ? 4 : JOBS ))" pyLIEF
  mkdir -p "$SITE"
  cp -r "$S/api/python/lief" "$SITE/lief"
  find "$SITE/lief" -name '__pycache__' -prune -exec rm -rf {} +
  cp "$BP/build/api/python/_lief.so" "$SITE/lief/_lief.cpython-313.so"
  # LIEF's license, and those of the libraries it carries (as their archives unpacked them)
  licenses=()
  for dep in spdlog mbedtls utfcpp frozen expected tcb-span nanobind json; do
    f="$(find "$BP/build" -path "*${dep}*" -maxdepth 4 \( -iname 'LICENSE' -o -iname 'LICENSE.txt' -o -iname 'LICENSE.md' -o -iname 'COPYING' \) 2>/dev/null | head -1)"
    [[ -n "$f" ]] && licenses+=(--license "$dep-$(basename "$f")=$f")
  done
  "$HOSTPY" "$PYEXT/tools/dist-info.py" "$S/api/python" "$SITE" --version "$VERSION" --license "$S/LICENSE" "${licenses[@]}"
  touch "$SITE/.done"
fi
