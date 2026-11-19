//! Charts' data: what a chart shows — its kind, title, categories and series with the cells they
//! come from — and changing it: the title, or the whole table of categories × series, written into
//! the chart's cached values and into its embedded workbook (Word, PowerPoint), so Office (and
//! `docu render`) show the new numbers. In a workbook, a chart reads cells: after an edit its
//! cached values are brought up to date from them on save.

use crate::doc::AnyDoc;
use crate::formula;
use crate::opc::Package;
use crate::xml::{self, Element, Node, A, C, R};
use crate::{err, Result};
use std::collections::HashMap;
use std::fmt::Write;

const CHART_TYPES: &[&str] = &["barChart", "bar3DChart", "lineChart", "line3DChart", "areaChart", "area3DChart", "pieChart", "pie3DChart", "ofPieChart", "doughnutChart", "scatterChart", "bubbleChart", "radarChart", "stockChart", "surfaceChart", "surface3DChart"];

pub struct Series {
    pub name: String,
    pub name_ref: Option<String>,
    pub values: Vec<Option<f64>>,
    pub values_ref: Option<String>,
    pub categories: Vec<String>,
    pub categories_ref: Option<String>,
}

pub struct Info {
    pub part: String,
    pub kinds: Vec<String>,
    pub title: Option<String>,
    pub series: Vec<Series>,
}

fn child<'a>(e: &'a Element, name: &str) -> Option<&'a Element> {
    e.elements().find(|c| c.local() == name)
}

fn ref_and_cache(e: &Element) -> (Option<String>, Vec<String>) {
    let r = e.elements().find(|c| matches!(c.local(), "strRef" | "numRef" | "multiLvlStrRef"));
    let f = r.and_then(|r| child(r, "f")).map(|f| f.text());
    let cache = e.descendants().into_iter().find(|c| matches!(c.local(), "strCache" | "numCache" | "strLit" | "numLit" | "lvl"));
    let mut out = Vec::new();
    if let Some(cache) = cache {
        let n: usize = child(cache, "ptCount").and_then(|p| p.attr_ns(xml::NONE, "val")).and_then(|v| v.parse().ok()).unwrap_or(0);
        out = vec![String::new(); n];
        for p in cache.elements().filter(|c| c.local() == "pt") {
            let i: usize = p.attr_ns(xml::NONE, "idx").and_then(|v| v.parse().ok()).unwrap_or(0);
            if i >= out.len() {
                out.resize(i + 1, String::new());
            }
            out[i] = child(p, "v").map(|v| v.text()).unwrap_or_default();
        }
    } else if let Some(v) = child(e, "v") {
        out.push(v.text());
    }
    (f, out)
}

pub fn info(pkg: &Package, part: &str) -> Result<Info> {
    let doc = pkg.xml(part)?;
    let chart = child(&doc.root, "chart").ok_or_else(|| err(format!("{part}: not a chart")))?;
    let plot = child(chart, "plotArea").ok_or_else(|| err(format!("{part}: no plot area")))?;
    let mut kinds = Vec::new();
    let mut series = Vec::new();
    for g in plot.elements().filter(|g| CHART_TYPES.contains(&g.local())) {
        let mut kind = g.local().trim_end_matches("Chart").to_string();
        if let Some(d) = child(g, "barDir").and_then(|d| d.attr_ns(xml::NONE, "val")) {
            kind = if d == "bar" { "bar".into() } else { "column".into() };
        }
        if let Some(gr) = child(g, "grouping").and_then(|d| d.attr_ns(xml::NONE, "val")).filter(|v| *v != "standard" && *v != "clustered") {
            kind = format!("{kind} ({gr})");
        }
        kinds.push(kind);
        for s in g.elements().filter(|e| e.local() == "ser") {
            let (name_ref, name) = child(s, "tx").map(ref_and_cache).unwrap_or((None, Vec::new()));
            let (values_ref, vals) = child(s, "val").or_else(|| child(s, "yVal")).map(ref_and_cache).unwrap_or((None, Vec::new()));
            let (categories_ref, categories) = child(s, "cat").or_else(|| child(s, "xVal")).map(ref_and_cache).unwrap_or((None, Vec::new()));
            series.push(Series { name: name.join(" "), name_ref, values: vals.iter().map(|v| v.trim().parse().ok()).collect(), values_ref, categories, categories_ref });
        }
    }
    let title = child(chart, "title").map(|t| {
        child(t, "tx").map(|tx| {
            let rich = child(tx, "rich");
            match rich {
                Some(r) => r.elements().filter(|p| p.local() == "p").map(|p| p.descendants().into_iter().filter(|x| x.local() == "t").map(|x| x.text()).collect::<String>()).collect::<Vec<_>>().join("\n"),
                None => ref_and_cache(tx).1.join(" "),
            }
        })
        .unwrap_or_default()
    });
    Ok(Info { part: part.to_string(), kinds, title, series })
}

