#!/usr/bin/env bash
# Writes patches/imagemagick/*.patch from the changes in build/src (a git repository whose first
# commit is the pristine export: build.sh's src stage), one patch per topic below. Run it before
# anything re-exports build/src (FORCE=src, a changed patch), which drops changes not saved.
set -euo pipefail
A="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
S="$A/build/src"
P="$A/patches/imagemagick"
save() { # name file...
  local name=$1; shift
  git -C "$S" diff --quiet -- "$@" && { echo "  (no changes for $name)"; return; }
  git -C "$S" diff -- "$@" > "$P/$name.patch"
  echo "  $name.patch"
}
mkdir -p "$P"
save 0001-delegates-spawned-without-fork MagickCore/delegate.c
save 0002-text-fallback-fonts MagickCore/annotate.c
save 0003-threads-every-cpu-of-the-guest MagickCore/resource.c
# a change in a file no topic names is not saved
others="$(git -C "$S" diff --name-only | grep -vxF -e MagickCore/delegate.c -e MagickCore/annotate.c -e MagickCore/resource.c || true)"
[[ -z "$others" ]] || { echo "changed, in no patch: $others" >&2; exit 1; }
