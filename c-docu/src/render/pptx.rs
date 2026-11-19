//! Presentations as Typst: a page per slide, its background, then the master's and layout's own
//! drawings and the slide's shapes, each placed where its transform puts it. Text takes the
//! style chain PowerPoint uses (presentation defaults, master text styles, master and layout
//! placeholders' list styles, the shape's list style, the paragraph and run), with bullets,
//! autofit's font scale and the body's insets and anchoring.

use super::office::{self, Theme};
use super::typ::{self, pt, rgb, rgba, s};
use super::world::Fonts;
use super::{images, shapes, Job};
use crate::opc::{self, Rels};
use crate::pptx::Pptx;
use crate::xml::{Document, Element, A, MC, NONE, P, R};
use crate::Result;
use std::collections::HashMap;
use std::fmt::Write;

fn emu(v: Option<&str>) -> f64 {
    v.and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0) / 12700.0
}

fn attr<'a>(e: &'a Element, a: &str) -> Option<&'a str> {
    e.attr_ns(NONE, a)
}

#[derive(Clone, Default, Debug)]
struct DRun {
    size: Option<f64>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<String>,
    strike: Option<String>,
    baseline: Option<f64>,
    cap: Option<String>,
    spc: Option<f64>,
    color: Option<(String, f64)>,
    latin: Option<String>,
    ea: Option<String>,
    highlight: Option<String>,
    lang: Option<String>,
}

#[derive(Clone, Debug)]
enum Spacing {
    Pct(f64),
    Pts(f64),
}

#[derive(Clone, Debug)]
enum Bullet {
    None,
    Char(String),
    Auto(String, i64),
}

#[derive(Clone, Default, Debug)]
struct DPara {
    algn: Option<String>,
    mar_l: Option<f64>,
    indent: Option<f64>,
    lnspc: Option<Spacing>,
    spc_bef: Option<Spacing>,
    spc_aft: Option<Spacing>,
    bullet: Option<Bullet>,
    bu_font: Option<String>,
    bu_color: Option<(String, f64)>,
    bu_size: Option<f64>,
    run: DRun,
}

struct Ctx<'a> {
    theme: &'a Theme,
    map: &'a HashMap<String, String>,
}

impl DRun {
    fn apply(&mut self, r: &Element, c: &Ctx) {
        if let Some(v) = attr(r, "sz").and_then(|v| v.parse::<f64>().ok()) {
            self.size = Some(v / 100.0);
        }
        if let Some(v) = attr(r, "b") {
            self.bold = Some(v == "1" || v == "true");
        }
        if let Some(v) = attr(r, "i") {
            self.italic = Some(v == "1" || v == "true");
        }
        if let Some(v) = attr(r, "u") {
            self.underline = Some(v.to_string());
        }
        if let Some(v) = attr(r, "strike") {
            self.strike = Some(v.to_string());
        }
        if let Some(v) = attr(r, "baseline").and_then(|v| v.parse::<f64>().ok()) {
            self.baseline = Some(v / 100000.0);
        }
        if let Some(v) = attr(r, "cap") {
            self.cap = Some(v.to_string());
        }
        if let Some(v) = attr(r, "spc").and_then(|v| v.parse::<f64>().ok()) {
            self.spc = Some(v / 100.0);
        }
        if let Some(v) = attr(r, "lang") {
            self.lang = Some(v.to_string());
        }
        for e in r.elements() {
            match e.local() {
                "solidFill" => self.color = office::dml_color(e, c.theme, c.map, None),
                "gradFill" => {
                    // The first stop's color.
                    if let Some(gs) = e.child(A, "gsLst").and_then(|l| l.child(A, "gs")) {
                        self.color = office::dml_color(gs, c.theme, c.map, None);
                    }
                }
                "noFill" => self.color = Some(("FFFFFF".into(), 0.0)),
                "latin" => self.latin = attr(e, "typeface").map(str::to_string),
                "ea" => self.ea = attr(e, "typeface").map(str::to_string),
                "highlight" => self.highlight = office::dml_color(e, c.theme, c.map, None).map(|x| x.0),
                _ => {}
            }
        }
    }
}

fn spacing(e: &Element) -> Option<Spacing> {
    if let Some(p) = e.child(A, "spcPct") {
        return attr(p, "val").and_then(|v| v.trim_end_matches('%').parse::<f64>().ok()).map(|v| Spacing::Pct(if v > 1000.0 { v / 100000.0 } else { v / 100.0 }));
    }
    if let Some(p) = e.child(A, "spcPts") {
        return attr(p, "val").and_then(|v| v.parse::<f64>().ok()).map(|v| Spacing::Pts(v / 100.0));
    }
    None
}

impl DPara {
    fn apply(&mut self, p: &Element, c: &Ctx) {
        if let Some(v) = attr(p, "algn") {
            self.algn = Some(v.to_string());
        }
        if let Some(v) = attr(p, "marL") {
            self.mar_l = Some(emu(Some(v)));
        }
        if let Some(v) = attr(p, "indent") {
            self.indent = Some(emu(Some(v)));
        }
        for e in p.elements() {
            match e.local() {
                "lnSpc" => self.lnspc = spacing(e),
                "spcBef" => self.spc_bef = spacing(e),
                "spcAft" => self.spc_aft = spacing(e),
                "buNone" => self.bullet = Some(Bullet::None),
                "buChar" => self.bullet = Some(Bullet::Char(attr(e, "char").unwrap_or("•").to_string())),
                "buAutoNum" => self.bullet = Some(Bullet::Auto(attr(e, "type").unwrap_or("arabicPeriod").to_string(), attr(e, "startAt").and_then(|v| v.parse().ok()).unwrap_or(1))),
                "buFont" => self.bu_font = attr(e, "typeface").map(str::to_string),
                "buFontTx" => self.bu_font = None,
                "buClr" => self.bu_color = office::dml_color(e, c.theme, c.map, None),
                "buClrTx" => self.bu_color = None,
                "buSzPct" => self.bu_size = attr(e, "val").and_then(|v| v.parse::<f64>().ok()).map(|v| v / 100000.0),
                "buSzTx" => self.bu_size = None,
                "defRPr" => self.run.apply(e, c),
                _ => {}
            }
        }
    }
}

/// A list style's level (`a:lvl1pPr` …) properties.
fn level_of(list: Option<&Element>, lvl: usize) -> Option<&Element> {
    let l = list?;
    l.child(A, &format!("lvl{}pPr", lvl + 1))
}

struct Master {
    doc: Document,
    part: String,
    rels: Rels,
    map: HashMap<String, String>,
}

pub fn convert(p: &Pptx, fonts: &Fonts) -> Result<Job> {
    let mut c = Conv { p, fonts, files: Vec::new(), media: HashMap::new(), warnings: Vec::new(), masters: HashMap::new(), layouts: HashMap::new() };
    let src = c.document()?;
    Ok(Job { source: src, files: c.files, warnings: c.warnings })
}

