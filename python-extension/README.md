# python-extension add-on

More of Python in the sandbox: packages whose extension modules are compiled code (C, C++, Rust),
built for the guest as shared libraries that its CPython 3.13 loads as it does on Linux — `import numpy` opens
`numpy/_core/_multiarray_umath.cpython-313.so` with `dlopen` — installed into its site-packages.
With it on, `python3` has:

| Package | Version | What it can do in the guest | What it cannot |
|---|---|---|---|
| [numpy](https://numpy.org) | 2.5.4 | all of numpy: arrays, ufuncs, casting, sorting, `np.strings` and `StringDType`, datetimes, `linalg` (with its `LinAlgError`s), `fft`, `random` (every bit generator, `Generator`, the legacy `RandomState`), `polynomial`, `ma`, `f2py`'s Python side, `numpy-config`, the test modules its suite needs | no BLAS/LAPACK library: `linalg` and matrix products run on numpy's own fallbacks (lapack_lite), at plain C speed; no SIMD; WebAssembly has no floating-point exception flags, so what the hardware would flag — a float division by zero, an overflow, an invalid result — raises no `FloatingPointError` and warns nothing (what numpy raises itself does: singular matrices, integer division by zero). `arr.ctypes` and `numpy.ctypeslib` work with the add-on's ctypes (below) |
| [Pillow](https://python-pillow.org) | 12.3.0 | JPEG (libjpeg-turbo 3.1), JPEG 2000 (openjpeg 2.5), PNG, TIFF (libtiff 4.7: LZW, Deflate, JPEG, LZMA, WebP), WebP and animated WebP (libwebp 1.6), GIF, BMP, ICO, TGA, QOI, PDF and the rest of Pillow's formats; drawing, resampling, filters; `ImageFont` with FreeType 2.14 (TrueType, OpenType, `load_default()`); `ImageCms` (LittleCMS 2.19); `ImageMath`, `ImageMorph`; numpy arrays both ways | no raqm (HarfBuzz/FriBiDi text shaping: Arabic and Indic scripts, ligatures; the basic layout does Latin, Hangul and CJK); no libimagequant (GPL-3; Pillow's own quantizers remain); no AVIF; no XCB (`ImageGrab`), no Tk |
| [LIEF](https://lief.re) | 1.0.0 | parsing, changing and writing ELF, PE (imports, exports, resources, Authenticode signatures and authentihashes, mbedtls), COFF, Mach-O (fat binaries, dyld info) and the Android formats (DEX, OAT, VDEX, ART); the abstract `lief.parse` layer; JSON export | what LIEF's own wheels lack too: the extended (closed-source) features — DWARF/PDB, Objective-C metadata, dyld shared caches, the disassembler and assembler |
| [lxml](https://lxml.de) | 6.1.3 | all of lxml, with the libraries its own wheels carry (libxml2 2.14.6, libxslt 1.1.43, GNU libiconv): `lxml.etree` — XML and HTML parsing and serializing in every encoding libiconv converts, the ElementTree API, XPath 1.0 with EXSLT and Python extension functions, XSLT 1.0 with EXSLT, DTD, XML Schema, RelaxNG and Schematron validation, C14N 1.0 and 2.0, XInclude, catalogs and resolvers, iterparse, feed and pull parsers, `xmlfile`, gzip'ed documents; `lxml.objectify`, `lxml.html` (forms, links, `htmldiff`), `lxml.builder`, `lxml.sax`, `lxml.isoschematron` | what lxml's own wheels lack too: libxml2 loads nothing over HTTP or FTP itself (Python's own HTTP does), no lxml.html.clean (now the separate, pure-Python `lxml_html_clean`), `lxml.cssselect` and `lxml.html.html5parser` only with their pure-Python packages (`cssselect`, `html5lib`), no libxslt crypto functions; and no libxml2 of its own to make documents for `adopt_external_document` with (ctypes is here; libxml2 is inside `lxml.etree`, as in its wheels) |
| [cffi](https://cffi.readthedocs.io) | 2.1.1 | calling C from Python: ABI mode (`ffi.dlopen(None)`: the C library and everything else the interpreter exports; side modules by path), out-of-line ABI mode (`ffi.compile` writes a pure-Python module), every C type cffi has, structs and unions by value, variadic calls, `long double`, `_Complex`, callbacks (`ffi.callback`, as many as wanted, of any type, from any thread), handles, `ffi.gc`, buffers, `errno`; on libffi 3.8 with a backend for this platform; with [pycparser](https://github.com/eliben/pycparser) 3.11 (its declarations' parser) | API mode (`ffi.set_source` with C, `ffi.verify`) needs a C compiler in the guest, the llvm add-on's (zstandard's `_cffi`, built by pip there, runs on it); also as wheels for pip, 1.17.1, 2.0.0 and 2.1.1 (below); a call through a function declared with the wrong type (natively often harmless: `void getenv(char *)`) ends the program here — WebAssembly checks the type of every call through a pointer |
| [cryptography](https://cryptography.io) | 50.0.2 | all of it, with the OpenSSL its own wheels carry (4.0.3, the legacy provider built in): hashes, MACs, KDFs (HKDF, PBKDF2, scrypt, Argon2id, ...), AES in every mode and the AEADs, ChaCha20-Poly1305, the legacy ciphers (`hazmat.decrepit`), RSA, DSA, ECDSA (RFC 6979 too), ECDH, Ed25519/Ed448, X25519/X448, DH, keys in PEM/DER/OpenSSH, SSH certificates, X.509 (build, sign, parse, verify chains), CSRs, CRLs, OCSP, PKCS#7, PKCS#12, Fernet, HOTP/TOTP | what its wheels lack too (no FIPS provider); encrypted OpenSSH private keys need `bcrypt`, a package of its own (not here) |
| [rpds-py](https://github.com/crate-py/rpds) | 2026.9.1 | the Rust `rpds` crate's persistent data structures: `HashTrieMap`, `HashTrieSet`, `List`, `Queue`, `Stack` (what jsonschema and referencing are built on) | nothing its wheels have (a map or set iterates in the order of its keys' hashes, 32-bit here: not that of a 64-bit wheel, as on any 32-bit platform) |
| [mcp](https://modelcontextprotocol.io) | 2.3.0 | the Model Context Protocol's Python SDK: `MCPServer` (tools with structured output, resources and templates, prompts, progress, logging) and `Client` — in process, over stdio (a server as a process of its own), over streamable HTTP and over SSE (the server's Starlette apps on uvicorn); with what it needs that the Python image has not, from their wheels: mcp-types, jsonschema 4.26 (and jsonschema-specifications, referencing), pyjwt 2.15 (`[crypto]`, on cryptography above), python-multipart, Starlette 1.9, sse-starlette, uvicorn 0.54 (pure Python: no uvloop, httptools or websockets), click, opentelemetry-api, attrs | the `cli` extra (`mcp dev`, `mcp run`, `mcp install`: typer, rich, python-dotenv), as `pip install mcp` leaves it out: the `mcp` command says so |
| [unicorn](https://www.unicorn-engine.org) | 2.1.4 | the Unicorn CPU emulator and its Python binding: x86 (16, 32 and 64-bit, x87 and SSE), ARM and Thumb, ARM64 (with SIMD and floating point), M68K, MIPS (32/64, either endianness), SPARC, PPC, RISC-V, S390X and TriCore; memory maps, protections, MMIO, every kind of hook (code, blocks, memory, unmapped memory, interrupts, IN/OUT, SYSCALL, CPUID, invalid instructions, TLB fills, TCG opcodes) as Python callbacks, contexts, exits and controls; the C headers in `unicorn/include` | no code generation for the host: WebAssembly cannot run code it writes, so TCG's interpreter runs the translated code (below): about 20 million instructions of a simple loop a second here, where the wheel, translating to x86-64, runs 470 million on this machine; not the wheel's `libunicorn.a`; hooks set from C must be of their hook's type (a memory-access hook's callback returns nothing), which the binding's are |

```
$ python3
>>> import numpy as np, lief
>>> from PIL import Image
>>> np.linalg.inv(np.array([[2.0, 1.0], [1.0, 3.0]]))
array([[ 0.6, -0.2],
       [-0.2,  0.4]])
>>> Image.open("photo.jpg").resize((320, 240)).save("small.webp")
>>> elf = lief.parse("elf-x86_64-static")          # tests/samples
>>> [s.name for s in elf.sections][:4], hex(elf.entrypoint)
(['', '.rodata', '.eh_frame_hdr', '.eh_frame'], '0x201250')
>>> from lxml import etree
>>> etree.fromstring("<a><b n='1'/><b n='2'/></a>").xpath("sum(//b/@n)")
3.0
>>> from cryptography.fernet import Fernet
>>> f = Fernet(Fernet.generate_key()); f.decrypt(f.encrypt(b"secret"))
b'secret'
>>> import cffi; ffi = cffi.FFI(); ffi.cdef("size_t strlen(const char *);")
>>> ffi.dlopen(None).strlen(b"hello")
5
>>> from mcp.server import MCPServer
>>> from mcp import Client
>>> server = MCPServer("Demo")
>>> @server.tool()
... def add(a: int, b: int) -> int:
...     return a + b
...
>>> async def main():
...     async with Client(server) as client:          # or Client("http://host:8000/mcp"), Client(StdioServerParameters(...))
...         return (await client.call_tool("add", {"a": 1, "b": 2})).structured_content
...
>>> import asyncio; asyncio.run(main())
{'result': 3}
```

Every package prints, in the guest, what its own PyPI wheel prints on this machine for the same
work (`tests/`), and pip sees them as installed distributions (`pip list`, `pip show`; `pip install
numpy` says it is there already). Other packages that need a compiler pip builds in the guest from
their sources, with the llvm and build-tools add-ons on (`pip install zstandard`); those that need
more than that are added here, one folder each in `packages/` (`packages/README.md`).

The add-on also carries what a build of an extension module reads: the interpreter's headers in
`/usr/include/python3.13` (where `sysconfig.get_paths()["include"]` points) and its pkg-config
file (`python3.pc`: `pkg-config --cflags python3` is `-I/usr/include/python3.13`), as a
distribution's python3-dev has them; numpy's headers are in its package (`numpy.get_include()`).
They are for the guest's own compiler: the llvm add-on, driven by the build-tools add-on's make,
ninja and pkg-config.

And it carries **wheels for pip**: `/usr/share/python-wheels` has cffi 1.17.1, 2.0.0 and 2.1.1 —
the last release of each minor line from 1.17, the first that builds for CPython 3.13 — and
pycparser 3.11, which cffi needs; `/etc/pip.conf` points pip's `find-links` there. pip builds a
package in an isolated environment, with the build requirements its `pyproject.toml` names
installed afresh rather than the add-on's: zstandard asks for `cffi~=1.17` on Python < 3.14, and
pip would build cffi 1.17.1 from PyPI's sdist, which cannot work here (cffi allocates its callbacks
in executable memory, with `mmap`, and calls variadic functions as fixed ones: `packages/cffi`).
pip takes, for a requirement, the highest version the wheels or the index have, and of one version
the wheel: `pip install zstandard` builds zstandard on cffi 1.17.1, and its cffi module then runs
on the installed 2.1.1 (the C that cffi generates is the same for both). A virtual environment and
`pip install cffi==2.0.0` take them too, without the network. A requirement no wheel here
matches — a cffi newer than 2.1.1, before the add-on carries it — still gets PyPI's sdist, and
fails. site-packages has one cffi, 2.1.1: `pip install "zstandard[cffi]"` would replace it with
1.17.1, which cryptography (`cffi>=2.0.0`) does not take.

The wheels have **Unicorn**'s versions too: 1.0.3, 2.0.1.post1 and 2.1.4 (the installed one) — the
last release of each minor line, one of Unicorn 1 — for the packages that pin Unicorn to the line
their code was written for: pwntools asks for `unicorn>=2.0.1,!=2.1.3,!=2.1.4` and gets 2.0.1.post1,
angr's `unicorn` extra `==2.1.4`, a Unicorn 1 user `unicorn<2` and gets 1.0.3. PyPI's sdists cannot
build here (CMake knows no wasm32 host, and Unicorn has no TCG interpreter to run on: below), so
`/etc/pip.conf` has pip take Unicorn from wheels alone (`only-binary = unicorn`): without it, pip
would take PyPI's 2.1.2 sdist for pwntools, a higher version than the wheel's, and fail to build it.
A version no wheel has is then pip's "no matching distribution" with the versions there are —
speakeasy's `unicorn==1.0.2`, androidemu's `==2.1.2`; `pip install --only-binary :none: ...` lifts
the setting for one command. Each version runs what its own PyPI wheel runs, alike
(`tests/checks/unicorn_check.py`); pip installing one into site-packages replaces the 2.1.4 there.

And `collabo-config-site.pth` in site-packages: with the build-tools add-on's
`/usr/share/config.site` there, every Python process sets `CONFIG_SITE` to it (unless it is set
already), and so the builds pip starts, and the `configure` scripts they run, read it whatever
their `--prefix`. autoconf looks only in `$prefix/share` without it, and a package that builds the
library it carries with a prefix of its own — PyNaCl's libsodium, `--prefix <its build folder>` —
stopped at "cannot guess build type" (`uname -m` is `wasm`, which no `config.guess` knows). A
virtual environment without the system's site-packages does not read this `.pth`: set
`CONFIG_SITE=/usr/share/config.site` there.

And the interpreter's **`ctypes`**: its `_ctypes`, which the Python image leaves out (no libffi
there), in `/usr/lib/python3.13/lib-dynload`, on cffi's libffi (its backend for this platform,
below); `ctypes`' Python half is the image's standard library. It calls C as on Linux — `CDLL(None)`
is the C library and the interpreter, `CDLL(path)` a side module, `pythonapi`, structures and unions
by value, callbacks, `errno` — with what WebAssembly asks of a call: every call through a pointer is
checked against the function's type, so a function's `restype` must be what it returns (ctypes'
default is `int`: `free`, `qsort` and the like need `restype = None`), it is called with as many
arguments as it has, a variadic one (`printf`) with its fixed arguments in `argtypes` and at least
one more (WebAssembly passes the rest in memory), at most 1000 arguments (`CTYPES_MAX_ARGCOUNT`, as
on Emscripten); a call of another type ends the program, where natively it goes unnoticed.
`ctypes.util.find_library` finds nothing (no `ldconfig`, and the C library is the program's).
Unicorn's binding is ctypes: it is here because ctypes is.

## Turning it on

```json
"addons": {"python-extension": true}
```

or `--addon-dir addons/python-extension/out --addon python-extension` on the engine's command line.
It has no settings. It needs the Python image (`python.cpio`, which `--stdio` adds unless the app
asks for `python: false`): the add-on is an overlay over it that adds the packages to
`/usr/lib/python3.13/site-packages` and their console scripts to `/usr/bin`; the interpreter, the
standard library, pip and the openai SDK are the image's own. The add-on's image is 96 MB (rpds-py,
mcp and its wheels: 9 MB of it; unicorn 19 MB, most of it libunicorn.so.2, and its three wheels
9.3 MB; the interpreter's headers 2.3 MB).

The guest must be able to load shared libraries, which upstream's cannot yet: the C library's
`dlopen`, the engine's dynamic linker and an interpreter that uses them are `addons/mod/0019`–`0022`
(and `0016`, so that a C++ exception nothing catches ends the program instead of the machine). On
a Python image without them, `EXTENSION_SUFFIXES` is empty and `import numpy` fails; `build.sh`
refuses to build against one. Only the native engine loads libraries: in the browser (the
JavaScript host) Python runs as before, and `import numpy` raises `ImportError: ...: this host
cannot load shared libraries`.

A module is compiled when a process first loads it in a machine; those of a megabyte or more
(numpy's core, Pillow's `_imaging`, LIEF) are then kept compiled on disk by the engine's cache
of programs, for the next machines too. `import numpy, PIL.Image, lief` takes 3.1 s here when
nothing is compiled yet, 0.8 s once the cache has them (in the next machines too), and `import
numpy` alone 0.15 s. (The interpreter itself is the image's: the first `python3` in a machine
compiles it, 1.3 s, with or without the add-on.)

## How it works

The guest's CPython loads extension modules from files (`dynload_shlib`, as on Linux): the python
step builds it linked for dynamic loading (`addons/mod/0021`), exporting the Python API and the
whole C library, with a function table that can grow. An extension module is a **WebAssembly side
module** (`wasm-ld -shared --experimental-pic`): its own code and data, with what it does not
define imported from the interpreter. The C library's `dlopen` reads the file and allocates the
module's memory in the process's heap; the engine compiles it, links its imports and instantiates
it in the program's memory, its functions appended to the program's table, in every thread of the
process alike (`addons/mod/0019`, `0020`). So the packages are installed as pip would install their
wheels, modules included.

Each package is built from its source by its own build system (meson, setuptools, CMake, cargo),
with compilers that look like any cross compiler to it (`tools/wasm-pyext-cc`, `wasm-pyext-c++`):

- everything is compiled **position-independent** (`-fPIC`), static libraries included: they go
  into the modules that use them, which are shared libraries;
- a link with `-shared` — how a build system makes an extension module — is a side module's link
  (`wasm-cc -shared`); each module carries the static libraries it needs, as a Linux wheel's do.
  Not the interpreter's own (`python/deps`: zlib, liblzma, OpenSSL, ...), which it neither exports
  nor builds PIC: Pillow builds its own zlib and liblzma, and a module's link that names one of the
  interpreter's stops the build;
- C++ has **exceptions** (nanobind, LIEF's bindings, cannot do without): libc++, libc++abi and
  libunwind of LLVM 19.1.7 built with WebAssembly exception handling, which the engine runs (the
  other add-ons' libc++ is built without), PIC, linked into each C++ module;
- **Rust** (cryptography's and rpds-py's PyO3 modules) is the guest's Rust toolchain (`python/build-rust-toolchain.sh`:
  `wasm32-unknown-linux-musl`) with a standard library built again PIC for the add-on
  (`tools/lib.sh` `rust_std_pic`, stage `rust-std`); the crate is built as a static library and
  linked as any other module, exporting `PyInit_<name>` alone (std's symbols are not hidden, and
  exported they would keep all of it, `fork` included);
- **libffi** (cffi's) calls a function, and makes a callback, of any C type — and WebAssembly checks
  the type of every call through a pointer, so each needs a function of that very type, which
  nothing can have ready for every type and WebAssembly cannot make at run time but by
  instantiating a module. libffi's own wasm32 port makes them in JavaScript (Emscripten); the
  add-on's backend (`packages/cffi/patches/libffi`) writes them as small side modules and loads them
  with `dlopen`: a call module per type, table slots reserved for closures (so a closure's code
  pointer exists from `ffi_closure_alloc` on, as libffi's API wants), and modules whose element
  segments put the closures' functions in those slots, in every thread alike. Nothing of the
  engine changed for it; libffi's own testsuite runs in the guest (`tests/upstream.py libffi`);
- **Unicorn** translates the guest's code into the host CPU's and runs it, which WebAssembly
  cannot (no memory is executable, nor can a program make code but by instantiating a module).
  QEMU's interpreter of TCG's code (TCI), which Unicorn 2 left out of its QEMU 5.0.1, runs it
  instead (`packages/unicorn/patches/0001`): QEMU's `tcg/tci.c` on Unicorn's TCG, and its calls of
  the helpers each of the helper's own type — QEMU 5.0's called them all as one function of twelve
  arguments, natively harmless, a trap here — from the helper's sizemask, through a table of the
  381 types a helper can have (`tci-call-gen.py`). Memory is the heap's (`0002`: mmap gives a
  program none of its own memory here); glib_compat's comparisons and the binding's variadic calls
  are made as the types they are (`0003`, `0004`). Every test of Unicorn's runs as with its wheels
  but those whose hooks are of another type than the hook (below);
- every **function CPython will call** — a method table's, a type slot's, a getter's, a module
  slot's — must have the type CPython calls it as: WebAssembly checks the type of every indirect
  call, so a function written with fewer parameters than CPython passes traps, where natively it
  goes unnoticed. The guest's interpreter forgives it for methods, getters and setters, as
  Pyodide's does (`python/patches/cpython-call-trampoline.patch`: such a function is called with
  the parameters it has, which the engine reads from its table slot) — what lets packages pip
  builds in the guest run (python-xxhash's METH_NOARGS methods take only self) — but not for type
  and module slots. `tools/fpcast-scan.py` reads every compile of a package for both (stage
  `fpcast-<package>`), and the build stops on one: numpy had seven (`copy.copy(np.float64(1))`,
  `arr.dtype = ...` ended the interpreter), Pillow three (`import PIL._imagingtk` did); their fixes
  stay, so the packages need no trampoline;
- every module is **checked to link** before it is installed (`tools/install-tree.py`): each of its
  imports is the interpreter's, its own or weak — what the guest's `dlopen` would otherwise report
  as an undefined symbol at the first import.

The modules link against the interpreter's exports alone, so a rebuilt Python image (the same
CPython 3.13) runs them as they are. Packages that are pure Python (pycparser, which cffi needs)
are installed from their wheels as they are.

## Building it

```sh
python3 addons/build.py build python-extension        # or addons/python-extension/build.sh [PACKAGE...]
python3 addons/build.py test python-extension
```

It needs the `userspace` and `python` steps (`./build.sh tools userspace python`, with
`addons/mod/0019`–`0021`): the sysroot (with `dlopen`), the interpreter's headers and exports, the
host CPython (`python/host`) and the guest's Rust toolchain (`python/build-rust-toolchain.sh`:
its `wasm-ld`, and `rustc` for cryptography and rpds-py). From scratch it takes about five minutes
here (6 cores), most of it LIEF's (pycparser, cffi and cryptography with OpenSSL: a little over a
minute; rpds-py and mcp: well under one).
Stages (`FORCE=stage,...` redoes them, `JOBS=N`):

| Stage | |
|---|---|
| `libcxx` | libc++/libc++abi/libunwind with exceptions, PIC → `build/libcxx-install` |
| `host` | `build/host`: Cython, setuptools and pybind11 for the host CPython; `python3-guest`, the host CPython answering sysconfig with the guest's build variables |
| `numpy-src`, `numpy` | `packages/numpy` |
| `pillow-deps`, `pillow-src`, `pillow` | `packages/pillow`: its eight libraries, then Pillow |
| `lief-src`, `lief` | `packages/lief` (the longest: nanobind's bindings are large, so at most 4 jobs) |
| `lxml-src`, `lxml-deps`, `lxml` | `packages/lxml`: lxml's source (its libxslt patch is in it), its four libraries, then lxml |
| `pycparser` | `packages/pycparser`: its wheel |
| `cffi-src`, `cffi-libffi`, `cffi-host`, `cffi`, `cffi-1.17.1`, `cffi-2.0.0` | `packages/cffi`: cffi's source, libffi for the guest (with the add-on's backend) and for the host, cffi for the host CPython (`build/cffi/host-site`, what cryptography's build runs), then cffi, and its earlier minor lines for wheels (`build/cffi/wheels`) |
| `rust-std` | the guest's Rust std built PIC → `build/rust` (`tools/lib.sh` `rust_std_pic`; for the packages in Rust) |
| `cryptography-src`, `cryptography-openssl`, `cryptography` | `packages/cryptography`: its source, OpenSSL 4.0.3, then its Rust module (cargo) and the package |
| `rpds-py-src`, `rpds-py` | `packages/rpds-py`: its source, then its Rust module (cargo) and the package as maturin lays it out |
| `mcp` | `packages/mcp`: mcp's wheel and those of what it needs (pure Python), unpacked side by side |
| `fpcast-numpy`, ... | each package's compiles (recorded in `build/<package>/compdb`) scanned for functions CPython would call as another type; stops the build on one (`packages/<package>/fpcasts.allow` for those reviewed as never called) |
| `install` | the packages into `build/site`, each module checked to link against the interpreter |
| `ctypes` | the interpreter's `_ctypes` from CPython's `Modules/_ctypes` (the python step's source) on cffi's libffi → `build/ctypes/lib-dynload`, scanned and checked to link as the packages' are; `_ctypes_test` beside it for the tests (`build/ctypes/test`), not installed |
| `unicorn-src`, `unicorn`, `unicorn-2.0.1.post1-src`, `unicorn-2.0.1.post1`, `unicorn-1.0.3-src`, `unicorn-1.0.3` | `packages/unicorn`: Unicorn's sdist with its patches (TCI), then its CMake build (`libunicorn.so.2`, and the archives the tests' programs link) and the package; and its earlier minor lines for wheels (`build/unicorn/wheels`), each with the patches of its line: 2.0 with CMake, 1.0 with its Makefile (`libunicorn.so`) |
| `wheels` | the `WHEELS` packages (cffi, pycparser, unicorn) as wheels → `build/wheels`: each one's installed version and the others its build leaves in `build/<name>/wheels/`, the modules and libraries checked as `install` checks them (`tools/make-wheels.py`) |
| `dev` | the interpreter's headers (`python/stage/usr/include/python3.13`) and `python-3.13.pc` (without `Libs.private`: the interpreter's own static libraries, at their build paths) → `build/dev`; not the `-embed` files (there is no libpython to link a program with) |
| `cpio` | `out/python-extension.cpio` (with `build/wheels` in `/usr/share/python-wheels`, `etc/pip.conf`, `etc/collabo-config-site.pth` and `_ctypes` in `/usr/lib/python3.13/lib-dynload`), `out/licenses/` |

Sources are pinned by sha256 in `sources.lock`. The packages' patches are in
`packages/<name>/patches/`:

| Patch | |
|---|---|
| `numpy/0001` | no `madvise` where the C library has none (`HAVE_MADVISE`) |
| `numpy/0002` | getters, setters and methods with the parameters CPython calls them with (seven of them) |
| `pillow/freetype/0001` | FreeType's stdio streams on wasm32 (its Unix ones `mmap` font files) |
| `pillow/pillow/0001` | the Arrow export methods with the parameters CPython calls them with |
| `pillow/pillow/0002` | `_imagingtk`'s `Py_mod_exec` slot with the parameter CPython calls it with |
| `cffi/libffi/0001` | libffi's wasm32 port for a Linux guest that loads side modules (`src/wasm/ffi_dlopen.c`): calls and closures as side modules it writes and `dlopen`s; `_Complex` as structs; `closures.c` left out on any wasm (it maps memory executable) |
| `cffi/cffi/0001` | cffi takes callbacks from libffi on wasm as on Emscripten (it would allocate executable memory itself); the same on 1.17.1 and 2.0.0 |
| `unicorn/0001` | TCG's interpreter for a WebAssembly host: QEMU 5.0.1's TCI (`tcg/tci.c`, `tcg/tci/`) on Unicorn's TCG, each helper called as its type (`tcg/tci-call.inc.c`, from `tci-call-gen.py`), `GETPC()` the interpreter's place; `__wasm32__` a host CPU in `CMakeLists.txt` and `qemu/configure` |
| `unicorn/0002` | the code buffer and RAM blocks from the heap, no memory protections (on WebAssembly) |
| `unicorn/0003` | glib_compat's `g_tree_new`, `g_list_sort`, `g_slist_sort` call their comparison as its type |
| `unicorn/0004` | the binding calls `uc_hook_add` and `uc_ctl` as variadic (at least one variable argument) |
| `unicorn/patches-2.0/0001`–`0004` | the same for Unicorn 2.0.1.post1 (its tree: `qemu/configure` has no LoongArch, the binding is one `unicorn.py`) |
| `unicorn/patches-2.0/0005` | the binding finds its library without `distutils` and `pkg_resources` (gone from Python 3.12 and from setuptools; `import unicorn` failed before anything) |
| `unicorn/patches-2.0/0006` | the 128-bit atomic helpers (AArch64 CASP, PPC64 LQ/STQ/STQCX., S390X CDSG/LPQ/STPQ/CSST in a parallel context) only where the host has 128-bit atomics, as Unicorn 2.1 has it: `libunicorn.so.2` imported the missing functions, and its `dlopen` failed |
| `unicorn/patches-1.0/0001` | TCG's interpreter for Unicorn 1 (QEMU 2.2.1's TCG): QEMU 2.2.1's TCI (`tci.c`, `tcg/tci/`) on Unicorn 1's per-context TCG, its registers its own (not one static set), each helper called as its type (the same `tci-call.inc.c`), per guest architecture (`UNICORN_ARCH_POSTFIX` in `qemu/<arch>.h`, as Unicorn 2's); `__wasm32__` a host CPU with the interpreter in `qemu/configure` |
| `unicorn/patches-1.0/0002` | the code buffer and RAM blocks from the heap; no file-backed RAM, no remap (on WebAssembly) |
| `unicorn/patches-1.0/0003` | glib_compat's `g_list_sort`, `g_slist_sort` call their comparison as its type |
| `unicorn/patches-1.0/0004` | the binding gives `uc_hook_add` its fixed parameters' types (it had none, so ctypes called it as a fixed function: the instruction of an `UC_HOOK_INSN` was read as the address of the variable arguments, every IN/OUT/SYSCALL hook `UC_ERR_HOOK`) and at least one variable argument |
| `unicorn/patches-1.0/0005` | as `patches-2.0/0005` |

cryptography and rpds-py have none: their Rust builds as it is. lxml has none of its own; its libxslt gets the
security fixes lxml backports (`libxslt-1.1.43-backport1.patch`, from lxml's sdist), as lxml's own
static build applies them.
(numpy's floating-point status needs no patch: the guest's C library keeps `<fenv.h>`'s flags in
software, and every module raises and tests the same ones through it.)

## Tests

`tests/run.sh` boots the packaged runtime (`dist/runtime/collabo-core-<platform>`) with the Python
image and the add-on and checks: every extension module loads from its shared library;
`tests/checks/*.py` — the
packages at work: linear algebra, FFTs, every random generator and distribution, sorting,
strings, every codec and drawing operation, fonts, colour management, ELF/PE/COFF/Mach-O binaries
(`tests/samples`, made by `tests/make-samples.sh`) parsed, changed and written, XML and XSLT, C
called through cffi with every kind of argument, result and callback, every algorithm of
cryptography with fixed keys, rpds's persistent maps, sets, lists, queues and stacks, and mcp: an
`MCPServer`'s tools, resources, prompts, progress and logs used by a `Client` in process, over stdio
(the server a process of its own), over streamable HTTP and over SSE (uvicorn on the loopback), with
jsonschema, referencing, pyjwt and python-multipart at work — printing
**exactly what they print with the packages' own wheels** from PyPI on this machine
(`tests/expected/`, written by `tests/reference.py`, or `tests/run.sh --reference`; the host
CPython has no `ssl`, which mcp's check needs: `reference.py` builds `_ssl` and `_hashlib` for it
once, against the python step's OpenSSL source, in `build/reference/ssl`); the interpreter's
headers and `python3.pc` where `sysconfig` says a build finds them; pip, the
console scripts, the packages in threads (one of them started before the modules were loaded,
calling a cffi callback another thread made), the differences listed above, and the wheels: without
the network, `cffi~=1.17`, `cffi>=2.0,<2.1` and `cffi>=1.12` each install their version (with
pycparser), which calls C, varargs and callbacks, and a build's isolated environment takes cffi
1.17.1 or 2.1.1 from them as its `pyproject.toml` asks; `CONFIG_SITE`, which a Python
process sets only when a `config.site` is there, and never over one already set; ctypes from
lib-dynload (calls, a variadic one, callbacks, structures and unions by value, pointers, `errno`,
`pythonapi`, the 1000-argument limit); and unicorn (`tests/checks/unicorn_check.py`, against its
wheel like the others: each architecture's code, x86's FPU and SSE, ARM64's SIMD and floating
point, every kind of hook, memory and MMIO, contexts, exits, controls) — and each of its versions
installed by pip from the wheels, against its own PyPI wheel (`expected/unicorn_check-<version>.out`;
what Unicorn 1 has not left out), with the requirements pwntools', `unicorn<2` and `unicorn>=2`
getting 2.0.1.post1, 1.0.3 and 2.1.4 from the wheels though a higher version's sdist is beside them,
and `unicorn==2.1.2` none.

`tests/run.sh --upstream` then runs the packages' own test suites in the guest
(`tests/upstream.py`, with pytest and hypothesis from `tests/upstream.lock`, libffi's C
testsuite, CPython's test_ctypes and Unicorn's unit tests, built for the guest against the build's
libffi and Unicorn's archives; about eight minutes, rpds-py's, jsonschema's and referencing's a
minute and a half of it, and five more for Unicorn's):

| Suite | Passed | Failed |
|---|---|---|
| numpy (every subpackage) | 47 961 | 397, all known (`tests/upstream-known.txt`): the floating-point exception flags WebAssembly does not have (numpy marks these tests `IS_WASM`; its check looks for a `wasm32` machine, the guest's is `wasm`), `ctypes`, no compiler for the tests that build a module, no shared memory, no C headers |
| Pillow (`Tests/`) | 4 738 | none |
| lxml (`lxml.tests`, `lxml.html.tests`, with their doctests) | 1 918 | 1, known: `adopt_external_document`'s test makes its document with a libxml2 of its own (`find_library('xml2')`), which the guest has not: lxml's is inside its module, as in its wheels (87 skipped, as the wheel's: html5lib, cssselect, HTTP in libxml2) |
| cffi (`src/c/test_c.py`, and the files of `testing/` that need no compiler) | 571 | 6, all known: a C compiler (cffi's `verify()`), functions called as another type (`void getenv(char *)`: here that ends the program, which the run goes on after), musl's `dlopen` without `RTLD_DEEPBIND` |
| cryptography (`tests/`, with `cryptography_vectors`) | 4 450 | none (200 skipped: the 194 the wheel skips too — the Wycheproof and x509-limbo corpora not given, `bcrypt`, FIPS, tests for an OpenSSL without some algorithm — and 6 that need `mmap` and a 64-bit platform) |
| libffi (its testsuite's C programs: `libffi.call`, `.closures`, `.complex`, `.bhaible` — every argument and result type, exhaustively — and `.threads`) | 208 | 5, all known: calls of functions as another type (natively harmless, a trap here; corrected, they pass) and a check that the closure's code is its memory (left out on Emscripten too) |
| rpds-py (`tests/`) | 132 | none |
| jsonschema (`jsonschema.tests`, with the JSON Schema Test Suite its sdist carries) | 7 389 | 1, known: a test that compares a warning's file name with `importlib.__file__`, which in the guest names the standard library's zip of `.pyc` files (425 skipped, as with the wheel). Its pytest then traps at its exit, after the summary, in the interpreter's last garbage collection (`EXIT_TRAP_KNOWN` in `upstream.py`: not rpds-py's, whose objects the collector does not see; not found yet) |
| referencing (`referencing.tests`, with the referencing suite) | 1 006 (and 812 subtests) | none (264 xfailed, as the suite marks them) |
| ctypes (CPython's `test_ctypes`, with `_ctypes_test`) | 467 | 7, all known: calls of a function as another type (`void *f(void *)` with restype None, a callback or function given more arguments than it has, `atan2` with a structure of two doubles by value), each ending the interpreter, which the run goes on after; `_testcapi`, which the image has not |
| unicorn (`tests/unit`: C programs with acutest, each test on its own) | 158 | 9, all known: hooks given a callback of another type than the hook's (memory-access hooks returning `bool`, an IN hook returning nothing, a sysreg hook of three parameters for four) |
| unicorn's Python regression tests (`tests/regress`) | 65 | 4, known: they print with capstone, which is not here |

(numpy's core test modules import `ctypes`; the run gives them a stand-in so that they are
collected, and only their tests that use it fail. lxml's are run as its own `test.py` runs them:
each module's `test_suite()`, from a folder laid out as its sdist. cffi's read the sizes of C's
types from `ctypes` and look for libraries with `ctypes.util.find_library`: a stand-in answers,
the libraries `c`, `m`, `dl`, ... are the program (`dlopen(NULL)`: musl has them in one), and its
`TestOwnLib` gets the library it would compile, built beforehand with the add-on's compiler.)
LIEF's own suite needs its corpus of sample binaries from the network;
`tests/checks/lief_check.py` covers what it can. mcp's own suite is not run: its `conftest.py`
needs logfire (with OpenTelemetry's SDK) and blockbuster (forbiddenfruit, a C module), and much of
it trio, inline-snapshot and pytest-examples; `tests/checks/mcp_check.py` covers its transports.

## Files

```
addons/python-extension/
  build.sh            the stages above
  addon.json          its description (not bundled: an app takes it from out/)
  sources.lock        every download, with its sha256
  packages/           one folder per package: build.sh, patches/ (packages/README.md)
  etc/pip.conf        /etc/pip.conf in the guest: pip's find-links, the wheels; unicorn from wheels only
  etc/collabo-config-site.pth   in site-packages: CONFIG_SITE for the configure scripts of Python's builds
  tools/
    lib.sh            shell helpers of build.sh and the packages' scripts
    wasm-pyext-cc     the C/C++ compilers for the guest (wasm-pyext-c++)
    dist-info.py      a package's .dist-info, from its source
    install-tree.py   the packages into one site-packages; each module checked to link
    make-wheels.py    built trees as wheels (/usr/share/python-wheels), checked the same way
    fpcast-scan.py    the functions CPython would call as a type they do not have
    crates.py         the crates a Rust module carries, with their licenses (cargo metadata)
  tests/              run.sh, test_guest.py, checks/, expected/, samples/, reference.py,
                      upstream.py, upstream.lock, upstream-known.txt
  build/              the build's work, out/: python-extension.cpio, python-extension.json, licenses/
```

## Licenses

numpy: BSD-3-Clause (and the licenses of what it carries: `out/licenses/addon-python-extension-numpy-*`);
Pillow: MIT-CMU, with libjpeg-turbo (IJG, BSD-3-Clause, zlib), openjpeg (BSD-2-Clause), libtiff
(libtiff), libwebp (BSD-3-Clause), FreeType (FTL), LittleCMS (MIT), zlib (zlib) and liblzma (0BSD); LIEF: Apache-2.0, with
spdlog, mbedtls, utfcpp, frozen, tl-expected, tcb-span, nanobind and nlohmann/json; libc++,
libc++abi, libunwind: Apache-2.0 WITH LLVM-exception; lxml: BSD-3-Clause (and what its
LICENSES.txt lists), with libxml2 (MIT), libxslt (MIT), GNU libiconv (LGPL-2.1-or-later) and zlib
(zlib); cffi: MIT-0, with libffi (MIT); pycparser: BSD-3-Clause; cryptography: Apache-2.0 OR
BSD-3-Clause, with OpenSSL (Apache-2.0), Rust's standard library (MIT OR Apache-2.0, and what
its COPYRIGHT lists) and the crates listed in `addon-python-extension-cryptography-crates.txt`
(MIT, Apache-2.0, BSD-3-Clause); rpds-py: MIT, with Rust's standard library and the crates listed in
`addon-python-extension-rpds_py-crates.txt` (MIT, MIT OR Apache-2.0); mcp and mcp-types: MIT, with
jsonschema, jsonschema-specifications, referencing, attrs and pyjwt (MIT), opentelemetry-api and
python-multipart (Apache-2.0), Starlette, sse-starlette, uvicorn and click (BSD-3-Clause); the
interpreter's headers and `_ctypes`: CPython's (PSF-2.0, `addon-python-extension-cpython-headers.txt`;
`_ctypes` with libffi, MIT); unicorn: GPL-2.0 (its README; the C library is QEMU's, GPL-2.0-or-later and
LGPL-2.1-or-later as `addon-python-extension-unicorn-qemu-LICENSE` says, TCI included; its Python
binding BSD). All in
`out/licenses/`.
