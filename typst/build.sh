#!/usr/bin/env bash
# The typst add-on: Typst (the typesetting system, typst-cli, Rust) for the guest, cross-compiled
# as a wasm32 Linux program and packed as out/typst.cpio (an overlay):
#
#   /usr/bin/typst                       typst itself (compile, watch, query, eval, init, fonts, info)
#   /usr/share/fonts/nanum               Korean fonts: Nanum Gothic, Myeongjo, Gothic Coding
#   /usr/share/typst/packages            packages the guest has without a download: @collabo/report,
#                                        and Typst Universe ones (cmarker, mitex, cetz, lilaq, …)
#   /usr/share/doc/typst                 the agents' guide (AGENTS.md) and examples
#   /etc/codex/skills/typst-report       the guide as a skill, for Codex (its system skills),
#   /root/.agent/skills/typst-report     agent (the agent add-on: the guest user's skills) and
#   /root/.claude/skills/typst-report    Claude Code (the guest user's skills; the node add-on runs it)
#
# Stages (each skipped when its output is up to date):
#   src     typst at TYPST_REV, exported from the clone, with patches/typst/*.patch applied; its
#           crates vendored (cargo vendor) with patches/crates/<crate>-<version>[@what].patch
#   cross   typst, cross-compiled for wasm32 (Linux, musl) with the guest's Rust toolchain
#   fonts   the fonts sources.lock names, downloaded into build/dl and checked
#   packages  the Typst Universe packages sources.lock names (universe:…), likewise, unpacked
#   cpio    pack out/typst.cpio, and the licenses
#
# Needs: ./build.sh userspace (musl sysroot, compiler-rt) and the guest's Rust toolchain
# (python/build-rust-toolchain.sh, which this runs; it skips when up to date), the typst clone at
# ../org/typst (beside CollaboCore; never modified), network access for the first vendoring and
# the fonts.
#
#   FORCE=src,cross,fonts,packages,cpio  re-run the named stages
#   JOBS=N                      cargo's -j (default: the number of CPUs)
#   TEST=1                      also run the add-on's unit tests (typst-cli's host module) natively
#                               (builds typst for this machine in build/host: about 2 GB)
set -euo pipefail

A="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$A/../.." && pwd)"
TYPST_SRC="${TYPST_SRC:-$ROOT/../org/typst}"
TYPST_REV=9dfd3a08500b7896045f907433cf7b4b02434fad   # v0.15.1
U="$ROOT/userspace"
R="$ROOT/.tools/rust-wasm"
TARGET=wasm32-unknown-linux-musl
NIGHTLY=nightly-2026-07-22   # as python/build-rust-toolchain.sh
B="$A/build"
OUT="$A/out"
S="$B/src"
V="$B/vendor"
FORCE=",${FORCE:-},"

[[ -f "$U/sysroot/lib/crt1.o" ]] || { echo "missing userspace/sysroot; run: ./build.sh userspace" >&2; exit 1; }
[[ -d "$TYPST_SRC/.git" ]] || { echo "missing the typst clone at $TYPST_SRC (git clone https://github.com/typst/typst)" >&2; exit 1; }
bash "$ROOT/python/build-rust-toolchain.sh"

export RUSTUP_HOME="$ROOT/.tools/rustup" CARGO_HOME="$ROOT/.tools/cargo"
export PATH="$CARGO_HOME/bin:$PATH"
CARGO=(cargo "+$NIGHTLY")
# Build scripts and proc macros are host programs, and this machine's C compiler is clang-19 (no `cc`).
HOST_TRIPLE="$(rustc +"$NIGHTLY" -vV | sed -n 's/^host: //p')"
HOST_ENV="$(echo "$HOST_TRIPLE" | tr a-z- A-Z_)"
export "CARGO_TARGET_${HOST_ENV}_LINKER=clang-19" "CC_${HOST_TRIPLE//-/_}=clang-19" "CXX_${HOST_TRIPLE//-/_}=clang++-19"
export "AR_${HOST_TRIPLE//-/_}=llvm-ar-19"
# typst --version names the commit; the exported tree has no .git to ask.
export TYPST_COMMIT_SHA="$TYPST_REV"

