// @collabo/report: a report template for agents, from collaboCore's typst add-on.
//
//   #import "@collabo/report:0.1.0": *
//   #show: report.with(title: [분기 보고서], author: "분석팀", lang: "ko")
//
// Besides the page set-up (title page, contents, headers, Korean fonts) it has helpers that
// need no other package, so they work offline: callout, badge, kpis, data-table, bar-chart,
// line-chart, progress, format-number. See README.md beside this file.

// ── fonts, words, colors ─────────────────────────────────────────────────────────────────────

/// The font families by role: Korean first for Korean text, Latin first otherwise (the other
/// is the fallback for what the first lacks).
#let _fonts(lang) = if lang == "ko" {
  (
    serif: ("NanumMyeongjo", "Libertinus Serif"),
    sans: ("NanumGothic",),
    mono: ("NanumGothicCoding", "DejaVu Sans Mono"),
  )
} else {
  (
    serif: ("Libertinus Serif", "NanumMyeongjo"),
    sans: ("NanumGothic",),
    mono: ("DejaVu Sans Mono", "NanumGothicCoding"),
  )
}

#let _words = (
  ko: (
    contents: "목차",
    summary: "요약",
    author: "작성",
    organization: "기관",
    date: "작성일",
    version: "버전",
    status: "상태",
    note: "참고",
    info: "정보",
    tip: "팁",
    success: "완료",
    warning: "주의",
    danger: "위험",
    question: "질문",
    source: "출처",
  ),
  en: (
    contents: "Contents",
    summary: "Summary",
    author: "Author",
    organization: "Organization",
    date: "Date",
    version: "Version",
    status: "Status",
    note: "Note",
    info: "Info",
    tip: "Tip",
    success: "Done",
    warning: "Warning",
    danger: "Danger",
    question: "Question",
    source: "Source",
  ),
)

/// The words for the text language in effect (call in a context).
#let _word(key) = _words.at(text.lang, default: _words.en).at(key)

/// Colors for series and kinds, distinguishable in print and for most color vision.
#let palette = (
  rgb("#2563eb"),
  rgb("#f97316"),
  rgb("#16a34a"),
  rgb("#dc2626"),
  rgb("#9333ea"),
  rgb("#0891b2"),
  rgb("#ca8a04"),
  rgb("#db2777"),
)

#let _kinds = (
  note: rgb("#2563eb"),
  info: rgb("#0891b2"),
  tip: rgb("#16a34a"),
  success: rgb("#16a34a"),
  warning: rgb("#d97706"),
  danger: rgb("#dc2626"),
  question: rgb("#9333ea"),
)

/// The report's accent color: a state, so the helpers follow `report(accent: …)`.
#let _accent = state("collabo-report-accent", rgb("#1f4e79"))

// ── numbers ──────────────────────────────────────────────────────────────────────────────────

/// A number with thousands separators and a fixed number of decimals:
/// `format-number(1234567.891, digits: 2)` is "1,234,567.89".
#let format-number(value, digits: auto, sep: ",", point: ".") = {
  if type(value) not in (int, float, decimal) { return value }
  let digits = if digits == auto { if type(value) == int { 0 } else { 2 } } else { digits }
  let negative = value < 0
  let value = calc.abs(value)
  let rounded = calc.round(float(value), digits: digits)
  let whole = int(calc.floor(rounded))
  let fraction = rounded - whole
  let s = str(whole)
  let grouped = ""
  let n = s.len()
  for (i, c) in s.clusters().enumerate() {
    if i > 0 and calc.rem(n - i, 3) == 0 { grouped += sep }
    grouped += c
  }
  if digits > 0 {
    let f = str(calc.round(fraction * calc.pow(10, digits)))
    // 0.999 rounds up into the whole part above; guard the digits' count.
    if f.len() > digits { f = "0" * digits }
    grouped += point + "0" * (digits - f.len()) + f
  }
  if negative and rounded != 0 { "−" + grouped } else { grouped }
}

/// A round step at or above `x`, for chart axes: 1, 2, 2.5 or 5 times a power of ten.
#let _nice-step(x) = {
  if x <= 0 { return 1 }
  let magnitude = calc.pow(10.0, calc.floor(calc.log(x, base: 10)))
  for step in (1, 2, 2.5, 5, 10) {
    if step * magnitude >= x * 0.999999 { return step * magnitude }
  }
  10 * magnitude
}

/// An axis from `low` to `high` in about `ticks` round steps: (start, end, tick values).
#let _axis(low, high, ticks) = {
  let step = _nice-step((high - low) / calc.max(ticks, 1))
  let start = calc.floor(low / step + 1e-9) * step
  let end = calc.ceil(high / step - 1e-9) * step
  if end <= start { end = start + step }
  let n = int(calc.round((end - start) / step))
  (start, end, range(n + 1).map(k => start + k * step), step)
}

