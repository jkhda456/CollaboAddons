#!/usr/bin/env bash
# The doom add-on: DOOM in the guest's terminal — doomgeneric (Chocolate Doom's engine) with the
# terminal frontend of src/ (block characters in 24-bit colour, scaled to the terminal in real
# time), packed as out/doom.cpio (an overlay):
#
#   /usr/bin/doom                      the game (-h: its keys and options)
#   /usr/share/games/doom/freedoom1.wad, freedoom2.wad
#                                      Freedoom 0.13.0's IWADs (Phase 1, Phase 2): where the game
#                                      looks for IWADs; DOOM's own (doom1.wad, doom.wad, doom2.wad,
#                                      ...) are found there too, in the current folder, $DOOMWADDIR
#   /usr/share/doc/doom                the guide (README.md), Freedoom's COPYING and CREDITS
#
# doomgeneric is exported from its archive (sources.lock) to build/src/doomgeneric with
# patches/doomgeneric/*.patch applied: a git repository whose first commit is the pristine export,
# so `git diff` there shows the add-on's changes and tools/save-patches.sh writes them back.
#
# Stages (each skipped when its output is there; FORCE=stage[,stage] redoes one):
#   src      the export and the patches
#   wads     Freedoom's release unpacked (build/wads)
#   host     the game for this machine with clang-19 (build/host): the reference the tests compare
#            the guest's frames with; and the frontend's unit tests (test_render, test_input)
#   cross    the same for the guest, wasm32 (build/cross); make runs every time
#   stage    what the image holds (build/stage)
#   cpio     out/doom.cpio, out/doom.json, out/licenses
#
# Needs: ./build.sh userspace (musl sysroot, compiler-rt) and tools (binaryen), clang-19/lld-19,
# make, unzip, python3, network access for the first downloads.
#
#   FORCE=stage[,stage]   redo those stages: src wads host cross stage cpio
#   JOBS=N                parallel jobs (default: the number of CPUs)
set -euo pipefail
A="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$A/../.." && pwd)"
U="$ROOT/userspace"
B="$A/build"
OUT="$A/out"
DL="$B/dl"
S="$B/src/doomgeneric"
W="$B/wads"
JOBS="${JOBS:-$(nproc)}"
FORCE=",${FORCE:-},"
FREEDOOM_VERSION=0.13.0
DOOMGENERIC_REV=e257a09
VERSION="1.0 (doomgeneric $DOOMGENERIC_REV, Freedoom $FREEDOOM_VERSION)"
# the guest's programs: memory imported and shared (threads), up to 4 GiB, the function table
# exported, an 8 MiB stack (as userspace/build.sh links its programs)
GUEST_LDFLAGS="-Wl,--import-memory -Wl,--max-memory=4294967296 -Wl,--shared-memory -Wl,--export-table -Wl,-z,stack-size=8388608"

stage() { # name output...: true when the stage must run
  local name="$1"; shift
  if [[ "$FORCE" != *",$name,"* ]]; then
    local f ok=1
    for f in "$@"; do [[ -e "$f" ]] || ok=0; done
    if [[ $ok == 1 ]]; then echo "== $name: up to date"; return 1; fi
  fi
  echo "== $name"; return 0
}
redo() { FORCE="$FORCE$1,"; } # name: the stage must run (its inputs changed)
log_run() { # log command...
  local log="$1"; shift
  "$@" >> "$log" 2>&1 || { tail -40 "$log"; echo "failed: $* (log: $log)" >&2; exit 1; }
}
fetch() { # name: download sources.lock's file for name into build/dl (once), check it; prints its path
  local url sum file
  read -r url sum file < <(awk -v n="$1" '$1 == n { print $2, $3, ($4 == "" ? "" : $4) }' "$A/sources.lock")
  [[ -n "$url" ]] || { echo "not in sources.lock: $1" >&2; exit 1; }
  file="$DL/${file:-$(basename "$url")}"
  mkdir -p "$DL"
  if [[ ! -f "$file" ]]; then
    echo "  download $(basename "$file")" >&2
    curl -fsSL --retry 3 -o "$file.part" "$url" >&2
    mv "$file.part" "$file"
  fi
  echo "$sum  $file" | sha256sum -c --quiet >&2 || { echo "checksum mismatch: $file" >&2; exit 1; }
  echo "$file"
}
git_pristine() { # dir: a git repository whose first commit is what dir holds
  git -C "$1" init -q && echo .stamp >> "$1/.git/info/exclude" && git -C "$1" add -A -f
  git -C "$1" -c user.name=build -c user.email=build@localhost commit -q -m pristine
}
apply_patches() { # dir patch...
  local d="$1" p; shift
  for p in "$@"; do
    [[ -f "$p" ]] || continue
    echo "  patch $(basename "$p")"
    git -C "$d" apply --whitespace=nowarn "$p"
  done
}

