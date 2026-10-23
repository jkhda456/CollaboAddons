# @collabo/report 0.1.0

A report template for agents, from collaboCore's typst add-on. It sets up the page (title page,
table of contents, numbered headings, header and footer, Korean fonts) and gives helpers that need
no other package, so everything works offline: callouts, badges, key figures, tables from data
files, bar, line and pie charts.

```sh
typst init @collabo/report myreport     # a project to start from: main.typ, data.json, incidents.csv
cd myreport && typst compile main.typ   # → main.pdf
```

```typst
#import "@collabo/report:0.1.0": *
#show: report.with(
  title: [분기 보고서],
  author: "분석팀",
  summary: [한 문단 요약.],
)

= 개요
...
```

## `report(...)` — the document

`#show: report.with(...)` once, at the top. Every argument is optional.

| Argument | Meaning | Default |
|---|---|---|
| `title`, `subtitle` | content | none |
| `author` | a string or an array of strings (also the PDF's author) | none |
| `organization`, `version`, `status` | content, on the title page | none |
| `date` | `auto` (today), a `datetime`, content, or `none` | `auto` |
| `lang`, `region` | text language: `"ko"` or `"en"` (words, fonts, hyphenation) | `"ko"` |
| `summary` | an executive summary, in a box on the title page | none |
| `title-page` | a page of its own; `false`: a title block on the first page | `true` |
| `contents`, `contents-depth` | a table of contents, and its levels | `true`, `2` |
| `numbering` | headings' numbering, or `none` | `"1.1"` |
| `chapter-break` | each level-1 heading starts a new page | `false` |
| `confidential` | a label such as `"대외비"` in the header and on the title page | none |
| `watermark` | text across every page, e.g. `"DRAFT"` | none |
| `accent` | color of headings, rules, table headers, charts' first series | `rgb("#1f4e79")` |
| `font` | `"serif"` (Nanum Myeongjo / Libertinus body) or `"sans"` (Nanum Gothic) | `"serif"` |
| `keep-words` | break lines only between words, never inside a Korean word (어절) | on for `"ko"` |
| `paper`, `margin`, `size` | page size, margins, body text size | `"a4"`, …, `10.5pt` |

Headings, tables, code and figure captions get the report's style; `@label` references,
footnotes, math, `#bibliography(...)` and everything else in Typst work as usual.

## Helpers

All take content in `[...]` and data as Typst values, so they work with `json(...)`,
`csv(...)`, `yaml(...)` and `toml(...)` straight away.

- **`callout(kind: "note", title: auto)[body]`** — a highlighted box. `kind`: `"note"`,
  `"info"`, `"tip"`, `"success"`, `"warning"`, `"danger"`, `"question"`; the title defaults to
  the kind's word (참고, 주의, …); `title: none` for none.
- **`badge(color: auto)[text]`** — a small colored label in the text: `#badge[완료]`.
- **`kpis(..items, columns: auto)`** — key figures in cards. Each item:
  `(label: "매출", value: "12.4억", delta: "+8.2%", note: "전 분기 대비")`. A delta starting with
  `+`/`▲` is green and with `-`/`−`/`▼` red; `good: false` swaps them (a cost that rises). A
  numeric `value` gets thousands separators. `#kpis(..json("kpis.json"))` for a list in a file.
- **`progress(value, max: 1)`** — an inline bar with its percentage: `#progress(0.72)`.
- **`format-number(value, digits: auto, sep: ",")`** — `1,234,567.89`.
- **`data-table(data, ...)`** — a table from rows: arrays (`csv("x.csv")`, the first row is the
  header) or dictionaries (`json("rows.json")`, keys are the header). Numeric columns (also
  numbers in CSV strings) align right with separators.
  `columns: ("이름", "금액")` picks and orders columns; `header: none` / an array of cells;
  `digits: 2`; `format: (column, value) => ...` returns a cell, or `auto` for the default;
  `widths`, `align`; `caption: [...]` makes it a figure ("표 1") that a `<label>` can follow;
  `total: "합계"` adds a row summing the numeric columns.
- **`bar-chart(data, ...)`** — data: a dictionary `(Q1: 12, Q2: 18)`, pairs
  `(("1월", 12), ("2월", 18))`, or dictionaries with `label` and `value`. Values may be negative.
  Arrays as values give grouped bars: `(Q1: (12, 15), Q2: (18, 16))` with
  `series: ("2025", "2026")` for the legend. `horizontal: true` for long labels. `unit: "%"`,
  `digits`, `values: false` (no numbers on the bars), `min`, `max`, `ticks`, `height`,
  `bar-height`, `color` (one, or an array), `title: [...]`, `caption: [...]`.
- **`line-chart(labels, series, ...)`** — `#line-chart(("1월", "2월"), (매출: (10, 14), 비용: (8, 9)))`;
  `series` may also be a plain array. `values: true` labels each line's last value; `fill: true`
  shades a single series; `zero: false` lets the axis start near the data; `none` values are
  skipped; `markers`, `unit`, `digits`, `min`, `max`, `colors`, `legend`, `title`, `caption`.
- **`pie-chart(data, ...)`** — shares, with a legend of values and percentages. `donut: true`
  (the total in the middle, or `inside: [...]`), `size`, `percent`, `values`, `unit`, `colors`,
  `title`, `caption`.
- **`palette`** — the charts' colors, for your own figures.

A chart with `caption` is a figure: `#bar-chart(..., caption: [월별 매출]) <fig-sales>`, then
`@fig-sales` in the text.

## Fonts

Korean text uses Nanum Myeongjo (body), Nanum Gothic (headings, tables, charts) and Nanum Gothic
Coding (code), which the add-on installs; Latin text in English reports uses Libertinus Serif and
DejaVu Sans Mono, which are built into typst. `typst fonts` lists what there is.

License: MIT-0.
