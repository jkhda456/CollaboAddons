# typst add-on

`typst` in the sandbox: **Typst 0.15.1**, the markup-based typesetting system, cross-compiled for
the guest's WebAssembly Linux from Typst's own Rust sources (`typst-cli` at the v0.15.1 tag) with a
few patches — so agents can write reports and turn them into PDF (and PNG, SVG, HTML). Its output
is the official 0.15.1 binary's, byte for byte, but for two cases where the code picks its
implementation by the CPU (see *Checks*), and it says what it is:

```
$ typst --version
typst 0.15.1 (9dfd3a08) [WASM native port for collaboCore]
```

What the add-on installs, besides the program, is there to help agents write reports:

| In the guest | What |
|---|---|
| `/usr/bin/typst` | typst: `compile`, `watch`, `init`, `eval`, `query`, `fonts`, `info`, `completions` (44 MB) |
| `/usr/share/fonts/nanum` | Korean fonts: Nanum Myeongjo (serif), Nanum Gothic (sans), Nanum Gothic Coding (mono), regular and bold (SIL OFL); typst's own Libertinus Serif, New Computer Modern and DejaVu Sans Mono are built in |
| `/usr/share/typst/packages/collabo/report/0.1.0` | `@collabo/report`, a report template that needs no download: title page, contents, headers, callouts, key-figure cards, tables from JSON/CSV, bar/line/pie charts drawn with Typst alone, Korean lines broken between words |
| `/usr/share/typst/packages/preview` | Typst Universe packages for use without network: cmarker (Markdown), mitex (LaTeX math), cetz, cetz-plot, lilaq (plots), fletcher (diagrams) |
| `/usr/share/doc/typst/AGENTS.md` | the guide for agents: the workflow (data files, compile, look at PNG pages, `typst eval` to check the structure), Typst syntax on a page, the mistakes models make (escaping `$`, Markdown/LaTeX habits, what 0.12–0.15 removed) |
| `/usr/share/doc/typst/examples/report-en.typ` | an English report with the template |
| `/etc/codex/skills/typst-report/SKILL.md`, `/root/.agent/skills/typst-report/SKILL.md`, `/root/.claude/skills/typst-report/SKILL.md` | the same as a skill, for Codex (its system skills), agent (the agent add-on: the guest user's skills) and Claude Code (the guest user's skills: the official one on the node add-on, or the claude-code add-on) |

The image is 62 MB, so the add-on is not bundled in the runtime (`addons/README.md`); an app takes
`out/typst.cpio` and `out/typst.json`.

## Turning it on

```json
"addons": {"typst": true}
```

(`CollaboConfig(addons: {'typst': {}})` in Dart), or `--addon-dir addons/typst/out --addon typst`
on the engine's command line. It has no settings. A report the agent writes in a mounted folder
(`--mount`, `Mount(...)`) is on the host when it is done.

```sh
typst init @collabo/report report && cd report    # main.typ, data.json, incidents.csv to adapt
typst compile main.typ                            # main.pdf
typst compile main.typ 'page-{p}.png' --ppi 80    # the pages as images, to look at
typst eval --in main.typ 'query(heading).map(h => h.body.text)'
typst watch main.typ                              # recompile on every change
```

```typst
#import "@collabo/report:0.1.0": *
#show: report.with(title: [분기 보고서], author: "분석팀", summary: [한 문단 요약.])

= 핵심 지표
#kpis((label: "매출", value: "12.4억", delta: "+8.2%"), (label: "비용", value: "7.1억", delta: "+3.0%", good: false))
#bar-chart(json("monthly.json"), unit: "억", caption: [월별 매출]) <fig-monthly>
#data-table(csv("incidents.csv"), caption: [장애 목록], total: "합계")
```

The template's reference is its `README.md` (beside `lib.typ`); `lang: "en"` makes an English
report.

## Packages from Typst Universe

`#import "@preview/cetz:0.4.2"` and `typst init @preview/…` download from `packages.typst.org`
as on any computer, but the request goes through the host's request API (vsock port 1080, as
`hfetch` and the codex add-on's requests do): the host makes it under the sandbox's network policy
and its own TLS, and streams the answer back. So `packages.typst.org` must be allowed (a refusal
says so: *the sandbox's network policy does not allow …*), and the guest needs no certificates
(`--cert` has no effect). Downloads are cached in `~/.cache/typst/packages`, which goes with the
sandbox unless that folder is mounted. `COLLABO_BRIDGE=tcp:HOST:PORT` speaks the same protocol
over TCP (the tests use it), `COLLABO_BRIDGE=off` turns downloads off.

Some packages are in the image, so they work without network (at these versions; others
download): `@preview/cmarker:0.1.10` (Markdown to Typst — agents write Markdown readily),
`@preview/mitex:0.2.7` (LaTeX math), `@preview/cetz:0.5.2` and `@preview/cetz-plot:0.1.4`
(drawings, plots), `@preview/lilaq:0.6.0` (data plots), `@preview/fletcher:0.5.8` (diagrams), and
what they import (`tools/universe.py` lists them into `sources.lock`; the `packages` stage
unpacks them into `/usr/share/typst/packages/preview`, where typst looks before downloading).

## How it is built

`build.sh` (stages; `FORCE=src,cross,fonts,packages,cpio`, `JOBS=N`, `TEST=1` for the host module's
unit tests, built natively):

- **src** — `git archive` of `../org/typst` at `TYPST_REV` (v0.15.1) into `build/src`, with
  `patches/typst/*.patch`; `cargo vendor` of its crates into `build/vendor`, with
  `patches/crates/*.patch`; libc from tombl's fork (the platform's ILP32 musl ABI), as the other
  Rust add-ons do.
