#!/usr/bin/env bash
# The rust add-on: the Rust toolchain itself in the guest — rustc 1.99.1 and cargo as wasm32 Linux
# (musl) programs, with the standard library for the guest's own target (wasm32-unknown-linux-musl,
# built into this rustc), rust-lld to link with, and dlopen-loaded proc macros; rustdoc, clippy and
# rustfmt — packed as out/rust.cpio (an overlay: /usr/bin/{rustc,rustdoc,rustfmt,clippy-driver,
# cargo,cargo-clippy,cargo-fmt}, /usr/lib/rustlib, /.cargo/config.toml). README.md has the details.
#
# Sources: Rust 1.99.1 — RUST_SRC is the GitHub archive of its tag; the build takes the tree from
# the official rustc-1.99.1-src.tar.xz (checksum-pinned, sources.lock), which holds the same files
# and what the archive leaves out: the submodules (src/llvm-project, src/tools/cargo,
# library/backtrace) and the commit the release was built from (git-commit-info).
# `./build.sh verify-src` compares the two. The host compiler is the official Rust 1.99.1 for
# this machine (pinned the same way): it builds the guest's std and cross-compiles rustc and
# cargo, so the guest's rustc is the same release, identifies itself the same way and reads the
# same metadata.
#
# Stages (each skipped when its output is up to date):
#   dl        download and check the pinned tarballs ($W/dl)
#   host      the official Rust 1.99.1 for this machine (build/host-rust)
#   src       the rust tree with patches/rust/*.patch (build/src/rust) and LLVM (llvm, lld, cmake,
#             third-party, libc's shared headers) with patches/llvm/*.patch (build/src/llvm-project)
#   crates    crates.io packages with patches/crates/*.patch (build/crates)
#   std       the standard library for wasm32-unknown-linux-musl (build/sysroot)
#   tblgen    LLVM's tablegens and llvm-config for this machine (build/llvm-native)
#   llvm      LLVM (WebAssembly) and lld for the guest, cross-compiled ($W/llvm-cross; build/llvm)
#   rustc     rustc for the guest (build/work/target-rustc)
#   tools     rustc again as rustdoc, clippy-driver and rustfmt too (tools/multicall), cargo-clippy,
#             cargo-fmt
#   cargo     cargo for the guest (build/work/target-cargo)
#   cpio      pack out/rust.cpio and the licenses
#
# Needs: ./build.sh userspace (musl sysroot with dlopen, compiler-rt) and python (python/deps: the
# guest's OpenSSL, for cargo), the llvm add-on built (addons/llvm/build/native: its clang compiles
# LLVM's C++ for the guest; build/runtimes-install: libc++), clang-19/lld-19 on the host (the
# tablegens), and network for the first run (downloads, crates). tools/go runs this from a copy.
#
# Room: $W (RUST_ADDON_WORK, default: build/collabo inside RUST_SRC, a folder of its own) holds what
# is only written and read through, never mapped: the downloads, cargo's home (crates' sources) and
# the cross-compiled LLVM's build tree, whose products are copied to build/llvm. It may be a shared
# folder (9p), whose clock may differ from this machine's: the stages' stamps stay in build/, cargo
# decides what is fresh by checksums (-Zchecksum-freshness). The target directories (cargo maps
# rlibs, a linker its inputs) are local: build/work (RUST_ADDON_LOCAL), ~3 GB.
#
#   FORCE=dl,host,src,crates,std,tblgen,llvm,rustc,tools,cargo,cpio   re-run the named stages
#   JOBS=N                                               ninja's and cargo's -j
set -euo pipefail

A="${RUST_ADDON_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
ROOT="$(cd "$A/../.." && pwd)"
U="$ROOT/userspace"
TOOLS="$ROOT/.tools"
B="$A/build"
OUT="$A/out"
RUST_SRC="${RUST_SRC:-/mnt/utm/workspace/rust/rust-1.99.1}"
W="${RUST_ADDON_WORK:-$RUST_SRC/build/collabo}"
WL="${RUST_ADDON_LOCAL:-$A/build/work}"     # what must not be read from a shared folder
RT="$WL/target-rustc" TT="$WL/target-tools" CT="$WL/target-cargo"   # cargo's target directories
JOBS="${JOBS:-$(nproc)}"
FORCE=",${FORCE:-},"

RUST_VERSION=1.99.1
HOST_TRIPLE="$(uname -m)-unknown-linux-gnu"
TARGET=wasm32-unknown-linux-musl
LLVM_ADDON="$A/../llvm/build"
WASM_BIN="$LLVM_ADDON/native/bin"        # clang/lld/llvm-ar of the llvm add-on (exnref)
RTI="$LLVM_ADDON/runtimes-install"       # libc++, libc++abi, libunwind for the guest
SYSROOT="$U/sysroot"
RT_LIB="$U/out/rt/lib/wasm32-unknown-unknown/libclang_rt.builtins.a"

# How the guest's C and C++ are compiled (userspace/bin/wasm-cc's flags: atomics + bulk memory,
# setjmp/longjmp as exceptions, the Linux macros), by the llvm add-on's clang, in the standard
# exception encoding (exnref), which the engine runs.
WASM_CFLAGS="--target=wasm32-unknown-unknown -matomics -mbulk-memory -mllvm -wasm-enable-sjlj -mllvm -wasm-use-legacy-eh=false -D__linux__=1 -D__linux=1 -D__unix__=1 -D__unix=1 -D__gnu_linux__=1 -Wno-unused-command-line-argument -Wno-override-module --sysroot=$SYSROOT"

CMAKE="$TOOLS/cmake/bin/cmake"
NINJA="$(dirname "$CMAKE")/ninja"
cmake() { "$CMAKE" -DCMAKE_MAKE_PROGRAM="$NINJA" "$@"; }

# ── helpers ───────────────────────────────────────────────────────────────────────────────────
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
# record FILE TEXT: FILE holds TEXT, rewritten only when it changes (a stage's input for what is
# no file: its flags)
record() {
  mkdir -p "$(dirname "$1")"
  [[ -f "$1" && "$(cat "$1")" == "$2" ]] || printf '%s\n' "$2" > "$1"
  echo "$1"
}
check() { [[ -e "$1" ]] || { echo "missing: $1" >&2; echo "  $2" >&2; exit 1; }; }

check "$SYSROOT/lib/libc.a"       "run: ./build.sh userspace"
check "$SYSROOT/lib/libc.exports" "run: ./build.sh userspace"
check "$RT_LIB"                   "run: ./build.sh userspace"
check "$WASM_BIN/clang"           "build the llvm add-on first: addons/build.sh llvm"
check "$RTI/lib/libc++.a"         "build the llvm add-on first: addons/build.sh llvm"
check "$CMAKE"                    "run: ./build.sh tools"
for t in clang-19 clang++-19 lld-19; do command -v $t >/dev/null || { echo "missing: $t" >&2; exit 1; }; done