[[ -f "$U/sysroot/lib/crt1.o" ]] || { echo "missing userspace/sysroot; run: ./build.sh userspace" >&2; exit 1; }
[[ -x "$ROOT/.tools/binaryen/bin/wasm-opt" ]] || { echo "missing .tools/binaryen; run: ./build.sh tools" >&2; exit 1; }
for t in clang-19 lld-19 make unzip python3; do command -v $t > /dev/null || { echo "missing $t" >&2; exit 1; }; done
chmod +x "$A"/tools/*.sh "$A"/tests/run.sh 2>/dev/null || true
mkdir -p "$B" "$OUT" "$DL"

# ── src: doomgeneric with patches/doomgeneric ────────────────────────────────────────────────────
# A patch newer than the export redoes it (and so the builds).
[[ -f "$S/.stamp" && -n "$(find "$A/patches" -name '*.patch' -newer "$S/.stamp" -print -quit)" ]] && redo src
if stage src "$S/.stamp"; then
  tb="$(fetch doomgeneric)"
  rm -rf "$S"; mkdir -p "$S"
  tar -xzmf "$tb" -C "$S" --strip-components=1
  git_pristine "$S"
  apply_patches "$S" "$A"/patches/doomgeneric/*.patch
  touch "$S/.stamp"
  rm -rf "$B/host" "$B/cross"   # the objects of the old sources
fi

# ── wads: Freedoom ───────────────────────────────────────────────────────────────────────────────
FD="$W/freedoom-$FREEDOOM_VERSION"
if stage wads "$FD/freedoom1.wad" "$FD/freedoom2.wad" "$FD/COPYING.txt"; then
  zip="$(fetch freedoom)"
  rm -rf "$W"; mkdir -p "$W"
  unzip -q -o "$zip" -d "$W"
  touch "$FD"/*.wad
fi

# ── host and cross: the game and the unit tests ─────────────────────────────────────────────────
# make knows what changed in the sources; the C library or compiler-rt newer than the guest's
# program (a fix in userspace) relinks it.
MK=(make -s -f "$A/src/Makefile" SRC="$S/doomgeneric" VERSION="$VERSION" -j"$JOBS")
echo "== host"
log_run "$B/host.log" "${MK[@]}" OUT="$B/host" CC=clang-19 LDFLAGS="-fuse-ld=lld-19" CFLAGS="-ffp-contract=off"
if [[ -f "$B/cross/doom" ]]; then
  for lib in "$U/sysroot/lib/libc.a" "$U/out/rt/lib/wasm32-unknown-unknown/libclang_rt.builtins.a"; do
    [[ "$lib" -nt "$B/cross/doom" ]] && { echo "  $(basename "$lib") is newer: relink"; rm -f "$B"/cross/{doom,test_render,test_input}; break; }
  done
fi
echo "== cross"
log_run "$B/cross.log" "${MK[@]}" OUT="$B/cross" CC="$U/bin/wasm-cc" LDFLAGS="$GUEST_LDFLAGS" CFLAGS="-ffp-contract=off"

# ── stage: what the image holds ──────────────────────────────────────────────────────────────────
ST="$B/stage"
cmp -s "$B/cross/doom" "$ST/usr/bin/doom" || redo stage
[[ -f "$ST/usr/share/doc/doom/README.md" && "$A/share/doc/README.md" -nt "$ST/usr/share/doc/doom/README.md" ]] && redo stage
if stage stage "$ST/usr/bin/doom" "$ST/usr/share/games/doom/freedoom1.wad" "$ST/usr/share/doc/doom/README.md"; then
  rm -rf "$ST"
  mkdir -p "$ST/usr/bin" "$ST/usr/share/games/doom" "$ST/usr/share/doc/doom"
  cp "$B/cross/doom" "$ST/usr/bin/doom"
  # (hard links: the WADs are 28 MB each)
  ln "$FD/freedoom1.wad" "$ST/usr/share/games/doom/" 2>/dev/null || cp "$FD/freedoom1.wad" "$ST/usr/share/games/doom/"
  ln "$FD/freedoom2.wad" "$ST/usr/share/games/doom/" 2>/dev/null || cp "$FD/freedoom2.wad" "$ST/usr/share/games/doom/"
  cp "$A/share/doc/README.md" "$ST/usr/share/doc/doom/README.md"
  cp "$FD/COPYING.txt" "$ST/usr/share/doc/doom/Freedoom-COPYING.txt"
  cp "$FD/CREDITS.txt" "$ST/usr/share/doc/doom/Freedoom-CREDITS.txt"
  redo cpio
fi

# ── cpio ─────────────────────────────────────────────────────────────────────────────────────────
CPIO="$OUT/doom.cpio"
[[ -f "$CPIO" && "$A/addon.json" -nt "$CPIO" ]] && redo cpio
if stage cpio "$CPIO" "$OUT/doom.json" "$OUT/licenses/GPL-3.0.txt"; then
  trap 'rm -f "$CPIO"' ERR
  python3 "$U/mkinitramfs.py" --overlay -o "$CPIO" --tree usr/bin="$ST/usr/bin" \
    --tree usr/share/games/doom="$ST/usr/share/games/doom" --tree usr/share/doc/doom="$ST/usr/share/doc/doom"
  cp "$A/addon.json" "$OUT/doom.json"
  # doomgeneric (Chocolate Doom, id Software's DOOM source) is GPL-2.0-or-later; Freedoom BSD-3-Clause;
  # the add-on's own work (the terminal frontend, its patches, tools, tests): ../LICENSE, GPL-3.0
  L="$OUT/licenses"
  rm -rf "$L"; mkdir -p "$L"
  cp "$A/../LICENSE" "$L/GPL-3.0.txt"
  cp "$S/LICENSE" "$L/doomgeneric-LICENSE.txt"
  cp "$FD/COPYING.txt" "$L/freedoom-COPYING.txt"
  trap - ERR
fi
ls -l "$CPIO"