struct Conv<'a> {
    p: &'a Pptx,
    fonts: &'a Fonts,
    files: Vec<(String, Vec<u8>)>,
    media: HashMap<String, String>,
    warnings: Vec<String>,
    masters: HashMap<String, (Master, Theme)>,
    layouts: HashMap<String, (Document, Rels, String)>,
}

/// The shape a placeholder inherits from: (its element, from the layout) and (from the master).
struct Inherit {
    layout: Option<Element>,
    master: Option<Element>,
}

impl<'a> Conv<'a> {
    fn document(&mut self) -> Result<String> {
        let (cx, cy) = self.p.slide_size();
        let (w, h) = (cx as f64 / 12700.0, cy as f64 / 12700.0);
        let mut out = String::new();
        out.push_str(typ::PRELUDE);
        let _ = writeln!(out, "#set page(width: {}, height: {}, margin: 0pt)\n#set text(hyphenate: false, kerning: true, ligatures: false, top-edge: \"ascender\", bottom-edge: \"descender\")\n#set par(spacing: 0pt, leading: 0pt)\n#set block(spacing: 0pt)", pt(w), pt(h));
        let n = self.p.slides.len();
        for si in 0..n {
            if si > 0 {
                out.push_str("#pagebreak()\n");
            }
            let slide_src = self.slide(si, w, h);
            out.push_str(&slide_src);
        }
        if n == 0 {
            out.push_str("#[]\n");
        }
        Ok(out)
    }

    fn load_layout(&mut self, part: &str) -> Option<String> {
        if !self.layouts.contains_key(part) {
            let doc = self.p.pkg.xml(part).ok()?;
            let rels = self.p.pkg.rels(part).ok()?;
            let master = rels.first_part("slideMaster")?;
            self.layouts.insert(part.to_string(), (doc, rels, master.clone()));
            if !self.masters.contains_key(&master) {
                let mdoc = self.p.pkg.xml(&master).ok()?;
                let mrels = self.p.pkg.rels(&master).ok()?;
                let theme = Theme::load(&self.p.pkg, mrels.first_part("theme"));
                let mut map = HashMap::new();
                if let Some(cm) = mdoc.root.child(P, "clrMap") {
                    for a in cm.attrs() {
                        map.insert(a.local().to_string(), a.value.clone());
                    }
                }
                self.masters.insert(master.clone(), (Master { doc: mdoc, part: master.clone(), rels: mrels, map }, theme));
            }
        }
        Some(self.layouts.get(part)?.2.clone())
    }

    fn slide(&mut self, si: usize, w: f64, h: f64) -> String {
        let slide = &self.p.slides[si];
        let mut out = String::new();
        let Some(layout_part) = slide.layout.clone() else {
            return "#[]\n".into();
        };
        let Some(master_part) = self.load_layout(&layout_part) else { return "#[]\n".into() };
        let (ldoc, lrels, _) = self.layouts.get(&layout_part).cloned().unwrap();
        let (master, theme) = self.masters.get(&master_part).map(|(m, t)| (Master { doc: m.doc.clone(), part: m.part.clone(), rels: m.rels.clone(), map: m.map.clone() }, t.clone())).unwrap();
        // The slide's color map: the master's unless the slide or layout overrides it.
        let mut map = master.map.clone();
        for d in [&ldoc, &slide.doc] {
            if let Some(ov) = d.root.child(P, "clrMapOvr").and_then(|o| o.child(A, "overrideClrMapping")) {
                map.clear();
                for a in ov.attrs() {
                    map.insert(a.local().to_string(), a.value.clone());
                }
            }
        }
        let ctx = Ctx { theme: &theme, map: &map };
        // Background: the slide's, the layout's or the master's.
        let bg = [&slide.doc, &ldoc, &master.doc].iter().find_map(|d| d.root.child(P, "cSld").and_then(|c| c.child(P, "bg")).cloned());
        if let Some(bg) = bg {
            if let Some(fill) = self.background(&bg, &ctx, &slide.rels, &slide.part) {
                let _ = writeln!(out, "#place(top + left, {})", fill.replace("WIDTH", &pt(w)).replace("HEIGHT", &pt(h)));
            }
        }
        let show_master = |d: &Document| attr(&d.root, "showMasterSp").map(|v| v != "0").unwrap_or(true);
        // The master's and layout's own drawings (not their placeholders).
        if show_master(&slide.doc) && show_master(&ldoc) {
            let tree = master.doc.root.child(P, "cSld").and_then(|c| c.child(P, "spTree")).cloned();
            if let Some(t) = tree {
                out.push_str(&self.tree(&t, &ctx, &master.rels, &master.part, None, true, si, &Inherits::none(), (0.0, 0.0, 1.0, 1.0)));
            }
        }
        if show_master(&slide.doc) {
            let tree = ldoc.root.child(P, "cSld").and_then(|c| c.child(P, "spTree")).cloned();
            if let Some(t) = tree {
                out.push_str(&self.tree(&t, &ctx, &lrels, &layout_part, None, true, si, &Inherits::none(), (0.0, 0.0, 1.0, 1.0)));
            }
        }
        let inh = Inherits { layout: Some(ldoc), master: Some(master.doc), pres: self.p.pres.root.child(P, "defaultTextStyle").cloned() };
        if let Some(t) = slide.doc.root.child(P, "cSld").and_then(|c| c.child(P, "spTree")).cloned() {
            let rels = self.p.pkg.rels(&slide.part).unwrap_or_else(|_| slide.rels.clone());
            let part = slide.part.clone();
            out.push_str(&self.tree(&t, &ctx, &rels, &part, None, false, si, &inh, (0.0, 0.0, 1.0, 1.0)));
        }
        out
    }

    fn background(&mut self, bg: &Element, ctx: &Ctx, rels: &Rels, part: &str) -> Option<String> {
        if let Some(pr) = bg.child(P, "bgPr") {
            let fill = pr.elements().find(|e| matches!(e.local(), "solidFill" | "gradFill" | "pattFill" | "blipFill" | "noFill"))?;
            if fill.local() == "blipFill" {
                let rid = fill.child(A, "blip").and_then(|b| b.attr_ns(R, "embed"))?;
                let file = self.image_file(rels, part, rid)?;
                return Some(format!("image({}, width: WIDTH, height: HEIGHT, fit: \"cover\")", s(&format!("/{file}"))));
            }
            let paint = shapes::paint_of_fill(fill, ctx.theme, ctx.map, None)?;
            return Some(format!("rect(width: WIDTH, height: HEIGHT, fill: {paint}, stroke: none)"));
        }
        if let Some(r) = bg.child(P, "bgRef") {
            let (hex, a) = office::dml_color(r, ctx.theme, ctx.map, None)?;
            let paint = if a < 1.0 { rgba(&hex, a) } else { rgb(&hex) };
            return Some(format!("rect(width: WIDTH, height: HEIGHT, fill: {paint}, stroke: none)"));
        }
        None
    }