mkdir -p "$B" "$OUT" "$W" "$WL"

# ── verify-src: RUST_SRC against the official tarball's tree (what they have in common) ─────
if [[ "${1:-}" == verify-src ]]; then
  [[ -f "$RUST_SRC/src/version" && "$(cat "$RUST_SRC/src/version")" == "$RUST_VERSION" ]] \
    || { echo "RUST_SRC ($RUST_SRC) is not rust $RUST_VERSION" >&2; exit 1; }
  V="$(mktemp -d "$B/verify.XXXXXX")"; trap 'rm -rf "$V"' EXIT
  tar -xJf "$W/dl/rustc-$RUST_VERSION-src.tar.xz" -C "$V" --strip-components=1 \
    "rustc-$RUST_VERSION-src/compiler" "rustc-$RUST_VERSION-src/library" "rustc-$RUST_VERSION-src/src/version" \
    "rustc-$RUST_VERSION-src/Cargo.lock" "rustc-$RUST_VERSION-src/Cargo.toml"
  rc=0
  for p in compiler library src/version Cargo.lock Cargo.toml; do
    # the submodule (library/backtrace) is empty in the archive
    # (and the tarball's own additions: std's dependencies vendored, library/vendor and .cargo)
    diff -rq -x backtrace -x '.*' -x vendor "$V/$p" "$RUST_SRC/$p" || rc=1
  done
  [[ $rc == 0 ]] && echo "RUST_SRC and rustc-$RUST_VERSION-src.tar.xz agree (compiler, library, Cargo.toml/lock)"
  exit $rc
fi

# ── dl: the pinned tarballs ───────────────────────────────────────────────────────────────────
DL="$W/dl"
# shellcheck disable=SC1091
source "$A/sources.lock"
if stage dl "$B/dl.stamp" "$A/sources.lock"; then
  mkdir -p "$DL"
  while read -r name sha url; do
    [[ -n "$name" ]] || continue
    if [[ ! -f "$DL/$name" ]] || ! echo "$sha  $DL/$name" | sha256sum -c --quiet - 2>/dev/null; then
      echo "  fetch $name"
      curl -fSL --retry 3 -o "$DL/$name.part" "$url" && mv "$DL/$name.part" "$DL/$name"
    fi
    echo "$sha  $DL/$name" | sha256sum -c --quiet - || { echo "checksum mismatch: $name" >&2; exit 1; }
  done <<< "$DOWNLOADS"
  touch "$B/dl.stamp"
fi

# ── host: the official Rust for this machine ──────────────────────────────────────────────────
HR="$B/host-rust"
if stage host "$HR/bin/rustc" "$B/dl.stamp"; then
  rm -rf "$HR" "$B/host-rust.x"; mkdir -p "$B/host-rust.x"
  tar -xJf "$DL/rust-$RUST_VERSION-$HOST_TRIPLE.tar.xz" -C "$B/host-rust.x"
  "$B/host-rust.x/rust-$RUST_VERSION-$HOST_TRIPLE/install.sh" --prefix="$HR" --disable-ldconfig \
    --components="rustc,cargo,rust-std-$HOST_TRIPLE" > /dev/null
  rm -rf "$B/host-rust.x"
fi
"$HR/bin/rustc" -V | grep -q "^rustc $RUST_VERSION " || { echo "host rustc is not $RUST_VERSION" >&2; exit 1; }

