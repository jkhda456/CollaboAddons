# Typst in this sandbox — a guide for agents

`typst` here is Typst 0.15.1, the typesetting system, built for collaboCore's WebAssembly guest
(the *WASM native port for collaboCore*; its output is the same, byte for byte, as the official
0.15.1 binary's). Use it to turn text and data into PDF reports, and PNG/SVG/HTML.

What the sandbox adds:

- **Korean fonts**: NanumMyeongjo (serif), NanumGothic (sans), NanumGothicCoding (mono) — the
  family names have no spaces. Built in: Libertinus Serif, New Computer Modern (+ Math),
  DejaVu Sans Mono. `typst fonts` lists them.
- **`@collabo/report`**, a report template that needs no download: title page, contents,
  headers, callouts, key-figure cards, tables from JSON/CSV, bar/line/pie charts.
  Its reference: `/usr/share/typst/packages/collabo/report/0.1.0/README.md`.
- **Typst Universe packages that work offline** — in the image, at these versions exactly:
  `@preview/cmarker:0.1.10` (Markdown → Typst: `#cmarker.render(read("notes.md"))`),
  `@preview/mitex:0.2.7` (LaTeX math: ``#mitex(`\frac{a}{b}`)``), `@preview/cetz:0.5.2` (drawing),
  `@preview/cetz-plot:0.1.4` (plots on cetz), `@preview/lilaq:0.6.0` (data plots:
  `#lq.diagram(lq.plot(xs, ys))`), `@preview/fletcher:0.5.8` (diagrams with nodes and arrows).
- **Other Universe packages and versions** are downloaded through the sandbox's host, under its
  network policy: `packages.typst.org` must be allowed. Without network, stick to Typst itself,
  `@collabo/report` and the packages above.

## Writing a report

```sh
typst init @collabo/report report     # report/main.typ + data.json + incidents.csv to adapt
cd report
typst compile main.typ                # → main.pdf; errors and warnings go to stderr
```

1. **Keep numbers in data files** (JSON, CSV, YAML, TOML) and load them: `#let d = json("data.json")`,
   `csv("t.csv")` (rows as arrays, the header first). Compute in Typst rather than copying numbers
   by hand: `d.rows.map(r => r.amount).sum()`.
2. **Compile after each larger edit** and fix every error; read warnings too (an unknown font
   family is a warning, and the text silently uses another font).
   `--diagnostic-format short` gives one line per problem: `file:line:col: error: message`.
3. **Look at the result.** If you can view images, render the pages and check them:
   `typst compile main.typ 'preview/p{p}.png' --ppi 60` (`{p}` is the page number; make the
   folder first; `--pages 2-3` for some only). Look for overflowing tables, empty pages, wrong
   fonts (tofu boxes), charts that are too large.
4. **Check the structure** without looking:
   `typst eval --in main.typ 'query(heading).map(h => (h.level, h.body.text, h.location().page()))'`
   lists the headings with their pages; `query(figure)` the figures; `query(<my-label>)` one.
5. Deliver the PDF (and say where it is). For reproducible files set `SOURCE_DATE_EPOCH` or
   `--creation-timestamp`; `--pdf-standard a-2b` makes PDF/A for archives.

Other outputs: `typst compile main.typ out.svg` (one file per page with `{p}`),
`typst compile --features html --format html main.typ` (experimental HTML).
Values from the command line: `typst compile --input author=홍길동 main.typ` →
`sys.inputs.author` (always strings). `typst watch main.typ` recompiles on every change.

## @collabo/report in brief

```typst
#import "@collabo/report:0.1.0": *
#show: report.with(
  title: [2026년 3분기 운영 보고서],
  subtitle: [지표와 개선 과제],
  author: "운영 분석팀",
  organization: [collaboCore],
  summary: [한 문단의 요약.],      // executive summary on the title page
  // lang: "en", title-page: false, contents: false, confidential: "대외비", watermark: "DRAFT",
  // font: "sans", accent: rgb("#0f766e"), numbering: none, chapter-break: true
)

= 핵심 지표
#kpis(
  (label: "매출", value: "12.4억", delta: "+8.2%", note: "전 분기 대비"),
  (label: "비용", value: "7.1억", delta: "+3.0%", good: false),
)
#callout(kind: "warning")[결제 응답 시간이 늘고 있다.]   // note info tip success warning danger question

#bar-chart((Q1: 12, Q2: 18, Q3: 9), unit: "억", caption: [분기별 매출]) <fig-q>
#bar-chart((Q1: (12, 15), Q2: (18, 16)), series: ("2025", "2026"))        // grouped
#bar-chart(json("regions.json"), horizontal: true, unit: "%")
#line-chart(("7월", "8월", "9월"), (매출: (3.8, 4.1, 4.5), 비용: (2.2, 2.4, 2.5)), values: true)
#pie-chart((완료: 34, 진행: 12, 대기: 6), donut: true)
#data-table(csv("incidents.csv"), caption: [장애 목록], total: "합계") <tbl-inc>

@fig-q 와 @tbl-inc 를 보라. #badge[완료] #progress(0.72) #format-number(1234567.8, digits: 1)
```

## Typst syntax in one page

Markup (the default mode):

| Write | For |
|---|---|
| `= Title`, `== Section` | headings (not `#`: that starts code) |
| `*bold*`, `_italic_`, `` `code` `` | emphasis, inline code |
| `- item`, `+ numbered`, `/ Term: description` | lists |
| a blank line | a new paragraph; `\` at the end of a line breaks it |
| ```` ```python ... ``` ```` | a code block with highlighting |
| `$x^2 + y_1$`, `$ sum_(i=1)^n i $` | inline math; display math has spaces inside the `$` |
| `<intro>` after an element, `@intro` | a label and a reference to it |
| `#footnote[...]`, `#link("https://…")[text]` | footnotes, links |
| `#image("chart.png", width: 80%)` | images (PNG, JPEG, GIF, SVG, WebP, PDF) |
| `// comment`, `/* comment */` | comments |

Code (after `#` in markup, or inside `{...}`): `#let x = 5`, `#if x > 3 [big] else [small]`,
`#for r in rows [#r.name]`, functions `#let f(a, b: 1) = a + b`. Strings are `"..."`, content is
`[...]`; dictionaries `(a: 1, b: 2)`, arrays `(1, 2, 3)` and a one-element array `(1,)`.

Styling: `#set text(size: 11pt)`, `#set par(justify: true)`, `#set page(margin: 2cm)`,
`#show heading: set text(fill: blue)`, `#show "ACME": strong`. A set rule applies to the rest of
its block.

```typst
#figure(
  table(
    columns: (auto, 1fr, auto),
    align: (left, left, right),
    table.header[*항목*][*설명*][*금액*],
    [A], [첫 번째], [1,200],
    [B], [두 번째], [3,400],
  ),
  caption: [비용 내역],
) <tbl-cost>
```

### Mistakes models make in Typst

- **Characters with a meaning in markup** must be escaped in plain text: `\$` (else math
  starts: "Price $5" is an *unclosed delimiter* error), `\*`, `\_` (snake_case words!), `\#`,
  `\@` (e-mail addresses, or use `#link("mailto:…")`), `\<`, `\[`. Inside `` `raw` `` nothing
  needs escaping.
- **Not Markdown, not LaTeX**: no `**bold**`, no `# Heading`, no `\section{}`, no `\textbf{}`,
  no Markdown tables or `[text](url)` links. Math: `frac(a, b)` or `a/b`, `sqrt(x)`,
  `vec(1, 2)`, Greek as `alpha`; no backslashes.
- **Calling a function in markup needs `#`**: `#table(...)`, `#image(...)`; inside a function's
  arguments it doesn't: `#figure(image("a.png"), caption: [...])`.
- **Removed or renamed in Typst 0.12–0.15**: `path` → `curve`; `pattern` → `tiling`;
  `locate(loc => ...)` and `style(...)` → `context ...`; `json.decode(s)` (and `csv.decode`,
  `yaml.decode`, …) → `json(bytes(s))`; file paths take forward slashes only.
- **Files are read relative to the file that names them**, and only inside the project root
  (the folder of the main file unless `--root DIR`): `../data.csv` above the root is an error.
- **Lengths**: `1em`, `10pt`, `2cm`, `50%`, `1fr` (a share of the leftover space).
- Text in Korean: `#set text(lang: "ko")` (`@collabo/report` does it): figure and table
  captions become 그림/표, the outline's title 차 례 (the template's is 목차). Typst breaks
  Korean lines between any two syllables; `@collabo/report` keeps words whole (`keep-words`).
  Without the template: `#show regex("\S*\p{Hangul}\S*"): it => box(it)`.

## When something goes wrong

- `unknown font family: nanum gothic` → the name is `NanumGothic`; `typst fonts` lists names.
- `file not found (searched at …)` → a path outside the root, or a typo; paths are relative.
- `failed to download package` / *network policy does not allow* → the sandbox's network policy
  refuses `packages.typst.org`; ask for it to be allowed, or do without the package.
- A slow document: big images (scale them down first), thousands of table rows (summarize),
  `--jobs N` uses more CPUs for export.

Typst's full documentation: <https://typst.app/docs/> (reference, tutorial, guides).