stage() {
  local name=$1 out=$2; shift 2
  if [[ "$FORCE" == *",$name,"* || ! -e "$out" ]]; then echo "==== stage $name"; return 0; fi
  local src
  for src in "$@"; do
    [[ -e "$src" && "$src" -nt "$out" ]] && { echo "==== stage $name ($(basename "$src") changed)"; return 0; }
  done
  echo "==== stage $name: up to date"; return 1
}
fail() { tail -40 "$1"; echo "FAILED: $2 (log: $1)" >&2; exit 1; }
# fetch NAME: download (once) the file sources.lock names into build/dl, check its sha256, print
# its path.
fetch() {
  local name="$1" url sum file
  read -r _ url sum < <(grep -E "^$name " "$A/sources.lock") \
    || { echo "fetch: $name is not in sources.lock" >&2; return 1; }
  file="$B/dl/$(basename "$url")"
  mkdir -p "$B/dl"
  if [[ ! -f "$file" ]]; then
    echo "  fetch $(basename "$url")" >&2
    curl -fsSL --retry 3 -o "$file.part" "$url" && mv "$file.part" "$file"
  fi
  echo "$sum  $file" | sha256sum -c --quiet - >&2 \
    || { echo "checksum mismatch: $file (deleted)" >&2; rm -f "$file"; return 1; }
  echo "$file"
}

mkdir -p "$B" "$OUT"

