#!/usr/bin/env bash
# Checks for the rust add-on, in the guest (test_guest.py, guest.sh): rustc and cargo of the
# guest's own target — programs (unwinding panics, threads, processes), compile errors, a cargo
# workspace with a proc macro (a side module rustc loads with dlopen), a build script, unit,
# integration and doc tests, `cargo new`, clippy and rustfmt. Offline; NET=1 adds crates.io (serde),
# LLVM=1 the llvm add-on (C in build scripts, zstd).
#
# The guest is the packaged runtime (dist/runtime/collabo-core-<platform>, COLLABO_RUNTIME) with
# out/rust.cpio (RUST_CPIO); RUST_ENGINE picks another engine.
set -euo pipefail
exec python3 "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/test_guest.py" "$@"
