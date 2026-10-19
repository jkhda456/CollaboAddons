#!/usr/bin/env bash
# c-sh's checks:
#
#   tests/run.sh            unit tests (crypto vectors, SSH against this machine's OpenSSH: every
#                           cipher, MAC and key exchange c-sh offers); the scripted session
#                           (tests/test_csh.py) with a native build; the same session in the guest
#   tests/run.sh native     without a runtime
#
# Needs OpenSSH's sshd and ssh-keygen here (the SSH server the tests talk to, also from the
# guest, as 192.0.2.1), python3, and this machine's libcrypto.so.3 for the native build. The guest
# half boots RUNTIME (default dist/runtime/collabo-core-<platform>) with out/c-sh.cpio
# (addons/c-sh/build.sh) and python.cpio (the telnet/rlogin/raw test servers run in the guest).
set -euo pipefail
T="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A="$(dirname "$T")"
ROOT="$(cd "$A/../.." && pwd)"
NIGHTLY=nightly-2026-07-22
export RUSTUP_HOME="$ROOT/.tools/rustup" CARGO_HOME="$ROOT/.tools/cargo"
export PATH="$CARGO_HOME/bin:$PATH"
HOST_TRIPLE="$(rustc +"$NIGHTLY" -vV | sed -n 's/^host: //p')"
export "CARGO_TARGET_$(echo "$HOST_TRIPLE" | tr a-z- A-Z_)_LINKER=clang-19"

echo "== unit tests"
cargo +"$NIGHTLY" test --quiet --manifest-path "$A/rust/Cargo.toml" --target-dir "$A/build/host" 2>&1 \
  | grep -E "^test result|FAILED|panicked"
echo "== native build"
cargo +"$NIGHTLY" build --quiet --manifest-path "$A/rust/Cargo.toml" --target-dir "$A/build/host"
echo "== native session"
python3 "$T/test_csh.py" native

[[ "${1:-}" == native ]] && exit 0
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=linux-x64 ;; Linux-aarch64) platform=linux-arm64 ;;
  Darwin-arm64) platform=darwin-arm64 ;; Darwin-x86_64) platform=darwin-x64 ;;
esac
RT="${1:-$ROOT/dist/runtime/collabo-core-$platform}"
[[ -f "$A/out/c-sh.cpio" ]] || { echo "build the add-on first: addons/c-sh/build.sh" >&2; exit 1; }
[[ -f "$RT/app/images/python.cpio" ]] || { echo "no runtime with python.cpio at $RT" >&2; exit 1; }
echo "== the session in the guest"
python3 "$T/test_csh.py" guest "$RT"
