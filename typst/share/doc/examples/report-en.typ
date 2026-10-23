// An English report with @collabo/report, its data inline.
// typst compile /usr/share/doc/typst/examples/report-en.typ /tmp/report-en.pdf
#import "@collabo/report:0.1.0": *

#let quarters = ("Q1", "Q2", "Q3", "Q4")
#let revenue = (1.8, 2.1, 2.6, 3.0)
#let cost = (1.5, 1.6, 1.8, 1.9)

#show: report.with(
  title: [Annual Platform Review 2026],
  author: ("Analysis Agent",),
  organization: [Example Corp.],
  lang: "en",
  title-page: false,
  contents: false,
  summary: [Revenue grew every quarter and reached \$#format-number(revenue.sum(), digits: 1) M for the year, while
    costs rose more slowly. Margins improved from #calc.round((1 - cost.first() / revenue.first()) * 100) %
    to #calc.round((1 - cost.last() / revenue.last()) * 100) %.],
)

= Results
#kpis(
  (label: "Revenue", value: "$" + format-number(revenue.sum(), digits: 1) + "M", delta: "+67%", note: "Q4 vs Q1"),
  (label: "Cost", value: "$" + format-number(cost.sum(), digits: 1) + "M", delta: "+27%", good: false),
  (label: "Customers", value: 4210, delta: "+512"),
)

#bar-chart(
  quarters.zip(revenue.zip(cost)).map(((q, rc)) => (q, rc)),
  series: ("Revenue", "Cost"),
  unit: "M",
  digits: 1,
  caption: [Revenue and cost per quarter (USD)],
) <fig-rc>

As @fig-rc shows, the gap between revenue and cost widened in the second half.

#data-table(
  quarters.enumerate().map(((i, q)) => (
    Quarter: q,
    Revenue: revenue.at(i),
    Cost: cost.at(i),
    Margin: str(calc.round((1 - cost.at(i) / revenue.at(i)) * 100)) + " %",
  )),
  digits: 1,
  caption: [Quarterly figures (USD M)],
  total: "Year",
)

#callout(kind: "note")[Figures are unaudited.]
