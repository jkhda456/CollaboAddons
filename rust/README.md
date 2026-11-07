# rust add-on

The Rust toolchain in the sandbox: **rustc 1.99.1 and cargo**, built for the guest's WebAssembly
Linux, so that agents can build and run Rust there — `cargo new`, `cargo add`, `cargo build`,
`cargo run`, `cargo test` (with doc tests), `cargo clippy`, `cargo fmt` — for the guest's own target,
`wasm32-unknown-linux-musl`, which this rustc knows as its host. Proc macros work (each one a
WebAssembly side module that rustc loads with `dlopen`: serde's derive, from crates.io, builds and
runs), and so do build scripts, C code in build scripts (with the llvm add-on), threads, processes
and unwinding panics in what it builds.

| In the guest | What |
|---|---|
| `/usr/bin/rustc` | rustc 1.99.1 (`rustc -vV`: `host: wasm32-unknown-linux-musl`, LLVM 23.1.1), one program that is also `rustdoc`, `clippy-driver` and `rustfmt` (links to it: it is what it is run as), 158 MB |
| `/usr/bin/cargo` | cargo 1.99.1 (`/usr/lib/rustlib/wasm32-unknown-linux-musl/bin/cargo`, with curl and HTTP/2, libgit2, libssh2, OpenSSL and SQLite built in), through a script that hands it the guest's CA certificates |
| `/usr/bin/cargo-clippy`, `/usr/bin/cargo-fmt` | `cargo clippy`, `cargo fmt` |
| `/usr/lib/rustlib/wasm32-unknown-linux-musl/lib` | the standard library (`std`, `alloc`, `core`, `proc_macro`, `test`, …: position-independent, with bitcode for LTO), and in `self-contained/` what a program is linked with: musl's `crt1.o` and `libc.a`, compiler-rt, `libunwind.a` |
| `/usr/lib/rustlib/wasm32-unknown-linux-musl/bin/rust-lld` | lld 23.1.1, rustc's linker |
| `/.cargo/config.toml`, `/usr/lib/rustlib/src/crates` | cargo's defaults for every project in the guest: the crates.io crates that get this platform's ABI wrong (below), patched, and the cc crate's sysroot |

rustc's sysroot is `/usr`, as a distribution's rustc. Only the guest's target: this LLVM has the
WebAssembly backend alone, and there is no standard library for another target. The image is
365 MB, so the add-on is not bundled in the runtime (`addons/README.md`); an app takes
`out/rust.cpio` and `out/rust.json`.

## Turning it on

```json
"addons": {"rust": true}
```

or `--addon-dir addons/rust/out --addon rust` on the engine's command line. It has no settings.
Crates come from crates.io through the guest's network and its policy: allow `index.crates.io`,
`static.crates.io` (and `crates.io` for `cargo search`/`publish`, `github.com` for git
dependencies). Offline: path dependencies, a vendored directory, `CARGO_NET_OFFLINE=true`.

```sh
cargo new hello && cd hello && cargo run
cargo add serde --features derive && cargo add serde_json && cargo build --release
cargo test && cargo clippy && cargo fmt
rustc -O main.rs && ./main
```

C and C++ in a crate (the cc crate: `zstd-sys`, `libsqlite3-sys`, `ring`, …) need a C compiler in
the guest: the llvm add-on's `cc` (turn on both add-ons; `"addons": {"llvm": true, "rust": true}`).

Speed: the engine compiles each program once (it caches them); then, e.g., the offline checks
below (a workspace with a proc macro and a build script, built twice, tested, `cargo new`, clippy,
fmt) take ~8 s in the guest, and serde + serde_json from crates.io build in ~14 s.

## The target

`wasm32-unknown-linux-musl` is the guest's platform as its C compiler sees it (userspace/bin/wasm-cc,
the llvm add-on's cc): tombl/linux's wasm32 port with musl, ILP32 (`long` is 32 bits, unlike
WALI's `wasm32-wali-linux-musl`), a program a module of its own that imports a shared memory (up to
4 GiB), with threads. patches/rust/0005 makes it one of rustc's built-in targets
(`rustc --print target-spec-json -Zunstable-options`); `target/wasm32-unknown-linux-musl.json` is
the same as a JSON spec, for the build machine's rustc.

