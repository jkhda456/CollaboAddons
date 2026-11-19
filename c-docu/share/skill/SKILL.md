---
name: office-docs
description: Read, edit, create, convert and render Word, Excel and PowerPoint files (.docx .xlsx .pptx and old .doc .xls .ppt) with the `docu` command - change text, formatting, cells, formulas, tables, lists, pictures, slides and sheets in place without rewriting the rest of the file, make new documents, export PDF/PNG/Markdown/CSV. Use whenever a task involves an office document; do not write Python for it.
---

# Office documents with docu

`docu` edits office files **in part**: only what you name changes; every other part, run and
setting stays as it was (comments, tracked changes, macros, charts survive). Edit the user's
file in place; never re-create it from scratch. Full guide: `/usr/share/doc/docu/AGENTS.md` —
read it before the first larger task. `docu help`, `docu help style`, `docu help formulas`.

## Steps

1. **Read with addresses**: `docu read FILE` (or `docu read FILE p3-9 t1`, `'Sheet1!A1:F20'`,
   `s2`). `--runs` shows formatting, `--json` data. `docu find FILE TEXT` locates text.
2. **Edit with the narrowest command**:
   - text: `docu replace FILE OLD NEW` (keeps formatting; `--in ADDR`, `--nth N`, `--expect N`,
     `-i`, `-w`, `--regex`)
   - look of some words: `docu style FILE p3 bold color=#C00000 --match "net profit"`
   - whole paragraph/cell/shape: `docu set FILE ADDR TEXT`
   - structure: `docu insert FILE ADDR TEXT [style="Heading 2"|list=bullet]`, `docu delete FILE ADDR`
   - Word tables: `docu table FILE p5 "a|b;1|2" --header`; pictures: `docu image FILE ADDR pic.png w=8cm`
   - Excel: `docu set FILE 'Sheet1!B3' 1250`, `'=SUM(B2:B9)'`, blocks `"a|b;c|d"`;
     `docu style FILE 'Sheet1!B2:D9' numfmt='#,##0' fill=#DDEBF7`; rows `docu insert FILE 'Sheet1!5:5' "x|1|2"`;
     `docu sheet FILE add|rename|delete ...` — formulas are recalculated on save
   - PowerPoint: `docu set FILE s2.sh2 "line 1\nline 2"` (one paragraph per line),
     `docu slide FILE add --layout "Title and Content" --title T`, `dup|delete|move|hide`,
     `docu textbox FILE s3 TEXT x= y= w= h=`, `docu set FILE s2.notes TEXT`
   - charts: `docu chart FILE s2.sh4` (or a Word paragraph, an Excel sheet) shows the data;
     `docu chart FILE s2.sh4 data "Quarter|Sales|Cost;Q1|8|5;Q2|3|2"`, `... title "T"`;
     in Excel set the cells the chart reads (it follows on save)
   - several edits at once: `docu batch FILE -e 'replace a b' -e 'style p1 bold'`
3. **Check**: `docu read FILE ADDR` again; for layout, `docu render FILE -o check.png` and look at
   the pages. `--dry-run` previews; `-o OUT` writes a copy.
4. Tell the user what changed (the command prints where), and the file's path.

## Notes

- Addresses: Word `p3`, `p3[2:9]`, `t1.r2.c3`, `hdr1.p1`, `fn1.p1`; Excel `Sheet1!B3`,
  `Sheet1!3:5`, `Sheet1!B:B`; PowerPoint `s2.sh3.p1`, `s2.sh[Title 1]`, `s2.notes`.
  They move after insert/delete — read again (or use one `batch`).
- Quote shell specials: `'Sheet1!B3'`, `'s2.sh[Title 1]'`, `numfmt='#,##0'`.
- New files: `docu new report.docx "Title"`, `docu new book.xlsx`, `docu new deck.pptx "Title" "Subtitle"`.
- Old formats edit in place (text, characters' style, cells); for more, `docu convert old.doc new.docx`
  (`xls → xlsx`, `ppt → pptx`). Also `docu convert FILE out.md|.txt|.csv|.json|.pdf`.
- `docu render FILE -o out.pdf|png|svg [--pages 2-3]` lays it out as Office does (metric-compatible
  fonts, Korean included).