/// A tick's label, with as many decimals as the step (m × 10^e, m in 1, 2, 2.5, 5) needs.
#let _tick-label(v, step) = {
  let e = int(calc.floor(calc.log(step, base: 10) + 1e-9))
  let m = step / calc.pow(10.0, e)
  let digits = calc.max(0, -e + if calc.abs(m - 2.5) < 1e-6 { 1 } else { 0 })
  format-number(if calc.abs(v) < step / 1e6 { 0 } else { v }, digits: digits)
}

// ── the document ─────────────────────────────────────────────────────────────────────────────

/// The report: `#show: report.with(…)` at the top of the document.
///
/// - title, subtitle: content
/// - author: a string or an array of strings (the PDF's author too)
/// - organization, version, status: content shown on the title page
/// - date: `auto` (today), a datetime, content, or none
/// - lang: "ko" (default) or "en", …; region: e.g. "KR"
/// - summary: content in a box on the title page (an executive summary)
/// - title-page: a page of its own (true) or a heading block on the first page (false)
/// - contents: a table of contents after the title (true/false); contents-depth: its levels
/// - numbering: the headings' numbering ("1.1", or none)
/// - chapter-break: each level-1 heading starts a page
/// - confidential: a label such as "대외비" in the header and on the title page
/// - watermark: text across every page, e.g. "DRAFT"
/// - accent: the color of headings, rules and charts' first series
/// - font: "serif" (body in Myeongjo/Libertinus) or "sans" (Nanum Gothic throughout)
/// - keep-words: break lines between words only, never inside a Korean word (어절), as Korean
///   documents do; default: on for Korean (Typst alone would break between any two syllables)
/// - paper, margin, size (the body's text size)
#let report(
  title: none,
  subtitle: none,
  author: none,
  organization: none,
  date: auto,
  version: none,
  status: none,
  lang: "ko",
  region: none,
  summary: none,
  title-page: true,
  contents: true,
  contents-depth: 2,
  numbering: "1.1",
  chapter-break: false,
  confidential: none,
  watermark: none,
  accent: rgb("#1f4e79"),
  font: "serif",
  keep-words: auto,
  paper: "a4",
  margin: (x: 2.2cm, top: 2.5cm, bottom: 2.4cm),
  size: 10.5pt,
  body,
) = {
  let fonts = _fonts(lang)
  let body-font = if font == "sans" { fonts.sans } else { fonts.serif }
  let words = _words.at(lang, default: _words.en)
  let authors = if author == none { () } else if type(author) == array { author } else { (author,) }
  let date = if date == auto { datetime.today() } else { date }
  let date-text = if type(date) == datetime {
    if lang == "ko" {
      date.display("[year]년 [month padding:none]월 [day padding:none]일")
    } else { date.display("[month repr:long] [day padding:none], [year]") }
  } else { date }
  let plain(it) = if type(it) == str { it } else if it == none { none } else { [#it].text }

  set document(
    title: if title != none { plain(title) },
    author: authors.filter(a => type(a) == str),
    ..if type(date) == datetime { (date: date) },
  )
  set text(font: body-font, size: size, lang: lang, region: region)
  set par(justify: true, leading: if lang == "ko" { 0.85em } else { 0.7em }, spacing: 1.3em)
  // A word with Hangul in it, up to 24 characters, is one unbreakable box.
  show regex("\S*\p{Hangul}\S*"): it => if (
    (keep-words == true or (keep-words == auto and lang == "ko")) and it.text.clusters().len() <= 24
  ) { box(it) } else { it }
  set page(
    paper: paper,
    margin: margin,
    header: context {
      if here().page() > 1 {
        set text(font: fonts.sans, size: 0.8em, fill: luma(110))
        grid(
          columns: (1fr, auto),
          align: (left, right),
          if title != none { title },
          if confidential != none { text(fill: rgb("#dc2626"), weight: "bold", confidential) },
        )
        v(-0.55em)
        line(length: 100%, stroke: 0.4pt + luma(200))
      }
    },
    footer: context {
      set text(font: fonts.sans, size: 0.8em, fill: luma(110))
      align(center)[#counter(page).display() / #counter(page).final().first()]
    },
    background: if watermark != none {
      rotate(-40deg, text(font: fonts.sans, size: 72pt, weight: "bold", fill: luma(235), watermark))
    },
  )
  _accent.update(accent)

  // Headings
  set heading(numbering: numbering)
  show heading: set text(font: fonts.sans, weight: "bold")
  show heading.where(level: 1): it => {
    // A new page for every chapter but the first (which follows the title or the contents).
    if chapter-break {
      context if query(heading.where(level: 1, outlined: true).before(here())).len() > 1 {
        pagebreak(weak: true)
      }
    }
    set text(size: 1.45em, fill: accent)
    v(0.6em)
    it
    v(-0.35em)
    line(length: 100%, stroke: 0.8pt + accent)
    v(0.3em)
  }
  show heading.where(level: 2): set text(size: 1.18em)
  show heading.where(level: 3): set text(size: 1.04em)
  show heading: set block(above: 1.5em, below: 0.9em)

  // Tables: a tinted header row, light rules, striped rows.
  set table(
    inset: (x: 0.6em, y: 0.5em),
    stroke: (x, y) => (
      top: if y == 0 { 1pt + accent } else { 0.4pt + luma(210) },
      bottom: 1pt + accent.lighten(40%),
    ),
    fill: (x, y) => if y == 0 { accent.lighten(88%) } else if calc.even(y) { luma(248) },
  )
  show table.cell.where(y: 0): set text(font: fonts.sans, weight: "bold", size: 0.95em)
  show table: set text(size: 0.95em)
  show table: set par(justify: false)
  show figure.where(kind: table): set figure.caption(position: top)
  show figure.caption: set text(size: 0.9em)
  show figure.caption: it => [#text(font: fonts.sans, weight: "bold", it.supplement + [ ] + context it.counter.display(it.numbering) + it.separator)#it.body]

  // Code, links, lists, quotes
  show raw: set text(font: fonts.mono, size: 0.92em)
  show raw.where(block: true): block.with(fill: luma(246), inset: 0.8em, radius: 3pt, width: 100%)
  show link: set text(fill: accent)
  set list(indent: 0.4em, marker: ([•], [‣], [–]))
  set enum(indent: 0.4em)
  show quote.where(block: true): block.with(
    stroke: (left: 2pt + accent.lighten(50%)), inset: (left: 1em, y: 0.4em),
  )

  // The title: a page of its own, or a block on the first page.
  let meta = (
    (words.author, if authors.len() > 0 { authors.join(", ") }),
    (words.organization, organization),
    (words.date, date-text),
    (words.version, version),
    (words.status, status),
  ).filter(((k, v)) => v != none)

  if title-page {
    page(header: none, footer: none, {
      set par(justify: false)
      v(12%)
      if organization != none {
        text(font: fonts.sans, size: 1.05em, fill: luma(90), organization)
        v(0.4em)
      }
      line(length: 30%, stroke: 3pt + accent)
      v(0.8em)
      if title != none { text(font: fonts.sans, size: 2.3em, weight: "bold", fill: accent, title) }
      if subtitle != none {
        v(0.3em)
        text(font: fonts.sans, size: 1.3em, fill: luma(70), subtitle)
      }
      if confidential != none {
        v(0.8em)
        box(stroke: 1pt + rgb("#dc2626"), inset: (x: 0.6em, y: 0.35em), radius: 2pt,
          text(font: fonts.sans, weight: "bold", fill: rgb("#dc2626"), confidential))
      }
      v(1.6em)
      if meta.len() > 0 {
        grid(
          columns: (auto, 1fr),
          column-gutter: 1.2em,
          row-gutter: 0.75em,
          ..meta.map(((k, v)) => (text(font: fonts.sans, fill: luma(110), k), v)).flatten(),
        )
      }
      if summary != none {
        v(1fr)
        block(width: 100%, fill: accent.lighten(92%), stroke: (left: 3pt + accent), inset: 1.1em, {
          text(font: fonts.sans, weight: "bold", fill: accent, words.summary)
          v(0.3em)
          set par(justify: true)
          summary
        })
      }
      v(6%)
    })
  } else {
    block(width: 100%, below: 1.5em, {
      set par(justify: false)
      if title != none { text(font: fonts.sans, size: 1.9em, weight: "bold", fill: accent, title) }
      if subtitle != none {
        linebreak()
        text(font: fonts.sans, size: 1.15em, fill: luma(70), subtitle)
      }
      if meta.len() > 0 or confidential != none {
        v(0.2em)
        set text(size: 0.9em, fill: luma(90))
        (
          meta.map(((k, v)) => [#k: #v])
            + if confidential != none { (text(fill: rgb("#dc2626"), weight: "bold", confidential),) } else { () }
        ).join(h(1.2em))
      }
      v(-0.3em)
      line(length: 100%, stroke: 1pt + accent)
      if summary != none {
        block(width: 100%, fill: accent.lighten(92%), inset: 0.9em, radius: 2pt, {
          text(font: fonts.sans, weight: "bold", fill: accent, words.summary)
          h(0.6em)
          summary
        })
      }
    })
  }

  if contents {
    show outline.entry.where(level: 1): set text(font: fonts.sans, weight: "bold")
    show outline.entry.where(level: 1): set block(above: 1em)
    outline(title: words.contents, depth: contents-depth, indent: auto)
    if title-page { pagebreak(weak: true) } else { v(1em) }
  }

  body
}

// ── blocks ───────────────────────────────────────────────────────────────────────────────────

/// A highlighted box: kind is "note", "info", "tip", "success", "warning", "danger" or
/// "question"; the title defaults to the kind's word ("참고", "주의", …); `title: none` for none.
#let callout(kind: "note", title: auto, body) = context {
  let color = _kinds.at(kind, default: _accent.get())
  let title = if title == auto { _word(kind) } else { title }
  block(
    width: 100%,
    fill: color.lighten(92%),
    stroke: (left: 3pt + color),
    inset: (x: 1em, y: 0.8em),
    radius: (right: 3pt),
    breakable: true,
    {
      if title != none {
        text(weight: "bold", fill: color.darken(15%), font: _fonts(text.lang).sans, title)
        parbreak()
      }
      body
    },
  )
}

/// A small colored label in the text: `#badge[완료]`, `#badge(color: red)[지연]`.
#let badge(color: auto, body) = context {
  let color = if color == auto { _accent.get() } else { color }
  box(
    fill: color.lighten(85%),
    stroke: 0.5pt + color.lighten(30%),
    inset: (x: 0.4em, y: 0.15em),
    outset: (y: 0.1em),
    radius: 2pt,
    text(size: 0.85em, weight: "bold", fill: color.darken(20%), font: _fonts(text.lang).sans, body),
  )
}

/// A row of key figures, each a dictionary: (label: "매출", value: "12.4억", delta: "+8%",
/// note: "전년 대비"). A delta starting with "+" is green and with "-"/"−" red; `good: false`
/// in an item swaps them (a rising cost). columns: how many per row (default: all, up to 4).
#let kpis(columns: auto, ..items) = context {
  let items = items.pos()
  if items.len() == 0 { return [] }
  let n = if columns == auto { calc.min(items.len(), 4) } else { columns }
  let sans = _fonts(text.lang).sans
  let accent = _accent.get()
  grid(
    columns: (1fr,) * n,
    gutter: 0.8em,
    ..items.map(item => {
      let delta = item.at("delta", default: none)
      let delta-color = if delta == none { luma(100) } else {
        let s = if type(delta) == str { delta } else { str(delta) }
        let up = s.starts-with("+") or s.starts-with("▲")
        let down = s.starts-with("-") or s.starts-with("−") or s.starts-with("▼")
        let good = item.at("good", default: true)
        if (up and good) or (down and not good) { rgb("#16a34a") } else if up or down {
          rgb("#dc2626")
        } else { luma(100) }
      }
      let value = if type(item.value) in (int, float, decimal) { format-number(item.value) } else { item.value }
      block(width: 100%, inset: 0.9em, radius: 3pt, fill: luma(248), stroke: (top: 2pt + accent), {
        set par(justify: false)
        set text(font: sans)
        stack(
          spacing: 0.55em,
          text(size: 0.85em, fill: luma(90), [#item.label]),
          {
            text(size: 1.6em, weight: "bold", [#value])
            if delta != none {
              h(0.4em)
              text(size: 0.9em, weight: "bold", fill: delta-color, [#delta])
            }
          },
          ..if item.at("note", default: none) != none {
            (text(size: 0.8em, fill: luma(110), [#item.note]),)
          },
        )
      })
    }),
  )
}

/// A horizontal bar for a share or a progress: `#progress(0.72)`, `#progress(36, max: 50)`.
#let progress(value, max: 1, width: 5cm, height: 0.7em, color: auto, label: true) = context {
  let color = if color == auto { _accent.get() } else { color }
  let ratio = calc.clamp(value / max, 0, 1)
  box(baseline: 0.1em, {
    box(width: width, height: height, fill: luma(232), radius: height / 2, clip: true,
      align(left, box(width: ratio * width, height: height, fill: color, radius: height / 2)))
    if label {
      h(0.4em)
      text(size: 0.85em, format-number(ratio * 100, digits: 0) + "%")
    }
  })
}

// ── tables from data ─────────────────────────────────────────────────────────────────────────

/// A table from data: rows as arrays (`csv("data.csv")`: the first row is the header) or
/// dictionaries (`json("rows.json")`: the keys are the header, in the first row's order).
///
/// - columns: which columns, in order (names for dictionaries, indices or names for arrays)
/// - header: the header cells (default: from the data), or none
/// - digits: decimals for floats; numbers get thousands separators and align right
/// - format: a function (column, value) => content for cells of your own
/// - widths: the columns' widths (default: auto)
/// - caption: makes it a figure ("표 1: …") that can carry a <label>
/// - total: a last row adding up the numeric columns (its first cell is this text)
#let data-table(
  data,
  columns: auto,
  header: auto,
  digits: 2,
  format: none,
  widths: auto,
  align: auto,
  caption: none,
  total: none,
) = {
  if data.len() == 0 { return [] }
  let dicts = type(data.first()) == dictionary
  let (names, rows) = if dicts {
    (data.first().keys(), data)
  } else if header == none {
    (range(data.first().len()).map(str), data)
  } else {
    (data.first().map(c => if type(c) == str { c } else { [#c] }), data.slice(1))
  }
  let picks = if columns == auto { range(names.len()) } else {
    columns.map(c => {
      let i = if type(c) == int { c } else { names.position(n => n == c) }
      assert(i != none and i < names.len(), message: "data-table: no column " + repr(c) + "; there are " + repr(names))
      i
    })
  }
  let get(row, i) = if dicts { row.at(names.at(i), default: none) } else { row.at(i, default: none) }
  // CSV gives strings: a column whose cells all read as numbers is numeric.
  let as-number(v) = {
    if type(v) in (int, float, decimal) { return v }
    if type(v) != str { return none }
    let s = v.trim().replace(",", "")
    if s.match(regex("^[-+]?\d+$")) != none { int(s) } else if (
      s.match(regex("^[-+]?(\d+\.\d*|\.\d+|\d+)([eE][-+]?\d+)?$")) != none
    ) { float(s) } else { none }
  }
  let numeric = picks.map(i => {
    let values = rows.map(r => get(r, i)).filter(v => v != none and v != "")
    values.len() > 0 and values.all(v => as-number(v) != none)
  })
  let cell(k, v) = {
    let i = picks.at(k)
    if format != none {
      let out = format(names.at(i), v)
      if out != auto { return out }
    }
    if v == none or v == "" { return text(fill: luma(150))[–] }
    if numeric.at(k) {
      let n = as-number(v)
      format-number(n, digits: if type(n) == int { 0 } else { digits })
    } else if type(v) in (str, content) { v } else { str(v) }
  }
  let header-cells = if header == none { () } else if header == auto {
    picks.map(i => names.at(i))
  } else { header }
  let total-cells = if total == none { () } else {
    picks.enumerate().map(((k, i)) => {
      if k == 0 { strong(total) } else if numeric.at(k) {
        let sum = rows.map(r => as-number(get(r, i))).filter(v => v != none).sum(default: 0)
        strong(format-number(sum, digits: if type(sum) == int { 0 } else { digits }))
      } else { [] }
    })
  }
  let aligns = if align == auto {
    numeric.map(n => if n { right } else { left })
  } else { align }
  let t = table(
    columns: if widths == auto { picks.len() } else { widths },
    align: (x, y) => aligns.at(x) + horizon,
    ..if header-cells.len() > 0 { (table.header(..header-cells),) },
    ..rows.map(r => picks.enumerate().map(((k, i)) => cell(k, get(r, i)))).flatten(),
    ..if total-cells.len() > 0 { (table.footer(repeat: false, ..total-cells),) },
  )
  if caption != none { figure(t, caption: caption, kind: table) } else { t }
}

// ── charts ───────────────────────────────────────────────────────────────────────────────────

/// (labels, values) from what a chart takes: a dictionary (`(Q1: 12, Q2: 18)`), an array of
/// pairs (`(("1월", 12), ("2월", 18))`) or of dictionaries with `label` and `value`. A value may
/// be an array (one number per series: grouped bars); numbers in strings ("1,234") are read.
#let _pairs(data) = {
  let number(v) = if type(v) == str { float(v.replace(",", "").trim()) } else { v }
  let value(v) = if type(v) == array { v.map(number) } else { number(v) }
  if type(data) == dictionary { return (data.keys(), data.values().map(value)) }
  let labels = data.map(d => if type(d) == dictionary { d.label } else { d.at(0) })
  let values = data.map(d => if type(d) == dictionary { d.value } else { d.at(1) })
  (labels, values.map(value))
}

/// A number as charts show it: `digits` decimals (auto: none for integers, 1 otherwise), unit.
#let _chart-number(v, digits, unit) = {
  let d = if digits == auto { if type(v) == int { 0 } else { 1 } } else { digits }
  format-number(v, digits: d) + if unit != none { unit }
}

/// The colors of `n` series: the accent first, then the palette.
#let _series-colors(colors, n) = {
  let all = if colors == auto { (_accent.get(),) + palette.slice(1) } else if type(colors) == array {
    colors
  } else { (colors,) }
  range(n).map(k => all.at(calc.rem(k, all.len())))
}

#let _legend(names, colors) = align(center, names.zip(colors).map(((name, color)) => box({
  box(width: 0.8em, height: 0.8em, baseline: 0.1em, fill: color, radius: 1pt)
  h(0.3em)
  text(size: 0.8em, [#name])
})).join(h(1.2em)))

/// A figure when there is a caption, so that a <label> after the chart refers to it: made
/// outside the chart's context.
#let _figure(caption, body) = if caption != none { figure(body, caption: caption, kind: image) } else { body }

#let _chart-frame(title, body) = {
  block(width: 100%, breakable: false, {
    set text(font: _fonts(text.lang).sans)
    set par(justify: false)
    if title != none {
      align(center, text(weight: "bold", size: 0.95em, title))
      v(0.3em)
    }
    body
  })
}

/// The axis for `values` with `min`/`max` given or not (`auto`: round numbers around the data,
/// zero included).
#let _value-axis(values, min, max, ticks) = {
  let low = if min == auto { calc.min(0, ..values) } else { min }
  let high = if max == auto { calc.max(0, ..values) } else { max }
  let (start, end, marks, step) = _axis(low, high, ticks)
  if min != auto { start = min }
  if max != auto { end = max }
  (start, end, marks.filter(v => v >= start - step / 1e6 and v <= end + step / 1e6), step)
}

/// A bar chart: `#bar-chart((Q1: 12, Q2: 18, Q3: 9))`.
///
/// - data: see above; with arrays as values (`(Q1: (12, 15), Q2: (18, 16))`) the bars are grouped,
///   one per series, and `series` names them in a legend (`series: ("2025", "2026")`)
/// - horizontal: bars from the left (long labels) instead of from the bottom
/// - height: the plot's height (vertical); bar-height: each bar's thickness (horizontal)
/// - color: one color, an array (one per bar, or one per series when grouped); default the accent
/// - min, max: the value axis' ends (default: round numbers around the data and zero)
/// - values: each bar's value at its end; digits: its decimals; unit: after it ("%", "억")
/// - title: above the chart; caption: makes it a figure ("그림 1: …") that can carry a <label>
#let bar-chart(
  data,
  series: none,
  horizontal: false,
  height: 5.5cm,
  bar-height: 1.1em,
  color: auto,
  min: auto,
  max: auto,
  values: true,
  digits: auto,
  unit: none,
  ticks: 5,
  title: none,
  caption: none,
) = _figure(caption, context {
  let (labels, raw) = _pairs(data)
  let n = raw.len()
  if n == 0 { return [] }
  let grouped = type(raw.first()) == array
  let rows = if grouped { raw } else { raw.map(v => (v,)) }
  let k = calc.max(..rows.map(r => r.len()))
  let colors = if grouped { _series-colors(color, k) } else if type(color) == array { color } else {
    (if color == auto { _accent.get() } else { color },)
  }
  let color-of(i, j) = if grouped { colors.at(j) } else { colors.at(calc.rem(i, colors.len())) }
  let all = rows.flatten().filter(v => v != none)
  let (start, end, marks, step) = _value-axis(all, min, max, ticks)
  let span = end - start
  let small = 0.8em
  let fmt(v) = _chart-number(v, digits, unit)
  let label-text(l) = text(size: small, [#l])
  // Positions are compared and subtracted: all in absolute lengths.
  let em = 1em.to-absolute()
  let height = height.to-absolute()
  let bar-height = bar-height.to-absolute()

  _chart-frame(title, {
    layout(size => {
      if horizontal {
        let label-w = calc.max(..labels.map(l => measure(label-text(l)).width)) + 0.6 * em
        let value-w = if values { calc.max(..all.map(v => measure(text(size: small, fmt(v))).width)) + 0.5 * em } else { 0.4 * em }
        let neg-w = if values and calc.min(..all) < 0 { value-w } else { 0pt }
        let plot-x = label-w + neg-w
        let plot-w = size.width - plot-x - value-w
        let x-of(v) = plot-x + plot-w * ((calc.clamp(v, start, end) - start) / span)
        let gap = bar-height * 0.5
        let band = k * bar-height + gap
        let plot-h = n * band + gap
        box(width: size.width, height: plot-h + 1.5em, {
          for v in marks {
            place(dx: x-of(v), line(angle: 90deg, length: plot-h, stroke: 0.4pt + luma(222)))
            place(dx: x-of(v) - 2.5em, dy: plot-h + 0.35em, box(width: 5em, align(center, text(size: small, fill: luma(100), _tick-label(v, step)))))
          }
          for (i, (l, row)) in labels.zip(rows).enumerate() {
            let y = gap + i * band
            place(dy: y, box(width: label-w - 0.4em, height: k * bar-height, align(right + horizon, label-text(l))))
            for (j, v) in row.enumerate() {
              if v == none { continue }
              let (x0, x1) = (x-of(0), x-of(v))
              let yy = y + j * bar-height
              place(dx: calc.min(x0, x1), dy: yy, rect(width: calc.abs(x1 - x0), height: bar-height, fill: color-of(i, j)))
              if values {
                let t = text(size: small, fmt(v))
                let dx = if v >= 0 { x1 + 0.3em } else { x1 - measure(t).width - 0.3em }
                place(dx: dx, dy: yy, box(height: bar-height, align(horizon, t)))
              }
            }
          }
          place(dx: x-of(calc.clamp(0, start, end)), line(angle: 90deg, length: plot-h, stroke: 0.8pt + luma(120)))
        })
      } else {
        let axis-w = calc.max(..marks.map(v => measure(text(size: small, _tick-label(v, step))).width)) + 0.5 * em
        let plot-w = size.width - axis-w
        let slot = plot-w / n
        let group-w = slot * if grouped { 0.8 } else { 0.62 }
        let bar-w = group-w / k
        let label-h = calc.max(..labels.map(l => measure(box(width: slot, align(center, label-text(l)))).height)) + 0.5 * em
        let pad = if values { 1.3 * em } else { 0.5 * em }
        let y-of(v) = pad + height * ((end - calc.clamp(v, start, end)) / span)
        box(width: size.width, height: pad + height + label-h + if values and start < 0 { 1.2em } else { 0pt }, {
          for v in marks {
            place(dx: axis-w, dy: y-of(v), line(length: plot-w, stroke: 0.4pt + luma(222)))
            place(dy: y-of(v) - 0.5em, box(width: axis-w - 0.4em, height: 1em, align(right + horizon, text(size: small, fill: luma(100), _tick-label(v, step)))))
          }
          for (i, (l, row)) in labels.zip(rows).enumerate() {
            for (j, v) in row.enumerate() {
              if v == none { continue }
              let x = axis-w + i * slot + (slot - group-w) / 2 + j * bar-w
              let (y0, y1) = (y-of(0), y-of(v))
              place(dx: x, dy: calc.min(y0, y1), rect(width: bar-w, height: calc.abs(y1 - y0), fill: color-of(i, j)))
              if values {
                let dy = if v >= 0 { y1 - 1.15em } else { y1 + 0.2em }
                place(dx: x - 1em, dy: dy, box(width: bar-w + 2em, align(center, text(size: small, fmt(v)))))
              }
            }
            place(dx: axis-w + i * slot, dy: pad + height + 0.4em, box(width: slot, align(center, label-text(l))))
          }
          place(dx: axis-w, dy: y-of(calc.clamp(0, start, end)), line(length: plot-w, stroke: 0.8pt + luma(120)))
        })
      }
    })
    if grouped and series != none {
      v(0.3em)
      _legend(series, colors)
    }
  })
})

/// A line chart of one or more series over the same labels:
/// `#line-chart(("1월", "2월", "3월"), (매출: (10, 14, 13), 비용: (8, 9, 11)))`.
///
/// - labels: the x axis' labels; series: a dictionary name: values (or an array of values);
///   a `none` value leaves a gap's point out
/// - min, max: the y axis' ends (default: round numbers around the data and zero; `min: auto`
///   with `zero: false` starts near the smallest value instead)
/// - colors: per series (default: the accent, then the palette); markers: dots at the values
/// - values: the last value of each series at its end; unit, digits: as bar-chart
/// - fill: shade the area under a single series
/// - title, caption: as bar-chart
#let line-chart(
  labels,
  series,
  height: 5.5cm,
  min: auto,
  max: auto,
  zero: true,
  colors: auto,
  markers: true,
  values: false,
  digits: auto,
  unit: none,
  ticks: 5,
  fill: false,
  legend: true,
  title: none,
  caption: none,
) = _figure(caption, context {
  let series = if type(series) == dictionary { series } else { ("": series) }
  let names = series.keys()
  let all = series.values().flatten().filter(v => v != none)
  if all.len() == 0 { return [] }
  let colors = _series-colors(colors, names.len())
  let (start, end, marks, step) = if zero {
    _value-axis(all, min, max, ticks)
  } else {
    let (s, e, m, st) = _axis(if min == auto { calc.min(..all) } else { min }, if max == auto { calc.max(..all) } else { max }, ticks)
    (s, e, m, st)
  }
  let span = end - start
  let small = 0.8em
  let fmt(v) = _chart-number(v, digits, unit)
  let n = labels.len()
  let em = 1em.to-absolute()
  let height = height.to-absolute()

  _chart-frame(title, {
    layout(size => {
      let axis-w = calc.max(..marks.map(v => measure(text(size: small, _tick-label(v, step))).width)) + 0.5 * em
      let end-w = if values { calc.max(..all.map(v => measure(text(size: small, fmt(v))).width)) + 0.6 * em } else { 0.8 * em }
      let plot-w = size.width - axis-w - end-w
      let step-x = if n > 1 { plot-w / (n - 1) } else { 0pt }
      let x-of(i) = axis-w + if n > 1 { i * step-x } else { plot-w / 2 }
      let pad = 0.6 * em
      let y-of(v) = pad + height * ((end - calc.clamp(v, start, end)) / span)
      box(width: size.width, height: pad + height + 1.7em, {
        for v in marks {
          place(dx: axis-w, dy: y-of(v), line(length: plot-w, stroke: 0.4pt + luma(222)))
          place(dy: y-of(v) - 0.5em, box(width: axis-w - 0.4em, height: 1em, align(right + horizon, text(size: small, fill: luma(100), _tick-label(v, step)))))
        }
        // At most about 12 labels on the x axis: every k-th one when there are more.
        let every = calc.max(1, calc.ceil(n / 12))
        for (i, l) in labels.enumerate() {
          if calc.rem(i, every) == 0 or i == n - 1 {
            place(dx: x-of(i) - 3em, dy: pad + height + 0.6em, box(width: 6em, align(center, text(size: small, [#l]))))
          }
        }
        place(dx: axis-w, dy: y-of(calc.clamp(0, start, end)), line(length: plot-w, stroke: 0.8pt + luma(120)))
        for (s, name) in names.enumerate() {
          let vals = series.at(name)
          let color = colors.at(s)
          let points = vals.enumerate().filter(((i, v)) => v != none).map(((i, v)) => (x-of(i), y-of(v)))
          if fill and names.len() == 1 and points.len() > 1 {
            let base = y-of(calc.clamp(0, start, end))
            place(curve(
              fill: color.transparentize(82%),
              stroke: none,
              curve.move((points.first().at(0), base)),
              ..points.map(p => curve.line(p)),
              curve.line((points.last().at(0), base)),
              curve.close(),
            ))
          }
          if points.len() > 1 {
            place(curve(stroke: 1.6pt + color, curve.move(points.first()), ..points.slice(1).map(p => curve.line(p))))
          }
          if markers {
            for p in points {
              place(dx: p.at(0) - 2.2pt, dy: p.at(1) - 2.2pt, circle(radius: 2.2pt, fill: white, stroke: 1.2pt + color))
            }
          }
          if values and points.len() > 0 {
            let last = vals.filter(v => v != none).last()
            place(dx: points.last().at(0) + 0.4em, dy: points.last().at(1) - 0.5em, box(height: 1em, align(horizon, text(size: small, fill: color, weight: "bold", fmt(last)))))
          }
        }
      })
    })
    if legend and (names.len() > 1 or names.first() != "") {
      v(0.3em)
      _legend(names, colors)
    }
  })
})

/// A pie (or donut) chart of shares: `#pie-chart((서울: 52, 부산: 18, 기타: 30))`.
///
/// - data: as bar-chart (one number per slice; negative and zero slices are left out)
/// - size: the pie's diameter; donut: a hole in the middle, with the total in it (or `inside`)
/// - colors: per slice (default: the accent, then the palette)
/// - percent: the legend shows each slice's share; values: and its value (digits, unit as above)
/// - title, caption: as bar-chart
#let pie-chart(
  data,
  size: 4.2cm,
  donut: false,
  inside: auto,
  colors: auto,
  percent: true,
  values: true,
  digits: auto,
  unit: none,
  title: none,
  caption: none,
) = _figure(caption, context {
  let (labels, nums) = _pairs(data)
  let slices = labels.zip(nums).filter(((l, v)) => v != none and v > 0)
  if slices.len() == 0 { return [] }
  let total = slices.map(((l, v)) => v).sum()
  let colors = _series-colors(colors, slices.len())
  let size = size.to-absolute()
  let r = size / 2
  // An arc as cubic Béziers of at most 90° each (angles clockwise from the top; y points down).
  let point(a) = (r + r * calc.cos(a), r + r * calc.sin(a))
  let arc(a0, a1) = {
    let parts = calc.max(1, int(calc.ceil((a1 - a0) / 90deg - 1e-9)))
    let d = (a1 - a0) / parts
    let kk = 4 / 3 * calc.tan(d / 4)
    range(parts).map(i => {
      let (s, e) = (a0 + i * d, a0 + (i + 1) * d)
      let (p0, p3) = (point(s), point(e))
      curve.cubic(
        (p0.at(0) - kk * r * calc.sin(s), p0.at(1) + kk * r * calc.cos(s)),
        (p3.at(0) + kk * r * calc.sin(e), p3.at(1) - kk * r * calc.cos(e)),
        p3,
      )
    })
  }
  let pie = box(width: size, height: size, {
    let a = -90deg
    if slices.len() == 1 { place(circle(radius: r, fill: colors.first())) }
    for ((l, v), color) in if slices.len() > 1 { slices.zip(colors) } else { () } {
      let sweep = 360deg * (v / total)
      place(curve(
        fill: color,
        stroke: 0.8pt + white,
        curve.move((r, r)),
        curve.line(point(a)),
        ..arc(a, a + sweep),
        curve.close(),
      ))
      a += sweep
    }
    if donut {
      place(dx: r * 0.45, dy: r * 0.45, circle(radius: r * 0.55, fill: white))
      place(dx: r * 0.45, dy: r * 0.45, box(width: r * 1.1, height: r * 1.1, align(center + horizon, {
        set text(size: 0.9em, weight: "bold")
        if inside == auto { _chart-number(total, digits, unit) } else { inside }
      })))
    }
  })
  let legend = grid(
    columns: (auto, auto, auto),
    column-gutter: 0.6em,
    row-gutter: 0.55em,
    align: (center + horizon, left + horizon, right + horizon),
    ..slices.zip(colors).map((((l, v), color)) => (
      box(width: 0.8em, height: 0.8em, fill: color, radius: 1pt),
      text(size: 0.85em, [#l]),
      text(size: 0.85em, fill: luma(80), {
        if values { _chart-number(v, digits, unit) }
        if values and percent { [ ] }
        if percent { "(" + format-number(100 * v / total, digits: 1) + "%)" }
      }),
    )).flatten(),
  )
  _chart-frame(title, align(center, grid(
    columns: (auto, auto),
    column-gutter: 1.8em,
    align: horizon,
    pie, legend,
  )))
})
