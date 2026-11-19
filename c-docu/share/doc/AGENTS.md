# docu in this sandbox — a guide for agents

`docu` (also `collabo-docu`) reads, edits and renders Word, Excel and PowerPoint files from the
command line: `.docx`, `.xlsx`, `.pptx` and the older binary `.doc`, `.xls`, `.ppt` (templates and
macro files — `.dotx`, `.xlsm`, `.pptm`, … — too). Use it instead of writing Python for documents.

What makes it different from rebuilding a file: **edits are partial**. `docu` changes only the
text, run, cell, paragraph or style you name and copies everything else through untouched — other
parts byte for byte, the XML it did not touch as it was written. Comments, tracked changes, macros,
charts, custom XML, unknown extensions all survive. So: edit the user's file in place; do not
re-create it.

`docu help` lists the commands; `docu help style` the formatting properties; `docu help
formulas` the spreadsheet functions.

## The loop: read, edit, check

```sh
docu read report.docx                 # every paragraph, table cell, header, footnote — with an address
docu find report.docx "2023"          # where a text occurs: p3[47:51] …the lazy dog in [2023].
docu replace report.docx 2023 2024    # in place; keeps each run's formatting
docu read report.docx p3 --runs       # check: the runs and their formatting
docu render report.docx -o check.png  # look at it (one PNG per page: check-1.png, check-2.png …)
```

1. **Read first.** `docu read FILE` prints the structure with addresses; pass addresses to see
   part of it (`docu read report.docx p3-8 t1`, `docu read book.xlsx 'Sales!A1:F20'`,
   `docu read deck.pptx s2`). `--runs` shows each run's formatting; `--json` gives the same as
   data; `docu text FILE` plain text; `docu info FILE` the format and counts.
2. **Edit with the narrowest command**: `replace` for text (it finds text across runs and keeps
   their formatting), `style ... --match TEXT` for the look of some words, `set` for a whole
   paragraph or cell, `insert`/`delete` for structure.
3. **Check**: read the address again; `render` and look at the page when the layout matters.
   `--dry-run` shows what an edit would do without writing; `--expect N` makes `replace` fail
   (and write nothing) unless it finds exactly N matches; `-o OUT` writes a copy instead.
4. **Addresses move** after `insert` and `delete` (p5 becomes p6). Read again before the next
   structural edit, or put the edits in one `batch` (each line sees the previous ones' result).

Every edit prints what it did (`replaced 3 occurrences in p3, p9, t1.r2.c1`) and `saved FILE`.
Errors say what was wrong and write nothing.

## Addresses

| | |
|---|---|
| Word | `p3` (paragraph 3 of the body), `p3-7`, `p3[2:9]` (characters 2..8 of p3), `t1` (table 1), `t1.r2`, `t1.r2.c3`, `t1.r2.c3.p2`, `hdr1.p1` / `ftr1.p1` (headers, footers), `fn2.p1` (footnote 2), `en1` (endnotes), `cmt0` (comment 0), `body` |
| Excel | `Sheet1!B3`, `B3` (first sheet), `'My sheet'!A1:C9`, `Sheet1!B:B` (columns), `Sheet1!3:5` (rows), `Sheet1` (its used range) |
| PowerPoint | `s2` (slide 2), `s2.sh3` (shape 3), `s2.sh3.p1` (its paragraph 1), `s2.sh3.sh1` (in a group), `s2.sh3.r1.c2` (table cell), `s2.notes`, `s2.sh[Title 1]` (a shape by name) |
| .doc | `p3`, `hdr.p1` (headers and footers), `fn.p1`, `en.p1`, `txbx.p2` (text boxes), `cmt.p1` |
| .ppt | `s1.sh2.p1`, `s1.notes`, `s1.text` |
| .xls | as Excel |

Text in `read` output: `\n` is a line break inside a paragraph, `\t` a tab, `[^fn1]` a footnote
reference, `[image]`, `[chart]`, `[text box]` objects. Text you pass: `\t` is a tab. When you set
**one paragraph**, `\n` is a line break and a blank line (`\n\n`) starts another paragraph; when you
**insert**, or set a whole shape, cell or notes, each line becomes a paragraph.

## Editing

```sh
docu replace FILE OLD NEW [--in ADDR] [--nth N] [--expect N] [-i] [-w] [--regex]
docu set FILE ADDR TEXT
docu style FILE ADDR PROP... [--match TEXT [--nth N]]
docu insert FILE ADDR TEXT [PROP...] [--before]
docu delete FILE ADDR...
docu table FILE ADDR "a|b|c;1|2|3" [--header]       # Word: a table after paragraph ADDR (or a CSV file)
docu image FILE ADDR picture.png [w=8cm] [alt=...]   # Word paragraph, slide, or cell
docu batch FILE script.txt                           # or: docu batch FILE -e 'replace a b' -e 'style p1 bold'
```

- `replace` works everywhere text is: body, tables, headers, footers, footnotes, text boxes,
  slides, notes, a workbook's cells (`--formulas` to replace in formulas too). Comments are
  left alone unless you name them: `--in cmt0` (`--in cmt` in a .doc).
  `--regex` takes Rust regex syntax; `$1` in NEW is a group. `-i` ignores case, `-w` matches
  whole words. `--in p3-9` (or `--in Sheet2`, `--in s4`) limits it.
- `style` properties (`docu help style`):
  - characters: `bold italic underline[=double|wave|none] strike color=red|#C00000 size=14pt
    font=NAME "font.ea=맑은 고딕" highlight=yellow bg=#FFF2CC super sub caps link=URL clear`
  - paragraphs: `align=center|right|justify style="Heading 1" indent=1cm indent.first=0.5cm
    before=6pt after=12pt line=1.5|exact:14pt keepnext pagebreak list=bullet|number|none level=2`
  - cells and shapes: `fill=#DDEBF7 numfmt="#,##0.00" valign=middle wrap border="single 1pt black"
    border.bottom=... colwidth=18 rowheight=24pt x= y= w= h= rotate=90 line=COLOR name= alt=`
  - `=false` turns a flag off (`bold=false`); lengths take `pt cm mm in`.
  - `--match "lazy dog"` styles only that text (every occurrence in ADDR, or `--nth 2`);
    `p3[4:9]` styles characters 4..8.
- `insert` adds paragraphs after ADDR (before with `--before`) with ADDR's paragraph look (after a
  heading, Word's next style: Normal; inside a list, the list continues — `list=none` to leave
  it). Properties after the text style the new paragraphs: `docu insert f.docx p2 "Risks" style="Heading 2"`.
  A row address inserts rows: `docu insert f.docx t1.r2 "a|b|c"`, `docu insert book.xlsx
  'Sales!5:5' "x|1|2"` (rows after row 5; `--count 3` for more), columns with `Sales!C:C`.