    fn image_file(&mut self, rels: &Rels, part: &str, rid: &str) -> Option<String> {
        let rel = rels.get(rid)?;
        if rel.external {
            return None;
        }
        let target = opc::resolve(part, &rel.target);
        if let Some(v) = self.media.get(&target) {
            return Some(v.clone());
        }
        let data = self.p.pkg.zip.read(&target).ok()?;
        let (vpath, bytes) = images::prepare(&target, data, &mut self.warnings)?;
        self.files.push((vpath.clone(), bytes));
        self.media.insert(target, vpath.clone());
        Some(vpath)
    }

    /// The shapes of a tree (`p:spTree` or a group); `skip_ph` leaves placeholders out (master,
    /// layout). `xf`: the group's mapping of child coordinates (offset x, y, scale x, y).
    #[allow(clippy::too_many_arguments)]
    fn tree(&mut self, tree: &Element, ctx: &Ctx, rels: &Rels, part: &str, _parent: Option<&Element>, skip_ph: bool, si: usize, inh: &Inherits, xf: (f64, f64, f64, f64)) -> String {
        let mut out = String::new();
        for c in tree.elements() {
            let c = if c.ns() == MC && c.local() == "AlternateContent" {
                match c.child(MC, "Choice").and_then(|x| x.elements().next()).or_else(|| c.child(MC, "Fallback").and_then(|x| x.elements().next())) {
                    Some(x) => x,
                    None => continue,
                }
            } else {
                c
            };
            let nv = c.elements().find(|e| e.local().starts_with("nv"));
            let ph = nv.and_then(|v| v.child(P, "nvPr")).and_then(|v| v.child(P, "ph"));
            if skip_ph && ph.is_some() {
                continue;
            }
            if nv.and_then(|v| v.child(P, "cNvPr")).and_then(|c| attr(c, "hidden")).map(|h| h == "1" || h == "true").unwrap_or(false) {
                continue;
            }
            let shape = match (c.ns(), c.local()) {
                (P, "sp") | (P, "cxnSp") => self.sp(c, ctx, rels, part, si, inh, ph, xf),
                (P, "pic") => self.pic(c, ctx, rels, part, inh, ph, xf),
                (P, "grpSp") => self.group(c, ctx, rels, part, skip_ph, si, inh, xf),
                (P, "graphicFrame") => self.frame(c, ctx, rels, part, si, inh, xf),
                _ => None,
            };
            if let Some(sh) = shape {
                out.push_str(&sh);
                out.push('\n');
            }
        }
        out
    }

    /// Position and size of a shape (own transform or the placeholder's), mapped through groups.
    fn geometry(&self, el: &Element, inh_shapes: &Inherit, xf: (f64, f64, f64, f64)) -> Option<(f64, f64, f64, f64, f64, bool, bool)> {
        let find = |e: &Element| -> Option<Element> {
            e.child(P, "spPr").and_then(|s| s.child(A, "xfrm")).or_else(|| e.child(P, "xfrm")).or_else(|| e.child(P, "grpSpPr").and_then(|s| s.child(A, "xfrm"))).cloned()
        };
        let x = find(el).filter(|x| x.child(A, "off").is_some()).or_else(|| inh_shapes.layout.as_ref().and_then(find).filter(|x| x.child(A, "off").is_some())).or_else(|| inh_shapes.master.as_ref().and_then(find))?;
        let off = x.child(A, "off")?;
        let ext = x.child(A, "ext")?;
        let rot = attr(&x, "rot").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 60000.0;
        let fh = attr(&x, "flipH").map(|v| v == "1" || v == "true").unwrap_or(false);
        let fv = attr(&x, "flipV").map(|v| v == "1" || v == "true").unwrap_or(false);
        let (ox, oy, sx, sy) = xf;
        Some((ox + emu(attr(off, "x")) * sx, oy + emu(attr(off, "y")) * sy, emu(attr(ext, "cx")) * sx, emu(attr(ext, "cy")) * sy, rot, fh, fv))
    }

    fn place(&self, x: f64, y: f64, w: f64, h: f64, rot: f64, fh: bool, fv: bool, content: &str) -> String {
        let mut c = format!("box(width: {}, height: {}, {content})", pt(w), pt(h));
        if fh || fv {
            c = format!("scale(x: {}, y: {}, reflow: false, {c})", if fh { "-100%" } else { "100%" }, if fv { "-100%" } else { "100%" });
        }
        if rot != 0.0 {
            c = format!("rotate({}deg, origin: center + horizon, reflow: false, {c})", typ::num(rot));
        }
        format!("#place(top + left, dx: {}, dy: {}, {c})", pt(x), pt(y))
    }

    #[allow(clippy::too_many_arguments)]
    fn sp(&mut self, el: &Element, ctx: &Ctx, rels: &Rels, part: &str, si: usize, inh: &Inherits, ph: Option<&Element>, xf: (f64, f64, f64, f64)) -> Option<String> {
        let ph_shapes = inh.find(ph);
        let (x, y, w, h, rot, fh, fv) = self.geometry(el, &ph_shapes, xf)?;
        let sppr = el.child(P, "spPr");
        let style = el.child(P, "style");
        // Fill and outline: the shape's, else the placeholder's.
        let ph_sppr = ph_shapes.layout.as_ref().and_then(|e| e.child(P, "spPr")).or_else(|| ph_shapes.master.as_ref().and_then(|e| e.child(P, "spPr")));
        let has_own_fill = sppr.map(|s| s.elements().any(|e| matches!(e.local(), "noFill" | "solidFill" | "gradFill" | "blipFill" | "pattFill" | "grpFill"))).unwrap_or(false);
        let fill_src = if has_own_fill || style.is_some() { sppr } else { ph_sppr.or(sppr) };
        let mut fill = shapes::fill_of(fill_src, style, ctx.theme, ctx.map);
        if let Some(bf) = fill_src.and_then(|s| s.child(A, "blipFill")) {
            if let Some(rid) = bf.child(A, "blip").and_then(|b| b.attr_ns(R, "embed")) {
                if let Some(f) = self.image_file(rels, part, rid) {
                    fill = Some(format!("tiling(size: ({}, {}), image({}, width: {}, height: {}, fit: \"stretch\"))", pt(w.max(1.0)), pt(h.max(1.0)), s(&format!("/{f}")), pt(w.max(1.0)), pt(h.max(1.0))));
                }
            }
        }
        let has_own_ln = sppr.and_then(|s| s.child(A, "ln")).is_some();
        let stroke = shapes::stroke_of(if has_own_ln || style.is_some() { sppr } else { ph_sppr.or(sppr) }, style, ctx.theme, ctx.map);
        let geom_el = sppr.and_then(|s| s.child(A, "prstGeom")).or_else(|| ph_sppr.and_then(|s| s.child(A, "prstGeom")));
        let cust = sppr.and_then(|s| s.child(A, "custGeom"));
        let drawing = if let Some(cg) = cust {
            shapes::custom(cg, w, h, &fill, &stroke)
        } else {
            let name = geom_el.and_then(|g| attr(g, "prst")).unwrap_or("rect");
            if fill.is_none() && stroke == "none" {
                "none".into()
            } else {
                shapes::preset(name, w, h, &fill, &stroke, geom_el)
            }
        };
        // Arrowheads of lines.
        let mut heads = String::new();
        if el.is(P, "cxnSp") || geom_el.and_then(|g| attr(g, "prst")).map(|p| p.contains("line") || p.contains("Connector")).unwrap_or(false) {
            if let Some(ln) = sppr.and_then(|s| s.child(A, "ln")) {
                heads = arrowheads(ln, w, h, &stroke);
            }
        }
        let text = match el.child(P, "txBody") {
            Some(tb) => self.text_body(tb, ctx, w, h, ph, inh, &ph_shapes, si, rels, part),
            None => String::new(),
        };
        let mut content = String::new();
        if drawing != "none" {
            let _ = write!(content, "#place(top + left, {drawing})");
        }
        content.push_str(&heads);
        content.push_str(&text);
        if content.is_empty() {
            return None;
        }
        Some(self.place(x, y, w, h, rot, fh, fv, &format!("[{content}]")))
    }

