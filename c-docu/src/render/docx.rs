//! Word documents as Typst: sections become page settings (size, margins, columns, headers and
//! footers), paragraphs blocks with Word's spacing and line heights, runs text with their fonts,
//! tables grids with merged cells, borders and shading, drawings images and boxes placed where
//! Word anchors them. Styles resolve as Word resolves them: document defaults, the table style,
//! the paragraph style chain, numbering, the character style chain, direct formatting.

use super::office::{self, Theme};
use super::typ::{self, pt, rgb, s};
use super::world::Fonts;
use super::Job;
use crate::docx::Docx;
use crate::opc::{self, Rels};
use crate::xml::{Element, Node, A, MC, NONE, PIC, R, V, W, WP, WPG, WPS};
use crate::Result;
use std::collections::HashMap;
use std::fmt::Write;

fn twip(v: Option<&str>) -> Option<f64> {
    v.and_then(|s| s.trim().parse::<f64>().ok()).map(|t| t / 20.0)
}

fn on(e: &Element) -> bool {
    !matches!(e.val(), Some("0") | Some("false") | Some("off") | Some("none"))
}

#[derive(Clone, Default, Debug)]
pub struct RP {
    ascii: Option<String>,
    ea: Option<String>,
    cs: Option<String>,
    ascii_theme: Option<String>,
    ea_theme: Option<String>,
    size: Option<f64>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<(String, Option<String>)>,
    strike: Option<bool>,
    dstrike: Option<bool>,
    color: Option<String>,
    highlight: Option<String>,
    shade: Option<String>,
    caps: Option<bool>,
    small_caps: Option<bool>,
    vert: Option<String>,
    hidden: Option<bool>,
    spacing: Option<f64>,
    kern: Option<f64>,
    position: Option<f64>,
    lang: Option<String>,
    lang_ea: Option<String>,
}

