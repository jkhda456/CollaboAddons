//! DrawingML text (`a:p`, `a:r`, `a:t`, `a:br`, `a:fld`) as slides, charts and drawings hold
//! it, and its run, paragraph and shape properties.

use crate::props::{Color, LineSpacing, Props, Vert};
use crate::text::Dialect;
use crate::xml::{put_in_order, Element, Node, A};

pub struct Dml;

impl Dialect for Dml {
    fn is_run(&self, e: &Element) -> bool {
        e.is(A, "r") || e.is(A, "fld")
    }
    fn is_text(&self, e: &Element) -> bool {
        e.is(A, "t")
    }
    fn atom(&self, _e: &Element) -> Option<(char, Option<String>)> {
        None
    }
    fn run_atom(&self, e: &Element) -> Option<char> {
        e.is(A, "br").then_some('\n')
    }
    fn descend(&self, _e: &Element) -> bool {
        false
    }
    fn is_props(&self, e: &Element) -> bool {
        e.is(A, "rPr")
    }
    fn unsplittable(&self, run: &Element) -> bool {
        run.is(A, "fld")
    }
    fn new_text(&self, run: &Element, text: &str) -> Element {
        let mut t = Element::new(&run.qualify("t"), A);
        t.set_text(text);
        t
    }
    fn new_run(&self, para: &Element, _text: &str) -> Element {
        let mut r = Element::new(&para.qualify("r"), A);
        // The properties of the nearest run, else the paragraph end's.
        let rpr = para
            .elements()
            .find(|e| e.is(A, "r"))
            .and_then(|r| r.child(A, "rPr"))
            .cloned()
            .or_else(|| {
                para.child(A, "endParaRPr").map(|e| {
                    let mut c = e.clone();
                    let mut n = Element::new(&para.qualify("rPr"), A);
                    for a in c.attrs().to_vec() {
                        n.set_attr(&a.name, a.ns, &a.value);
                    }
                    n.children = std::mem::take(&mut c.children);
                    n
                })
            });
        if let Some(mut p) = rpr {
            p.remove_attr(crate::xml::NONE, "err");
            r.push(p);
        }
        r
    }
    fn new_run_at(&self, para: &Element) -> usize {
        para.position(A, "endParaRPr").unwrap_or(para.children.len())
    }
    fn settle(&self, parent: &mut Element, run: usize, text_el: usize) {
        let Some(r) = parent.children[run].as_element() else { return };
        let Some(t) = r.children.get(text_el).and_then(|n| n.as_element()) else { return };
        let s = t.text();
        if !s.contains(['\n', '\u{b}']) {
            return;
        }
        // A line break is a run-level a:br: the run (rPr and one t) splits around it.
        let run_el = r.clone();
        let mut new_nodes: Vec<Node> = Vec::new();
        for (i, piece) in s.split(['\n', '\u{b}']).enumerate() {
            if i > 0 {
                let mut br = Element::new(&run_el.qualify("br"), A);
                if let Some(rpr) = run_el.child(A, "rPr") {
                    br.push(rpr.clone());
                }
                new_nodes.push(Node::Element(br));
            }
            if !piece.is_empty() {
                let mut nr = run_el.clone();
                if let Some(t) = nr.children.get_mut(text_el).and_then(|n| n.as_element_mut()) {
                    t.set_text(piece);
                }
                new_nodes.push(Node::Element(nr));
            }
        }
        parent.children.splice(run..=run, new_nodes);
    }
}

pub const RPR_ORDER: &[&str] = &[
    "ln", "noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill", "effectLst", "effectDag", "highlight", "uLnTx", "uLn", "uFillTx", "uFill",
    "latin", "ea", "cs", "sym", "hlinkClick", "hlinkMouseOver", "rtl", "extLst",
];
pub const PPR_ORDER: &[&str] = &[
    "lnSpc", "spcBef", "spcAft", "buClrTx", "buClr", "buSzTx", "buSzPct", "buSzPts", "buFontTx", "buFont", "buNone", "buAutoNum", "buChar", "buBlip",
    "tabLst", "defRPr", "extLst",
];
pub const SPPR_ORDER: &[&str] = &["xfrm", "custGeom", "prstGeom", "noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill", "ln", "effectLst", "effectDag", "scene3d", "sp3d", "extLst"];
const FILLS: &[&str] = &["noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill"];

pub fn solid_fill(like: &Element, hex: &str) -> Element {
    let mut f = Element::new(&like.qualify("solidFill"), A);
    let mut c = Element::new(&like.qualify("srgbClr"), A);
    c.set_attr("val", crate::xml::NONE, hex);
    f.push(c);
    f
}

fn set_fill(e: &mut Element, c: &Color, order: &[&str]) {
    for f in FILLS {
        e.remove_children(A, f);
    }
    match c {
        Color::Rgb(h) => {
            let f = solid_fill(e, h);
            put_in_order(e, f, order);
        }
        Color::None => {
            let f = Element::new(&e.qualify("noFill"), A);
            put_in_order(e, f, order);
        }
        Color::Auto => {}
    }
}