    #[allow(clippy::too_many_arguments)]
    fn pic(&mut self, el: &Element, ctx: &Ctx, rels: &Rels, part: &str, inh: &Inherits, ph: Option<&Element>, xf: (f64, f64, f64, f64)) -> Option<String> {
        let ph_shapes = inh.find(ph);
        let (x, y, w, h, rot, fh, fv) = self.geometry(el, &ph_shapes, xf)?;
        let bf = el.child(P, "blipFill")?;
        let blip = bf.child(A, "blip")?;
        let svg = blip.descendants().iter().find(|e| e.local() == "svgBlip").and_then(|e| e.attr_ns(R, "embed")).map(str::to_string);
        let rid = blip.attr_ns(R, "embed").or_else(|| blip.attr_ns(R, "link"))?.to_string();
        let file = svg.and_then(|r| self.image_file(rels, part, &r)).or_else(|| self.image_file(rels, part, &rid))?;
        let img = images::image_expr(&file, w, h, bf.child(A, "srcRect"));
        let sppr = el.child(P, "spPr");
        let stroke = shapes::stroke_of(sppr, el.child(P, "style"), ctx.theme, ctx.map);
        let geom = sppr.and_then(|s| s.child(A, "prstGeom")).and_then(|g| attr(g, "prst")).unwrap_or("rect");
        let mut content = if geom == "ellipse" {
            format!("box(width: {}, height: {}, clip: true, radius: 50%, {img})", pt(w), pt(h))
        } else if geom == "roundRect" {
            format!("box(width: {}, height: {}, clip: true, radius: {}, {img})", pt(w), pt(h), pt(w.min(h) * 0.1667))
        } else {
            img
        };
        if stroke != "none" {
            content = format!("box(stroke: {stroke}, {content})");
        }
        Some(self.place(x, y, w, h, rot, fh, fv, &content))
    }

    #[allow(clippy::too_many_arguments)]
    fn group(&mut self, el: &Element, ctx: &Ctx, rels: &Rels, part: &str, skip_ph: bool, si: usize, inh: &Inherits, xf: (f64, f64, f64, f64)) -> Option<String> {
        let gx = el.child(P, "grpSpPr").and_then(|s| s.child(A, "xfrm"))?;
        let (ox, oy, sx, sy) = xf;
        let off = gx.child(A, "off")?;
        let ext = gx.child(A, "ext")?;
        let ch_off = gx.child(A, "chOff");
        let ch_ext = gx.child(A, "chExt");
        let (x, y) = (ox + emu(attr(off, "x")) * sx, oy + emu(attr(off, "y")) * sy);
        let (w, h) = (emu(attr(ext, "cx")) * sx, emu(attr(ext, "cy")) * sy);
        let (cox, coy) = (ch_off.map(|c| emu(attr(c, "x"))).unwrap_or(0.0), ch_off.map(|c| emu(attr(c, "y"))).unwrap_or(0.0));
        let (cw, ch) = (ch_ext.map(|c| emu(attr(c, "cx"))).unwrap_or(w), ch_ext.map(|c| emu(attr(c, "cy"))).unwrap_or(h));
        let nsx = if cw > 0.0 { w / cw } else { 1.0 };
        let nsy = if ch > 0.0 { h / ch } else { 1.0 };
        // Children in slide coordinates: x' = x + (cx - chOff) * scale.
        let child_xf = (x - cox * nsx, y - coy * nsy, nsx, nsy);
        let rot = attr(gx, "rot").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 60000.0;
        let inner = self.tree(el, ctx, rels, part, None, skip_ph, si, inh, child_xf);
        if rot == 0.0 {
            return Some(inner);
        }
        // A rotated group: its children are drawn in a box of its own, rotated whole.
        let local_xf = (-cox * nsx, -coy * nsy, nsx, nsy);
        let inner = self.tree(el, ctx, rels, part, None, skip_ph, si, inh, local_xf);
        Some(self.place(x, y, w, h, rot, false, false, &format!("[{inner}]")))
    }

    #[allow(clippy::too_many_arguments)]
    fn frame(&mut self, el: &Element, ctx: &Ctx, rels: &Rels, part: &str, si: usize, inh: &Inherits, xf: (f64, f64, f64, f64)) -> Option<String> {
        let (x, y, w, h, rot, fh, fv) = self.geometry(el, &Inherit { layout: None, master: None }, xf)?;
        let gd = el.child(A, "graphic").and_then(|g| g.child(A, "graphicData"))?;
        if let Some(tbl) = gd.child(A, "tbl") {
            let t = self.table(tbl, ctx, w, si, inh, rels, part);
            return Some(self.place(x, y, w, h, rot, fh, fv, &t));
        }
        if let Some(c) = gd.elements().find(|e| e.local() == "chart") {
            let rid = c.attr_ns(R, "id")?;
            let cp = rels.part(rid)?;
            let t = super::chart::render_part(&self.p.pkg, &cp, w, h, ctx.theme, self.fonts, &mut self.warnings);
            return Some(self.place(x, y, w, h, rot, fh, fv, &t));
        }
        if gd.elements().any(|e| e.local() == "relIds") {
            // SmartArt: the drawing PowerPoint saved with it.
            let dm = gd.elements().find(|e| e.local() == "relIds")?;
            let _ = dm;
            let drawing = rels.list.iter().find(|r| r.rtype.ends_with("/diagramDrawing")).map(|r| opc::resolve(part, &r.target));
            if let Some(dp) = drawing {
                if let Ok(dd) = self.p.pkg.xml(&dp) {
                    let drels = self.p.pkg.rels(&dp).unwrap_or_else(|_| rels.clone());
                    if let Some(tree) = dd.root.elements().find(|e| e.local() == "spTree") {
                        let tree = rename_dsp(tree);
                        let inner = self.tree(&tree, ctx, &drels, &dp, None, false, si, &Inherits::none(), (x, y, 1.0, 1.0));
                        return Some(inner);
                    }
                }
            }
            return None;
        }
        // An OLE object or other content with a fallback picture.
        if let Some(pic) = el.descendants().iter().find(|e| e.is(P, "pic")) {
            return self.pic(pic, ctx, rels, part, inh, None, xf).map(|s| s.to_string());
        }
        None
    }

