#!/usr/bin/env bash
# The guided-arrow add-on: `guided-arrow`, verified command-line cheat sheets (tldr pages and the
# sheets cheat.sh serves) carried in an archive of its own compression, packed as
# out/guided-arrow.cpio (an overlay: /usr/bin/guided-arrow, /usr/share/guided-arrow/guided-arrow.gar).
#
# One Rust program (rust/, no crates) does both halves: built for this machine it packs the
# archive from the pinned upstream commits below; built for the guest it reads it.
#
# Sources: each repository at its pinned commit, cloned into build/src from a local clone under
# GUIDED_ARROW_SRC (default ~/workspace/guided-arrow, which has tldr/, cheat.sheets/,
# cheatsheets/) when it has the commit, else from GitHub.
#
# Needs the userspace step (musl, compiler-rt: userspace/sysroot) and the guest's Rust toolchain
# (python/build-rust-toolchain.sh, which this runs; it skips when it is up to date).
#
#   addons/guided-arrow/build.sh
#   TEST=1 addons/guided-arrow/build.sh       also run the unit tests natively first
#   GUIDED_ARROW_LANGS=en,ko addons/...       tldr's pages in these languages too (default: en,
#                                             English only; all: every translation)
#   GUIDED_ARROW_EXTRA="team=DIR ..."         also pack these folders of sheets (every file a sheet
#                                             in the cheat format) into the archive, each as a source
#                                             of that name (sheets added at run time: --add)
#   FORCE=pack addons/guided-arrow/build.sh   pack the archive again even if nothing changed
set -euo pipefail

A="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$A/../.." && pwd)"
U="$ROOT/userspace"
R="$ROOT/.tools/rust-wasm"
TARGET=wasm32-unknown-linux-musl
NIGHTLY=nightly-2026-07-22   # as python/build-rust-toolchain.sh
B="$A/build"

# The upstreams (cheat.sh itself is a server; these are the sheets it serves, plus tldr).
TLDR_REV=e04d778abf4966667808c70b21e62ad79d8bb462          # tldr-pages/tldr, 2026-10-28
CHEAT_SHEETS_REV=0bcedfb1d218d1218bbc0e9cf809ebca0d367e6d  # chubin/cheat.sheets, 2025-03-21
CHEATSHEETS_REV=36bdb99dcfadde210503d8c2dcf94b34ee950e1d   # cheat/cheatsheets, 2022-12-17
SRC_HINT="${GUIDED_ARROW_SRC:-$HOME/workspace/guided-arrow}"
LANGS="${GUIDED_ARROW_LANGS:-en}"
EXTRA="${GUIDED_ARROW_EXTRA:-}"

[[ -f "$U/sysroot/lib/crt1.o" ]] || { echo "missing userspace/sysroot; run: ./build.sh userspace" >&2; exit 1; }
bash "$ROOT/python/build-rust-toolchain.sh" >/dev/null

export RUSTUP_HOME="$ROOT/.tools/rustup" CARGO_HOME="$ROOT/.tools/cargo"
export PATH="$CARGO_HOME/bin:$PATH"
# Build scripts are host programs, and this machine's C compiler is clang-19 (no `cc`).
HOST_TRIPLE="$(rustc +"$NIGHTLY" -vV | sed -n 's/^host: //p')"
HOST_ENV="$(echo "$HOST_TRIPLE" | tr a-z- A-Z_)"
export "CARGO_TARGET_${HOST_ENV}_LINKER=clang-19" "CC_${HOST_TRIPLE//-/_}=clang-19"
mkdir -p "$B/src" "$A/out"

# fetch NAME GITHUB-REPO COMMIT: build/src/NAME checked out at COMMIT.
fetch() {
  local name="$1" url="https://github.com/$2" rev="$3" d="$B/src/$1"
  if [[ -d "$d/.git" && "$(git -C "$d" rev-parse HEAD 2>/dev/null)" == "$rev" ]]; then
    return
  fi
  echo "== source: $name at ${rev:0:12}"
  if [[ ! -d "$d/.git" ]]; then
    if [[ -d "$SRC_HINT/$name/.git" ]]; then
      git -c safe.directory='*' clone -q --no-checkout "$SRC_HINT/$name" "$d"
    else
      git clone -q --no-checkout --filter=blob:none "$url" "$d"
    fi
  fi
  git -C "$d" cat-file -e "$rev^{commit}" 2>/dev/null || git -C "$d" fetch -q "$url" "$rev"
  git -C "$d" -c advice.detachedHead=false checkout -q --force "$rev"
}
fetch tldr tldr-pages/tldr "$TLDR_REV"
fetch cheat.sheets chubin/cheat.sheets "$CHEAT_SHEETS_REV"
fetch cheatsheets cheat/cheatsheets "$CHEATSHEETS_REV"

if [[ -n "${TEST:-}" ]]; then
  echo "== guided-arrow: unit tests ($HOST_TRIPLE)"
  cargo +"$NIGHTLY" test --manifest-path "$A/rust/Cargo.toml" --target-dir "$B/host" --locked
fi

echo "== guided-arrow (Rust -> $HOST_TRIPLE, the packer)"
cargo +"$NIGHTLY" build --release --manifest-path "$A/rust/Cargo.toml" --target-dir "$B/host" --locked \
  > "$B/host.log" 2>&1 || { tail -40 "$B/host.log"; echo "FAILED: guided-arrow for $HOST_TRIPLE (log: $B/host.log)" >&2; exit 1; }
HOST_BIN="$B/host/release/guided-arrow"

