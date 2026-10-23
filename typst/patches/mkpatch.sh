#!/usr/bin/env bash
# Write the changes made by hand to a vendored crate (build/vendor/<crate>-<version>) as
# patches/crates/<crate>-<version>@<what>.patch, against the crate as published plus its
# narrowed-cfg patch (<crate>-<version>.patch), if it has one.
#
#   patches/mkpatch.sh CRATE-VERSION WHAT        e.g. patches/mkpatch.sh linux-raw-sys-0.12.1 wasm32
#
# The pristine crate comes from cargo's registry cache (or its git checkouts), where
# `cargo vendor` found it.
set -euo pipefail
P="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A="$(dirname "$P")"
ROOT="$(cd "$A/../.." && pwd)"
crate="$1" what="$2"
vendored="$A/build/vendor/$crate"
[[ -d "$vendored" ]] || { echo "no vendored crate $crate" >&2; exit 1; }
pristine="$(ls -d "$ROOT/.tools/cargo/registry/src/"*/"$crate" 2>/dev/null | head -1)"
if [[ -z "$pristine" ]]; then
  # A git dependency: find the checkout holding this package and version.
  name="${crate%-*}" version="${crate##*-}"
  for manifest in "$ROOT/.tools/cargo/git/checkouts"/*/*/Cargo.toml "$ROOT/.tools/cargo/git/checkouts"/*/*/*/Cargo.toml; do
    if grep -q "^name = \"$name\"" "$manifest" 2>/dev/null && grep -q "^version = \"$version\"" "$manifest"; then
      pristine="$(dirname "$manifest")"; break
    fi
  done
fi
[[ -n "$pristine" ]] || { echo "no pristine copy of $crate in cargo's caches" >&2; exit 1; }

work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
cp -r "$pristine" "$work/a"
[[ -f "$P/crates/$crate.patch" ]] && patch -p1 --quiet -d "$work/a" < "$P/crates/$crate.patch"
cp -r "$vendored" "$work/b"
out="$P/crates/$crate@$what.patch"
(cd "$work" && diff -ruN -x .cargo-checksum.json -x .cargo_vcs_info.json -x .cargo-ok -x target -x .gitignore -x .github a b) \
  | sed -E 's/^((---|\+\+\+) [^\t]+)\t.*$/\1/' > "$out" || true
[[ -s "$out" ]] || { rm -f "$out"; echo "$crate: no changes by hand"; exit 0; }
echo "$crate: $(grep -c '^[-+][^-+]' "$out") changed lines -> patches/crates/$(basename "$out")"