fn a_child(like: &Element, local: &str) -> Element {
    Element::new(&like.qualify(local), A)
}

/// Run properties (`a:rPr`, `a:defRPr`, `a:endParaRPr`).
pub fn apply_rpr(rpr: &mut Element, p: &Props) {
    let n = crate::xml::NONE;
    if p.clear {
        let keep: Vec<_> = rpr.attrs().iter().filter(|a| matches!(a.local(), "lang" | "altLang" | "dirty")).cloned().collect();
        let mut fresh = a_child(rpr, rpr.local());
        for a in keep {
            fresh.set_attr(&a.name, a.ns, &a.value);
        }
        *rpr = fresh;
    }
    let flag = |rpr: &mut Element, name: &str, v: Option<bool>| {
        if let Some(on) = v {
            rpr.set_attr(name, n, if on { "1" } else { "0" });
        }
    };
    flag(rpr, "b", p.bold);
    flag(rpr, "i", p.italic);
    if let Some(u) = &p.underline {
        let v = match u.as_str() {
            "none" => "none",
            "double" => "dbl",
            "thick" | "bold" => "heavy",
            "dotted" | "dot" => "dotted",
            "dash" | "dashed" => "dash",
            "wave" | "wavy" => "wavy",
            "words" => "words",
            _ => "sng",
        };
        rpr.set_attr("u", n, v);
    }
    if let Some(s) = p.strike {
        rpr.set_attr("strike", n, if s { "sngStrike" } else { "noStrike" });
    }
    if let Some(s) = p.dstrike {
        rpr.set_attr("strike", n, if s { "dblStrike" } else { "noStrike" });
    }
    if let Some(sz) = p.size {
        rpr.set_attr("sz", n, &((sz * 100.0).round() as i64).to_string());
    }
    if let Some(c) = p.caps {
        rpr.set_attr("cap", n, if c { "all" } else { "none" });
    }
    if let Some(c) = p.small_caps {
        rpr.set_attr("cap", n, if c { "small" } else { "none" });
    }
    if let Some(v) = &p.vert {
        match v {
            Vert::Super => rpr.set_attr("baseline", n, "30000"),
            Vert::Sub => rpr.set_attr("baseline", n, "-25000"),
            Vert::Baseline => {
                rpr.remove_attr(n, "baseline");
            }
        }
    }
    if let Some(s) = p.letter_spacing {
        rpr.set_attr("spc", n, &((s * 100.0).round() as i64).to_string());
    }
    if let Some(l) = &p.lang {
        let low = l.to_ascii_lowercase();
        if low.starts_with("ko") || low.starts_with("ja") || low.starts_with("zh") {
            rpr.set_attr("altLang", n, "en-US");
        }
        rpr.set_attr("lang", n, l);
    }
    if let Some(c) = &p.color {
        set_fill(rpr, c, RPR_ORDER);
    }
    if let Some(h) = &p.highlight {
        rpr.remove_children(A, "highlight");
        if let Color::Rgb(hex) = h {
            let mut e = a_child(rpr, "highlight");
            let mut c = a_child(rpr, "srgbClr");
            c.set_attr("val", n, hex);
            e.push(c);
            put_in_order(rpr, e, RPR_ORDER);
        }
    }
    if let Some(c) = &p.shade {
        // DrawingML text has no shading: a highlight is the closest.
        rpr.remove_children(A, "highlight");
        if let Color::Rgb(hex) = c {
            let mut e = a_child(rpr, "highlight");
            let mut cc = a_child(rpr, "srgbClr");
            cc.set_attr("val", n, hex);
            e.push(cc);
            put_in_order(rpr, e, RPR_ORDER);
        }
    }
    let fonts = [(&p.font, &["latin", "ea", "cs"][..]), (&p.font_latin, &["latin"][..]), (&p.font_ea, &["ea"][..]), (&p.font_cs, &["cs"][..])];
    for (font, which) in fonts {
        if let Some(f) = font {
            for w in which {
                let mut e = rpr.child(A, w).cloned().unwrap_or_else(|| a_child(rpr, w));
                e.set_attr("typeface", n, f);
                e.remove_attr(n, "panose");
                e.remove_attr(n, "pitchFamily");
                e.remove_attr(n, "charset");
                put_in_order(rpr, e, RPR_ORDER);
            }
        }
    }
}