fn num(v: f64) -> String {
    crate::numfmt::general(v)
}

impl Info {
    pub fn print(&self) -> String {
        let mut o = String::new();
        let title = match &self.title {
            Some(t) if t.is_empty() && self.series.len() == 1 => format!(" — title {:?} (the series' name)", self.series[0].name),
            Some(t) => format!(" — title {t:?}"),
            None => String::new(),
        };
        let _ = writeln!(o, "chart: {}{title}", self.kinds.join(" + "));
        let cats = self.series.iter().find(|s| !s.categories.is_empty());
        if let Some(c) = cats {
            let _ = writeln!(o, "categories: {}{}", c.categories.join(" | "), c.categories_ref.as_ref().map(|r| format!("   ({r})")).unwrap_or_default());
        }
        for (i, s) in self.series.iter().enumerate() {
            let vals: Vec<String> = s.values.iter().map(|v| v.map(num).unwrap_or_default()).collect();
            let _ = writeln!(o, "series {} {:?}: {}{}", i + 1, s.name, vals.join(" | "), s.values_ref.as_ref().map(|r| format!("   ({r})")).unwrap_or_default());
        }
        // The same as a table, as `chart ... data` takes it.
        let n = self.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
        let mut rows = vec![std::iter::once(String::new()).chain(self.series.iter().map(|s| s.name.clone())).collect::<Vec<_>>()];
        for k in 0..n {
            let mut r = vec![cats.and_then(|c| c.categories.get(k).cloned()).unwrap_or_else(|| (k + 1).to_string())];
            r.extend(self.series.iter().map(|s| s.values.get(k).copied().flatten().map(num).unwrap_or_default()));
            rows.push(r);
        }
        let table: Vec<String> = rows.iter().map(|r| r.iter().map(|c| c.replace(['|', ';'], " ")).collect::<Vec<_>>().join("|")).collect();
        let _ = writeln!(o, "data: \"{}\"", table.join(";"));
        o
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "part": self.part,
            "kinds": self.kinds,
            "title": self.title,
            "series": self.series.iter().map(|s| serde_json::json!({
                "name": s.name, "name_ref": s.name_ref, "values": s.values, "values_ref": s.values_ref,
                "categories": s.categories, "categories_ref": s.categories_ref,
            })).collect::<Vec<_>>(),
        })
    }
}

// ── finding a document's charts ─────────────────────────────────────────────────────────────

