# c-docu add-on

`docu` (also `collabo-docu`) in the sandbox: office documents on the command line, written for
agents so that they do not reach for Python to change a Word, Excel or PowerPoint file. It reads,
edits, creates, converts and renders `.docx`, `.xlsx`, `.pptx` (and their template and macro
variants) and the older binary `.doc`, `.xls`, `.ppt`.

The point is **partial editing**. `docu` does not load a document into a model and write a new
file from it: it changes the text, run, cell, paragraph, style or record an edit names and copies
everything else through as it was — the other parts of a package byte for byte (their compressed
bytes), the XML it did not touch as it was written (attribute order, prefixes, whitespace,
unknown elements and extensions), the records of a binary file it did not change. Comments,
tracked changes, macros, charts, custom XML, signatures' other parts survive an edit.

```
$ docu read report.docx p3 --runs
p3: The quick brown fox jumps over the lazy dog in 2023.
    r1 @0 "The "
    r2 @4 "quick" bold
    r3 @9 " brown fox jumps over the lazy dog in 2023."
$ docu replace report.docx 2023 2024
replaced 8 occurrences in p1, p3, p6, p8, p16, p18, hdr1.p1, fn1.p1
saved report.docx
$ docu style report.docx p3 bold color=#C00000 --match "lazy dog"
styled p3
saved report.docx
$ docu set book.xlsx 'Summary!B2' 1500        # dependent formulas recalculated on save
$ docu render deck.pptx -o slides.png         # slides-1.png, slides-2.png
```

| In the guest | What |
|---|---|
| `/usr/bin/docu`, `/usr/bin/collabo-docu` | the program (34 MB of wasm) |
| `/usr/share/docu/fonts` | fonts for rendering: metric-compatible stand-ins for Office's (Carlito for Calibri, Caladea for Cambria, Liberation Sans/Serif/Mono for Arial, Times New Roman, Courier New), Nanum Gothic and Myeongjo (Korean), Droid Sans Fallback (Chinese characters — Hanja, Kanji, Hanzi — and kana), DejaVu Sans and Mono |
| `/usr/share/doc/docu/AGENTS.md` | the guide for agents: the read–edit–check loop, addresses, every command, per-format notes, pitfalls |
| `/etc/codex/skills/office-docs/SKILL.md`, `/root/.agent/skills/office-docs/SKILL.md`, `/root/.claude/skills/office-docs/SKILL.md` | the guide as a skill, for Codex, agent and Claude Code |

The image is 60 MB, so the add-on is not bundled in the runtime (`addons/README.md`); an app
takes `out/c-docu.cpio` and `out/c-docu.json`.

## Turning it on

```json
"addons": {"c-docu": true}
```

or `--addon-dir addons/c-docu/out --addon c-docu` on the engine's command line. It has no
settings. Documents in a mounted folder (`--mount`) are edited where they are.

## What it does

`docu help` has the reference; the guide (`share/doc/AGENTS.md`) the details.

- **look**: `info`, `read [ADDR...] [--runs] [--json]` (every paragraph, cell, shape, header,
  footnote, comment with its address), `text`, `find`.