- `delete` removes paragraphs, character ranges (`p3[0:6]`), tables, rows, shapes, slides, sheet
  rows/columns (`Sales!4:6`, `Sales!C:C`), cell contents (`Sales!B3:B9`).

## Excel

```sh
docu read book.xlsx 'Sales!A1:F12'           # values as shown, {=formula}, {value=raw}
docu set book.xlsx 'Sales!B3' 1250           # numbers, TRUE/FALSE, 2024-10-16 (a date), text
docu set book.xlsx 'Sales!D3' '=SUM(B3:C3)'  # a formula
docu set book.xlsx 'Sales!A1' 'Item|Q1|Q2;Rent|300|310'   # a block from A1: | between cells, ; between rows
docu style book.xlsx 'Sales!B2:D9' numfmt='#,##0' bold fill=#DDEBF7
docu style book.xlsx 'Sales!A:A' colwidth=24
docu sheet book.xlsx list|add NAME|rename OLD NEW|delete NAME|hide NAME|show NAME
```

- **Formulas are recalculated on save**: every cell that depends on what you changed gets its new
  value written into the file, so readers that do not calculate (and `docu read`) see the right
  numbers; Excel also recalculates on open. `docu recalc FILE` recalculates everything.
  Around 200 functions are known (`docu help formulas`); a formula using another keeps its old
  cached value and `docu` warns.
- Inserting or deleting rows and columns moves formulas, merged cells, conditional formats, data
  validation, defined names, tables, charts' ranges and pictures as Excel does.
- Number formats show as Excel shows them (`1,250.00`, `25.6%`, `2024-10-16`); `{value=...}` is
  the stored number.

## PowerPoint

```sh
docu read deck.pptx s2                       # shapes: placeholder type, name, box (x,y w×h cm)
docu set deck.pptx s2.sh2 "First point
Second point"                                # a whole body: one paragraph per line
docu style deck.pptx s2.sh2.p2 level=2       # indent a bullet
docu slide deck.pptx add --layout "Title and Content" --title "Next steps"   # after the last
docu slide deck.pptx add 3 --layout "Title Only"   # after slide 3
docu slide deck.pptx dup 2 | delete 4 | move 5 2 | hide 3 | list
docu textbox deck.pptx s3 "Draft" x=2cm y=1cm w=6cm h=1.5cm size=24pt color=red
docu image deck.pptx s3 chart.png x=15cm y=4cm w=12cm   # a picture; on a picture shape: replace it
docu set deck.pptx s2.notes "Speaker notes here"
```

