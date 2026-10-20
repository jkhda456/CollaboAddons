# Shell helpers of the add-on's build.sh and of each packages/<name>/build.sh (sourced; build.sh sets
# the variables below and exports them to the packages' scripts).
#
#   PYEXT    the add-on's folder          B      build/ (BP: build/<package>, the package's own)
#   ROOT     collaboCore                  DL     build/dl: the downloads, checked against sources.lock
#   JOBS     parallel jobs                FORCE  ",stage,stage," (FORCE=stage[,stage])

# stage NAME OUTPUT [INPUT...]: true when the stage must run (forced, the output missing, or an input
# newer than it).
stage() {
  local name=$1 out=$2 src; shift 2
  if [[ "$FORCE" != *",$name,"* && -e "$out" ]]; then
    for src in "$@"; do
      if [[ "$src" -nt "$out" ]]; then echo "== $name: $(basename "$src") changed"; return 0; fi
    done
    echo "== $name: up to date"; return 1
  fi
  echo "== $name"; return 0
}
redo() { FORCE="$FORCE$1,"; }   # NAME: that stage must run (what it depends on was redone)

# log_run LOG COMMAND...: run it with its output in LOG; on failure show the end of LOG and stop.
log_run() {
  local log=$1; shift
  "$@" >> "$log" 2>&1 || { tail -40 "$log"; echo "FAILED: $* (log: $log)" >&2; exit 1; }
}

