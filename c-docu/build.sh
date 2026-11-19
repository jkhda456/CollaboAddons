#!/usr/bin/env bash
# The c-docu add-on: `docu` (collabo-docu), office documents on the command line — read, edit in
# place and in part, render — as a wasm32 Linux program (Rust, Cargo.toml and src/ here, the Typst
# library for layout), packed as out/c-docu.cpio (an overlay):
#
#   /usr/bin/docu, /usr/bin/collabo-docu   the program
#   /usr/share/docu/fonts                  fonts for rendering: metric-compatible stand-ins for
#                                          Office's (Carlito, Caladea, Liberation), Nanum (Korean),
#                                          Droid Sans Fallback (Chinese characters, kana), DejaVu
#   /usr/share/doc/docu                    the agents' guide (AGENTS.md)
#   /etc/codex/skills/office-docs          the guide as a skill, for Codex (its system skills),
#   /root/.agent/skills/office-docs        agent (the agent add-on: the guest user's skills) and
#   /root/.claude/skills/office-docs       Claude Code (the guest user's skills)
#
# Stages (each skipped when its output is up to date):
#   fetch   what sources.lock names, into build/dl, checked by sha256
#   fonts   build/fonts from the downloads
#   src     the crate copied to build/src; crates whose wasm cfgs mean "a browser" narrowed to
#           wasm that is not Linux (build/patched, as the typst add-on does)
#   cross   docu for wasm32 (Linux, musl) with the guest's Rust toolchain
#   cpio    out/c-docu.cpio, out/c-docu.json, out/licenses
#
# Needs: ./build.sh userspace (musl sysroot, compiler-rt), the guest's Rust toolchain
# (python/build-rust-toolchain.sh, which this runs), network access for the first fetch and for
# crates not in the cargo cache.
#
#   FORCE=fetch,fonts,src,cross,cpio   re-run the named stages
#   JOBS=N                             cargo's -j (default: the number of CPUs)
#   TEST=1                             also build natively and run the unit tests
set -euo pipefail

A="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$A/../.." && pwd)"
U="$ROOT/userspace"
R="$ROOT/.tools/rust-wasm"
TARGET=wasm32-unknown-linux-musl
NIGHTLY=nightly-2026-07-22   # as python/build-rust-toolchain.sh
B="$A/build"
OUT="$A/out"
NAME=c-docu
FORCE=",${FORCE:-},"
# Crates whose `wasm` cfgs select browser code that the guest (wasm32 Linux) must not take.
NARROW_CRATES=(parking_lot_core time)
# Crates changed by hand: patches/crates/<crate>-<version>@<what>.patch (fontdb: read font files
# instead of mapping them, as the typst add-on does; the guest has no mmap).

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
fetch() {
  local name="$1" url sum file
  read -r _ url sum < <(grep -E "^$name " "$A/sources.lock") || { echo "fetch: $name is not in sources.lock" >&2; return 1; }
  file="$B/dl/$name-$(basename "$url")"
  mkdir -p "$B/dl"
  if [[ ! -f "$file" ]]; then
    echo "  fetch $(basename "$url")" >&2
    curl -fsSL --retry 3 -o "$file.part" "$url" && mv "$file.part" "$file"
  fi
  echo "$sum  $file" | sha256sum -c --quiet - >&2 || { echo "checksum mismatch: $file (deleted)" >&2; rm -f "$file"; return 1; }
  echo "$file"
}
newest() { find "$@" -type f -printf '%T@ %p\n' 2>/dev/null | sort -n | tail -1 | cut -d' ' -f2-; }

