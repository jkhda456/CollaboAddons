#!/usr/bin/env bash
# The c-sh add-on: `c-sh` (collabo sh), terminal sessions that stay connected (Rust, rust/),
# packed as out/c-sh.cpio (an overlay: /usr/bin/c-sh).
#
# Everything is our own Rust code except the cryptography, which is the guest's OpenSSL
# (python/deps/lib/libcrypto.a, the library python, curl and git link), called directly.
#
# Needs the userspace step (musl, compiler-rt: userspace/sysroot), the python step (python/deps:
# OpenSSL for the guest) and the guest's Rust toolchain (python/build-rust-toolchain.sh, which
# this runs; it skips when it is up to date).
#
#   addons/c-sh/build.sh
#   TEST=1 addons/c-sh/build.sh     also run the unit tests natively first (needs this machine's
#                                   libcrypto.so.3; the SSH tests use OpenSSH's sshd and ssh-keygen)
set -euo pipefail

A="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$A/../.." && pwd)"
U="$ROOT/userspace"
R="$ROOT/.tools/rust-wasm"
DEPS="$ROOT/python/deps"
TARGET=wasm32-unknown-linux-musl
NIGHTLY=nightly-2026-07-22   # as python/build-rust-toolchain.sh
B="$A/build"

[[ -f "$U/sysroot/lib/crt1.o" ]] || { echo "missing userspace/sysroot; run: ./build.sh userspace" >&2; exit 1; }
[[ -f "$DEPS/lib/libcrypto.a" ]] || { echo "missing the guest's OpenSSL; run: ./build.sh python" >&2; exit 1; }
bash "$ROOT/python/build-rust-toolchain.sh" >/dev/null

export RUSTUP_HOME="$ROOT/.tools/rustup" CARGO_HOME="$ROOT/.tools/cargo"
export PATH="$CARGO_HOME/bin:$PATH"
# Build scripts are host programs, and this machine's C compiler is clang-19 (no `cc`).
HOST_TRIPLE="$(rustc +"$NIGHTLY" -vV | sed -n 's/^host: //p')"
HOST_ENV="$(echo "$HOST_TRIPLE" | tr a-z- A-Z_)"
export "CARGO_TARGET_${HOST_ENV}_LINKER=clang-19" "CC_${HOST_TRIPLE//-/_}=clang-19"
mkdir -p "$B" "$A/out"

if [[ -n "${TEST:-}" ]]; then
  echo "== c-sh: unit tests ($HOST_TRIPLE)"
  cargo +"$NIGHTLY" test --manifest-path "$A/rust/Cargo.toml" --target-dir "$B/host" --locked
fi

echo "== c-sh (Rust -> $TARGET)"
# Built from a copy: the libc patch below replaces a crates.io package in the lock file, which
# must not rewrite the checked-in one. rsync -c keeps unchanged files' times, so cargo rebuilds
# only after a change.
mkdir -p "$B/src"
rsync -a -c --delete "$A/rust/src/" "$B/src/src/"
rsync -a -c "$A/rust/Cargo.toml" "$A/rust/Cargo.lock" "$A/rust/build.rs" "$B/src/"
# rustc hands the linker --target=wasm32-unknown-linux-musl (the spec's name), which clang-19
# behind wasm-cc does not know; wasm-cc sets its own --target=wasm32.
cat > "$B/wasm-cc-rust" <<SH
#!/bin/sh
for a; do shift; case "\$a" in --target=*) ;; *) set -- "\$@" "\$a" ;; esac; done
exec "$U/bin/wasm-cc" "\$@"
SH
chmod +x "$B/wasm-cc-rust"
LIBC_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$R/libc/Cargo.toml" | head -1)"
cargo +"$NIGHTLY" update --manifest-path "$B/src/Cargo.toml" --config "patch.crates-io.libc.path=\"$R/libc\"" \
  -p libc --precise "$LIBC_VERSION" > "$B/c-sh.log" 2>&1 || true
# The link goes through wasm-cc with the toolchain's own wasm-ld; std is the one
# build-rust-toolchain.sh built; libc is tombl's fork (this platform's ILP32 musl ABI);
# C_SH_OPENSSL_LIB points build.rs at the guest's static libcrypto.
C_SH_OPENSSL_LIB="$DEPS/lib" WASM_LD="$R/bin/wasm-ld" RUSTC="$R/bin/rustc" \
RUSTFLAGS="-Clinker=$B/wasm-cc-rust --remap-path-prefix=$B/src=/usr/src/c-sh --remap-path-prefix=$CARGO_HOME=/usr/src/cargo" \
  cargo +"$NIGHTLY" build --release --target "$TARGET" \
    --manifest-path "$B/src/Cargo.toml" --target-dir "$B/wasm" \
    --config "patch.crates-io.libc.path=\"$R/libc\"" \
    >> "$B/c-sh.log" 2>&1 || { tail -40 "$B/c-sh.log"; echo "FAILED: c-sh (log: $B/c-sh.log)" >&2; exit 1; }
WASM="$B/wasm/$TARGET/release/c-sh.wasm"
[[ -f "$WASM" ]] || WASM="$B/wasm/$TARGET/release/c-sh"
[[ -f "$WASM" ]] || { echo "cargo wrote no c-sh binary" >&2; exit 1; }

echo "== cpio"
python3 "$U/mkinitramfs.py" --overlay -o "$A/out/c-sh.cpio" --file usr/bin/c-sh="$WASM"
cp "$A/addon.json" "$A/out/c-sh.json"

# The crates linked in, with their licenses; OpenSSL's license ships with python.cpio's, and
# Rust's std is listed by scripts/build-engine.sh.
mkdir -p "$A/out/licenses"
cargo +"$NIGHTLY" metadata --format-version 1 --offline --manifest-path "$B/src/Cargo.toml" \
    --config "patch.crates-io.libc.path=\"$R/libc\"" \
    --filter-platform "$HOST_TRIPLE" 2>/dev/null \
  | python3 -c '
import json, sys
meta = json.load(sys.stdin)
used = {n["id"] for n in meta["resolve"]["nodes"]}
for p in sorted(meta["packages"], key=lambda p: p["name"]):
    if p["id"] in used and p["name"] != "collabo-sh":
        print(p["name"], p["version"] + ":", p.get("license") or "see crate", p.get("repository") or "")
' > "$A/out/licenses/crates.txt"
echo "== c-sh: $(du -k "$WASM" | cut -f1) KiB, $(du -k "$A/out/c-sh.cpio" | cut -f1) KiB cpio"
