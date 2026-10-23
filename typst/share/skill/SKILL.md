---
name: typst-report
description: Write reports and other documents as PDF with Typst (the `typst` command in this sandbox) - business/analysis reports, summaries of results, tables and charts from data, Korean or English. Use when the user asks for a report, a PDF, a printable document, or a formatted write-up of findings.
---

# Reports with Typst

The sandbox has `typst` 0.15.1 (a WebAssembly port, same output as the official binary), Korean
fonts (NanumMyeongjo, NanumGothic, NanumGothicCoding) and a report template, `@collabo/report`,
that works offline. Full guide: `/usr/share/doc/typst/AGENTS.md` — read it before your first
report. Template reference: `/usr/share/typst/packages/collabo/report/0.1.0/README.md`.

## Steps

1. Put the facts and numbers in data files (`data.json`, `*.csv`) next to the document;
   compute totals and shares in Typst, do not copy numbers by hand.
2. Start from the template: `typst init @collabo/report <dir>` (gives `main.typ`, `data.json`,
   `incidents.csv` to adapt), or write `main.typ` with
   `#import "@collabo/report:0.1.0": *` and `#show: report.with(title: [...], author: "...", summary: [...])`.
   Use `lang: "en"` for English. Helpers: `kpis`, `callout`, `badge`, `data-table`,
   `bar-chart`, `line-chart`, `pie-chart`, `progress`, `format-number`.
3. Compile: `typst compile main.typ` (→ `main.pdf`); add `--diagnostic-format short` for
   one-line errors. Fix every error and read the warnings.
4. Check the pages: `mkdir -p preview && typst compile main.typ 'preview/p{p}.png' --ppi 60`,
   then look at the images (overflow, empty pages, missing glyphs). Check the structure with
   `typst eval --in main.typ 'query(heading).map(h => (h.body.text, h.location().page()))'`.
5. Report the PDF's path to the user, with a short summary of what is in it.

## Typst, not Markdown or LaTeX

- Headings `= H1`, `== H2`; `*bold*`, `_italic_`; lists `-` / `+`; code blocks with backticks.
- In plain text escape `\$` (else math starts), `\*`, `\_`, `\#`, `\@`, `\<`.
- Functions in markup need `#`: `#table(columns: 3, [a], [b], [c])`, `#image("x.png", width: 60%)`.
- Labels and references: `#figure(...) <fig-x>` … `@fig-x`.
- Gone in 0.15: `path` (use `curve`), `pattern` (`tiling`), `locate`/`style` (`context`),
  `json.decode` (`json(bytes(...))`).
- Offline packages (exactly these versions): `@preview/cmarker:0.1.10` (render Markdown:
  `#cmarker.render(read("x.md"))`), `@preview/mitex:0.2.7` (LaTeX math), `@preview/cetz:0.5.2`,
  `@preview/cetz-plot:0.1.4`, `@preview/lilaq:0.6.0` (plots), `@preview/fletcher:0.5.8`
  (diagrams). Other packages download through the host's network policy (`packages.typst.org`).