# ── src: the source tree and its vendored crates ────────────────────────────────────────────
# patches/typst/*.patch apply to the exported tree, patches/crates/*.patch inside
# build/vendor/<crate>-<version>: <crate>-<version>.patch narrows the crate's wasm cfgs (made by
# patches/narrow-wasm-cfg.py), <crate>-<version>@<what>.patch are changes by hand
# (patches/mkpatch.sh). Both trees are made afresh beside the old ones and synced into them by
# content, so a file that came out the same keeps its time and cargo rebuilds only what a change
# touched.
if stage src "$S/.stamp" "$A/build.sh" "$A/patches/typst" "$A/patches/crates" "$A/patches/typst"/*.patch \
     "$A/patches/crates"/*.patch; then
  git -C "$TYPST_SRC" cat-file -e "$TYPST_REV^{commit}" 2>/dev/null \
    || { echo "the typst clone has no commit $TYPST_REV (git -C $TYPST_SRC fetch --tags)" >&2; exit 1; }
  rm -rf "$B/src.new" "$B/vendor.new"; mkdir -p "$B/src.new" "$S" "$V"
  # -m: the commit's dates may be ahead of this machine's clock, which would confuse cargo.
  git -C "$TYPST_SRC" archive --format=tar "$TYPST_REV" | tar -x -m -C "$B/src.new"
  for patch in "$A/patches/typst"/*.patch; do
    [[ -e "$patch" ]] || continue
    patch -p1 --quiet --no-backup-if-mismatch -d "$B/src.new" < "$patch" || { echo "FAILED: $(basename "$patch")" >&2; exit 1; }
  done
  echo "  vendoring crates"
  # Not --locked: the patches drop dependencies (OpenSSL), which takes them out of the lock file.
  "${CARGO[@]}" vendor --versioned-dirs --manifest-path "$B/src.new/Cargo.toml" "$B/vendor.new" \
      > "$B/vendor-config.toml" 2> "$B/vendor.log" || fail "$B/vendor.log" "cargo vendor"
  for patch in "$A/patches/crates"/*.patch; do
    [[ -e "$patch" ]] || continue
    crate="$(basename "$patch" .patch)"; crate="${crate%%@*}"   # <crate>-<version>[@what].patch
    [[ -d "$B/vendor.new/$crate" ]] || { echo "FAILED: $(basename "$patch"): no vendored crate $crate" >&2; exit 1; }
    patch -p1 --quiet --no-backup-if-mismatch -d "$B/vendor.new/$crate" < "$patch" || { echo "FAILED: $(basename "$patch")" >&2; exit 1; }
    # cargo checks vendored files against their checksums; a patched crate keeps only the package's.
    python3 - "$B/vendor.new/$crate/.cargo-checksum.json" <<'PY'
import json, sys
path = sys.argv[1]
data = json.load(open(path))
data["files"] = {}
json.dump(data, open(path, "w"))
PY
  done
  # cargo takes vendored crates for immutable, as registry ones: it does not look at their files'
  # times. A crate whose files came out different (its patches changed) is listed for the cross
  # stage to clean.
  if [[ -d "$B/target" ]]; then
    rsync -rlc --delete --dry-run --itemize-changes "$B/vendor.new/" "$V/" \
      | sed -nE 's|^[^ ]+ +([^/]+)/.+$|\1|p' | sort -u >> "$B/stale-crates.txt"
  fi
  rsync -rlc --delete "$B/vendor.new/" "$V/" && rm -rf "$B/vendor.new"
  # The vendored sources, and libc from tombl's fork (this platform's ILP32 musl ABI).
  mkdir -p "$B/src.new/.cargo"
  {
    sed -n '/^\[source/,$p' "$B/vendor-config.toml" | sed "s|$B/vendor.new|$V|"
    printf '\n[patch.crates-io]\nlibc = { path = "%s" }\n' "$R/libc"
  } > "$B/src.new/.cargo/config.toml"
  # The lock file names crates.io's libc; resolve it to the fork (the same version).
  LIBC_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$R/libc/Cargo.toml" | head -1)"
  (cd "$B/src.new" && "${CARGO[@]}" update --offline -p libc --precise "$LIBC_VERSION") >> "$B/vendor.log" 2>&1 \
    || fail "$B/vendor.log" "cargo update -p libc"
  rsync -rlc --delete --exclude .stamp "$B/src.new/" "$S/" && rm -rf "$B/src.new"
  touch "$S/.stamp"
fi

if [[ -n "${TEST:-}" ]]; then
  echo "== unit tests: typst-cli's host module ($HOST_TRIPLE)"
  # Without debug information: typst's test build is otherwise over 4 GB.
  (cd "$S" && CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 "${CARGO[@]}" test --offline -p typst-cli \
     --bin typst --target-dir "$B/host" -j "${JOBS:-$(nproc)}" host::) 2>&1 | tail -15
  [[ ${PIPESTATUS[0]} -eq 0 ]] || { echo "FAILED: the host module's unit tests" >&2; exit 1; }
fi

# ── cross: typst for wasm32 ──────────────────────────────────────────────────────────────────
WASM="$B/typst.wasm"
if stage cross "$WASM" "$S/.stamp"; then
  # rustc hands the linker --target=wasm32-unknown-linux-musl (the spec's name), which clang-19
  # behind wasm-cc does not know; wasm-cc sets its own --target=wasm32.
  cat > "$B/wasm-cc-rust" <<SH
#!/bin/sh
for a; do shift; case "\$a" in --target=*) ;; *) set -- "\$@" "\$a" ;; esac; done
exec "$U/bin/wasm-cc" "\$@"
SH
  chmod +x "$B/wasm-cc-rust"
  # C in the crates (psm, …) is compiled as every guest object is, by wasm-cc.
  export "CC_${TARGET//-/_}=$B/wasm-cc-rust" "AR_${TARGET//-/_}=llvm-ar-19" "RANLIB_${TARGET//-/_}=llvm-ranlib-19"
  export WASM_MUSL_SYSROOT="$U/sysroot"   # cc wants it for wasm32 Linux targets (--sysroot)
  # The link goes through wasm-cc with the toolchain's own wasm-ld (the objects are newer LLVM
  # than clang-19's); std is the one build-rust-toolchain.sh built.
  export WASM_LD="$R/bin/wasm-ld" RUSTC="$R/bin/rustc"
  export RUSTFLAGS="-Clinker=$B/wasm-cc-rust --remap-path-prefix=$S=/usr/src/typst --remap-path-prefix=$V=/usr/src/vendor --remap-path-prefix=$CARGO_HOME=/usr/src/cargo"
  # Speed over size (typst's work is layout), no LTO: this machine's memory. Line tables are
  # useless without a symbolizer in the guest.
  export CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
         CARGO_PROFILE_RELEASE_DEBUG=false CARGO_PROFILE_RELEASE_STRIP=true
  if [[ -s "$B/stale-crates.txt" ]]; then
    for crate in $(sort -u "$B/stale-crates.txt"); do
      [[ "$crate" =~ ^(.+)-([0-9]+\.[0-9]+\.[0-9]+.*)$ ]] || continue
      echo "  rebuilding ${BASH_REMATCH[1]} ${BASH_REMATCH[2]} (its files changed)"
      for dir in "$B/target" "$B/host"; do
        [[ -d "$dir" ]] || continue
        (cd "$S" && "${CARGO[@]}" clean --offline -p "${BASH_REMATCH[1]}@${BASH_REMATCH[2]}" --target-dir "$dir" \
           $([[ "$dir" == "$B/target" ]] && echo --release --target "$TARGET")) >> "$B/clean.log" 2>&1 || true
      done
    done
    rm -f "$B/stale-crates.txt"
  fi
  echo "  cargo build (log: $B/cross.log)"
  # From the source tree: cargo reads .cargo/config.toml (the vendored sources) from the working
  # directory, not the manifest's.
  (cd "$S" && "${CARGO[@]}" build --release --offline --target "$TARGET" -p typst-cli --bin typst \
     --target-dir "$B/target" -j "${JOBS:-$(nproc)}") > "$B/cross.log" 2>&1 || fail "$B/cross.log" "typst (cargo build)"
  built="$B/target/$TARGET/release/typst"   # the target spec gives programs no suffix
  [[ -f "$built" ]] || built="$built.wasm"
  [[ -f "$built" ]] || { echo "cargo wrote no typst binary" >&2; exit 1; }
  cp "$built" "$WASM"
fi

# ── fonts ────────────────────────────────────────────────────────────────────────────────────
F="$B/fonts"
if stage fonts "$F/.stamp" "$A/sources.lock"; then
  rm -rf "$F"; mkdir -p "$F/nanum"
  for font in gothic-regular gothic-bold myeongjo-regular myeongjo-bold coding-regular coding-bold; do
    file="$(fetch "nanum-$font")"; cp "$file" "$F/nanum/"
  done
  cp "$(fetch nanum-ofl)" "$F/nanum/OFL.txt"
  touch "$F/.stamp"
fi

# ── packages: Typst Universe packages, for documents without network ─────────────────────────
P="$B/packages"
if stage packages "$P/.stamp" "$A/sources.lock"; then
  rm -rf "$P"; mkdir -p "$P/preview" "$P/licenses"
  while read -r entry _; do
    IFS=: read -r _ package version <<< "$entry"
    file="$(fetch "$entry")"
    mkdir -p "$P/preview/$package/$version"
    tar -xzmf "$file" -C "$P/preview/$package/$version"
    for license in "$P/preview/$package/$version"/LICEN[SC]E*; do
      if [[ -f "$license" ]]; then cp "$license" "$P/licenses/universe-$package-$version-$(basename "$license")"; fi
    done
  done < <(grep -E '^universe:' "$A/sources.lock")
  touch "$P/.stamp"
fi

# ── cpio ─────────────────────────────────────────────────────────────────────────────────────
newest() { find "$@" -type f -printf '%T@ %p\n' 2>/dev/null | sort -n | tail -1 | cut -d' ' -f2-; }
if stage cpio "$OUT/typst.cpio" "$WASM" "$F/.stamp" "$P/.stamp" "$A/addon.json" "$(newest "$A/share")"; then
  python3 "$U/mkinitramfs.py" --overlay -o "$OUT/typst.cpio" --file usr/bin/typst="$WASM" \
    --tree usr/share/fonts/nanum="$F/nanum" \
    --tree usr/share/typst="$A/share/typst" \
    --tree usr/share/typst/packages/preview="$P/preview" \
    --tree usr/share/doc/typst="$A/share/doc" \
    --tree etc/codex/skills/typst-report="$A/share/skill" \
    --tree root/.claude/skills/typst-report="$A/share/skill" \
    --tree root/.agent/skills/typst-report="$A/share/skill"
  # Beside it, as addons/build.sh does.
  cp "$A/addon.json" "$OUT/typst.json"
  # Typst's license and the crates linked in, with theirs; Rust's std is listed by
  # scripts/build-engine.sh with python.cpio's.
  rm -rf "$OUT/licenses"; mkdir -p "$OUT/licenses"
  cp "$S/LICENSE" "$OUT/licenses/typst-LICENSE.txt"
  cp "$S/NOTICE" "$OUT/licenses/typst-NOTICE.txt"
  cp "$F/nanum/OFL.txt" "$OUT/licenses/nanum-fonts-OFL.txt"
  cp "$P/licenses/"* "$OUT/licenses/"
  # The cpio goes again if this fails. RUSTC: the target spec is the guest toolchain's.
  trap 'rm -f "$OUT/typst.cpio"' ERR
  (cd "$S" && RUSTC="$R/bin/rustc" "${CARGO[@]}" metadata --format-version 1 --offline --filter-platform "$TARGET") \
    | python3 -c '
import json, sys
meta = json.load(sys.stdin)
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
packages = {p["id"]: p for p in meta["packages"]}
root = next(i for i, p in packages.items() if p["name"] == "typst-cli")
seen, stack = set(), [root]
while stack:
    i = stack.pop()
    if i in seen: continue
    seen.add(i)
    stack += [d["pkg"] for d in nodes[i]["deps"] if any(k["kind"] in (None, "normal") for k in d["dep_kinds"])]
for p in sorted((packages[i] for i in seen), key=lambda p: (p["name"], p["version"])):
    if p["source"]:  # the workspace crates are typst itself (LICENSE)
        print(p["name"], p["version"] + ":", p.get("license") or "see crate", p.get("repository") or "")
' > "$OUT/licenses/crates.txt"
  trap - ERR
fi
ls -l "$OUT/typst.cpio" | awk '{print "  " $NF, $5}'
