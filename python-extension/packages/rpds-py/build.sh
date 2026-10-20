#!/usr/bin/env bash
# rpds-py 2026.9 for the guest: Python bindings (PyO3) to the rpds crate's persistent data
# structures, rpds.rpds — what jsonschema and referencing (and so mcp) need. One crate with no C
# in it, built as cryptography's Rust is: for wasm32-unknown-linux-musl with the guest's Rust
# toolchain and a std built PIC (tools/lib.sh rust_std_pic), as a static library linked as an
# extension module by tools/wasm-pyext-cc. Its Python half is what maturin writes into the wheel:
# rpds/__init__.py re-exporting the module, its stub (rpds.pyi as __init__.pyi) and py.typed.
#
# PyO3 is told what the guest's interpreter is (CPython 3.13, 32-bit; not the stable ABI, as the
# crate does not ask for it; no libpython to link: the module imports the interpreter's symbols).
# Run by ../../build.sh (packages/README.md); result: build/rpds-py/site.
set -euo pipefail
source "$PYEXT/tools/lib.sh"
HERE="$PYEXT/packages/rpds-py"
SITE="$BP/site"
mkdir -p "$BP"

# ── the source (patches/, if any) ───────────────────────────────────────────────────────────────
if stage rpds-py-src "$BP/src/.stamp" "$(lock rpds-py)" $( [[ -d "$HERE/patches" ]] && patchset "$HERE/patches" "$BP/patches.sum"); then
  rm -rf "$BP/src"
  S="$(unpack rpds-py "$BP/src")"
  apply_patches "$S" "$HERE/patches"
  touch "$BP/src/.stamp"
  redo rpds-py
fi
S="$(echo "$BP"/src/rpds_py-*/)"; S="${S%/}"

# ── rpds.rpds, and the package ──────────────────────────────────────────────────────────────────
rust_std_pic
STD_STAMP="$B/rust/sysroot/lib/rustlib/$RUST_TARGET/lib/libstd.stamp"
if stage rpds-py "$SITE/.done" "$BP/src/.stamp" "$STD_STAMP" "$HERE/build.sh" "$PYEXT/tools/crates.py" \
     "$PYEXT/tools/wasm-pyext-cc" "$ROOT/userspace/bin/wasm-cc"; then
  rm -rf "$SITE"
  LOG="$BP/build.log"; : > "$LOG"
  cat > "$BP/pyo3-config.txt" <<'EOF'
implementation=CPython
version=3.13
shared=false
abi3=false
lib_name=python3.13
pointer_width=32
build_flags=
suppress_build_script_link_lines=true
EOF
  # tombl's libc fork, as cryptography's build names it. (Cargo.lock pins crates.io's libc 0.2.177,
  # an older version than the fork's, so cargo leaves the patch unused, as with cryptography's and
  # the python step's crates; what pyo3-ffi takes from libc — size_t, ssize_t, intptr_t, wchar_t,
  # FILE, getenv — is the same in both: they differ in the layout of structs such as stat.)
  LIBC="patch.crates-io.libc.path=\"$ROOT/.tools/rust-wasm/libc\""
  (
    cd "$S"
    export RUSTC="$RUSTC_PIC" CARGO_TARGET_DIR="$BP/target" PYO3_CONFIG_FILE="$BP/pyo3-config.txt"
    export RUSTFLAGS="-Crelocation-model=pic --remap-path-prefix=$S=/usr/src/rpds-py --remap-path-prefix=$CARGO_HOME=/usr/src/cargo"
    log_run "$LOG" cargo +"$RUST_NIGHTLY" rustc --release --lib --target "$RUST_TARGET" -j"$JOBS" \
      --config "$LIBC" --crate-type staticlib
  )
  mkdir -p "$SITE/rpds"
  # Only PyInit_rpds exported (Rust's symbols are not hidden: exported, they would keep all of std);
  # std's _Unwind_ names are the toolchain's stubs (panic=abort)
  log_run "$LOG" "$PYEXT/tools/wasm-pyext-cc" -shared -o "$SITE/rpds/rpds.cpython-313.so" \
    -Wl,--no-export-dynamic -Wl,--export=PyInit_rpds "$BP/target/$RUST_TARGET/release/librpds.a" \
    "$(dirname "$STD_STAMP")/libunwind.a"
  # maturin's Python half of the wheel, as its wheel has it
  printf 'from .rpds import *\n\n__doc__ = rpds.__doc__\nif hasattr(rpds, "__all__"):\n    __all__ = rpds.__all__\n' \
    > "$SITE/rpds/__init__.py"
  cp "$S/rpds.pyi" "$SITE/rpds/__init__.pyi"
  : > "$SITE/rpds/py.typed"
  # the crates linked in, with their licenses (Rust's standard library's: ../../build.sh)
  (cd "$S" && RUSTC="$RUSTC_PIC" cargo +"$RUST_NIGHTLY" metadata --format-version 1 --offline \
     --filter-platform "$RUST_TARGET" --config "$LIBC" 2>>"$LOG") \
    | "$HOSTPY" "$PYEXT/tools/crates.py" rpds-py rpds/rpds.cpython-313.so > "$BP/crates.txt"
  grep -q "^  pyo3 " "$BP/crates.txt" || { echo "no crate list (cargo metadata: $LOG)" >&2; exit 1; }
  "$HOSTPY" "$PYEXT/tools/dist-info.py" "$S" "$SITE" --license "crates.txt=$BP/crates.txt"
  touch "$SITE/.done"
fi