    // ── text ────────────────────────────────────────────────────────────────────────────────

    /// The paragraph and run properties at a level, from the chain of list styles.
    fn level_props(&self, lvl: usize, ph: Option<&Element>, inh: &Inherits, ph_shapes: &Inherit, own_list: Option<&Element>, ctx: &Ctx) -> DPara {
        let mut p = DPara::default();
        let ph_type = ph.map(|e| attr(e, "type").unwrap_or("body")).unwrap_or("");
        if let Some(d) = &inh.pres {
            if let Some(l) = level_of(Some(d), lvl) {
                p.apply(l, ctx);
            }
        }
        if let Some(m) = &inh.master {
            if let Some(ts) = m.root.child(P, "txStyles") {
                let which = match ph_type {
                    "title" | "ctrTitle" => "titleStyle",
                    "" => "otherStyle",
                    "dt" | "ftr" | "sldNum" | "hdr" => "otherStyle",
                    _ => "bodyStyle",
                };
                if ph.is_some() || which == "otherStyle" {
                    if let Some(l) = level_of(ts.child(P, which), lvl) {
                        p.apply(l, ctx);
                    }
                }
            }
        }
        for src in [ph_shapes.master.as_ref(), ph_shapes.layout.as_ref()] {
            if let Some(s) = src {
                if let Some(l) = level_of(s.child(P, "txBody").and_then(|t| t.child(A, "lstStyle")), lvl) {
                    p.apply(l, ctx);
                }
            }
        }
        if let Some(l) = level_of(own_list, lvl) {
            p.apply(l, ctx);
        }
        p
    }

    #[allow(clippy::too_many_arguments)]
    fn text_body(&mut self, tb: &Element, ctx: &Ctx, w: f64, h: f64, ph: Option<&Element>, inh: &Inherits, ph_shapes: &Inherit, si: usize, _rels: &Rels, _part: &str) -> String {
        let paras: Vec<&Element> = tb.children_named(A, "p").collect();
        if paras.iter().all(|p| crate::text::text_of(&crate::dml::Dml, p).trim().is_empty()) {
            return String::new();
        }
        // Body properties: the shape's over the placeholders'.
        let mut body: HashMap<String, String> = HashMap::new();
        let mut autofit: Option<Element> = None;
        for src in [ph_shapes.master.as_ref().and_then(|e| e.child(P, "txBody")), ph_shapes.layout.as_ref().and_then(|e| e.child(P, "txBody")), Some(tb)] {
            if let Some(bp) = src.and_then(|t| t.child(A, "bodyPr")) {
                for a in bp.attrs() {
                    body.insert(a.local().to_string(), a.value.clone());
                }
                if let Some(af) = bp.elements().find(|e| matches!(e.local(), "normAutofit" | "spAutoFit" | "noAutofit")) {
                    autofit = Some(af.clone());
                }
            }
        }
        let bget = |k: &str, d: f64| body.get(k).and_then(|v| v.parse::<f64>().ok()).map(|v| v / 12700.0).unwrap_or(d);
        let (li, ri, ti, bi) = (bget("lIns", 7.2), bget("rIns", 7.2), bget("tIns", 3.6), bget("bIns", 3.6));
        let anchor = match body.get("anchor").map(|s| s.as_str()) {
            Some("ctr") => "horizon",
            Some("b") => "bottom",
            _ => "top",
        };
        let wrap = body.get("wrap").map(|v| v != "none").unwrap_or(true);
        let (font_scale, ln_reduce) = match &autofit {
            Some(a) if a.local() == "normAutofit" => (
                attr(a, "fontScale").and_then(|v| v.parse::<f64>().ok()).map(|v| v / 100000.0).unwrap_or(1.0),
                attr(a, "lnSpcReduction").and_then(|v| v.parse::<f64>().ok()).map(|v| v / 100000.0).unwrap_or(0.0),
            ),
            _ => (1.0, 0.0),
        };
        let own_list = tb.child(A, "lstStyle");
        let mut out = String::new();
        let mut auto_counters: HashMap<usize, i64> = HashMap::new();
        let mut prev_aft = 0.0;
        for (pi, p) in paras.iter().enumerate() {
            let ppr = p.child(A, "pPr");
            let lvl: usize = ppr.and_then(|x| attr(x, "lvl")).and_then(|v| v.parse().ok()).unwrap_or(0);
            let mut dp = self.level_props(lvl, ph, inh, ph_shapes, own_list, ctx);
            if let Some(x) = ppr {
                dp.apply(x, ctx);
            }
            let size = dp.run.size.unwrap_or(18.0) * font_scale;
            let ea_name = self.font_name(&dp.run.ea, ctx, true);
            let stack = office::font_stack(self.fonts, &self.font_name(&dp.run.latin, ctx, false), Some(ea_name.as_str()).filter(|s| !s.is_empty()));
            let (asc, desc) = office::line_metrics(self.fonts, &stack);
            let natural = (asc + desc) * size;
            // Line spacing: a percentage of single (PowerPoint's single is the font's line), or
            // points.
            let (top_edge, bottom_edge, leading) = match &dp.lnspc {
                Some(Spacing::Pts(v)) => (pt(v * 0.8), pt(-v * 0.2), "0pt".to_string()),
                Some(Spacing::Pct(v)) => {
                    let v = (v - ln_reduce).max(0.1);
                    (format!("{}em", typ::num(asc)), format!("{}em", typ::num(-desc)), pt(natural * (v - 1.0)))
                }
                None => (format!("{}em", typ::num(asc)), format!("{}em", typ::num(-desc)), "0pt".to_string()),
            };
            let sp = |s: &Option<Spacing>| match s {
                Some(Spacing::Pts(v)) => *v * if font_scale < 1.0 { font_scale.max(0.8) } else { 1.0 },
                Some(Spacing::Pct(v)) => v * natural,
                None => 0.0,
            };
            let before = if pi == 0 { 0.0 } else { sp(&dp.spc_bef) };
            let gap = prev_aft + before;
            prev_aft = sp(&dp.spc_aft);
            let mar = dp.mar_l.unwrap_or(0.0);
            let ind = dp.indent.unwrap_or(0.0);
            let align = match dp.algn.as_deref() {
                Some("ctr") => "center",
                Some("r") => "right",
                Some("just") | Some("dist") | Some("justLow") | Some("thaiDist") => "justify",
                _ => "left",
            };
            // Runs.
            let mut content = String::new();
            let has_text = !crate::text::text_of(&crate::dml::Dml, p).is_empty();
            let mut first_run_props = dp.run.clone();
            let mut got_first = false;
            for r in p.elements() {
                match r.local() {
                    "r" | "fld" => {
                        let mut rp = dp.run.clone();
                        if let Some(x) = r.child(A, "rPr") {
                            rp.apply(x, ctx);
                        }
                        if !got_first {
                            first_run_props = rp.clone();
                            got_first = true;
                        }
                        let mut t = r.child(A, "t").map(|t| t.text()).unwrap_or_default();
                        if r.local() == "fld" {
                            match attr(r, "type") {
                                Some("slidenum") => t = (si + 1).to_string(),
                                Some(d) if d.starts_with("datetime") => {
                                    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
                                    let (y, m, dd) = crate::numfmt::civil_from_days(secs.div_euclid(86400));
                                    t = format!("{y}-{m:02}-{dd:02}");
                                }
                                _ => {}
                            }
                        }
                        let e = self.run_expr(&t, &rp, ctx, font_scale);
                        let e = match r.child(A, "rPr").and_then(|x| x.child(A, "hlinkClick")).and_then(|hl| hl.attr_ns(R, "id")).and_then(|id| _rels.get(id)).filter(|rel| rel.external) {
                            Some(rel) => format!("link({}, {e})", s(&rel.target)),
                            None => e,
                        };
                        let _ = write!(content, "#{e}");
                    }
                    "br" => content.push_str("#linebreak()"),
                    _ => {}
                }
            }
            if !has_text {
                let mut rp = dp.run.clone();
                if let Some(e) = p.child(A, "endParaRPr") {
                    rp.apply(e, ctx);
                }
                let e = self.run_expr("\u{200B}", &rp, ctx, font_scale);
                let _ = write!(content, "#{e}");
            }
            // The bullet.
            let mut bullet = String::new();
            if has_text {
                let counter = auto_counters.entry(lvl).or_insert(0);
                match &dp.bullet {
                    Some(Bullet::Char(c)) => {
                        let font = dp.bu_font.clone().unwrap_or_default();
                        let ch: String = if office::is_symbol_font(&font) { c.chars().map(|x| office::symbol_char(&font, x)).collect() } else { c.clone() };
                        bullet = self.bullet_expr(&ch, &dp, &first_run_props, ctx, font_scale, &font);
                        *counter = 0;
                    }
                    Some(Bullet::Auto(kind, start)) => {
                        *counter += 1;
                        let n = start + *counter - 1;
                        bullet = self.bullet_expr(&auto_number(kind, n), &dp, &first_run_props, ctx, font_scale, "");
                    }
                    _ => *counter = 0,
                }
            }
            // marL is where the text starts; indent (negative) moves the first line (and the
            // bullet) left of it.
            let (inset_l, first_indent, hanging) = if ind < 0.0 { (mar + ind, 0.0, -ind) } else { (mar, ind, 0.0) };
            let first_line = if bullet.is_empty() {
                content
            } else if hanging > 0.0 {
                format!("#context {{ let b = {bullet}; let bw = measure(b).width; box(width: calc.max({}, bw), b) }}{content}", pt(hanging))
            } else {
                format!("#{bullet}#h(0.3em){content}")
            };
            let _ = writeln!(
                out,
                "#block(above: {}, below: 0pt, width: 100%, inset: (left: {}))[#set par(justify: {}, leading: {leading}, first-line-indent: (amount: {}, all: true), hanging-indent: {})\n#set text(top-edge: {top_edge}, bottom-edge: {bottom_edge})\n#set align({})\n{first_line}]",
                pt(gap),
                pt(inset_l),
                align == "justify",
                pt(first_indent),
                pt(hanging),
                if align == "justify" { "left" } else { align }
            );
        }
        let inner_w = (w - li - ri).max(1.0);
        let body_box = if wrap {
            format!("block(width: {}, [{out}])", pt(inner_w))
        } else {
            // Unwrapped: as wide as the longest line (at least the box, for the alignment).
            format!("context {{ let c = [{out}]; box(width: calc.max(measure(c).width, {}), c) }}", pt(inner_w))
        };
        format!("#box(width: {}, height: {}, inset: (left: {}, right: {}, top: {}, bottom: {}), align({anchor}, {body_box}))", pt(w), pt(h.max(0.0)), pt(li), pt(ri), pt(ti), pt(bi))
    }

