# build-tools add-on

The programs that drive a build, in the sandbox: **GNU make** 4.4.1, **ninja** 1.13.2,
**pkgconf** 3.0.10 and **CMake** 4.4.5, built for the guest as static programs. The guest has no
`fork()`: make and ninja start their commands with `posix_spawn` (make's `USE_POSIX_SPAWN`, ninja's
own `subprocess-posix.cc`), and CMake does too, with its libuv and KWSys patched to (below) — so
they run as they do on Linux: parallel jobs, make's jobserver (which ninja joins), `$(shell)`,
recursive make, `execute_process`, `cmake --build`, ctest. Adds to `/usr/bin`:

| Program | |
|---|---|
| `make`, `gmake` | GNU make: parallel jobs (`-j`) with the jobserver (a fifo, `--jobserver-auth=fifo:`), `$(shell)`, recursive `$(MAKE)`, makefiles remade and read again, output sync (`-O`), a recipe that is a script without `#!` (spawned, then run by `/bin/sh`), `.ONESHELL` |
| `ninja` | ninja: depfiles (`deps = gcc`), pools, restat, generator rules (a regenerated manifest is read again), a jobserver client under make, its tools (`-t targets`, `-t compdb`, `-t deps`, `-t clean`, ...) |
| `pkgconf`, `pkg-config` | pkgconf, as pkg-config too: `--cflags`, `--libs`, `--static`, `Requires`, versions, variables, `--define-prefix`; its search path `/usr/lib/pkgconfig:/usr/share/pkgconfig`, `/usr/include` and `/usr/lib` the system's (left out of the flags, as pkg-config does) |
| `patchelf` | for WebAssembly files, which have no run path, soname or interpreter: `--print-rpath` (and `--print-needed`, `--print-soname`) print nothing, `--set-rpath` and the other edits leave the file as it is, anything not WebAssembly is refused. meson-python fixes the run paths of what it installs with it (without one on `PATH` it asks pip to build patchelf, which fails) |
| `/usr/share/config.site` | what a `configure` cannot find out here (autoconf reads it as `$prefix/share/config.site`, `/usr/local/share` linked to it): the build machine, `wasm32-unknown-linux-musl` (`uname -m` says `wasm`, which no `config.guess` knows); static libraries unless `--enable-shared` (a program here loads libraries with `dlopen` only, so libtool's link of a project's programs to its shared libraries fails; the objects are position-independent, so a static library still goes into an extension module); and no `fork`, `vfork`, `mmap` of files or `sigaltstack` |
| `cmake`, `ctest` | CMake with its Ninja and Unix Makefiles generators (and the others it has), `execute_process` (pipelines, `WORKING_DIRECTORY`, `TIMEOUT`), `cmake --build`, `cmake -P`, `cmake -E` (`tar` with gzip, bzip2, xz and zstd: libarchive; `sha256sum`, `env`, ...), `file(DOWNLOAD)` over HTTP and HTTPS (curl on OpenSSL 4.0, verified against the guest's CA bundle, through the sandbox's network policy), `cmake --help-*` (its documentation is in the image); ctest runs a project's tests. Its modules in `/usr/share/cmake-4.4` |

