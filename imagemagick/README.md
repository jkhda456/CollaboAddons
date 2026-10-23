# imagemagick add-on

`magick` in the sandbox: **ImageMagick 7.1.2-34** (Q16 HDRI, with OpenMP), built from the clone
`../org/ImageMagick` at its 7.1.2-34 tag for the guest's WebAssembly Linux, one static program with
its delegate libraries — so that agents can edit, convert, compose and inspect images on the command
line. For the same input and command it writes the same pixels as the same sources built natively
(see *Checks*).

| In the guest | What |
|---|---|
| `/usr/bin/magick` | ImageMagick (13 MB), and `convert`, `identify`, `mogrify`, `composite`, `compare`, `montage`, `stream`, `conjure`, `magick-script` as links to it (not the X11 tools `display`, `animate`, `import`) |
| `/etc/ImageMagick-7` | its configuration: `policy.xml` (the add-on's: open, with limits for a wasm32 program and the fallback fonts), `type.xml` (the fonts by name, `tools/type-xml.py`), `sRGB.icm` (for `-profile`), and ImageMagick's own colors, delegates, log, mime, thresholds, quantization tables |
| `/usr/share/ImageMagick-7`, `/usr/lib/ImageMagick-7.1.2` | its messages (English, French), `configure.xml` (`magick -list configure`) |
| `/usr/share/fonts/dejavu` | DejaVu Sans, Serif, Sans Mono, Condensed, ExtraLight, Math (Latin, Greek, Cyrillic) |
| `/usr/share/fonts/nanum` | NanumGothic and NanumMyeongjo, regular and bold (Korean, with Latin) — the same files as the typst add-on's |
| `/usr/share/doc/imagemagick/AGENTS.md` | the guide for agents: the workflow (inspect, edit into a new file, look at the result), ImageMagick 7's syntax (settings before operators, geometry, quoting), recipes for sizes, formats, color, text, composing, comparing, batches; the fonts; speed and memory; what fails and why |
| `/etc/codex/skills/imagemagick/SKILL.md`, `/root/.agent/skills/imagemagick/SKILL.md`, `/root/.claude/skills/imagemagick/SKILL.md` | the same as a skill, for Codex (its system skills), agent (the agent add-on: the guest user's skills) and Claude Code (the guest user's skills: the official one on the node add-on, or the claude-code add-on) |

Formats: PNG, JPEG, GIF and WebP (animated), AVIF, TIFF, JPEG 2000, BMP, ICO, PSD, TGA, PNM,
QOI, MIFF and the many formats ImageMagick reads by itself; HEIC (iPhone photos) **read only** (no
HEVC encoder: x265 is not built); SVG read with its own renderer (MSVG, libxml2) and written; PDF
and PostScript **written only** (no Ghostscript). ICC profiles through Little CMS. Not built: JPEG
XL, camera RAW, OpenEXR, FFTW (`-fft`), Liquid Rescale, Pango, fontconfig, X11; no external
programs (Ghostscript, Inkscape, ffmpeg, curl) in the guest.

The image is 34 MB (the fonts are 20 of it), so the add-on is not bundled in the runtime
(`addons/README.md`); an app takes `out/imagemagick.cpio` and `out/imagemagick.json`.

## Turning it on

```json
"addons": {"imagemagick": true}
```

(`CollaboConfig(addons: {'imagemagick': {}})` in Dart), or `--addon-dir addons/imagemagick/out
--addon imagemagick` on the engine's command line. It has no settings. Images in a mounted folder
(`--mount`, `Mount(...)`) are edited in place on the host.

```sh
magick identify -format '%m %wx%h %[colorspace] %b\n' photo.jpg
magick photo.jpg -resize '1600x1600>' -quality 85 smaller.jpg
magick photo.jpg -gravity south -font NanumGothic-Bold -pointsize 48 -fill white \
  -stroke black -strokewidth 2 -annotate +0+30 '2026 봄 신제품' captioned.jpg
magick montage -label '%f' -font DejaVu-Sans *.jpg -tile 4x -geometry 200x200+4+4 sheet.jpg
```

## Fonts

Without fontconfig, `type.xml` names the fonts as fontconfig would in ImageMagick
(`DejaVu-Sans-Bold-Oblique`, `NanumGothic-Bold`), with families, weights, styles and stretches
for `-family`/`-weight`/`-style`/`-stretch`, and maps the names scripts use for the standard fonts
to DejaVu: Helvetica, Arial, Sans (DejaVu Sans; text without `-font` is in it), Times, Times New
Roman, Serif (DejaVu Serif), Courier, Courier New, Monospace (DejaVu Sans Mono).

ImageMagick draws a character its font has no glyph for as a space, silently. In the add-on it
draws it with a **fallback font** instead (patch 0002): the first of policy.xml's
`system:fallback-font` list (`NanumGothic,DejaVu Sans`) that has the glyph, of the weight and slant
closest to the font's, so Korean in a Latin font (or Greek in NanumGothic) comes out right.
`-define type:fallback-font=FONT[,FONT]` (names, families or files) replaces the list for a
command, `none` turns it off. None of the fonts has Chinese characters (Hanja), kana or emoji.

## How it is built

`build.sh` (stages, each skipped when up to date; `FORCE=src,llvm,deps-wasm,magick-wasm,fonts,cpio`,
`JOBS=N`; `REF=1` also builds the native reference the checks compare with, `build/native`):

- **src** — `git archive` of the clone at `IM_REV` (the 7.1.2-34 tag) into `build/src`, a git
  repository whose first commit is the pristine export, with `patches/imagemagick/*.patch`
  (`tools/save-patches.sh` writes them back from `git diff` there).
- **llvm** — from LLVM 19.1.7 (clang-19's release, the archive other add-ons download): the
  OpenMP runtime with `patches/openmp`, and libc++ (whose headers libomp's C++ includes).
- **deps-wasm** (and **deps-native**) — the delegate libraries `sources.lock` names, static, the
  versions the wpe and python-extension add-ons build: zlib 1.3.1, xz 5.6.4 (liblzma), libpng
  1.6.58, libjpeg-turbo 3.2.0 (no SIMD), libwebp 1.6.0 (with mux/demux), libtiff 4.7.1, OpenJPEG
  2.5.4, FreeType 2.14.3 (stdio streams: `patches/freetype`, as Pillow's), Little CMS 2.19.1,
  libxml2 2.15.4, libde265 1.1.3 (HEVC decoding), libaom 3.15.2 (AV1, its generic C code, decoding
  and encoding), libheif 1.23.6 (HEIC and AVIF, built with those two; no plugins); before them
  the runtimes: for wasm libc++ with C++ exceptions (WebAssembly exception handling, libunwind's
  wasm unwinder merged into libc++abi, as the python-extension add-on builds it — libheif turns
  exceptions, out of memory among them, into errors), and libomp. Release flags with `-DNDEBUG`;
  the reference's C and C++ with `-ffp-contract=off`.
- **magick-wasm** (and **magick-native**) — ImageMagick's configure and make (`--without-modules`:
  every coder in the program; `--enable-openmp`; `pkg-config --static`; every delegate not built
  here off explicitly, so that nothing of the build machine's is picked up), its check programs
  (`tests/validate`, `drawtest`, `wandtest`), installed into `build/<target>/root`.
- **fonts** — DejaVu 2.37 and the Nanum fonts from `sources.lock`, checked by sha256.
- **cpio** — `out/imagemagick.cpio`, `out/imagemagick.json`, `out/licenses/` (ImageMagick's
  license and notice, each library's, libomp's and libc++'s, the fonts'; libheif and libde265 are
  LGPL-3.0 and linked statically: with the LGPL and GPL texts, and this build and their sources
  are what relinking with other versions of them needs).

The program is linked as the other C add-ons' (`--shared-memory`, up to 4 GiB, 8 MiB stack) by
`userspace/bin/wasm-cc` (through `tools/wasm-cc`, which keeps CMake's `-v` from turning a link
into a question), C++ by `tools/wasm-c++` (clang-19 with wasm-cc's flags, `-fwasm-exceptions` and
libc++); the runtimes (libc++, libomp) are compiled by clang-19 with wasm-cc's flags.

The patches to ImageMagick (`patches/imagemagick`):

| Patch | Why |
|---|---|
| `0001-delegates-spawned-without-fork` | a delegate (an external program ImageMagick runs, e.g. Inkscape for SVG) is started with `posix_spawnp`: the guest has no `fork`. One that is not there fails quietly with status 1, as its `execvp` would (ImageMagick then renders SVG itself) |
| `0002-text-fallback-fonts` | glyphs a font lacks from fallback fonts (*Fonts*); in the simple text layout (no raqm), kerning only between glyphs of the same font |
| `0003-threads-every-cpu-of-the-guest` | ImageMagick leaves one CPU to the system (`threads = CPUs − 1`); the guest's CPUs are its engine's threads, all the program's: with the default 2, one thread was half the speed |

And to the OpenMP runtime (`patches/openmp`), which knows wasm32 from the WASI and Emscripten
builds: `0001` no thread affinity on wasm32 Linux (no CPU mask; kmp_affinity.h has no system
calls for it); `0002` microtasks with up to 64 arguments — WebAssembly checks the type of every
indirect call, so the runtime calls each parallel region's function with a case per argument
count, and had 15, but clang passes every shared variable as an argument (ImageMagick's regions
take up to 24: `-distort` stopped with *Too many args to microtask: 16!*).

## Checks

`tests/run.sh` (`--quick`: `test_guest.py` only; the runtime in `dist/runtime`, `COLLABO_RUNTIME`,
`IM_ENGINE`, `IM_KERNEL`, `IM_CPIO`, `IM_CPUS` as in the other add-ons; builds the reference with
`REF=1 build.sh` when it is missing):

- `test_guest.py` (2 minutes): 87 cases of what agents do — resizing (every geometry form,
  filters), cropping, trimming, rotating, EXIF orientation, borders, distortions; color, levels,
  quantizing, colorspaces, ICC profiles, CLUTs; blur, sharpen, edges, artistic effects,
  morphology, `-fx`, connected components; watermarks, compose modes, append, rounded corners,
  shadows, layers; text in Korean and Latin (annotate, label, caption, fonts by name, family and
  file, the fallback), drawing, MVG, gradients, SVG; PNG variants, JPEG options (`jpeg:extent`),
  WebP, GIF and WebP animations, HEIC and AVIF read (libheif's samples), AVIF written, TIFF
  compressions, JPEG 2000, ICO, BMP, TGA, PNM, QOI, PDF, PSD,
  MIFF, raw RGB, inline, txt; identify (format, ping, verbose), EXIF, statistics, histograms,
  every compare metric, sub-image search, montage, mogrify, the legacy `convert`/`composite` —
  each run in the guest and by the reference, their output and what they print compared: all
  194 files have the same pixels (45 the same bytes, AVIF included; the others differ only in the
  dates PNG and PDF record), all printed text is the same. Then PerlMagick's 94 sample files of many formats,
  read alike (82 decoded); every command of the guide and the skill, run in the guest; and what
  only the guest shows: the files and links installed, the version, delegates and formats, the
  limits and threads, the fallback font (with `-define`, and off), a delegate program started by
  `posix_spawn`, write errors on a full device, pipes and `-script` from a descriptor, error
  messages, the pixel cache on disk, one thread giving the same pixels as two, `-seed`, SVG
  without Inkscape, HEIC not written (and said so); and speed (below).
- `test_suite.py` (2 minutes): ImageMagick's own `make check` — `tests/*.tap`: the validate
  program's suites (conversions, formats in memory, on disk and mapped, colorspaces, composites,
  compares, montages, identify, import, stream), drawtest and wandtest of the MagickWand API, the
  command line's colorspace, EXIF, HEIC/AVIF, mogrify, pipe, PCX, statistic and SVG checks — in the guest
  (`build/wasm/im`, the build folder mounted at its own path) beside the reference: 104 of 104
  TAP checks. All pass but
  three probes of `cli-pipe.tap` the guest's shell answers otherwise, whose subjects
  `test_guest.py` checks: BusyBox ash (no `fork`) cannot duplicate a pipeline's standard input
  (`5<&0`), and BusyBox's `printf` returns 0 after a failed write to `/dev/full`. The Ultra HDR
  checks skip themselves in both builds (no libultrahdr).

The reference is the same sources and libraries built for this machine with clang-19 and
`-ffp-contract=off` (the guest has no fused multiply-add), so the output can be compared
exactly: the guest's C library is musl, the reference's glibc, and no case came out different in
its math. What differs by design: random numbers with `-seed` are hashed from an `unsigned long`,
4 bytes on wasm32 as in any 32-bit build, so the guest's seeded noise is that of 32-bit builds,
not of this 64-bit machine (it is the same from run to run).

Speed, a 12-megapixel photo in the guest with its default 2 CPUs (natively, 5 threads on this
6-core aarch64 machine): resize to 1600×1200 0.9 s (0.16 s), JPEG 1.5 s, WebP 3.2 s, AVIF 3.6 s
(1.3 s at `heic:speed=8`), `-gaussian-blur 0x3` 12 s (2.9 s), rotate 15° 3.7 s (1.5 s),
`identify -verbose` 20 s (9 s: its statistics are single-threaded upstream too); a command's first
start in a fresh guest compiles the program (0.7 s). The blur takes 17 s on one thread;
`--cpus` gives the guest more.

The guest ignores `SIGPIPE` (its processes start with it ignored), so `magick … txt:- | head`
reports *unable to write file `-'* when `head` closes the pipe, where natively the process would
end quietly; the guide tells agents to write to a file instead.
