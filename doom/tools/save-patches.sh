#!/usr/bin/env bash
# Writes the changes made in build/src/<repo> (doomgeneric: a git repository whose first commit
# is the pristine export) back into patches/<repo>/*.patch. Each patch keeps its description (the
# text before its first `diff --git`) and its files; a changed file no patch has yet goes into the
# patch named by NEW=NNNN-name.patch (with the description DESC=...), or the script stops and names
# it.
#
#   tools/save-patches.sh          after editing build/src/doomgeneric
#   NEW=0004-thing.patch DESC='...' tools/save-patches.sh
set -euo pipefail
A="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO="${1:-doomgeneric}"
S="$A/build/src/$REPO"
P="$A/patches/$REPO"
mkdir -p "$P"
declare -A owner
for p in "$P"/*.patch; do
  [[ -f "$p" ]] || continue
  for f in $(sed -n 's|^diff --git a/\([^ ]*\) b/.*|\1|p' "$p"); do owner[$f]="$(basename "$p")"; done
done
# (new files too: git add -N makes them part of `git diff`)
git -C "$S" add -N . 2>/dev/null || true
changed=$(git -C "$S" diff --name-only -- . ':!.stamp')
missing=()
for f in $changed; do
  if [[ -z "${owner[$f]:-}" ]]; then
    if [[ -n "${NEW:-}" ]]; then owner[$f]="$NEW"; else missing+=("$f"); fi
  fi
done
if (( ${#missing[@]} )); then
  echo "changed files no patch has (NEW=NNNN-name.patch DESC=... to make one): ${missing[*]}" >&2
  exit 1
fi
declare -A files
for f in "${!owner[@]}"; do files[${owner[$f]}]+=" $f"; done
for name in "${!files[@]}"; do
  out="$P/$name"
  if [[ -f "$out" ]]; then
    desc="$(sed '/^diff --git /,$d' "$out")"
  else
    desc="${DESC:?a new patch needs DESC}"
  fi
  # shellcheck disable=SC2086
  diff="$(git -C "$S" diff -- ${files[$name]})"
  if [[ -z "$diff" ]]; then
    echo "  $name: no changes left (removed)"; rm -f "$out"; continue
  fi
  printf '%s\n\n%s\n' "$desc" "$diff" > "$out.tmp"
  if cmp -s "$out.tmp" "$out"; then rm "$out.tmp"; else mv "$out.tmp" "$out"; echo "  wrote $name"; fi
done