    fn font_name(&self, f: &Option<String>, ctx: &Ctx, ea: bool) -> String {
        match f.as_deref() {
            Some(t) if t.starts_with('+') => ctx.theme.font(t, if ea { Some("ko-KR") } else { None }).unwrap_or_default(),
            Some(t) => t.to_string(),
            None => if ea { ctx.theme.font("+mn-ea", Some("ko-KR")).unwrap_or_default() } else { ctx.theme.minor_latin.clone() },
        }
    }

    fn run_expr(&self, t: &str, rp: &DRun, ctx: &Ctx, scale: f64) -> String {
        let latin = self.font_name(&rp.latin, ctx, false);
        let ea = self.font_name(&rp.ea, ctx, true);
        let stack = office::font_stack(self.fonts, &latin, Some(ea.as_str()).filter(|s| !s.is_empty()));
        let mut text = t.to_string();
        if rp.cap.as_deref() == Some("all") {
            text = text.to_uppercase();
        }
        if office::is_symbol_font(&latin) {
            text = text.chars().map(|c| office::symbol_char(&latin, c)).collect();
        }
        let size = rp.size.unwrap_or(18.0) * scale;
        let mut args = format!("font: {}, size: {}", typ::fonts(&stack), pt(size));
        if rp.bold.unwrap_or(false) {
            args.push_str(", weight: \"bold\"");
        }
        if rp.italic.unwrap_or(false) {
            args.push_str(", style: \"italic\"");
        }
        let (hex, a) = rp.color.clone().unwrap_or_else(|| (ctx.theme.color(ctx.map.get("tx1").map(|s| s.as_str()).unwrap_or("dk1")).unwrap_or("000000".into()), 1.0));
        let _ = write!(args, ", fill: {}", if a < 1.0 { rgba(&hex, a) } else { rgb(&hex) });
        if let Some(sp) = rp.spc {
            if sp != 0.0 {
                let _ = write!(args, ", tracking: {}", pt(sp * scale));
            }
        }
        if t.chars().any(|c| matches!(c as u32, 0xAC00..=0xD7A3)) {
            args.push_str(", lang: \"ko\"");
        }
        let mut e = format!("text({args}, {})", s(&text));
        if let Some(b) = rp.baseline {
            if b > 0.0 {
                e = format!("super(typographic: false, size: 0.67em, baseline: -0.33em, {e})");
            } else if b < 0.0 {
                e = format!("sub(typographic: false, size: 0.67em, baseline: 0.12em, {e})");
            }
        }
        if rp.cap.as_deref() == Some("small") {
            e = format!("smallcaps({e})");
        }
        if let Some(u) = &rp.underline {
            if u != "none" {
                e = format!("underline(evade: false, {e})");
                if u == "dbl" {
                    e = format!("underline(offset: 0.2em, evade: false, {e})");
                }
            }
        }
        if let Some(st) = &rp.strike {
            if st != "noStrike" {
                e = format!("strike({e})");
            }
        }
        if let Some(h) = &rp.highlight {
            e = format!("highlight(fill: {}, {e})", rgb(h));
        }
        e
    }