New slides take their layout's placeholders; fill them with `set s4.sh1 ...`, `set s4.sh2 ...`
(read the slide to see which is which).

## Charts

```sh
docu chart deck.pptx s3.sh4                  # kind, title, categories, series and the cells they read
docu chart deck.pptx s3.sh4 data "Quarter|Sales|Cost;Q1|8|5;Q2|3|2;Q3|9|4"
docu chart deck.pptx s3.sh4 title "Sales and cost"     # or: title none
docu chart report.docx p12                   # Word: the paragraph holding the chart
docu chart book.xlsx Dashboard --nth 2       # Excel: a sheet's charts, in order
```

`data` takes a table: the first row names the series (its first cell is the categories'
heading), each other row is a category and its values — the same table `docu chart` prints on
its `data:` line, so read it, change it, give it back. Series are added or removed to match (a
new one looks like the last). The chart's embedded data sheet (what PowerPoint's and Word's *Edit
Data* opens) is rewritten to match. In a workbook a chart shows cells: change them with `docu
set`, and its cached values follow on save; `docu chart` shows which cells it reads.

## Word specifics

- Lists: `style FILE p4-7 list=bullet` (or `number`; `level=2` nests), `list=none` ends one.
- Headings: `style=Heading1` or the style's name, `"Heading 1"` (`docu read` shows each
  paragraph's style in brackets).
- Tables: `docu table FILE p5 "Item|Cost;Servers|20k" --header`; `style FILE t1.r1 fill=#DDEBF7
  bold`; `insert FILE t1.r3 "Travel|5k"` adds a row like row 3.
- Pictures: `docu image FILE p3 photo.jpg w=10cm align=center` (a new paragraph after p3);
  `docu image FILE p8 new.png --replace` swaps the picture in p8, keeping its width.
- Tracked changes: `read` shows the text as it reads with changes accepted; edits are not tracked.

## The old formats (.doc, .xls, .ppt)

They are edited in place in their own binary format: `replace`, `set`, `style` (characters:
bold, italic, underline, size, color, highlight …), `insert` and `delete` of paragraphs in .doc;
`set`/`replace`/`delete` of cells (formulas recalculated) in .xls; `replace`, `set`, character
`style` and `delete` in .ppt. For anything more (tables, pictures, new slides, rows), convert and
work on the new file:

```sh
docu convert old.doc new.docx      # also xls → xlsx, ppt → pptx
```

## Making documents

```sh
docu new report.docx "Quarterly Plan"            # styles Title, Heading 1-4, List Paragraph, Table Grid…
docu style report.docx p1 style=Title
docu insert report.docx p1 "Goals" style="Heading 1"
docu insert report.docx p2 "Ship 2.0
Hire two engineers" list=bullet
docu new book.xlsx && docu set book.xlsx A1 "Item|Qty|Price;Apple|3|1.5"
docu new deck.pptx "Launch Review" "October 2026"   # 16:9, a title slide; layouts: Title Slide,
                                                    # Title and Content, Section Header,
                                                    # Two Content, Title Only, Blank
```

`docu new` refuses to overwrite a file (`--force` replaces it).

## Rendering and converting

```sh
docu render report.docx -o report.pdf
docu render deck.pptx -o slides.png --pages 2-4 --ppi 96    # slides-2.png … (or 'p{p}.png')
docu render book.xlsx -o sheets.png                          # one page per sheet with content
docu convert report.docx report.md     # Markdown (headings, lists, tables); also .txt .json
docu convert book.xlsx sales.csv --sheet Sales
```

Rendering lays the document out the way Office does — fonts, spacing, tables, headers and footers,
footnotes, lists, shapes, pictures, charts — with metric-compatible fonts when Office's are not
there (Carlito for Calibri, Caladea for Cambria, Liberation for Arial/Times New Roman/Courier New,
Nanum for Korean), so line breaks fall where Office puts them. Look at the PNGs to check layout;
do not promise pixel-identical output to Office. Warnings name what was not drawn exactly.

## Pitfalls

- Quote addresses with `!` or spaces for the shell: `'Sales!B3'`, `'s2.sh[Title 1]'`.
- `set` replaces the whole text of its target — for part of a paragraph use `replace` or
  `p3[a:b]`; `set` on a cell range writes the block starting at its first cell.
- A replace that finds nothing fails and writes nothing; check with `docu find` first.
- `replace` does not touch formulas unless `--formulas`; numbers in cells are numbers, not text
  (`set` them).
- Merged cells are addressed by their top-left cell.
- Files with a password (encrypted) cannot be opened.
