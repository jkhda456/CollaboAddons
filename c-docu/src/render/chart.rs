//! Charts (DrawingML chart parts) drawn with Typst shapes: bar and column (clustered, stacked,
//! percent), line, area, pie and doughnut, scatter and radar charts, combined on one or two value
//! axes, with their titles, axes, gridlines, legends and data labels — from the values the chart
//! part caches (Office always writes them), laid out as Office 2013 and later lay charts out.

use super::office::{self, Theme};
use super::typ::{fonts as tfonts, pt, rgb, s};
use super::world::Fonts;
use crate::opc::Package;
use crate::xml::{Element, NONE};
use std::collections::HashMap;
use std::fmt::Write;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Bar,
    Line,
    Area,
    Pie,
    Doughnut,
    Scatter,
    Radar,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Grouping {
    Clustered,
    Stacked,
    Percent,
}

#[derive(Default, Clone, Copy)]
struct Labels {
    val: bool,
    pct: bool,
    cat: bool,
    ser: bool,
}

impl Labels {
    fn any(&self) -> bool {
        self.val || self.pct || self.cat || self.ser
    }
}

struct Series {
    name: String,
    cats: Vec<String>,
    vals: Vec<Option<f64>>,
    xs: Vec<Option<f64>>,
    fmt: String,
    /// Fill and line: `None` for the default, `Some(None)` for none.
    fill: Option<Option<String>>,
    line: Option<Option<String>>,
    line_w: Option<f64>,
    marker: Option<(String, f64, Option<String>)>,
    points: HashMap<usize, String>,
    labels: Labels,
    color: String,
    /// Pie slices pulled out, in percent of the radius: the series', then each point's.
    explosion: f64,
    point_explosion: HashMap<usize, f64>,
}

struct Group {
    kind: Kind,
    horizontal: bool,
    grouping: Grouping,
    vary: bool,
    gap: f64,
    overlap: f64,
    hole: f64,
    first_angle: f64,
    markers: bool,
    series: Vec<Series>,
    axes: Vec<String>,
    labels: Labels,
}

#[derive(Clone)]
struct Axis {
    id: String,
    is_val: bool,
    pos: String,
    deleted: bool,
    min: Option<f64>,
    max: Option<f64>,
    major: Option<f64>,
    reverse: bool,
    grid: bool,
    fmt: Option<String>,
    title: Option<String>,
    between: bool,
    text: TextStyle,
    line: Option<String>,
}

#[derive(Clone)]
struct TextStyle {
    size: f64,
    color: String,
    bold: bool,
}

struct Ctx<'a> {
    classic: bool,
    theme: &'a Theme,
    map: HashMap<String, String>,
    font: String,
}

fn child<'a>(e: &'a Element, name: &str) -> Option<&'a Element> {
    e.elements().find(|c| c.local() == name)
}

fn val_of(e: &Element, name: &str) -> Option<String> {
    child(e, name).and_then(|c| c.attr_ns(NONE, "val")).map(str::to_string)
}

fn flag(e: &Element, name: &str, default: bool) -> bool {
    match child(e, name) {
        Some(c) => !matches!(c.attr_ns(NONE, "val"), Some("0" | "false")),
        None => default,
    }
}

fn numv(e: &Element, name: &str) -> Option<f64> {
    val_of(e, name).and_then(|v| v.parse().ok())
}

/// The strings of a reference's cache (`c:strRef`, `c:numRef`, `c:multiLvlStrRef`, literals).
fn cache_strings(e: &Element) -> Vec<String> {
    let cache = e.descendants().into_iter().find(|c| matches!(c.local(), "strCache" | "numCache" | "strLit" | "numLit" | "lvl"));
    let Some(cache) = cache else { return Vec::new() };
    let n = numv(cache, "ptCount").unwrap_or(0.0) as usize;
    let fmt = child(cache, "formatCode").map(|f| f.text()).unwrap_or_default();
    let mut out = vec![String::new(); n];
    for p in cache.elements().filter(|c| c.local() == "pt") {
        let i: usize = p.attr_ns(NONE, "idx").and_then(|v| v.parse().ok()).unwrap_or(0);
        let v = child(p, "v").map(|v| v.text()).unwrap_or_default();
        let text = match (cache.local(), v.parse::<f64>()) {
            ("numCache" | "numLit", Ok(x)) => format_value(x, if fmt.is_empty() { "General" } else { &fmt }),
            _ => v,
        };
        if i >= out.len() {
            out.resize(i + 1, String::new());
        }
        out[i] = text;
    }
    out
}

fn cache_nums(e: &Element) -> (Vec<Option<f64>>, String) {
    let cache = e.descendants().into_iter().find(|c| matches!(c.local(), "numCache" | "numLit"));
    let Some(cache) = cache else {
        // Text where numbers belong: positions 1..n.
        let s = cache_strings(e);
        return (s.iter().map(|t| t.parse().ok()).collect(), "General".into());
    };
    let n = numv(cache, "ptCount").unwrap_or(0.0) as usize;
    let fmt = child(cache, "formatCode").map(|f| f.text()).filter(|f| !f.is_empty()).unwrap_or_else(|| "General".into());
    let mut out = vec![None; n];
    for p in cache.elements().filter(|c| c.local() == "pt") {
        let i: usize = p.attr_ns(NONE, "idx").and_then(|v| v.parse().ok()).unwrap_or(0);
        if i >= out.len() {
            out.resize(i + 1, None);
        }
        out[i] = child(p, "v").and_then(|v| v.text().trim().parse().ok());
    }
    (out, fmt)
}

fn format_value(v: f64, fmt: &str) -> String {
    if fmt.is_empty() || fmt.eq_ignore_ascii_case("general") {
        crate::numfmt::general(v)
    } else {
        crate::numfmt::format_number(v, fmt, false)
    }
}

/// Text of a `c:tx` (a reference's cache, a value, or rich text).
fn tx_text(tx: &Element) -> String {
    if let Some(rich) = child(tx, "rich") {
        let paras: Vec<String> = rich.elements().filter(|p| p.local() == "p").map(|p| p.descendants().into_iter().filter(|t| t.local() == "t").map(|t| t.text()).collect::<String>()).collect();
        return paras.join("\n");
    }
    if let Some(v) = child(tx, "v") {
        return v.text();
    }
    cache_strings(tx).join(" ")
}

impl<'a> Ctx<'a> {
    fn color(&self, parent: &Element) -> Option<String> {
        office::dml_color(parent, self.theme, &self.map, None).map(|(h, _)| h)
    }

    /// (fill, line, line width) of a `c:spPr`.
    fn sppr(&self, e: Option<&Element>) -> (Option<Option<String>>, Option<Option<String>>, Option<f64>) {
        let Some(sp) = e.and_then(|e| child(e, "spPr")) else { return (None, None, None) };
        let fill = sp.elements().find(|f| matches!(f.local(), "noFill" | "solidFill" | "gradFill" | "pattFill")).map(|f| match f.local() {
            "noFill" => None,
            "solidFill" => self.color(f),
            "gradFill" => f.descendants().into_iter().find(|g| g.local() == "gs").and_then(|g| self.color(g)),
            _ => child(f, "fgClr").and_then(|g| self.color(g)),
        });
        let ln = child(sp, "ln");
        let line = ln.and_then(|l| l.elements().find(|f| matches!(f.local(), "noFill" | "solidFill" | "gradFill"))).map(|f| match f.local() {
            "noFill" => None,
            "solidFill" => self.color(f),
            _ => f.descendants().into_iter().find(|g| g.local() == "gs").and_then(|g| self.color(g)),
        });
        let w = ln.and_then(|l| l.attr_ns(NONE, "w")).and_then(|v| v.parse::<f64>().ok()).map(|v| v / 12700.0);
        (fill, line, w)
    }

    /// The text style of an element's `c:txPr` (or rich text), over a default.
    fn text(&self, e: Option<&Element>, def: &TextStyle) -> TextStyle {
        let mut t = def.clone();
        let Some(e) = e else { return t };
        let src = child(e, "txPr").or_else(|| child(e, "tx").and_then(|x| child(x, "rich")));
        let Some(src) = src else { return t };
        for r in src.descendants() {
            if matches!(r.local(), "defRPr" | "rPr") {
                if let Some(sz) = r.attr_ns(NONE, "sz").and_then(|v| v.parse::<f64>().ok()) {
                    t.size = sz / 100.0;
                }
                if let Some(b) = r.attr_ns(NONE, "b") {
                    t.bold = b == "1" || b == "true";
                }
                if let Some(f) = child(r, "solidFill").and_then(|f| self.color(f)) {
                    t.color = f;
                }
            }
        }
        t
    }
}