/// The chart parts an address names, in order: a Word paragraph (or range), a slide or a shape,
/// a sheet.
pub fn charts_at(d: &AnyDoc, addr: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let from = |e: &Element, rels: &crate::opc::Rels, out: &mut Vec<String>| {
        for c in e.descendants() {
            if c.local() == "chart" && c.ns() == C {
                if let Some(p) = c.attr_ns(R, "id").and_then(|id| rels.part(id)) {
                    out.push(p);
                }
            }
        }
    };
    match d {
        AnyDoc::Docx(x) => {
            let crate::docx::Target::Paras { story, paras, .. } = x.resolve(addr)? else {
                return Err(err(format!("{addr}: a chart is in a paragraph (p5)")));
            };
            let part = &x.parts[x.stories[story].part];
            let rels = if x.stories[story].part == 0 { x.rels.clone() } else { x.pkg.rels(&part.name)? };
            for p in &paras {
                from(x.para_el(p), &rels, &mut out);
            }
        }
        AnyDoc::Pptx(x) => match x.resolve(addr)? {
            crate::pptx::Target::Slide(si) => from(&x.slides[si].doc.root, &x.slides[si].rels, &mut out),
            crate::pptx::Target::Shape(si, loc) => from(crate::text::at(&x.slides[si].doc.root, &loc.path), &x.slides[si].rels, &mut out),
            _ => return Err(err(format!("{addr}: a chart is a shape (s2.sh3) on a slide"))),
        },
        AnyDoc::Xlsx(x) => {
            let name = addr.split('!').next().unwrap_or(addr).trim_matches('\'');
            let si = x.sheet_index(name).ok_or_else(|| err(format!("no sheet {name:?}; the sheets are: {}", x.sheet_names().join(", "))))?;
            out = sheet_charts(&x.pkg, &x.sheets[si].part);
        }
        _ => return Err(err(format!("charts are read and edited in .docx, .pptx and .xlsx files (docu convert this {} first)", d.kind()))),
    }
    Ok(out)
}

/// The charts of a worksheet or chart sheet (through its drawing).
pub fn sheet_charts(pkg: &Package, sheet_part: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (Ok(doc), Ok(rels)) = (pkg.xml(sheet_part), pkg.rels(sheet_part)) else { return out };
    let Some(rid) = doc.root.elements().find(|e| e.local() == "drawing").and_then(|d| d.attr_ns(R, "id")) else { return out };
    let Some(dp) = rels.part(rid) else { return out };
    let (Ok(drawing), Ok(drels)) = (pkg.xml(&dp), pkg.rels(&dp)) else { return out };
    for c in drawing.root.descendants() {
        if c.local() == "chart" && c.ns() == C {
            if let Some(p) = c.attr_ns(R, "id").and_then(|id| drels.part(id)) {
                out.push(p);
            }
        }
    }
    out
}

fn pkg_mut(d: &mut AnyDoc) -> Result<&mut Package> {
    match d {
        AnyDoc::Docx(x) => Ok(&mut x.pkg),
        AnyDoc::Pptx(x) => Ok(&mut x.pkg),
        AnyDoc::Xlsx(x) => Ok(&mut x.pkg),
        _ => Err(err("charts are edited in .docx, .pptx and .xlsx files")),
    }
}

pub fn pkg_of(d: &AnyDoc) -> Option<&Package> {
    match d {
        AnyDoc::Docx(x) => Some(&x.pkg),
        AnyDoc::Pptx(x) => Some(&x.pkg),
        AnyDoc::Xlsx(x) => Some(&x.pkg),
        _ => None,
    }
}

// ── editing ─────────────────────────────────────────────────────────────────────────────────

fn frag(src: &str) -> Result<Element> {
    Ok(xml::fragment(src, &[("c", C), ("a", A), ("r", R)])?.remove(0))
}

