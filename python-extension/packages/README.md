# packages/: one folder per Python package

Each folder here is one package whose extension modules are compiled for the guest, as shared
libraries its CPython loads with `dlopen`: `numpy`, `pillow`, `lief`, `lxml`, `cffi`,
`cryptography`, `rpds-py` — and the pure-Python ones those need or need them: `pycparser` (cffi's), and
`mcp` (the MCP SDK, with the wheels it needs but rpds-py and cryptography). `../build.sh` runs
`packages/<name>/build.sh` for each name in its `PACKAGES` list (or those on its command line),
then installs what they made into one site-packages.

## What a package's build.sh does

It is run with `bash`, after the toolchain and host stages, with these in its environment:

| | |
|---|---|
| `PYEXT`, `ROOT` | the add-on's folder; collaboCore |
| `B`, `BP` | `build/`; `build/<name>`, the package's own work folder (kept between builds) |
| `DL` | `build/dl`: the downloads, each checked against `../sources.lock` |
| `JOBS`, `FORCE` | parallel jobs; the stages to redo (`,stage,stage,`) |
| `CMAKE`, `NINJA`, `HOSTPY` | CMake and Ninja (`.tools/cmake`); the host CPython 3.13 (`python/host`) |
| `PATH` | has `build/host/bin`: `python3-guest` (the host CPython answering sysconfig with the guest's build variables), `cython` |

and `source "$PYEXT/tools/lib.sh"` gives it `stage`, `redo`, `log_run`, `fetch`, `unpack`, `lock`
(a stage's dependency on one `sources.lock` line), `apply_patches`, `cmake_cross` and
`configure_cross` (CMake and autoconf projects for the guest, static and PIC, with the add-on's
compilers and nothing of this machine's libraries), `zlib_pic` (zlib, as the python step builds
it), `record` and `patchset` (a stage's dependency on its flags, or on its set of patches),
`rust_std_pic` (the guest's Rust standard library built position-independent, for a package in
Rust: it sets `RUSTC_PIC`, `RUST_TARGET`, `RUST_NIGHTLY` and cargo's environment).

It must leave **`$BP/site/`**: a site-packages tree with

- the package's Python files, as an installed wheel has them;
- its extension modules as `tools/wasm-pyext-cc` links them when a build system asks for a shared
  library (`-shared`): WebAssembly side modules under the module's file name
  (`numpy/_core/_multiarray_umath.cpython-313.so`; `.abi3.so` for the stable ABI's) — none for a
  pure-Python package, which is installed from its wheel (`pycparser/build.sh`);
- exactly one `.dist-info`, made by `tools/dist-info.py` from the sdist's `PKG-INFO` (or
  `pyproject.toml`), with the licenses of the package and of every library it carries — or, for
  pure-Python wheels unpacked as they are, one each, with the wheel's own `RECORD`, which says
  which files are that distribution's (`mcp/build.sh`: thirteen wheels in one tree);

and then `touch $BP/site/.done`. Its stages should skip when their output is up to date (`stage`),
so that a build of the add-on redoes only what changed.

Every compile it makes is recorded (`build/<name>/compdb`), and the stage after it,
`fpcast-<name>`, scans them with `tools/fpcast-scan.py`: a function in a method table, a type
slot, a module slot or a getset that CPython would call as a type it does not have —
`f(PyObject *self)` registered METH_NOARGS (CPython passes two arguments), a getter without its
closure, an `int f(void)` as `Py_mod_exec` — traps in the guest, and stops the build. Fix them with
a patch (numpy's `patches/0002`, Pillow's `patches/pillow/0001`, `0002`), or list the source in
`packages/<name>/fpcasts.allow` when it is never called (with the reason).

`../build.sh` then (`tools/install-tree.py`) byte-compiles the Python files, writes `INSTALLER`,
`RECORD` and the console scripts of `entry_points.txt`, and checks that each module links: what it
imports must be the interpreter's (the Python API, the C library), its own, or weak.

## Building one

The compilers are `$PYEXT/tools/wasm-pyext-cc` and `wasm-pyext-c++` (C++17 and later, with
exceptions; every compile `-fPIC`); the archiver `llvm-ar-19`. However the package builds:

- **meson** (numpy): a cross file naming those, `python = build/host/bin/python3-guest`,
  `[host_machine] cpu_family = 'wasm32'` (`numpy/build.sh`), then `meson install --destdir`.
- **CMake** (LIEF, and Pillow's libraries): `cmake_cross SOURCE BUILD PREFIX -D...`; for a Python
  module, `-DPython_EXECUTABLE=$B/host/bin/python3-guest -DPython_INCLUDE_DIR=$ROOT/python/stage/usr/include/python3.13`.
- **setuptools** (Pillow): `python3-guest setup.py build` with `CC=wasm-pyext-cc` and
  `LDSHARED="wasm-pyext-cc -shared"`, and the package's switches that keep it from looking in
  `/usr/include` (Pillow: `--disable-platform-guessing`).
- **autoconf**: `configure_cross SOURCE PREFIX ...`.
- **cargo** (cryptography, PyO3): `rust_std_pic`, then `cargo rustc --release --lib --target
  $RUST_TARGET --crate-type staticlib` with `RUSTC=$RUSTC_PIC`, `RUSTFLAGS=-Crelocation-model=pic`,
  `--config patch.crates-io.libc.path=...` (tombl's libc fork, the guest's ILP32 musl), the cc
  crate's `CC_wasm32_unknown_linux_musl` a wrapper of `wasm-pyext-cc` without `--target`, and
  `PYO3_CONFIG_FILE` saying what the guest's interpreter is (no library to link). The static
  library is linked by `wasm-pyext-cc -shared -Wl,--no-export-dynamic -Wl,--export=PyInit_<name>`
  with std's `libunwind.a` (its stubs): Rust's symbols are not hidden, and exported they would keep
  all of std. Add the package to `RUST` in `../build.sh` (Rust's std's license), and list the crates
  it links with `tools/crates.py` (from `cargo metadata`). A crate with no C (rpds-py) leaves no
  compile records: its `fpcast-` stage has nothing to scan (PyO3 declares each method as CPython
  calls it).
- a build that **runs cffi** (cryptography's `_openssl.c`): `PYTHONPATH=$B/cffi/host-site`, cffi
  for the host CPython (`cffi/build.sh`; so `cffi` comes first in `PACKAGES`).

Build systems find the host CPython's headers through its sysconfig paths; the compilers read them
as the guest's (`python/stage/usr/include/python3.13`), so that does no harm.

What the guest is, as far as a package's C and C++ care:

- wasm32: 32-bit pointers and `long`, `long double` IEEE binary128 (in software); no SIMD here;
- Linux, musl, with `dlopen` (of WebAssembly side modules: `.so` files built here); but no
  `fork` (`posix_spawn` works), no `mmap` (`<sys/mman.h>` declares none: a library that maps files
  needs its stdio path, as FreeType did); the floating-point exception flags are the C library's,
  in software: what code raises with `feraiseexcept` it finds with `fetestexcept` (numpy's
  singular matrices, its integer division), but the arithmetic itself raises none (WebAssembly
  has no flags: a float division by zero is not seen);
- threads (pthreads) and thread-local storage work; so do C++ exceptions and `setjmp`/`longjmp`
  (WebAssembly exception handling); an exception nothing catches ends the program, with the
  engine's `addons/mod/0016`;
- every call through a function pointer is checked against the function's type: one of another
  type traps (ends the program), where natively a mismatch in the result often goes unnoticed;
- libffi is there, in cffi's module (`cffi/patches/libffi`: calls and callbacks of any C type, made
  as side modules it loads); a package that needs it links its own copy (`build/cffi/deps`).

A static library that needs another (libtiff needs libwebp and liblzma; FreeType needs zlib;
libxml2 needs libiconv) must carry it when the package links only the first: a shared library would have brought it along
(`pillow/build.sh` merges the archives with `llvm-ar qcL`). The libraries the interpreter has —
zlib, bzip2, liblzma, OpenSSL, SQLite, ncurses, libedit, in `python/deps` — are not for the
modules: the interpreter does not export them, and they are not PIC. A package that needs one
builds its own (`pillow/build.sh`: zlib and liblzma, the python step's versions); `wasm-pyext-cc`
stops a module's link that names the interpreter's.

## Adding one

1. `packages/<name>/build.sh` (and `patches/` for the source's patches, applied in order);
2. its downloads in `../sources.lock`, under a `# packages/<name>` line;
3. `<name>` in `PACKAGES` in `../build.sh` (after those its build uses), and in `RUST` if it has Rust;
4. `tests/checks/<name>_check.py`, which prints what the package computes — the same with its
   PyPI wheel on this machine (`tests/reference.py <name>_check` writes `tests/expected/`) as in
   the guest; the wheels it needs in `PACKAGES` of `tests/reference.py` — and its modules in
   `MODULES` of `tests/test_guest.py`;
5. its own test suite, if it can run in the guest, in `SUITES` of `tests/upstream.py` (and its
   known failures, with their reasons, in `tests/upstream-known.txt`);
6. a row in `../README.md`.