/// Default series colors: the six accents, then darker and lighter rounds of them.
fn default_color(theme: &Theme, i: usize) -> String {
    let base = theme.color(&format!("accent{}", i % 6 + 1)).unwrap_or_else(|| "4472C4".into());
    let round = i / 6;
    if round == 0 {
        return base;
    }
    let mods: Vec<(String, f64)> = if round % 2 == 1 { vec![("lumMod".into(), 60000.0)] } else { vec![("lumMod".into(), 80000.0), ("lumOff".into(), 20000.0)] };
    office::transform(&base, &mods).0
}

fn est_width(t: &str, size: f64) -> f64 {
    t.chars().map(|c| if (c as u32) >= 0x1100 { 1.0 } else if c.is_ascii_uppercase() || c == 'm' || c == 'w' { 0.62 } else if matches!(c, 'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '\'' | '|' | '!' | ' ') { 0.28 } else { 0.5 }).sum::<f64>() * size
}

/// A nice axis: (low, high, step) around the data, as Excel picks them.
fn nice_scale(dmin: f64, dmax: f64, ticks: f64, fixed: (Option<f64>, Option<f64>, Option<f64>), zero_based: bool) -> (f64, f64, f64) {
    let (mut lo, mut hi) = (dmin.min(dmax), dmin.max(dmax));
    if zero_based {
        if lo > 0.0 && (hi - lo) / hi > 1.0 / 6.0 {
            lo = 0.0;
        }
        if hi < 0.0 && (lo - hi) / lo > 1.0 / 6.0 {
            hi = 0.0;
        }
    }
    if let Some(m) = fixed.0 {
        lo = m;
    }
    if let Some(m) = fixed.1 {
        hi = m;
    }
    if (hi - lo).abs() < 1e-12 {
        if hi == 0.0 {
            hi = 1.0;
        } else if hi > 0.0 {
            lo = 0.0;
        } else {
            hi = 0.0;
        }
    }
    // Excel: 5% headroom, then a major unit of 2, 5 or 10 times a power of ten that gives five
    // to ten intervals.
    let headroom = if fixed.1.is_none() && hi > 0.0 { (hi - lo) * 0.05 } else { 0.0 };
    let step = fixed.2.filter(|s| *s > 0.0).unwrap_or_else(|| {
        let range = (hi + headroom - lo).abs().max(1e-12);
        let p = 10f64.powf(range.log10().floor());
        let sc = range / p;
        let unit = if sc <= 2.0 {
            0.2
        } else if sc <= 5.0 {
            0.5
        } else {
            1.0
        };
        // Fewer lines on a small axis: the next unit of the 1-2-5 sequence.
        let mut unit = unit * p;
        let count = |u: f64| ((hi + headroom) / u - 1e-9).ceil() - (lo / u).floor();
        while count(unit) > ticks.max(2.0) {
            let m = unit / 10f64.powf(unit.log10().floor());
            unit *= if m < 1.5 { 2.0 } else if m < 3.0 { 2.5 } else { 2.0 };
        }
        unit
    });
    if fixed.0.is_none() {
        lo = (lo / step).floor() * step;
    }
    if fixed.1.is_none() {
        hi = ((hi + headroom) / step - 1e-9).ceil() * step;
        if hi <= lo {
            hi = lo + step;
        }
    }
    (lo, hi, step)
}

struct Out {
    s: String,
    font: String,
}

impl Out {
    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, fill: Option<&str>, stroke: Option<(&str, f64)>) {
        let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
        let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
        let fill = fill.map(rgb).unwrap_or_else(|| "none".into());
        let stroke = stroke.map(|(c, w)| format!("{} + {}", pt(w), rgb(c))).unwrap_or_else(|| "none".into());
        let _ = writeln!(self.s, "place(top + left, dx: {}, dy: {}, rect(width: {}, height: {}, fill: {fill}, stroke: {stroke}))", pt(x), pt(y), pt(w), pt(h));
    }

    fn line(&mut self, pts: &[(f64, f64)], color: &str, w: f64, dash: bool) {
        if pts.len() < 2 {
            return;
        }
        let stroke = if dash { format!("(paint: {}, thickness: {}, dash: \"dashed\")", rgb(color), pt(w)) } else { format!("(paint: {}, thickness: {}, join: \"round\", cap: \"round\")", rgb(color), pt(w)) };
        let v: Vec<String> = pts.iter().map(|(x, y)| format!("({}, {})", pt(*x), pt(*y))).collect();
        if pts.len() == 2 {
            let _ = writeln!(self.s, "place(top + left, line(start: {}, end: {}, stroke: {stroke}))", v[0], v[1]);
        } else {
            let _ = writeln!(self.s, "place(top + left, curve(stroke: {stroke}, curve.move({}), {}))", v[0], v[1..].iter().map(|p| format!("curve.line({p})")).collect::<Vec<_>>().join(", "));
        }
    }

    fn polygon(&mut self, pts: &[(f64, f64)], fill: Option<&str>, stroke: Option<(&str, f64)>) {
        if pts.len() < 3 {
            return;
        }
        let fill = fill.map(rgb).unwrap_or_else(|| "none".into());
        let stroke = stroke.map(|(c, w)| format!("{} + {}", pt(w), rgb(c))).unwrap_or_else(|| "none".into());
        let v: Vec<String> = pts.iter().map(|(x, y)| format!("({}, {})", pt(*x), pt(*y))).collect();
        let _ = writeln!(self.s, "place(top + left, polygon(fill: {fill}, stroke: {stroke}, {}))", v.join(", "));
    }

    fn marker(&mut self, x: f64, y: f64, sym: &str, size: f64, fill: &str, line: &str) {
        let r = size / 2.0;
        let stroke = format!("{} + {}", pt(0.75), rgb(line));
        match sym {
            "none" => {}
            "square" => self.rect(x - r, y - r, size, size, Some(fill), Some((line, 0.75))),
            "diamond" => self.polygon(&[(x, y - r), (x + r, y), (x, y + r), (x - r, y)], Some(fill), Some((line, 0.75))),
            "triangle" => self.polygon(&[(x, y - r), (x + r, y + r), (x - r, y + r)], Some(fill), Some((line, 0.75))),
            "x" | "star" | "plus" => {
                self.line(&[(x - r, y - r), (x + r, y + r)], line, 0.75, false);
                self.line(&[(x - r, y + r), (x + r, y - r)], line, 0.75, false);
            }
            "dash" => self.line(&[(x - r, y), (x + r, y)], line, 1.5, false),
            "dot" => {
                let _ = writeln!(self.s, "place(top + left, dx: {}, dy: {}, circle(radius: {}, fill: {}))", pt(x - r / 2.0), pt(y - r / 2.0), pt(r / 2.0), rgb(fill));
            }
            _ => {
                let _ = writeln!(self.s, "place(top + left, dx: {}, dy: {}, circle(radius: {}, fill: {}, stroke: {stroke}))", pt(x - r), pt(y - r), pt(r), rgb(fill));
            }
        }
    }

    /// Text with its anchor point at (x, y): h is l/c/r, v is t/m/b.
    fn text(&mut self, x: f64, y: f64, h: char, v: char, t: &str, st: &TextStyle) {
        if t.is_empty() {
            return;
        }
        let lines = t.lines().count().max(1) as f64;
        let bw = t.lines().map(|l| est_width(l, st.size)).fold(0.0, f64::max) + st.size * 2.0;
        let bh = st.size * 1.25 * lines;
        let dx = match h {
            'l' => x,
            'c' => x - bw / 2.0,
            _ => x - bw,
        };
        let dy = match v {
            't' => y,
            'm' => y - bh / 2.0,
            _ => y - bh,
        };
        let al = match h {
            'l' => "left",
            'c' => "center",
            _ => "right",
        };
        let _ = writeln!(
            self.s,
            "place(top + left, dx: {}, dy: {}, box(width: {}, height: {}, align({al} + horizon, text(font: {}, size: {}, fill: {}{}, top-edge: \"ascender\", bottom-edge: \"descender\", {}))))",
            pt(dx),
            pt(dy),
            pt(bw),
            pt(bh),
            self.font,
            pt(st.size),
            rgb(&st.color),
            if st.bold { ", weight: \"bold\"" } else { "" },
            s(t)
        );
    }

    fn vtext(&mut self, x: f64, y: f64, t: &str, st: &TextStyle) {
        let bw = est_width(t, st.size) + st.size;
        let bh = st.size * 1.3;
        let _ = writeln!(
            self.s,
            "place(top + left, dx: {}, dy: {}, rotate(-90deg, reflow: true, box(width: {}, height: {}, align(center + horizon, text(font: {}, size: {}, fill: {}{}, {})))))",
            pt(x - bh / 2.0),
            pt(y - bw / 2.0),
            pt(bw),
            pt(bh),
            self.font,
            pt(st.size),
            rgb(&st.color),
            if st.bold { ", weight: \"bold\"" } else { "" },
            s(t)
        );
    }
}