The add-on's image is 44 MB: CMake is most of it (cmake 15.6 MB, ctest 16.8 MB, its modules and
documentation 23 MB); make 254 KB, ninja 0.5 MB, pkgconf 126 KB. A program is compiled when a
machine first runs it (CMake's first start takes about 15 s here) and then kept compiled by the
engine's cache of programs.

The compilers are not here: they are the [llvm add-on](../llvm)'s (`clang`, `wasm-ld`), and the
headers and libraries a C program needs (a sysroot, `cc`, `ar`) are still to come there. Until
then make and ninja drive what the guest has: scripts, Python, perl (the tools image), and
`clang` for freestanding code. With the [python-extension add-on](../python-extension), whose
image has the interpreter's headers, `pkg-config --cflags python3` says `-I/usr/include/python3.13`.

## Turning it on

```json
"addons": {"build-tools": true}
```

or `--addon-dir addons/build-tools/out --addon build-tools` on the engine's command line. It has
no settings and needs no network. With the other two:

```json
"addons": {"build-tools": true, "llvm": true, "python-extension": true}
```

```sh
cd /tmp && mkdir p && cd p
printf 'int sq(int x) { return x * x; }\n' > sq.c
cat > Makefile <<'EOF'
lib.wasm: sq.o
	wasm-ld --no-entry --export=sq -o $@ $^
%.o: %.c
	clang --target=wasm32 -O2 -nostdlib -MMD -c $< -o $@
-include sq.d
EOF
make -j4 && llvm-nm lib.wasm          # with the llvm add-on
pkg-config --cflags python3            # with python-extension: -I/usr/include/python3.13
```

## Limits

- **No compiler, no `ar`** without the llvm add-on (`cc`, `c++`, `ar` and the guest's sysroot are
  its).
- **`configure` runs with `/bin/sh`**, busybox's hush, to which collaboCore added what autoconf's
  scripts and libtool ask of a shell (`userspace/patches/busybox-hush-posix-scripts.patch`;
  `help` in the shell lists it): `[n]>&WORD` with the word expanded when run (configure's
  `as_fn_error` writes `>&$4`; without it hush could not even read a configure), `exec 5<&0`,
  an unquoted expansion in a `case` pattern as a pattern, `${VAR="WORD"}` assigning the word as
  it reads (libtool's `: ${MV="mv -f"}`), `( ... )` and a pipeline's functions run in a cloned
  child rather than a shell started again with its whole state in `argv` (libtool's
  `( cd ... && ... )` went past what `execve` takes), `set -u`, and a function definition across
  backslash-newlines (automake's test harness: `result_count () \` then `{ \`). hush is not
  bash: scripts that need bash's arrays, `declare`, `((...))`, `<<<`, `<(...)` and the like do
  not run. GNU make 4.4.1 builds from its tarball with `./configure && make` in the guest
  (configure in some 45 s), and the result is what bash's configure gives (`config.h` and its
  197 checks the same). So do zlib (`--static`: its own configure, not autoconf's, makes a
  shared library by default), xz (and its `make check`: 19 tests), pkgconf, libtiff, libiconv
  and ncurses (`--without-tests`: its test programs' `ditto` forks).
- **What forks does not run**: a build that starts processes with `fork` (sqlite's autosetup
  builds and runs `jimsh`, which does) fails; the C library has no `fork` or `vfork`.
- **Not in CMake**: `cpack` (left out of the image), `ccmake` (curses) and `cmake-gui` (Qt); a
  detached `exec_program`/KWSys process (it takes a second fork).
- **No sockets or FIFOs in a shared folder.** The engine's shared folders (`/work`, the app's
  mounts) cannot make them (virtio-fs `MKNOD`): `bind` of a Unix socket or `mkfifo` there fails with
  `ENOSYS`. The guest's own folders (`/tmp`) have both — make's jobserver fifo is in `/tmp`.
- **`ninja -t browse`** is left out: it forks a Python web server.
- **make's `load`** (loading a shared object into make) and **Guile** are left out: programs here
  do not `dlopen`.
- **Guest's root.** The guest runs as root, so what tests a directory's permissions by writing to it
  finds it writable (two of make's tests, below).

## Building

```sh
python3 addons/build.py build build-tools    # or addons/build-tools/build.sh
python3 addons/build.py test build-tools     # tests/run.sh; --upstream: their own suites too
```

It needs the userspace step (musl, compiler-rt: `userspace/sysroot`), the python step (OpenSSL for
the guest, `python/deps`: CMake's curl) and cmake and ninja (`./build.sh tools userspace python`). From scratch it takes about 30 minutes here (2 cores; 25 of
them CMake's; a few more with `TEST=1`). Stages (`FORCE=stage,...` redoes them, `JOBS=N`; logs are `build/<stage>.log`):

| Stage | |
|---|---|
| `src-make`, `src-ninja`, `src-pkgconf`, `src-cmake` | the release archives in `sources.lock`, unpacked into `build/src/<name>` with `patches/<name>/*.patch`: a git repository whose first commit is the pristine source (`git diff` there regenerates a patch) |
| `libcxx` | libc++, libc++abi and libunwind of LLVM 19.1.7 for wasm32, with C++ exceptions (WebAssembly exception handling, which the engine runs; CMake throws and catches), as the python-extension add-on builds them but not PIC |
| `make` | configure (`python/config.site`: no `fork`) and make; stops if configure did not take `posix_spawn` |
| `ninja` | CMake; `HAVE_FORK=0` leaves `-t browse` out |
| `pkgconf` | configure, static, with the guest's search path and system directories |
| `cmake` | CMake cross-built by this machine's CMake (the longest stage: about 1,060 steps, half an hour here), with its own copies of curl (on python/deps' OpenSSL), libarchive, expat, jsoncpp, zlib, zstd, xz, bzip2, nghttp2, rhash and libuv; looking for headers and libraries only in the guest's sysroot and python/deps (`CMAKE_FIND_ROOT_PATH`); installed into `build/cmake-install` |
| `cpio` | `out/build-tools.cpio`, `out/licenses/` |

With `TEST=1` the build also makes what `tests/run.sh --upstream` runs, in `build/tests`:
`ninja_test` (on googletest 1.16.0, which ninja's CMake pins: `src-googletest`) and pkgconf's
`test-runner`, the tools its tests run (`bomtool`, `spdxtool`, `pccritic`) and its API tests; and
CMake is built with its tests (`BUILD_TESTING`), which stay in `build/cmake`.

The compilers of the build are `tools/wasm-bt-cc` and `tools/wasm-bt-c++`: `userspace/bin/wasm-cc`,
with libc++ for C++ (`-fwasm-exceptions` first, so a project's `-fno-exceptions` still wins) and
the guest's link flags for programs. (make's `main(argc, argv, envp)`, which clang-19 does not name
as the C library's start code calls it, is the C library's: `userspace/patches/musl-main-envp.patch`.)

## Patches

| Patch | |
|---|---|
| `make/0001-tests-without-fork` | the test suite's driver (`tests/test_driver.pl`) and `functions/shell` start programs with `fork`, which the guest's perl has not: with `system(1, LIST)` there, as on Windows (spawned without waiting, the pid returned). Tests only |
| `make/0002-close-stdout-without-exit-in-atexit` | make's `close_stdout`, an `atexit` handler, calls `exit` on a write error — inside `exit`, which musl 1.2.5 traps (`a_crash`): `make > /dev/full` ended as a trap. `_exit`, as gnulib's `close_stdout` does |
| `googletest/0001-no-death-tests-on-wasm` | googletest's death tests (`fork`, `mmap` of a stack) left out on wasm; ninja's tests have none |
| `cmake/0001-libuv-posix-spawn-without-fork` | every process CMake starts goes through its libuv's `uv_spawn` (`cmUVProcessChain`), which forks: on the guest it takes libuv's own `posix_spawn` path, macOS's, without macOS's extensions — `posix_spawn_file_actions_addchdir_np` called directly, a descriptor `dup2`'d onto itself where macOS has `addinherit_np` (it clears close-on-exec), `POSIX_SPAWN_SETSID` always there, and `loop->cloexec_lock` held as around a fork (no `POSIX_SPAWN_CLOEXEC_DEFAULT`); the fork path left out |
| `cmake/0002-kwsys-process-posix-spawn-without-fork` | KWSys' Process (`exec_program()`) the same: the child's setup before its `execvp` — stdin, stdout, stderr, default signal handlers, the signal mask, `setsid` — as `posix_spawn`'s file actions and attributes, and a failed start reported as a failed exec is (and with no pid left behind for the cleanup to kill) |
| `cmake/0003-libuv-no-io-uring-on-wasm` | libuv's io_uring left out on wasm (the guest's kernel has none, and its rings are `mmap`ed): epoll, as on a kernel without it |
| `cmake/0004-libuv-thread-start-routine-type-on-wasm` | libuv starts its threads (its thread pool: asynchronous `uv_fs_*`, `uv_getaddrinfo`, `uv_queue_work`) with a `void (*)(void*)` cast to the `void* (*)(void*)` `pthread_create` calls: WebAssembly checks the type of every call through a pointer, and the new thread trapped at its start. A start routine of that type, which calls it |
| `cmake/0005-linux-wasm-no-run-path` | CMake's Linux platform on the guest (`CMAKE_SYSTEM_PROCESSOR` is `wasm`): no run path flags (WebAssembly has no run path; with them, a target installed by the Ninja generator "requires changing an RPATH", which it can do on ELF only), and `ENABLE_EXPORTS` as `-rdynamic`, which exports the C library with the program's symbols for the libraries it loads |

Also for builds, in the base image's busybox (`userspace/patches/`): `diff -c` and `-C N`, the
context format, as GNU diff prints it (`busybox-diff-context.patch`; make's test driver shows its
differences with it), and a file without `#!` run as a shell script by hush, `env`, `xargs`,
`find -exec`, ... (`busybox-exec-script.patch`: musl's `execvp` fails with `ENOEXEC` where
POSIX, and glibc, run `/bin/sh` with it). `tests/test_busybox_tools.py` checks both.

## Tests

`tests/run.sh` boots the packaged runtime (`dist/runtime/collabo-core-<platform>`) with the add-on
and checks make (four 1 s jobs under `-j4` in at most 3 s, the jobserver's fifo in a sub-make's
`MAKEFLAGS`, `$(shell)`, a remade makefile, `-Otarget`, a script without `#!` as a recipe, a failing
recipe, a write error at exit, `touch` then rebuild), ninja (a depfile's deps and a rebuild after
the header changed, a pool of depth 1 against `-j3`, its tools, a failing command, a regenerated
manifest, three jobs at once under `make -j3`), pkg-config (flags with `Requires`, `--static`,
versions, variables, a missing package, the search path, the system's directories) and CMake (its
generators; `execute_process`'s output, errors and status, in a working directory, a pipeline of
three, the environment, a missing program, a timeout; `-G Ninja` and `-G 'Unix Makefiles'` built,
up to date and rebuilt after an input changed; ctest; `cmake -E tar` with gzip, xz and zstd,
`sha256sum`, `env`, ...; `file(DOWNLOAD)` from an HTTP server on the host, its hash checked). All
of it in `/work`, the shared folder: make sees nothing "in the future" there. And `patchelf` on
WebAssembly and other files, and `config.site`. When their
images are built it adds the python-extension add-on (the headers where `sysconfig` and
`pkg-config` say, numpy's, a `.pc` without build paths) and the llvm add-on (make and ninja
building a wasm library with `clang -MMD`/`-MD` and `wasm-ld`, rebuilt after a header changed; a
CMake project in C and C++ — the compilers and try_compile's checks, a static library, a shared one
a program loads with dlopen, exceptions and threads, ctest, install — with Ninja and with make).

`tests/run.sh --upstream` then runs their own suites in the guest (`TEST=1 build.sh` first; about
3.5 minutes):

| Suite | Passed | Failed |
|---|---|---|
| ninja (`ninja_test`: googletest, `SubprocessTest` and the jobserver's included) | 410 | none |
| pkgconf (`test-runner` over `t/*`, 14 groups; its 16 API test programs) | 423 + 222 | none (4 skipped: Windows only) |
| CMake (`Tests/CMakeLib` and KWSys' tests, by the guest's ctest in `build/cmake`, mounted where it was built; not KWSys' DynamicLoader test: these programs load no libraries) | 62 of 67, `testUVProcessChain` and 6 `kwsys.testProcess-*` among them | 5, all known (`tests/upstream-known.txt`): `kwsys.testProcess-4` and `-5` (a child crashes by writing to address 4, which in WebAssembly's memory is memory: no fault), the debugger's three pipe tests (a Unix socket bound in the build tree, a shared folder: above) |
| GNU make (`tests/run_make_tests.pl`, perl from the tools image) | 128 scripts | 18 tests in 4 scripts, all known (`tests/upstream-known.txt`): `features/archives` (no `ar` in the guest yet), `features/output-sync` and `features/temp_stdin` (an unwritable `TMPDIR`, which root writes to), `functions/shell` (the shell's message `can't execute 'bad-command'` has quotes, which break the test's own `echo '...'`) |

## Licenses

GNU make: GPL-3.0-or-later; ninja: Apache-2.0; pkgconf: ISC-style (its `COPYING`); CMake:
BSD-3-Clause, with the licenses of what it carries (curl, libarchive, expat, jsoncpp, zlib, zstd,
liblzma, bzip2, nghttp2, librhash, libuv, cppdap) and OpenSSL (Apache-2.0); libc++, libc++abi and
libunwind: Apache-2.0 WITH LLVM-exception. All in `out/licenses/`.