/// Sets (or with `None` removes) a chart's title.
pub fn set_title(d: &mut AnyDoc, part: &str, title: Option<&str>) -> Result<()> {
    let pkg = pkg_mut(d)?;
    let mut doc = pkg.xml(part)?;
    let cp = doc.prefix_for(C);
    let ap = doc.prefix_for(A);
    let retag = |s: String| s.replace("<c:", &format!("<{cp}:")).replace("</c:", &format!("</{cp}:")).replace("<a:", &format!("<{ap}:")).replace("</a:", &format!("</{ap}:"));
    let bind = |s: &str| -> Result<Element> { Ok(xml::fragment(s, &[(cp.as_str(), C), (ap.as_str(), A)])?.remove(0)) };
    let chart = doc.root.elements_mut().find(|e| e.local() == "chart").ok_or_else(|| err("not a chart"))?;
    let set_auto_deleted = |chart: &mut Element, v: &str| -> Result<()> {
        let e = bind(&retag(format!("<c:autoTitleDeleted val=\"{v}\"/>")))?;
        if let Some(i) = chart.children.iter().position(|n| n.as_element().map(|x| x.local() == "autoTitleDeleted").unwrap_or(false)) {
            if chart.children[i].as_element().and_then(|old| old.attr_ns(xml::NONE, "val")) != Some(v) {
                chart.children[i] = Node::Element(e);
            }
        } else {
            let at = chart.children.iter().position(|n| n.as_element().map(|x| x.local() != "title").unwrap_or(false)).unwrap_or(0);
            chart.children.insert(at, Node::Element(e));
        }
        Ok(())
    };
    match title {
        None => {
            chart.children.retain(|n| !n.as_element().map(|e| e.local() == "title").unwrap_or(false));
            set_auto_deleted(chart, "1")?;
        }
        Some(t) => {
            let paras: String = t.split('\n').map(|l| format!("<a:p><a:r><a:t>{}</a:t></a:r></a:p>", xml::escape(l))).collect();
            let rich = bind(&retag(format!("<c:tx><c:rich><a:bodyPr/><a:lstStyle/>{paras}</c:rich></c:tx>")))?;
            let ti = chart.children.iter().position(|n| n.as_element().map(|e| e.local() == "title").unwrap_or(false));
            match ti {
                Some(ti) => {
                    let te = chart.children[ti].as_element_mut().unwrap();
                    let txi = te.children.iter().position(|n| n.as_element().map(|e| e.local() == "tx" && e.elements().any(|r| r.local() == "rich")).unwrap_or(false));
                    match txi {
                        Some(txi) => {
                            let tx = te.children[txi].as_element_mut().unwrap();
                            let r = tx.elements_mut().find(|e| e.local() == "rich").unwrap();
                            crate::pptx::set_body_text(r, t);
                        }
                        None => {
                            te.children.retain(|n| !n.as_element().map(|e| e.local() == "tx").unwrap_or(false));
                            te.children.insert(0, Node::Element(rich));
                        }
                    }
                }
                None => {
                    let mut te = bind(&retag("<c:title><c:overlay val=\"0\"/></c:title>".into()))?;
                    te.children.insert(0, Node::Element(rich));
                    chart.children.insert(0, Node::Element(te));
                }
            }
            set_auto_deleted(chart, "0")?;
        }
    }
    pkg.put_xml(part, &doc);
    Ok(())
}

fn col_letter(c: u32) -> String {
    formula::col_name(c)
}

fn is_number(s: &str) -> bool {
    !s.trim().is_empty() && s.trim().parse::<f64>().is_ok()
}

