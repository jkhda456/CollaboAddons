// A report from @collabo/report. Compile: typst compile main.typ   (→ main.pdf)
// Preview pages as images: typst compile main.typ 'page-{p}.png' --ppi 80
//
// The numbers come from data.json and incidents.csv beside this file: change the data, not the
// layout. For an English report: lang: "en".
#import "@collabo/report:0.1.0": *

#let data = json("data.json")

#show: report.with(
  title: [서비스 운영 보고서],
  subtitle: [#data.period 지표와 개선 과제],
  author: "운영 분석팀",
  organization: [collaboCore],
  version: "1.0",
  // status: [검토 중],
  // confidential: "대외비",
  // watermark: "DRAFT",
  summary: [
    #data.period 매출은 전 분기 대비 8.2% 늘었고, 장애 시간은 절반 아래로 줄었다. 비용 증가는
    인프라 확장에 따른 것으로 계획 범위 안에 있다. 결제 응답 시간은 계속 살펴야 한다.
  ],
)

= 핵심 지표
#kpis(..data.kpis)

매출은 매달 늘었고(@fig-monthly), 장애는 세 건이었다(@tbl-incidents).

#callout(kind: "tip", title: [핵심 요약])[
  - 매출 증가: 신규 요금제 효과
  - 장애 감소: 배포 자동 검증 도입
]

= 분석
== 월별 매출과 비용
#bar-chart(
  data.monthly.map(m => (m.month, (m.revenue, m.cost))),
  series: ("매출", "비용"),
  unit: "억",
  caption: [월별 매출과 비용],
) <fig-monthly>

#line-chart(
  data.monthly.map(m => m.month),
  (매출: data.monthly.map(m => m.revenue), 비용: data.monthly.map(m => m.cost)),
  unit: "억",
  values: true,
  caption: [추이],
)

== 지역별 사용자
#grid(
  columns: (1fr, 1fr),
  gutter: 1.5em,
  bar-chart(data.regions, horizontal: true, unit: "%"),
  pie-chart(data.regions, unit: "%", values: false),
)

= 장애
#data-table(
  csv("incidents.csv"),
  caption: [분기 중 장애],
  total: "합계",
) <tbl-incidents>

#callout(kind: "warning")[인증서 만료로 인한 장애는 자동 갱신 도입 뒤 재발하지 않았다.]

= 결론과 권고
+ 결제 모듈의 연결 풀 상한을 상시 감시한다. #badge[진행 중]
+ 배포 설정 검증을 모든 서비스로 넓힌다. #badge(color: rgb("#16a34a"))[완료]
+ 다음 분기 비용 목표를 #progress(0.72) 수준에서 관리한다.