# ── src: the rust tree and LLVM, patched ──────────────────────────────────────────────────────
# The rust tree is made afresh in src/rust.new and synced into src/rust by content, so a file that
# came out the same keeps its time. Left out: tests, docs and the tools this add-on does not build.
RS="$B/src/rust"
LS="$B/src/llvm-project"
TARBALL="$DL/rustc-$RUST_VERSION-src.tar.xz"
TOP="rustc-$RUST_VERSION-src"
RUST_PATHS="compiler library src/version src/stage0 src/tools/cargo src/tools/rustfmt src/tools/clippy
src/tools/rust-installer src/librustdoc src/rustdoc-json-types src/tools/lld-wrapper src/build_helper
src/rustc-std-workspace Cargo.toml Cargo.lock COPYRIGHT LICENSE-APACHE LICENSE-MIT LICENSES
license-metadata.json git-commit-info git-commit-hash version"
RUST_EXCLUDE="src/tools/rustfmt/tests library/backtrace/tests"
LLVM_PATHS="llvm cmake third-party lld libunwind libc"
LLVM_EXCLUDE="llvm/test llvm/unittests llvm/docs llvm/benchmarks lld/test lld/unittests libc/test
libc/docs libc/benchmarks libc/fuzzing libc/utils/gpu"
SRC_REC="$(record "$B/src.paths" "$RUST_PATHS | $RUST_EXCLUDE | $LLVM_PATHS | $LLVM_EXCLUDE")"
if stage src "$B/src.stamp" "$B/dl.stamp" "$SRC_REC" "$A"/patches/rust/*.patch "$A"/patches/llvm/*.patch \
     $(find "$A/tools/multicall" -type f); then
  # LLVM
  rm -rf "$LS"; mkdir -p "$B/src"
  excl=(); for p in $LLVM_EXCLUDE; do excl+=("--exclude=$TOP/src/llvm-project/$p"); done
  inc=(); for p in $LLVM_PATHS; do inc+=("$TOP/src/llvm-project/$p"); done
  tar -xJf "$TARBALL" -C "$B/src" --strip-components=2 "${excl[@]}" "${inc[@]}"
  for p in "$A"/patches/llvm/*.patch; do
    [[ -e "$p" ]] || continue
    echo "  patch llvm: $(basename "$p")"; patch -d "$LS" -p1 --no-backup-if-mismatch -s < "$p"
  done
  # the rust tree
  rm -rf "$RS.new"; mkdir -p "$RS.new"
  excl=(); for p in $RUST_EXCLUDE; do excl+=("--exclude=$TOP/$p"); done
  inc=(); for p in $RUST_PATHS; do inc+=("$TOP/$p"); done
  tar -xJf "$TARBALL" -C "$RS.new" --strip-components=1 "${excl[@]}" "${inc[@]}"
  for p in "$A"/patches/rust/*.patch; do
    [[ -e "$p" ]] || continue
    echo "  patch rust: $(basename "$p")"; patch -d "$RS.new" -p1 --no-backup-if-mismatch -s < "$p"
  done
  # the add-on's multicall rustc (tools/multicall), a member of the workspace
  mkdir -p "$RS.new/src/tools/collabo-multicall"
  cp -r "$A/tools/multicall/." "$RS.new/src/tools/collabo-multicall/"
  sed -i 's|^  "compiler/rustc",$|  "compiler/rustc",\n  "src/tools/collabo-multicall",|' "$RS.new/Cargo.toml"
  # the workspace without the members left out
  python3 - "$RS.new" <<'PY'
import os, re, sys
root = sys.argv[1]; p = os.path.join(root, "Cargo.toml"); s = open(p).read()
def keep(m):
    d = m.group(1)
    return m.group(0) if os.path.isdir(os.path.join(root, d)) else ""
s = re.sub(r'^  "([^"]+)",\n', keep, s, flags=re.M)
open(p, "w").write(s)
PY
  # the lock files as they came (pin_crates changes the tree's), for the crates' checksums
  mkdir -p "$B/locks"
  cp "$RS.new/Cargo.lock" "$B/locks/rust.lock"; cp "$RS.new/library/Cargo.lock" "$B/locks/library.lock"
  cp "$RS.new/src/tools/cargo/Cargo.lock" "$B/locks/cargo.lock"
  mkdir -p "$RS"
  rsync -rlc --delete "$RS.new/" "$RS/"
  rm -rf "$RS.new"
  touch "$B/src.stamp"
fi

# ── the host compiler, told about the guest's target ──────────────────────────────────────────
# target/wasm32-unknown-linux-musl.json is the target patches/rust/0005 builds into the guest's
# rustc (`rustc --print target-spec-json` gives the same in both). Found by name through
# RUST_TARGET_PATH, the target is recorded in what is built for it by its name alone, as a
# built-in one: the guest's rustc takes the std built here as its own.
SYSR="$B/sysroot"                       # std for the guest + a link to this machine's own
TLIB="$SYSR/lib/rustlib/$TARGET/lib"
mkdir -p "$B/bin"
cat > "$B/bin/rustc" <<EOF
#!/bin/sh
# the official rustc $RUST_VERSION, with the guest's target and (once built) its std
export RUST_TARGET_PATH="$A/target" RUSTC_BOOTSTRAP=1
[ -n "\${RUST_ADDON_NO_SYSROOT:-}" ] || set -- --sysroot="$SYSR" "\$@"
exec "$HR/bin/rustc" -Zunstable-options "\$@"
EOF
chmod +x "$B/bin/rustc"
export CARGO_HOME="$W/cargo-home" RUSTC_BOOTSTRAP=1
export PATH="$HR/bin:$PATH"
# Build scripts and proc macros are this machine's programs; its C compiler is clang-19 (no `cc`).
HOST_ENV="$(echo "$HOST_TRIPLE" | tr a-z- A-Z_)"
export "CARGO_TARGET_${HOST_ENV}_LINKER=clang-19" "CC_${HOST_TRIPLE//-/_}=clang-19" \
       "CXX_${HOST_TRIPLE//-/_}=clang++-19" "AR_${HOST_TRIPLE//-/_}=llvm-ar-19"
# C for the guest (cc crate): the llvm add-on's clang, as wasm-cc compiles
GUEST_CC="$B/bin/guest-cc"
cat > "$GUEST_CC" <<EOF
#!/bin/sh
# C and C++ for the guest (the cc crate's compiler for $TARGET), as userspace/bin/wasm-cc
# compiles, position-independent as the target's Rust code; compiling only (rust-lld links).
for a; do case "\$a" in --target=*) ;; *) set -- "\$@" "\$a" ;; esac; shift; done
case "\$(basename "\$0")" in
  *++) exec "$WASM_BIN/clang++" $WASM_CFLAGS -fPIC -nostdinc++ -isystem "$RTI/include/c++/v1" "\$@" ;;
esac
exec "$WASM_BIN/clang" $WASM_CFLAGS -fPIC "\$@"
EOF
chmod +x "$GUEST_CC"; ln -sf guest-cc "$B/bin/guest-c++"

# ── crates: crates.io packages the guest needs changed (patches/crates/<name>-<version>@*.patch) ─
# libc: tombl's fork's ILP32 musl ABI for wasm32 (the guest's toolchain uses the fork at
# python/build-rust-toolchain.sh's LIBC_REV; the same change, on the version std locks). rustix and
# linux-raw-sys: wasm32 Linux as the asm-generic 32-bit ABI (riscv32's), from the codex add-on.
# object: a wasm32 Linux program's native format is wasm. @wasm-cfg: crates that take wasm32 for a
# browser narrowed to wasm that is not Linux (by the codex add-on's patches/narrow-wasm-cfg.py;
# some from codex's and typst's patches).
# Each takes the place of its crates.io package (cargo --config patch) in the standard library,
# rustc and cargo, which all take these versions (cargo update --precise).
CR="$B/crates"
PATCHED_CRATES="$(ls "$A"/patches/crates/ | sed -n 's/^\(.*-[0-9][0-9A-Za-z.+]*\)@.*\.patch$/\1/p' | sort -u)"
if stage crates "$CR/.stamp" "$B/dl.stamp" "$A"/patches/crates/*.patch; then
  rm -rf "$CR"; mkdir -p "$CR"
  for nv in $PATCHED_CRATES; do
    # fetched from crates.io, checked against the checksum the lock files give it
    n="${nv%-*}" v="${nv##*-}"
    sum="$(cat "$B"/locks/*.lock \
      | awk -v n="$n" -v v="$v" '/^\[\[package\]\]/ { m = 0 } $0 == "name = \""n"\"" { nm = 1; next }
             nm && $0 == "version = \""v"\"" { m = 1 } { nm = 0 }
             m && /^checksum = / { split($0, x, "\""); print x[2]; exit }')"
    [[ -n "$sum" ]] || sum="$(echo "$GUEST_CRATE_SUMS" | awk -v nv="$nv" '$1 == nv { print $2 }')"
    [[ -n "$sum" ]] || { echo "no checksum for $nv (lock files, sources.lock)" >&2; exit 1; }
    if [[ ! -f "$DL/$nv.crate" ]] || ! echo "$sum  $DL/$nv.crate" | sha256sum -c --quiet - >/dev/null 2>&1; then
      curl -fsSL --retry 3 -o "$DL/$nv.crate" "https://static.crates.io/crates/$n/$nv.crate"
      echo "$sum  $DL/$nv.crate" | sha256sum -c --quiet - || { echo "checksum mismatch: $nv" >&2; exit 1; }
    fi
    tar -xzf "$DL/$nv.crate" -C "$CR"
    for p in "$A"/patches/crates/"$nv"@*.patch; do
      patch -d "$CR/$nv" -p1 --no-backup-if-mismatch -s < "$p"
    done
  done
  touch "$CR/.stamp"
fi
PATCH_CRATES=()
for nv in $PATCHED_CRATES; do
  # one entry for each version (a crate may be there in two: object), named after both
  k="$(echo "$nv" | tr '.+-' '___')"
  PATCH_CRATES+=(--config "patch.crates-io.$k.path='$CR/$nv'" --config "patch.crates-io.$k.package='${nv%-*}'")
done
LIBC_VERSION=0.2.189
# pin_crates: the workspace's Cargo.lock (in the current directory) at the patched versions (two
# rounds: one may need another first, as linux-raw-sys's version follows rustix's)
pin_crates() {
  local nv n v old round
  for round in 1 2; do
    for nv in $PATCHED_CRATES; do
      n="${nv%-*}" v="${nv##*-}"
      grep -q "^name = \"$n\"$" Cargo.lock || continue
      grep -A1 "^name = \"$n\"$" Cargo.lock | grep -q "^version = \"$v\"$" && continue
      # a version of it the patched one can stand for (0.x.*: the same x; else the same major); of
      # more than one, the newest
      old="$(grep -A1 "^name = \"$n\"$" Cargo.lock | sed -n 's/^version = "\(.*\)"/\1/p' \
        | awk -v v="$v" -F. 'BEGIN { split(v, p, ".") } ($1 == p[1] && ($1 != "0" || $2 == p[2]))' | sort -V | tail -1)"
      [[ -n "$old" ]] || continue
      cargo update -p "$n@$old" --precise "$v" 2>/dev/null || [[ $round == 1 ]] \
        || echo "note: $n stays at $old here (its patched $v does not fit this workspace)"
    done
  done
}

# ── std: the standard library for the guest ───────────────────────────────────────────────────
# -Zbuild-std of the patched library (std, proc_macro for the guest's proc macros, test for its
# `cargo test`), as bootstrap builds it: unwinding panics, -Zforce-unstable-if-unmarked, bitcode
# in the rlibs (LTO), paths as /rustc/<commit> (the official std's). Position-independent (the
# target's default), so the same rlibs go into programs and into side modules (proc macros).
# Beside it, in self-contained/: musl's crt1.o and libc.a, compiler-rt (and its copy for side
# modules, without __c_longjmp) and libunwind.a (rt/unwind-wasm.c).
STD_B="$WL/std-build"
COMMIT="$(cat "$RS/git-commit-hash")"
STD_FLAGS="-Cembed-bitcode=yes -Zforce-unstable-if-unmarked --remap-path-prefix=$RS=/rustc/$COMMIT --remap-path-prefix=$CARGO_HOME=/rust/deps"
STD_REC="$(record "$B/std.flags" "$STD_FLAGS")"
if stage std "$TLIB/.stamp" "$B/src.stamp" "$A/patches/crates/libc-$LIBC_VERSION@wasm32.patch" "$STD_REC" "$A"/rt/*.c "$SYSROOT/lib/libc.a" "$RT_LIB"; then
  # afresh (half a minute): cargo does not always see that the library's sources changed
  rm -rf "$STD_B/target"
  mkdir -p "$STD_B/src" "$STD_B/.cargo"
  touch "$STD_B/src/lib.rs"
  printf '[package]\nname = "sysroot-build"\nversion = "0.0.0"\nedition = "2024"\n' > "$STD_B/Cargo.toml"
  printf '[unstable]\nbuild-std = ["std", "panic_unwind", "panic_abort", "proc_macro", "test"]\nbuild-std-features = ["panic-unwind"]\n' \
    > "$STD_B/.cargo/config.toml"
  # the library's own Cargo.toml takes the patched libc
  grep -q "^libc = { path" "$RS/library/Cargo.toml" || \
    sed -i "s|^\[patch.crates-io\]$|[patch.crates-io]\nlibc = { path = '$CR/libc-$LIBC_VERSION' }|" "$RS/library/Cargo.toml"
  (
    cd "$STD_B"
    export RUST_ADDON_NO_SYSROOT=1 RUSTC="$B/bin/rustc" __CARGO_TESTS_ONLY_SRC_ROOT="$RS/library"
    export RUSTFLAGS="$STD_FLAGS"
    cargo build --release --target "$TARGET" -j"$JOBS"
  ) > "$B/std.log" 2>&1 || fail "$B/std.log" "std for $TARGET"

  rm -rf "$SYSR"; mkdir -p "$TLIB/self-contained"
  ln -s "$HR/lib/rustlib/$HOST_TRIPLE" "$SYSR/lib/rustlib/$HOST_TRIPLE"
  for f in "$STD_B/target/$TARGET/release/deps/"*.rlib; do
    case "$(basename "$f")" in libsysroot_build-*) ;; *) cp "$f" "$TLIB/" ;; esac
  done
  SC="$TLIB/self-contained"
  cp "$SYSROOT/lib/crt1.o" "$SYSROOT/lib/libc.a" "$SC/"
  # with the memory-mapping calls the C library lacks (copies: rt/mman-wasm.c)
  "$WASM_BIN/clang" $WASM_CFLAGS -O2 -c "$A/rt/mman-wasm.c" -o "$STD_B/mman-wasm.o"
  "$WASM_BIN/llvm-ar" rs "$SC/libc.a" "$STD_B/mman-wasm.o"
  cp "$RT_LIB" "$SC/libclang_rt.builtins.a"
  cp "$RT_LIB" "$SC/libclang_rt.builtins-shared.a"
  "$WASM_BIN/llvm-ar" d "$SC/libclang_rt.builtins-shared.a" __c_longjmp.S.o
  # the C library's other names (empty in musl: all of it is in libc.a), and libatomic, which
  # crates ask for on 32-bit targets (WebAssembly's atomics are instructions)
  for l in m pthread dl rt util crypt resolv xnet; do cp "$SYSROOT/lib/lib$l.a" "$SC/" 2>/dev/null || true; done
  "$WASM_BIN/llvm-ar" rc "$SC/libatomic.a"
  "$WASM_BIN/clang" $WASM_CFLAGS -fPIC -fwasm-exceptions -O2 -c "$A/rt/unwind-wasm.c" -o "$STD_B/unwind-wasm.o"
  rm -f "$TLIB/libunwind.a"; "$WASM_BIN/llvm-ar" rcs "$TLIB/libunwind.a" "$STD_B/unwind-wasm.o"
  # what was built against another std goes (cargo cannot tell a std of the same rustc apart)
  rm -rf "$RT/$TARGET" "$TT/$TARGET" "$CT/$TARGET"
  touch "$TLIB/.stamp"
fi

# ── tblgen: LLVM's tablegens and llvm-config for this machine ─────────────────────────────────
# With the cross build's options (targets, features), so this llvm-config's answers (components,
# library names, flags) are the cross build's too, but for the paths (llvm-config, below).
LN="$B/llvm-native"
LLVM_TARGETS=WebAssembly
LLVM_COMMON=(
  -DCMAKE_BUILD_TYPE=Release
  -DLLVM_TARGETS_TO_BUILD="$LLVM_TARGETS"
  -DLLVM_ENABLE_ASSERTIONS=OFF
  -DLLVM_UNREACHABLE_OPTIMIZE=OFF
  -DLLVM_ENABLE_PLUGINS=OFF
  -DLLVM_INCLUDE_EXAMPLES=OFF -DLLVM_INCLUDE_DOCS=OFF -DLLVM_INCLUDE_BENCHMARKS=OFF -DLLVM_INCLUDE_TESTS=OFF
  -DLLVM_ENABLE_LIBEDIT=OFF -DLLVM_ENABLE_BINDINGS=OFF -DLLVM_ENABLE_Z3_SOLVER=OFF
  -DLLVM_ENABLE_ZLIB=OFF -DLLVM_ENABLE_ZSTD=OFF -DLLVM_ENABLE_LIBXML2=OFF -DLLVM_ENABLE_CURL=OFF
  -DLLVM_ENABLE_HTTPLIB=OFF -DLLVM_ENABLE_TERMINFO=OFF -DLLVM_ENABLE_WARNINGS=OFF
  -DLLVM_BUILD_LLVM_DYLIB=OFF -DLLVM_LINK_LLVM_DYLIB=OFF -DBUILD_SHARED_LIBS=OFF
  -DLLVM_ENABLE_PROJECTS=lld
  -DLLVM_FORCE_VC_REPOSITORY=https://github.com/rust-lang/llvm-project.git
  -DLLVM_FORCE_VC_REVISION="$LLVM_REV"
)
if stage tblgen "$LN/.stamp" "$B/src.stamp"; then
  [[ -f "$LN/build.ninja" ]] || cmake -S "$LS/llvm" -B "$LN" -G Ninja --log-level=WARNING "${LLVM_COMMON[@]}" \
    -DCMAKE_C_COMPILER=clang-19 -DCMAKE_CXX_COMPILER=clang++-19 -DLLVM_USE_LINKER=lld-19 \
    -DLLVM_DEFAULT_TARGET_TRIPLE="$TARGET" \
    > "$B/llvm-native-configure.log" 2>&1 || fail "$B/llvm-native-configure.log" "configure native LLVM"
  "$NINJA" -C "$LN" -j"$JOBS" llvm-tblgen llvm-min-tblgen llvm-config > "$B/llvm-native.log" 2>&1 \
    || fail "$B/llvm-native.log" "native tablegens"
  touch "$LN/.stamp"
fi

# ── llvm: LLVM and lld for the guest ──────────────────────────────────────────────────────────
# Compiled by the llvm add-on's clang (LLVM trunk, for this machine) as userspace/bin/wasm-cc
# compiles (atomics, bulk memory, the Linux macros; exceptions in the standard exnref encoding),
# against the add-on's libc++. Static libraries only, no exceptions, no RTTI (as rustc's LLVM).
LC="$W/llvm-cross"
LO="$B/llvm"                                   # its products, local (below)
LLD_OBJ="tools/lld/tools/lld/CMakeFiles/lld.dir"
LIBCXX_FLAGS="-nostdinc++ -isystem $RTI/include/c++/v1"
WASM_LINK_FLAGS="--target=wasm32-unknown-unknown -nostdlib -fuse-ld=$WASM_BIN/wasm-ld -Wl,--table-base=3 -Wl,--import-memory -Wl,--max-memory=4294967296 -Wl,--shared-memory -Wl,--export-table -Wl,-z,stack-size=8388608 -L$SYSROOT/lib $SYSROOT/lib/crt1.o"
LLVM_CROSS=(
  "${LLVM_COMMON[@]}"
  -DCMAKE_TOOLCHAIN_FILE="$A/../llvm/cmake/wasm32-toolchain.cmake"
  -DWASM_SYSROOT="$SYSROOT" -DWASM_BIN="$WASM_BIN"
  "-DCMAKE_C_FLAGS=$WASM_CFLAGS"
  "-DCMAKE_CXX_FLAGS=$WASM_CFLAGS -fno-exceptions $LIBCXX_FLAGS"
  "-DCMAKE_EXE_LINKER_FLAGS=$WASM_LINK_FLAGS"
  "-DCMAKE_CXX_STANDARD_LIBRARIES=-L$RTI/lib -lc++ -lc++abi -lunwind -lc $RT_LIB"
  "-DCMAKE_C_STANDARD_LIBRARIES=-L$RTI/lib -lc++ -lc++abi -lunwind -lc $RT_LIB"
  -DLLVM_DEFAULT_TARGET_TRIPLE="$TARGET"
  -DLLVM_HOST_TRIPLE="$TARGET"
  -DLLVM_TARGET_ARCH=wasm32
  -DLLVM_TABLEGEN="$LN/bin/llvm-tblgen"
  -DLLVM_NATIVE_TOOL_DIR="$LN/bin"
  -DLLVM_CONFIG_PATH="$LN/bin/llvm-config"
  -DLLVM_ENABLE_PIC=OFF
  -DLLVM_ENABLE_THREADS=ON
  -DLLVM_PARALLEL_LINK_JOBS=1
  -DLLVM_BUILD_TOOLS=OFF
  -DLLVM_BUILD_UTILS=OFF
  -DLLVM_INSTALL_UTILS=OFF
  -DLLVM_OPTIMIZED_TABLEGEN=ON
  -DCMAKE_SKIP_RPATH=ON
)
LLVM_REC="$(record "$B/llvm-cross.config" "${LLVM_CROSS[*]}")"
# rustc_llvm/build.rs: REQUIRED_COMPONENTS and the targets of OPTIONAL_COMPONENTS this LLVM has
RUSTC_LLVM_COMPONENTS="ipo bitreader bitwriter linker asmparser lto coverage instrumentation webassembly"
if stage llvm "$B/llvm.stamp" "$B/src.stamp" "$LN/.stamp" "$LLVM_REC" "$RTI/lib/libc++.a"; then
  if [[ ! -f "$LC/build.ninja" || "$LLVM_REC" -nt "$LC/build.ninja" ]]; then
    cmake -S "$LS/llvm" -B "$LC" -G Ninja --log-level=WARNING "${LLVM_CROSS[@]}" \
      > "$B/llvm-cross-configure.log" 2>&1 || fail "$B/llvm-cross-configure.log" "configure LLVM for the guest"
    touch "$LC/build.ninja"
  fi
  # The native llvm-config answers for the cross tree (bin/llvm-config above), but checks that its
  # own tree has the libraries it names: empty stand-ins there, one for each library of the cross
  # build (the native build made only the tablegens' few, which stay).
  for l in $(grep -oE '^build lib/libLLVM[A-Za-z0-9]+\.a' "$LC/build.ninja" | sed 's|^build lib/||'); do
    [[ -e "$LN/lib/$l" ]] || : > "$LN/lib/$l"
  done
  # the libraries rustc_llvm links (llvm-config --libs of its components: rustc_llvm/build.rs) and
  # lld (rust-lld); not all of LLVM: its JIT, for one, wants memory mappings the guest has not
  libs="$("$LN/bin/llvm-config" --link-static --libnames $RUSTC_LLVM_COMPONENTS | sed -e 's/lib\(LLVM[A-Za-z0-9]*\)\.a/\1/g')"
  "$NINJA" -C "$LC" -j"$JOBS" ${NINJA_FLAGS:-} $libs lld > "$B/llvm-cross.log" 2>&1 \
    || fail "$B/llvm-cross.log" "LLVM for the guest"
  # What the next stages read of it, copied off $W first: a linker maps its inputs, and on a shared
  # folder (9p) a mapped file has come back with pages of other files in it (a rustc whose code held
  # LLVM's intrinsic names; the archives themselves, made and read without mappings, were right).
  # The libraries and the generated headers, and lld linked again from local copies.
  rm -rf "$LO"; mkdir -p "$LO/lib" "$LO/bin" "$LO/$LLD_OBJ"
  cp "$LC"/lib/*.a "$LO/lib/"
  cp -r "$LC/include" "$LO/"
  cp "$LC/$LLD_OBJ"/*.o "$LO/$LLD_OBJ/"
  link="$("$NINJA" -C "$LC" -t commands bin/lld | tail -1)"
  (cd "$LO" && eval "$link") > "$B/lld-link.log" 2>&1 || fail "$B/lld-link.log" "linking lld"
  touch "$B/llvm.stamp"
fi

# ── rustc: the compiler for the guest ─────────────────────────────────────────────────────────
# compiler/rustc (rustc-main) for wasm32-unknown-linux-musl, as bootstrap builds it (its CFG_*
# variables, features llvm and max_level_info), with rustc_driver an rlib (patches/rust/0008: no
# dylibs a program links at start here), linked by the llvm add-on's wasm-ld (newer than this
# LLVM, it reads the C++ objects of its clang). A program that loads proc macros (side modules,
# dlopen): the whole C library in it and exported, its stack pointer exported, its function table
# growable.
cat > "$B/bin/llvm-config" <<EOF
#!/bin/sh
# llvm-config of the cross-compiled LLVM: the native one (same options) with the paths of the
# cross build's products (build/llvm)
"$LN/bin/llvm-config" "\$@" | sed -e "s|$LN|$LO|g"
EOF
chmod +x "$B/bin/llvm-config"
mkdir -p "$B/cxxlib"; ln -sf "$RTI/lib/libc++.a" "$RTI/lib/libc++abi.a" "$B/cxxlib/"
VER_DATE="$(sed -n 3p "$RS/git-commit-info")"
GUEST_ENV=(
  CFG_RELEASE="$RUST_VERSION" CFG_RELEASE_CHANNEL=stable CFG_VERSION="$(cat "$RS/version")"
  CFG_VER_HASH="$COMMIT" CFG_VER_DATE="$VER_DATE" CFG_DISABLE_UNSTABLE_FEATURES=1
  CFG_DEFAULT_CODEGEN_BACKEND=llvm CFG_LIBDIR_RELATIVE=lib CFG_COMPILER_HOST_TRIPLE="$TARGET"
  CFG_VIRTUAL_RUST_SOURCE_BASE_DIR="/rustc/$COMMIT" RUSTC_INSTALL_BINDIR=bin
  REAL_LIBRARY_PATH_VAR=LD_LIBRARY_PATH REAL_LIBRARY_PATH="${LD_LIBRARY_PATH:-}"
  LLVM_CONFIG="$B/bin/llvm-config" LLVM_RUSTLLVM=1 LLVM_USE_LIBCXX=1 LLVM_LINKER_FLAGS="-L$B/cxxlib -lc++abi"
  "CC_${TARGET//-/_}=$GUEST_CC" "CXX_${TARGET//-/_}=$B/bin/guest-c++" "AR_${TARGET//-/_}=$WASM_BIN/llvm-ar"
  RUSTC="$B/bin/rustc"
)
# what patches/rust/0006 has the guest's rustc add to a program's link, given by hand here
EXE_LINK_FLAGS="-Clink-args=--table-base=3 -Clink-args=--export-table -Clink-args=-zstack-size=8388608 -Clink-args=-lclang_rt.builtins"
GUEST_RUSTFLAGS="$EXE_LINK_FLAGS -Clinker=$WASM_BIN/wasm-ld -Zon-broken-pipe=kill --remap-path-prefix=$RS=/rustc/$COMMIT --remap-path-prefix=$CARGO_HOME=/rust/deps"
DLOPEN_FLAGS="-Clink-arg=--growable-table -Clink-arg=@$SYSROOT/lib/libc.exports -Clink-arg=--export-if-defined=__stack_pointer"
RUSTC_REC="$(record "$B/rustc.flags" "${GUEST_ENV[*]} $GUEST_RUSTFLAGS $DLOPEN_FLAGS")"
RUSTC_WASM="$RT/$TARGET/release/rustc-main"
if stage rustc "$B/rustc.stamp" "$B/src.stamp" "$CR/.stamp" "$TLIB/.stamp" "$B/llvm.stamp" "$RUSTC_REC"; then
  # afresh (a few minutes): the tools stage takes all of its rlibs as rustc-main's
  rm -rf "$RT/$TARGET"
  (
    cd "$RS"
    export "${GUEST_ENV[@]}"
    export "CARGO_TARGET_$(echo "$TARGET" | tr a-z- A-Z_)_RUSTFLAGS=$GUEST_RUSTFLAGS $DLOPEN_FLAGS"
    pin_crates
    cargo build --release --target "$TARGET" -Zchecksum-freshness "${PATCH_CRATES[@]}" -j"$JOBS" \
      --manifest-path compiler/rustc/Cargo.toml --features llvm,max_level_info --target-dir "$RT"
  ) > "$B/rustc.log" 2>&1 || fail "$B/rustc.log" "rustc for $TARGET"
  [[ -f "$RUSTC_WASM" ]] || { echo "cargo wrote no rustc-main" >&2; exit 1; }
  touch "$B/rustc.stamp"
fi

# ── tools: rustc, rustdoc, clippy and rustfmt for the guest ───────────────────────────────────
# One program (tools/multicall: rustc, or rustdoc, clippy-driver or rustfmt by the name it is run
# as) and cargo-clippy and cargo-fmt, built as bootstrap builds rustc's tools: a sysroot that has
# the compiler's rlibs beside std (rustc-dev: what rustc-main was built from), whose crates the
# tools' `extern crate rustc_*` take, and a target directory of their own.
SD="$WL/sysroot-dev"
MULTI_WASM="$TT/$TARGET/release/rustc"
cat > "$B/bin/rustc-dev" <<EOF
#!/bin/sh
# the host's rustc as bin/rustc, with the rustc-dev sysroot (std and the compiler's rlibs)
export RUST_TARGET_PATH="$A/target" RUSTC_BOOTSTRAP=1
exec "$HR/bin/rustc" -Zunstable-options --sysroot="$SD" "\$@"
EOF
chmod +x "$B/bin/rustc-dev"
if stage tools "$B/tools.stamp" "$B/rustc.stamp" $(find "$A/tools/multicall" -type f); then
  rm -rf "$SD"; mkdir -p "$SD/lib/rustlib/$TARGET" "$SD/lib/rustlib/$HOST_TRIPLE/lib"
  cp -r "$TLIB" "$SD/lib/rustlib/$TARGET/lib"
  # the compiler's rlibs (the rustc stage's target directory holds only those), and the proc
  # macros they were built with (this machine's, beside its std)
  cp "$RT/$TARGET/release/deps/"*.rlib "$SD/lib/rustlib/$TARGET/lib/"
  for f in "$HR/lib/rustlib/$HOST_TRIPLE"/*; do
    [[ "$(basename "$f")" == lib ]] || ln -s "$f" "$SD/lib/rustlib/$HOST_TRIPLE/"
  done
  for f in "$HR/lib/rustlib/$HOST_TRIPLE/lib"/*; do ln -s "$f" "$SD/lib/rustlib/$HOST_TRIPLE/lib/"; done
  cp "$RT/release/deps/"*.so "$SD/lib/rustlib/$HOST_TRIPLE/lib/"
  (
    cd "$RS"
    # (as bootstrap's tool builds: their own copies of crates the compiler also has get other
    # crate ids than the sysroot's, or the two collide)
    export "${GUEST_ENV[@]}" RUSTC="$B/bin/rustc-dev" __CARGO_DEFAULT_LIB_METADATA=collabo-tools \
      CFG_RELEASE_NUM="$RUST_VERSION" DOC_RUST_LANG_ORG_CHANNEL="https://doc.rust-lang.org/$RUST_VERSION"
    # the C++ library rustc_llvm names (its build script's search path is not in this build)
    export "CARGO_TARGET_$(echo "$TARGET" | tr a-z- A-Z_)_RUSTFLAGS=$GUEST_RUSTFLAGS $DLOPEN_FLAGS -Lnative=$B/cxxlib"
    cargo build --release --target "$TARGET" -Zchecksum-freshness "${PATCH_CRATES[@]}" -j"$JOBS" \
      -p collabo-rustc -p clippy -p rustfmt-nightly --bin rustc --bin cargo-clippy --bin cargo-fmt \
      --target-dir "$TT"
  ) > "$B/tools.log" 2>&1 || fail "$B/tools.log" "rustdoc, clippy, rustfmt for $TARGET"
  [[ -f "$MULTI_WASM" ]] || { echo "cargo wrote no multicall rustc" >&2; exit 1; }
  touch "$B/tools.stamp"
fi

# ── cargo: cargo for the guest ─────────────────────────────────────────────────────────────────
# src/tools/cargo (its own workspace and lock) for wasm32-unknown-linux-musl, with its C libraries
# built in as the official cargo has them (curl, libgit2, libssh2, nghttp2, zlib, SQLite: the
# crates' bundled sources, compiled by guest-cc) but OpenSSL: the guest's own build of it
# (python/deps, OpenSSL 3.5 with /etc/ssl, as the guest's Python and curl use).
PYDEPS="$ROOT/python/deps"
CARGO_WASM="$CT/$TARGET/release/cargo"
CARGO_ENV=(
  CFG_RELEASE="$RUST_VERSION" CFG_RELEASE_CHANNEL=stable CFG_VERSION="$(cat "$RS/version")"
  CFG_VER_HASH="$COMMIT" CFG_VER_DATE="$VER_DATE"
  OPENSSL_DIR="$PYDEPS" OPENSSL_STATIC=1 OPENSSL_NO_VENDOR=1 LIBZ_SYS_STATIC=1
  WASM_MUSL_SYSROOT="$SYSROOT"   # (newer cc crates add it, and wasm Linux's flags, for wasm32 Linux)
  # The C libraries as the guest's own builds of them: SQLite without mmap (python/build-deps.sh's
  # flags: the C library has none), libgit2 without it; size_t is unsigned long here, as wide as
  # the unsigned int libgit2's overflow checks take it for (an error to newer clangs).
  'LIBSQLITE3_FLAGS=-DSQLITE_WASI=1 -DSQLITE_DEFAULT_UNIX_VFS="unix" -DSQLITE_MAX_MMAP_SIZE=0'
  "CFLAGS_${TARGET//-/_}=-DNO_MMAP=1 -Wno-error=incompatible-pointer-types"
  LIBSSH2_SYS_USE_PKG_CONFIG= LIBGIT2_NO_VENDOR=0
  "CC_${TARGET//-/_}=$GUEST_CC" "CXX_${TARGET//-/_}=$B/bin/guest-c++" "AR_${TARGET//-/_}=$WASM_BIN/llvm-ar"
  RUSTC="$B/bin/rustc"
)
CARGO_REC="$(record "$B/cargo.flags" "${CARGO_ENV[*]} $GUEST_RUSTFLAGS")"
if stage cargo "$B/cargo.stamp" "$B/src.stamp" "$CR/.stamp" "$TLIB/.stamp" "$CARGO_REC"; then
  check "$PYDEPS/lib/libssl.a" "run: ./build.sh python (the guest's OpenSSL, python/deps)"
  (
    cd "$RS/src/tools/cargo"
    export "${CARGO_ENV[@]}"
    export "CARGO_TARGET_$(echo "$TARGET" | tr a-z- A-Z_)_RUSTFLAGS=$GUEST_RUSTFLAGS"
    pin_crates
    cargo build --release --target "$TARGET" -Zchecksum-freshness "${PATCH_CRATES[@]}" -j"$JOBS" \
      --features curl/static-curl,vendored-libgit2 --bin cargo --target-dir "$CT"
  ) > "$B/cargo.log" 2>&1 || fail "$B/cargo.log" "cargo for $TARGET"
  [[ -f "$CARGO_WASM" ]] || { echo "cargo wrote no cargo" >&2; exit 1; }
  touch "$B/cargo.stamp"
fi

# ── cpio: the overlay ─────────────────────────────────────────────────────────────────────────
#   /usr/bin/rustc (and rustdoc, clippy-driver, rustfmt: links to it), cargo, cargo-clippy,
#   cargo-fmt; /usr/lib/rustlib/wasm32-unknown-linux-musl/{lib,bin/rust-lld} (std, the C runtime
#   it links with, lld): rustc's sysroot is /usr, as for a distribution's rustc.
CPIO="$OUT/rust.cpio"
if stage cpio "$CPIO" "$B/tools.stamp" "$B/cargo.stamp" "$TLIB/.stamp" "$B/llvm.stamp" "$A/share/cargo/config.toml" \
     "$A"/guest/* "$CR/.stamp"; then
  ST="$B/stage"
  rm -rf "$ST"; mkdir -p "$ST/usr/bin" "$ST/usr/lib/rustlib/$TARGET/bin" "$ST/usr/share/doc/rust"
  # without DWARF (none: debuginfo is off) and producers, but with the name section: a trap's
  # backtrace then names Rust functions
  wstrip() { "$WASM_BIN/llvm-objcopy" --strip-debug --remove-section=producers "$1" "$2"; chmod 755 "$2"; }
  wstrip "$MULTI_WASM" "$ST/usr/bin/rustc"
  for l in rustdoc clippy-driver rustfmt; do ln -s rustc "$ST/usr/bin/$l"; done
  # cargo itself beside rust-lld; /usr/bin/cargo is guest/cargo (the CA certificates for it)
  wstrip "$CARGO_WASM" "$ST/usr/lib/rustlib/$TARGET/bin/cargo"
  install -m 755 "$A/guest/cargo" "$ST/usr/bin/cargo"
  for b in cargo-clippy cargo-fmt; do wstrip "$TT/$TARGET/release/$b" "$ST/usr/bin/$b"; done
  wstrip "$LO/bin/lld" "$ST/usr/lib/rustlib/$TARGET/bin/rust-lld"
  cp -r "$TLIB" "$ST/usr/lib/rustlib/$TARGET/lib"
  rm -f "$ST/usr/lib/rustlib/$TARGET/lib/.stamp"
  # what rustup's components list says is installed (rustc --print sysroot users look here)
  printf 'rustc\ncargo\nrust-std-%s\nrustfmt\nclippy\n' "$TARGET" > "$ST/usr/lib/rustlib/components"
  chmod -R u+w,go+rX "$ST/usr"
  echo "== packing cpio"
  # The crates whose crates.io versions get this platform's ABI wrong (libc takes wasm32 Linux for
  # WALI, with a 64-bit long; rustix and linux-raw-sys know no wasm32 Linux at all), as patched
  # here, for every project in the guest: /.cargo/config.toml (share/cargo/config.toml) has them as
  # its [patch.crates-io]. Each stands for the versions of its line (0.2.*, 1.*, 0.38.*, ...).
  GUEST_PATCH="libc-$LIBC_VERSION rustix-1.1.4 linux-raw-sys-0.12.1 rustix-0.38.44 linux-raw-sys-0.4.15"
  mkdir -p "$ST/usr/lib/rustlib/src/crates" "$ST/.cargo"
  cp "$A/share/cargo/config.toml" "$ST/.cargo/config.toml"
  printf '\n[patch.crates-io]\n' >> "$ST/.cargo/config.toml"
  for nv in $GUEST_PATCH; do
    cp -r "$CR/$nv" "$ST/usr/lib/rustlib/src/crates/"
    # linux-raw-sys: of its bindings for every architecture, the one wasm32 is (riscv32's: the
    # asm-generic 32-bit ABI); the others are behind cfgs never true here
    if [[ "$nv" == linux-raw-sys-* ]]; then
      for d in "$ST/usr/lib/rustlib/src/crates/$nv/src"/*/; do
        [[ "$(basename "$d")" == riscv32 ]] || rm -rf "$d"
      done
    fi
    printf '%s = { package = "%s", path = "/usr/lib/rustlib/src/crates/%s" }\n' \
      "$(echo "$nv" | tr '.+-' '___')" "${nv%-*}" "$nv" >> "$ST/.cargo/config.toml"
  done
  chmod -R u+w,go+rX "$ST/.cargo" "$ST/usr/lib/rustlib/src"
  python3 "$U/mkinitramfs.py" --overlay -o "$CPIO" --tree "usr=$ST/usr" --tree ".cargo=$ST/.cargo"
  rm -rf "$ST"
  cp "$A/addon.json" "$OUT/rust.json"

  # licenses: Rust (rustc, std, cargo, clippy, rustfmt: MIT OR Apache-2.0, and what COPYRIGHT
  # lists), LLVM and lld (Apache-2.0 WITH LLVM-exception), musl (MIT), compiler-rt, OpenSSL
  # (Apache-2.0), and the C libraries cargo has built in (curl, libgit2, libssh2, zlib, SQLite)
  L="$OUT/licenses"; rm -rf "$L"; mkdir -p "$L"
  cp "$RS/COPYRIGHT" "$L/rust-COPYRIGHT.txt"
  cp "$RS/LICENSE-APACHE" "$L/rust-LICENSE-APACHE.txt"; cp "$RS/LICENSE-MIT" "$L/rust-LICENSE-MIT.txt"
  cp "$RS/src/tools/cargo/LICENSE-THIRD-PARTY" "$L/cargo-LICENSE-THIRD-PARTY.txt"
  cp "$LS/llvm/LICENSE.TXT" "$L/llvm-LICENSE.txt"; cp "$LS/lld/LICENSE.TXT" "$L/lld-LICENSE.txt"
  cp "$U/src/musl/COPYRIGHT" "$L/musl-COPYRIGHT.txt" 2>/dev/null || true
  cp "$U/src/llvm/compiler-rt/LICENSE.TXT" "$L/compiler-rt-LICENSE.txt" 2>/dev/null || true
  for f in "$ROOT"/python/build-deps/openssl-*/LICENSE.txt; do [[ -f "$f" ]] && cp "$f" "$L/openssl-LICENSE.txt"; done
  # what cargo has built in: each -sys crate's bundled C library (its license file in the sources)
  for cl in curl-sys:curl/COPYING:curl libgit2-sys:libgit2/COPYING:libgit2 libssh2-sys:libssh2/COPYING:libssh2 \
            libnghttp2-sys:nghttp2/COPYING:nghttp2 libz-sys:src/zlib/LICENSE:zlib; do
    IFS=: read -r c f name <<< "$cl"
    d="$(ls -d "$CARGO_HOME"/registry/src/*/"$c"-* 2>/dev/null | sort -V | tail -1)"
    [[ -n "$d" && -f "$d/$f" ]] && cp "$d/$f" "$L/$name-LICENSE.txt"
  done
  # SQLite is in the public domain (sqlite3.c's header says so); the Rust crates' licenses: cargo's
  # LICENSE-THIRD-PARTY and each crate's MIT/Apache-2.0
  ls -la "$CPIO"
fi

echo "== done"