fn parse_series(c: &Ctx, ser: &Element, i: usize, kind: Kind) -> Series {
    let name = child(ser, "tx").map(tx_text).unwrap_or_else(|| format!("Series{}", i + 1));
    let idx: usize = numv(ser, "idx").map(|v| v as usize).unwrap_or(i);
    let (vals, fmt) = child(ser, "val").or_else(|| child(ser, "yVal")).map(cache_nums).unwrap_or_default();
    let cats = child(ser, "cat").or_else(|| child(ser, "xVal")).map(cache_strings).unwrap_or_default();
    let xs = if kind == Kind::Scatter { child(ser, "xVal").map(|x| cache_nums(x).0).unwrap_or_default() } else { Vec::new() };
    let (fill, line, line_w) = c.sppr(Some(ser));
    let marker = child(ser, "marker").map(|m| {
        let sym = val_of(m, "symbol").unwrap_or_else(|| "circle".into());
        let size = numv(m, "size").unwrap_or(5.0);
        let (mf, _, _) = c.sppr(Some(m));
        (sym, size, mf.flatten())
    });
    let mut points = HashMap::new();
    let mut point_explosion = HashMap::new();
    for dp in ser.elements().filter(|e| e.local() == "dPt") {
        let k = numv(dp, "idx").unwrap_or(0.0) as usize;
        if let Some(x) = numv(dp, "explosion") {
            point_explosion.insert(k, x);
        }
        let (f, l, _) = c.sppr(Some(dp));
        if let Some(col) = f.flatten().or(if kind == Kind::Line { l.flatten() } else { None }) {
            points.insert(k, col);
        }
    }
    let labels = child(ser, "dLbls").map(|d| labels_of(d, Labels::default())).unwrap_or_default();
    let color = default_color(c.theme, idx);
    let marker = marker.or_else(|| if c.classic { Some((["diamond", "square", "triangle", "x", "star", "circle", "plus"][idx % 7].to_string(), 5.0, None)) } else { None });
    Series { name, cats, vals, xs, fmt, fill, line, line_w, marker, points, labels, color, explosion: numv(ser, "explosion").unwrap_or(0.0), point_explosion }
}

fn labels_of(d: &Element, base: Labels) -> Labels {
    if flag(d, "delete", false) {
        return Labels::default();
    }
    Labels { val: flag(d, "showVal", base.val), pct: flag(d, "showPercent", base.pct), cat: flag(d, "showCatName", base.cat), ser: flag(d, "showSerName", base.ser) }
}

fn parse_group(c: &Ctx, g: &Element) -> Option<Group> {
    let (kind, horizontal) = match g.local() {
        "barChart" | "bar3DChart" => (Kind::Bar, val_of(g, "barDir").as_deref() == Some("bar")),
        "lineChart" | "line3DChart" | "stockChart" => (Kind::Line, false),
        "areaChart" | "area3DChart" => (Kind::Area, false),
        "pieChart" | "pie3DChart" | "ofPieChart" => (Kind::Pie, false),
        "doughnutChart" => (Kind::Doughnut, false),
        "scatterChart" | "bubbleChart" => (Kind::Scatter, false),
        "radarChart" => (Kind::Radar, false),
        _ => return None,
    };
    let grouping = match val_of(g, "grouping").as_deref() {
        Some("stacked") => Grouping::Stacked,
        Some("percentStacked") => Grouping::Percent,
        _ => Grouping::Clustered,
    };
    let labels = child(g, "dLbls").map(|d| labels_of(d, Labels::default())).unwrap_or_default();
    let series: Vec<Series> = g.elements().filter(|e| e.local() == "ser").enumerate().map(|(i, s)| parse_series(c, s, i, kind)).collect();
    let axes: Vec<String> = g.elements().filter(|e| e.local() == "axId").filter_map(|e| e.attr_ns(NONE, "val").map(str::to_string)).collect();
    Some(Group {
        kind,
        horizontal,
        grouping,
        vary: flag(g, "varyColors", false),
        gap: numv(g, "gapWidth").unwrap_or(150.0),
        overlap: numv(g, "overlap").unwrap_or(if grouping == Grouping::Clustered { 0.0 } else { 100.0 }),
        hole: numv(g, "holeSize").unwrap_or(50.0),
        first_angle: numv(g, "firstSliceAng").unwrap_or(0.0),
        markers: flag(g, "marker", true),
        series,
        axes,
        labels,
    })
}

fn parse_axis(c: &Ctx, e: &Element, def: &TextStyle) -> Axis {
    let sc = child(e, "scaling");
    let fmt = child(e, "numFmt").filter(|f| !matches!(f.attr_ns(NONE, "sourceLinked"), Some("1" | "true"))).and_then(|f| f.attr_ns(NONE, "formatCode")).map(str::to_string);
    let (_, line, _) = c.sppr(Some(e));
    Axis {
        id: val_of(e, "axId").unwrap_or_default(),
        is_val: e.local() == "valAx",
        pos: val_of(e, "axPos").unwrap_or_else(|| "b".into()),
        deleted: flag(e, "delete", false),
        min: sc.and_then(|s| numv(s, "min")),
        max: sc.and_then(|s| numv(s, "max")),
        major: numv(e, "majorUnit"),
        reverse: sc.and_then(|s| val_of(s, "orientation")).as_deref() == Some("maxMin"),
        grid: child(e, "majorGridlines").is_some(),
        fmt,
        title: child(e, "title").map(|t| child(t, "tx").map(tx_text).unwrap_or_else(|| "Axis Title".into())),
        between: val_of(e, "crossBetween").as_deref() != Some("midCat"),
        text: c.text(Some(e), def),
        line: match line {
            Some(None) => None,
            Some(Some(col)) => Some(col),
            None => Some("BFBFBF".into()),
        },
    }
}

pub fn render_part(pkg: &Package, part: &str, w: f64, h: f64, theme: &Theme, fonts: &Fonts, warnings: &mut Vec<String>) -> String {
    match draw(pkg, part, w, h, theme, fonts) {
        Ok(s) => s,
        Err(e) => {
            warnings.push(format!("{part}: {e}"));
            format!("rect(width: {}, height: {}, stroke: 0.5pt + gray, align(center + horizon, text(fill: gray, \"[chart]\")))", pt(w), pt(h))
        }
    }
}