/// Replaces a chart's data with a table: the first row names the series (its first cell is the
/// categories' heading), each further row is a category and its values. Series are added (like
/// the last one) or removed to match; the cell references become the usual layout of a chart's
/// data sheet (categories in A2…, series in B, C, …), written into the embedded workbook too.
pub fn set_data(d: &mut AnyDoc, part: &str, rows: &[Vec<String>]) -> Result<String> {
    if matches!(d, AnyDoc::Xlsx(_)) {
        return Err(err("a workbook's chart shows its cells: change them with `docu set` (the chart follows on save); `docu chart FILE SHEET` shows which"));
    }
    if rows.len() < 2 || rows[0].len() < 2 {
        return Err(err("chart data is a table: \"Category|Series 1|Series 2;Q1|10|12;Q2|11|14\" (a header row, then a row per category)"));
    }
    let names: Vec<String> = rows[0][1..].to_vec();
    let cats: Vec<String> = rows[1..].iter().map(|r| r.first().cloned().unwrap_or_default()).collect();
    let ns = names.len();
    let nc = cats.len();
    let value = |k: usize, s: usize| -> Option<f64> { rows.get(k + 1).and_then(|r| r.get(s + 1)).and_then(|v| v.trim().replace(',', "").parse().ok()) };
    let numeric_cats = cats.iter().all(|c| is_number(c));

    let pkg = pkg_mut(d)?;
    let mut doc = pkg.xml(part)?;
    let cp = doc.prefix_for(C);
    let old = info(pkg, part)?;
    let sheet = old.series.iter().find_map(|s| s.values_ref.as_deref().or(s.categories_ref.as_deref()))
        .and_then(|f| f.rsplit_once('!').map(|(s, _)| s.trim_matches('\'').to_string())).unwrap_or_else(|| "Sheet1".into());
    let qs = formula::quote_sheet(&sheet);
    let cat_ref = format!("{qs}!$A$2:$A${}", nc + 1);
    let retag = |s: String| if cp == "c" { s } else { s.replace("<c:", &format!("<{cp}:")).replace("</c:", &format!("</{cp}:")) };

    // The series elements, in order (path of their group, index in it).
    let plot_i = doc.root.elements().position(|e| e.local() == "chart").ok_or_else(|| err("not a chart"))?;
    let chart = doc.root.elements_mut().nth(plot_i).unwrap();
    let plot = chart.elements_mut().find(|e| e.local() == "plotArea").ok_or_else(|| err("no plot area"))?;
    let groups: Vec<usize> = plot.children.iter().enumerate().filter(|(_, n)| n.as_element().map(|e| CHART_TYPES.contains(&e.local())).unwrap_or(false)).map(|(i, _)| i).collect();
    let mut sers: Vec<(usize, usize)> = Vec::new();
    for &gi in &groups {
        let g = plot.children[gi].as_element().unwrap();
        for (i, n) in g.children.iter().enumerate() {
            if n.as_element().map(|e| e.local() == "ser").unwrap_or(false) {
                sers.push((gi, i));
            }
        }
    }
    if sers.is_empty() {
        return Err(err("the chart has no series to take as a model"));
    }
    // Fewer series: remove the last ones.
    while sers.len() > ns {
        let (gi, i) = sers.pop().unwrap();
        plot.children[gi].as_element_mut().unwrap().children.remove(i);
    }
    // More: copies of the last one, after it.
    let mut max_idx = 0;
    for &(gi, i) in &sers {
        let s = plot.children[gi].as_element().unwrap().children[i].as_element().unwrap();
        for k in ["idx", "order"] {
            max_idx = max_idx.max(child(s, k).and_then(|e| e.attr_ns(xml::NONE, "val")).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0));
        }
    }
    while sers.len() < ns {
        let (gi, i) = *sers.last().unwrap();
        let g = plot.children[gi].as_element_mut().unwrap();
        let mut s = g.children[i].as_element().unwrap().clone();
        max_idx += 1;
        for e in s.elements_mut() {
            if matches!(e.local(), "idx" | "order") {
                e.set_attr("val", xml::NONE, &max_idx.to_string());
            }
        }
        // Its own color: the default for its index, not the copied one's.
        s.children.retain(|n| !n.as_element().map(|e| matches!(e.local(), "spPr" | "dPt" | "dLbls")).unwrap_or(false));
        g.children.insert(i + 1, Node::Element(s));
        sers.push((gi, i + 1));
    }
    // Each series' name, categories and values.
    for (k, &(gi, i)) in sers.iter().enumerate() {
        let col = col_letter(k as u32 + 1);
        let g = plot.children[gi].as_element_mut().unwrap();
        let s = g.children[i].as_element_mut().unwrap();
        let scatter = s.elements().any(|e| e.local() == "yVal");
        let (cat_name, val_name) = if scatter { ("xVal", "yVal") } else { ("cat", "val") };
        let fmt = s.elements().find(|e| e.local() == val_name).and_then(|v| v.descendants().into_iter().find(|x| x.local() == "formatCode").map(|x| x.text())).unwrap_or_else(|| "General".into());
        let tx = retag(format!("<c:tx><c:strRef><c:f>{}</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>{}</c:v></c:pt></c:strCache></c:strRef></c:tx>", xml::escape(&format!("{qs}!${col}$1")), xml::escape(&names[k])));
        let cat_pts: String = cats.iter().enumerate().map(|(i, c)| format!("<c:pt idx=\"{i}\"><c:v>{}</c:v></c:pt>", xml::escape(c))).collect();
        let cat = if numeric_cats || scatter {
            retag(format!("<c:{cat_name}><c:numRef><c:f>{}</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"{nc}\"/>{cat_pts}</c:numCache></c:numRef></c:{cat_name}>", xml::escape(&cat_ref)))
        } else {
            retag(format!("<c:{cat_name}><c:strRef><c:f>{}</c:f><c:strCache><c:ptCount val=\"{nc}\"/>{cat_pts}</c:strCache></c:strRef></c:{cat_name}>", xml::escape(&cat_ref)))
        };
        let val_pts: String = (0..nc).filter_map(|i| value(i, k).map(|v| format!("<c:pt idx=\"{i}\"><c:v>{}</c:v></c:pt>", num(v)))).collect();
        let val = retag(format!(
            "<c:{val_name}><c:numRef><c:f>{}</c:f><c:numCache><c:formatCode>{}</c:formatCode><c:ptCount val=\"{nc}\"/>{val_pts}</c:numCache></c:numRef></c:{val_name}>",
            xml::escape(&format!("{qs}!${col}$2:${col}${}", nc + 1)),
            xml::escape(&fmt)
        ));
        let bind = |src: &str| -> Result<Element> { Ok(xml::fragment(src, &[(cp.as_str(), C)])?.remove(0)) };
        let order = ["idx", "order", "tx", "spPr", "invertIfNegative", "pictureOptions", "marker", "dPt", "dLbls", "trendline", "errBars", "cat", "val", "xVal", "yVal", "smooth", "shape", "bubbleSize", "bubble3D", "explosion", "extLst"];
        for (name, src) in [("tx", tx), (cat_name, cat), (val_name, val)] {
            let e = bind(&src)?;
            let _ = name;
            xml::put_in_order(s, e, &order);
        }
        // Points past the end lose their own formatting.
        s.children.retain(|n| {
            n.as_element()
                .map(|e| !(e.local() == "dPt" && child(e, "idx").and_then(|i| i.attr_ns(xml::NONE, "val")).and_then(|v| v.parse::<usize>().ok()).map(|i| i >= nc).unwrap_or(false)))
                .unwrap_or(true)
        });
    }
    pkg.put_xml(part, &doc);

    // The embedded workbook: the same table on its sheet, the old ranges cleared.
    let rels = pkg.rels(part)?;
    let ext = pkg.xml(part)?.root.elements().find(|e| e.local() == "externalData").and_then(|e| e.attr_ns(R, "id")).map(str::to_string);
    let mut note = String::new();
    if let Some(wb_part) = ext.and_then(|id| rels.part(&id)) {
        if wb_part.ends_with(".xlsx") {
            let bytes = pkg.zip.read(&wb_part)?;
            let mut wb = AnyDoc::from_bytes(bytes, "embedded.xlsx")?;
            if let AnyDoc::Xlsx(x) = &mut wb {
                let si = x.sheet_index(&sheet).unwrap_or(0);
                let sname = formula::quote_sheet(&x.sheets[si].name);
                // Clear what the chart read before (a table's worth around it).
                let (er, ec) = x.extent(si);
                let clear = format!("{sname}!A1:{}{}", col_letter(ec.max(old.series.len() as u32)), er.max(old.series.iter().map(|s| s.values.len()).max().unwrap_or(0) as u32) + 1);
                let _ = x.delete(&clear);
                let block: Vec<String> = rows.iter().map(|r| r.iter().map(|c| c.replace(['|', ';'], " ")).collect::<Vec<_>>().join("|")).collect();
                x.set(&format!("{sname}!A1"), &block.join(";"))?;
                resize_tables(x, si, ns as u32, nc as u32, &rows[0]);
                let out = wb.save()?;
                pkg.zip.write(&wb_part, out);
                note = format!(" and its data sheet ({wb_part})");
            }
        }
    }
    Ok(format!("{ns} series × {nc} categories in {part}{note}"))
}

