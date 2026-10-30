# guided-arrow

`guided-arrow`: verified command-line cheat sheets in the guest, offline — the
[tldr pages](https://github.com/tldr-pages/tldr) in English, and the sheets
[cheat.sh](https://github.com/chubin/cheat.sh) serves ([chubin/cheat.sheets](https://github.com/chubin/cheat.sheets),
[cheat/cheatsheets](https://github.com/cheat/cheatsheets)) — from one archive of its own compression.
It is meant for agents first: where a web search or a RAG lookup hands back whatever text matched,
this hands back examples from reviewed repositories, each traceable to its upstream file at a
pinned commit, under its license.

```sh
guided-arrow tar                         # tldr's page, then cheat.sh's sheets for tar
guided-arrow tar extract gz              # only the examples that mention "extract" and "gz"
guided-arrow 'tar~list'                  # the same, as cheat.sh writes it
guided-arrow git commit                  # the git-commit page
guided-arrow -s find files modified in the last day    # examples for a task, from every sheet
guided-arrow python/lambda               # a programming language's sheet; python/ lists them
guided-arrow -p osx cal                  # the macOS page first (default: linux, then common)
guided-arrow --info tar                  # where each page came from: path, commit, URL, license, git blob id
guided-arrow --json -s convert video to gif           # the same as JSON, with provenance
guided-arrow -l docker                   # page names that start with docker
guided-arrow --verify                    # decompress everything, check every checksum
guided-arrow --add kubectl-secret 'Create a secret from literal values:' \
  'kubectl create secret generic {{name}} --from-literal={{key}}={{value}}'   # a sheet of one's own
guided-arrow --remove kubectl-secret literal                 # ... or only its examples that say so
```

An agent with a shell needs nothing else (`guided-arrow --help` is written for it); an app on the
Dart package gives its model `CheatTools` (below).

## Output

```
=== tar [tldr: common] ===
# Archiving utility.
# Often combined with a compression method, such as `gzip` or `bzip2`.
# More information: https://www.gnu.org/software/tar/manual/tar.html

# E[x]tract a (compressed) archive [f]ile into the current directory [v]erbosely:
tar xvf {{path/to/source.tar.ext}}
...
=== tar [cheat.sheets] ===
...
```

Every page starts with `=== NAME [SOURCE: PLATFORM] ===` (and notes such as `(also: osx)`,
`(no ko translation)`, `(no linux or common page)`); descriptions are `#` comments (a language
sheet's own comment marker), commands are plain lines, so a page reads like a shell script. A
whole cheat sheet is shown as upstream wrote it; a tldr page in that shape. In tldr's commands
`{{...}}` marks a value to fill in — kept when the output is not a terminal (or `--braces`), shown
underlined without the braces on one; option pairs `{{[-C|--directory]}}` show the long form
(`-S` short, `-E` long, both: `[-C|--directory]`), as tldr's client specification asks.

Lookup: words are joined with dashes to the longest page name they make (`git commit` →
`git-commit`); the rest filter the examples (those that mention them all; else those that mention
the most, said so in the header). tldr's page is chosen platform before language, as its clients
do: `--platform` (default the host's, `linux` in the guest), `common`, then any other platform.
The pages are English. An archive packed with tldr's translations (Building, below) also picks the
language as tldr's clients do, from `--language`, `$GUIDED_ARROW_LANG`, `$LANGUAGE`/`$LANG`, and
falls back to English where a page has no translation. A language the archive does not have at
all is English too: from `--language` with a note on stderr, from the environment without one.
A name that is not there exits 1 with the close names on stderr and the examples that best match
the words on stdout.

Search (`-s`): every example and every page's summary (in an archive with translations, those of
the language asked for too) is scored BM25-style — a word matches the words it begins (`extract`
matches `extracted`, tldr's `E[x]tract` counts as `extract`), stop words and common English
endings are dropped, examples that mention every word come first — and the same command from
several pages (platforms, translations) is shown once. There is no index to carry: the blocks are
decompressed and scanned, a page none of whose text holds a word is not even parsed. tldr pages of
another platform are searched only where no page of this platform has the name (brew, winget).

Exit status: 0 found, 1 not found (closest printed), 2 usage, 3 the archive (missing, damaged).
`--json` gives pages (summary, examples, see-also, upstream: repository, path, commit, date, URL,
license), search results (with scores) and lists as one JSON object.

## Sources

| Source | What | Pinned (build.sh) | License |
|---|---|---|---|
| tldr-pages/tldr | 7,601 English pages (common, linux, osx, windows, freebsd, android, ...); its 39 translations (32,569 more pages, Korean 6,959) only when asked for | `TLDR_REV` e04d778 (2026-10-28) | CC BY 4.0 |
| chubin/cheat.sheets | cheat.sh's own: 433 command sheets, 139 programming language sheets (`_python/lambda` → `python/lambda`), 11 topic lists ("tools for a task"), 714 see-also lists | `CHEAT_SHEETS_REV` 0bcedfb (2025-03-21) | MIT |
| cheat/cheatsheets | the sheets of `cheat`, which cheat.sh also serves: 279 | `CHEATSHEETS_REV` 36bdb99 (2022-12-17) | CC0 1.0 |

cheat.sh itself is a server: these three are the static sheets it serves (its other sources —
StackOverflow answers, Rosetta Code, learnxinyminutes — are searches or tutorials, not cheat
sheets). Pages are stored byte for byte as upstream has them (`--raw` prints them so); `--info
NAME` gives each one's git blob id, the same id `git ls-tree` shows upstream at that commit, so a
page can be checked against its repository. The licenses ship as `addon-guided-arrow-*` in the
runtime's `licenses/`; the program itself is the add-ons' GPL-3.0.

## Sheets of one's own

What the archive lacks — a team's conventions, a tool it has no page for (`az keyvault`, `gcloud
secrets`), an example that was checked to work here — goes in sheets of one's own: plain files in
a folder, in the cheat format, read at every run. They are the source `local`, shown before the
archive's pages of the same name (`=== tar [local] ===`, then tldr's) and searched with them;
`--from local` shows only them, `--no-local` only the archive's.

```sh
guided-arrow --add NAME 'DESCRIPTION' 'COMMAND'      # one example (a new sheet, or added to it)
guided-arrow --add NAME 'DESCRIPTION' -- --a-command-starting-with-a-dash
guided-arrow --add NAME < sheet.txt                  # a sheet's text: # description lines over the commands
guided-arrow --remove NAME                           # the sheet
guided-arrow --remove NAME WORD...                   # only its examples that mention all the words
guided-arrow -l --from local                         # the sheets there are; --info: their folders
```

```
# Create a secret from literal values:
kubectl create secret generic {{name}} --from-literal={{key}}={{value}}

# Show one value of a secret, decoded:
kubectl get secret {{name}} -o jsonpath={{.data.key}} | base64 -d
```

A name is a command's name as it is looked up (lower case, spaces as dashes: `--add 'az keyvault'`
writes `az-keyvault`; `python/walrus` makes a folder). `--add NAME DESCRIPTION COMMAND` keeps the
same command once and checks that what it wrote reads back as the example given (a command with a blank line, or a line
starting with `#`, goes in through stdin), and writes through a temporary file; `--remove` cuts
the examples out as they were written and prints them, so they can go back in. Only these sheets
can be removed — the archive's stay as upstream wrote them.

The folder, the first of these that is set:

| | |
|---|---|
| `$GUIDED_ARROW_SHEETS` | folders, `:`-separated; `--add` writes to the first |
| the add-on's `sheets` setting | `/etc/collabo/addons/guided-arrow.json`, from the app: a folder it mounts, so the sheets outlive the sandbox (`--addon-config guided-arrow:sheets=/sheets` on the engine's command line) |
| `${XDG_DATA_HOME:-~/.local/share}/guided-arrow/sheets` | in a sandbox `/root/...`, gone when it stops — `--add` says so |

and, read only, `sheets/` beside the archive (`/usr/share/guided-arrow/sheets`): sheets that the
image or another add-on's overlay puts there. A file over 1 MiB, or with a NUL byte, is not a sheet
(left out, said so on stderr). `--info NAME` gives a local sheet's file and git blob id; `--verify`
counts the local sheets and names those with no examples.

Sheets can also go into the archive itself, when it is built: `GUIDED_ARROW_EXTRA="team=DIR"`
packs a folder as a source of that name (below), checksummed and compressed with the rest.

## The archive and its compression

`/usr/share/guided-arrow/guided-arrow.gar`, found next to the program (`../share/guided-arrow/`),
or given with `--data` / `$GUIDED_ARROW_DATA`. 9,177 pages, 5.2 MB of text, in 1.15 MB:

| | English (the default) | with every translation | page by page? |
|---|---|---|---|
| the pages as files | 5,176,623 (9,177 files) | 20,087,175 (41,746 files) | yes |
| gzip -9, all in one stream | 1,379,106 | 4,739,294 | no |
| bzip2 -9 | 1,127,313 | 3,671,470 | no |
| zstd -19 | 1,061,207 | 3,305,274 | no |
| zstd --ultra -22 --long | 1,060,932 | 3,190,080 | no |
| xz -9e | 1,047,264 | 3,126,540 | no |
| **guided-arrow.gar** (with its directory of names, sources, checksums) | **1,154,813** | **3,457,162** | **yes** |

```
"GARROW\0" version
block 0 .. block n-1     compressed, back to back
directory                compressed: sources, languages, platforms, blocks, pages
trailer                  directory offset, sizes, CRC-32, "GAR!"
```

- **The codec** (`rust/src/codec.rs`) is our own: LZ77 found by hash chains (3-byte, plus the last
  2-byte match) and parsed for price — a dynamic program over up to 4096 positions that weighs
  literals, matches, short repeats and the four repeat distances by what the adaptive models would
  charge for them now — coded bit by bit with adaptive binary models and a range coder. The models
  are the LZMA family's design (a literal right after a match coded against the byte at the
  match's distance, a 12-state history, length and distance-slot trees), the format and code
  ours. On a 1 MiB sample it is within 1% of xz -9e; it decodes at about 90 MB/s natively,
  45 MB/s in the guest.
- **Blocks of 2 MiB**, English first, every language's pages sorted by name, so a command's pages
  from the three sources and its platforms sit together. A page is found by decompressing its
  block (each checked against its CRC-32): pages are joined by NUL bytes, so no offsets are stored.
- **Translations take English as a preset dictionary** (when they are packed): a translated
  block is cut where English is and its matches may reach into the English block of the same
  names — the commands, URLs and option names it repeats: the archive with every translation is
  11% smaller for it (3.89 → 3.46 MB; Korean −20%, Spanish −15%, German −14%, Chinese −12%).
- **The directory** (names front-coded, then a column each for source, kind and platform) is 82 KB
  for 9,177 entries, 33 KB compressed (with every translation: 398 KB for 41,746, 64 KB).

Tried and left out: a Burrows–Wheeler transform with a context-mixing coder (order 0/1/2, a run
model, a logistic mixer, two APMs) came out 4% smaller than the LZ codec on English and 30 times
slower to decode (search decompresses everything); the adaptation rate (4, 5, 6) moves less than
1%; 4 MiB blocks would save another 2% for twice the work per lookup. In the measurements most of
the LZ codec's output is match distances (135 of 220 KB on the sample) — the text repeats in short
phrases (~13 bytes) from anywhere — which is where a next codec would have to win.

In the guest (2 CPUs, after the first start compiled the program): a lookup 0.03 s, a search
0.24 s, listing 0.01 s, `--verify` of all 5.2 MB 0.11 s. (With every translation: a lookup 0.06 s,
a search with Korean 0.33 s, `--verify` of 20 MB 0.43 s.)

## Building

```sh
addons/guided-arrow/build.sh                     # out/guided-arrow.cpio, .json, licenses/
TEST=1 addons/guided-arrow/build.sh              # with the unit tests first
GUIDED_ARROW_LANGS=en,ko addons/guided-arrow/build.sh   # tldr's Korean pages too (1.85 MB)
GUIDED_ARROW_LANGS=all addons/guided-arrow/build.sh     # every translation (3.46 MB)
GUIDED_ARROW_EXTRA="team=path/to/sheets" addons/guided-arrow/build.sh   # a folder of sheets too
FORCE=pack addons/guided-arrow/build.sh          # pack the archive again
```

One Rust program, no crates (`rust/`): built for this machine it packs the archive
(`guided-arrow --pack`), built for the guest it reads it. The sources are checked out at the
pinned commits into `build/src`, cloned from `GUIDED_ARROW_SRC` (default `~/workspace/guided-arrow`,
holding `tldr/`, `cheat.sheets/`, `cheatsheets/`) when that has them, else from GitHub. The archive
is packed again only when the packer, a commit, the languages or the extra sheets change (~10 s
for English, ~40 s with every translation, all cores). The default is English only (`--langs en`,
the packer's default too).

`GUIDED_ARROW_EXTRA` (space-separated `NAME=FOLDER`s) packs folders of sheets into the archive:
every file is a sheet in the cheat format, named by its path, shown as `[NAME]`; its revision is
the commit of the repository it is in (`+changes` with uncommitted ones), if any (`local` is
taken: it is the run-time sheets' name). The packer does the same on its own:

```sh
guided-arrow --pack -o mine.gar --source mine=path/to/sheets [--source tldr=... ...]
guided-arrow --data mine.gar deploy
```

## Tests

```sh
addons/guided-arrow/tests/run.sh          # unit tests; 61 scripted steps natively, 63 in the guest
addons/guided-arrow/tests/run.sh native   # without a runtime
```

The unit tests cover the codec (edge inputs, noise, long distances, the dictionary, damaged
input), the directory's encoding, CRC-32/SHA-1, tldr's placeholder rules from its client
specification, the page readers (and where each example is, for `--remove`), the search's words
and sheet names. `tests/test_guided_arrow.py` checks the
archive against the sources it was made from: `--raw` equals the upstream file, `--info`'s git
blob id equals `git rev-parse HEAD:pages/common/tar.md`; and lookups, filters, platforms,
languages (`-L`, `$LANG`: Korean pages when the archive has them, else English and a note), option
forms, colors, language sheets, topics, search, JSON, not-found, a damaged archive (exit 3),
packing a folder of one's own; and sheets of one's own: `--add` by arguments and stdin (the file it
writes, byte for byte), the same command twice, shown first, filtered, searched, listed, `--info`,
JSON, `--verify`, `--remove` of some examples and of a sheet, bad names and commands, `sheets/`
beside the archive, and in the guest the add-on's `sheets` setting and the warning without it.

## For apps: `CheatTools`

`dart/collabo_core` (`lib/src/tools.dart`, `addons/mod/0035`): `cheat_lookup` (name, words,
platform, source), `cheat_search` (query, limit, ...) and `cheat_info` (name) as LLM
tools, used like `TermTools`:

```dart
final cheat = CheatTools(sandbox);
// request body: {"tools": [...cheat.definitions, ...]}
final text = await cheat.call(block['name'], block['input']);
```

`CheatTools(sandbox, allowEdits: true)` adds `cheat_add` (name, description, command: an example
the model ran and checked) and `cheat_remove` (name, words). To keep what it adds, the app mounts
a folder and names it in the add-on's settings:

```dart
CollaboConfig(
  mounts: [Mount(hostPath: 'cheat-sheets', guestPath: '/sheets')],
  addons: {'guided-arrow': {'sheets': '/sheets'}},
)
```

Not found comes back as text starting with `Not found:` (with the closest names and examples),
failures with `Error:`. Its test (`cheat tools`) runs with a runtime that has the add-on and is
skipped otherwise.
