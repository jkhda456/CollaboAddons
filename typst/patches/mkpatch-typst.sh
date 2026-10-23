#!/usr/bin/env bash
# Write the changes made to files of the exported source (build/src/...) as
# patches/typst/<name>.patch, against the clone at build.sh's TYPST_REV.
#
#   patches/mkpatch-typst.sh NNNN-what crates/typst-cli/Cargo.toml crates/typst-cli/src/host.rs ...
#
# Each file belongs to one patch: a later patch touching it again would repeat the earlier one's
# changes. New files (absent at TYPST_REV) are written as additions.
set -euo pipefail
P="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A="$(dirname "$P")"
ROOT="$(cd "$A/../.." && pwd)"
TYPST_SRC="${TYPST_SRC:-$ROOT/../org/typst}"
REV="$(sed -n 's/^TYPST_REV=\([0-9a-f]*\).*/\1/p' "$A/build.sh")"
name="$1"; shift
mkdir -p "$P/typst"
out="$P/typst/$name.patch"
: > "$out"
for file in "$@"; do
  file="${file#./}"
  [[ -e "$A/build/src/$file" ]] || { echo "no build/src/$file" >&2; exit 1; }
  if git -C "$TYPST_SRC" cat-file -e "$REV:$file" 2>/dev/null; then
    git -C "$TYPST_SRC" show "$REV:$file" > "$A/build/.pristine"
    diff -u --label "a/$file" --label "b/$file" "$A/build/.pristine" "$A/build/src/$file" >> "$out" || true
  else
    diff -u --label /dev/null --label "b/$file" /dev/null "$A/build/src/$file" >> "$out" || true
  fi
done
rm -f "$A/build/.pristine"
[[ -s "$out" ]] || { rm -f "$out"; echo "$name: no changes"; exit 0; }
echo "$name: $(grep -c '^[-+][^-+]' "$out") changed lines -> patches/typst/$name.patch"
