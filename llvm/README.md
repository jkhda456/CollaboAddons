# llvm add-on

The LLVM toolchain in the sandbox, and what it takes to build the guest's own programs and shared
libraries there: Clang, the LLVM IR tools, the MC layer, LLD, `llvm-ar` and the object-file tools
as wasm32 Linux programs (LLVM 24.0.0git, statically linked), `cc`/`c++`, and a sysroot — the C
library, libc++ with exceptions, compiler-rt — in the guest's own `/usr`.

```sh
cc hello.c && ./a.out                          # a program
c++ -std=c++20 -O2 app.cpp -o app && ./app     # libc++, exceptions, threads, std::format
cc -shared lib.c -o lib.so                     # a library dlopen loads (a WebAssembly side module)
cc -rdynamic main.c -o main                    # a program that loads them
cc -c a.c b.c && ar rcs libab.a a.o b.o        # archives, nm, strip, objdump, ...
```

## What it adds

| In the guest | |
|---|---|
| `clang`, `clang++` | C/C++/Objective-C compiler (driver + cc1 in one process) |
| `cc`, `c++` | the guest's C and C++ compiler: clang as `userspace/bin/wasm-cc` builds the system's programs (a script, `guest/cc`; see below) |
| `cpp` | the C preprocessor (`cc -E`) |
| `opt`, `llc`, `llvm-mc` | LLVM IR optimizer; IR → assembly / object; assembler / disassembler |
| `lld`, `ld.lld`, `wasm-ld`, `ld` | linker: WebAssembly, ELF (and Mach-O / COFF via `lld -flavor`); `ld` is `wasm-ld` (a script: lld run as `ld` would be its ELF linker) |
| `llvm-ar`, `llvm-ranlib`, `ar`, `ranlib` | archives |
| `llvm-nm`, `nm` | symbol tables |
| `llvm-objcopy`, `llvm-strip`, `objcopy`, `strip` | copy / transform / strip object files |
| `llvm-objdump`, `objdump` | disassembler / object dumper |
| `llvm-readobj`, `llvm-readelf`, `readelf` | object-file headers, sections, symbols |
| `llvm-dwarfdump` | DWARF debug info |
| `llvm-symbolizer`, `llvm-addr2line`, `addr2line` | address → function, file, line |
| `/usr/include` | the C library's headers (musl, with Linux's uapi headers), libc++'s (`c++/v1`) |
| `/usr/lib` | `libc.a`, `crt1.o`, `libc.exports` (the userspace step's C library, with dlopen); `libc++.a`, `libc++abi.a`, `libunwind.a` (with C++ exceptions, position-independent); empty `libm.a`, `libpthread.a`, `libdl.a`, ... |
| `/usr/lib/clang/24` | clang's own headers; compiler-rt's builtins, and the copy `-shared` links |

Targets: **WebAssembly, X86, AArch64, ARM, RISCV**. Clang's default target triple is
`wasm32-unknown-linux-musl`; give `--target=` (`-mtriple=`, `-triple=`) for the others (objects,
IR and assembly: there is a sysroot for the guest only).

## Turning it on

```json
"addons": {"llvm": true}
```

or `--addon-dir app/images/addons --addon llvm` on the engine's command line. It has no
settings and needs no network.

## cc and c++

What the host's `userspace/bin/wasm-cc` does for the system's programs, as a script of the guest's
(`/bin/sh`; `c++` is `cc` by another name, which adds libc++):

- **compile**: `--target=wasm32-unknown-unknown --sysroot=/usr` with Linux's macros (`__linux__`,
  `__unix__`, `__gnu_linux__`), atomics and bulk memory (a program's memory is shared: threads),
  setjmp/longjmp and C++ exceptions as WebAssembly exceptions (`-mllvm -wasm-enable-sjlj
  -fwasm-exceptions`) **in the standard exnref encoding** (`-mllvm -wasm-use-legacy-eh=false`: the
  engine runs no other), position-independent with default visibility, as on Linux (any object
  can go into a shared library, which exports what it does not hide). Headers: libc++'s, clang's,
  `/usr/include`, then `/usr/local/include`.
- **a program**: `crt1.o`, the C library, compiler-rt (C++: libc++, libc++abi, libunwind first);
  its memory imported and shared (`--import-memory --shared-memory --max-memory=4294967296`), as
  the kernel runs every program; the function table from 3 (function pointers 1 and 2 are SIG_IGN
  and SIG_HOLD), exported; 8 MiB of stack.
- **`-shared`**: a library the C library's dlopen loads (`addons/mod/0019`–`0020`, upstream since
  0.2.4): a WebAssembly side module (`wasm-ld --experimental-pic -shared`), its own symbols bound to
  itself (`-Bsymbolic`), what it does not define imported from the program or the libraries loaded
  before it, its constructors in `__wasm_call_ctors` and its thread-local storage set up by
  `__wasm_init_tls` (both exported: the loader calls them); compiler-rt in it without its
  `__c_longjmp` (a library's setjmp must catch the program's tag), libc++ too for C++ (its own
  copy); not the C library.