/// Paragraph properties (`a:pPr`).
pub fn apply_ppr(ppr: &mut Element, p: &Props) {
    let n = crate::xml::NONE;
    if let Some(a) = &p.align {
        ppr.set_attr("algn", n, match a.as_str() {
            "center" => "ctr",
            "right" => "r",
            "justify" => "just",
            "distribute" => "dist",
            _ => "l",
        });
    }
    if let Some(l) = p.level {
        ppr.set_attr("lvl", n, &(l.saturating_sub(1).min(8)).to_string());
    }
    if let Some(v) = p.indent_left {
        ppr.set_attr("marL", n, &emu(v).to_string());
    }
    if let Some(v) = p.indent_right {
        ppr.set_attr("marR", n, &emu(v).to_string());
    }
    if let Some(v) = p.indent_first {
        ppr.set_attr("indent", n, &emu(v).to_string());
    }
    if let Some(v) = p.indent_hanging {
        ppr.set_attr("indent", n, &(-emu(v)).to_string());
    }
    let spacing = |ppr: &mut Element, name: &str, pt: f64| {
        let mut e = a_child(ppr, name);
        let mut s = a_child(ppr, "spcPts");
        s.set_attr("val", n, &((pt * 100.0).round() as i64).to_string());
        e.push(s);
        put_in_order(ppr, e, PPR_ORDER);
    };
    if let Some(b) = p.space_before {
        spacing(ppr, "spcBef", b);
    }
    if let Some(a) = p.space_after {
        spacing(ppr, "spcAft", a);
    }
    if let Some(l) = &p.line {
        let mut e = a_child(ppr, "lnSpc");
        match l {
            LineSpacing::Multiple(m) => {
                let mut s = a_child(ppr, "spcPct");
                s.set_attr("val", n, &((m * 100000.0).round() as i64).to_string());
                e.push(s);
            }
            LineSpacing::Exact(pt) | LineSpacing::AtLeast(pt) => {
                let mut s = a_child(ppr, "spcPts");
                s.set_attr("val", n, &((pt * 100.0).round() as i64).to_string());
                e.push(s);
            }
        }
        put_in_order(ppr, e, PPR_ORDER);
    }
    if let Some(b) = &p.bullet {
        for x in ["buNone", "buAutoNum", "buChar", "buBlip"] {
            ppr.remove_children(A, x);
        }
        match b.as_str() {
            "none" | "" => {
                let e = a_child(ppr, "buNone");
                put_in_order(ppr, e, PPR_ORDER);
            }
            "number" | "1" | "1." | "arabic" => {
                let mut e = a_child(ppr, "buAutoNum");
                e.set_attr("type", n, "arabicPeriod");
                put_in_order(ppr, e, PPR_ORDER);
            }
            c => {
                let mut e = a_child(ppr, "buChar");
                e.set_attr("char", n, c);
                put_in_order(ppr, e, PPR_ORDER);
            }
        }
    }
    if let Some(l) = &p.list {
        for x in ["buNone", "buAutoNum", "buChar", "buBlip"] {
            ppr.remove_children(A, x);
        }
        let e = match l.as_str() {
            "none" => a_child(ppr, "buNone"),
            "number" | "numbered" => {
                let mut e = a_child(ppr, "buAutoNum");
                e.set_attr("type", n, "arabicPeriod");
                e
            }
            _ => {
                let mut e = a_child(ppr, "buChar");
                e.set_attr("char", n, "•");
                e
            }
        };
        put_in_order(ppr, e, PPR_ORDER);
    }
}

pub fn emu(pt: f64) -> i64 {
    (pt * 12700.0).round() as i64
}

/// A summary of a run's direct properties, for `read --runs`.
pub fn rpr_summary(rpr: Option<&Element>) -> Vec<String> {
    let mut v = Vec::new();
    let Some(r) = rpr else { return v };
    let n = crate::xml::NONE;
    if r.attr_ns(n, "b") == Some("1") {
        v.push("bold".into());
    }
    if r.attr_ns(n, "i") == Some("1") {
        v.push("italic".into());
    }
    if let Some(u) = r.attr_ns(n, "u") {
        if u != "none" {
            v.push(format!("underline={u}"));
        }
    }
    if let Some(s) = r.attr_ns(n, "strike") {
        if s != "noStrike" {
            v.push("strike".into());
        }
    }
    if let Some(sz) = r.attr_ns(n, "sz").and_then(|s| s.parse::<f64>().ok()) {
        v.push(format!("size={}pt", sz / 100.0));
    }
    if let Some(b) = r.attr_ns(n, "baseline").and_then(|s| s.parse::<i64>().ok()) {
        v.push(if b > 0 { "super".into() } else if b < 0 { "sub".into() } else { "baseline".into() });
    }
    if let Some(f) = r.child(A, "solidFill") {
        if let Some(c) = f.child(A, "srgbClr").and_then(|c| c.attr_ns(n, "val")) {
            v.push(format!("color={c}"));
        } else if let Some(c) = f.child(A, "schemeClr").and_then(|c| c.attr_ns(n, "val")) {
            v.push(format!("color=scheme:{c}"));
        }
    }
    if let Some(h) = r.child(A, "highlight").and_then(|h| h.child(A, "srgbClr")).and_then(|c| c.attr_ns(n, "val")) {
        v.push(format!("highlight={h}"));
    }
    if let Some(f) = r.child(A, "latin").and_then(|f| f.attr_ns(n, "typeface")) {
        v.push(format!("font={f}"));
    }
    if let Some(f) = r.child(A, "ea").and_then(|f| f.attr_ns(n, "typeface")) {
        v.push(format!("font.ea={f}"));
    }
    if r.child(A, "hlinkClick").is_some() {
        v.push("link".into());
    }
    v
}