impl RP {
    fn apply(&mut self, rpr: &Element, theme: &Theme) {
        for e in rpr.elements() {
            if e.ns() != W {
                continue;
            }
            match e.local() {
                "rFonts" => {
                    if let Some(v) = e.own("ascii").or_else(|| e.own("hAnsi")) {
                        self.ascii = Some(v.to_string());
                        self.ascii_theme = None;
                    }
                    if let Some(v) = e.own("asciiTheme").or_else(|| e.own("hAnsiTheme")) {
                        self.ascii_theme = Some(v.to_string());
                    }
                    if let Some(v) = e.own("eastAsia") {
                        self.ea = Some(v.to_string());
                        self.ea_theme = None;
                    }
                    if let Some(v) = e.own("eastAsiaTheme") {
                        self.ea_theme = Some(v.to_string());
                    }
                    if let Some(v) = e.own("cs") {
                        self.cs = Some(v.to_string());
                    }
                }
                "b" => self.bold = Some(on(e)),
                "i" => self.italic = Some(on(e)),
                "caps" => self.caps = Some(on(e)),
                "smallCaps" => self.small_caps = Some(on(e)),
                "strike" => self.strike = Some(on(e)),
                "dstrike" => self.dstrike = Some(on(e)),
                "vanish" => self.hidden = Some(on(e)),
                "specVanish" => {}
                "color" => {
                    let mut c = e.val().map(|v| v.to_ascii_uppercase());
                    if let Some(t) = e.own("themeColor") {
                        if let Some(h) = theme.color(t) {
                            c = Some(office::wml_tint_shade(&h, e.own("themeTint"), e.own("themeShade")));
                        }
                    }
                    self.color = c;
                }
                "sz" => self.size = e.val().and_then(|v| v.parse::<f64>().ok()).map(|h| h / 2.0),
                "highlight" => self.highlight = e.val().filter(|v| *v != "none").and_then(crate::docx::highlight_rgb).map(str::to_string).or(if e.val() == Some("none") { Some(String::new()) } else { None }),
                "u" => {
                    let v = e.val().unwrap_or("single").to_string();
                    self.underline = Some((v, e.own("color").filter(|c| *c != "auto").map(|c| c.to_ascii_uppercase())));
                }
                "shd" => {
                    if let Some(f) = e.own("fill").filter(|f| *f != "auto") {
                        self.shade = Some(f.to_ascii_uppercase());
                    } else if let Some(t) = e.own("themeFill").and_then(|t| theme.color(t)) {
                        self.shade = Some(office::wml_tint_shade(&t, e.own("themeFillTint"), e.own("themeFillShade")));
                    }
                }
                "vertAlign" => self.vert = e.val().map(str::to_string),
                "spacing" => self.spacing = twip(e.val()),
                "kern" => self.kern = e.val().and_then(|v| v.parse::<f64>().ok()).map(|h| h / 2.0),
                "position" => self.position = e.val().and_then(|v| v.parse::<f64>().ok()).map(|h| h / 2.0),
                "lang" => {
                    if let Some(v) = e.val() {
                        self.lang = Some(v.to_string());
                    }
                    if let Some(v) = e.own("eastAsia") {
                        self.lang_ea = Some(v.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    fn latin(&self, theme: &Theme) -> String {
        if let Some(t) = &self.ascii_theme {
            if let Some(f) = theme.font(t, self.lang.as_deref()) {
                return f;
            }
        }
        self.ascii.clone().unwrap_or_else(|| "Times New Roman".into())
    }

    fn east_asian(&self, theme: &Theme) -> Option<String> {
        if let Some(t) = &self.ea_theme {
            if let Some(f) = theme.font(t, self.lang_ea.as_deref()) {
                return Some(f);
            }
        }
        self.ea.clone()
    }
}

#[derive(Clone, Default, Debug)]
pub struct PP {
    jc: Option<String>,
    left: Option<f64>,
    right: Option<f64>,
    first: Option<f64>,
    before: Option<f64>,
    after: Option<f64>,
    before_auto: bool,
    after_auto: bool,
    line: Option<f64>,
    line_rule: Option<String>,
    keep_next: Option<bool>,
    keep_lines: Option<bool>,
    page_break_before: Option<bool>,
    contextual: Option<bool>,
    tabs: Vec<(f64, String, Option<String>)>,
    num: Option<(String, u32)>,
    shade: Option<String>,
    borders: Vec<(String, f64, String)>,
    style: Option<String>,
    bidi: Option<bool>,
}

impl PP {
    fn apply(&mut self, ppr: &Element, theme: &Theme) {
        for e in ppr.elements() {
            if e.ns() != W {
                continue;
            }
            match e.local() {
                "pStyle" => self.style = e.val().map(str::to_string),
                "jc" => self.jc = e.val().map(str::to_string),
                "ind" => {
                    if let Some(v) = twip(e.own("left").or_else(|| e.own("start"))) {
                        self.left = Some(v);
                    }
                    if let Some(v) = twip(e.own("right").or_else(|| e.own("end"))) {
                        self.right = Some(v);
                    }
                    if let Some(v) = twip(e.own("firstLine")) {
                        self.first = Some(v);
                    }
                    if let Some(v) = twip(e.own("hanging")) {
                        self.first = Some(-v);
                    }
                    // Character units (100ths of a character, about the font size): approximate
                    // with 10.5pt characters.
                    if let Some(v) = e.own("leftChars").or_else(|| e.own("startChars")).and_then(|v| v.parse::<f64>().ok()) {
                        if v != 0.0 {
                            self.left = Some(v / 100.0 * 10.5);
                        }
                    }
                    if let Some(v) = e.own("firstLineChars").and_then(|v| v.parse::<f64>().ok()) {
                        if v != 0.0 {
                            self.first = Some(v / 100.0 * 10.5);
                        }
                    }
                    if let Some(v) = e.own("hangingChars").and_then(|v| v.parse::<f64>().ok()) {
                        if v != 0.0 {
                            self.first = Some(-v / 100.0 * 10.5);
                        }
                    }
                }
                "spacing" => {
                    if let Some(v) = twip(e.own("before")) {
                        self.before = Some(v);
                        self.before_auto = false;
                    }
                    if let Some(v) = twip(e.own("after")) {
                        self.after = Some(v);
                        self.after_auto = false;
                    }
                    if let Some(v) = e.own("beforeLines").and_then(|v| v.parse::<f64>().ok()) {
                        self.before = Some(v / 100.0 * 12.0);
                    }
                    if let Some(v) = e.own("afterLines").and_then(|v| v.parse::<f64>().ok()) {
                        self.after = Some(v / 100.0 * 12.0);
                    }
                    if e.own("beforeAutospacing").map(|v| v == "1" || v == "true").unwrap_or(false) {
                        self.before_auto = true;
                    }
                    if e.own("afterAutospacing").map(|v| v == "1" || v == "true").unwrap_or(false) {
                        self.after_auto = true;
                    }
                    if let Some(v) = e.own("line").and_then(|v| v.parse::<f64>().ok()) {
                        self.line = Some(v);
                        self.line_rule = Some(e.own("lineRule").unwrap_or("auto").to_string());
                    }
                }
                "keepNext" => self.keep_next = Some(on(e)),
                "keepLines" => self.keep_lines = Some(on(e)),
                "pageBreakBefore" => self.page_break_before = Some(on(e)),
                "contextualSpacing" => self.contextual = Some(on(e)),
                "bidi" => self.bidi = Some(on(e)),
                "tabs" => {
                    for t in e.children_named(W, "tab") {
                        let pos = twip(t.own("pos")).unwrap_or(0.0);
                        let kind = t.val().unwrap_or("left").to_string();
                        self.tabs.retain(|x| (x.0 - pos).abs() > 0.5);
                        if kind != "clear" {
                            let leader = t.own("leader").filter(|l| *l != "none").map(str::to_string);
                            self.tabs.push((pos, kind, leader));
                        }
                    }
                    self.tabs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                }
                "numPr" => {
                    let id = e.child(W, "numId").and_then(|x| x.val()).map(str::to_string);
                    let lvl = e.child(W, "ilvl").and_then(|x| x.val()).and_then(|v| v.parse().ok());
                    match (id, &mut self.num) {
                        (Some(id), _) => self.num = Some((id, lvl.unwrap_or(0))),
                        (None, Some(n)) => {
                            if let Some(l) = lvl {
                                n.1 = l;
                            }
                        }
                        _ => {}
                    }
                }
                "shd" => {
                    if let Some(f) = e.own("fill").filter(|f| *f != "auto") {
                        self.shade = Some(f.to_ascii_uppercase());
                    } else if let Some(t) = e.own("themeFill").and_then(|t| theme.color(t)) {
                        self.shade = Some(office::wml_tint_shade(&t, e.own("themeFillTint"), e.own("themeFillShade")));
                    }
                }
                "pBdr" => {
                    for b in e.elements() {
                        let val = b.val().unwrap_or("nil");
                        self.borders.retain(|x| x.0 != b.local());
                        if val != "nil" && val != "none" {
                            let w = b.own("sz").and_then(|v| v.parse::<f64>().ok()).unwrap_or(4.0) / 8.0;
                            let c = b.own("color").filter(|c| *c != "auto").unwrap_or("000000").to_ascii_uppercase();
                            self.borders.push((b.local().to_string(), w, c));
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

struct StyleDef {
    kind: String,
    based_on: Option<String>,
    ppr: Option<Element>,
    rpr: Option<Element>,
    tblpr: Option<Element>,
    tcpr: Option<Element>,
    conds: Vec<(String, Element)>,
    link: Option<String>,
}

struct Level {
    fmt: String,
    text: String,
    start: i64,
    jc: String,
    suff: String,
    ppr: Option<Element>,
    rpr: Option<Element>,
    restart: Option<i64>,
    legal: bool,
}

#[derive(Default)]
struct Numbering {
    abstracts: HashMap<String, Vec<Level>>,
    /// numId → (abstractNumId, overrides: level → (start, Option<Level>))
    nums: HashMap<String, (String, HashMap<u32, (Option<i64>, Option<Level>)>)>,
    counters: HashMap<String, [i64; 9]>,
    started: HashMap<String, bool>,
}

fn parse_level(l: &Element) -> Level {
    Level {
        fmt: l.child(W, "numFmt").and_then(|e| e.val()).unwrap_or("decimal").to_string(),
        text: l.child(W, "lvlText").and_then(|e| e.val()).unwrap_or("").to_string(),
        start: l.child(W, "start").and_then(|e| e.val()).and_then(|v| v.parse().ok()).unwrap_or(1),
        jc: l.child(W, "lvlJc").and_then(|e| e.val()).unwrap_or("left").to_string(),
        suff: l.child(W, "suff").and_then(|e| e.val()).unwrap_or("tab").to_string(),
        ppr: l.child(W, "pPr").cloned(),
        rpr: l.child(W, "rPr").cloned(),
        restart: l.child(W, "lvlRestart").and_then(|e| e.val()).and_then(|v| v.parse().ok()),
        legal: l.child(W, "isLgl").is_some(),
    }
}

pub fn format_number(n: i64, fmt: &str) -> String {
    let roman = |mut n: i64| {
        let t = [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
        let mut s = String::new();
        for (v, r) in t {
            while n >= v {
                s.push_str(r);
                n -= v;
            }
        }
        s
    };
    let letters = |n: i64, base: &str| {
        let cs: Vec<char> = base.chars().collect();
        let k = cs.len() as i64;
        if n < 1 {
            return String::new();
        }
        let c = cs[((n - 1) % k) as usize];
        std::iter::repeat(c).take(((n - 1) / k + 1) as usize).collect::<String>()
    };
    match fmt {
        "decimal" | "decimalHalfWidth" => n.to_string(),
        "decimalZero" => format!("{:02}", n),
        "lowerLetter" => letters(n, "abcdefghijklmnopqrstuvwxyz"),
        "upperLetter" => letters(n, "ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
        "lowerRoman" => roman(n),
        "upperRoman" => roman(n).to_uppercase(),
        "ganada" => letters(n, "가나다라마바사아자차카타파하"),
        "chosung" => letters(n, "ㄱㄴㄷㄹㅁㅂㅅㅇㅈㅊㅋㅌㅍㅎ"),
        "decimalEnclosedCircle" | "decimalEnclosedCircleChinese" => {
            if (1..=20).contains(&n) {
                char::from_u32(0x2460 + n as u32 - 1).unwrap().to_string()
            } else {
                n.to_string()
            }
        }
        "decimalFullWidth" | "decimalFullWidth2" => n.to_string().chars().map(|c| char::from_u32(c as u32 - '0' as u32 + 0xFF10).unwrap_or(c)).collect(),
        "koreanDigital" | "koreanDigital2" | "koreanCounting" | "koreanLegal" => {
            let d = ["", "일", "이", "삼", "사", "오", "육", "칠", "팔", "구"];
            if (1..10).contains(&n) {
                d[n as usize].to_string()
            } else if (10..100).contains(&n) {
                format!("{}십{}", if n / 10 > 1 { d[(n / 10) as usize] } else { "" }, d[(n % 10) as usize])
            } else {
                n.to_string()
            }
        }
        "bullet" | "none" => String::new(),
        "ordinal" => format!("{n}{}", match (n % 10, n % 100) {
            (1, x) if x != 11 => "st",
            (2, x) if x != 12 => "nd",
            (3, x) if x != 13 => "rd",
            _ => "th",
        }),
        _ => n.to_string(),
    }
}

#[derive(Clone, Default)]
struct Section {
    width: f64,
    height: f64,
    margins: (f64, f64, f64, f64), // top right bottom left
    header_dist: f64,
    footer_dist: f64,
    cols: u32,
    col_gap: f64,
    title_pg: bool,
    kind: String,
    headers: HashMap<String, String>, // type → part
    footers: HashMap<String, String>,
    page_start: Option<i64>,
    page_fmt: Option<String>,
}

pub struct Conv<'a> {
    d: &'a Docx,
    fonts: &'a Fonts,
    theme: Theme,
    styles: HashMap<String, StyleDef>,
    default_para: Option<String>,
    default_table: Option<String>,
    defaults_r: RP,
    defaults_p: PP,
    numbering: Numbering,
    files: Vec<(String, Vec<u8>)>,
    media: HashMap<String, String>,
    warnings: Vec<String>,
    labels: usize,
    default_tab: f64,
    even_odd: bool,
    footnotes: HashMap<String, Element>,
    endnotes: Vec<(String, Element)>,
    endnote_parts: HashMap<String, Element>,
    metrics: HashMap<String, (f64, f64)>,
    /// The part whose relationships images and links resolve against.
    part_stack: Vec<(String, Rels)>,
    /// Column widths of the text area (for percentages and auto tables).
    area_width: f64,
    page: (f64, f64, f64, f64),
}

pub fn convert(d: &Docx, fonts: &Fonts) -> Result<Job> {
    let mut c = Conv::new(d, fonts);
    let out = c.document()?;
    Ok(Job { source: out, files: c.files, warnings: c.warnings })
}

impl<'a> Conv<'a> {
    fn new(d: &'a Docx, fonts: &'a Fonts) -> Conv<'a> {
        let main = d.parts[0].name.clone();
        let theme = Theme::load(&d.pkg, d.rels.first_part("theme"));
        let mut c = Conv {
            d,
            fonts,
            theme,
            styles: HashMap::new(),
            default_para: None,
            default_table: None,
            defaults_r: RP::default(),
            defaults_p: PP::default(),
            numbering: Numbering::default(),
            files: Vec::new(),
            media: HashMap::new(),
            warnings: Vec::new(),
            labels: 0,
            default_tab: 36.0,
            even_odd: false,
            footnotes: HashMap::new(),
            endnotes: Vec::new(),
            endnote_parts: HashMap::new(),
            metrics: HashMap::new(),
            part_stack: vec![(main.clone(), d.pkg.rels(&main).unwrap_or_else(|_| d.pkg.rels("").unwrap()))],
            area_width: 450.0,
            page: (0.0, 0.0, 0.0, 0.0),
        };
        c.load_styles();
        c.load_numbering();
        c.load_settings();
        c.load_notes();
        c
    }

    fn load_styles(&mut self) {
        // Word's defaults without docDefaults: Times New Roman 10pt.
        self.defaults_r.size = Some(10.0);
        let Some(part) = self.d.rels.first_part("styles") else { return };
        let Ok(doc) = self.d.pkg.xml(&part) else { return };
        if let Some(dd) = doc.root.child(W, "docDefaults") {
            if let Some(r) = dd.child(W, "rPrDefault").and_then(|x| x.child(W, "rPr")) {
                let t = self.theme.clone();
                self.defaults_r.apply(r, &t);
            }
            if let Some(p) = dd.child(W, "pPrDefault").and_then(|x| x.child(W, "pPr")) {
                let t = self.theme.clone();
                self.defaults_p.apply(p, &t);
            }
        }
        for st in doc.root.children_named(W, "style") {
            let id = st.own("styleId").unwrap_or("").to_string();
            let kind = st.own("type").unwrap_or("paragraph").to_string();
            if st.own("default").map(|v| v == "1" || v == "true").unwrap_or(false) {
                match kind.as_str() {
                    "paragraph" => self.default_para = Some(id.clone()),
                    "table" => self.default_table = Some(id.clone()),
                    _ => {}
                }
            }
            let conds = st.children_named(W, "tblStylePr").map(|c| (c.own("type").unwrap_or("").to_string(), c.clone())).collect();
            self.styles.insert(
                id,
                StyleDef {
                    kind,
                    based_on: st.child(W, "basedOn").and_then(|b| b.val()).map(str::to_string),
                    ppr: st.child(W, "pPr").cloned(),
                    rpr: st.child(W, "rPr").cloned(),
                    tblpr: st.child(W, "tblPr").cloned(),
                    tcpr: st.child(W, "tcPr").cloned(),
                    conds,
                    link: st.child(W, "link").and_then(|b| b.val()).map(str::to_string),
                },
            );
        }
    }

    fn load_numbering(&mut self) {
        let Some(part) = self.d.rels.first_part("numbering") else { return };
        let Ok(doc) = self.d.pkg.xml(&part) else { return };
        for a in doc.root.children_named(W, "abstractNum") {
            let id = a.own("abstractNumId").unwrap_or("").to_string();
            let mut levels: Vec<Level> = Vec::new();
            for l in a.children_named(W, "lvl") {
                levels.push(parse_level(l));
            }
            // A link to a numbering style: the levels of the abstractNum that defines it.
            self.numbering.abstracts.insert(id, levels);
        }
        for n in doc.root.children_named(W, "num") {
            let id = n.own("numId").unwrap_or("").to_string();
            let abs = n.child(W, "abstractNumId").and_then(|e| e.val()).unwrap_or("").to_string();
            let mut ov = HashMap::new();
            for o in n.children_named(W, "lvlOverride") {
                let ilvl: u32 = o.own("ilvl").and_then(|v| v.parse().ok()).unwrap_or(0);
                let start = o.child(W, "startOverride").and_then(|e| e.val()).and_then(|v| v.parse().ok());
                let lvl = o.child(W, "lvl").map(parse_level);
                ov.insert(ilvl, (start, lvl));
            }
            self.numbering.nums.insert(id, (abs, ov));
        }
    }

    fn load_settings(&mut self) {
        let Some(part) = self.d.rels.first_part("settings") else { return };
        let Ok(doc) = self.d.pkg.xml(&part) else { return };
        if let Some(v) = twip(doc.root.child(W, "defaultTabStop").and_then(|e| e.val())) {
            if v > 1.0 {
                self.default_tab = v;
            }
        }
        self.even_odd = doc.root.child(W, "evenAndOddHeaders").map(on).unwrap_or(false);
    }

    fn load_notes(&mut self) {
        for (kind, el) in [("footnotes", "footnote"), ("endnotes", "endnote")] {
            let Some(part) = self.d.rels.first_part(kind) else { continue };
            let Ok(doc) = self.d.pkg.xml(&part) else { continue };
            for f in doc.root.children_named(W, el) {
                if let Some(id) = f.own("id") {
                    if kind == "footnotes" {
                        self.footnotes.insert(id.to_string(), f.clone());
                    } else {
                        self.endnote_parts.insert(id.to_string(), f.clone());
                    }
                }
            }
        }
    }

    fn style_chain(&self, id: &str) -> Vec<&StyleDef> {
        let mut out = Vec::new();
        let mut cur = Some(id.to_string());
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            if guard > 20 {
                break;
            }
            let Some(st) = self.styles.get(&c) else { break };
            out.push(st);
            cur = st.based_on.clone();
        }
        out.reverse();
        out
    }

    /// Paragraph properties and the paragraph's run properties (its mark), for a paragraph.
    fn para_props(&self, p: &Element, table_style: Option<&str>) -> (PP, RP) {
        let mut pp = self.defaults_p.clone();
        let mut rp = self.defaults_r.clone();
        let theme = &self.theme;
        if let Some(ts) = table_style {
            for st in self.style_chain(ts) {
                if let Some(x) = &st.ppr {
                    pp.apply(x, theme);
                }
                if let Some(x) = &st.rpr {
                    rp.apply(x, theme);
                }
            }
        }
        let direct = p.child(W, "pPr");
        let sid = direct.and_then(|x| x.child(W, "pStyle")).and_then(|s| s.val()).map(str::to_string).or_else(|| self.default_para.clone());
        if let Some(sid) = &sid {
            for st in self.style_chain(sid) {
                if let Some(x) = &st.ppr {
                    pp.apply(x, theme);
                }
                if let Some(x) = &st.rpr {
                    rp.apply(x, theme);
                }
            }
        }
        // Numbering's indents come before direct formatting.
        let mut num = pp.num.clone();
        if let Some(d) = direct.and_then(|x| x.child(W, "numPr")) {
            let mut tmp = PP { num: num.clone(), ..Default::default() };
            let mut holder = Element::new("w:pPr", W);
            holder.push(d.clone());
            tmp.apply(&holder, theme);
            num = tmp.num;
        }
        if let Some((nid, lvl)) = &num {
            if let Some(level) = self.level(nid, *lvl) {
                if let Some(x) = &level.ppr {
                    pp.apply(x, theme);
                }
            }
        }
        if let Some(d) = direct {
            pp.apply(d, theme);
            if let Some(r) = d.child(W, "rPr") {
                rp.apply(r, theme);
            }
        }
        pp.num = num;
        pp.style = sid;
        (pp, rp)
    }

    /// Run properties: the paragraph's (without its mark's direct ones), the character style
    /// chain, then direct.
    fn run_props(&self, base: &RP, r: &Element) -> RP {
        let mut rp = base.clone();
        let theme = &self.theme;
        if let Some(rpr) = r.child(W, "rPr") {
            if let Some(cs) = rpr.child(W, "rStyle").and_then(|s| s.val()) {
                for st in self.style_chain(cs) {
                    if let Some(x) = &st.rpr {
                        rp.apply(x, theme);
                    }
                }
            }
            rp.apply(rpr, theme);
        }
        rp
    }

    fn level(&self, num_id: &str, ilvl: u32) -> Option<&Level> {
        let (abs, ov) = self.numbering.nums.get(num_id)?;
        if let Some((_, Some(l))) = ov.get(&ilvl) {
            return Some(l);
        }
        self.numbering.abstracts.get(abs)?.get(ilvl as usize)
    }

    /// The text of a list item's number, counting it.
    fn next_number(&mut self, num_id: &str, ilvl: u32) -> Option<(String, Option<Element>, String, String)> {
        let (abs, ov) = self.numbering.nums.get(num_id)?.clone_shallow();
        let levels_len = self.numbering.abstracts.get(&abs).map(|l| l.len()).unwrap_or(0);
        if levels_len == 0 {
            return None;
        }
        let key = abs.clone();
        let starts: Vec<i64> = (0..9).map(|l| ov.get(&(l as u32)).and_then(|o| o.0).or_else(|| self.level(num_id, l as u32).map(|x| x.start)).unwrap_or(1)).collect();
        let first_use = !self.numbering.started.contains_key(num_id);
        self.numbering.started.insert(num_id.to_string(), true);
        let counters = self.numbering.counters.entry(key).or_insert_with(|| {
            let mut c = [0i64; 9];
            for (i, s) in starts.iter().enumerate() {
                c[i] = s - 1;
            }
            c
        });
        if first_use {
            for (l, o) in ov.iter() {
                if let Some(s) = o.0 {
                    if (*l as usize) < 9 {
                        counters[*l as usize] = s - 1;
                    }
                }
            }
        }
        let l = (ilvl as usize).min(8);
        counters[l] += 1;
        for deeper in l + 1..9 {
            counters[deeper] = starts[deeper] - 1;
        }
        let snapshot = *counters;
        let level = self.level(num_id, ilvl)?;
        let mut text = level.text.clone();
        for k in 0..9 {
            let pat = format!("%{}", k + 1);
            if text.contains(&pat) {
                let lv = self.level(num_id, k as u32);
                let fmt = if level.legal { "decimal".to_string() } else { lv.map(|x| x.fmt.clone()).unwrap_or("decimal".into()) };
                let v = snapshot[k].max(lv.map(|x| x.start).unwrap_or(1));
                text = text.replace(&pat, &format_number(v, &fmt));
            }
        }
        if level.fmt == "bullet" {
            let font = level.rpr.as_ref().and_then(|r| r.child(W, "rFonts")).and_then(|f| f.own("ascii").or_else(|| f.own("hAnsi"))).unwrap_or("").to_string();
            if office::is_symbol_font(&font) {
                text = text.chars().map(|c| office::symbol_char(&font, c)).collect();
            }
            if text.chars().any(|c| ('\u{F000}'..='\u{F0FF}').contains(&c)) {
                text = "•".into();
            }
        }
        if level.fmt == "none" && !text.contains('%') {
            // Text only.
        }
        Some((text, level.rpr.clone(), level.jc.clone(), level.suff.clone()))
    }

    fn metrics_of(&mut self, stack: &[String]) -> (f64, f64) {
        let key = stack.first().cloned().unwrap_or_default();
        if let Some(m) = self.metrics.get(&key) {
            return *m;
        }
        let m = office::line_metrics(self.fonts, stack);
        self.metrics.insert(key, m);
        m
    }

    /// The tallest line metrics among the fonts a paragraph's text uses (its mark's when it has
    /// none): Word makes a line as tall as its tallest font.
    fn para_metrics(&mut self, p: &Element, mark: &RP) -> (f64, f64) {
        let stack = self.font_stack(mark);
        let (mut asc, mut desc) = self.metrics_of(&stack);
        let mut runs = Vec::new();
        fn collect<'e>(e: &'e Element, out: &mut Vec<&'e Element>) {
            for c in e.elements() {
                if c.is(W, "r") {
                    out.push(c);
                } else if !c.is(W, "pPr") && !c.is(W, "del") && !c.is(W, "txbxContent") {
                    collect(c, out);
                }
            }
        }
        collect(p, &mut runs);
        for r in runs.into_iter().take(64) {
            let text: String = r.children_named(W, "t").map(|t| t.text()).collect();
            if text.is_empty() {
                continue;
            }
            let rp = self.run_props(mark, r);
            let east = text.chars().any(|c| is_hangul(c) || ('\u{3000}'..='\u{9FFF}').contains(&c));
            let stack = if east {
                let ea = rp.east_asian(&self.theme).unwrap_or_default();
                office::font_stack(self.fonts, &ea, None)
            } else {
                self.font_stack(&rp)
            };
            let (a, d) = self.metrics_of(&stack);
            asc = asc.max(a);
            desc = desc.max(d);
        }
        (asc, desc)
    }

    /// A note's paragraphs as inline content (its number comes before it on the same line).
    fn note_content(&mut self, note: &Element) -> String {
        let mut out = String::new();
        let paras: Vec<Element> = note.elements().filter(|e| e.is(W, "p")).cloned().collect();
        for (i, p) in paras.iter().enumerate() {
            if i > 0 {
                out.push_str("#parbreak()");
            }
            let (_, mark) = self.para_props(p, None);
            let label = self.label();
            let mut ctx = RunCtx { base: mark, label, tabs: "()".into(), origin_offset: 0.0, field: Vec::new(), pieces: vec![String::new()], open_tabs: vec![0], hyperlink: None, anchors: Vec::new(), floats: Vec::new(), default_tab: self.default_tab };
            self.inline(p, &mut ctx, None);
            ctx.close_tabs();
            out.push_str(&ctx.pieces.join(""));
        }
        out
    }

    fn font_stack(&self, rp: &RP) -> Vec<String> {
        office::font_stack(self.fonts, &rp.latin(&self.theme), rp.east_asian(&self.theme).as_deref())
    }

    fn label(&mut self) -> String {
        self.labels += 1;
        format!("dl{}", self.labels)
    }

    // ── the document ────────────────────────────────────────────────────────────────────────

    fn section_of(&self, sp: &Element, prev: Option<&Section>) -> Section {
        let mut s = Section { width: 612.0, height: 792.0, margins: (72.0, 72.0, 72.0, 72.0), header_dist: 36.0, footer_dist: 36.0, cols: 1, col_gap: 36.0, kind: "nextPage".into(), ..Default::default() };
        if let Some(p) = prev {
            s.headers = p.headers.clone();
            s.footers = p.footers.clone();
        }
        if let Some(sz) = sp.child(W, "pgSz") {
            s.width = twip(sz.own("w")).unwrap_or(612.0);
            s.height = twip(sz.own("h")).unwrap_or(792.0);
        }
        if let Some(m) = sp.child(W, "pgMar") {
            let g = |k: &str, d: f64| twip(m.own(k)).map(f64::abs).unwrap_or(d);
            let gutter = g("gutter", 0.0);
            s.margins = (g("top", 72.0), g("right", 72.0), g("bottom", 72.0), g("left", 72.0) + gutter);
            s.header_dist = g("header", 36.0);
            s.footer_dist = g("footer", 36.0);
        }
        if let Some(c) = sp.child(W, "cols") {
            s.cols = c.own("num").and_then(|v| v.parse().ok()).unwrap_or(1).max(1);
            s.col_gap = twip(c.own("space")).unwrap_or(36.0);
        }
        s.title_pg = sp.child(W, "titlePg").map(on).unwrap_or(false);
        s.kind = sp.child(W, "type").and_then(|t| t.val()).unwrap_or("nextPage").to_string();
        let rels = &self.d.rels;
        for r in sp.elements() {
            let (map, ok) = if r.is(W, "headerReference") { (&mut s.headers, true) } else if r.is(W, "footerReference") { (&mut s.footers, true) } else { (&mut s.headers, false) };
            if !ok {
                continue;
            }
            let t = r.own("type").unwrap_or("default").to_string();
            if let Some(part) = r.attr_ns(R, "id").and_then(|id| rels.part(id)) {
                map.insert(t, part);
            }
        }
        if let Some(pn) = sp.child(W, "pgNumType") {
            s.page_start = pn.own("start").and_then(|v| v.parse().ok());
            s.page_fmt = pn.own("fmt").map(str::to_string);
        }
        s
    }

    fn page_numbering(fmt: Option<&str>) -> &'static str {
        match fmt {
            Some("lowerRoman") => "i",
            Some("upperRoman") => "I",
            Some("lowerLetter") => "a",
            Some("upperLetter") => "A",
            Some("ganada") => "가",
            _ => "1",
        }
    }

    fn page_setup(&mut self, sec: &Section, start_label: &str) -> String {
        let (t, r, b, l) = sec.margins;
        self.area_width = (sec.width - l - r) / sec.cols as f64 - if sec.cols > 1 { sec.col_gap * (sec.cols - 1) as f64 / sec.cols as f64 } else { 0.0 };
        self.page = (sec.width, sec.height, l, t);
        let numbering = Self::page_numbering(sec.page_fmt.as_deref());
        let hf = |this: &mut Self, map: &HashMap<String, String>, header: bool| -> String {
            let mut parts: HashMap<String, String> = HashMap::new();
            for (k, part) in map {
                let content = this.story_part(part);
                parts.insert(k.clone(), content);
            }
            if parts.is_empty() {
                return "none".into();
            }
            let place = |c: &str| {
                if header {
                    format!("place(top + left, dy: {}, block(width: 100%, {c}))", pt(sec.header_dist))
                } else {
                    format!("place(bottom + left, dy: {}, block(width: 100%, {c}))", pt(-sec.footer_dist))
                }
            };
            let default = parts.get("default").map(|c| place(c)).unwrap_or_else(|| "none".into());
            let first = if sec.title_pg { parts.get("first").map(|c| place(c)).unwrap_or_else(|| "none".into()) } else { default.clone() };
            let even = if this.even_odd { parts.get("even").map(|c| place(c)).unwrap_or_else(|| default.clone()) } else { default.clone() };
            format!("context {{ let pg = here().page(); let first = locate(<{start_label}>).page(); if pg == first {{ {first} }} else if calc.even(counter(page).get().first()) {{ {even} }} else {{ {default} }} }}")
        };
        let header = hf(self, &sec.headers, true);
        let footer = hf(self, &sec.footers, false);
        let mut out = format!(
            "#set page(width: {}, height: {}, margin: (top: {}, right: {}, bottom: {}, left: {}), columns: {}, header: {header}, footer: {footer}, header-ascent: 0pt, footer-descent: 0pt, numbering: {})\n",
            pt(sec.width),
            pt(sec.height),
            pt(t),
            pt(r),
            pt(b),
            pt(l),
            sec.cols,
            s(numbering)
        );
        if sec.cols > 1 {
            let _ = writeln!(out, "#set columns(gutter: {})", pt(sec.col_gap));
        }
        out
    }

    /// A header or footer part's content.
    fn story_part(&mut self, part: &str) -> String {
        let Ok(doc) = self.d.pkg.xml(part) else { return "[]".into() };
        let rels = self.d.pkg.rels(part).unwrap_or_else(|_| self.d.pkg.rels("").unwrap());
        self.part_stack.push((part.to_string(), rels));
        let saved = self.area_width;
        let mut out = String::from("[");
        let mut st = Flow::default();
        self.blocks(&doc.root, &mut out, None, &mut st);
        out.push(']');
        self.area_width = saved;
        self.part_stack.pop();
        out
    }

    fn document(&mut self) -> Result<String> {
        let body = self.d.story_el(0).clone();
        // Sections: each paragraph with a sectPr ends one; the body's sectPr is the last.
        let mut sect_els: Vec<Element> = Vec::new();
        for c in body.elements() {
            if let Some(sp) = c.child(W, "pPr").and_then(|p| p.child(W, "sectPr")) {
                sect_els.push(sp.clone());
            }
        }
        if let Some(sp) = body.child(W, "sectPr") {
            sect_els.push(sp.clone());
        } else {
            sect_els.push(Element::new("w:sectPr", W));
        }
        let mut sections: Vec<Section> = Vec::new();
        for (i, sp) in sect_els.iter().enumerate() {
            let prev = if i > 0 { sections.last() } else { None };
            let s = self.section_of(sp, prev);
            sections.push(s);
        }
        let mut out = String::new();
        out.push_str(typ::PRELUDE);
        let base_stack = self.font_stack(&self.defaults_r.clone());
        let lang = self.defaults_r.lang.as_deref().map(lang_code).unwrap_or("en");
        let _ = writeln!(
            out,
            "#set text(font: {}, size: {}, lang: {}, hyphenate: false, kerning: false, ligatures: false, top-edge: \"ascender\", bottom-edge: \"descender\")\n#set par(spacing: 0pt, leading: 0pt, justify: false)\n#set block(spacing: 0pt)\n#set table(stroke: none, inset: 0pt)\n#set footnote.entry(separator: line(length: 30% + 0pt, stroke: 0.5pt), clearance: 6pt, gap: 2pt, indent: 0pt)\n#set footnote(numbering: \"1\")\n#show footnote.entry: set text(size: 0.9em)",
            typ::fonts(&base_stack),
            pt(self.defaults_r.size.unwrap_or(10.0)),
            s(lang)
        );
        let first_label = self.label();
        let setup = self.page_setup(&sections[0].clone(), &first_label);
        out.push_str(&setup);
        if let Some(start) = sections[0].page_start {
            let _ = writeln!(out, "#counter(page).update({start})");
        }
        let _ = writeln!(out, "#metadata(none)<{first_label}>");
        // The body's blocks, section by section.
        let mut st = Flow::default();
        let mut sec = 0;
        let mut i = 0;
        let kids: Vec<Element> = body.elements().cloned().collect();
        while i < kids.len() {
            let c = &kids[i];
            let mut one = Element::new("w:body", W);
            one.push(c.clone());
            self.blocks(&one, &mut out, None, &mut st);
            if c.is(W, "p") && c.child(W, "pPr").and_then(|p| p.child(W, "sectPr")).is_some() && sec + 1 < sections.len() {
                sec += 1;
                let next = sections[sec].clone();
                let label = self.label();
                let continuous = next.kind == "continuous" && next.width == sections[sec - 1].width && next.height == sections[sec - 1].height;
                if continuous && next.cols == sections[sec - 1].cols {
                    // Same page: nothing to change.
                } else {
                    if continuous {
                        // A column change on the same page: typst needs a new page for it.
                    }
                    let setup = self.page_setup(&next, &label);
                    out.push_str(&setup);
                    if let Some(start) = next.page_start {
                        let _ = writeln!(out, "#counter(page).update({start})");
                    }
                    let _ = writeln!(out, "#metadata(none)<{label}>");
                    st = Flow::default();
                }
            }
            i += 1;
        }
        // Endnotes at the end.
        if !self.endnotes.is_empty() {
            out.push_str("#v(12pt)\n#line(length: 30%, stroke: 0.5pt)\n");
            let notes = std::mem::take(&mut self.endnotes);
            for (n, e) in notes {
                let mut content = String::new();
                let mut st = Flow::default();
                self.blocks(&e, &mut content, None, &mut st);
                let _ = writeln!(out, "#grid(columns: (auto, 1fr), column-gutter: 4pt, text(size: 0.8em, baseline: -0.3em, {}), [{content}])", s(&n));
            }
        }
        Ok(out)
    }

    // ── blocks ──────────────────────────────────────────────────────────────────────────────

    /// Paragraphs, tables and content controls of a container.
    fn blocks(&mut self, container: &Element, out: &mut String, tstyle: Option<&TableCtx>, st: &mut Flow) {
        for c in container.elements() {
            if c.is(W, "p") {
                self.paragraph(c, out, tstyle, st);
            } else if c.is(W, "tbl") {
                self.table(c, out, st);
                st.prev_after = 0.0;
                st.prev_style = None;
            } else if c.is(W, "sdt") {
                if let Some(content) = c.child(W, "sdtContent") {
                    self.blocks(content, out, tstyle, st);
                }
            } else if c.is(W, "customXml") || (c.ns() == MC && c.local() == "AlternateContent") {
                let inner = if c.ns() == MC { c.child(MC, "Choice").or_else(|| c.child(MC, "Fallback")) } else { Some(c) };
                if let Some(i) = inner {
                    self.blocks(i, out, tstyle, st);
                }
            }
        }
    }

    fn paragraph(&mut self, p: &Element, out: &mut String, tctx: Option<&TableCtx>, st: &mut Flow) {
        let (pp, mut mark) = self.para_props(p, tctx.and_then(|t| t.style.as_deref()));
        if let Some(t) = tctx {
            mark = t.apply_run(&mark, &self.theme);
        }
        // Spacing: Word adds the space after the previous paragraph and before this one; with
        // contextual spacing the same style takes none.
        let same_style = st.prev_style.is_some() && st.prev_style == pp.style;
        let mut before = if pp.before_auto { 14.0 } else { pp.before.unwrap_or(0.0) };
        let mut prev_after = st.prev_after;
        if same_style && pp.contextual.unwrap_or(false) {
            before = 0.0;
        }
        if same_style && st.prev_contextual {
            prev_after = 0.0;
        }
        if st.first_in_cell {
            prev_after = 0.0;
        }
        let gap = prev_after + before;
        let after = if pp.after_auto { 14.0 } else { pp.after.unwrap_or(0.0) };
        st.prev_after = after;
        st.prev_style = pp.style.clone();
        st.prev_contextual = pp.contextual.unwrap_or(false);
        st.first_in_cell = false;

        if pp.page_break_before.unwrap_or(false) {
            out.push_str("#pagebreak(weak: true)\n");
        }
        // Line height from the paragraph's font.
        let size = mark.size.unwrap_or(10.0);
        let (asc, desc) = self.para_metrics(p, &mark);
        let (top_edge, bottom_edge, leading) = match (pp.line, pp.line_rule.as_deref()) {
            (Some(l), Some("exact")) => {
                let h = l / 20.0;
                (format!("{}", pt(h * 0.8)), format!("{}", pt(-h * 0.2)), "0pt".to_string())
            }
            (Some(l), Some("atLeast")) => {
                let h = l / 20.0;
                let natural = (asc + desc) * size;
                if h > natural {
                    let extra = h - natural;
                    (format!("{}em + {}", typ::num(asc), pt(extra)), format!("{}em", typ::num(-desc)), "0pt".to_string())
                } else {
                    (format!("{}em", typ::num(asc)), format!("{}em", typ::num(-desc)), "0pt".to_string())
                }
            }
            (Some(l), _) => {
                let m = l / 240.0;
                (format!("{}em", typ::num(asc)), format!("{}em", typ::num(-desc)), format!("{}em", typ::num((asc + desc) * (m - 1.0))))
            }
            (None, _) => (format!("{}em", typ::num(asc)), format!("{}em", typ::num(-desc)), "0pt".to_string()),
        };
        let left = pp.left.unwrap_or(0.0);
        let right = pp.right.unwrap_or(0.0);
        let first = pp.first.unwrap_or(0.0);
        let (inset_left, first_indent, hanging) = if first < 0.0 { (left + first, 0.0, -first) } else { (left, first, 0.0) };
        let align = match pp.jc.as_deref() {
            Some("center") => "center",
            Some("right") | Some("end") => if pp.bidi.unwrap_or(false) { "left" } else { "right" },
            Some("both") | Some("distribute") | Some("thaiDistribute") | Some("lowKashida") | Some("mediumKashida") | Some("highKashida") => "justify",
            _ => if pp.bidi.unwrap_or(false) { "right" } else { "left" },
        };
        // The number of a list item.
        let mut number = String::new();
        if let Some((nid, lvl)) = pp.num.clone() {
            if nid != "0" {
                if let Some((text, lrpr, jc, suff)) = self.next_number(&nid, lvl) {
                    let mut nrp = mark.clone();
                    if let Some(r) = &lrpr {
                        nrp.apply(r, &self.theme);
                    }
                    // A number does not take the paragraph's underline or highlight.
                    nrp.underline = None;
                    nrp.highlight = None;
                    let t = self.text_expr(&text, &nrp);
                    if !text.is_empty() {
                        let gapw = match suff.as_str() {
                            "space" => "0.25em".to_string(),
                            "nothing" => "0pt".to_string(),
                            _ => String::new(),
                        };
                        number = if gapw.is_empty() && hanging > 0.0 {
                            let al = match jc.as_str() {
                                "right" => "right",
                                "center" => "center",
                                _ => "left",
                            };
                            if al == "left" {
                                format!("#context {{ let n = {t}; let w = measure(n).width; box(width: calc.max({h}, w + 3pt), n) }}", h = pt(hanging))
                            } else {
                                format!("#box(width: {h}, align({al}, {t}))", h = pt(hanging))
                            }
                        } else if gapw.is_empty() {
                            // A tab after it: to the next default stop.
                            format!("#{t}#h(0.5em)")
                        } else {
                            format!("#{t}#h({gapw})")
                        };
                    }
                }
            }
        }
        // Content, split at page breaks.
        let label = self.label();
        let tabs = self.tab_spec(&pp, left, first);
        let mut ctx = RunCtx { base: mark.clone(), label: label.clone(), tabs, origin_offset: left + if first < 0.0 { first } else { first }, field: Vec::new(), pieces: vec![String::new()], open_tabs: vec![0], hyperlink: None, anchors: Vec::new(), floats: Vec::new(), default_tab: self.default_tab };
        self.inline(p, &mut ctx, tctx);
        ctx.close_tabs();
        let pieces = std::mem::take(&mut ctx.pieces);
        let has_text = pieces.iter().any(|x| !x.is_empty());
        let mut block_opts = format!("above: {}, below: 0pt, width: 100%", pt(gap));
        if pp.keep_next.unwrap_or(false) {
            block_opts.push_str(", sticky: true");
        }
        if pp.keep_lines.unwrap_or(false) {
            block_opts.push_str(", breakable: false");
        }
        if inset_left != 0.0 || right != 0.0 {
            let _ = write!(block_opts, ", inset: (left: {}, right: {})", pt(inset_left), pt(right));
        }
        if let Some(sh) = &pp.shade {
            let _ = write!(block_opts, ", fill: {}", rgb(sh));
        }
        if !pp.borders.is_empty() {
            let mut parts = Vec::new();
            for (side, w, c) in &pp.borders {
                let k = match side.as_str() {
                    "top" => "top",
                    "bottom" => "bottom",
                    "left" | "start" => "left",
                    "right" | "end" => "right",
                    _ => continue,
                };
                parts.push(format!("{k}: {} + {}", pt(*w), rgb(c)));
            }
            if !parts.is_empty() {
                let _ = write!(block_opts, ", stroke: ({})", parts.join(", "));
            }
        }
        let par_set = format!(
            "#set par(justify: {}, leading: {leading}, first-line-indent: (amount: {}, all: true), hanging-indent: {})\n#set text(top-edge: {top_edge}, bottom-edge: {bottom_edge})\n#set align({})",
            align == "justify",
            pt(first_indent),
            pt(hanging),
            if align == "justify" { "left" } else { align }
        );
        let floats = std::mem::take(&mut ctx.floats);
        let outer = out;
        let mut inner = String::new();
        let out = if floats.is_empty() { &mut *outer } else { &mut inner };
        let n = pieces.len();
        for (k, piece) in pieces.into_iter().enumerate() {
            let opts = if k == 0 { block_opts.clone() } else { block_opts.replacen(&format!("above: {}", pt(gap)), "above: 0pt", 1) };
            let content = if piece.is_empty() || !has_text {
                // An empty paragraph still has a line, as tall as its mark's font.
                let strut = self.text_expr("\u{200B}", &mark);
                format!("{}#{}", if k == 0 { number.as_str() } else { "" }, strut)
            } else if k == 0 {
                format!("{number}#metadata(none)<{label}>{piece}")
            } else {
                piece
            };
            let _ = writeln!(out, "#block({opts})[\n{par_set}\n{content}\n]");
            if k + 1 < n {
                out.push_str("#pagebreak()\n");
            }
        }
        if !floats.is_empty() {
            // The paragraph between its floats: a grid of (left floats, text, right floats).
            let (lw, rw) = floats.iter().fold((0f64, 0f64), |(l, r), f| if f.0 { (l.max(f.2), r) } else { (l, r.max(f.2)) });
            let col = |left: bool| -> String {
                let items: Vec<String> = floats.iter().filter(|f| f.0 == left).map(|f| format!("#v({})#{}", pt(f.3), f.1)).collect();
                format!("[{}]", items.join("\n"))
            };
            let _ = writeln!(
                outer,
                "#block(above: {}, below: 0pt, width: 100%, grid(columns: ({}, 1fr, {}), column-gutter: 0pt, {}, [\n{inner}], align(right, {})))",
                pt(gap),
                pt(lw),
                pt(rw),
                col(true),
                col(false)
            );
        }
        // Drawings anchored to the paragraph.
        for a in ctx.anchors {
            outer.push_str(&a);
            outer.push('\n');
        }
    }

    fn tab_spec(&self, pp: &PP, left: f64, first: f64) -> String {
        let mut stops: Vec<(f64, String, Option<String>)> = pp.tabs.clone();
        // A hanging indent is a stop.
        if first < 0.0 && !stops.iter().any(|s| (s.0 - left).abs() < 0.5) {
            stops.push((left, "left".into(), None));
            stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        }
        let items: Vec<String> = stops
            .iter()
            .filter(|s| s.1 != "bar")
            .map(|(p, k, l)| {
                let kind = match k.as_str() {
                    "right" | "end" | "decimal" => "right",
                    "center" => "center",
                    _ => "left",
                };
                let leader = match l.as_deref() {
                    Some("dot") | Some("middleDot") => s("."),
                    Some("hyphen") => s("-"),
                    Some("underscore") | Some("heavy") => s("_"),
                    _ => "none".into(),
                };
                format!("(pos: {}, kind: {}, leader: {leader})", pt(*p), s(kind))
            })
            .collect();
        format!("({}{})", items.join(", "), if items.len() == 1 { "," } else { "" })
    }

    /// Typst code for a run of text with its properties (a content expression).
    fn text_expr(&mut self, t: &str, rp: &RP) -> String {
        let stack = self.font_stack(rp);
        let mut text = t.to_string();
        if rp.caps.unwrap_or(false) {
            text = text.to_uppercase();
        }
        // Symbol fonts: their characters as Unicode.
        let latin = rp.latin(&self.theme);
        if office::is_symbol_font(&latin) {
            text = text.chars().map(|c| office::symbol_char(&latin, c)).collect();
        }
        let mut args = format!("font: {}", typ::fonts(&stack));
        let size = rp.size.unwrap_or(10.0);
        let _ = write!(args, ", size: {}", pt(size));
        if rp.bold.unwrap_or(false) {
            args.push_str(", weight: \"bold\"");
        }
        if rp.italic.unwrap_or(false) {
            args.push_str(", style: \"italic\"");
        }
        match rp.color.as_deref() {
            Some(c) if c != "AUTO" && c.len() == 6 => {
                let _ = write!(args, ", fill: {}", rgb(c));
            }
            _ => {}
        }
        if let Some(sp) = rp.spacing {
            if sp != 0.0 {
                let _ = write!(args, ", tracking: {}", pt(sp));
            }
        }
        if let Some(k) = rp.kern {
            if k > 0.0 && size >= k {
                args.push_str(", kerning: true");
            }
        }
        if let Some(pos) = rp.position {
            if pos != 0.0 {
                let _ = write!(args, ", baseline: {}", pt(-pos));
            }
        }
        if t.chars().any(is_hangul) {
            args.push_str(", lang: \"ko\"");
        }
        let mut e = format!("text({args}, {})", s(&text));
        match rp.vert.as_deref() {
            Some("superscript") => e = format!("super(typographic: false, size: 0.65em, baseline: -0.35em, {e})"),
            Some("subscript") => e = format!("sub(typographic: false, size: 0.65em, baseline: 0.15em, {e})"),
            _ => {}
        }
        if rp.small_caps.unwrap_or(false) {
            e = format!("smallcaps(all: false, {e})");
        }
        if let Some((u, color)) = &rp.underline {
            if u != "none" {
                let stroke_color = color.as_ref().map(|c| rgb(c));
                let dash = match u.as_str() {
                    "dotted" | "dottedHeavy" => Some("dotted"),
                    "dash" | "dashedHeavy" | "dashLong" | "dashLongHeavy" => Some("dashed"),
                    "dotDash" | "dotDotDash" | "dashDotHeavy" => Some("dash-dotted"),
                    "wave" | "wavyHeavy" | "wavyDouble" => Some("loosely-dotted"),
                    _ => None,
                };
                let thick = u.contains("thick") || u.contains("Heavy");
                let mut stroke = Vec::new();
                if let Some(c) = &stroke_color {
                    stroke.push(format!("paint: {c}"));
                }
                if let Some(d) = dash {
                    stroke.push(format!("dash: {}", s(d)));
                }
                if thick {
                    stroke.push("thickness: 0.1em".into());
                }
                let st = if stroke.is_empty() { String::new() } else { format!("stroke: ({}), ", stroke.join(", ")) };
                e = format!("underline({st}evade: false, {e})");
                if u == "double" {
                    e = format!("underline({st}offset: 0.2em, evade: false, {e})");
                }
            }
        }
        if rp.strike.unwrap_or(false) {
            e = format!("strike({e})");
        }
        if rp.dstrike.unwrap_or(false) {
            e = format!("strike(offset: -0.2em, strike(offset: -0.35em, {e}))");
        }
        let bg = rp.highlight.clone().filter(|h| !h.is_empty()).or_else(|| rp.shade.clone());
        if let Some(h) = bg {
            e = format!("highlight(fill: {}, top-edge: \"ascender\", bottom-edge: \"descender\", extent: 0pt, {e})", rgb(&h));
        }
        e
    }

    // ── runs ────────────────────────────────────────────────────────────────────────────────

    fn inline(&mut self, container: &Element, ctx: &mut RunCtx, tctx: Option<&TableCtx>) {
        for c in container.elements() {
            match (c.ns(), c.local()) {
                (W, "pPr") => {}
                (W, "r") => self.run(c, ctx, tctx),
                (W, "hyperlink") => {
                    let url = c.attr_ns(R, "id").and_then(|id| self.part_stack.last().and_then(|(_, rels)| rels.get(id).map(|r| r.target.clone())));
                    let saved = ctx.hyperlink.take();
                    ctx.hyperlink = url;
                    self.inline(c, ctx, tctx);
                    ctx.hyperlink = saved;
                }
                (W, "fldSimple") => {
                    let instr = c.own("instr").unwrap_or("").trim().to_string();
                    if let Some(value) = field_code(&instr) {
                        let first_rp = c.child(W, "r").map(|r| self.run_props(&ctx.base, r)).unwrap_or_else(|| ctx.base.clone());
                        let e = self.text_expr("X", &first_rp);
                        ctx.push(&format!("#context {}", e.replacen("\"X\"", &value, 1)));
                    } else {
                        self.inline(c, ctx, tctx);
                    }
                }
                (W, "smartTag") | (W, "customXml") | (W, "ins") | (W, "moveTo") | (W, "bdo") | (W, "dir") => self.inline(c, ctx, tctx),
                (W, "sdt") => {
                    if let Some(sc) = c.child(W, "sdtContent") {
                        self.inline(sc, ctx, tctx);
                    }
                }
                (MC, "AlternateContent") => {
                    if let Some(ch) = c.child(MC, "Choice").or_else(|| c.child(MC, "Fallback")) {
                        self.inline(ch, ctx, tctx);
                    }
                }
                (W, "del") | (W, "moveFrom") => {}
                _ => {}
            }
        }
    }

    fn run(&mut self, r: &Element, ctx: &mut RunCtx, tctx: Option<&TableCtx>) {
        let base_rp = ctx.base.clone();
        let mut rp = self.run_props(&base_rp, r);
        if let Some(t) = tctx {
            // Table conditional formatting sits under direct run formatting; re-apply direct.
            rp = t.apply_run(&rp, &self.theme);
            if let Some(d) = r.child(W, "rPr") {
                rp.apply(d, &self.theme);
            }
        }
        if rp.hidden.unwrap_or(false) {
            return;
        }
        let mut buf = String::new();
        let flush = |this: &mut Self, buf: &mut String, ctx: &mut RunCtx, rp: &RP| {
            if buf.is_empty() {
                return;
            }
            let e = this.text_expr(buf, rp);
            let e = match &ctx.hyperlink {
                Some(u) if !ctx.in_field_code() => format!("link({}, {e})", s(u)),
                _ => e,
            };
            if !ctx.suppressed() {
                ctx.push(&format!("#{e}"));
            }
            buf.clear();
        };
        for c in r.elements() {
            match (c.ns(), c.local()) {
                (W, "t") => {
                    if !ctx.in_field_code() {
                        buf.push_str(&c.text());
                    }
                }
                (W, "instrText") => {
                    if let Some(f) = ctx.field.last_mut() {
                        f.0.push_str(&c.text());
                    }
                }
                (W, "fldChar") => {
                    flush(self, &mut buf, ctx, &rp);
                    match c.own("fldCharType") {
                        Some("begin") => ctx.field.push((String::new(), false, false)),
                        Some("separate") => {
                            if let Some(f) = ctx.field.last_mut() {
                                f.1 = true;
                                if let Some(value) = field_code(f.0.trim()) {
                                    f.2 = true;
                                    let e = self.text_expr("X", &rp);
                                    ctx.field_push(&format!("#context {}", e.replacen("\"X\"", &value, 1)));
                                }
                            }
                        }
                        Some("end") => {
                            ctx.field.pop();
                        }
                        _ => {}
                    }
                }
                (W, "tab") | (W, "ptab") => {
                    flush(self, &mut buf, ctx, &rp);
                    if !ctx.suppressed() {
                        ctx.tab();
                    }
                }
                (W, "br") => {
                    flush(self, &mut buf, ctx, &rp);
                    if ctx.suppressed() {
                        continue;
                    }
                    match c.own("type") {
                        Some("page") | Some("column") => {
                            ctx.close_tabs();
                            ctx.pieces.push(String::new());
                            ctx.open_tabs.push(0);
                        }
                        _ => ctx.push("#linebreak()"),
                    }
                }
                (W, "cr") => {
                    flush(self, &mut buf, ctx, &rp);
                    ctx.push("#linebreak()");
                }
                (W, "noBreakHyphen") => buf.push('\u{2011}'),
                (W, "softHyphen") => buf.push('\u{ad}'),
                (W, "sym") => {
                    let font = c.own("font").unwrap_or("Symbol").to_string();
                    let ch = c.own("char").and_then(|h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32).unwrap_or('?');
                    buf.push(office::symbol_char(&font, ch));
                }
                (W, "footnoteReference") | (W, "endnoteReference") => {
                    flush(self, &mut buf, ctx, &rp);
                    let id = c.own("id").unwrap_or("").to_string();
                    if c.local() == "footnoteReference" {
                        if let Some(f) = self.footnotes.get(&id).cloned() {
                            let content = self.note_content(&f);
                            ctx.push(&format!("#footnote[{content}]"));
                        }
                    } else if let Some(f) = self.endnote_parts.get(&id).cloned() {
                        let n = (self.endnotes.len() + 1).to_string();
                        let e = self.text_expr(&format_number(n.parse().unwrap_or(1), "lowerRoman"), &RP { vert: Some("superscript".into()), ..rp.clone() });
                        ctx.push(&format!("#{e}"));
                        self.endnotes.push((format_number(n.parse().unwrap_or(1), "lowerRoman"), f));
                    }
                }
                (W, "footnoteRef") | (W, "endnoteRef") => {
                    // The note's own number, in the note's text: typst writes it.
                }
                (W, "drawing") => {
                    flush(self, &mut buf, ctx, &rp);
                    self.drawing(c, ctx);
                }
                (W, "pict") | (W, "object") => {
                    flush(self, &mut buf, ctx, &rp);
                    self.vml(c, ctx);
                }
                (MC, "AlternateContent") => {
                    flush(self, &mut buf, ctx, &rp);
                    if let Some(ch) = c.child(MC, "Choice") {
                        for x in ch.elements() {
                            if x.is(W, "drawing") {
                                self.drawing(x, ctx);
                            }
                        }
                    } else if let Some(fb) = c.child(MC, "Fallback") {
                        for x in fb.elements() {
                            if x.is(W, "pict") {
                                self.vml(x, ctx);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        flush(self, &mut buf, ctx, &rp);
    }

    // ── drawings ────────────────────────────────────────────────────────────────────────────

    fn image_file(&mut self, rid: &str) -> Option<String> {
        let (part, rels) = self.part_stack.last()?;
        let rel = rels.get(rid)?;
        if rel.external {
            return None;
        }
        let target = opc::resolve(part, &rel.target);
        if let Some(v) = self.media.get(&target) {
            return Some(v.clone());
        }
        let data = self.d.pkg.zip.read(&target).ok()?;
        let (vpath, bytes) = super::images::prepare(&target, data, &mut self.warnings)?;
        self.files.push((vpath.clone(), bytes));
        self.media.insert(target, vpath.clone());
        Some(vpath)
    }

    fn drawing(&mut self, d: &Element, ctx: &mut RunCtx) {
        let Some(container) = d.elements().next() else { return };
        let inline = container.is(WP, "inline");
        let ext = container.child(WP, "extent");
        let emu = |e: Option<&Element>, a: &str| e.and_then(|x| x.attr_ns(NONE, a)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 12700.0;
        let w = emu(ext, "cx");
        let h = emu(ext, "cy");
        let Some(gd) = container.child(A, "graphic").and_then(|g| g.child(A, "graphicData")) else { return };
        let content = self.graphic(gd, w, h);
        let Some(content) = content else { return };
        if inline {
            ctx.push(&format!("#box(width: {}, height: {}, {content})", pt(w), pt(h)));
            return;
        }
        // Anchored: positioned from the page, margin or paragraph.
        let ph = container.child(WP, "positionH");
        let pv = container.child(WP, "positionV");
        let off = |p: Option<&Element>| p.and_then(|x| x.child(WP, "posOffset")).map(|o| o.text().trim().parse::<f64>().unwrap_or(0.0) / 12700.0);
        let (pw, phh, lm, tm) = self.page;
        let rel_h = ph.and_then(|p| p.attr_ns(NONE, "relativeFrom")).unwrap_or("column");
        let rel_v = pv.and_then(|p| p.attr_ns(NONE, "relativeFrom")).unwrap_or("paragraph");
        let align_h = ph.and_then(|p| p.child(WP, "align")).map(|a| a.text());
        let align_v = pv.and_then(|p| p.child(WP, "align")).map(|a| a.text());
        let area_w = self.area_width;
        let x_abs: String = {
            let (origin, width) = match rel_h {
                "page" => (0.0, pw),
                "leftMargin" | "insideMargin" => (0.0, lm),
                "rightMargin" | "outsideMargin" => (pw - (pw - lm - area_w), pw - lm - area_w),
                _ => (lm, area_w),
            };
            match align_h.as_deref() {
                Some("center") => pt(origin + (width - w) / 2.0),
                Some("right") | Some("outside") => pt(origin + width - w),
                Some("left") | Some("inside") => pt(origin),
                _ => pt(origin + off(ph).unwrap_or(0.0)),
            }
        };
        let (y_kind, y_val) = match rel_v {
            "page" => ("abs", match align_v.as_deref() {
                Some("center") => (phh - h) / 2.0,
                Some("bottom") => phh - h,
                Some("top") => 0.0,
                _ => off(pv).unwrap_or(0.0),
            }),
            "margin" | "topMargin" => ("abs", tm + off(pv).unwrap_or(0.0)),
            _ => ("rel", off(pv).unwrap_or(0.0)),
        };
        let behind = container.attr_ns(NONE, "behindDoc") == Some("1");
        let wraps_block = container.child(WP, "wrapTopAndBottom").is_some();
        let wraps_side = !behind && container.elements().any(|e| matches!(e.local(), "wrapSquare" | "wrapTight" | "wrapThrough"));
        if wraps_side && y_kind == "rel" && w < area_w * 0.75 && rel_v == "paragraph" {
            // Text beside it: the paragraph and the drawing side by side (the text does not
            // flow on below the drawing, but nothing overlaps).
            let x_num: f64 = x_abs.trim_end_matches("pt").parse().unwrap_or(lm);
            let left_side = x_num - lm + w / 2.0 < area_w / 2.0;
            let dist = |a: &str| container.attr_ns(NONE, a).and_then(|v| v.parse::<f64>().ok()).map(|v| v / 12700.0).unwrap_or(9.0);
            let gap = if left_side { dist("distR") } else { dist("distL") };
            ctx.floats.push((left_side, format!("box(width: {}, height: {}, {content})", pt(w), pt(h)), w + gap, y_val.max(0.0)));
            return;
        }
        let y_expr = if y_kind == "abs" { format!("{} - p.y", pt(y_val)) } else { pt(y_val) };
        let placed = format!("#context {{ let p = here().position(); place(top + left, dx: {x_abs} - p.x + {}, dy: {y_expr}, box(width: {}, height: {}, {content})) }}", pt(0.0), pt(w), pt(h));
        // Positions are relative to the paragraph's block; its origin is where the first line
        // starts, less the indents (computed in the block: p.x of the block's start).
        let fixed = placed.replace("- p.x + 0pt", &format!("- (p.x - {})", pt(ctx.origin_offset)));
        if wraps_block && y_kind == "rel" {
            // Text goes above and below: a block of its own height before the paragraph's text.
            ctx.push(&format!("#box(width: 0pt, height: {})", pt(h + y_val.max(0.0))));
        }
        ctx.push(&fixed);
    }

    /// The content of a DrawingML graphic (picture, shape with text, group, chart) at a size.
    fn graphic(&mut self, gd: &Element, w: f64, h: f64) -> Option<String> {
        if let Some(pic) = gd.child(PIC, "pic") {
            return self.picture(pic, w, h);
        }
        if let Some(wsp) = gd.child(WPS, "wsp") {
            return Some(self.wordshape(wsp, w, h));
        }
        if let Some(g) = gd.child(WPG, "wgp") {
            return Some(self.wordgroup(g, w, h));
        }
        if gd.elements().any(|e| e.local() == "chart") {
            let rid = gd.elements().find(|e| e.local() == "chart").and_then(|c| c.attr_ns(R, "id")).map(str::to_string)?;
            let (part, rels) = self.part_stack.last()?;
            let cp = rels.part(&rid)?;
            let _ = part;
            return Some(super::chart::render_part(&self.d.pkg, &cp, w, h, &self.theme, self.fonts, &mut self.warnings));
        }
        Some(format!("rect(width: {}, height: {}, stroke: 0.5pt + gray)", pt(w), pt(h)))
    }

    fn picture(&mut self, pic: &Element, w: f64, h: f64) -> Option<String> {
        let blip = pic.child(PIC, "blipFill").and_then(|b| b.child(A, "blip"))?;
        let rid = blip.attr_ns(R, "embed").or_else(|| blip.attr_ns(R, "link"))?.to_string();
        // An SVG version, when there is one.
        let svg_rid = blip.descendants().iter().find(|e| e.local() == "svgBlip").and_then(|e| e.attr_ns(R, "embed")).map(str::to_string);
        let file = svg_rid.and_then(|r| self.image_file(&r)).or_else(|| self.image_file(&rid))?;
        let src = pic.child(PIC, "blipFill").and_then(|b| b.child(A, "srcRect"));
        Some(super::images::image_expr(&file, w, h, src))
    }

    fn wordshape(&mut self, wsp: &Element, w: f64, h: f64) -> String {
        let sppr = wsp.child(WPS, "spPr");
        let geom = sppr.and_then(|s| s.child(A, "prstGeom")).and_then(|g| g.attr_ns(NONE, "prst")).unwrap_or("rect").to_string();
        let empty = HashMap::new();
        let style = wsp.child(WPS, "style");
        let fill = super::shapes::fill_of(sppr, style, &self.theme, &empty);
        let stroke = super::shapes::stroke_of(sppr, style, &self.theme, &empty);
        let mut text = String::new();
        if let Some(tb) = wsp.child(WPS, "txbx").and_then(|t| t.child(W, "txbxContent")) {
            let saved = self.area_width;
            self.area_width = w;
            let mut st = Flow { first_in_cell: true, ..Default::default() };
            self.blocks(tb, &mut text, None, &mut st);
            self.area_width = saved;
        }
        let body = wsp.child(WPS, "bodyPr");
        let ins = |a: &str, d: f64| body.and_then(|b| b.attr_ns(NONE, a)).and_then(|v| v.parse::<f64>().ok()).map(|v| v / 12700.0).unwrap_or(d);
        let inset = format!("(left: {}, right: {}, top: {}, bottom: {})", pt(ins("lIns", 7.2)), pt(ins("rIns", 7.2)), pt(ins("tIns", 3.6)), pt(ins("bIns", 3.6)));
        let anchor = match body.and_then(|b| b.attr_ns(NONE, "anchor")) {
            Some("ctr") => "horizon",
            Some("b") => "bottom",
            _ => "top",
        };
        let shape = super::shapes::preset(&geom, w, h, &fill, &stroke, sppr.and_then(|s| s.child(A, "prstGeom")));
        if text.is_empty() {
            shape
        } else {
            format!("{{ place(top + left, {shape}); box(width: {}, height: {}, inset: {inset}, align({anchor}, [{text}])) }}", pt(w), pt(h))
        }
    }

    fn wordgroup(&mut self, g: &Element, w: f64, h: f64) -> String {
        let x = g.child(WPG, "grpSpPr").and_then(|s| s.child(A, "xfrm"));
        let gi = |e: Option<&Element>, a: &str| e.and_then(|x| x.attr_ns(NONE, a)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        let ch_off = x.and_then(|x| x.child(A, "chOff"));
        let ch_ext = x.and_then(|x| x.child(A, "chExt"));
        let (cox, coy, cw, chh) = (gi(ch_off, "x"), gi(ch_off, "y"), gi(ch_ext, "cx").max(1.0), gi(ch_ext, "cy").max(1.0));
        let sx = w * 12700.0 / cw;
        let sy = h * 12700.0 / chh;
        let mut out = format!("box(width: {}, height: {})[", pt(w), pt(h));
        for c in g.elements() {
            let xf = c.elements().find(|e| e.local() == "spPr" || e.local() == "grpSpPr").and_then(|s| s.child(A, "xfrm"));
            let off = xf.and_then(|x| x.child(A, "off"));
            let ext = xf.and_then(|x| x.child(A, "ext"));
            let (ox, oy, ew, eh) = (gi(off, "x"), gi(off, "y"), gi(ext, "cx"), gi(ext, "cy"));
            let (px, py, pw, phh) = ((ox - cox) * sx / 12700.0, (oy - coy) * sy / 12700.0, ew * sx / 12700.0, eh * sy / 12700.0);
            let inner = if c.is(WPS, "wsp") {
                Some(self.wordshape(c, pw, phh))
            } else if c.is(PIC, "pic") {
                self.picture(c, pw, phh)
            } else if c.is(WPG, "grpSp") {
                Some(self.wordgroup(c, pw, phh))
            } else {
                None
            };
            if let Some(i) = inner {
                let _ = write!(out, "#place(top + left, dx: {}, dy: {}, box(width: {}, height: {}, {i}))", pt(px), pt(py), pt(pw), pt(phh));
            }
        }
        out.push(']');
        out
    }

    /// VML pictures (`w:pict`): images and text boxes of older documents.
    fn vml(&mut self, pict: &Element, ctx: &mut RunCtx) {
        for e in pict.descendants() {
            if e.ns() == V && e.local() == "imagedata" {
                let Some(rid) = e.attr_ns(R, "id").map(str::to_string) else { continue };
                let shape = pict.descendants().into_iter().find(|x| x.ns() == V && (x.local() == "shape" || x.local() == "rect"));
                let style = shape.and_then(|s| s.attr_ns(NONE, "style")).unwrap_or("").to_string();
                let (w, h) = vml_size(&style);
                if let Some(f) = self.image_file(&rid) {
                    ctx.push(&format!("#box(width: {}, height: {}, {})", pt(w), pt(h), super::images::image_expr(&f, w, h, None)));
                }
                return;
            }
        }
        for e in pict.descendants() {
            if e.is(W, "txbxContent") {
                let mut text = String::new();
                let mut st = Flow { first_in_cell: true, ..Default::default() };
                self.blocks(e, &mut text, None, &mut st);
                ctx.push(&format!("#box(stroke: 0.5pt, inset: 4pt)[{text}]"));
                return;
            }
        }
    }

    // ── tables ──────────────────────────────────────────────────────────────────────────────

    fn table(&mut self, tbl: &Element, out: &mut String, st: &mut Flow) {
        let tpr = tbl.child(W, "tblPr");
        let style = tpr.and_then(|t| t.child(W, "tblStyle")).and_then(|s| s.val()).map(str::to_string).or_else(|| self.default_table.clone());
        // Table-level properties: the style chain, then direct.
        let mut props = TableProps::default();
        if let Some(sid) = &style {
            let chain: Vec<(Option<Element>, Option<Element>)> = self.style_chain(sid).iter().map(|s| (s.tblpr.clone(), s.tcpr.clone())).collect();
            for (tp, tc) in chain {
                if let Some(t) = tp {
                    props.apply(&t, &self.theme);
                }
                if let Some(t) = tc {
                    props.apply_tc(&t, &self.theme);
                }
            }
        }
        if let Some(t) = tpr {
            props.apply(t, &self.theme);
        }
        let look = tpr.and_then(|t| t.child(W, "tblLook"));
        let look_flag = |name: &str, bit: u32| -> bool {
            match look {
                Some(l) => match l.own(name) {
                    Some(v) => v == "1" || v == "true",
                    None => l.val().and_then(|v| u32::from_str_radix(v, 16).ok()).map(|v| v & bit != 0).unwrap_or(false),
                },
                None => false,
            }
        };
        let flags = Look {
            first_row: look_flag("firstRow", 0x20),
            last_row: look_flag("lastRow", 0x40),
            first_col: look_flag("firstColumn", 0x80),
            last_col: look_flag("lastColumn", 0x100),
            no_hband: look_flag("noHBand", 0x200),
            no_vband: look_flag("noVBand", 0x400),
        };
        let conds: Vec<(String, Element)> = style.as_ref().map(|sid| self.style_chain(sid).iter().flat_map(|s| s.conds.clone()).collect()).unwrap_or_default();
        // The grid.
        let mut cols: Vec<f64> = tbl.child(W, "tblGrid").map(|g| g.children_named(W, "gridCol").map(|c| twip(c.own("w")).unwrap_or(0.0)).collect()).unwrap_or_default();
        let rows: Vec<&Element> = collect_rows(tbl);
        let ncols = rows.iter().map(|r| row_cells(r).iter().map(|c| span_of(c)).sum::<usize>() + grid_before(r)).max().unwrap_or(1).max(cols.len()).max(1);
        while cols.len() < ncols {
            cols.push(0.0);
        }
        // Columns without a width share what is left.
        let known: f64 = cols.iter().sum();
        let target = match props.width {
            Some(TW::Dxa(w)) => w,
            Some(TW::Pct(p)) => self.area_width * p,
            _ => known.max(self.area_width.min(known.max(1.0))),
        };
        let zero = cols.iter().filter(|c| **c <= 0.0).count();
        if zero > 0 {
            let rest = ((target - known) / zero as f64).max(18.0);
            for c in cols.iter_mut() {
                if *c <= 0.0 {
                    *c = rest;
                }
            }
        }
        // Vertical merges: rowspans.
        let mut grid: Vec<Vec<Option<CellRef>>> = Vec::new();
        for (ri, r) in rows.iter().enumerate() {
            let mut line: Vec<Option<CellRef>> = vec![None; ncols];
            let mut col = grid_before(r);
            for (ci, c) in row_cells(r).iter().enumerate() {
                let span = span_of(c).max(1);
                if col < ncols {
                    line[col] = Some(CellRef { row: ri, idx: ci, span, vmerge: vmerge_of(c) });
                }
                col += span;
            }
            grid.push(line);
        }
        let nrows = rows.len();
        let mut rowspan: HashMap<(usize, usize), usize> = HashMap::new();
        let mut skip: std::collections::HashSet<(usize, usize)> = Default::default();
        for ri in 0..nrows {
            for ci in 0..ncols {
                if let Some(cr) = &grid[ri][ci] {
                    if cr.vmerge.as_deref() == Some("restart") {
                        let mut n = 1;
                        while ri + n < nrows {
                            match &grid[ri + n][ci] {
                                Some(below) if below.vmerge.as_deref() == Some("continue") => {
                                    skip.insert((ri + n, ci));
                                    n += 1;
                                }
                                _ => break,
                            }
                        }
                        rowspan.insert((ri, ci), n);
                    } else if cr.vmerge.as_deref() == Some("continue") && ri == 0 {
                    }
                }
            }
        }
        let cm = props.cell_margins;
        let col_list: Vec<String> = cols.iter().map(|c| pt(*c)).collect();
        let mut body = String::new();
        let banded_rows = props.row_band.max(1);
        let header_rows: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| r.child(W, "trPr").and_then(|t| t.child(W, "tblHeader")).map(on).unwrap_or(false)).map(|(i, _)| i).collect();
        let mut body_row = 0usize;
        for (ri, r) in rows.iter().enumerate() {
            let cells = row_cells(r);
            let trpr = r.child(W, "trPr");
            let height = trpr.and_then(|t| t.child(W, "trHeight"));
            let row_h = height.and_then(|h| twip(h.val()));
            let h_rule = height.and_then(|h| h.own("hRule")).unwrap_or("atLeast");
            let is_first = ri == 0;
            let is_last = ri + 1 == nrows;
            let is_header = header_rows.contains(&ri);
            if !(is_first && flags.first_row) && !(is_last && flags.last_row) {
                body_row += 1;
            }
            let mut col = grid_before(r);
            if col > 0 {
                let _ = write!(body, "table.cell(colspan: {col}, stroke: none)[], ");
            }
            for (ci, c) in cells.iter().enumerate() {
                let span = span_of(c).max(1);
                let gc = col;
                col += span;
                if skip.contains(&(ri, gc)) {
                    continue;
                }
                // Conditional formats in order of precedence (later wins).
                let mut kinds: Vec<&str> = vec!["wholeTable"];
                if !flags.no_hband && !(is_first && flags.first_row) && !(is_last && flags.last_row) {
                    kinds.push(if ((body_row - 1) / banded_rows) % 2 == 0 { "band1Horz" } else { "band2Horz" });
                }
                if !flags.no_vband {
                    let vc = gc + if flags.first_col { 0 } else { 1 };
                    kinds.push(if (vc / props.col_band.max(1)) % 2 == 1 { "band1Vert" } else { "band2Vert" });
                }
                if gc == 0 && flags.first_col {
                    kinds.push("firstCol");
                }
                if col >= ncols && flags.last_col {
                    kinds.push("lastCol");
                }
                if is_first && flags.first_row {
                    kinds.push("firstRow");
                }
                if is_last && flags.last_row {
                    kinds.push("lastRow");
                }
                if is_first && flags.first_row && gc == 0 && flags.first_col {
                    kinds.push("nwCell");
                }
                if is_first && flags.first_row && col >= ncols && flags.last_col {
                    kinds.push("neCell");
                }
                let mut cell_props = props.clone();
                let mut tctx = TableCtx { style: style.clone(), run_props: Vec::new(), para_props: Vec::new() };
                for k in &kinds {
                    for (ck, ce) in &conds {
                        if ck == k {
                            if let Some(t) = ce.child(W, "tblPr") {
                                cell_props.apply(t, &self.theme);
                            }
                            if let Some(t) = ce.child(W, "tcPr") {
                                cell_props.apply_tc(t, &self.theme);
                            }
                            if let Some(rp) = ce.child(W, "rPr") {
                                tctx.run_props.push(rp.clone());
                            }
                            if let Some(pp) = ce.child(W, "pPr") {
                                tctx.para_props.push(pp.clone());
                            }
                        }
                    }
                }
                if let Some(tcpr) = c.child(W, "tcPr") {
                    cell_props.apply_tc(tcpr, &self.theme);
                }
                // Borders: outer edges from top/bottom/left/right, inner from insideH/V.
                let rs = rowspan.get(&(ri, gc)).copied().unwrap_or(1);
                let top = if ri == 0 { cell_props.b("top") } else { cell_props.b_cell("top").or_else(|| cell_props.b("insideH")) };
                let bottom = if ri + rs >= nrows { cell_props.b("bottom") } else { cell_props.b_cell("bottom").or_else(|| cell_props.b("insideH")) };
                let left = if gc == 0 { cell_props.b("left") } else { cell_props.b_cell("left").or_else(|| cell_props.b("insideV")) };
                let right = if col >= ncols { cell_props.b("right") } else { cell_props.b_cell("right").or_else(|| cell_props.b("insideV")) };
                let stroke_part = |b: Option<(f64, String)>| match b {
                    Some((w, c)) => format!("{} + {}", pt(w), rgb(&c)),
                    None => "none".into(),
                };
                let stroke = format!("(top: {}, bottom: {}, left: {}, right: {})", stroke_part(top), stroke_part(bottom), stroke_part(left), stroke_part(right));
                let m = cell_props.tc_margins.unwrap_or(cm);
                let inset = format!("(top: {}, bottom: {}, left: {}, right: {})", pt(m.0), pt(m.2), pt(m.3), pt(m.1));
                let valign = match cell_props.valign.as_deref() {
                    Some("center") => "horizon",
                    Some("bottom") => "bottom",
                    _ => "top",
                };
                // Content.
                let saved = self.area_width;
                let cw: f64 = cols[gc..(gc + span).min(ncols)].iter().sum::<f64>() - m.1 - m.3;
                self.area_width = cw.max(10.0);
                let mut content = String::new();
                let mut fst = Flow { first_in_cell: true, ..Default::default() };
                self.blocks(c, &mut content, Some(&tctx), &mut fst);
                // The last paragraph's space after is part of the cell.
                if fst.prev_after > 0.0 {
                    let _ = write!(content, "#block(above: 0pt, height: {}, width: 0pt)", pt(fst.prev_after));
                }
                self.area_width = saved;
                if let (Some(h), true) = (row_h, ci == 0) {
                    if h_rule != "exact" {
                        // At least this tall: a strut beside the content.
                        content = format!("#grid(columns: (0pt, 1fr), box(height: {}), [{content}])", pt((h - m.0 - m.2).max(0.0)));
                    }
                }
                let mut args = format!("colspan: {span}, rowspan: {rs}, stroke: {stroke}, inset: {inset}, align: {valign}");
                if let Some(f) = &cell_props.shade {
                    let _ = write!(args, ", fill: {}", rgb(f));
                }
                let _ = write!(body, "table.cell({args})[{content}], ");
            }
            // Missing cells at the end of a row.
            if col < ncols {
                let _ = write!(body, "table.cell(colspan: {}, stroke: none)[], ", ncols - col);
            }
            let _ = is_header;
        }
        let rows_spec: Vec<String> = rows
            .iter()
            .map(|r| {
                let height = r.child(W, "trPr").and_then(|t| t.child(W, "trHeight"));
                match (height.and_then(|h| twip(h.val())), height.and_then(|h| h.own("hRule"))) {
                    (Some(h), Some("exact")) => pt(h),
                    _ => "auto".into(),
                }
            })
            .collect();
        let header_n = header_rows.iter().take_while(|&&i| header_rows.contains(&i)).count();
        let mut table = format!("table(columns: ({}), rows: ({}), ", col_list.join(", "), rows_spec.join(", "));
        if header_n > 0 && header_rows[0] == 0 {
            // Repeated header rows are a table.header in typst; kept simple: not repeated.
        }
        table.push_str(&body);
        table.push(')');
        let indent = props.indent.unwrap_or(0.0) - cm.3;
        let aligned = match props.jc.as_deref() {
            Some("center") => format!("align(center, {table})"),
            Some("right") | Some("end") => format!("align(right, {table})"),
            _ if indent != 0.0 => format!("pad(left: {}, {table})", pt(indent)),
            _ => table,
        };
        let gap = st.prev_after;
        let _ = writeln!(out, "#block(above: {}, below: 0pt, {aligned})", pt(gap));
    }
}

fn is_hangul(c: char) -> bool {
    matches!(c as u32, 0xAC00..=0xD7A3 | 0x1100..=0x11FF | 0x3130..=0x318F)
}

fn lang_code(l: &str) -> &'static str {
    let l = l.to_ascii_lowercase();
    if l.starts_with("ko") {
        "ko"
    } else if l.starts_with("ja") {
        "ja"
    } else if l.starts_with("zh") {
        "zh"
    } else if l.starts_with("de") {
        "de"
    } else if l.starts_with("fr") {
        "fr"
    } else if l.starts_with("es") {
        "es"
    } else {
        "en"
    }
}

/// A Typst expression giving (as a string) the value of the fields whose result depends on the
/// page: PAGE, NUMPAGES, SECTIONPAGES.
fn field_code(instr: &str) -> Option<String> {
    let mut words = instr.split_whitespace();
    let name = words.next()?.to_ascii_uppercase();
    let rest: Vec<&str> = words.collect();
    let fmt = if rest.iter().any(|w| *w == "ROMAN" || *w == "Roman") {
        "I"
    } else if rest.iter().any(|w| w.eq_ignore_ascii_case("roman")) {
        "i"
    } else if rest.iter().any(|w| *w == "ALPHABETIC") {
        "A"
    } else if rest.iter().any(|w| w.eq_ignore_ascii_case("alphabetic")) {
        "a"
    } else {
        "1"
    };
    match name.as_str() {
        "PAGE" => Some(format!("counter(page).display({})", s(fmt))),
        "NUMPAGES" | "SECTIONPAGES" => Some("str(counter(page).final().first())".into()),
        _ => None,
    }
}

#[derive(Default)]
struct Flow {
    prev_after: f64,
    prev_style: Option<String>,
    prev_contextual: bool,
    first_in_cell: bool,
}

struct RunCtx {
    base: RP,
    label: String,
    /// The tab stops (Typst array) and the origin of the paragraph's lines.
    tabs: String,
    origin_offset: f64,
    /// Open fields: (instruction, past `separate`, result replaced by a computed value).
    field: Vec<(String, bool, bool)>,
    /// The paragraph's content, one piece per page (split at page breaks).
    pieces: Vec<String>,
    /// How many tab calls are open in the current piece.
    open_tabs: Vec<usize>,
    hyperlink: Option<String>,
    anchors: Vec<String>,
    /// Drawings the text wraps around (square, tight, through) beside the paragraph: (on the
    /// left, the drawing, its width with the gap to the text, its offset down).
    floats: Vec<(bool, String, f64, f64)>,
    default_tab: f64,
}

impl RunCtx {
    fn push(&mut self, s: &str) {
        self.pieces.last_mut().unwrap().push_str(s);
    }
    fn in_field_code(&self) -> bool {
        self.field.iter().any(|f| !f.1)
    }
    /// Inside a field whose result was replaced by a computed value: its cached result is not
    /// shown.
    fn suppressed(&self) -> bool {
        self.field.iter().any(|f| !f.1 || f.2)
    }
    fn field_push(&mut self, s: &str) {
        // Pushed while the field's own flag is set; bypass suppression.
        self.pieces.last_mut().unwrap().push_str(s);
    }
    fn tab(&mut self) {
        let origin = format!("<{}>", self.label);
        let off = super::typ::pt(self.origin_offset);
        let call = format!("#docu-tab({}, {}, {origin}, {off}, l => text(l), [", self.tabs, super::typ::pt(self.default_tab));
        self.push(&call);
        *self.open_tabs.last_mut().unwrap() += 1;
    }
    fn close_tabs(&mut self) {
        let n = *self.open_tabs.last().unwrap();
        for _ in 0..n {
            self.push("])");
        }
        *self.open_tabs.last_mut().unwrap() = 0;
    }
}

#[derive(Clone)]
struct CellRef {
    #[allow(dead_code)]
    row: usize,
    #[allow(dead_code)]
    idx: usize,
    #[allow(dead_code)]
    span: usize,
    vmerge: Option<String>,
}

struct Look {
    first_row: bool,
    last_row: bool,
    first_col: bool,
    last_col: bool,
    no_hband: bool,
    no_vband: bool,
}

#[derive(Clone, Debug)]
enum TW {
    Dxa(f64),
    Pct(f64),
    Auto,
}

#[derive(Clone, Debug)]
struct TableProps {
    width: Option<TW>,
    jc: Option<String>,
    indent: Option<f64>,
    borders: HashMap<String, Option<(f64, String)>>,
    cell_borders: HashMap<String, Option<(f64, String)>>,
    cell_margins: (f64, f64, f64, f64), // top right bottom left
    tc_margins: Option<(f64, f64, f64, f64)>,
    shade: Option<String>,
    valign: Option<String>,
    row_band: usize,
    col_band: usize,
}

impl Default for TableProps {
    fn default() -> Self {
        TableProps { width: None, jc: None, indent: None, borders: HashMap::new(), cell_borders: HashMap::new(), cell_margins: (0.0, 5.4, 0.0, 5.4), tc_margins: None, shade: None, valign: None, row_band: 1, col_band: 1 }
    }
}

fn border_val(b: &Element, theme: &Theme) -> Option<(f64, String)> {
    let v = b.val().unwrap_or("nil");
    if v == "nil" || v == "none" {
        return None;
    }
    let w = b.own("sz").and_then(|v| v.parse::<f64>().ok()).unwrap_or(4.0) / 8.0;
    let w = if v == "double" { w * 3.0 } else { w };
    let mut c = b.own("color").filter(|c| *c != "auto").unwrap_or("000000").to_ascii_uppercase();
    if let Some(t) = b.own("themeColor").and_then(|t| theme.color(t)) {
        c = office::wml_tint_shade(&t, b.own("themeTint"), b.own("themeShade"));
    }
    Some((w.max(0.25), c))
}

impl TableProps {
    fn apply(&mut self, t: &Element, theme: &Theme) {
        for e in t.elements() {
            match e.local() {
                "tblW" => {
                    self.width = match e.own("type") {
                        Some("dxa") => twip(e.own("w")).map(TW::Dxa),
                        Some("pct") => e.own("w").map(|w| {
                            let w = w.trim_end_matches('%');
                            let v: f64 = w.parse().unwrap_or(0.0);
                            TW::Pct(if e.own("w").map(|x| x.ends_with('%')).unwrap_or(false) { v / 100.0 } else { v / 5000.0 })
                        }),
                        _ => Some(TW::Auto),
                    }
                }
                "jc" => self.jc = e.val().map(str::to_string),
                "tblInd" => self.indent = twip(e.own("w")),
                "tblBorders" => {
                    for b in e.elements() {
                        let k = match b.local() {
                            "start" => "left",
                            "end" => "right",
                            x => x,
                        };
                        self.borders.insert(k.to_string(), border_val(b, theme));
                    }
                }
                "tblCellMar" => {
                    for m in e.elements() {
                        let v = twip(m.own("w")).unwrap_or(0.0);
                        match m.local() {
                            "top" => self.cell_margins.0 = v,
                            "right" | "end" => self.cell_margins.1 = v,
                            "bottom" => self.cell_margins.2 = v,
                            "left" | "start" => self.cell_margins.3 = v,
                            _ => {}
                        }
                    }
                }
                "shd" => {
                    if let Some(f) = e.own("fill").filter(|f| *f != "auto") {
                        self.shade = Some(f.to_ascii_uppercase());
                    }
                }
                "tblStyleRowBandSize" => self.row_band = e.val().and_then(|v| v.parse().ok()).unwrap_or(1),
                "tblStyleColBandSize" => self.col_band = e.val().and_then(|v| v.parse().ok()).unwrap_or(1),
                _ => {}
            }
        }
    }

    fn apply_tc(&mut self, t: &Element, theme: &Theme) {
        for e in t.elements() {
            match e.local() {
                "tcBorders" => {
                    for b in e.elements() {
                        let k = match b.local() {
                            "start" => "left",
                            "end" => "right",
                            x => x,
                        };
                        self.cell_borders.insert(k.to_string(), border_val(b, theme));
                    }
                }
                "shd" => {
                    if let Some(f) = e.own("fill").filter(|f| *f != "auto") {
                        self.shade = Some(f.to_ascii_uppercase());
                    } else if let Some(tc) = e.own("themeFill").and_then(|t| theme.color(t)) {
                        self.shade = Some(office::wml_tint_shade(&tc, e.own("themeFillTint"), e.own("themeFillShade")));
                    } else if e.own("fill") == Some("auto") && e.val() == Some("clear") {
                        self.shade = None;
                    }
                }
                "vAlign" => self.valign = e.val().map(str::to_string),
                "tcMar" => {
                    let mut m = self.tc_margins.unwrap_or(self.cell_margins);
                    for x in e.elements() {
                        let v = twip(x.own("w")).unwrap_or(0.0);
                        match x.local() {
                            "top" => m.0 = v,
                            "right" | "end" => m.1 = v,
                            "bottom" => m.2 = v,
                            "left" | "start" => m.3 = v,
                            _ => {}
                        }
                    }
                    self.tc_margins = Some(m);
                }
                _ => {}
            }
        }
    }

    fn b(&self, k: &str) -> Option<(f64, String)> {
        let side = match k {
            "insideH" | "insideV" => None,
            k => self.cell_borders.get(k).cloned(),
        };
        match side {
            Some(v) => v,
            None => self.borders.get(k).cloned().flatten(),
        }
    }

    fn b_cell(&self, k: &str) -> Option<(f64, String)> {
        self.cell_borders.get(k).cloned().flatten()
    }
}

/// Run and paragraph properties a table style's conditional formats give its cells' text.
struct TableCtx {
    style: Option<String>,
    run_props: Vec<Element>,
    #[allow(dead_code)]
    para_props: Vec<Element>,
}

impl TableCtx {
    fn apply_run(&self, base: &RP, theme: &Theme) -> RP {
        let mut r = base.clone();
        for e in &self.run_props {
            r.apply(e, theme);
        }
        r
    }
}

fn collect_rows(tbl: &Element) -> Vec<&Element> {
    let mut out = Vec::new();
    fn walk<'a>(e: &'a Element, out: &mut Vec<&'a Element>) {
        for c in e.elements() {
            if c.is(W, "tr") {
                out.push(c);
            } else if c.is(W, "sdt") || c.is(W, "sdtContent") || c.is(W, "customXml") {
                walk(c, out);
            }
        }
    }
    walk(tbl, &mut out);
    out
}

fn row_cells(tr: &Element) -> Vec<&Element> {
    let mut out = Vec::new();
    fn walk<'a>(e: &'a Element, out: &mut Vec<&'a Element>) {
        for c in e.elements() {
            if c.is(W, "tc") {
                out.push(c);
            } else if c.is(W, "sdt") || c.is(W, "sdtContent") || c.is(W, "customXml") {
                walk(c, out);
            }
        }
    }
    walk(tr, &mut out);
    out
}

fn span_of(tc: &Element) -> usize {
    tc.child(W, "tcPr").and_then(|p| p.child(W, "gridSpan")).and_then(|g| g.val()).and_then(|v| v.parse().ok()).unwrap_or(1)
}

fn vmerge_of(tc: &Element) -> Option<String> {
    let v = tc.child(W, "tcPr").and_then(|p| p.child(W, "vMerge"))?;
    Some(if v.val() == Some("restart") { "restart".into() } else { "continue".into() })
}

fn grid_before(tr: &Element) -> usize {
    tr.child(W, "trPr").and_then(|p| p.child(W, "gridBefore")).and_then(|g| g.val()).and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// `width:100pt;height:50pt` (VML style) in points.
fn vml_size(style: &str) -> (f64, f64) {
    let mut w = 72.0;
    let mut h = 72.0;
    for part in style.split(';') {
        let Some((k, v)) = part.split_once(':') else { continue };
        let v = crate::props::length(v.trim()).unwrap_or(72.0);
        match k.trim() {
            "width" => w = v,
            "height" => h = v,
            _ => {}
        }
    }
    (w, h)
}

trait CloneShallow {
    fn clone_shallow(&self) -> (String, HashMap<u32, (Option<i64>, Option<Level>)>);
}

impl CloneShallow for (String, HashMap<u32, (Option<i64>, Option<Level>)>) {
    fn clone_shallow(&self) -> (String, HashMap<u32, (Option<i64>, Option<Level>)>) {
        (self.0.clone(), self.1.iter().map(|(k, v)| (*k, (v.0, None))).collect())
    }
}

#[allow(dead_code)]
fn unused(_: &Node) {}