- **`-rdynamic`**: a program that loads them: the whole C library in it and exported
  (`/usr/lib/libc.exports`), everything else not hidden exported, a function table that can grow.
- **`-r`**: a partial link, objects only.
- **every link** passes the compile's exception options to the linker too
  (`-Wl,-mllvm,-exception-model=wasm`, `-wasm-use-legacy-eh=false`, `-wasm-enable-sjlj`): with
  `-flto` (pybind11's modules in Release, CMake's `INTERPROCEDURAL_OPTIMIZATION`) the objects are
  bitcode, compiled in wasm-ld, which takes none of the compile's options — without them a C++
  exception went uncaught and `longjmp` trapped.

Build systems for Linux pass GNU ld's options, which wasm-ld lacks: `cc` leaves them out
(`--as-needed`, `--no-undefined`, `--version-script`, `--dynamic-list`, `--exclude-libs`,
`--start-group`, `-z relro`/`now`/`defs`, `--hash-style`, `--eh-frame-hdr`, `-rpath-link`, ... —
`-z stack-size` stays), and a program's table options in a library's link: Python's `LDSHARED`,
`cc -shared` with the interpreter's `LDFLAGS`, has `--export-table`. `-lstdc++`, `-lgcc`,
`-lgcc_s`, `-latomic` name what is linked anyway; `-static`, `-pie`, `-fuse-ld=` are dropped.
`cc -dumpmachine` says `wasm32-unknown-linux-musl`; `-print-libgcc-file-name` names compiler-rt.

