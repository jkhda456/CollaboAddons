#!/usr/bin/env bash
# cryptography 50.0 for the guest: its Rust half (cryptography.hazmat.bindings._rust, PyO3) with
# the OpenSSL its own wheels carry, 4.0.3, static and position-independent in the module.
#
#   OpenSSL 4.0.3   as the python step builds its 3.5 (linux-generic32: no assembly; no async, no
#                   secure memory, no fork), but no-module: the legacy provider built in, as in
#                   the wheels; its configuration and CA bundle in /etc/ssl, the guest's
#   _rust           cryptography's crates for wasm32-unknown-linux-musl (the guest's Rust
#                   toolchain, python/build-rust-toolchain.sh) with a std built PIC (tools/lib.sh
#                   rust_std_pic), as a static library, linked as an extension module by
#                   tools/wasm-pyext-cc like any other; with it the _openssl module cffi makes of
#                   src/_cffi_src (on the host: build/cffi/host-site, so cffi before this)
#
# PyO3 is told what the guest's interpreter is (CPython 3.13, 32-bit, the stable ABI the crate
# asks for; no libpython to link: the module imports the interpreter's symbols).
# Run by ../../build.sh (packages/README.md); result: build/cryptography/site.
set -euo pipefail
source "$PYEXT/tools/lib.sh"
HERE="$PYEXT/packages/cryptography"
SITE="$BP/site"
D="$BP/deps"
mkdir -p "$BP"
[[ -f "$B/cffi/host-site/.done" ]] || { echo "cryptography needs cffi for the host: build packages/cffi first" >&2; exit 1; }

# ── cryptography's source (patches/cryptography) ────────────────────────────────────────────────
patches() { [[ -d "$HERE/patches/$1" ]] && patchset "$HERE/patches/$1" "$BP/patches-$1.sum" || true; }
if stage cryptography-src "$BP/src/.stamp" "$(lock cryptography)" $(patches cryptography); then
  rm -rf "$BP/src"
  S="$(unpack cryptography "$BP/src")"
  apply_patches "$S" "$HERE/patches/cryptography"
  touch "$BP/src/.stamp"
  redo cryptography
fi
S="$(echo "$BP"/src/cryptography-*/)"; S="${S%/}"

# ── OpenSSL ─────────────────────────────────────────────────────────────────────────────────────
if stage cryptography-openssl "$D/.done" "$(lock openssl)" "$HERE/build.sh" $(patches openssl); then
  rm -rf "$BP/deps-src" "$D"; mkdir -p "$BP/deps-src" "$D"
  LOG="$BP/deps.log"; : > "$LOG"
  L="$(unpack openssl "$BP/deps-src")"
  apply_patches "$L" "$HERE/patches/openssl"
  (cd "$L" && log_run "$LOG" perl ./Configure linux-generic32 CC="$PYEXT/tools/wasm-pyext-cc" AR=llvm-ar-19 \
     RANLIB=llvm-ranlib-19 -O2 -DHAVE_FORK=0 --prefix="$D" --libdir=lib --openssldir=/etc/ssl \
     no-shared no-module no-dso no-async no-secure-memory no-tests no-apps no-docs)
  log_run "$LOG" make -C "$L" -j"$JOBS" build_libs
  log_run "$LOG" make -C "$L" install_dev
  touch "$D/.done"
  redo cryptography
fi

# ── _rust, and the package ──────────────────────────────────────────────────────────────────────
rust_std_pic
STD_STAMP="$B/rust/sysroot/lib/rustlib/$RUST_TARGET/lib/libstd.stamp"
if stage cryptography "$SITE/.done" "$BP/src/.stamp" "$D/.done" "$STD_STAMP" "$HERE/build.sh" "$B/cffi/host-site/.done" \
     "$PYEXT/tools/crates.py" "$PYEXT/tools/wasm-pyext-cc" "$ROOT/userspace/bin/wasm-cc"; then
  rm -rf "$SITE"
  LOG="$BP/build.log"; : > "$LOG"
  # The C in the crates (cffi's _openssl.c; openssl-sys's look at OpenSSL's headers) compiled as
  # the add-on compiles C; the cc crate passes --target=wasm32-unknown-linux-musl, which clang-19
  # does not know (wasm-cc sets its own)
  cat > "$BP/cc" <<SH