- `panic=unwind`: panics unwind with WebAssembly exception handling (exnref): `catch_unwind`, the
  destructors on the way, a panicking thread `join`ed as `Err`, a proc macro's panic reported as an
  error, and rustc's own fatal errors (a compile error exits with status 1, not an abort).
- Position-independent code by default (as on other Linux targets), so the same rlibs go into
  programs and into side modules: `dylib`, `cdylib` and `proc-macro` crates are WebAssembly side
  modules (`dylink.0`, `lib*.so`) that the C library's `dlopen` loads (patches/rust/0006, 0011).
- A program is linked by rust-lld with musl's crt1.o and libc.a and compiler-rt (self-contained, as
  Rust's musl targets): a function table based at 3 and exported (`SIG_IGN`/`SIG_HOLD` are 1 and
  2; the kernel calls signal handlers through it), an 8 MiB stack.
- Memory mappings: the guest's C library has no `mmap` (WebAssembly has no MMU); the target's
  `libc.a` adds `mmap`, `munmap`, `mremap`, `msync`, `mprotect`, `madvise`, … as copies
  (rt/mman-wasm.c), so crates that map files (memmap2) work. A copy does not see the file change
  under it, and others see its writes once they are written back (msync, munmap).
- Processes: the guest cannot fork a process that has threads; std starts a child with
  `clone(CLONE_VM | CLONE_VFORK)` (patches/rust/0007) where it cannot use `posix_spawn` (pre_exec
  closures, such as the jobserver's, a changed PATH, setsid, …).
- crates.io: libc takes `wasm32` + Linux + musl for WALI (a 64-bit `long`: every struct with a
  `long` in it laid out wrong here), and rustix and linux-raw-sys know no wasm32 Linux at all. The
  guest's `/.cargo/config.toml` (cargo reads the config of every directory above the current one)
  patches them for every project, with the versions in `/usr/lib/rustlib/src/crates`: libc
  0.2.189, rustix 1.1.4 and 0.38.44, linux-raw-sys 0.12.1 and 0.4.15 — each standing for the
  versions of its line. A project that requires a newer one of those lines (libc 0.2.190, say)
  gets crates.io's, which compiles but is wrong; cargo then warns that the patch was not used.
  Crates whose own `cfg(target_arch = "wasm32")` means "a browser" may need the same
  (`patches/crates/*@wasm-cfg.patch` and the codex add-on's `patches/narrow-wasm-cfg.py`).

## How it is built

`./build.sh` (stages, inputs and outputs in its header; `./build.sh verify-src` compares the trees
below). The host compiler is the **official Rust 1.99.1** for the build machine (checksum-pinned in
`sources.lock`), with RUSTC_BOOTSTRAP and the JSON spec: it builds the guest's standard library
(`-Zbuild-std`, as bootstrap builds it) and cross-compiles rustc and cargo with plain cargo and
bootstrap's settings (`CFG_*`). A target found by name through `RUST_TARGET_PATH` is recorded in
metadata as a built-in one is, and the guest's rustc says the same version as the official one
(`1.99.1 (58cf8f9c3 2026-10-13)`), so it reads the std built on the host as its own. No native
stage-1 compiler is built. rustdoc, clippy and rustfmt are built as bootstrap builds tools: with a
sysroot that holds the compiler's rlibs (rustc-dev), as one program with rustc (tools/multicall).

Sources: the user's tree (`RUST_SRC`, the GitHub archive of tag 1.99.1) and the official
`rustc-1.99.1-src.tar.xz`; the build takes the tarball's tree, which has what the archive leaves
out (the submodules `src/llvm-project`, `src/tools/cargo`, `library/backtrace`, and
`git-commit-info`). `verify-src` found compiler/ and library/ the same files (the tarball adds
`library/vendor` and `library/.cargo`, and has no dotfiles).

LLVM (the tarball's `src/llvm-project`, rust-lang/llvm-project `1b9c0d5f`, LLVM 23.1.1) is
cross-compiled with the llvm add-on's clang and libc++ (as that add-on builds its tools): the
libraries rustc_llvm links (WebAssembly only) and lld. Rust code is linked with the llvm add-on's
wasm-ld. OpenSSL is the guest's own (python/deps), as its Python and curl have it.

Needs: `./build.sh userspace` (musl, compiler-rt) and `python` (python/deps: OpenSSL for cargo),
the llvm add-on built (its `build/native` clang, `build/runtimes-install` libc++), clang-19/lld-19
on the host, and network (downloads, crates). A build from scratch: ~1 h here (6 cores), most of it
LLVM; rustc itself ~3 min, the tools ~1 min, cargo ~2 min.

Room: `RUST_ADDON_WORK` (default `build/collabo` inside `RUST_SRC`) holds what is only written and
read through, never mapped: the downloads, cargo's home and the cross-compiled LLVM's build tree
(~1.5 GB). It may be a shared folder (9p), whose clock may differ from this machine's: the stages'
stamps stay in `build/`, and cargo decides what is fresh by checksums. What gets mapped (the cargo
target directories: rustc maps rlibs, a linker its inputs) stays in `build/work`
(`RUST_ADDON_LOCAL`, ~3 GB): on the 9p share a linker's mapped inputs came back with pages of other
files in them (a rustc whose code held another object's strings), so LLVM's products are copied to
`build/llvm` and lld linked again there. `tools/go` runs build.sh from a copy (editing a running
bash script corrupts it).

## What changed from Rust

patches/rust/ (each patch's header says why):

- **0001–0003** (from distro's rust-toolchain, as python/build-rust-toolchain.sh): ILP32 `c_long`,
  std's args, no stack-overflow handler on wasm32.
- **0004** (from the codex add-on): duplicating file descriptors on wasm32 Linux.
- **0005** the built-in target `wasm32-unknown-linux-musl`.
- **0006** rustc's wasm-ld linker: programs and side modules for wasm32 Linux (no C library in a
  side module; libgcc_s, asked for without crt-static, is libunwind).
- **0007** std: child processes without fork (vfork-style clone; PATH searched in the child's
  environment).
- **0008** rustc_driver as an rlib (a program here cannot link a dylib at start).
- **0009** clippy-driver's and rustfmt's mains usable as modules of the multicall rustc.
- **0010** the sysroot of a statically linked rustc (dladdr knows only dlopen's libraries).
- **0011** proc macros on wasm32 Linux (rustc keeps wasm proc macros behind -Zwasm-proc-macros).

patches/llvm/ (from the llvm add-on): no fork, no mmap, no madvise in LLVM's Support and lld.
patches/crates/: libc (tombl's fork's ILP32 musl ABI), rustix and linux-raw-sys (from the codex
and typst add-ons), object (a wasm32 Linux program is a wasm module), libgit2-sys (posix_spawn, no
fork), and `@wasm-cfg` ones narrowing "wasm32 means a browser" cfgs (dirs, filetime, futures-timer,
gix-*, is_executable, parking_lot_core, tar). rt/: libunwind.a (unwinding over WebAssembly
exceptions) and the memory-mapping calls. guest/cargo, share/cargo/config.toml: the guest's
defaults for cargo.

## Checks

`tests/run.sh` boots the runtime with the add-on and runs tests/guest.sh, offline: the tools and
their versions; a program built in the guest (hello); tests/src/smoke.rs (unwinding, catch_unwind,
threads and thread-locals, posix_spawn and the vfork path, files); an uncaught panic (exit 101); a
compile error (exit 1); a cargo workspace (tests/src/proj: a proc macro without syn, a build script,
path dependencies) built in debug and release and run; `cargo test` (unit, integration, doc
tests); a panicking proc macro; `cargo new`; clippy and rustfmt through cargo. `NET=1` adds serde
and serde_json from crates.io, `LLVM=1` C in a build script (cc) and the zstd crate with the llvm
add-on. 2026-10-31: 13/13 (`NET=1 LLVM=1`), ~45 s in the guest.

## License

The add-on's own work is GPL-3.0-only (`../LICENSE`). What it builds keeps its license, in
`out/licenses`: Rust (rustc, std, cargo, clippy, rustfmt: MIT OR Apache-2.0, `COPYRIGHT`), LLVM and
lld (Apache-2.0 WITH LLVM-exception), musl (MIT), compiler-rt, OpenSSL (Apache-2.0), curl, libgit2
(GPL-2.0 with the linking exception), libssh2, nghttp2, zlib; SQLite is in the public domain.