    fn bullet_expr(&self, ch: &str, dp: &DPara, first: &DRun, ctx: &Ctx, scale: f64, font: &str) -> String {
        let mut rp = first.clone();
        rp.underline = None;
        rp.strike = None;
        rp.highlight = None;
        if let Some(c) = &dp.bu_color {
            rp.color = Some(c.clone());
        }
        if let Some(sz) = dp.bu_size {
            rp.size = Some(first.size.or(dp.run.size).unwrap_or(18.0) * sz);
        }
        if !font.is_empty() && !office::is_symbol_font(font) {
            rp.latin = Some(font.to_string());
        }
        self.run_expr(ch, &rp, ctx, scale)
    }

    // ── tables ──────────────────────────────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn table(&mut self, tbl: &Element, ctx: &Ctx, _w: f64, si: usize, inh: &Inherits, rels: &Rels, part: &str) -> String {
        let cols: Vec<f64> = tbl.child(A, "tblGrid").map(|g| g.children_named(A, "gridCol").map(|c| emu(attr(c, "w"))).collect()).unwrap_or_default();
        let tpr = tbl.child(A, "tblPr");
        let flag = |k: &str| tpr.and_then(|t| attr(t, k)).map(|v| v == "1" || v == "true").unwrap_or(false);
        let (first_row, band_row, first_col, last_row) = (flag("firstRow"), flag("bandRow"), flag("firstCol"), flag("lastRow"));
        // The table style: Office's default look (Medium Style 2, accent 1) when it is the
        // built-in one, which files do not define.
        let accent = ctx.theme.color("accent1").unwrap_or("4472C4".into());
        let style_id = tpr.and_then(|t| t.child(A, "tableStyleId")).map(|t| t.text()).unwrap_or_default();
        let styled = !style_id.is_empty() || tpr.is_some();
        let rows: Vec<&Element> = tbl.children_named(A, "tr").collect();
        let nrows = rows.len();
        let mut body = String::new();
        for (ri, tr) in rows.iter().enumerate() {
            let mut ci = 0;
            for tc in tr.children_named(A, "tc") {
                let gs: usize = attr(tc, "gridSpan").and_then(|v| v.parse().ok()).unwrap_or(1);
                let rs: usize = attr(tc, "rowSpan").and_then(|v| v.parse().ok()).unwrap_or(1);
                if attr(tc, "hMerge").is_some() || attr(tc, "vMerge").is_some() {
                    ci += 1;
                    continue;
                }
                let header = first_row && ri == 0;
                let footer = last_row && ri + 1 == nrows;
                let band = band_row && !header && ((ri - if first_row { 1 } else { 0 }) % 2 == 0);
                let mut fill: Option<String> = if styled && style_id.contains("5C22544A") || styled && style_id.is_empty() {
                    if header {
                        Some(rgb(&accent))
                    } else if band {
                        Some(rgb(&office::transform(&accent, &[("tint".into(), 40000.0)]).0))
                    } else {
                        Some(rgb(&office::transform(&accent, &[("tint".into(), 20000.0)]).0))
                    }
                } else {
                    None
                };
                let tcpr = tc.child(A, "tcPr");
                if let Some(f) = tcpr.and_then(|p| p.elements().find(|e| matches!(e.local(), "solidFill" | "noFill" | "gradFill"))) {
                    fill = shapes::paint_of_fill(f, ctx.theme, ctx.map, None);
                }
                let m = |k: &str, d: f64| tcpr.and_then(|p| attr(p, k)).map(|v| emu(Some(v))).unwrap_or(d);
                let inset = format!("(left: {}, right: {}, top: {}, bottom: {})", pt(m("marL", 7.2)), pt(m("marR", 7.2)), pt(m("marT", 3.6)), pt(m("marB", 3.6)));
                let anchor = match tcpr.and_then(|p| attr(p, "anchor")) {
                    Some("ctr") => "horizon",
                    Some("b") => "bottom",
                    _ => "top",
                };
                let border = |k: &str| -> String {
                    match tcpr.and_then(|p| p.child(A, k)) {
                        Some(ln) => {
                            let w = attr(ln, "w").map(|v| emu(Some(v))).unwrap_or(1.0);
                            match ln.elements().find(|e| matches!(e.local(), "solidFill" | "noFill")) {
                                Some(f) if f.local() == "solidFill" => office::dml_color(f, ctx.theme, ctx.map, None).map(|(h, _)| format!("{} + {}", pt(w), rgb(&h))).unwrap_or("none".into()),
                                _ => "none".into(),
                            }
                        }
                        None => if styled { "1pt + white".into() } else { "none".into() },
                    }
                };
                let stroke = format!("(left: {}, right: {}, top: {}, bottom: {})", border("lnL"), border("lnR"), border("lnT"), border("lnB"));
                // Text: header rows white and bold in the default style.
                let mut text = String::new();
                if let Some(tb) = tc.child(A, "txBody") {
                    let mut tbx = tb.clone();
                    if (header || footer || (first_col && ci == 0)) && (styled && (style_id.contains("5C22544A") || style_id.is_empty())) {
                        for p in tbx.elements_mut() {
                            if !p.is(A, "p") {
                                continue;
                            }
                            for r in p.elements_mut() {
                                if r.is(A, "r") {
                                    if r.child(A, "rPr").is_none() {
                                        r.children.insert(0, crate::xml::Node::Element(Element::new(&r.qualify("rPr"), A)));
                                    }
                                    let rpr = r.child_mut(A, "rPr").unwrap();
                                    if attr(rpr, "b").is_none() {
                                        rpr.set_attr("b", NONE, "1");
                                    }
                                    if header && rpr.child(A, "solidFill").is_none() {
                                        rpr.children.insert(0, crate::xml::Node::Element(shapes_white(rpr)));
                                    }
                                }
                            }
                        }
                    }
                    let cw: f64 = cols.get(ci..(ci + gs).min(cols.len())).map(|x| x.iter().sum()).unwrap_or(72.0);
                    let lst = tbl.clone();
                    let _ = lst;
                    let none = Inherit { layout: None, master: None };
                    let mut list_inh = Inherits { layout: None, master: None, pres: inh.pres.clone() };
                    if let Some(m) = &inh.master {
                        list_inh.master = Some(m.clone());
                    }
                    text = self.text_body(&tbx, ctx, cw, 0.0, None, &list_inh, &none, si, rels, part);
                    // Cells grow with their text: no fixed height.
                    text = text.replacen(", height: 0pt,", ",", 1).replace(", inset: (left: 7.2pt, right: 7.2pt, top: 3.6pt, bottom: 3.6pt)", ", inset: 0pt");
                }
                let mut args = format!("colspan: {gs}, rowspan: {rs}, inset: {inset}, align: {anchor}, stroke: {stroke}");
                if let Some(f) = fill {
                    let _ = write!(args, ", fill: {f}");
                }
                let _ = write!(body, "table.cell({args})[{text}], ");
                ci += gs;
            }
        }
        let heights: Vec<String> = rows.iter().map(|r| attr(r, "h").map(|v| pt(emu(Some(v)))).unwrap_or("auto".into())).collect();
        let _ = heights;
        format!("table(columns: ({}), {body})", cols.iter().map(|c| pt(*c)).collect::<Vec<_>>().join(", "))
    }
}