/// A table (list object) on the data sheet covers the new data.
fn resize_tables(x: &mut crate::xlsx::Xlsx, si: usize, ns: u32, nc: u32, header: &[String]) {
    let part = x.sheets[si].part.clone();
    let Ok(rels) = x.pkg.rels(&part) else { return };
    for r in rels.list.iter().filter(|r| r.kind() == "table") {
        let tp = crate::opc::resolve(&part, &r.target);
        let Ok(mut t) = x.pkg.xml(&tp) else { continue };
        let area = format!("A1:{}{}", col_letter(ns), nc + 1);
        t.root.set_attr("ref", xml::NONE, &area);
        if let Some(af) = t.root.elements_mut().find(|e| e.local() == "autoFilter") {
            af.set_attr("ref", xml::NONE, &area);
        }
        if let Some(tc) = t.root.elements_mut().find(|e| e.local() == "tableColumns") {
            // New columns in the same namespace and prefix as their parent.
            let mut cols = Vec::new();
            for (i, h) in header.iter().take(ns as usize + 1).enumerate() {
                let mut c = tc.sibling("tableColumn");
                c.set_attr("id", xml::NONE, &(i + 1).to_string());
                c.set_attr("name", xml::NONE, if h.is_empty() { " " } else { h });
                cols.push(Node::Element(c));
            }
            tc.children = cols;
            tc.set_attr("count", xml::NONE, &(ns + 1).to_string());
        }
        x.pkg.put_xml(&tp, &t);
    }
}

