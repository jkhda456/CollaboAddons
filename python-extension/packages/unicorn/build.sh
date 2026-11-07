#!/usr/bin/env bash
# Unicorn 2.1 for the guest: the CPU emulator (Unicorn's QEMU 5.0.1 TCG, with its API: x86, ARM,
# ARM64, M68K, MIPS, SPARC, PPC, RISC-V, S390X, TriCore) and its Python binding, which loads the
# library with ctypes (the interpreter's _ctypes, which the add-on adds: ../../build.sh, stage ctypes).
#
# Unicorn translates guest code into code for the host CPU; WebAssembly can have no such code (no
# memory is executable), so it runs TCG's interpreter instead: patches/0001, QEMU 5.0.1's TCI, which
# Unicorn 2 left out, each helper called as the type it has (tci-call-gen.py writes the table);
# 0002 the code buffer and RAM from the heap; 0003 and 0004 glib_compat's comparisons and the
# binding's variadic calls made as the types they are (WebAssembly checks every call through a
# pointer). The library is a side module, libunicorn.so.2, in unicorn/lib, and the headers in
# unicorn/include, as the PyPI wheel has them; not the wheel's libunicorn.a (33 MB: a program
# here links build/unicorn/build's archives, as the tests do).
#
# And Unicorn's earlier minor lines, for pip in the guest (build/unicorn/wheels: ../../build.sh packs
# them as wheels, with the installed version, into /usr/share/python-wheels): packages pin Unicorn
# to the line their code was written for (pwntools `unicorn>=2.0.1,!=2.1.3,!=2.1.4`, speakeasy
# `==1.0.2`), and PyPI's sdists cannot build here (CMake knows no wasm32 host; nor could they run:
# no interpreter), so /etc/pip.conf has pip take Unicorn from wheels only. One version per minor
# line, its last release: 2.0.1.post1 (patches-2.0: patches/ redone for its tree, and 0005, its
# binding finding the library without distutils and pkg_resources, which Python 3.12 has not) and
# 1.0.3 (patches-1.0: Unicorn 1 is QEMU 2.2.1's TCG, and its TCI is QEMU 2.2.1's, ported alike).
# Run by ../../build.sh (packages/README.md); result: build/unicorn/site, build/unicorn/wheels.
set -euo pipefail
source "$PYEXT/tools/lib.sh"
HERE="$PYEXT/packages/unicorn"
SITE="$BP/site"
mkdir -p "$BP"

# ── the source (patches/) ───────────────────────────────────────────────────────────────────────
if stage unicorn-src "$BP/src/.stamp" "$(lock unicorn)" "$(patchset "$HERE/patches" "$BP/patches.sum")" "$HERE/tci-call-gen.py"; then
  rm -rf "$BP/src"
  S="$(unpack unicorn "$BP/src")"
  apply_patches "$S" "$HERE/patches"
  # the interpreter's call table in patches/0001 is what the generator writes
  "$HOSTPY" "$HERE/tci-call-gen.py" | cmp -s - "$S/src/qemu/tcg/tci-call.inc.c" \
    || { echo "patches/0001's tci-call.inc.c is not what tci-call-gen.py writes" >&2; exit 1; }
  touch "$BP/src/.stamp"
  redo unicorn
fi
S="$(echo "$BP"/src/unicorn-*/)"; S="${S%/}"

# ── libunicorn.so.2 (and its archives, for the tests' programs), and the package ────────────────
if stage unicorn "$SITE/.done" "$BP/src/.stamp" "$HERE/build.sh" "$PYEXT/tools/wasm-pyext-cc" "$ROOT/userspace/bin/wasm-cc"; then
  rm -rf "$BP/build" "$SITE"
  LOG="$BP/build.log"; : > "$LOG"
  # (configure's compiles are not recorded for the fpcast stage: qemu/configure deletes its tests)
  ( unset PYEXT_COMPDB
    log_run "$LOG" cmake_cross "$S/src" "$BP/build" "$BP/prefix" -DBUILD_SHARED_LIBS=ON -DUNICORN_BUILD_TESTS=OFF \
      -DCMAKE_SKIP_RPATH=ON )
  log_run "$LOG" "$NINJA" -C "$BP/build" -j"$JOBS" unicorn unicorn_static
  mkdir -p "$SITE/unicorn/lib" "$SITE/unicorn/include"
  cp -r "$S/unicorn/." "$SITE/unicorn/"
  cp "$BP/build/libunicorn.so.2" "$SITE/unicorn/lib/"
  cp -r "$S/src/include/unicorn" "$SITE/unicorn/include/"
  # Unicorn is GPL-2.0 (its README; the sdist has no COPYING of its own): QEMU's texts, which
  # cover Unicorn's QEMU and glib_compat (LGPL-2.1), and TCI (GPL-2.0-or-later)
  "$HOSTPY" "$PYEXT/tools/dist-info.py" "$S" "$SITE" --license "COPYING=$S/src/qemu/COPYING" \
    --license "COPYING.LIB=$S/src/qemu/COPYING.LIB" --license "qemu-LICENSE=$S/src/qemu/LICENSE"
  touch "$SITE/.done"
