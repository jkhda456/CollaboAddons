//! Workbooks as Typst: a page per sheet holding its used range as Excel shows it — column widths
//! and row heights, cell fonts, fills, borders and alignment, values in their number formats,
//! merged cells, text running into empty neighbors — with gridlines where the sheet shows them
//! (and, with headings, the column letters and row numbers), plus its pictures and charts.

use super::office::{self, Theme};
use super::typ::{self, pt, rgb, s};
use super::world::Fonts;
use super::{images, Job};
use crate::formula::{self, Val};
use crate::opc;
use crate::xlsx::{Raw, Xlsx};
use crate::xml::{Element, A, NONE, R, S, XDR};
use crate::Result;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

pub struct Opts {
    pub headings: bool,
}

pub fn convert(x: &Xlsx, fonts: &Fonts) -> Result<Job> {
    convert_with(x, fonts, &Opts { headings: std::env::var("DOCU_HEADINGS").map(|v| v == "1").unwrap_or(false) })
}

#[derive(Clone, Default)]
struct CellStyle {
    font: String,
    size: f64,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    color: Option<String>,
    fill: Option<String>,
    borders: [Option<(f64, String, &'static str)>; 4], // top right bottom left
    halign: Option<String>,
    valign: Option<String>,
    wrap: bool,
    indent: u32,
    rotation: i32,
}

/// Excel's indexed colors (the legacy palette).
const INDEXED: [&str; 64] = [
    "000000", "FFFFFF", "FF0000", "00FF00", "0000FF", "FFFF00", "FF00FF", "00FFFF", "000000", "FFFFFF", "FF0000", "00FF00", "0000FF", "FFFF00", "FF00FF", "00FFFF", "800000", "008000", "000080", "808000", "800080", "008080", "C0C0C0", "808080",
    "9999FF", "993366", "FFFFCC", "CCFFFF", "660066", "FF8080", "0066CC", "CCCCFF", "000080", "FF00FF", "FFFF00", "00FFFF", "800080", "800000", "008080", "0000FF", "00CCFF", "CCFFFF", "CCFFCC", "FFFF99", "99CCFF", "FF99CC", "CC99FF", "FFCC99",
    "3366FF", "33CCCC", "99CC00", "FFCC00", "FF9900", "FF6600", "666699", "969696", "003366", "339966", "003300", "333300", "993300", "993366", "333399", "333333",
];

fn sml_color(e: &Element, theme: &Theme) -> Option<String> {
    let base = if let Some(rgb) = e.attr_ns(NONE, "rgb") {
        Some(if rgb.len() == 8 { rgb[2..].to_ascii_uppercase() } else { rgb.to_ascii_uppercase() })
    } else if let Some(t) = e.attr_ns(NONE, "theme").and_then(|t| t.parse::<usize>().ok()) {
        // SpreadsheetML theme indexes: 0 lt1, 1 dk1, 2 lt2, 3 dk2, 4.. accents.
        let name = ["lt1", "dk1", "lt2", "dk2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"].get(t).copied().unwrap_or("dk1");
        theme.color(name)
    } else if let Some(i) = e.attr_ns(NONE, "indexed").and_then(|t| t.parse::<usize>().ok()) {
        if i == 64 {
            None
        } else {
            INDEXED.get(i).map(|s| s.to_string())
        }
    } else {
        None
    }?;
    let tint: f64 = e.attr_ns(NONE, "tint").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    if tint == 0.0 {
        return Some(base);
    }
    let (r, g, b) = office::hex_rgb(&base);
    let f = |c: f64| if tint < 0.0 { c * (1.0 + tint) } else { c + (1.0 - c) * tint };
    Some(office::rgb_hex(f(r), f(g), f(b)))
}

struct Conv<'a> {
    x: &'a Xlsx,
    fonts: &'a Fonts,
    theme: Theme,
    styles: Vec<CellStyle>,
    files: Vec<(String, Vec<u8>)>,
    warnings: Vec<String>,
    max_digit: f64,
}

pub fn convert_with(x: &Xlsx, fonts: &Fonts, opts: &Opts) -> Result<Job> {
    let theme = Theme::load(&x.pkg, x.wb_rels.first_part("theme"));
    let mut c = Conv { x, fonts, theme, styles: Vec::new(), files: Vec::new(), warnings: Vec::new(), max_digit: 7.0 };
    c.load_styles();
    let mut out = String::new();
    out.push_str("#set text(hyphenate: false, kerning: true, ligatures: false, top-edge: \"ascender\", bottom-edge: \"descender\")\n#set par(spacing: 0pt, leading: 0.2em)\n#set block(spacing: 0pt)\n");
    let mut first = true;
    for si in 0..x.sheets.len() {
        if x.sheets[si].hidden {
            continue;
        }
        // A chart sheet: its chart on a page of its own.
        let page = if x.sheets[si].doc.is_none() {
            match c.chart_sheet(si) {
                Some(p) => p,
                None => continue,
            }
        } else {
            c.sheet(si, opts)
        };
        if page.is_empty() {
            continue;
        }
        if !first {
            out.push_str("#pagebreak()\n");
        }
        first = false;
        out.push_str(&page);
    }
    if first {
        out.push_str("#set page(width: 200pt, height: 60pt, margin: 10pt)\n#text(size: 9pt, fill: gray, \"(the workbook is empty)\")\n");
    }
    Ok(Job { source: out, files: c.files, warnings: c.warnings })
}

impl<'a> Conv<'a> {
    fn load_styles(&mut self) {
        let Some(st) = &self.x.styles else {
            self.styles.push(CellStyle { font: "Calibri".into(), size: 11.0, ..Default::default() });
            return;
        };
        let list = |name: &str| -> Vec<Element> { st.root.child(S, name).map(|l| l.elements().cloned().collect()).unwrap_or_default() };
        let fonts = list("fonts");
        let fills = list("fills");
        let borders = list("borders");
        let xfs = list("cellXfs");
        let theme = self.theme.clone();
        let font_of = |f: &Element| -> (String, f64, bool, bool, bool, bool, Option<String>) {
            let name = f.child(S, "name").and_then(|n| n.attr_ns(NONE, "val")).unwrap_or("Calibri").to_string();
            let name = match f.child(S, "scheme").and_then(|n| n.attr_ns(NONE, "val")) {
                Some("major") => theme.major_latin.clone(),
                Some("minor") => theme.minor_latin.clone(),
                _ => name,
            };
            let size = f.child(S, "sz").and_then(|n| n.attr_ns(NONE, "val")).and_then(|v| v.parse().ok()).unwrap_or(11.0);
            let flag = |k: &str| f.child(S, k).map(|e| e.attr_ns(NONE, "val").map(|v| v != "0" && v != "false").unwrap_or(true)).unwrap_or(false);
            let color = f.child(S, "color").and_then(|c| sml_color(c, &theme));
            (name, size, flag("b"), flag("i"), f.child(S, "u").is_some(), flag("strike"), color)
        };
        if let Some(f0) = fonts.first() {
            // The default font's digit width sets column widths.
            let (name, size, ..) = font_of(f0);
            self.max_digit = digit_width(self.fonts, &name, size);
        }
        for xf in &xfs {
            let g = |a: &str| xf.attr_ns(NONE, a).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
            let mut cs = CellStyle::default();
            if let Some(f) = fonts.get(g("fontId")) {
                let (name, size, b, i, u, st_, color) = font_of(f);
                cs.font = name;
                cs.size = size;
                cs.bold = b;
                cs.italic = i;
                cs.underline = u;
                cs.strike = st_;
                cs.color = color;
            } else {
                cs.font = "Calibri".into();
                cs.size = 11.0;
            }
            if let Some(f) = fills.get(g("fillId")) {
                if let Some(pf) = f.child(S, "patternFill") {
                    match pf.attr_ns(NONE, "patternType") {
                        Some("none") | None => {}
                        Some("solid") => cs.fill = pf.child(S, "fgColor").and_then(|c| sml_color(c, &theme)),
                        Some(_) => cs.fill = pf.child(S, "fgColor").and_then(|c| sml_color(c, &theme)).or_else(|| pf.child(S, "bgColor").and_then(|c| sml_color(c, &theme))),
                    }
                } else if let Some(gf) = f.child(S, "gradientFill") {
                    cs.fill = gf.child(S, "stop").and_then(|st| st.child(S, "color")).and_then(|c| sml_color(c, &theme));
                }
            }
            if let Some(b) = borders.get(g("borderId")) {
                for (k, side) in ["top", "right", "bottom", "left"].iter().enumerate() {
                    let e = b.child(S, side).or_else(|| match *side {
                        "left" => b.child(S, "start"),
                        "right" => b.child(S, "end"),
                        _ => None,
                    });
                    if let Some(e) = e {
                        if let Some(style) = e.attr_ns(NONE, "style") {
                            let (w, dash) = match style {
                                "thin" => (0.75, ""),
                                "medium" => (1.5, ""),
                                "thick" => (2.25, ""),
                                "double" => (2.25, ""),
                                "hair" => (0.5, "dotted"),
                                "dotted" => (0.75, "dotted"),
                                "dashed" | "mediumDashed" => (if style == "dashed" { 0.75 } else { 1.5 }, "dashed"),
                                "dashDot" | "mediumDashDot" | "dashDotDot" | "mediumDashDotDot" | "slantDashDot" => (0.75, "dash-dotted"),
                                _ => (0.75, ""),
                            };
                            let color = e.child(S, "color").and_then(|c| sml_color(c, &theme)).unwrap_or("000000".into());
                            cs.borders[k] = Some((w, color, dash));
                        }
                    }
                }
            }
            if let Some(al) = xf.child(S, "alignment") {
                cs.halign = al.attr_ns(NONE, "horizontal").map(str::to_string);
                cs.valign = al.attr_ns(NONE, "vertical").map(str::to_string);
                cs.wrap = al.attr_ns(NONE, "wrapText").map(|v| v == "1" || v == "true").unwrap_or(false);
                cs.indent = al.attr_ns(NONE, "indent").and_then(|v| v.parse().ok()).unwrap_or(0);
                cs.rotation = al.attr_ns(NONE, "textRotation").and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            self.styles.push(cs);
        }
        if self.styles.is_empty() {
            self.styles.push(CellStyle { font: "Calibri".into(), size: 11.0, ..Default::default() });
        }
    }

    fn chart_sheet(&mut self, si: usize) -> Option<String> {
        let part = &self.x.sheets[si].part;
        let doc = self.x.pkg.xml(part).ok()?;
        let rels = self.x.pkg.rels(part).ok()?;
        let rid = doc.root.elements().find(|e| e.local() == "drawing").and_then(|d| d.attr_ns(R, "id"))?;
        let dp = rels.part(rid)?;
        let drawing = self.x.pkg.xml(&dp).ok()?;
        let drels = self.x.pkg.rels(&dp).ok()?;
        let chart = drawing.root.descendants().into_iter().find(|e| e.local() == "chart")?;
        let cp = drels.part(chart.attr_ns(R, "id")?)?;
        let (w, h) = (720.0, 540.0);
        let body = super::chart::render_part(&self.x.pkg, &cp, w - 36.0, h - 36.0, &self.theme, self.fonts, &mut self.warnings);
        Some(format!("#set page(width: {}, height: {}, margin: 18pt)\n#{body}\n", pt(w), pt(h)))
    }

    /// Column width in characters → points (Excel: pixels = trunc(chars × digit + 5)).
    fn col_pt(&self, chars: f64) -> f64 {
        ((chars * self.max_digit + 5.0).floor()).max(0.0) * 0.75
    }

    fn sheet(&mut self, si: usize, opts: &Opts) -> String {
        let x = self.x;
        let doc = x.sheets[si].doc.as_ref().unwrap();
        let root = &doc.root;
        let fmt = root.child(S, "sheetFormatPr");
        let default_h = fmt.and_then(|f| f.attr_ns(NONE, "defaultRowHeight")).and_then(|v| v.parse::<f64>().ok()).unwrap_or(15.0);
        let default_w_chars = fmt
            .and_then(|f| f.attr_ns(NONE, "defaultColWidth"))
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or_else(|| {
                let base = fmt.and_then(|f| f.attr_ns(NONE, "baseColWidth")).and_then(|v| v.parse::<f64>().ok()).unwrap_or(8.0);
                // Base width plus margin padding, in characters.
                ((base * self.max_digit + 5.0) / self.max_digit * 256.0).floor() / 256.0
            });
        let show_grid = root.child(S, "sheetViews").and_then(|v| v.child(S, "sheetView")).and_then(|v| v.attr_ns(NONE, "showGridLines")).map(|v| v != "0" && v != "false").unwrap_or(true);
        // Cells and their extent (values, styled cells with fills or borders, merges).
        let cells = x.cells(si);
        let mut grid: HashMap<(u32, u32), (Val, u32)> = HashMap::new();
        let (mut maxr, mut maxc) = (0u32, 0u32);
        let mut any = false;
        for c in &cells {
            let v = match &c.raw {
                Raw::Value(v) | Raw::Formula(_, v) => v.clone(),
            };
            let st = self.styles.get(c.style as usize);
            let visible = !matches!(v, Val::Empty) || st.map(|s| s.fill.is_some() || s.borders.iter().any(|b| b.is_some())).unwrap_or(false);
            if visible {
                maxr = maxr.max(c.row);
                maxc = maxc.max(c.col);
                any = true;
            }
            grid.insert((c.row, c.col), (v, c.style));
        }
        let merges: Vec<formula::Area> = root.child(S, "mergeCells").map(|m| m.children_named(S, "mergeCell").filter_map(|c| c.attr_ns(NONE, "ref").and_then(formula::parse_area)).collect()).unwrap_or_default();
        for m in &merges {
            maxr = maxr.max(m.rows().1);
            maxc = maxc.max(m.cols().1);
            any = true;
        }
        // Drawings extend the range too.
        let drawings = self.drawings(si);
        for (_, (r1, c1)) in drawings.iter().map(|d| (0, d.1)) {
            maxr = maxr.max(r1);
            maxc = maxc.max(c1);
            any = true;
        }
        if !any {
            // Nothing to print, as Excel says: no page.
            return String::new();
        }
        let maxr = maxr.min(5000);
        let maxc = maxc.min(300);
        // Column widths.
        let mut widths: Vec<f64> = vec![self.col_pt(default_w_chars); (maxc + 1) as usize];
        let mut hidden_cols: HashSet<u32> = HashSet::new();
        if let Some(cols) = root.child(S, "cols") {
            for col in cols.children_named(S, "col") {
                let min: u32 = col.attr_ns(NONE, "min").and_then(|v| v.parse().ok()).unwrap_or(1);
                let max: u32 = col.attr_ns(NONE, "max").and_then(|v| v.parse().ok()).unwrap_or(min);
                let w = col.attr_ns(NONE, "width").and_then(|v| v.parse::<f64>().ok());
                let hidden = col.attr_ns(NONE, "hidden").map(|v| v == "1" || v == "true").unwrap_or(false);
                for c in min..=max.min(maxc + 1) {
                    let i = (c - 1) as usize;
                    if i < widths.len() {
                        if let Some(w) = w {
                            widths[i] = self.col_pt(w);
                        }
                        if hidden {
                            hidden_cols.insert(c - 1);
                            widths[i] = 0.0;
                        }
                    }
                }
            }
        }
        // Row heights.
        let mut heights: Vec<f64> = vec![default_h; (maxr + 1) as usize];
        if let Some(data) = root.child(S, "sheetData") {
            for row in data.children_named(S, "row") {
                let r: usize = row.attr_ns(NONE, "r").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
                if r == 0 || r > heights.len() {
                    continue;
                }
                if let Some(h) = row.attr_ns(NONE, "ht").and_then(|v| v.parse::<f64>().ok()) {
                    heights[r - 1] = h;
                }
                if row.attr_ns(NONE, "hidden").map(|v| v == "1" || v == "true").unwrap_or(false) {
                    heights[r - 1] = 0.0;
                }
            }
        }
        // Merges: the cells they cover.
        let mut covered: HashSet<(u32, u32)> = HashSet::new();
        let mut spans: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
        for m in &merges {
            let (r0, r1) = m.rows();
            let (c0, c1) = m.cols();
            spans.insert((r0, c0), (r1 - r0 + 1, c1 - c0 + 1));
            for r in r0..=r1 {
                for c in c0..=c1 {
                    if (r, c) != (r0, c0) {
                        covered.insert((r, c));
                    }
                }
            }
        }
        let grid_stroke = if show_grid { "0.5pt + rgb(\"#D9D9D9\")" } else { "none" };
        let total_w: f64 = widths.iter().sum::<f64>() + if opts.headings { 30.0 } else { 0.0 };
        let total_h: f64 = heights.iter().sum::<f64>() + if opts.headings { default_h } else { 0.0 };
        let mut out = String::new();
        let margin = 18.0;
        let _ = writeln!(out, "#set page(width: {}, height: {}, margin: {})", pt(total_w + margin * 2.0), pt(total_h + margin * 2.0 + 14.0), pt(margin));
        let _ = writeln!(out, "#place(top + left, dy: -14pt, text(size: 8pt, fill: gray, font: (\"Liberation Sans\", \"NanumGothic\", \"DejaVu Sans\"), {}))", s(&x.sheets[si].name));
        let mut col_spec: Vec<String> = Vec::new();
        if opts.headings {
            col_spec.push("30pt".into());
        }
        col_spec.extend(widths.iter().map(|w| pt(*w)));
        let mut row_spec: Vec<String> = Vec::new();
        if opts.headings {
            row_spec.push(pt(default_h));
        }
        row_spec.extend(heights.iter().map(|h| pt(*h)));
        let _ = write!(out, "#table(columns: ({}), rows: ({}), stroke: {grid_stroke}, inset: 0pt, ", col_spec.join(", "), row_spec.join(", "));
        if opts.headings {
            let _ = write!(out, "table.cell(fill: rgb(\"#EFEFEF\"))[], ");
            for c in 0..=maxc {
                let _ = write!(out, "table.cell(fill: rgb(\"#EFEFEF\"), align: center + horizon, text(size: 8pt, fill: rgb(\"#666666\"), font: (\"Liberation Sans\",), {})), ", s(&formula::col_name(c)));
            }
        }
        for r in 0..=maxr {
            if opts.headings {
                let _ = write!(out, "table.cell(fill: rgb(\"#EFEFEF\"), align: center + horizon, text(size: 8pt, fill: rgb(\"#666666\"), font: (\"Liberation Sans\",), {})), ", s(&(r + 1).to_string()));
            }
            for c in 0..=maxc {
                if covered.contains(&(r, c)) {
                    continue;
                }
                let (v, style) = grid.get(&(r, c)).cloned().unwrap_or((Val::Empty, x.find_cell(si, r, c).and_then(|e| e.attr_ns(NONE, "s")).and_then(|s| s.parse().ok()).unwrap_or(0)));
                let (rs, cs_) = spans.get(&(r, c)).copied().unwrap_or((1, 1));
                // Text runs into empty cells to its right (unless wrapped or merged).
                let next_empty = c < maxc && !matches!(grid.get(&(r, c + 1)).map(|g| &g.0), Some(v) if !matches!(v, Val::Empty)) && !covered.contains(&(r, c + 1));
                let cell = self.cell(si, r, c, &v, style, rs, cs_, next_empty);
                out.push_str(&cell);
                out.push_str(", ");
            }
        }
        out.push_str(")\n");
        // Pictures and charts over the cells.
        for (anchor, _) in drawings {
            let (content, (r0, c0, dx0, dy0), size) = anchor;
            let ox: f64 = widths[..(c0 as usize).min(widths.len())].iter().sum::<f64>() + dx0 + if opts.headings { 30.0 } else { 0.0 };
            let oy: f64 = heights[..(r0 as usize).min(heights.len())].iter().sum::<f64>() + dy0 + if opts.headings { default_h } else { 0.0 };
            let (w, h) = match size {
                Size::Abs(w, h) => (w, h),
                Size::To(r1, c1, dx1, dy1) => {
                    let x1: f64 = widths[..(c1 as usize).min(widths.len())].iter().sum::<f64>() + dx1 + if opts.headings { 30.0 } else { 0.0 };
                    let y1: f64 = heights[..(r1 as usize).min(heights.len())].iter().sum::<f64>() + dy1 + if opts.headings { default_h } else { 0.0 };
                    ((x1 - ox).max(1.0), (y1 - oy).max(1.0))
                }
            };
            let c = match content.strip_prefix("\u{1}CHART:") {
                Some(cp) => super::chart::render_part(&self.x.pkg, cp, w, h, &self.theme, self.fonts, &mut self.warnings),
                None => content.replace("WIDTH", &pt(w)).replace("HEIGHT", &pt(h)),
            };
            let _ = writeln!(out, "#place(top + left, dx: {}, dy: {}, box(width: {}, height: {}, {c}))", pt(ox), pt(oy), pt(w), pt(h));
        }
        let _ = hidden_cols;
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn cell(&mut self, si: usize, r: u32, c: u32, v: &Val, style: u32, rs: u32, cs: u32, overflow: bool) -> String {
        let st = self.styles.get(style as usize).cloned().unwrap_or_else(|| self.styles[0].clone());
        let mut args = format!("rowspan: {rs}, colspan: {cs}");
        if let Some(f) = &st.fill {
            let _ = write!(args, ", fill: {}", rgb(f));
        }
        let sides = ["top", "right", "bottom", "left"];
        let strokes: Vec<String> = st
            .borders
            .iter()
            .enumerate()
            .filter_map(|(k, b)| b.as_ref().map(|(w, c, d)| format!("{}: (paint: {}, thickness: {}{})", sides[k], rgb(c), pt(*w), if d.is_empty() { String::new() } else { format!(", dash: \"{d}\"") })))
            .collect();
        if !strokes.is_empty() {
            let _ = write!(args, ", stroke: ({})", strokes.join(", "));
        }
        if matches!(v, Val::Empty) {
            return format!("table.cell({args})[]");
        }
        let text = self.x.display(v, style);
        // Alignment: general aligns numbers right, text left, booleans and errors center.
        let h = match st.halign.as_deref() {
            Some("center") | Some("centerContinuous") => "center",
            Some("right") => "right",
            Some("left") => "left",
            Some("justify") | Some("distributed") => "left",
            Some("fill") => "left",
            _ => match v {
                Val::Num(_) => "right",
                Val::Bool(_) | Val::Err(_) => "center",
                _ => "left",
            },
        };
        let vv = match st.valign.as_deref() {
            Some("top") => "top",
            Some("center") => "horizon",
            _ => "bottom",
        };
        let stack = office::font_stack(self.fonts, &st.font, None);
        let mut targs = format!("font: {}, size: {}", typ::fonts(&stack), pt(st.size));
        if st.bold {
            targs.push_str(", weight: \"bold\"");
        }
        if st.italic {
            targs.push_str(", style: \"italic\"");
        }
        let color = st.color.clone().filter(|c| c != "000000");
        if let Some(c) = &color {
            let _ = write!(targs, ", fill: {}", rgb(c));
        }
        // Negative numbers in red formats ([Red]).
        let fmt = self.x.number_format(style);
        if let Val::Num(n) = v {
            if fmt.contains("[Red]") && (*n < 0.0 || !fmt.contains(';')) {
                targs.push_str(", fill: red");
            }
        }
        if text.chars().any(|c| matches!(c as u32, 0xAC00..=0xD7A3)) {
            targs.push_str(", lang: \"ko\"");
        }
        // Unwrapped text stays on one line.
        let keep = |t: &str| if st.wrap { t.to_string() } else { t.replace(' ', "\u{a0}") };
        let mut t = format!("text({targs}, {})", s(&keep(&text)));
        if st.underline {
            t = format!("underline({t})");
        }
        if st.strike {
            t = format!("strike({t})");
        }
        // Rich text: each run with its own font properties.
        if let Some(rich) = self.x.rich_string(si, r, c) {
            let mut parts = Vec::new();
            for run in rich.children_named(S, "r") {
                let rt = run.child(S, "t").map(|t| crate::xlsx::decode_x(&t.text())).unwrap_or_default();
                let mut rs_ = st.clone();
                if let Some(rpr) = run.child(S, "rPr") {
                    let flag = |k: &str| rpr.child(S, k).map(|e| e.attr_ns(NONE, "val").map(|v| v != "0" && v != "false").unwrap_or(true));
                    if let Some(b) = flag("b") {
                        rs_.bold = b;
                    }
                    if let Some(i) = flag("i") {
                        rs_.italic = i;
                    }
                    if let Some(x) = flag("strike") {
                        rs_.strike = x;
                    }
                    rs_.underline = rpr.child(S, "u").is_some();
                    if let Some(sz) = rpr.child(S, "sz").and_then(|e| e.attr_ns(NONE, "val")).and_then(|v| v.parse().ok()) {
                        rs_.size = sz;
                    }
                    if let Some(f) = rpr.child(S, "rFont").and_then(|e| e.attr_ns(NONE, "val")) {
                        rs_.font = f.to_string();
                    }
                    if let Some(col) = rpr.child(S, "color").and_then(|e| sml_color(e, &self.theme)) {
                        rs_.color = Some(col);
                    }
                }
                let stack = office::font_stack(self.fonts, &rs_.font, None);
                let mut a = format!("font: {}, size: {}", typ::fonts(&stack), pt(rs_.size));
                if rs_.bold {
                    a.push_str(", weight: \"bold\"");
                }
                if rs_.italic {
                    a.push_str(", style: \"italic\"");
                }
                if let Some(col) = rs_.color.as_ref().filter(|c| *c != "000000") {
                    let _ = write!(a, ", fill: {}", rgb(col));
                }
                let mut e = format!("text({a}, {})", s(&keep(&rt)));
                if rs_.underline {
                    e = format!("underline({e})");
                }
                if rs_.strike {
                    e = format!("strike({e})");
                }
                parts.push(format!("#{e}"));
            }
            t = format!("[{}]", parts.join(""));
        }
        // Numbers too wide for their column show ####, as Excel does; text runs on.
        let pad = format!("{}pt", 2.25 + st.indent as f64 * 9.0);
        let content = if st.wrap {
            format!("block(width: 100%, inset: (left: {pad}, right: 2.25pt), align({h}, {t}))")
        } else if overflow && h == "left" {
            format!("box(inset: (left: {pad}), {t})")
        } else if overflow {
            format!("box(inset: (left: {pad}, right: 2.25pt), {t})")
        } else {
            // The next cell has a value: the text stops at the border.
            format!("box(width: 100%, clip: true, inset: (left: {pad}, right: 2.25pt), align({h}, {t}))")
        };
        let content = if st.rotation != 0 {
            let deg = if st.rotation <= 90 { -st.rotation } else { st.rotation - 90 };
            format!("rotate({deg}deg, reflow: true, {content})")
        } else {
            content
        };
        let _ = write!(args, ", align: {h} + {vv}");
        format!("table.cell({args}, {content})")
    }

    /// Pictures and charts of a sheet: (Typst content with WIDTH/HEIGHT, from (row, col, dx, dy),
    /// size), and the far cell they reach.
    #[allow(clippy::type_complexity)]
    fn drawings(&mut self, si: usize) -> Vec<((String, (u32, u32, f64, f64), Size), (u32, u32))> {
        let mut out = Vec::new();
        let part = self.x.sheets[si].part.clone();
        let Ok(rels) = self.x.pkg.rels(&part) else { return out };
        for d in rels.by_kind("drawing") {
            let Some(dp) = rels.part(&d.id) else { continue };
            let Ok(doc) = self.x.pkg.xml(&dp) else { continue };
            let Ok(drels) = self.x.pkg.rels(&dp) else { continue };
            for anchor in doc.root.elements() {
                let corner = |name: &str| -> Option<(u32, u32, f64, f64)> {
                    let e = anchor.child(XDR, name)?;
                    let g = |k: &str| e.child(XDR, k).map(|x| x.text().trim().parse::<f64>().unwrap_or(0.0)).unwrap_or(0.0);
                    Some((g("row") as u32, g("col") as u32, g("colOff") / 12700.0, g("rowOff") / 12700.0))
                };
                let Some((r0, c0, dx, dy)) = corner("from") else { continue };
                let size = match corner("to") {
                    Some((r1, c1, dx1, dy1)) => Size::To(r1, c1, dx1, dy1),
                    None => {
                        let ext = anchor.child(XDR, "ext");
                        let g = |k: &str| ext.and_then(|e| e.attr_ns(NONE, k)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(914400.0) / 12700.0;
                        Size::Abs(g("cx"), g("cy"))
                    }
                };
                let far = match &size {
                    Size::To(r1, c1, ..) => (*r1, *c1),
                    Size::Abs(..) => (r0 + 1, c0 + 1),
                };
                let content = if let Some(pic) = anchor.child(XDR, "pic") {
                    let rid = pic.child(XDR, "blipFill").and_then(|b| b.child(A, "blip")).and_then(|b| b.attr_ns(R, "embed")).map(str::to_string);
                    let Some(rid) = rid else { continue };
                    let Some(rel) = drels.get(&rid) else { continue };
                    let target = opc::resolve(&dp, &rel.target);
                    let Ok(data) = self.x.pkg.zip.read(&target) else { continue };
                    let Some((vpath, bytes)) = images::prepare(&target, data, &mut self.warnings) else { continue };
                    if !self.files.iter().any(|f| f.0 == vpath) {
                        self.files.push((vpath.clone(), bytes));
                    }
                    format!("image({}, width: WIDTH, height: HEIGHT, fit: \"stretch\")", s(&format!("/{vpath}")))
                } else if let Some(gf) = anchor.child(XDR, "graphicFrame") {
                    let Some(chart) = gf.descendants().into_iter().find(|e| e.local() == "chart") else { continue };
                    let Some(cp) = chart.attr_ns(R, "id").and_then(|id| drels.part(id)) else { continue };
                    // Drawn once its size is known.
                    format!("\u{1}CHART:{cp}")
                } else {
                    continue;
                };
                out.push(((content, (r0, c0, dx, dy), size), far));
            }
        }
        out
    }
}

enum Size {
    Abs(f64, f64),
    To(u32, u32, f64, f64),
}

/// The width of the widest digit of a font, in pixels at 96 dpi (Excel's column unit).
fn digit_width(fonts: &Fonts, name: &str, size: f64) -> f64 {
    let stack = office::font_stack(fonts, name, None);
    for f in &stack {
        if let Some(font) = fonts.first_of(f) {
            if let Ok(face) = ttf_parser::Face::parse(font.data(), font.index()) {
                let upem = face.units_per_em() as f64;
                let mut w: f64 = 0.0;
                for d in '0'..='9' {
                    if let Some(g) = face.glyph_index(d) {
                        w = w.max(face.glyph_hor_advance(g).unwrap_or(0) as f64);
                    }
                }
                if w > 0.0 {
                    return (w / upem * size * 96.0 / 72.0).round();
                }
            }
        }
    }
    7.0
}