fn shapes_white(like: &Element) -> Element {
    crate::dml::solid_fill(like, "FFFFFF")
}

struct Inherits {
    layout: Option<Document>,
    master: Option<Document>,
    pres: Option<Element>,
}

impl Inherits {
    fn none() -> Inherits {
        Inherits { layout: None, master: None, pres: None }
    }

    /// The layout's and master's placeholders a slide placeholder takes after.
    fn find(&self, ph: Option<&Element>) -> Inherit {
        let Some(ph) = ph else { return Inherit { layout: None, master: None } };
        let t = attr(ph, "type").unwrap_or("body");
        let idx = attr(ph, "idx").unwrap_or("");
        let find_in = |d: &Document, by_idx: bool| -> Option<Element> {
            let tree = d.root.child(P, "cSld")?.child(P, "spTree")?;
            let mut by_type = None;
            for e in tree.elements() {
                let Some(p) = e.elements().find(|x| x.local().starts_with("nv")).and_then(|v| v.child(P, "nvPr")).and_then(|v| v.child(P, "ph")) else { continue };
                let pt_ = attr(p, "type").unwrap_or("body");
                let pidx = attr(p, "idx").unwrap_or("");
                if by_idx && !idx.is_empty() && pidx == idx {
                    return Some(e.clone());
                }
                let same = pt_ == t || (t == "ctrTitle" && pt_ == "title") || (t == "title" && pt_ == "ctrTitle") || (t == "subTitle" && pt_ == "body") || (t == "obj" && pt_ == "body") || (t == "body" && pt_ == "obj");
                if same && by_type.is_none() {
                    by_type = Some(e.clone());
                }
            }
            by_type
        };
        let layout = self.layout.as_ref().and_then(|d| find_in(d, true));
        // The master matches by type (bodies by body).
        let master = self.master.as_ref().and_then(|d| find_in(d, false));
        Inherit { layout, master }
    }
}

fn auto_number(kind: &str, n: i64) -> String {
    let (fmt, wrap): (&str, &str) = match kind {
        "arabicPeriod" => ("decimal", "{}."),
        "arabicParenR" => ("decimal", "{})"),
        "arabicParenBoth" => ("decimal", "({})"),
        "arabicPlain" => ("decimal", "{}"),
        "romanUcPeriod" => ("upperRoman", "{}."),
        "romanLcPeriod" => ("lowerRoman", "{}."),
        "romanLcParenR" => ("lowerRoman", "{})"),
        "alphaUcPeriod" => ("upperLetter", "{}."),
        "alphaLcPeriod" => ("lowerLetter", "{}."),
        "alphaLcParenR" => ("lowerLetter", "{})"),
        "alphaUcParenR" => ("upperLetter", "{})"),
        "alphaLcParenBoth" => ("lowerLetter", "({})"),
        "circleNumDbPlain" | "circleNumWdBlackPlain" | "circleNumWdWhitePlain" => ("decimalEnclosedCircle", "{}"),
        "hangulAlphaPeriod" | "hangulKoreanPeriod" => ("ganada", "{}."),
        _ => ("decimal", "{}."),
    };
    wrap.replace("{}", &super::docx::format_number(n, fmt))
}

fn arrowheads(ln: &Element, w: f64, h: f64, stroke: &str) -> String {
    let paint = stroke.split("paint: ").nth(1).and_then(|p| p.split(", thickness").next()).unwrap_or("black").to_string();
    let thick: f64 = stroke.split("thickness: ").nth(1).and_then(|t| t.trim_end_matches(')').trim_end_matches("pt").parse().ok()).unwrap_or(1.0);
    let size = (thick * 3.0).max(4.0);
    let mut out = String::new();
    let head = |tip: (f64, f64), from: (f64, f64)| -> String {
        let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
        let len = (dx * dx + dy * dy).sqrt().max(0.001);
        let (ux, uy) = (dx / len, dy / len);
        let base = (tip.0 - ux * size, tip.1 - uy * size);
        let (px, py) = (-uy * size * 0.5, ux * size * 0.5);
        format!("#place(top + left, polygon(fill: {paint}, stroke: none, ({}, {}), ({}, {}), ({}, {})))", pt(tip.0), pt(tip.1), pt(base.0 + px), pt(base.1 + py), pt(base.0 - px), pt(base.1 - py))
    };
    if ln.child(A, "tailEnd").and_then(|e| attr(e, "type")).map(|t| t != "none").unwrap_or(false) {
        out.push_str(&head((w, h), (0.0, 0.0)));
    }
    if ln.child(A, "headEnd").and_then(|e| attr(e, "type")).map(|t| t != "none").unwrap_or(false) {
        out.push_str(&head((0.0, 0.0), (w, h)));
    }
    out
}

/// SmartArt's saved drawing names its elements in the `dsp` namespace: read as `p` shapes.
fn rename_dsp(tree: &Element) -> Element {
    let mut t = tree.clone();
    fn walk(e: &mut Element) {
        let local = e.local().to_string();
        if e.ns() != A && e.ns() != R && !e.name().starts_with("a:") {
            let mapped = match local.as_str() {
                "spTree" | "sp" | "grpSp" | "spPr" | "grpSpPr" | "txBody" | "style" | "nvSpPr" | "nvGrpSpPr" | "cNvPr" | "cNvSpPr" | "nvPr" | "txXfrm" => true,
                _ => false,
            };
            if mapped {
                let mut n = Element::new(&format!("p:{local}"), P);
                for a in e.attrs() {
                    n.set_attr(&a.name, a.ns, &a.value);
                }
                n.children = std::mem::take(&mut e.children);
                *e = n;
            }
        }
        for c in e.elements_mut() {
            walk(c);
        }
    }
    walk(&mut t);
    t
}