# fetch NAME: download (once) the source sources.lock names into build/dl and check its sha256;
# prints its path. (A file another add-on downloaded already is taken from there, hard-linked.)
fetch() {
  local line url sum file other
  line="$(grep -E "^$1 " "$PYEXT/sources.lock")" || { echo "$1 is not in sources.lock" >&2; exit 1; }
  read -r _ url sum file <<<"$line"
  file="$DL/${file:-$(basename "$url")}"
  mkdir -p "$DL"
  if [[ ! -f "$file" ]]; then
    for other in "$PYEXT"/../*/build/dl/"$(basename "$file")"; do
      [[ -f "$other" ]] && { ln "$other" "$file" 2>/dev/null || cp "$other" "$file"; break; }
    done
  fi
  if [[ ! -f "$file" ]]; then
    echo "== fetch $(basename "$file")" >&2
    curl -fL --retry 3 -s -S -o "$file.part" "$url" && mv "$file.part" "$file"
  fi
  echo "$sum  $file" | sha256sum -c --quiet - >&2 || { echo "checksum mismatch: $file (deleted)" >&2; rm -f "$file"; exit 1; }
  echo "$file"
}

# unpack NAME DIR: a fresh copy of the source in DIR/<its top directory>, every file with the same
# mtime (so no generated file looks older than its input); prints that directory.
unpack() {
  local file top
  file="$(fetch "$1")"
  mkdir -p "$2"
  # (the top directory from Python: `tar -t | head -1` fails under pipefail once tar gets SIGPIPE)
  case "$file" in
    *.whl|*.zip) top="$(python3 -I -c 'import sys, zipfile; print(zipfile.ZipFile(sys.argv[1]).namelist()[0].split("/")[0])' "$file")"
                 rm -rf "${2:?}/$top"; python3 -I -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' "$file" "$2" ;;
    *)           top="$(python3 -I -c 'import sys, tarfile; print(tarfile.open(sys.argv[1]).next().name.split("/")[0])' "$file")"
                 rm -rf "${2:?}/$top"; tar -xmf "$file" -C "$2" ;;
  esac
  find "$2/$top" -exec touch -h -d "@$(date +%s)" {} +
  echo "$2/$top"
}

# lock NAME...: for each, a file (build/locks/NAME) holding its sources.lock line, rewritten only
# when that line changes: what a stage built from that source depends on (not the whole of
# sources.lock, which every new package changes). Prints the paths.
lock() {
  local name line file
  mkdir -p "$B/locks"
  for name; do
    line="$(grep -E "^$name " "$PYEXT/sources.lock")" || { echo "$name is not in sources.lock" >&2; exit 1; }
    file="$B/locks/$name"
    [[ "$(cat "$file" 2>/dev/null)" == "$line" ]] || echo "$line" > "$file"
    echo "$file"
  done
}

# record FILE TEXT: FILE holds TEXT, rewritten only when it changes; prints FILE, a stage's input
# for what is no file (a stage's flags).
record() {
  mkdir -p "$(dirname "$1")"
  [[ -f "$1" && "$(cat "$1")" == "$2" ]] || printf '%s\n' "$2" > "$1"
  echo "$1"
}

# patchset PATCHDIR RECORD: RECORD names PATCHDIR's patches with their checksums (record); an input
# of the stage that applies them (a patch removed or renamed leaves no file newer than the stage's
# output, but changes the record).
patchset() {
  record "$2" "$(cd "$1" && sha256sum -- *.patch 2>/dev/null)"
}

# apply_patches DIR PATCHDIR: every PATCHDIR/*.patch on DIR, in order (patch -p1).
apply_patches() {
  local p
  for p in "$2"/*.patch; do
    [[ -f "$p" ]] || continue
    patch -d "$1" -p1 --forward --no-backup-if-mismatch --quiet < "$p" || { echo "patch failed: $p" >&2; exit 1; }
  done
}

# cmake_cross SOURCE BUILD PREFIX [ARG...]: configure a CMake project for the guest with the add-on's
# compilers, static libraries only, installing to PREFIX. Libraries and headers are looked for in
# PREFIX and python/deps alone (CMAKE_FIND_ROOT_PATH, ONLY): never this machine's.
cmake_cross() {
  local src=$1 build=$2 prefix=$3; shift 3
  "$CMAKE" -S "$src" -B "$build" -G Ninja -Wno-dev --log-level=WARNING -DCMAKE_MAKE_PROGRAM="$NINJA" \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR=wasm32 \
    -DCMAKE_C_COMPILER="$PYEXT/tools/wasm-pyext-cc" -DCMAKE_CXX_COMPILER="$PYEXT/tools/wasm-pyext-c++" \
    -DCMAKE_AR=/usr/bin/llvm-ar-19 -DCMAKE_RANLIB=/usr/bin/llvm-ranlib-19 \
    -DCMAKE_C_FLAGS=-O2 -DCMAKE_CXX_FLAGS=-O2 \
    -DCMAKE_INSTALL_PREFIX="$prefix" -DCMAKE_INSTALL_LIBDIR=lib -DBUILD_SHARED_LIBS=OFF \
    -DCMAKE_FIND_ROOT_PATH="$prefix;$ROOT/python/deps" -DCMAKE_FIND_ROOT_PATH_MODE_PROGRAM=NEVER \
    -DCMAKE_FIND_ROOT_PATH_MODE_LIBRARY=ONLY -DCMAKE_FIND_ROOT_PATH_MODE_INCLUDE=ONLY \
    -DCMAKE_FIND_ROOT_PATH_MODE_PACKAGE=ONLY "$@"
}

# zlib_pic SOURCE PREFIX LOG: zlib from its source as the python step builds its own (a handful of
# C files, python/build-deps.sh), for the guest and PIC: PREFIX/lib/libz.a, PREFIX/include.
zlib_pic() {
  local src=$1 prefix=$2 log=$3 f
  mkdir -p "$prefix/lib" "$prefix/include"
  for f in adler32 crc32 deflate infback inffast inflate inftrees trees zutil compress uncompr gzclose gzlib gzread gzwrite; do
    log_run "$log" "$PYEXT/tools/wasm-pyext-cc" -O2 -DZ_HAVE_UNISTD_H -c "$src/$f.c" -o "$src/$f.o"
  done
  rm -f "$prefix/lib/libz.a"
  llvm-ar-19 rcs "$prefix/lib/libz.a" "$src"/*.o
  cp "$src/zlib.h" "$src/zconf.h" "$prefix/include/"
}

# configure_cross SOURCE PREFIX [ARG...]: an autoconf configure for the guest (in SOURCE), static.
configure_cross() {
  local src=$1 prefix=$2; shift 2
  (cd "$src" && CONFIG_SITE="$ROOT/python/config.site" ./configure \
     --host=wasm32-unknown-linux-musl --build="$(uname -m)-pc-linux-gnu" --prefix="$prefix" \
     --disable-shared --enable-static CC="$PYEXT/tools/wasm-pyext-cc" CXX="$PYEXT/tools/wasm-pyext-c++" \
     AR=llvm-ar-19 RANLIB=llvm-ranlib-19 CFLAGS=-O2 CXXFLAGS=-O2 "$@")
}

# rust_std_pic: the guest's Rust standard library built position-independent, for the packages
# written in Rust: an extension module is a shared library, and the std it links goes into it,
# which the shared toolchain's (python/build-rust-toolchain.sh: static code, for programs) cannot.
# Built as that script builds its own — its patched copy of rust-src, tombl's libc fork (the
# guest's ILP32 musl), the locked dependencies, no backtraces (they need mmap and an unwinder) —
# with -Crelocation-model=pic, into build/rust (stage rust-std). Sets RUST_NIGHTLY, RUST_TARGET,
# RUSTC_PIC (a rustc that uses that std: cargo's RUSTC) and the cargo environment (RUSTUP_HOME,
# CARGO_HOME, PATH, the host's C compiler for build scripts).
rust_std_pic() {
  local R="$ROOT/.tools/rust-wasm" RS="$B/rust" tc host flags
  RUST_NIGHTLY=nightly-2026-07-22 RUST_TARGET=wasm32-unknown-linux-musl   # as python/build-rust-toolchain.sh
  export RUSTUP_HOME="$ROOT/.tools/rustup" CARGO_HOME="$ROOT/.tools/cargo"
  export PATH="$CARGO_HOME/bin:$PATH"
  bash "$ROOT/python/build-rust-toolchain.sh"
  tc="$(rustc +"$RUST_NIGHTLY" --print sysroot)"
  host="$(rustc +"$RUST_NIGHTLY" -vV | sed -n 's/^host: //p')"
  # build scripts and proc macros are host programs, and this machine's C compiler is clang-19 (no `cc`)
  export "CARGO_TARGET_$(echo "$host" | tr a-z- A-Z_)_LINKER=clang-19" "CC_${host//-/_}=clang-19" \
         "CXX_${host//-/_}=clang++-19" "AR_${host//-/_}=llvm-ar-19"
  RUSTC_PIC="$RS/rustc"
  flags="-Cembed-bitcode=yes -Crelocation-model=pic --remap-path-prefix=$RS/src=/usr/src/rust --remap-path-prefix=$CARGO_HOME=/usr/src/cargo"
  local stamp="$RS/sysroot/lib/rustlib/$RUST_TARGET/lib/libstd.stamp"
  if stage rust-std "$stamp" "$R/sysroot/lib/rustlib/$RUST_TARGET/lib/libstd.stamp" "$(record "$B/rust-std.flags" "$flags")"; then
    rm -rf "$RS"; mkdir -p "$RS/build/src" "$RS/build/.cargo"
    cp -r "$R/src" "$RS/src"
    touch "$RS/build/src/lib.rs"
    printf '[package]\nname = "sysroot-build"\nversion = "0.0.0"\nedition = "2021"\n' > "$RS/build/Cargo.toml"
    printf '[unstable]\nbuild-std = ["std", "panic_abort"]\nbuild-std-features = []\n' > "$RS/build/.cargo/config.toml"
    echo "  building std (log: $B/rust-std.log)"
    (
      cd "$RS/build"
      export RUST_WASM_NO_SYSROOT=1 RUSTC="$R/bin/rustc" __CARGO_TESTS_ONLY_SRC_ROOT="$RS/src/library" RUSTFLAGS="$flags"
      cargo +"$RUST_NIGHTLY" build --release --target "$RUST_TARGET" -j"$JOBS"
    ) > "$B/rust-std.log" 2>&1 || { tail -30 "$B/rust-std.log"; echo "FAILED: std for $RUST_TARGET (log: $B/rust-std.log)" >&2; exit 1; }
    local lib; lib="$(dirname "$stamp")"
    mkdir -p "$lib"
    ln -s "$tc/lib/rustlib/$host" "$RS/sysroot/lib/rustlib/$host"
    cp "$RS/build/target/$RUST_TARGET/release/deps/"*.rlib "$lib/"
    # std names _Unwind_ symbols though nothing unwinds (panic=abort): the shared toolchain's stubs, PIC
    "$ROOT/userspace/bin/wasm-cc" -fPIC -c "$ROOT/third_party/distro/distro/rust-toolchain/unwind-stubs.c" -o "$RS/unwind-stubs.o"
    llvm-ar-19 rcs "$lib/libunwind.a" "$RS/unwind-stubs.o"
    cat > "$RUSTC_PIC" <<SH
#!/bin/sh
export RUST_TARGET_PATH="$R/target"
exec "$tc/bin/rustc" -Zunstable-options --sysroot="$RS/sysroot" "\$@"
SH
    chmod +x "$RUSTC_PIC"
    rm -rf "$RS/build/target" "$RS/src"   # the objects, and the source copy: the rlibs are what is used
    cp "$R/sysroot/lib/rustlib/$RUST_TARGET/lib/libstd.stamp" "$stamp"
  fi
}