[[ -f "$U/sysroot/lib/crt1.o" ]] || { echo "missing userspace/sysroot; run: ./build.sh userspace" >&2; exit 1; }
bash "$ROOT/python/build-rust-toolchain.sh"
export RUSTUP_HOME="$ROOT/.tools/rustup" CARGO_HOME="$ROOT/.tools/cargo"
export PATH="$CARGO_HOME/bin:$PATH"
HOST_TRIPLE="$(rustc +"$NIGHTLY" -vV | sed -n 's/^host: //p')"
HOST_ENV="$(echo "$HOST_TRIPLE" | tr a-z- A-Z_)"
export "CARGO_TARGET_${HOST_ENV}_LINKER=clang-19" "CC_${HOST_TRIPLE//-/_}=clang-19" "CXX_${HOST_TRIPLE//-/_}=clang++-19"
export "AR_${HOST_TRIPLE//-/_}=llvm-ar-19"
mkdir -p "$B" "$OUT"

# ── fetch ────────────────────────────────────────────────────────────────────────────────────
if stage fetch "$B/dl/.stamp" "$A/sources.lock"; then
  while read -r name _; do
    [[ -z "$name" || "$name" == \#* ]] && continue
    fetch "$name" > /dev/null
  done < "$A/sources.lock"
  touch "$B/dl/.stamp"
fi
export DOCU_SHAPES="$(fetch preset-shapes)"

# ── fonts ────────────────────────────────────────────────────────────────────────────────────
F="$B/fonts"
if stage fonts "$F/.stamp" "$B/dl/.stamp"; then
  rm -rf "$F"; mkdir -p "$F"/{carlito,caladea,liberation,nanum,dejavu,droid}
  for v in regular bold italic bolditalic; do
    cp "$(fetch carlito-$v)" "$F/carlito/"
    cp "$(fetch caladea-$v)" "$F/caladea/"
  done
  cp "$(fetch carlito-ofl)" "$F/carlito/OFL.txt"
  tar -xzf "$(fetch liberation)" -C "$F/liberation" --strip-components=1 --wildcards '*.ttf' '*/LICENSE'
  for f in gothic-regular gothic-bold myeongjo-regular myeongjo-bold; do cp "$(fetch nanum-$f)" "$F/nanum/"; done
  cp "$(fetch nanum-ofl)" "$F/nanum/OFL.txt"
  cp "$(fetch droid-fallback)" "$F/droid/DroidSansFallbackFull.ttf"
  cp "$(fetch droid-notice)" "$F/droid/NOTICE"
  # The downloads' names carry their sources.lock names: back to the fonts' own.
  for f in "$F"/*/*.ttf; do
    n="$(dirname "$f")/$(basename "$f" | sed -E 's/^(carlito|caladea|nanum-(gothic|myeongjo))-(regular|bold|italic|bolditalic)-//')"
    [[ "$n" == "$f" ]] || mv "$f" "$n"
  done
  python3 - "$(fetch dejavu)" "$F/dejavu" <<'PY'
import sys, tarfile
keep = {"DejaVuSans.ttf", "DejaVuSans-Bold.ttf", "DejaVuSans-Oblique.ttf", "DejaVuSans-BoldOblique.ttf", "DejaVuSansMono.ttf", "DejaVuSansMono-Bold.ttf", "LICENSE"}
with tarfile.open(sys.argv[1]) as t:
    for m in t.getmembers():
        n = m.name.rsplit("/", 1)[-1]
        if n in keep and m.isfile():
            open(f"{sys.argv[2]}/{n}", "wb").write(t.extractfile(m).read())
PY
  touch "$F/.stamp"
fi

if [[ -n "${TEST:-}" ]]; then
  echo "== unit tests ($HOST_TRIPLE)"
  cargo +"$NIGHTLY" test --manifest-path "$A/Cargo.toml" --target-dir "$B/host" --locked
fi

# ── src ──────────────────────────────────────────────────────────────────────────────────────
S="$B/src"
P="$B/patched"
if stage src "$S/.stamp" "$A/Cargo.toml" "$A/Cargo.lock" "$A/build.rs" "$A/build.sh" "$(newest "$A/src")" "$(newest "$A/patches")"; then
  rm -rf "$S.new"; mkdir -p "$S.new"
  cp -r "$A/Cargo.toml" "$A/Cargo.lock" "$A/build.rs" "$A/src" "$S.new/"
  # The crates to narrow, at the versions the lock file has, from the cargo cache.
  rm -rf "$P"; mkdir -p "$P"
  REG="$(ls -d "$CARGO_HOME"/registry/src/*/ | head -1)"
  : > "$S.new/patches.toml"
  for crate in "${NARROW_CRATES[@]}"; do
    version="$(awk -v n="$crate" '$0 == "name = \"" n "\"" {getline; gsub(/version = |"/, ""); print; exit}' "$A/Cargo.lock")"
    [[ -n "$version" ]] || continue
    [[ -d "$REG/$crate-$version" ]] || (cd "$S.new" && cargo +"$NIGHTLY" fetch --quiet)
    cp -r "$REG/$crate-$version" "$P/"
    python3 - "$P/$crate-$version" <<'PY'
import os, re, sys
pats = [(re.compile(r'target_arch\s*=\s*"wasm32"'), 'all(target_arch = "wasm32", not(target_os = "linux"))'),
        (re.compile(r'target_family\s*=\s*"wasm"'), 'all(target_family = "wasm", not(target_os = "linux"))')]
for d, sub, files in os.walk(sys.argv[1]):
    for n in files:
        if not (n.endswith(".rs") or n == "Cargo.toml"):
            continue
        p = os.path.join(d, n)
        t = open(p, encoding="utf-8").read()
        lines = t.splitlines(keepends=True)
        for i, l in enumerate(lines):
            if not l.lstrip().startswith("//"):
                for pat, rep in pats:
                    l = pat.sub(rep, l)
            lines[i] = l
        new = "".join(lines)
        if new != t:
            open(p, "w", encoding="utf-8").write(new)
PY
    echo "patch.crates-io.$crate.path=\"$P/$crate-$version\"" >> "$S.new/patches.toml"
  done
  for patch in "$A/patches/crates"/*.patch; do
    [[ -e "$patch" ]] || continue
    crate="$(basename "$patch" .patch)"; crate="${crate%%@*}"
    name="${crate%-*}"
    [[ -d "$P/$crate" ]] || cp -r "$REG/$crate" "$P/"
    patch -p1 --quiet --no-backup-if-mismatch -d "$P/$crate" < "$patch" || { echo "FAILED: $(basename "$patch")" >&2; exit 1; }
    grep -q "^patch.crates-io.$name.path" "$S.new/patches.toml" || echo "patch.crates-io.$name.path=\"$P/$crate\"" >> "$S.new/patches.toml"
  done
  rm -rf "$S"; mv "$S.new" "$S"
  # The lock file may name a newer libc than the fork; resolve libc to the fork.
  LIBC_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$R/libc/Cargo.toml" | head -1)"
  cargo +"$NIGHTLY" update --manifest-path "$S/Cargo.toml" --config "patch.crates-io.libc.path=\"$R/libc\"" \
    -p libc --precise "$LIBC_VERSION" > "$B/update.log" 2>&1 || true
  touch "$S/.stamp"
fi

# ── cross ────────────────────────────────────────────────────────────────────────────────────
WASM="$B/docu.wasm"
if stage cross "$WASM" "$S/.stamp"; then
  cat > "$B/wasm-cc-rust" <<SH
#!/bin/sh
for a; do shift; case "\$a" in --target=*) ;; *) set -- "\$@" "\$a" ;; esac; done
exec "$U/bin/wasm-cc" "\$@"
SH
  chmod +x "$B/wasm-cc-rust"
  export "CC_${TARGET//-/_}=$B/wasm-cc-rust" "AR_${TARGET//-/_}=llvm-ar-19" "RANLIB_${TARGET//-/_}=llvm-ranlib-19"
  export WASM_MUSL_SYSROOT="$U/sysroot" WASM_LD="$R/bin/wasm-ld" RUSTC="$R/bin/rustc"
  export RUSTFLAGS="-Clinker=$B/wasm-cc-rust --remap-path-prefix=$S=/usr/src/docu --remap-path-prefix=$CARGO_HOME=/usr/src/cargo"
  export CARGO_PROFILE_RELEASE_PANIC=abort CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
         CARGO_PROFILE_RELEASE_DEBUG=false CARGO_PROFILE_RELEASE_STRIP=true
  CONFIG=(--config "patch.crates-io.libc.path=\"$R/libc\"")
  while read -r line; do [[ -n "$line" ]] && CONFIG+=(--config "$line"); done < "$S/patches.toml"
  echo "  cargo build (log: $B/cross.log)"
  cargo +"$NIGHTLY" build --release --target "$TARGET" --manifest-path "$S/Cargo.toml" --target-dir "$B/wasm" \
    -j "${JOBS:-$(nproc)}" "${CONFIG[@]}" > "$B/cross.log" 2>&1 || fail "$B/cross.log" "docu (cargo build)"
  built="$B/wasm/$TARGET/release/docu"
  [[ -f "$built" ]] || built="$built.wasm"
  [[ -f "$built" ]] || { echo "cargo wrote no docu binary" >&2; exit 1; }
  cp "$built" "$WASM"
fi

# ── cpio ─────────────────────────────────────────────────────────────────────────────────────
if stage cpio "$OUT/$NAME.cpio" "$WASM" "$F/.stamp" "$A/addon.json" "$(newest "$A/share")"; then
  python3 "$U/mkinitramfs.py" --overlay -o "$OUT/$NAME.cpio" --file usr/bin/docu="$WASM" \
    --link usr/bin/collabo-docu=docu \
    --tree usr/share/docu/fonts="$F" \
    --tree usr/share/doc/docu="$A/share/doc" \
    --tree etc/codex/skills/office-docs="$A/share/skill" \
    --tree root/.claude/skills/office-docs="$A/share/skill" \
    --tree root/.agent/skills/office-docs="$A/share/skill"
  cp "$A/addon.json" "$OUT/$NAME.json"
  rm -rf "$OUT/licenses"; mkdir -p "$OUT/licenses"
  cp "$A/../LICENSE" "$OUT/licenses/addon-$NAME-GPL-3.0.txt"
  cp "$F/carlito/OFL.txt" "$OUT/licenses/addon-$NAME-carlito-caladea-OFL.txt"
  cp "$F/liberation/LICENSE" "$OUT/licenses/addon-$NAME-liberation-OFL.txt"
  cp "$F/nanum/OFL.txt" "$OUT/licenses/addon-$NAME-nanum-OFL.txt"
  cp "$F/dejavu/LICENSE" "$OUT/licenses/addon-$NAME-dejavu-LICENSE.txt"
  cp "$F/droid/NOTICE" "$OUT/licenses/addon-$NAME-droid-fallback-Apache-2.0.txt"
  trap 'rm -f "$OUT/$NAME.cpio"' ERR
  RUSTC="$R/bin/rustc" cargo +"$NIGHTLY" metadata --format-version 1 --offline --manifest-path "$S/Cargo.toml" \
      --config "patch.crates-io.libc.path=\"$R/libc\"" --filter-platform "$TARGET" 2>/dev/null \
    | python3 -c '
import json, sys
meta = json.load(sys.stdin)
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
packages = {p["id"]: p for p in meta["packages"]}
root = next(i for i, p in packages.items() if p["name"] == "collabo-docu")
seen, stack = set(), [root]
while stack:
    i = stack.pop()
    if i in seen: continue
    seen.add(i)
    stack += [d["pkg"] for d in nodes[i]["deps"] if any(k["kind"] in (None, "normal") for k in d["dep_kinds"])]
for p in sorted((packages[i] for i in seen), key=lambda p: (p["name"], p["version"])):
    if p["name"] != "collabo-docu":
        print(p["name"], p["version"] + ":", p.get("license") or "see crate", p.get("repository") or "")
' > "$OUT/licenses/addon-$NAME-crates.txt"
  trap - ERR
fi
ls -l "$OUT/$NAME.cpio" | awk '{print "  " $NF, $5}'