fn draw(pkg: &Package, part: &str, w: f64, h: f64, theme: &Theme, fonts: &Fonts) -> crate::Result<String> {
    let doc = pkg.xml(part)?;
    let space = &doc.root;
    let chart = child(space, "chart").ok_or_else(|| crate::err("no chart"))?;
    let plot = child(chart, "plotArea").ok_or_else(|| crate::err("no plot area"))?;
    let map: HashMap<String, String> = [("tx1", "dk1"), ("bg1", "lt1"), ("tx2", "dk2"), ("bg2", "lt2")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let stack = office::font_stack(fonts, &theme.minor_latin, Some(theme.minor_ea.as_str()).filter(|s| !s.is_empty()).or(Some("Malgun Gothic")));
    let mut c = Ctx { classic: false, theme, map, font: tfonts(&stack) };
    let mut o = Out { s: String::new(), font: c.font.clone() };
    // Charts of Office 2013 and later say how their text and series look; older ones (and those
    // other programs write) take the classic look: black text, a bold title, darker gridlines.
    let modern = child(space, "spPr").is_some() || plot.descendants().iter().any(|e| e.local() == "ser" && child(e, "spPr").is_some()) || plot.descendants().iter().any(|e| matches!(e.local(), "catAx" | "valAx") && child(e, "txPr").is_some());
    c.classic = !modern;
    let c = c;
    let base = c.text(Some(space), &TextStyle { size: 10.0, color: if modern { "595959".into() } else { "000000".into() }, bold: false });
    let small = TextStyle { size: (base.size * 0.9).max(6.0), ..base.clone() };
    let small = if child(space, "txPr").is_some() || !modern { base.clone() } else { small };
    let grid_color = if modern { "D9D9D9" } else { "868686" };
    // Titles: a fifth larger than the chart's own text size, or Office's 14pt (18pt classic).
    let title_size = if child(space, "txPr").is_some() { (base.size * 1.2 * 10.0).round() / 10.0 } else if modern { 14.0 } else { 18.0 };

    // Chart area: Excel gives its charts a border unless they say otherwise.
    let (fill, line, lw) = c.sppr(Some(space));
    let fill = fill.unwrap_or(Some("FFFFFF".into()));
    let line = line.unwrap_or_else(|| if part.starts_with("xl/") { Some(if modern { "D9D9D9".into() } else { "868686".into() }) } else { None });
    o.rect(0.0, 0.0, w, h, fill.as_deref(), line.as_deref().map(|l| (l, lw.unwrap_or(0.75))));

    let groups: Vec<Group> = plot.elements().filter_map(|g| parse_group(&c, g)).collect();
    if groups.is_empty() {
        return Err(crate::err("no chart type drawn yet"));
    }
    let axes: Vec<Axis> = plot.elements().filter(|e| matches!(e.local(), "catAx" | "valAx" | "dateAx" | "serAx")).map(|e| parse_axis(&c, e, &small)).collect();
    let pie_like = groups.iter().all(|g| matches!(g.kind, Kind::Pie | Kind::Doughnut));

    let pad = 7.0;
    let (mut top, mut bottom, mut left, mut right) = (pad, h - pad, pad, w - pad);

    // Title: its text, or the series name (one series) when it has none.
    let series_count: usize = groups.iter().map(|g| g.series.len()).sum();
    if let Some(t) = child(chart, "title").filter(|_| !flag(chart, "autoTitleDeleted", false)) {
        let text = child(t, "tx").map(tx_text).or_else(|| if series_count == 1 { groups.iter().flat_map(|g| g.series.iter()).next().map(|s| s.name.clone()) } else { Some("Chart Title".into()) }).unwrap_or_default();
        let st = c.text(Some(t), &TextStyle { size: title_size, color: base.color.clone(), bold: !modern });
        let lines = text.lines().count().max(1) as f64;
        o.text(w / 2.0, top, 'c', 't', &text, &st);
        top += st.size * 1.25 * lines + 4.0;
    } else if child(chart, "title").is_none() && !flag(chart, "autoTitleDeleted", true) && series_count == 1 {
        let text = groups[0].series.first().map(|s| s.name.clone()).unwrap_or_default();
        let st = TextStyle { size: title_size, bold: !modern, ..base.clone() };
        o.text(w / 2.0, top, 'c', 't', &text, &st);
        top += st.size * 1.25 + 4.0;
    }

    // Legend entries: (name, color, is_line, marker).
    let mut entries: Vec<(String, String, bool, Option<String>)> = Vec::new();
    for g in &groups {
        if (g.vary || pie_like) && g.series.len() == 1 && !matches!(g.kind, Kind::Line | Kind::Scatter | Kind::Area | Kind::Radar) {
            let s = &g.series[0];
            let names: Vec<String> = if s.cats.is_empty() { (1..=s.vals.len()).map(|i| i.to_string()).collect() } else { s.cats.clone() };
            for (k, cat) in names.iter().enumerate() {
                let col = s.points.get(&k).cloned().unwrap_or_else(|| default_color(theme, k));
                entries.push((cat.clone(), col, false, None));
            }
        } else {
            for s in &g.series {
                let col = series_color(s, g.kind);
                let is_line = matches!(g.kind, Kind::Line | Kind::Scatter | Kind::Radar) && !matches!(s.line, Some(None));
                let marker = if matches!(g.kind, Kind::Line | Kind::Scatter | Kind::Radar) && g.markers { s.marker.as_ref().map(|m| m.0.clone()).or(Some("circle".into())).filter(|m| m != "none") } else { None };
                entries.push((s.name.clone(), col, is_line, marker));
            }
        }
    }
    if let Some(lg) = child(chart, "legend") {
        let deleted: Vec<usize> = lg.elements().filter(|e| e.local() == "legendEntry" && flag(e, "delete", false)).filter_map(|e| numv(e, "idx").map(|v| v as usize)).collect();
        let entries: Vec<_> = entries.iter().enumerate().filter(|(i, _)| !deleted.contains(i)).map(|(_, e)| e.clone()).collect();
        let st = c.text(Some(lg), &small);
        let pos = val_of(lg, "legendPos").unwrap_or_else(|| "r".into());
        // Beside horizontal bars, the legend lists the series bottom-up, as the bars stand.
        let mut entries = entries;
        if matches!(pos.as_str(), "r" | "l" | "tr") && groups.iter().all(|g| g.kind == Kind::Bar && g.horizontal) && !groups.iter().any(|g| g.vary && g.series.len() == 1) {
            entries.reverse();
        }
        let sw = |e: &(String, String, bool, Option<String>)| if e.2 { 18.0 } else if e.3.is_some() { 7.0 } else { st.size * 0.7 };
        let item_w = |e: &(String, String, bool, Option<String>)| sw(e) + 4.0 + est_width(&e.0, st.size);
        let row_h = st.size * 1.4;
        let draw_entry = |o: &mut Out, x: f64, y: f64, e: &(String, String, bool, Option<String>)| {
            if e.2 {
                o.line(&[(x, y), (x + 18.0, y)], &e.1, 2.0, false);
                if let Some(m) = &e.3 {
                    o.marker(x + 9.0, y, m, 5.0, &e.1, &e.1);
                }
            } else if let Some(m) = &e.3 {
                o.marker(x + 3.5, y, m, 6.0, &e.1, &e.1);
            } else {
                let q = st.size * 0.7;
                o.rect(x, y - q / 2.0, q, q, Some(&e.1), None);
            }
            o.text(x + sw(e) + 4.0, y, 'l', 'm', &e.0, &st);
        };
        let overlay = flag(lg, "overlay", false);
        match pos.as_str() {
            "b" | "t" => {
                // Rows of entries, centred.
                let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
                let mut rw = 0.0;
                for (i, e) in entries.iter().enumerate() {
                    let iw = item_w(e) + 10.0;
                    if rw + iw > right - left && !rows.last().unwrap().is_empty() {
                        rows.push(Vec::new());
                        rw = 0.0;
                    }
                    rows.last_mut().unwrap().push(i);
                    rw += iw;
                }
                let total_h = rows.len() as f64 * row_h;
                let y0 = if pos == "b" { bottom - total_h } else { top };
                for (ri, row) in rows.iter().enumerate() {
                    let row_w: f64 = row.iter().map(|i| item_w(&entries[*i]) + 10.0).sum::<f64>() - 10.0;
                    let mut x = (w - row_w) / 2.0;
                    for i in row {
                        draw_entry(&mut o, x, y0 + ri as f64 * row_h + row_h / 2.0, &entries[*i]);
                        x += item_w(&entries[*i]) + 10.0;
                    }
                }
                if !overlay {
                    if pos == "b" {
                        bottom -= total_h + 4.0;
                    } else {
                        top += total_h + 4.0;
                    }
                }
            }
            _ => {
                let lw_: f64 = entries.iter().map(|e| item_w(e)).fold(0.0, f64::max) + 6.0;
                let total_h = entries.len() as f64 * row_h;
                let x0 = if pos == "l" { left } else { right - lw_ };
                let y0 = if pos == "tr" { top } else { top + ((bottom - top) - total_h).max(0.0) / 2.0 };
                for (i, e) in entries.iter().enumerate() {
                    draw_entry(&mut o, x0, y0 + i as f64 * row_h + row_h / 2.0, e);
                }
                if !overlay {
                    if pos == "l" {
                        left += lw_ + 6.0;
                    } else {
                        right -= lw_ + 6.0;
                    }
                }
            }
        }
    }

    // The plot area: a manual layout, or what is left.
    let manual = child(plot, "layout").and_then(|l| child(l, "manualLayout"));
    let manual_inner = manual.filter(|m| val_of(m, "layoutTarget").as_deref() == Some("inner")).and_then(|m| Some((numv(m, "x")?, numv(m, "y")?, numv(m, "w")?, numv(m, "h")?)));
    if pie_like {
        let (px, py, pw, ph) = match manual.and_then(|m| Some((numv(m, "x")?, numv(m, "y")?, numv(m, "w")?, numv(m, "h")?))) {
            Some((x, y, mw, mh)) => (x * w, y * h, mw * w, mh * h),
            None => (left, top, right - left, bottom - top),
        };
        for g in &groups {
            draw_pie(&mut o, g, theme, px, py, pw, ph, &small);
        }
        return Ok(format!("box(width: {}, height: {}, clip: true, {{\n{}}})", pt(w), pt(h), o.s));
    }

    if groups.iter().any(|g| g.kind == Kind::Radar) {
        let (px, py, pw, ph) = match manual_inner {
            Some((x, y, mw, mh)) => (x * w, y * h, mw * w, mh * h),
            None => (left, top, right - left, bottom - top),
        };
        let va = axes.iter().find(|a| a.is_val);
        let cst = axes.iter().find(|a| !a.is_val).map(|a| a.text.clone()).unwrap_or_else(|| small.clone());
        draw_radar(&mut o, &groups, va, &cst, px, py, pw, ph, grid_color);
        return Ok(format!("box(width: {}, height: {}, clip: true, {{\n{}}})", pt(w), pt(h), o.s));
    }

    // Axes: which value axis each group uses, and the category (or x) axis.
    let horizontal = groups.iter().any(|g| g.kind == Kind::Bar && g.horizontal);
    let val_axes: Vec<&Axis> = axes.iter().filter(|a| a.is_val).collect();
    let scatter = groups.iter().any(|g| g.kind == Kind::Scatter);
    let primary_val = groups.first().and_then(|g| g.axes.iter().find_map(|id| val_axes.iter().find(|a| &a.id == id && !(scatter && matches!(a.pos.as_str(), "b" | "t"))))).copied();
    let cat_axis = groups.first().and_then(|g| g.axes.iter().find_map(|id| axes.iter().find(|a| &a.id == id && Some(a.id.as_str()) != primary_val.map(|p| p.id.as_str()))));
    let ncat = groups.iter().flat_map(|g| g.series.iter()).map(|s| s.vals.len().max(s.cats.len())).max().unwrap_or(0).max(1);
    let cats: Vec<String> = groups.iter().flat_map(|g| g.series.iter()).find(|s| !s.cats.is_empty()).map(|s| s.cats.clone()).unwrap_or_else(|| (1..=ncat).map(|i| i.to_string()).collect());

    // Data ranges per value axis.
    let mut ranges: HashMap<String, (f64, f64, bool)> = HashMap::new();
    for g in &groups {
        let vid = g.axes.iter().find(|id| val_axes.iter().any(|a| &&a.id == id && !(scatter && g.kind == Kind::Scatter && matches!(a.pos.as_str(), "b" | "t")))).cloned().unwrap_or_default();
        let e = ranges.entry(vid).or_insert((f64::INFINITY, f64::NEG_INFINITY, false));
        if g.grouping == Grouping::Percent && matches!(g.kind, Kind::Bar | Kind::Line | Kind::Area) {
            e.0 = e.0.min(0.0);
            e.1 = e.1.max(1.0);
            e.2 = true;
            continue;
        }
        if g.grouping == Grouping::Stacked && matches!(g.kind, Kind::Bar | Kind::Line | Kind::Area) {
            for k in 0..ncat {
                let (mut pos, mut neg) = (0.0, 0.0);
                for s in &g.series {
                    let v = s.vals.get(k).copied().flatten().unwrap_or(0.0);
                    if v >= 0.0 {
                        pos += v
                    } else {
                        neg += v
                    }
                }
                e.0 = e.0.min(neg);
                e.1 = e.1.max(pos);
            }
        } else {
            for s in &g.series {
                for v in s.vals.iter().flatten() {
                    e.0 = e.0.min(*v);
                    e.1 = e.1.max(*v);
                }
            }
        }
        if matches!(g.kind, Kind::Bar | Kind::Area) {
            e.0 = e.0.min(0.0);
            e.1 = e.1.max(0.0);
        }
    }
    let len_along_val = if horizontal { right - left } else { bottom - top };
    // As many intervals as fit with labels not crowded: 1.6 lines apart, or a label's width.
    let label_st = primary_val.map(|a| a.text.size).unwrap_or(small.size);
    let ticks = (len_along_val / if horizontal { label_st * 3.5 + 8.0 } else { label_st * 1.6 }).clamp(2.0, 12.0);
    let scale_of = |a: Option<&Axis>| -> (f64, f64, f64, bool) {
        let id = a.map(|a| a.id.clone()).unwrap_or_default();
        let (mn, mx, pct) = ranges.get(&id).copied().or_else(|| ranges.values().next().copied()).unwrap_or((0.0, 1.0, false));
        let (mn, mx) = if mn.is_finite() { (mn, mx) } else { (0.0, 1.0) };
        let (lo, hi, st) = nice_scale(mn, mx, ticks, (a.and_then(|a| a.min), a.and_then(|a| a.max), a.and_then(|a| a.major).or(if pct { Some(0.1) } else { None })), true);
        (lo, hi, st, pct)
    };
    let (vlo, vhi, vstep, vpct) = scale_of(primary_val);
    let secondary = val_axes.iter().find(|a| Some(a.id.as_str()) != primary_val.map(|p| p.id.as_str()) && !(scatter && matches!(a.pos.as_str(), "b" | "t")) && groups.iter().any(|g| g.axes.contains(&a.id))).copied();
    let sec_scale = secondary.map(|a| scale_of(Some(a)));
    // The x scale of scatter charts.
    let xscale = if scatter {
        let xa = val_axes.iter().find(|a| matches!(a.pos.as_str(), "b" | "t")).copied();
        let (mut mn, mut mx) = (f64::INFINITY, f64::NEG_INFINITY);
        for g in &groups {
            for s in &g.series {
                for (k, _) in s.vals.iter().enumerate() {
                    let x = s.xs.get(k).copied().flatten().unwrap_or((k + 1) as f64);
                    mn = mn.min(x);
                    mx = mx.max(x);
                }
            }
        }
        if !mn.is_finite() {
            mn = 0.0;
            mx = 1.0;
        }
        Some((nice_scale(mn, mx, ((right - left) / (label_st * 3.5 + 8.0)).clamp(2.0, 12.0), (xa.and_then(|a| a.min), xa.and_then(|a| a.max), xa.and_then(|a| a.major)), true), xa.cloned()))
    } else {
        None
    };

    let fmt_val = |v: f64, a: Option<&Axis>, pct: bool, series_fmt: &str| -> String {
        let f = a.and_then(|a| a.fmt.clone()).unwrap_or_else(|| if pct { "0%".into() } else { series_fmt.to_string() });
        format_value(v, &f)
    };
    let series_fmt = groups.iter().flat_map(|g| g.series.iter()).next().map(|s| s.fmt.clone()).unwrap_or_else(|| "General".into());
    let val_labels: Vec<(f64, String)> = {
        let mut v = Vec::new();
        let mut x = vlo;
        while x <= vhi + vstep * 1e-9 {
            v.push((x, fmt_val(if x.abs() < vstep * 1e-9 { 0.0 } else { x }, primary_val, vpct, &series_fmt)));
            x += vstep;
        }
        v
    };
    let sec_labels: Vec<(f64, String)> = match (secondary, sec_scale) {
        (Some(a), Some((lo, hi, st, pct))) => {
            let mut v = Vec::new();
            let mut x = lo;
            while x <= hi + st * 1e-9 {
                v.push((x, fmt_val(if x.abs() < st * 1e-9 { 0.0 } else { x }, Some(a), pct, &series_fmt)));
                x += st;
            }
            v
        }
        _ => Vec::new(),
    };

    // Room for the axes' labels and titles.
    let vst = primary_val.map(|a| a.text.clone()).unwrap_or_else(|| small.clone());
    let cst = cat_axis.map(|a| a.text.clone()).unwrap_or_else(|| small.clone());
    let val_shown = primary_val.map(|a| !a.deleted).unwrap_or(false);
    let cat_shown = cat_axis.map(|a| !a.deleted).unwrap_or(false) || scatter;
    let x_labels: Vec<String> = if let Some(((lo, hi, st), xa)) = &xscale {
        let mut v = Vec::new();
        let mut x = *lo;
        while x <= *hi + st * 1e-9 {
            v.push(format_value(if x.abs() < st * 1e-9 { 0.0 } else { x }, xa.as_ref().and_then(|a| a.fmt.as_deref()).unwrap_or("General")));
            x += st;
        }
        v
    } else {
        Vec::new()
    };
    if let Some((x, y, mw, mh)) = manual_inner {
        left = x * w;
        top = y * h;
        right = left + mw * w;
        bottom = top + mh * h;
    } else {
        for a in [primary_val, cat_axis, secondary].into_iter().flatten() {
            if let Some(t) = &a.title {
                let st = TextStyle { size: a.text.size, color: a.text.color.clone(), bold: true };
                let is_vertical_axis = matches!(a.pos.as_str(), "l" | "r");
                if is_vertical_axis {
                    if a.pos == "l" {
                        o.vtext(left + st.size * 0.7, (top + bottom) / 2.0, t, &st);
                        left += st.size * 1.6;
                    } else {
                        o.vtext(right - st.size * 0.7, (top + bottom) / 2.0, t, &st);
                        right -= st.size * 1.6;
                    }
                } else {
                    o.text((left + right) / 2.0, bottom, 'c', 'b', t, &st);
                    bottom -= st.size * 1.5;
                }
            }
        }
        let val_w = |labels: &[(f64, String)], st: &TextStyle| labels.iter().map(|(_, l)| est_width(l, st.size)).fold(0.0, f64::max) + 6.0;
        let cat_labels_w = cats.iter().map(|l| est_width(l, cst.size)).fold(0.0, f64::max) + 6.0;
        if horizontal {
            if cat_shown {
                left += cat_labels_w.min(w * 0.35);
            }
            if val_shown {
                bottom -= vst.size * 1.4 + 2.0;
            }
        } else {
            if val_shown {
                left += val_w(&val_labels, &vst);
            }
            if secondary.map(|a| !a.deleted).unwrap_or(false) {
                right -= val_w(&sec_labels, &secondary.unwrap().text);
            }
            if cat_shown {
                // Labels that do not fit side by side are turned.
                let band = (right - left) / ncat as f64;
                let rows = if scatter { 1.0 } else { cats.iter().map(|l| ((est_width(l, cst.size) + 4.0) / band.max(1.0)).ceil()).fold(1.0, f64::max).min(3.0) };
                bottom -= cst.size * 1.3 * rows + 3.0;
            }
        }
        left += 2.0;
        right -= 4.0;
        top += 4.0;
    }
    let (pw, ph) = ((right - left).max(10.0), (bottom - top).max(10.0));
    // Plot area fill and border.
    let (pfill, pline, plw) = c.sppr(Some(plot));
    if pfill.clone().flatten().is_some() || pline.clone().flatten().is_some() {
        o.rect(left, top, pw, ph, pfill.flatten().as_deref(), pline.flatten().as_deref().map(|l| (l, plw.unwrap_or(0.75))));
    }

    // Value → position along the value axis.
    let vpos = |v: f64, (lo, hi): (f64, f64), rev: bool| -> f64 {
        let t = ((v - lo) / (hi - lo)).clamp(-0.05, 1.05);
        let t = if rev { 1.0 - t } else { t };
        if horizontal {
            left + t * pw
        } else {
            bottom - t * ph
        }
    };
    let prev = primary_val.map(|a| a.reverse).unwrap_or(false);

    // Gridlines and value labels.
    if primary_val.map(|a| a.grid).unwrap_or(false) {
        for (v, _) in &val_labels {
            let p = vpos(*v, (vlo, vhi), prev);
            if horizontal {
                o.line(&[(p, top), (p, bottom)], grid_color, 0.75, false);
            } else {
                o.line(&[(left, p), (right, p)], grid_color, 0.75, false);
            }
        }
    }
    if let Some(((xlo, xhi, xst), xa)) = &xscale {
        if xa.as_ref().map(|a| a.grid).unwrap_or(false) {
            let mut x = *xlo;
            while x <= *xhi + xst * 1e-9 {
                let px = left + (x - xlo) / (xhi - xlo) * pw;
                o.line(&[(px, top), (px, bottom)], grid_color, 0.75, false);
                x += xst;
            }
        }
    }
    if val_shown {
        for (v, l) in &val_labels {
            let p = vpos(*v, (vlo, vhi), prev);
            if horizontal {
                o.text(p, bottom + 3.0, 'c', 't', l, &vst);
            } else {
                o.text(left - 4.0, p, 'r', 'm', l, &vst);
            }
        }
    }
    if let (Some(a), Some((lo, hi, _, _))) = (secondary, sec_scale) {
        if !a.deleted {
            for (v, l) in &sec_labels {
                let p = bottom - (v - lo) / (hi - lo) * ph;
                o.text(right + 4.0, p, 'l', 'm', l, &a.text);
            }
        }
    }

    // Category positions.
    let band = if horizontal { ph / ncat as f64 } else { pw / ncat as f64 };
    let cat_rev = cat_axis.map(|a| a.reverse).unwrap_or(false);
    let between = groups.iter().any(|g| g.kind == Kind::Bar) || primary_val.map(|a| a.between).unwrap_or(true);
    let cat_center = |k: usize| -> f64 {
        let k = if (horizontal && !cat_rev) || (!horizontal && cat_rev) { ncat - 1 - k } else { k };
        let t = if between || ncat == 1 { band * (k as f64 + 0.5) } else { (if horizontal { ph } else { pw }) * k as f64 / (ncat - 1) as f64 };
        if horizontal {
            top + t
        } else {
            left + t
        }
    };

    // The series.
    for g in &groups {
        let use_sec = secondary.map(|a| g.axes.contains(&a.id)).unwrap_or(false);
        let (lo, hi, rev) = if use_sec { sec_scale.map(|(l, h, _, _)| (l, h, secondary.unwrap().reverse)).unwrap() } else { (vlo, vhi, prev) };
        let vp = |v: f64| -> f64 {
            if use_sec && !horizontal {
                bottom - ((v - lo) / (hi - lo)).clamp(-0.05, 1.05) * ph
            } else {
                vpos(v, (lo, hi), rev)
            }
        };
        let totals: Vec<f64> = (0..ncat).map(|k| g.series.iter().map(|s| s.vals.get(k).copied().flatten().unwrap_or(0.0).abs()).sum::<f64>()).collect();
        let value = |s: &Series, k: usize| -> Option<f64> {
            let v = s.vals.get(k).copied().flatten()?;
            Some(if g.grouping == Grouping::Percent { if totals[k] > 0.0 { v / totals[k] } else { 0.0 } } else { v })
        };
        match g.kind {
            Kind::Bar => {
                let n = g.series.len().max(1) as f64;
                let stacked = g.grouping != Grouping::Clustered;
                let gap = g.gap / 100.0;
                let ov = (g.overlap / 100.0).clamp(-1.0, 1.0);
                let bw = if stacked { band / (1.0 + gap) } else { band / (n - (n - 1.0) * ov + gap) };
                let mut pos_acc = vec![0.0; ncat];
                let mut neg_acc = vec![0.0; ncat];
                for (si, s) in g.series.iter().enumerate() {
                    let col = series_color(s, Kind::Bar);
                    let border = s.line.clone().flatten();
                    for k in 0..ncat {
                        let Some(v) = value(s, k) else { continue };
                        let (base, end) = if stacked {
                            let acc = if v >= 0.0 { &mut pos_acc[k] } else { &mut neg_acc[k] };
                            let b = *acc;
                            *acc += v;
                            (b, *acc)
                        } else {
                            (0f64.clamp(lo, hi), v)
                        };
                        let c0 = cat_center(k) - band / 2.0;
                        let off = if stacked { (band - bw) / 2.0 } else { (band - bw * (n - (n - 1.0) * ov)) / 2.0 + si as f64 * bw * (1.0 - ov) };
                        let off = if horizontal && !stacked { band - off - bw } else { off };
                        let fillc = s.points.get(&k).cloned().or_else(|| if g.vary && g.series.len() == 1 { Some(default_color(theme, k)) } else { None }).unwrap_or_else(|| col.clone());
                        let (a, b) = (vp(base), vp(end));
                        if horizontal {
                            o.rect(a, c0 + off, b - a, bw, Some(&fillc), border.as_deref().map(|l| (l, s.line_w.unwrap_or(0.75))));
                        } else {
                            o.rect(c0 + off, b, bw, a - b, Some(&fillc), border.as_deref().map(|l| (l, s.line_w.unwrap_or(0.75))));
                        }
                        let lb = if s.labels.any() { s.labels } else { g.labels };
                        if lb.any() {
                            let t = label_text(&lb, s, k, v, &cats, if g.grouping == Grouping::Percent { "0%" } else { &s.fmt });
                            if horizontal {
                                if stacked {
                                    o.text((a + b) / 2.0, c0 + off + bw / 2.0, 'c', 'm', &t, &small);
                                } else {
                                    o.text(b + 3.0, c0 + off + bw / 2.0, 'l', 'm', &t, &small);
                                }
                            } else if stacked {
                                o.text(c0 + off + bw / 2.0, (a + b) / 2.0, 'c', 'm', &t, &small);
                            } else {
                                o.text(c0 + off + bw / 2.0, b - 2.0, 'c', 'b', &t, &small);
                            }
                        }
                    }
                }
            }
            Kind::Line | Kind::Area => {
                let stacked = g.grouping != Grouping::Clustered;
                let mut acc = vec![0.0; ncat];
                let mut prev_line: Vec<(f64, f64)> = (0..ncat).map(|k| (cat_center(k), vp(0f64.clamp(lo, hi)))).collect();
                for s in &g.series {
                    let col = series_color(s, g.kind);
                    let mut pts: Vec<Option<(f64, f64)>> = Vec::new();
                    for k in 0..ncat {
                        match value(s, k) {
                            Some(v) => {
                                let v = if stacked {
                                    acc[k] += v;
                                    acc[k]
                                } else {
                                    v
                                };
                                pts.push(Some((cat_center(k), vp(v))));
                            }
                            None => pts.push(if stacked { Some((cat_center(k), vp(acc[k]))) } else { None }),
                        }
                    }
                    if g.kind == Kind::Area {
                        let upper: Vec<(f64, f64)> = pts.iter().enumerate().map(|(k, p)| p.unwrap_or(prev_line[k])).collect();
                        let mut poly = upper.clone();
                        poly.extend(prev_line.iter().rev().copied());
                        let fillc = s.fill.clone().unwrap_or(Some(col.clone()));
                        o.polygon(&poly, fillc.as_deref(), s.line.clone().flatten().as_deref().map(|l| (l, s.line_w.unwrap_or(0.75))));
                        if stacked {
                            prev_line = upper;
                        }
                    } else {
                        let lc = s.line.clone().unwrap_or(Some(col.clone()));
                        if let Some(lc) = &lc {
                            // Runs between gaps.
                            let mut run: Vec<(f64, f64)> = Vec::new();
                            for p in &pts {
                                match p {
                                    Some(p) => run.push(*p),
                                    None => {
                                        o.line(&run, lc, s.line_w.unwrap_or(2.25), false);
                                        run.clear();
                                    }
                                }
                            }
                            o.line(&run, lc, s.line_w.unwrap_or(2.25), false);
                        }
                        draw_markers(&mut o, g, s, &pts, &col);
                    }
                    let lb = if s.labels.any() { s.labels } else { g.labels };
                    if lb.any() {
                        for (k, p) in pts.iter().enumerate() {
                            if let (Some((x, y)), Some(v)) = (p, value(s, k)) {
                                o.text(*x, y - 4.0, 'c', 'b', &label_text(&lb, s, k, v, &cats, &s.fmt), &small);
                            }
                        }
                    }
                }
            }
            Kind::Scatter => {
                let Some(((xlo, xhi, _), _)) = &xscale else { continue };
                for s in &g.series {
                    let col = series_color(s, Kind::Scatter);
                    let pts: Vec<Option<(f64, f64)>> = (0..s.vals.len())
                        .map(|k| {
                            let v = value(s, k)?;
                            let x = s.xs.get(k).copied().flatten().unwrap_or((k + 1) as f64);
                            Some((left + (x - xlo) / (xhi - xlo) * pw, vp(v)))
                        })
                        .collect();
                    if let Some(lc) = s.line.clone().unwrap_or(Some(col.clone())) {
                        let run: Vec<(f64, f64)> = pts.iter().flatten().copied().collect();
                        o.line(&run, &lc, s.line_w.unwrap_or(2.25), false);
                    }
                    draw_markers(&mut o, g, s, &pts, &col);
                }
            }
            Kind::Radar => {}
            Kind::Pie | Kind::Doughnut => {}
        }
    }

    // Axis lines and category labels (over the bars, as Office draws them).
    if groups.iter().all(|g| g.kind != Kind::Radar) {
        if let Some(a) = cat_axis.filter(|a| !a.deleted) {
            if let Some(lc) = &a.line {
                let zero = vpos(0f64.clamp(vlo, vhi), (vlo, vhi), prev);
                if horizontal {
                    o.line(&[(zero, top), (zero, bottom)], lc, 0.75, false);
                } else {
                    o.line(&[(left, zero), (right, zero)], lc, 0.75, false);
                }
            }
            if !scatter {
                for (k, l) in cats.iter().enumerate().take(ncat) {
                    let p = cat_center(k);
                    if horizontal {
                        o.text(left - 4.0, p, 'r', 'm', l, &cst);
                    } else {
                        // Wrapped into the band when it does not fit.
                        let fits = est_width(l, cst.size) + 4.0 <= band;
                        if fits {
                            o.text(p, bottom + 3.0, 'c', 't', l, &cst);
                        } else {
                            let bw = band.max(cst.size * 3.0);
                            let _ = writeln!(
                                o.s,
                                "place(top + left, dx: {}, dy: {}, box(width: {}, align(center, text(font: {}, size: {}, fill: {}, {}))))",
                                pt(p - bw / 2.0),
                                pt(bottom + 3.0),
                                pt(bw),
                                o.font,
                                pt(cst.size),
                                rgb(&cst.color),
                                s(l)
                            );
                        }
                    }
                }
            }
        }
        if let Some(((xlo, xhi, xst), xa)) = &xscale {
            if xa.as_ref().map(|a| !a.deleted).unwrap_or(true) {
                let mut x = *xlo;
                let mut i = 0;
                while x <= *xhi + xst * 1e-9 {
                    let px = left + (x - xlo) / (xhi - xlo) * pw;
                    o.text(px, bottom + 3.0, 'c', 't', x_labels.get(i).map(|s| s.as_str()).unwrap_or(""), &cst);
                    x += xst;
                    i += 1;
                }
                o.line(&[(left, bottom), (right, bottom)], "BFBFBF", 0.75, false);
            }
        }
    }
    Ok(format!("box(width: {}, height: {}, clip: true, {{\n{}}})", pt(w), pt(h), o.s))
}

fn series_color(s: &Series, kind: Kind) -> String {
    match kind {
        Kind::Line | Kind::Scatter | Kind::Radar => s.line.clone().flatten().or_else(|| s.fill.clone().flatten()).unwrap_or_else(|| s.color.clone()),
        _ => s.fill.clone().flatten().unwrap_or_else(|| s.color.clone()),
    }
}

fn draw_markers(o: &mut Out, g: &Group, s: &Series, pts: &[Option<(f64, f64)>], col: &str) {
    let (sym, size, mfill) = match &s.marker {
        Some(m) => m.clone(),
        None if g.markers && g.kind != Kind::Area => ("circle".into(), 5.0, None),
        None => return,
    };
    if sym == "none" {
        return;
    }
    let fill = mfill.unwrap_or_else(|| col.to_string());
    for p in pts.iter().flatten() {
        o.marker(p.0, p.1, &sym, size, &fill, col);
    }
}

fn label_text(lb: &Labels, s: &Series, k: usize, v: f64, cats: &[String], fmt: &str) -> String {
    let mut parts = Vec::new();
    if lb.ser {
        parts.push(s.name.clone());
    }
    if lb.cat {
        parts.push(s.cats.get(k).or(cats.get(k)).cloned().unwrap_or_default());
    }
    if lb.val || lb.pct {
        parts.push(format_value(v, fmt));
    }
    parts.join(", ")
}

#[allow(clippy::too_many_arguments)]
fn draw_pie(o: &mut Out, g: &Group, theme: &Theme, x: f64, y: f64, w: f64, h: f64, st: &TextStyle) {
    let Some(s) = g.series.first() else { return };
    let vals: Vec<f64> = s.vals.iter().map(|v| v.unwrap_or(0.0).max(0.0)).collect();
    let total: f64 = vals.iter().sum();
    if total <= 0.0 {
        return;
    }
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let most = s.point_explosion.values().copied().fold(s.explosion, f64::max);
    let r = w.min(h) / 2.0 * 0.92 / (1.0 + most / 100.0);
    let inner = if g.kind == Kind::Doughnut { r * g.hole / 100.0 } else { 0.0 };
    let mut a = (g.first_angle - 90.0).to_radians();
    let border = s.line.clone().unwrap_or(Some("FFFFFF".into()));
    for (k, v) in vals.iter().enumerate() {
        let sweep = v / total * std::f64::consts::TAU;
        if sweep <= 0.0 {
            continue;
        }
        let steps = ((sweep / 0.05).ceil() as usize).max(2);
        let ex = s.point_explosion.get(&k).copied().unwrap_or(s.explosion) / 100.0 * r;
        let (cx, cy) = (cx + ex * (a + sweep / 2.0).cos(), cy + ex * (a + sweep / 2.0).sin());
        let mut pts = Vec::new();
        if inner <= 0.0 {
            pts.push((cx, cy));
        }
        for i in 0..=steps {
            let t = a + sweep * i as f64 / steps as f64;
            pts.push((cx + r * t.cos(), cy + r * t.sin()));
        }
        if inner > 0.0 {
            for i in (0..=steps).rev() {
                let t = a + sweep * i as f64 / steps as f64;
                pts.push((cx + inner * t.cos(), cy + inner * t.sin()));
            }
        }
        let col = s.points.get(&k).cloned().unwrap_or_else(|| if g.vary || g.series.len() == 1 { default_color(theme, k) } else { s.color.clone() });
        o.polygon(&pts, Some(&col), border.as_deref().map(|b| (b, s.line_w.unwrap_or(0.75))));
        let lb = if s.labels.any() { s.labels } else { g.labels };
        if lb.any() {
            let mid = a + sweep / 2.0;
            let rr = if inner > 0.0 { (r + inner) / 2.0 } else { r * 0.65 };
            let mut parts = Vec::new();
            if lb.cat {
                parts.push(s.cats.get(k).cloned().unwrap_or_default());
            }
            if lb.val {
                parts.push(format_value(*v, &s.fmt));
            }
            if lb.pct {
                parts.push(format_value(v / total, "0%"));
            }
            let lum = {
                let hx = u32::from_str_radix(&col, 16).unwrap_or(0);
                let (r_, g_, b_) = ((hx >> 16) & 255, (hx >> 8) & 255, hx & 255);
                0.299 * r_ as f64 + 0.587 * g_ as f64 + 0.114 * b_ as f64
            };
            let tst = TextStyle { color: if lum < 140.0 { "FFFFFF".into() } else { "404040".into() }, ..st.clone() };
            o.text(cx + rr * mid.cos(), cy + rr * mid.sin(), 'c', 'm', &parts.join("\n"), &tst);
        }
        a += sweep;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_radar(o: &mut Out, groups: &[Group], va: Option<&Axis>, cst: &TextStyle, x: f64, y: f64, w: f64, h: f64, grid: &str) {
    let ncat = groups.iter().flat_map(|g| g.series.iter()).map(|s| s.vals.len().max(s.cats.len())).max().unwrap_or(0).max(3);
    let cats: Vec<String> = groups.iter().flat_map(|g| g.series.iter()).find(|s| !s.cats.is_empty()).map(|s| s.cats.clone()).unwrap_or_else(|| (1..=ncat).map(|i| i.to_string()).collect());
    let label_w = cats.iter().map(|c| est_width(c, cst.size)).fold(0.0, f64::max);
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let r = ((w / 2.0 - label_w - 6.0).min(h / 2.0 - cst.size * 1.6)).max(10.0);
    let (mut mn, mut mx) = (f64::INFINITY, f64::NEG_INFINITY);
    for s in groups.iter().flat_map(|g| g.series.iter()) {
        for v in s.vals.iter().flatten() {
            mn = mn.min(*v);
            mx = mx.max(*v);
        }
    }
    if !mn.is_finite() {
        mn = 0.0;
        mx = 1.0;
    }
    let (lo, hi, step) = nice_scale(mn, mx, (r / (cst.size * 2.8)).clamp(2.0, 8.0), (va.and_then(|a| a.min), va.and_then(|a| a.max), va.and_then(|a| a.major)), true);
    let ang = |k: usize| -std::f64::consts::FRAC_PI_2 + (k % ncat) as f64 * std::f64::consts::TAU / ncat as f64;
    let at = |k: usize, rr: f64| (cx + rr * ang(k).cos(), cy + rr * ang(k).sin());
    let mut v = lo;
    let vst = va.map(|a| a.text.clone()).unwrap_or_else(|| cst.clone());
    while v <= hi + step * 1e-9 {
        let rr = (v - lo) / (hi - lo) * r;
        let ring: Vec<(f64, f64)> = (0..=ncat).map(|k| at(k, rr)).collect();
        o.line(&ring, grid, 0.75, false);
        if va.map(|a| !a.deleted).unwrap_or(true) {
            let t = format_value(if v.abs() < step * 1e-9 { 0.0 } else { v }, va.and_then(|a| a.fmt.as_deref()).unwrap_or("General"));
            o.text(cx + 3.0, cy - rr, 'l', 'm', &t, &vst);
        }
        v += step;
    }
    for k in 0..ncat {
        o.line(&[(cx, cy), at(k, r)], grid, 0.75, false);
        let (lx, ly) = at(k, r + 6.0);
        let c = ang(k).cos();
        let hz = if c > 0.3 { 'l' } else if c < -0.3 { 'r' } else { 'c' };
        let s_ = ang(k).sin();
        let vt = if s_ > 0.3 { 't' } else if s_ < -0.3 { 'b' } else { 'm' };
        o.text(lx, ly, hz, vt, cats.get(k).map(|s| s.as_str()).unwrap_or(""), cst);
    }
    for g in groups.iter().filter(|g| g.kind == Kind::Radar) {
        for s in &g.series {
            let col = series_color(s, Kind::Radar);
            let pts: Vec<(f64, f64)> = (0..=ncat).map(|k| at(k, ((s.vals.get(k % ncat).copied().flatten().unwrap_or(lo) - lo) / (hi - lo)).clamp(0.0, 1.0) * r)).collect();
            if let Some(f) = s.fill.clone().flatten() {
                o.polygon(&pts, Some(&f), None);
            }
            o.line(&pts, &s.line.clone().flatten().unwrap_or(col.clone()), s.line_w.unwrap_or(2.25), false);
            let opts: Vec<Option<(f64, f64)>> = pts[..ncat].iter().map(|p| Some(*p)).collect();
            if !matches!(s.marker.as_ref().map(|m| m.0.as_str()), Some("none") | None) {
                draw_markers(o, g, s, &opts, &col);
            }
        }
    }
}
