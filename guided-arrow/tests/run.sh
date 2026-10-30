#!/usr/bin/env bash
# guided-arrow's checks:
#
#   tests/run.sh            unit tests (the codec, the archive format, the page readers, search,
#                           JSON); the scripted steps (tests/test_guided_arrow.py) with the native
#                           build and build/guided-arrow.gar; the same steps in the guest
#   tests/run.sh native     without a runtime
#   tests/run.sh RUNTIME    the guest half on that runtime (default dist/runtime/collabo-core-<platform>)
#
# Needs the add-on built first (addons/guided-arrow/build.sh: build/src, build/guided-arrow.gar,
# out/guided-arrow.cpio) and python3.
set -euo pipefail
T="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A="$(dirname "$T")"
ROOT="$(cd "$A/../.." && pwd)"
NIGHTLY=nightly-2026-07-22
export RUSTUP_HOME="$ROOT/.tools/rustup" CARGO_HOME="$ROOT/.tools/cargo"
export PATH="$CARGO_HOME/bin:$PATH"
HOST_TRIPLE="$(rustc +"$NIGHTLY" -vV | sed -n 's/^host: //p')"
export "CARGO_TARGET_$(echo "$HOST_TRIPLE" | tr a-z- A-Z_)_LINKER=clang-19"

[[ -f "$A/build/guided-arrow.gar" ]] || { echo "build the add-on first: addons/guided-arrow/build.sh" >&2; exit 1; }
echo "== unit tests"
cargo +"$NIGHTLY" test --quiet --release --manifest-path "$A/rust/Cargo.toml" --target-dir "$A/build/host" 2>&1 \
  | grep -E "^test result|FAILED|panicked"
echo "== native build"
cargo +"$NIGHTLY" build --quiet --release --manifest-path "$A/rust/Cargo.toml" --target-dir "$A/build/host"
echo "== native steps"
python3 "$T/test_guided_arrow.py" native

[[ "${1:-}" == native ]] && exit 0
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=linux-x64 ;; Linux-aarch64) platform=linux-arm64 ;;
  Darwin-arm64) platform=darwin-arm64 ;; Darwin-x86_64) platform=darwin-x64 ;;
esac
RT="${1:-$ROOT/dist/runtime/collabo-core-$platform}"
[[ -f "$A/out/guided-arrow.cpio" ]] || { echo "build the add-on first: addons/guided-arrow/build.sh" >&2; exit 1; }
[[ -f "$RT/app/images/tools.cpio" ]] || { echo "no runtime at $RT" >&2; exit 1; }
echo "== the steps in the guest"
python3 "$T/test_guided_arrow.py" guest "$RT"