- **cross** — `typst-cli` for `wasm32-unknown-linux-musl` with the guest's Rust toolchain
  (`python/build-rust-toolchain.sh`; linked by `userspace/bin/wasm-cc`), release without LTO.
- **fonts** — the Nanum fonts from `sources.lock` (Google Fonts), checked by sha256.
- **packages** — the Typst Universe packages `sources.lock` names (`universe:…`), likewise.
- **cpio** — `out/typst.cpio`, `out/typst.json`, `out/licenses/` (Typst's Apache-2.0 license and
  NOTICE, the fonts' OFL, the Universe packages' licenses, `crates.txt`: every crate linked in
  with its license).

The patches to typst (`patches/typst`, made with `patches/mkpatch-typst.sh`):

| Patch | Why |
|---|---|
| `0001-build-for-the-guest` | the workspace without docs/tests; `typst-cli` without the OpenSSL downloader; a `fork()` that fails with `ENOSYS` (the guest has no fork; std and the `open` crate name it) |
| `0002-downloads-through-the-host` | `src/host.rs`: the downloader speaks the host's request API (above) |
| `0003-shared-package-directory` | packages in `/usr/share/typst/packages` (or `$TYPST_SHARED_PACKAGE_PATH`), read-only, after the user's data directory and before the cache; `typst init @collabo/report` finds the latest version there |
| `0004-port-notice` | *WASM native port for collaboCore* in `--version`, `--help`, `typst info` and the first-run greeting, with the guide's path |
| `0005-poll-without-inotify` | `typst watch` polls (every 300 ms) when the platform's watcher cannot start: the guest kernel has no inotify. Polling also sees changes the host makes in a mounted folder, which inotify would not |

The crates (`patches/crates`): `<crate>.patch` narrows `wasm32` cfgs that mean "a browser" to wasm
that is not Linux (`patches/narrow-wasm-cfg.py`: dirs, filetime, mio, parking_lot_core, tar,
time); by hand (`patches/mkpatch.sh`): linux-raw-sys and rustix learn wasm32 (the asm-generic
constants, as riscv32), and fontdb reads font files instead of mapping them (the guest has no
`mmap`).

Not there: `typst update` (self-update is off in the build), `--open` (no viewer in the guest).
`typst watch` on an HTML document serves it inside the guest (`127.0.0.1`, a port from 3000 or
`--port`); the engine forwards no ports to the host.

## Checks

`tests/run.sh` (the runtime in `dist/runtime`, `COLLABO_RUNTIME`, `TYPST_ENGINE`, `TYPST_KERNEL`,
`TYPST_CPIO` as in the other add-ons). The official typst 0.15.1 binary for this machine is
downloaded once into `build/` (`sources.lock`), with the guest's fonts and packages given to it:

- `test_guest.py` (half a minute): the files installed; documents (Korean and English text, math,
  code, tables, footnotes, outline; PNG/JPEG/GIF/WebP/SVG/PDF images; CSV/JSON/YAML/TOML/XML;
  a WebAssembly plugin; a bibliography; the template's report; `--input`) compiled to PDF, PNG,
  SVG, HTML and PDF/A in the guest and by the official binary — the same bytes; `-j 1` and
  `-j 4`; diagnostics; `eval`, `query`, stdin; `init` offline; `watch` (guest and host edits);
  the Universe packages in the image with `--no-network`;
  downloads through a fake request API (chunks, 404, a broken stream, a policy refusal, the
  Universe index), the real one under a deny rule, and a real `@preview/cetz` when this machine
  is online; a 40-chapter report's time.
- `test_suite.py` (about 10 minutes; `run.sh --quick` skips it): Typst's own test suite at
  v0.15.1 — every snippet compiled in the guest and by the official binary with the test
  runner's helpers and assets, the exit status, diagnostics and each PNG, PDF and HTML file
  compared. Of the 5,987 compilations (3,557 tests), 5,981 give the same bytes; the other six
  are the two cases below.

`TEST=1 build.sh` runs the host module's unit tests natively.

Where the output can differ, the code picks its implementation by the CPU it runs on (SSE or AVX
on x86, NEON — with or without RDM — on ARM, scalar elsewhere), and the guest runs the scalar
one:

- a PDF placed as an image (`image("x.pdf")`) is rasterized for PNG output by hayro (vello_cpu):
  anti-aliased edges can differ by a level or two, on under 1% of the pixels;
- CMYK colors (`cmyk(…)`) are converted to RGB by moxcms' ICC transform (a table built with the C
  library's math, interpolated with NEON fixed-point on this aarch64 machine): a channel can be a
  level off (`cmyk(4%, 5%, 6%, 7%)` is `#e2ddda` in the guest, `#e2dcda` natively).

The PDF and SVG output of the first are the same. A panic (a bug in typst) ends typst with
`SIGABRT` (status 134) in the guest, which is built with `panic=abort`, where it unwinds (101)
natively.

Speed: a 40-chapter, 63-page report with tables and charts takes 0.3 s in the guest (0.15 s
natively, on the same aarch64 machine); booting the guest takes 0.7–0.8 s. The add-on builds
from scratch in under 2 minutes on 6 cores (334 crates, no LTO).