fi

# ── the earlier minor lines, for wheels: build/unicorn/wheels/unicorn-<version> ────────────────
# Each from its sdist with the patches of its line (patches-<major>.<minor>), in build/unicorn/<name>:
# its source and its build, kept but for the objects (the fpcast stage compiles the sources again,
# with the headers the build generated). Unicorn 2.0 builds with CMake, as 2.1; Unicorn 1 with its
# Makefile and QEMU's configure, as its setup.py has make.sh do (the library only, no samples; its
# configure run apart, without compile records: it deletes its tests), into unicorn/lib/libunicorn.so,
# the name its binding loads and its wheel has.
OTHER=(unicorn-2.0.1.post1 unicorn-1.0.3)
for name in "${OTHER[@]}"; do
  line="$(echo "${name#unicorn-}" | cut -d. -f1,2)"
  PD="$HERE/patches-$line" VB="$BP/$name" W="$BP/wheels/$name"
  if stage "$name-src" "$VB/src/.stamp" "$(lock "$name")" "$(patchset "$PD" "$BP/patches-$line.sum")" "$HERE/tci-call-gen.py"; then
    rm -rf "$VB"
    VS="$(unpack "$name" "$VB/src")"
    apply_patches "$VS" "$PD"
    "$HOSTPY" "$HERE/tci-call-gen.py" | cmp -s - "$VS/src/qemu/tcg/tci-call.inc.c" \
      || { echo "patches-$line/0001's tci-call.inc.c is not what tci-call-gen.py writes" >&2; exit 1; }
    touch "$VB/src/.stamp"
    redo "$name"
  fi
  VS="$(echo "$VB"/src/unicorn-*/)"; VS="${VS%/}"
  if stage "$name" "$W/.done" "$VB/src/.stamp" "$HERE/build.sh" "$PYEXT/tools/dist-info.py" "$PYEXT/tools/wasm-pyext-cc" \
       "$ROOT/userspace/bin/wasm-cc"; then
    rm -rf "$W"
    mkdir -p "$W/unicorn/lib" "$W/unicorn/include"
    LOG="$VB/build.log"; : > "$LOG"
    if [[ "$line" == 1.* ]]; then
      make=(make -C "$VS/src" CC="$PYEXT/tools/wasm-pyext-cc" AR=llvm-ar-19 RANLIB=llvm-ranlib-19 UNICORN_DEBUG=no
            UNICORN_STATIC=no SMP_MFLAGS=-j"$JOBS" V=1)
      ( unset PYEXT_COMPDB; log_run "$LOG" "${make[@]}" qemu/config-host.mak )
      log_run "$LOG" "${make[@]}" unicorn
      cp "$VS/src/libunicorn.so.1" "$W/unicorn/lib/libunicorn.so"
      find "$VS/src" -name '*.o' -delete
    else
      rm -rf "$VB/build"
      # (2.0's CMakeLists.txt asks for CMake 3.1's policies, which CMake 4 has no more)
      ( unset PYEXT_COMPDB
        log_run "$LOG" cmake_cross "$VS/src" "$VB/build" "$VB/prefix" -DBUILD_SHARED_LIBS=ON -DUNICORN_BUILD_TESTS=OFF \
          -DCMAKE_SKIP_RPATH=ON -DCMAKE_POLICY_VERSION_MINIMUM=3.5 )
      log_run "$LOG" "$NINJA" -C "$VB/build" -j"$JOBS" unicorn
      cp "$VB/build/libunicorn.so.2" "$W/unicorn/lib/"
      find "$VB/build" -name '*.o' -delete
    fi
    cp -r "$VS/unicorn/." "$W/unicorn/"
    cp -r "$VS/src/include/unicorn" "$W/unicorn/include/"
    "$HOSTPY" "$PYEXT/tools/dist-info.py" "$VS" "$W" --license "COPYING=$VS/src/qemu/COPYING" \
      --license "COPYING.LIB=$VS/src/qemu/COPYING.LIB" --license "qemu-LICENSE=$VS/src/qemu/LICENSE"
    touch "$W/.done"
  fi
done
# (a version taken out of OTHER leaves no wheel behind, nor compile records of a source now gone)
for W in "$BP"/wheels/*/; do
  name="$(basename "$W")"
  [[ -d "$W" && " ${OTHER[*]} " != *" $name "* ]] || continue
  echo "  $name: no longer built, removed"
  rm -rf "$W" "$BP/$name"
  { grep -lF "\"$BP/$name/" "$BP"/compdb/*.json 2>/dev/null || true; } | xargs -r rm -f
done