The guest's Python is built for this: its `sysconfig` gives `CC=cc`, `LDSHARED=cc -shared ...`,
`CCSHARED=-fPIC`, `AR=ar` and `INCLUDEPY=/usr/include/python3.13`, so setuptools builds an
extension module with `cc` (the interpreter's headers are not in its image: the python-extension
add-on's part).

## Limits

- **WebAssembly programs only.** There is no sysroot for the other targets, and no JIT: nothing
  can be made executable here (no MMU), so `lli` and ORC are not included, and
  `Memory::allocateMappedMemory` refuses `MF_EXEC`.
- **No programs linked against shared libraries.** A program loads them with `dlopen`; `-shared`
  libraries may name others they need (DT_NEEDED), but a program's link takes archives only.
- **C++ exceptions stay inside a module.** Each program and library has its own copy of libc++,
  libc++abi and libunwind, and with it its own exception tag: an exception a library lets out is
  not caught in the program (or another library), not even by `catch (...)`, and the engine ends
  the program (`a guest program threw an exception nothing caught`). Extension modules (pybind11,
  nanobind, Cython) turn exceptions into Python's inside the module.
- **Size.** The image is 474 MB (clang 117 MB, lld 79 MB, opt and llc 72 MB each, the other
  tools 5–21 MB, headers 27 MB, libraries 21 MB). The engine of collaboCore 0.2.9 and later serves
  it read-only (virtio-fs under an overlay of `/usr`): a machine with it boots using some 40 MB of
  its memory, and only what is read comes in. Older engines unpack the whole image into the
  guest's memory at boot.

## Building

```sh
./build.sh tools userspace          # cmake/ninja, the musl sysroot (with dlopen), compiler-rt
addons/llvm/build.sh                # or python3 addons/build.py build llvm
addons/llvm/tests/run.sh            # the checks, in the guest
```

The host needs clang-19 and lld-19, and the llvm-project clone at `../org/llvm-project` (beside
`CollaboCore`) with `LLVM_REV` in it — a shallow fetch of that one commit is enough, nothing is
checked out:

```sh
git init ../org/llvm-project
git -C ../org/llvm-project fetch --depth 1 https://github.com/llvm/llvm-project.git <LLVM_REV>
```

Stages, each skipped when up to date (redo one with `FORCE=stage[,stage]`); logs are
`build/<stage>.log`:

| Stage | |
|---|---|
| `src` | `llvm`, `clang`, `lld`, `libunwind`, `libcxxabi`, `libcxx`, `runtimes` (+ `libc`'s shared headers) at `LLVM_REV`, exported from the clone without their tests, with `patches/*.patch` applied. The clone itself is never modified |
| `native` | for the build machine, built by clang-19: `llvm-tblgen`, `clang-tblgen` (the cross build runs them), and clang, lld, `llvm-ar`, `llvm-objcopy`, `llvm-objdump` for WebAssembly — **the compiler of every stage after it**. clang-19 cannot be: it writes WebAssembly exceptions (C++'s, setjmp's) only in the withdrawn legacy encoding, which the engine does not run, and trunk's libc++ wants one of the two newest clangs |
| `runtimes` | libunwind, libc++abi and libc++ for the guest: `-fwasm-exceptions`, `-fPIC`, static; checked to hold no legacy exception instruction |
| `cross` | the tools, cross-compiled with the flags `userspace/bin/wasm-cc` uses (`-matomics -mbulk-memory -mllvm -wasm-enable-sjlj`, the Linux macros, shared memory, table base 3) and `-mllvm -wasm-use-legacy-eh=false`, against those runtimes (`-fno-exceptions` code) |
| `cpio` | strip; the names; `guest/cc`, `cpp`, `ld`; the sysroot (`userspace/sysroot`, the runtimes, clang's headers, `userspace/out/rt`'s compiler-rt and its copy without `__c_longjmp`); pack `out/llvm.cpio` |

`native` and `cross` are the long stages (about 3,300 and 3,700 build steps); the cross build's
links run one at a time to keep memory down. `JOBS=N` and `NINJA_FLAGS="-k 0"` (keep going, to see
every error at once) are passed to their ninja. A change to the cross build's configuration
reconfigures its tree in place (`build/cross.config`).

`addons/llvm/build.sh dist [ARCHIVE]` builds nothing: it packs the code and `out/` — not `build/`
(gigabytes) — into `dist/llvm-addon-YYYYMMDD.tar.gz` (about 120 MB), as `llvm/` with the add-ons'
`LICENSE` beside it. Unpacked in `addons/`, it is a built add-on, which the steps above rebuild.
It warns when `guest/`, `patches/` or `cmake/` changed after `out/llvm.cpio` was packed.

## Checks

`tests/run.sh` boots the packaged runtime (`dist/runtime/collabo-core-<platform>`,
`COLLABO_RUNTIME`) with `out/llvm.cpio` and runs `tests/guest.sh` in one machine: every tool name;
`cc hello.c && ./a.out`; C (setjmp/longjmp, threads and thread-local storage, the math library,
`long double`, `__int128`, 64 MiB of heap, files); C++ (exceptions through frames with destructors,
rethrow, `exception_ptr` across threads, RTTI, static constructors, threads, `std::filesystem`,
`std::format`); `cc -shared` and `c++ -shared` libraries loaded by a `cc -rdynamic` program
(constructors, the program's C library and setjmp, thread-local storage, function pointers,
libc++ and an exception thrown and caught in a library); a library linked with GNU ld's options;
`ar`/`ranlib`/`nm`/`strip`/`objdump`/`cc -r`; `main(argc, argv, envp)` (the C library's
`userspace/patches/musl-main-envp.patch`); C++ exceptions and setjmp/longjmp through `-flto` and
`-flto=thin`; `strip` of shared libraries that still load; the compiler's answers to build systems
(`-dumpmachine`, `-print-libgcc-file-name`, the predefined macros, `cpp`, a program from standard
input, `cc -Wl,--version` as meson asks, `c++ -v`). Then `tests/pyext.sh`, with the Python image
and the interpreter's headers (`python/stage`) mounted at `/usr/include/python3.13`: a C extension
module compiled and linked with the interpreter's own `CC`, `CFLAGS`, `CCSHARED` and `LDSHARED`, as
setuptools does, and imported.

## Patches

What this platform lacks, in `patches/` (applied in order to the exported source):

| Patch | |
|---|---|
| `0001-support-no-fork-on-wasm` | no `fork()` without an MMU: programs are started with `posix_spawn` only (the clang driver → `wasm-ld` works) |
| `0002-support-no-mmap-on-wasm` | no `mmap`: file mappings report "not supported" so files are read instead; "mapped" memory is page-aligned heap memory (lld's output buffers use it) |
| `0003-clang-bytecode-no-musttail-without-wasm-tail-call` | the constant interpreter's `[[clang::musttail]]` needs the wasm tail-call feature; turned off as on PPC/i386 |
| `0004-lld-macho-no-madvise-on-wasm` | lld's Mach-O page-in uses `madvise` |
| `0005-objcopy-wasm-strip-keeps-dylink` | `strip`/`llvm-strip` (`--strip-all`) keeps only the sections the engine reads, and so removed a shared library's `dylink.0`, which the C library's dlopen reads: a stripped library (CMake's `install/strip`, pybind11's modules, which it strips) no longer loaded ("no dylink.0 section"). Kept, as ELF's `.dynamic` is |

To change one: edit `build/src/...`, rebuild with `addons/llvm/build.sh` (incremental), then
regenerate the patch against the pinned revision, e.g.

```sh
git -C ../org/llvm-project show $LLVM_REV:llvm/lib/Support/Unix/Path.inc > /tmp/a
diff -u --label a/llvm/lib/Support/Unix/Path.inc --label b/llvm/lib/Support/Unix/Path.inc \
  /tmp/a build/src/llvm/lib/Support/Unix/Path.inc > patches/NNNN-name.patch
touch build/src/.stamp     # the source tree already has it; skip re-exporting
```

## Licenses

LLVM, Clang, LLD, libc++, libc++abi, libunwind and compiler-rt: Apache-2.0 WITH LLVM-exception;
musl (`libc.a`, its headers): MIT; Linux's uapi headers: GPL-2.0 WITH Linux-syscall-note
(`out/licenses/`).