// ── a workbook's charts follow its cells ────────────────────────────────────────────────────

/// Brings the cached values of a workbook's charts up to date with the cells they read; charts
/// whose caches already match are left as they are.
pub fn refresh_xlsx(x: &mut crate::xlsx::Xlsx) -> usize {
    let mut parts: Vec<String> = Vec::new();
    for s in &x.sheets {
        for p in sheet_charts(&x.pkg, &s.part) {
            if !parts.contains(&p) {
                parts.push(p);
            }
        }
    }
    let mut values: SheetValues = HashMap::new();
    let mut changed = 0;
    for part in parts {
        let Ok(mut doc) = x.pkg.xml(&part) else { continue };
        let mut dirty = false;
        let cp = doc.root.prefix().to_string();
        let xr: &crate::xlsx::Xlsx = x;
        visit_refs(&mut doc.root, &mut |r: &mut Element| {
            if refresh_ref(xr, r, &cp, &mut values) {
                dirty = true;
            }
        });
        if dirty {
            x.pkg.put_xml(&part, &doc);
            changed += 1;
        }
    }
    let _ = frag;
    changed
}

/// Calls `f` on each `numRef`/`strRef` below `e`.
fn visit_refs(e: &mut Element, f: &mut dyn FnMut(&mut Element)) {
    for c in e.elements_mut() {
        if matches!(c.local(), "numRef" | "strRef") {
            f(c);
        } else {
            visit_refs(c, f);
        }
    }
}