#!/bin/sh
for a; do shift; case "\$a" in --target=*) ;; *) set -- "\$@" "\$a" ;; esac; done
exec "$PYEXT/tools/wasm-pyext-cc" "\$@"
SH
  chmod +x "$BP/cc"
  cat > "$BP/pyo3-config.txt" <<'EOF'
implementation=CPython
version=3.13
shared=false
abi3=true
lib_name=python3.13
pointer_width=32
build_flags=
suppress_build_script_link_lines=true
EOF
  T="${RUST_TARGET//-/_}"
  (
    cd "$S"
    export RUSTC="$RUSTC_PIC" CARGO_TARGET_DIR="$BP/target" PYO3_CONFIG_FILE="$BP/pyo3-config.txt"
    export "CC_$T=$BP/cc" "AR_$T=llvm-ar-19" "RANLIB_$T=llvm-ranlib-19" WASM_MUSL_SYSROOT="$ROOT/userspace/sysroot"
    export OPENSSL_DIR="$D" OPENSSL_STATIC=1 OPENSSL_NO_VENDOR=1
    # cryptography-cffi's build script runs cffi on src/_cffi_src with this Python
    export PYO3_PYTHON="$B/host/bin/python3-guest" PYTHONPATH="$B/cffi/host-site"
    export RUSTFLAGS="-Crelocation-model=pic --remap-path-prefix=$S=/usr/src/cryptography --remap-path-prefix=$CARGO_HOME=/usr/src/cargo"
    log_run "$LOG" cargo +"$RUST_NIGHTLY" rustc --release --lib --target "$RUST_TARGET" -j"$JOBS" \
      --manifest-path src/rust/Cargo.toml --config "patch.crates-io.libc.path=\"$ROOT/.tools/rust-wasm/libc\"" \
      --crate-type staticlib
  )
  A="$BP/target/$RUST_TARGET/release/libcryptography_rust.a"
  mkdir -p "$SITE"
  cp -r "$S/src/cryptography" "$SITE/cryptography"
  # Only PyInit__rust exported: Rust's symbols are not hidden, and exported they would keep all of
  # std (Command::spawn's fork among it). std names _Unwind_ functions it never calls (panic=abort):
  # the toolchain's stubs.
  log_run "$LOG" "$PYEXT/tools/wasm-pyext-cc" -shared -o "$SITE/cryptography/hazmat/bindings/_rust.abi3.so" \
    -Wl,--no-export-dynamic -Wl,--export=PyInit__rust "$A" "$(dirname "$STD_STAMP")/libunwind.a"
  # the licenses: OpenSSL's beside cryptography's, and the crates linked in, each with its license
  # (tools/crates.py). Rust's standard library's: ../../build.sh, with libc++'s
  (cd "$S" && RUSTC="$RUSTC_PIC" cargo +"$RUST_NIGHTLY" metadata --format-version 1 --offline \
     --manifest-path src/rust/Cargo.toml --filter-platform "$RUST_TARGET" \
     --config "patch.crates-io.libc.path=\"$ROOT/.tools/rust-wasm/libc\"" 2>>"$LOG") \
    | "$HOSTPY" "$PYEXT/tools/crates.py" cryptography-rust cryptography/hazmat/bindings/_rust.abi3.so > "$BP/crates.txt"
  grep -q "^  pyo3 " "$BP/crates.txt" || { echo "no crate list (cargo metadata: $LOG)" >&2; exit 1; }
  "$HOSTPY" "$PYEXT/tools/dist-info.py" "$S" "$SITE" --license "openssl-LICENSE.txt=$(echo "$BP"/deps-src/openssl-*/LICENSE.txt)" \
    --license "crates.txt=$BP/crates.txt"
  touch "$SITE/.done"
fi