- **edit in place**: `replace` (text across runs, keeping their formatting; regex, case, whole
  word, `--in`, `--nth`, `--expect`), `set`, `style` (characters, paragraphs, cells, shapes;
  `--match` for part of a paragraph), `insert` (paragraphs, table rows, sheet rows/columns),
  `delete`, `table`, `image` (insert or replace pictures), `chart` (a chart's data and title;
  the embedded data sheet kept in step; a workbook's charts follow its cells), `textbox`, `slide`
  (add/dup/delete/move/hide), `sheet` (add/rename/delete/hide), `recalc`, `batch`.
- **make**: `new` (docx with Word's usual styles and lists, xlsx, pptx with a theme, master and
  six layouts), `convert` (doc → docx, xls → xlsx, ppt → pptx; any → PDF, PNG, SVG, Markdown,
  text, CSV, JSON), `render` (PDF, PNG, SVG).

Addresses: Word `p3`, `p3[2:9]`, `t1.r2.c3`, `hdr1.p1`, `fn1.p1`, `cmt0`; Excel `Sheet1!B3`,
`Sheet1!A1:C9`, `Sheet1!3:5`, `Sheet1!B:B`; PowerPoint `s2.sh3.p1`, `s2.sh3.r1.c2`,
`s2.sh[Title 1]`, `s2.notes`; `.doc` stories `hdr.p1`, `fn.p1`, `txbx.p2`.

## How it works

One Rust crate (`src/`, about 29,000 lines), no Office or LibreOffice code:

| | |
|---|---|
| `zip.rs`, `opc.rs`, `xml.rs` | the package: zip entries copied raw unless changed; parts, relationships, content types; a lossless XML tree (each element keeps its source text until changed; namespaces resolved, Strict OOXML read as Transitional) |
| `text.rs` | text across runs (Word, DrawingML and SpreadsheetML rich text share it): segments, finding and replacing over run boundaries, splitting runs to style part of one, keeping `xml:space` right |
| `docx.rs` | Word: stories (body, headers, footers, footnotes, endnotes, comments, text boxes and their `mc:Fallback` copies), addresses, edits, tables, lists (numbering definitions reused or made), styles' next style |
| `xlsx.rs`, `xlsx_ops.rs`, `formula.rs`, `numfmt.rs` | Excel: cells, shared strings (forked when a shared one changes), styles (cell formats derived and shared), number formats as Excel shows them, a formula parser and evaluator (~200 functions) that recalculates what an edit changes and writes the cached values; rows/columns inserted and deleted with every reference moved (formulas, names, merges, conditional formats, validation, tables, drawings, charts) |
| `pptx.rs`, `dml.rs` | PowerPoint: shapes, groups, tables, placeholders inherited from layouts and masters, slides added from layouts, duplicated, moved, deleted |
| `chart_edit.rs` | charts' data: series, categories and the cells they read; new data written into the cached values and the embedded workbook (and its table), series added and removed; a workbook's charts brought up to date with its cells on save |
| `emf.rs` | Windows metafiles (EMF, WMF) as SVG for rendering: a GDI player (objects, saved states, window/viewport and world transforms, map modes) drawing lines, shapes, polygons, Béziers, paths, text (with the fonts' character sets: Shift-JIS, CP949, GBK, Big5, …) and bitmaps; EMF+ records skipped (dual files' GDI records are drawn) |
| `picture.rs`, `blank.rs` | pictures (PNG, JPEG, GIF, BMP, TIFF, EMF, WMF sizes and resolutions) inserted or swapped; new documents |
| `cfb.rs` | OLE2 compound files: read, and written back keeping every stream and directory entry |
| `word97.rs`, `word97_docx.rs` | `.doc`: the piece table; edits written as Word's own fast saves (new text and formatting pages appended, the piece table rewritten, every story's positions moved); character and paragraph properties, styles, lists, sections, tables, headers, footnotes and pictures read for conversion |
| `xls.rs`, `biff.rs`, `convert.rs` | `.xls`: BIFF8 records, the shared string table and cell blocks rebuilt, formulas decompiled, compiled and recalculated; conversion to `.xlsx` |
| `ppt.rs`, `ppt_pptx.rs` | `.ppt`: the record tree, persist directory and user edits remapped on save, text and its style runs edited; conversion to `.pptx` (shapes, pictures, text with the masters' styles) |
| `render/` | rendering: the document as Typst source, laid out and drawn by the Typst library (0.15.1, as in the typst add-on): Word's line metrics (OS/2 ascent and descent), spacing, tabs, lists, tables, headers and footers, footnotes, floating shapes; PowerPoint's placeholders, text styles, autofit, preset shapes (LibreOffice's `presetShapeDefinitions.xml`); Excel's column widths, number formats, fills, borders, merges; charts (bar, column, line, area, pie, doughnut, scatter, radar, combinations; Excel's axis scaling); `.doc`/`.xls`/`.ppt` through their conversions |

## How it is built

`build.sh` (stages, each skipped when up to date; `FORCE=fetch,fonts,src,cross,cpio`, `JOBS=N`,
`TEST=1` for the unit tests, built natively):

- **fetch** — what `sources.lock` names (fonts, LibreOffice's preset shape definitions), checked
  by sha256, into `build/dl`.
- **fonts** — `build/fonts` from the downloads.
- **src** — the crate copied to `build/src`; crates whose `wasm32` cfgs mean "a browser" narrowed
  to wasm that is not Linux (parking_lot_core, time) and `patches/crates` applied (fontdb reads
  font files instead of mapping them: the guest has no `mmap`), through `[patch.crates-io]`.
- **cross** — `docu` for `wasm32-unknown-linux-musl` with the guest's Rust toolchain
  (`python/build-rust-toolchain.sh`), release, `panic=abort`, no LTO.
- **cpio** — `out/c-docu.cpio`, `out/c-docu.json`, `out/licenses/` (the fonts' licenses,
  `crates.txt`: every crate linked in with its license).

Natively, for development and the checks (on this aarch64 machine, with clang-19 as the linker):

```sh
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=clang-19 CC_aarch64_unknown_linux_gnu=clang-19 \
  cargo build --target-dir build/host
DOCU_FONT_PATH=build/fonts DOCU_SYSTEM_FONTS=0 build/host/debug/docu render f.docx -o f.png
```

`DOCU_FONT_PATH` adds font folders; `DOCU_SYSTEM_FONTS=0` leaves out the system's (so a render
uses what the guest has).

## Checks

`tests/run.sh` (`--host` for the native checks only):

- `test_host.py` — the native build: that edits are partial (a replace in a `.docx` changes only
  `word/document.xml`, and only inside the paragraph; a cell set leaves the shared strings and
  styles alone; a slide edit changes only that slide), formulas recalculated, rows inserted with
  references moved, the old formats edited, new documents, conversions, rendering; when
  LibreOffice is there (`DOCU_SOFFICE`, `soffice` on `PATH`, `~/.local/bin/soffice`), that it
  opens what docu wrote and sees the edits (`.xlsx`, `.doc`, `.xls`, `.ppt`).
- `test_guest.py` — in the guest (the runtime in `dist/runtime`, `COLLABO_RUNTIME`, `DOCU_ENGINE`,
  `DOCU_KERNEL`, `DOCU_CPIO`): what the add-on installs, and `tests/edits.sh` — 75 commands
  over all six formats, reading, editing, making, converting and rendering — run in the guest
  and natively: the same output and the same files, byte for byte.

`tests/fixtures.py` makes the test documents as Office writes them (runs split mid-word, rsids,
fields, hyperlinks, text boxes with fallbacks, footnotes, comments, tracked changes, merged
cells, shared formulas, a theme, a master and layouts); `tests/legacy` holds LibreOffice's
`.doc`, `.xls` and `.ppt` saves of them.

## Not there (yet)

- Rendering is close to Office's but not identical: text does not flow around floating shapes
  (they stand beside their paragraph), metafiles are drawn without clipping and EMF+-only ones
  not at all, SmartArt only from its saved drawing, no equations' layout, 3-D charts drawn flat.
- Editing: chart formatting (colors, axes) is not edited, only data and titles; comments and
  tracked changes (read as accepted text) are not edited;
  `.doc`/`.xls`/`.ppt` take text, character formatting and cells — convert them for more.
- No ODF (`.odt`, `.ods`, `.odp`) or HWP/HWPX.
- Encrypted (password-protected) files are refused.