type SheetValues = HashMap<usize, HashMap<(u32, u32), (crate::formula::Val, u32)>>;

/// One reference's cache brought up to date with its cells; whether it changed.
fn refresh_ref(x: &crate::xlsx::Xlsx, r: &mut Element, cp: &str, values: &mut SheetValues) -> bool {
    let Some(f) = child(r, "f").map(|f| f.text()) else { return false };
    let Some((sheet, range)) = f.rsplit_once('!') else { return false };
    let Some(si) = x.sheet_index(sheet.trim_matches('\'')) else { return false };
    let Some(area) = formula::parse_area(&range.replace('$', "")) else { return false };
    let cells = values.entry(si).or_insert_with(|| {
        x.cells(si)
            .into_iter()
            .map(|c| {
                let v = match c.raw {
                    crate::xlsx::Raw::Value(v) | crate::xlsx::Raw::Formula(_, v) => v,
                };
                ((c.row, c.col), (v, c.style))
            })
            .collect()
    });
    let (r0, r1) = area.rows();
    let (c0, c1) = area.cols();
    let numeric = r.local() == "numRef";
    let mut pts: Vec<Option<String>> = Vec::new();
    for rr in r0..=r1 {
        for cc in c0..=c1 {
            pts.push(match cells.get(&(rr, cc)) {
                None | Some((crate::formula::Val::Empty, _)) => None,
                Some((crate::formula::Val::Num(n), st)) => Some(if numeric { num(*n) } else { x.display(&crate::formula::Val::Num(*n), *st) }),
                Some((val, st)) => {
                    if numeric {
                        None
                    } else {
                        Some(x.display(val, *st))
                    }
                }
            });
        }
    }
    let cache_name = if numeric { "numCache" } else { "strCache" };
    let current: Vec<Option<String>> = match child(r, cache_name) {
        Some(cache) => {
            let n: usize = child(cache, "ptCount").and_then(|p| p.attr_ns(xml::NONE, "val")).and_then(|v| v.parse().ok()).unwrap_or(0);
            let mut v = vec![None; n];
            for p in cache.elements().filter(|c| c.local() == "pt") {
                let i: usize = p.attr_ns(xml::NONE, "idx").and_then(|v| v.parse().ok()).unwrap_or(0);
                if i < v.len() {
                    v[i] = child(p, "v").map(|t| t.text());
                }
            }
            v
        }
        None => Vec::new(),
    };
    let same = current.len() == pts.len()
        && current.iter().zip(&pts).all(|(a, b)| match (a, b) {
            (Some(a), Some(b)) if numeric => a.trim().parse::<f64>().ok() == b.parse::<f64>().ok(),
            (a, b) => a == b,
        });
    if same {
        return false;
    }
    let fmt = child(r, cache_name).and_then(|c| child(c, "formatCode")).map(|f| f.text());
    let p = if cp.is_empty() { "c".to_string() } else { cp.to_string() };
    let mut src = format!("<{p}:{cache_name}>");
    if numeric {
        let _ = write!(src, "<{p}:formatCode>{}</{p}:formatCode>", xml::escape(&fmt.unwrap_or_else(|| "General".into())));
    }
    let _ = write!(src, "<{p}:ptCount val=\"{}\"/>", pts.len());
    for (i, v) in pts.iter().enumerate() {
        if let Some(v) = v {
            let _ = write!(src, "<{p}:pt idx=\"{i}\"><{p}:v>{}</{p}:v></{p}:pt>", xml::escape(v));
        }
    }
    let _ = write!(src, "</{p}:{cache_name}>");
    match xml::fragment(&src, &[(p.as_str(), C)]) {
        Ok(mut els) => {
            let e = els.remove(0);
            r.children.retain(|n| !n.as_element().map(|e| e.local() == cache_name).unwrap_or(false));
            r.push(e);
            true
        }
        Err(_) => false,
    }
}