# Extra folders of sheets: a source each, and what is in them part of the stamp.
EXTRA_ARGS=()
EXTRA_STAMP=""
for e in $EXTRA; do
  id="${e%%=*}" dir="${e#*=}"
  [[ "$id" != "$e" && -n "$id" && -d "$dir" ]] || { echo "GUIDED_ARROW_EXTRA: $e: want NAME=FOLDER (a folder of sheets)" >&2; exit 1; }
  dir="$(cd "$dir" && pwd)"
  [[ "$dir" != *,* ]] || { echo "GUIDED_ARROW_EXTRA: $dir: a folder without a comma in its path, please" >&2; exit 1; }
  # Its revision: the commit of the repository it is in (+changes when it has uncommitted ones).
  spec="$id=$dir"
  if rev="$(git -C "$dir" rev-parse HEAD 2>/dev/null)"; then
    [[ -z "$(git -C "$dir" status --porcelain -- . 2>/dev/null)" ]] || rev+="+changes"
    spec+=",rev=$rev,date=$(git -C "$dir" log -1 --format=%cs)"
  fi
  EXTRA_ARGS+=(--source "$spec")
  EXTRA_STAMP+=" $id=$(cd "$dir" && find . -type f -not -path '*/.*' -print0 | sort -z | xargs -0 -r sha256sum | sha256sum | cut -c1-16)"
done

# The archive, packed again when the packer, a commit, the languages or the extra sheets changed.
GAR="$B/guided-arrow.gar"
STAMP="$(sha256sum "$HOST_BIN" | cut -c1-16) $TLDR_REV $CHEAT_SHEETS_REV $CHEATSHEETS_REV $LANGS$EXTRA_STAMP"
if [[ ! -f "$GAR" || "$(cat "$GAR.stamp" 2>/dev/null)" != "$STAMP" || ",${FORCE:-}," == *,pack,* ]]; then
  echo "== archive (languages: $LANGS${EXTRA:+; extra: $EXTRA})"
  date_of() { git -C "$B/src/$1" log -1 --format=%cs; }
  "$HOST_BIN" --pack -o "$GAR" --quiet --langs "$LANGS" --created "$(date_of tldr)" \
    --source "tldr=$B/src/tldr,rev=$TLDR_REV,date=$(date_of tldr)" \
    --source "cheat.sheets=$B/src/cheat.sheets,rev=$CHEAT_SHEETS_REV,date=$(date_of cheat.sheets)" \
    --source "cheatsheets=$B/src/cheatsheets,rev=$CHEATSHEETS_REV,date=$(date_of cheatsheets)" \
    ${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}
  echo "$STAMP" > "$GAR.stamp"
fi
"$HOST_BIN" --data "$GAR" --verify

echo "== guided-arrow (Rust -> $TARGET)"
# rustc hands the linker --target=wasm32-unknown-linux-musl (the spec's name), which clang-19
# behind wasm-cc does not know; wasm-cc sets its own --target=wasm32.
cat > "$B/wasm-cc-rust" <<SH
#!/bin/sh
for a; do shift; case "\$a" in --target=*) ;; *) set -- "\$@" "\$a" ;; esac; done
exec "$U/bin/wasm-cc" "\$@"
SH
chmod +x "$B/wasm-cc-rust"
# The link goes through wasm-cc with the toolchain's own wasm-ld; std is the one
# build-rust-toolchain.sh built. There are no crates, so no libc fork to patch in.
WASM_LD="$R/bin/wasm-ld" RUSTC="$R/bin/rustc" \
RUSTFLAGS="-Clinker=$B/wasm-cc-rust --remap-path-prefix=$A/rust=/usr/src/guided-arrow --remap-path-prefix=$CARGO_HOME=/usr/src/cargo" \
  cargo +"$NIGHTLY" build --release --target "$TARGET" --locked \
    --manifest-path "$A/rust/Cargo.toml" --target-dir "$B/wasm" \
    > "$B/wasm.log" 2>&1 || { tail -40 "$B/wasm.log"; echo "FAILED: guided-arrow for $TARGET (log: $B/wasm.log)" >&2; exit 1; }
WASM="$B/wasm/$TARGET/release/guided-arrow.wasm"
[[ -f "$WASM" ]] || WASM="$B/wasm/$TARGET/release/guided-arrow"
[[ -f "$WASM" ]] || { echo "cargo wrote no guided-arrow binary" >&2; exit 1; }

echo "== cpio"
python3 "$U/mkinitramfs.py" --overlay -o "$A/out/guided-arrow.cpio" \
  --file usr/bin/guided-arrow="$WASM" \
  --file usr/share/guided-arrow/guided-arrow.gar="$GAR"
cp "$A/addon.json" "$A/out/guided-arrow.json"

# The program's own license (the add-ons' GPL-3.0, ../LICENSE) and the pages' (the archive
# carries them, and --info names each source's).
mkdir -p "$A/out/licenses"
cp "$A/../LICENSE" "$A/out/licenses/GPL-3.0.txt"
cp "$B/src/tldr/LICENSE.md" "$A/out/licenses/tldr-pages.CC-BY-4.0.md"
cp "$B/src/cheat.sheets/LICENSE" "$A/out/licenses/cheat.sheets.MIT.txt"
cp "$B/src/cheatsheets/.github/LICENSE.txt" "$A/out/licenses/cheatsheets.CC0-1.0.txt"
echo "== guided-arrow: $(du -k "$WASM" | cut -f1) KiB program, $(du -k "$GAR" | cut -f1) KiB archive," \
  "$(du -k "$A/out/guided-arrow.cpio" | cut -f1) KiB cpio"
