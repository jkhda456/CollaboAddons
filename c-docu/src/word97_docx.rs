//! A .doc as a .docx: the text with its character and paragraph properties (sprms read from the
//! formatting pages), styles, fonts, sections (page size, margins, columns, headers and footers),
//! tables, lists, footnotes and inline pictures, written as WordprocessingML — for rendering and
//! for `docu convert`.

use crate::word97::Doc;
use crate::xml::escape;
use crate::zip::Zip;
use crate::Result;
use std::collections::HashMap;
use std::fmt::Write;

fn u16_at(d: &[u8], i: usize) -> u16 {
    d.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0)
}
fn i16_at(d: &[u8], i: usize) -> i16 {
    u16_at(d, i) as i16
}
fn u32_at(d: &[u8], i: usize) -> u32 {
    d.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0)
}

fn attr(s: &str) -> String {
    let mut o = String::new();
    crate::xml::escape_attr(s, &mut o);
    o
}

/// The sprms of a grpprl: (opcode, operand).
pub fn sprms(g: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 <= g.len() {
        let op = u16_at(g, i);
        i += 2;
        let n = match op >> 13 {
            0 | 1 => 1,
            2 | 4 | 5 => 2,
            3 => 4,
            7 => 3,
            _ => {
                if op == 0xD608 {
                    // sprmTDefTable: a two-byte length (counting itself minus one).
                    let cb = u16_at(g, i) as usize;
                    i += 2;
                    let len = cb.saturating_sub(1);
                    out.push((op, g.get(i..(i + len).min(g.len())).unwrap_or(&[]).to_vec()));
                    i += len;
                    continue;
                }
                if op == 0xC615 {
                    let cb = *g.get(i).unwrap_or(&0) as usize;
                    i += 1;
                    let len = if cb == 255 {
                        // Delete tabs by distance: computed.
                        let del = *g.get(i).unwrap_or(&0) as usize;
                        let add = *g.get(i + 1 + del * 4).unwrap_or(&0) as usize;
                        1 + del * 4 + 1 + add * 3
                    } else {
                        cb
                    };
                    out.push((op, g.get(i..(i + len).min(g.len())).unwrap_or(&[]).to_vec()));
                    i += len;
                    continue;
                }
                let cb = *g.get(i).unwrap_or(&0) as usize;
                i += 1;
                cb
            }
        };
        if i + n > g.len() {
            break;
        }
        out.push((op, g[i..i + n].to_vec()));
        i += n;
    }
    out
}

const ICO: [&str; 17] = ["auto", "000000", "0000FF", "00FFFF", "00FF00", "FF00FF", "FF0000", "FFFF00", "FFFFFF", "000080", "008080", "008000", "800080", "800000", "808000", "808080", "C0C0C0"];
const HIGHLIGHT: [&str; 17] = ["none", "black", "blue", "cyan", "green", "magenta", "red", "yellow", "white", "darkBlue", "darkCyan", "darkGreen", "darkMagenta", "darkRed", "darkYellow", "darkGray", "lightGray"];

fn colorref(b: &[u8]) -> Option<String> {
    if b.len() < 4 || b[3] == 0xFF {
        return None;
    }
    Some(format!("{:02X}{:02X}{:02X}", b[0], b[1], b[2]))
}

fn toggle(v: u8) -> Option<bool> {
    match v {
        0 => Some(false),
        1 | 0x81 => Some(true),
        _ => None,
    }
}

/// Character properties as `w:rPr` children (in schema order), and whether the text is deleted.
pub fn rpr_xml(grpprl: &[u8], fonts: &[String], styles: &HashMap<u16, String>) -> (String, bool, Option<u32>) {
    let mut m: HashMap<&str, String> = HashMap::new();
    let mut deleted = false;
    let mut pic = None;
    let mut spec = false;
    for (op, v) in sprms(grpprl) {
        let b = |k: usize| v.get(k).copied().unwrap_or(0);
        match op {
            0x4A30 => {
                if let Some(s) = styles.get(&u16_at(&v, 0)) {
                    m.insert("rStyle", format!("<w:rStyle w:val=\"{}\"/>", attr(s)));
                }
            }
            0x4A4F | 0x4A50 | 0x4A51 => {
                let f = fonts.get(u16_at(&v, 0) as usize).cloned().unwrap_or_default();
                if !f.is_empty() {
                    let k = match op {
                        0x4A4F => "ascii",
                        0x4A50 => "eastAsia",
                        _ => "hAnsi",
                    };
                    let e = m.entry("rFonts").or_insert_with(|| "<w:rFonts/>".into());
                    *e = e.replace("/>", &format!(" w:{k}=\"{}\"/>", attr(&f)));
                }
            }
            0x0835 => {
                if let Some(t) = toggle(b(0)) {
                    m.insert("b", if t { "<w:b/>".into() } else { "<w:b w:val=\"0\"/>".into() });
                }
            }
            0x0836 => {
                if let Some(t) = toggle(b(0)) {
                    m.insert("i", if t { "<w:i/>".into() } else { "<w:i w:val=\"0\"/>".into() });
                }
            }
            0x083B => {
                if let Some(t) = toggle(b(0)) {
                    m.insert("caps", format!("<w:caps w:val=\"{}\"/>", t as u8));
                }
            }
            0x083A => {
                if let Some(t) = toggle(b(0)) {
                    m.insert("smallCaps", format!("<w:smallCaps w:val=\"{}\"/>", t as u8));
                }
            }
            0x0837 => {
                if let Some(t) = toggle(b(0)) {
                    m.insert("strike", format!("<w:strike w:val=\"{}\"/>", t as u8));
                }
            }
            0x2A53 => {
                if let Some(t) = toggle(b(0)) {
                    m.insert("dstrike", format!("<w:dstrike w:val=\"{}\"/>", t as u8));
                }
            }
            0x083C => {
                if let Some(t) = toggle(b(0)) {
                    m.insert("vanish", format!("<w:vanish w:val=\"{}\"/>", t as u8));
                }
            }
            0x2A42 => {
                if let Some(c) = ICO.get(b(0) as usize) {
                    m.insert("color", format!("<w:color w:val=\"{c}\"/>"));
                }
            }
            0x6870 => {
                if let Some(c) = colorref(&v) {
                    m.insert("color", format!("<w:color w:val=\"{c}\"/>"));
                }
            }
            0x8840 => {
                m.insert("spacing", format!("<w:spacing w:val=\"{}\"/>", i16_at(&v, 0)));
            }
            0x4852 => {
                m.insert("w", format!("<w:w w:val=\"{}\"/>", u16_at(&v, 0)));
            }
            0x4845 => {
                m.insert("position", format!("<w:position w:val=\"{}\"/>", i16_at(&v, 0)));
            }
            0x4A43 => {
                m.insert("sz", format!("<w:sz w:val=\"{}\"/>", u16_at(&v, 0)));
            }
            0x2A0C => {
                if let Some(h) = HIGHLIGHT.get(b(0) as usize) {
                    if *h != "none" {
                        m.insert("highlight", format!("<w:highlight w:val=\"{h}\"/>"));
                    }
                }
            }
            0x2A3E => {
                let u = match b(0) {
                    0 => "none",
                    2 => "words",
                    3 => "double",
                    4 => "dotted",
                    6 => "thick",
                    7 => "dash",
                    9 => "dotDash",
                    10 => "dotDotDash",
                    11 => "wave",
                    _ => "single",
                };
                m.insert("u", format!("<w:u w:val=\"{u}\"/>"));
            }
            0xCA71 => {
                if let Some(c) = v.get(4..8).and_then(colorref) {
                    m.insert("shd", format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{c}\"/>"));
                }
            }
            0x2A48 => {
                let s = match b(0) {
                    1 => "superscript",
                    2 => "subscript",
                    _ => "baseline",
                };
                m.insert("vertAlign", format!("<w:vertAlign w:val=\"{s}\"/>"));
            }
            0x0800 => deleted = b(0) == 1,
            0x0855 => spec = b(0) == 1,
            0x6A03 => pic = Some(u32_at(&v, 0)),
            _ => {}
        }
    }
    let order = ["rStyle", "rFonts", "b", "i", "caps", "smallCaps", "strike", "dstrike", "vanish", "color", "spacing", "w", "position", "sz", "highlight", "u", "shd", "vertAlign"];
    let mut out = String::new();
    for k in order {
        if let Some(x) = m.get(k) {
            out.push_str(x);
        }
    }
    (out, deleted, if spec { pic } else { None })
}

#[derive(Default, Clone)]
struct Pap {
    xml: String,
    in_table: bool,
    ttp: bool,
    itap: u16,
    ilfo: u16,
    ilvl: u8,
    tap: Vec<(u16, Vec<u8>)>,
}

fn brc80(b: &[u8]) -> Option<(f64, String)> {
    if b.len() < 4 || b[1] == 0 || b[1] == 0xFF {
        return None;
    }
    Some((b[0] as f64, ICO.get(b[2] as usize).map(|s| s.to_string()).unwrap_or("auto".into())))
}

fn border_xml(name: &str, b: Option<(f64, String)>) -> String {
    match b {
        Some((w, c)) => format!("<w:{name} w:val=\"single\" w:sz=\"{}\" w:space=\"0\" w:color=\"{c}\"/>", w.max(2.0) as i64),
        None => String::new(),
    }
}

fn pap(grpprl: &[u8], styles: &HashMap<u16, String>, istd: u16) -> Pap {
    let mut p = Pap::default();
    let mut m: HashMap<&str, String> = HashMap::new();
    if let Some(s) = styles.get(&istd) {
        if istd != 0 {
            m.insert("pStyle", format!("<w:pStyle w:val=\"{}\"/>", attr(s)));
        }
    }
    let mut ind: HashMap<&str, i32> = HashMap::new();
    let mut spacing: HashMap<&str, String> = HashMap::new();
    let mut tabs: Vec<(i16, u8)> = Vec::new();
    let mut borders: HashMap<&str, String> = HashMap::new();
    for (op, v) in sprms(grpprl) {
        let b = |k: usize| v.get(k).copied().unwrap_or(0);
        match op {
            0x2403 | 0x2461 => {
                let j = match b(0) {
                    1 => "center",
                    2 => "right",
                    3 => "both",
                    4 => "distribute",
                    _ => "left",
                };
                m.insert("jc", format!("<w:jc w:val=\"{j}\"/>"));
            }
            0x2405 => {
                m.insert("keepLines", format!("<w:keepLines w:val=\"{}\"/>", (b(0) != 0) as u8));
            }
            0x2406 => {
                m.insert("keepNext", format!("<w:keepNext w:val=\"{}\"/>", (b(0) != 0) as u8));
            }
            0x2407 => {
                m.insert("pageBreakBefore", format!("<w:pageBreakBefore w:val=\"{}\"/>", (b(0) != 0) as u8));
            }
            0x840E | 0x845D => {
                ind.insert("right", i16_at(&v, 0) as i32);
            }
            0x840F | 0x845E => {
                ind.insert("left", i16_at(&v, 0) as i32);
            }
            0x8411 | 0x8460 => {
                ind.insert("first", i16_at(&v, 0) as i32);
            }
            0xA413 => {
                spacing.insert("before", format!("w:before=\"{}\"", u16_at(&v, 0)));
            }
            0xA414 => {
                spacing.insert("after", format!("w:after=\"{}\"", u16_at(&v, 0)));
            }
            0x245B => {
                if b(0) != 0 {
                    spacing.insert("beforeAuto", "w:beforeAutospacing=\"1\"".into());
                }
            }
            0x245C => {
                if b(0) != 0 {
                    spacing.insert("afterAuto", "w:afterAutospacing=\"1\"".into());
                }
            }
            0x6412 => {
                let line = i16_at(&v, 0);
                let mult = i16_at(&v, 2);
                let (val, rule) = if mult == 1 { (line as i32, "auto") } else if line < 0 { (-(line as i32), "exact") } else { (line as i32, "atLeast") };
                spacing.insert("line", format!("w:line=\"{val}\" w:lineRule=\"{rule}\""));
            }
            0x260A => p.ilvl = b(0),
            0x460B => p.ilfo = u16_at(&v, 0),
            0x2416 => p.in_table = b(0) != 0,
            0x2417 => p.ttp = b(0) != 0,
            0x6649 => p.itap = u32_at(&v, 0) as u16,
            0x244B => {
                if b(0) != 0 {
                    p.in_table = true;
                }
            }
            0x244C => {
                if b(0) != 0 {
                    p.ttp = true;
                }
            }
            0x442D => {
                let x = u16_at(&v, 0);
                let back = ((x >> 5) & 0x1F) as usize;
                if back > 0 && back < ICO.len() {
                    m.insert("shd", format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", ICO[back]));
                }
            }
            0xC64D => {
                if let Some(c) = v.get(4..8).and_then(colorref) {
                    m.insert("shd", format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{c}\"/>"));
                }
            }
            0x2640 => {
                if b(0) < 9 {
                    m.insert("outlineLvl", format!("<w:outlineLvl w:val=\"{}\"/>", b(0)));
                }
            }
            0x6424 | 0x6425 | 0x6426 | 0x6427 => {
                let name = match op {
                    0x6424 => "top",
                    0x6425 => "left",
                    0x6426 => "bottom",
                    _ => "right",
                };
                let x = border_xml(name, brc80(&v));
                if !x.is_empty() {
                    borders.insert(name, x);
                }
            }
            0xC60D | 0xC615 => {
                // Tabs added: count, positions, then types.
                let del = b(0) as usize;
                let at = 1 + del * 2 * if op == 0xC615 { 2 } else { 1 };
                let add = *v.get(at).unwrap_or(&0) as usize;
                for k in 0..add {
                    let pos = i16_at(&v, at + 1 + k * 2);
                    let tbd = *v.get(at + 1 + add * 2 + k).unwrap_or(&0);
                    tabs.push((pos, tbd));
                }
            }
            0x246D => {
                m.insert("contextualSpacing", "<w:contextualSpacing/>".into());
            }
            0x2441 => {
                if b(0) != 0 {
                    m.insert("bidi", "<w:bidi/>".into());
                }
            }
            op if (op >> 10) & 7 == 5 => p.tap.push((op, v.clone())),
            _ => {}
        }
    }
    if !tabs.is_empty() {
        let mut t = String::from("<w:tabs>");
        tabs.sort();
        for (pos, tbd) in tabs {
            let val = match tbd & 7 {
                1 => "center",
                2 => "right",
                3 => "decimal",
                4 => "bar",
                _ => "left",
            };
            let leader = match (tbd >> 3) & 7 {
                1 => " w:leader=\"dot\"",
                2 => " w:leader=\"hyphen\"",
                3 => " w:leader=\"underscore\"",
                4 => " w:leader=\"heavy\"",
                5 => " w:leader=\"middleDot\"",
                _ => "",
            };
            let _ = write!(t, "<w:tab w:val=\"{val}\" w:pos=\"{pos}\"{leader}/>");
        }
        t.push_str("</w:tabs>");
        m.insert("tabs", t);
    }
    if !borders.is_empty() {
        let mut x = String::from("<w:pBdr>");
        for k in ["top", "left", "bottom", "right"] {
            if let Some(b) = borders.get(k) {
                x.push_str(b);
            }
        }
        x.push_str("</w:pBdr>");
        m.insert("pBdr", x);
    }
    if !spacing.is_empty() {
        let mut parts: Vec<&String> = spacing.values().collect();
        parts.sort();
        m.insert("spacing", format!("<w:spacing {}/>", parts.into_iter().cloned().collect::<Vec<_>>().join(" ")));
    }
    if !ind.is_empty() {
        let mut x = String::from("<w:ind");
        if let Some(l) = ind.get("left") {
            let _ = write!(x, " w:left=\"{l}\"");
        }
        if let Some(r) = ind.get("right") {
            let _ = write!(x, " w:right=\"{r}\"");
        }
        if let Some(f) = ind.get("first") {
            if *f < 0 {
                let _ = write!(x, " w:hanging=\"{}\"", -f);
            } else {
                let _ = write!(x, " w:firstLine=\"{f}\"");
            }
        }
        x.push_str("/>");
        m.insert("ind", x);
    }
    let order = ["pStyle", "keepNext", "keepLines", "pageBreakBefore", "numPr", "pBdr", "shd", "tabs", "bidi", "spacing", "ind", "contextualSpacing", "jc", "outlineLvl"];
    for k in order {
        if let Some(x) = m.get(k) {
            p.xml.push_str(x);
        }
    }
    p
}

struct Level {
    start: i32,
    nfc: u8,
    jc: u8,
    follow: u8,
    text: String,
    ppr: String,
    rpr: String,
}

pub struct Conv<'a> {
    d: &'a Doc,
    fonts: Vec<String>,
    style_ids: HashMap<u16, String>,
    styles_xml: String,
    chpx: Vec<(u32, u32, Vec<u8>)>,
    papx: Vec<(u32, u32, u16, Vec<u8>)>,
    lists: Vec<(i32, Vec<Level>)>,
    lfo: Vec<i32>,
    media: Vec<(String, Vec<u8>)>,
    footnotes: Vec<(u32, u32)>,
    headers: Vec<(String, String)>,
    /// The default fonts (ASCII, East Asian, other) of the style sheet.
    default_fonts: [Option<String>; 3],
}

impl<'a> Conv<'a> {
    pub fn new(d: &'a Doc) -> Conv<'a> {
        let mut c = Conv { d, fonts: Vec::new(), style_ids: HashMap::new(), styles_xml: String::new(), chpx: Vec::new(), papx: Vec::new(), lists: Vec::new(), lfo: Vec::new(), media: Vec::new(), footnotes: Vec::new(), headers: Vec::new(), default_fonts: [None, None, None] };
        c.read_fonts();
        c.read_styles();
        c.read_fkps();
        c.read_lists();
        c
    }

    fn read_fonts(&mut self) {
        let Some(t) = self.d.table_slice(15) else { return };
        let n = u16_at(t, 0) as usize;
        let mut i = 4;
        for _ in 0..n {
            let cb = *t.get(i).unwrap_or(&0) as usize + 1;
            let ffn = t.get(i..i + cb).unwrap_or(&[]);
            let mut units = Vec::new();
            let mut k = 40;
            while k + 1 < ffn.len() {
                let u = u16_at(ffn, k);
                if u == 0 {
                    break;
                }
                units.push(u);
                k += 2;
            }
            self.fonts.push(String::from_utf16_lossy(&units));
            i += cb;
        }
    }

    fn read_styles(&mut self) {
        let Some(t) = self.d.table_slice(1) else { return };
        let cb_stshi = u16_at(t, 0) as usize;
        let stshi = &t[2..(2 + cb_stshi).min(t.len())];
        let cstd = u16_at(stshi, 0) as usize;
        let cb_base = u16_at(stshi, 2) as usize;
        for k in 0..3 {
            if stshi.len() >= 14 + k * 2 {
                self.default_fonts[k] = self.fonts.get(u16_at(stshi, 12 + k * 2) as usize).cloned().filter(|f| !f.is_empty());
            }
        }
        let mut i = 2 + cb_stshi;
        let mut defs: Vec<(u16, u8, u16, String, Vec<u8>, Vec<u8>)> = Vec::new(); // istd, stk, base, name, papx, chpx
        for istd in 0..cstd {
            let cb = u16_at(t, i) as usize;
            i += 2;
            if cb == 0 {
                continue;
            }
            let std = t.get(i..i + cb).unwrap_or(&[]).to_vec();
            i += cb;
            if std.len() < cb_base + 2 {
                continue;
            }
            let stk = (u16_at(&std, 2) & 0xF) as u8;
            let base = u16_at(&std, 2) >> 4;
            let cupx = (u16_at(&std, 4) & 0xF) as usize;
            let mut k = cb_base;
            let cch = u16_at(&std, k) as usize;
            let units: Vec<u16> = (0..cch).map(|j| u16_at(&std, k + 2 + j * 2)).collect();
            let name = String::from_utf16_lossy(&units);
            k += 2 + cch * 2 + 2;
            let mut upx: Vec<Vec<u8>> = Vec::new();
            for _ in 0..cupx {
                if k % 2 == 1 {
                    k += 1;
                }
                let n = u16_at(&std, k) as usize;
                upx.push(std.get(k + 2..k + 2 + n).unwrap_or(&[]).to_vec());
                k += 2 + n;
            }
            let (papx, chpx) = match stk {
                1 => (upx.first().cloned().unwrap_or_default(), upx.get(1).cloned().unwrap_or_default()),
                2 => (Vec::new(), upx.first().cloned().unwrap_or_default()),
                _ => (Vec::new(), Vec::new()),
            };
            defs.push((istd as u16, stk, base, name, papx, chpx));
        }
        for (istd, _, _, name, _, _) in &defs {
            let id: String = name.chars().filter(|c| c.is_alphanumeric()).collect();
            let id = if id.is_empty() { format!("S{istd}") } else { id };
            let id = if self.style_ids.values().any(|v| *v == id) { format!("{id}{istd}") } else { id };
            self.style_ids.insert(*istd, id);
        }
        let empty = HashMap::new();
        let mut x = String::new();
        for (istd, stk, base, name, papx, chpx) in &defs {
            let kind = match stk {
                1 => "paragraph",
                2 => "character",
                3 => "table",
                _ => continue,
            };
            let id = &self.style_ids[istd];
            let _ = write!(x, "<w:style w:type=\"{kind}\" w:styleId=\"{}\"{}><w:name w:val=\"{}\"/>", attr(id), if *istd == 0 { " w:default=\"1\"" } else { "" }, attr(name));
            if *base != 0x0FFF {
                if let Some(b) = self.style_ids.get(base) {
                    let _ = write!(x, "<w:basedOn w:val=\"{}\"/>", attr(b));
                }
            }
            if papx.len() >= 2 {
                let p = pap(&papx[2..], &empty, 0);
                if !p.xml.is_empty() {
                    let _ = write!(x, "<w:pPr>{}</w:pPr>", p.xml);
                }
            }
            let (r, _, _) = rpr_xml(chpx, &self.fonts, &empty);
            if !r.is_empty() {
                let _ = write!(x, "<w:rPr>{r}</w:rPr>");
            }
            x.push_str("</w:style>");
        }
        self.styles_xml = x;
    }

    fn read_fkps(&mut self) {
        for (idx, chp) in [(12usize, true), (13, false)] {
            let Some(plc) = self.d.table_slice(idx) else { continue };
            let n = (plc.len().saturating_sub(4)) / 8;
            for k in 0..n {
                let pn = (u32_at(plc, (n + 1) * 4 + k * 4) & 0x3F_FFFF) as usize;
                let Some(page) = self.d.wd_slice(pn * 512, 512) else { continue };
                let crun = page[511] as usize;
                for j in 0..crun {
                    let a = u32_at(page, j * 4);
                    let b = u32_at(page, (j + 1) * 4);
                    if chp {
                        let off = page[(crun + 1) * 4 + j] as usize * 2;
                        let g = if off == 0 { Vec::new() } else { page.get(off + 1..off + 1 + page[off] as usize).unwrap_or(&[]).to_vec() };
                        self.chpx.push((a, b, g));
                    } else {
                        let off = page[(crun + 1) * 4 + j * 13] as usize * 2;
                        if off == 0 {
                            self.papx.push((a, b, 0, Vec::new()));
                            continue;
                        }
                        let cb = page[off] as usize;
                        let (start, len) = if cb == 0 { (off + 2, 2 * page[off + 1] as usize) } else { (off + 1, 2 * cb - 1) };
                        let g = page.get(start..(start + len).min(512)).unwrap_or(&[]).to_vec();
                        let istd = u16_at(&g, 0);
                        self.papx.push((a, b, istd, g.get(2..).unwrap_or(&[]).to_vec()));
                    }
                }
            }
        }
        // Later edits' pages come later: lookups take the last match.
    }

    fn read_lists(&mut self) {
        // The list definitions, then (beyond the pair's length) their levels.
        let Some(t) = self.d.table_from(73) else { return };
        let n = u16_at(t, 0) as usize;
        let mut lstfs = Vec::new();
        for k in 0..n {
            let o = 2 + k * 28;
            lstfs.push((u32_at(t, o) as i32, t.get(o + 26).map(|f| f & 1 != 0).unwrap_or(false)));
        }
        let mut i = 2 + n * 28;
        for (lsid, simple) in lstfs {
            let mut levels = Vec::new();
            for _ in 0..if simple { 1 } else { 9 } {
                let lv = t.get(i..i + 28).unwrap_or(&[0; 28]).to_vec();
                let cb_chpx = lv[24] as usize;
                let cb_papx = lv[25] as usize;
                let papx = t.get(i + 28..i + 28 + cb_papx).unwrap_or(&[]).to_vec();
                let chpx = t.get(i + 28 + cb_papx..i + 28 + cb_papx + cb_chpx).unwrap_or(&[]).to_vec();
                let mut k = i + 28 + cb_papx + cb_chpx;
                let cch = u16_at(t, k) as usize;
                let units: Vec<u16> = (0..cch).map(|j| u16_at(t, k + 2 + j * 2)).collect();
                k += 2 + cch * 2;
                // Placeholders: characters 0..8 at the positions rgbxchNums names.
                let nums: Vec<usize> = lv[6..15].iter().filter(|x| **x > 0).map(|x| *x as usize - 1).collect();
                let mut text = String::new();
                for (pos, u) in units.iter().enumerate() {
                    if nums.contains(&pos) && *u < 9 {
                        let _ = write!(text, "%{}", u + 1);
                    } else {
                        text.push(char::from_u32(*u as u32).unwrap_or('?'));
                    }
                }
                let empty = HashMap::new();
                let p = pap(&papx, &empty, 0);
                let (r, _, _) = rpr_xml(&chpx, &self.fonts, &empty);
                levels.push(Level { start: u32_at(&lv, 0) as i32, nfc: lv[4], jc: lv[5] & 3, follow: lv[15], text, ppr: p.xml, rpr: r });
                i = k;
            }
            self.lists.push((lsid, levels));
        }
        if let Some(lf) = self.d.table_slice(74) {
            let n = u32_at(lf, 0) as usize;
            for k in 0..n {
                self.lfo.push(u32_at(lf, 4 + k * 16) as i32);
            }
        }
    }

    fn chpx_at(&self, fc: u32) -> &[u8] {
        self.chpx.iter().rev().find(|(a, b, _)| *a <= fc && fc < *b).map(|x| x.2.as_slice()).unwrap_or(&[])
    }

    fn papx_at(&self, fc: u32) -> (u16, &[u8]) {
        self.papx.iter().rev().find(|(a, b, _, _)| *a <= fc && fc < *b).map(|x| (x.2, x.3.as_slice())).unwrap_or((0, &[]))
    }

    /// Paragraphs of a CP range as WordprocessingML (tables built from the table paragraphs).
    fn body(&mut self, start: u32, end: u32) -> String {
        let chars = self.d.cp_chars(start, end);
        let mut out = String::new();
        // Split into paragraphs with their CP ranges.
        let mut paras: Vec<(u32, u32, char)> = Vec::new();
        let mut s = start;
        for (i, (_, c)) in chars.iter().enumerate() {
            let cp = start + i as u32;
            if *c == '\r' || *c == '\u{7}' {
                paras.push((s, cp, *c));
                s = cp + 1;
            }
        }
        if s < end {
            paras.push((s, end, '\r'));
        }
        let mut row: Vec<String> = Vec::new();
        let mut cell = String::new();
        let mut rows: Vec<(Vec<String>, Vec<(u16, Vec<u8>)>)> = Vec::new();
        let flush_table = |rows: &mut Vec<(Vec<String>, Vec<(u16, Vec<u8>)>)>, out: &mut String| {
            if rows.is_empty() {
                return;
            }
            out.push_str(&table_xml(rows));
            rows.clear();
        };
        for (a, b, mark) in paras {
            let mark_fc = self.d.fc_of_cp(b).unwrap_or(0);
            let (istd, grpprl) = self.papx_at(mark_fc);
            let grpprl = grpprl.to_vec();
            let p = pap(&grpprl, &self.style_ids, istd);
            let para_xml = self.paragraph(a, b, &p);
            if p.in_table || mark == '\u{7}' {
                if p.ttp || (mark == '\u{7}' && a == b && !cell.is_empty() && false) {
                    rows.push((std::mem::take(&mut row), p.tap.clone()));
                    continue;
                }
                cell.push_str(&para_xml);
                if mark == '\u{7}' {
                    row.push(std::mem::take(&mut cell));
                }
                continue;
            }
            if !row.is_empty() || !cell.is_empty() {
                if !cell.is_empty() {
                    row.push(std::mem::take(&mut cell));
                }
                rows.push((std::mem::take(&mut row), Vec::new()));
            }
            flush_table(&mut rows, &mut out);
            out.push_str(&para_xml);
        }
        if !cell.is_empty() {
            row.push(cell);
        }
        if !row.is_empty() {
            rows.push((row, Vec::new()));
        }
        flush_table(&mut rows, &mut out);
        out
    }

    fn paragraph(&mut self, a: u32, b: u32, p: &Pap) -> String {
        let mut ppr = p.xml.clone();
        if p.ilfo > 0 && (p.ilfo as usize) <= self.lfo.len() {
            let num = format!("<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{}\"/></w:numPr>", p.ilvl, p.ilfo);
            // After pStyle and the keeps.
            let at = ["<w:pBdr", "<w:shd", "<w:tabs", "<w:bidi", "<w:spacing", "<w:ind", "<w:contextualSpacing", "<w:jc", "<w:outlineLvl"].iter().filter_map(|t| ppr.find(t)).min().unwrap_or(ppr.len());
            ppr.insert_str(at, &num);
        }
        let mut out = format!("<w:p><w:pPr>{ppr}</w:pPr>");
        // Runs: change at every CHPX boundary.
        let chars = self.d.cp_chars(a, b);
        let mut cur_g: Option<Vec<u8>> = None;
        let mut text = String::new();
        let mut field: Vec<(String, bool)> = Vec::new();
        let flush = |out: &mut String, g: &Option<Vec<u8>>, text: &mut String, this: &Self| {
            if text.is_empty() {
                return;
            }
            let (r, deleted, _) = rpr_xml(g.as_deref().unwrap_or(&[]), &this.fonts, &this.style_ids);
            if !deleted {
                let _ = write!(out, "<w:r><w:rPr>{r}</w:rPr>{}</w:r>", run_content(text));
            }
            text.clear();
        };
        for (fc, c) in chars {
            let g = self.chpx_at(fc).to_vec();
            if cur_g.as_ref() != Some(&g) {
                flush(&mut out, &cur_g, &mut text, self);
                cur_g = Some(g.clone());
            }
            match c {
                '\u{13}' => {
                    flush(&mut out, &cur_g, &mut text, self);
                    field.push((String::new(), true));
                }
                '\u{14}' => {
                    flush(&mut out, &cur_g, &mut text, self);
                    if let Some(f) = field.last_mut() {
                        f.1 = false;
                        let code = f.0.trim().to_uppercase();
                        if code.starts_with("PAGE") || code.starts_with("NUMPAGES") {
                            let (r, _, _) = rpr_xml(&g, &self.fonts, &self.style_ids);
                            let _ = write!(out, "<w:fldSimple w:instr=\" {} \"><w:r><w:rPr>{r}</w:rPr><w:t>1</w:t></w:r></w:fldSimple>", attr(code.split_whitespace().next().unwrap_or("PAGE")));
                            f.0 = "\u{1}SKIP".into();
                        }
                    }
                }
                '\u{15}' => {
                    flush(&mut out, &cur_g, &mut text, self);
                    field.pop();
                }
                _ if field.iter().any(|f| f.1) => {
                    if let Some(f) = field.last_mut() {
                        if f.1 {
                            f.0.push(c);
                        }
                    }
                }
                _ if field.last().map(|f| f.0 == "\u{1}SKIP").unwrap_or(false) => {}
                '\u{1}' => {
                    flush(&mut out, &cur_g, &mut text, self);
                    let (_, _, pic) = rpr_xml(&g, &self.fonts, &self.style_ids);
                    if let Some(pos) = pic {
                        if let Some(x) = self.picture(pos) {
                            out.push_str(&x);
                        }
                    }
                }
                '\u{2}' => {
                    flush(&mut out, &cur_g, &mut text, self);
                    // A footnote reference: its text from the footnote story, in order.
                    let n = self.footnotes.len() + 1;
                    let range = self.d.footnote_range(n - 1).unwrap_or((0, 0));
                    self.footnotes.push(range);
                    let _ = write!(out, "<w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:footnoteReference w:id=\"{n}\"/></w:r>");
                }
                '\u{8}' | '\u{5}' | '\u{1f}' => {}
                c => text.push(c),
            }
        }
        flush(&mut out, &cur_g, &mut text, self);
        out.push_str("</w:p>");
        out
    }

    fn picture(&mut self, pos: u32) -> Option<String> {
        let data = self.d.data_stream()?;
        let p = pos as usize;
        let lcb = u32_at(data, p) as usize;
        let cb_header = u16_at(data, p + 4) as usize;
        let picf = data.get(p..p + lcb.min(data.len() - p))?;
        let dxa = i16_at(picf, 28) as f64;
        let dya = i16_at(picf, 30) as f64;
        let mx = u16_at(picf, 32) as f64;
        let my = u16_at(picf, 34) as f64;
        let w = (dxa * mx / 1000.0 / 20.0 * 12700.0) as i64;
        let h = (dya * my / 1000.0 / 20.0 * 12700.0) as i64;
        // The first BLIP in the shape container after the header.
        let blob = picf.get(cb_header..)?;
        let (ext, bytes) = find_blip(blob)?;
        let name = format!("image{}.{ext}", self.media.len() + 1);
        self.media.push((name.clone(), bytes));
        let rid = format!("rIdImg{}", self.media.len());
        Some(format!(
            "<w:r><w:drawing><wp:inline><wp:extent cx=\"{w}\" cy=\"{h}\"/><wp:docPr id=\"{}\" name=\"Picture\"/><a:graphic xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:nvPicPr><pic:cNvPr id=\"0\" name=\"{name}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>",
            self.media.len()
        ))
    }

    fn sections(&mut self) -> Vec<(u32, String)> {
        // (end CP, sectPr xml) per section.
        let mut out = Vec::new();
        let Some(plc) = self.d.table_slice(6) else { return out };
        let n = (plc.len().saturating_sub(4)) / 16;
        let hdd = self.d.table_slice(11).map(|h| h.to_vec());
        for k in 0..n {
            let cp_end = u32_at(plc, (k + 1) * 4);
            let sed = &plc[(n + 1) * 4 + k * 12..(n + 1) * 4 + k * 12 + 12];
            let fc = u32_at(sed, 2) as usize;
            let mut g = Vec::new();
            if fc != 0xFFFF_FFFF {
                if let Some(cb) = self.d.wd_slice(fc, 2).map(|b| u16_at(b, 0) as usize) {
                    g = self.d.wd_slice(fc + 2, cb).map(|b| b.to_vec()).unwrap_or_default();
                }
            }
            let (mut w, mut h, mut l, mut r, mut t, mut b, mut hd, mut fd) = (12240i32, 15840i32, 1800i32, 1800i32, 1440i32, 1440i32, 720i32, 720i32);
            let mut cols = 1;
            let mut space = 720;
            let mut title = false;
            let mut kind = "nextPage";
            let mut landscape = false;
            for (op, v) in sprms(&g) {
                match op {
                    0xB01F => w = u16_at(&v, 0) as i32,
                    0xB020 => h = u16_at(&v, 0) as i32,
                    0xB021 => l = u16_at(&v, 0) as i32,
                    0xB022 => r = u16_at(&v, 0) as i32,
                    0x9023 => t = i16_at(&v, 0) as i32,
                    0x9024 => b = i16_at(&v, 0) as i32,
                    0xB017 => hd = u16_at(&v, 0) as i32,
                    0xB018 => fd = u16_at(&v, 0) as i32,
                    0x500B => cols = u16_at(&v, 0) as i32 + 1,
                    0x900C => space = u16_at(&v, 0) as i32,
                    0x300A => title = v.first() == Some(&1),
                    0x301D => landscape = v.first() == Some(&2),
                    0x3009 => {
                        kind = match v.first() {
                            Some(0) => "continuous",
                            Some(1) => "nextColumn",
                            Some(3) => "evenPage",
                            Some(4) => "oddPage",
                            _ => "nextPage",
                        }
                    }
                    _ => {}
                }
            }
            // Headers and footers of this section (PlcfHdd: six per section after the six
            // separators).
            let mut refs = String::new();
            if let Some(hdd) = &hdd {
                let hn = hdd.len() / 4;
                let story_start = self.d.story_start(2);
                for (j, (tag, typ)) in [("header", "even"), ("header", "default"), ("footer", "even"), ("footer", "default"), ("header", "first"), ("footer", "first")].iter().enumerate() {
                    let idx = 6 + k * 6 + j;
                    if idx + 1 >= hn {
                        continue;
                    }
                    let a = u32_at(hdd, idx * 4);
                    let b2 = u32_at(hdd, (idx + 1) * 4);
                    if b2 <= a + 1 {
                        continue;
                    }
                    let body = self.body(story_start + a, story_start + b2 - 1);
                    let rid = format!("rIdH{}", self.headers.len() + 1);
                    self.headers.push((format!("{tag}{}", self.headers.len() + 1), body));
                    let _ = write!(refs, "<w:{tag}Reference w:type=\"{typ}\" r:id=\"{rid}\"/>");
                }
            }
            if landscape && w < h {
                std::mem::swap(&mut w, &mut h);
            }
            let x = format!(
                "<w:sectPr>{refs}<w:type w:val=\"{kind}\"/><w:pgSz w:w=\"{w}\" w:h=\"{h}\"{}/><w:pgMar w:top=\"{t}\" w:right=\"{r}\" w:bottom=\"{b}\" w:left=\"{l}\" w:header=\"{hd}\" w:footer=\"{fd}\" w:gutter=\"0\"/><w:cols w:num=\"{cols}\" w:space=\"{space}\"/>{}</w:sectPr>",
                if landscape { " w:orient=\"landscape\"" } else { "" },
                if title { "<w:titlePg/>" } else { "" }
            );
            out.push((cp_end, x));
        }
        out
    }

    pub fn docx(mut self) -> Result<Vec<u8>> {
        let ccp = self.d.story_len(0);
        let sections = self.sections();
        let mut body = String::new();
        let mut start = 0;
        for (k, (end, sect)) in sections.iter().enumerate() {
            let end = (*end).min(ccp);
            let mut chunk = self.body(start, end);
            if k + 1 < sections.len() {
                // A section ends with its last paragraph.
                if let Some(i) = chunk.rfind("<w:pPr>") {
                    chunk.insert_str(i + 7, sect);
                }
            } else {
                chunk.push_str(sect);
            }
            body.push_str(&chunk);
            start = end;
        }
        if sections.is_empty() {
            body = self.body(0, ccp);
            body.push_str("<w:sectPr><w:pgSz w:w=\"12240\" w:h=\"15840\"/><w:pgMar w:top=\"1440\" w:right=\"1800\" w:bottom=\"1440\" w:left=\"1800\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/></w:sectPr>");
        }
        let ns = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"";
        const H: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
        let mut z = Zip::new();
        let mut ct = String::from("<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/><Default Extension=\"emf\" ContentType=\"image/x-emf\"/><Default Extension=\"wmf\" ContentType=\"image/x-wmf\"/><Default Extension=\"tiff\" ContentType=\"image/tiff\"/><Default Extension=\"bmp\" ContentType=\"image/bmp\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>");
        let mut rels = String::from("<Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>");
        // Numbering.
        if !self.lists.is_empty() && !self.lfo.is_empty() {
            let mut x = format!("{H}<w:numbering {ns}>");
            for (i, (_, levels)) in self.lists.iter().enumerate() {
                let _ = write!(x, "<w:abstractNum w:abstractNumId=\"{i}\">");
                for (k, l) in levels.iter().enumerate() {
                    let fmt = match l.nfc {
                        0 => "decimal",
                        1 => "upperRoman",
                        2 => "lowerRoman",
                        3 => "upperLetter",
                        4 => "lowerLetter",
                        5 => "ordinal",
                        22 => "decimalZero",
                        23 => "bullet",
                        24 => "ganada",
                        25 => "chosung",
                        255 => "none",
                        _ => "decimal",
                    };
                    let jc = ["left", "center", "right"].get(l.jc as usize).copied().unwrap_or("left");
                    let suff = ["tab", "space", "nothing"].get(l.follow as usize).copied().unwrap_or("tab");
                    let _ = write!(x, "<w:lvl w:ilvl=\"{k}\"><w:start w:val=\"{}\"/><w:numFmt w:val=\"{fmt}\"/><w:suff w:val=\"{suff}\"/><w:lvlText w:val=\"{}\"/><w:lvlJc w:val=\"{jc}\"/><w:pPr>{}</w:pPr><w:rPr>{}</w:rPr></w:lvl>", l.start, attr(&l.text), l.ppr, l.rpr);
                }
                x.push_str("</w:abstractNum>");
            }
            for (k, lsid) in self.lfo.iter().enumerate() {
                if let Some(ai) = self.lists.iter().position(|(id, _)| id == lsid) {
                    let _ = write!(x, "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{ai}\"/></w:num>", k + 1);
                }
            }
            x.push_str("</w:numbering>");
            z.write("word/numbering.xml", x.into_bytes());
            ct.push_str("<Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>");
            rels.push_str("<Relationship Id=\"rIdNumbering\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/>");
        }
        // Footnotes.
        if !self.footnotes.is_empty() {
            let mut x = format!("{H}<w:footnotes {ns}><w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>");
            for (i, (a, b)) in self.footnotes.clone().into_iter().enumerate() {
                let content = if b > a { self.body(a, b) } else { "<w:p/>".into() };
                let _ = write!(x, "<w:footnote w:id=\"{}\">{content}</w:footnote>", i + 1);
            }
            x.push_str("</w:footnotes>");
            z.write("word/footnotes.xml", x.into_bytes());
            ct.push_str("<Override PartName=\"/word/footnotes.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml\"/>");
            rels.push_str("<Relationship Id=\"rIdFootnotes\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes\" Target=\"footnotes.xml\"/>");
        }
        for (i, (name, content)) in self.headers.iter().enumerate() {
            let tag = if name.starts_with("header") { "hdr" } else { "ftr" };
            let kind = if tag == "hdr" { "header" } else { "footer" };
            z.write(&format!("word/{name}.xml"), format!("{H}<w:{tag} {ns}>{}</w:{tag}>", if content.is_empty() { "<w:p/>" } else { content }).into_bytes());
            let _ = write!(ct, "<Override PartName=\"/word/{name}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml\"/>");
            let _ = write!(rels, "<Relationship Id=\"rIdH{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{name}.xml\"/>", i + 1);
        }
        for (i, (name, bytes)) in self.media.iter().enumerate() {
            z.write(&format!("word/media/{name}"), bytes.clone());
            let _ = write!(rels, "<Relationship Id=\"rIdImg{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/{name}\"/>", i + 1);
        }
        z.write("word/document.xml", format!("{H}<w:document {ns}><w:body>{body}</w:body></w:document>").into_bytes());
        z.write("word/styles.xml", format!("{H}<w:styles {ns}><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"{}\" w:hAnsi=\"{}\" w:eastAsia=\"{}\"/><w:sz w:val=\"20\"/></w:rPr></w:rPrDefault></w:docDefaults>{}</w:styles>", attr(self.default_fonts[0].as_deref().unwrap_or("Times New Roman")), attr(self.default_fonts[2].as_deref().or(self.default_fonts[0].as_deref()).unwrap_or("Times New Roman")), attr(self.default_fonts[1].as_deref().unwrap_or("Times New Roman")), self.styles_xml).into_bytes());
        z.write("word/_rels/document.xml.rels", format!("{H}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rels}</Relationships>").into_bytes());
        z.write("_rels/.rels", format!("{H}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>").into_bytes());
        z.write("[Content_Types].xml", format!("{H}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">{ct}</Types>").into_bytes());
        z.save()
    }
}

/// Text as runs' content: tabs, breaks and text elements.
fn run_content(t: &str) -> String {
    let mut out = String::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut String| {
        if !cur.is_empty() {
            let _ = write!(out, "<w:t xml:space=\"preserve\">{}</w:t>", escape(cur));
            cur.clear();
        }
    };
    for c in t.chars() {
        match c {
            '\t' => {
                flush(&mut cur, &mut out);
                out.push_str("<w:tab/>");
            }
            '\u{b}' => {
                flush(&mut cur, &mut out);
                out.push_str("<w:br/>");
            }
            '\u{c}' => {
                flush(&mut cur, &mut out);
                out.push_str("<w:br w:type=\"page\"/>");
            }
            '\u{1e}' => cur.push('\u{2011}'),
            c if (c as u32) < 0x20 => {}
            c => cur.push(c),
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// A table from rows of cells (each cell's paragraphs) and each row's table properties.
fn table_xml(rows: &[(Vec<String>, Vec<(u16, Vec<u8>)>)]) -> String {
    let mut out = String::from("<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblLayout w:type=\"fixed\"/></w:tblPr>");
    // The grid from the first row with cell boundaries.
    let mut grid: Vec<i32> = Vec::new();
    for (_, tap) in rows {
        if let Some((_, v)) = tap.iter().find(|(op, _)| *op == 0xD608) {
            let n = v.first().copied().unwrap_or(0) as usize;
            let xs: Vec<i32> = (0..=n).map(|k| i16_at(v, 1 + k * 2) as i32).collect();
            grid = xs.windows(2).map(|w| (w[1] - w[0]).max(1)).collect();
            break;
        }
    }
    out.push_str("<w:tblGrid>");
    for g in &grid {
        let _ = write!(out, "<w:gridCol w:w=\"{g}\"/>");
    }
    out.push_str("</w:tblGrid>");
    for (cells, tap) in rows {
        out.push_str("<w:tr>");
        let mut tcs: Vec<(i32, u16, [Option<(f64, String)>; 4])> = Vec::new();
        let mut table_borders: Vec<Option<(f64, String)>> = vec![None; 6];
        let mut height = None;
        for (op, v) in tap {
            match *op {
                0xD608 => {
                    let n = v.first().copied().unwrap_or(0) as usize;
                    let xs: Vec<i32> = (0..=n).map(|k| i16_at(v, 1 + k * 2) as i32).collect();
                    for k in 0..n {
                        let tc = v.get(1 + (n + 1) * 2 + k * 20..1 + (n + 1) * 2 + k * 20 + 20).unwrap_or(&[]);
                        let flags = u16_at(tc, 0);
                        let br = |o: usize| tc.get(4 + o * 4..8 + o * 4).and_then(brc80);
                        tcs.push((xs.get(k + 1).copied().unwrap_or(0) - xs.get(k).copied().unwrap_or(0), flags, [br(0), br(1), br(2), br(3)]));
                    }
                }
                0xD605 => {
                    for k in 0..6 {
                        table_borders[k] = v.get(k * 4..k * 4 + 4).and_then(brc80);
                    }
                }
                0x9407 => height = Some(i16_at(v, 0)),
                _ => {}
            }
        }
        if let Some(h) = height {
            if h != 0 {
                let _ = write!(out, "<w:trPr><w:trHeight w:val=\"{}\" w:hRule=\"{}\"/></w:trPr>", h.abs(), if h < 0 { "exact" } else { "atLeast" });
            }
        }
        for (k, c) in cells.iter().enumerate() {
            let (w, flags, brc) = tcs.get(k).cloned().unwrap_or((0, 0, [None, None, None, None]));
            let mut tcpr = String::new();
            if w > 0 {
                let _ = write!(tcpr, "<w:tcW w:w=\"{w}\" w:type=\"dxa\"/>");
            }
            if flags & 0x40 != 0 {
                tcpr.push_str("<w:vMerge w:val=\"restart\"/>");
            } else if flags & 0x20 != 0 {
                tcpr.push_str("<w:vMerge/>");
            }
            let names = ["top", "left", "bottom", "right"];
            let bx: String = (0..4).map(|i| border_xml(names[i], brc[i].clone().or_else(|| table_borders[i].clone()))).collect();
            if !bx.is_empty() {
                let _ = write!(tcpr, "<w:tcBorders>{bx}</w:tcBorders>");
            }
            let va = match (flags >> 7) & 3 {
                1 => Some("center"),
                2 => Some("bottom"),
                _ => None,
            };
            if let Some(v) = va {
                let _ = write!(tcpr, "<w:vAlign w:val=\"{v}\"/>");
            }
            let content = if c.is_empty() { "<w:p/>".to_string() } else { c.clone() };
            let _ = write!(out, "<w:tc><w:tcPr>{tcpr}</w:tcPr>{content}</w:tc>");
        }
        out.push_str("</w:tr>");
    }
    out.push_str("</w:tbl>");
    out
}

/// The first picture (BLIP) in OfficeArt records: (extension, bytes).
pub fn find_blip(d: &[u8]) -> Option<(&'static str, Vec<u8>)> {
    let mut i = 0;
    while i + 8 <= d.len() {
        let ver = u16_at(d, i);
        let typ = u16_at(d, i + 2);
        let len = u32_at(d, i + 4) as usize;
        let body = d.get(i + 8..(i + 8 + len).min(d.len()))?;
        if ver & 0xF == 0xF {
            if let Some(x) = find_blip(body) {
                return Some(x);
            }
        } else if typ == 0xF007 {
            // A BSE (as inline pictures carry after their shape): its blip follows its 36 bytes.
            if let Some(x) = body.get(36..).and_then(find_blip) {
                return Some(x);
            }
        } else if (0xF018..=0xF117).contains(&typ) {
            let inst = ver >> 4;
            // UIDs: one, or two for the instances that have a second.
            let two = matches!(inst, 0x3D5 | 0x217 | 0x46B | 0x6E1 | 0x7A9 | 0x543 | 0x6E5 | 0x6E3);
            let uid = if two { 32 } else { 16 };
            return match typ {
                0xF01A | 0xF01B => {
                    // Metafile: a 34-byte header, then the (deflated, usually) data.
                    let mh = body.get(uid..uid + 34)?;
                    let compressed = mh[32] == 0;
                    let data = body.get(uid + 34..)?;
                    let bytes = if compressed { miniz_oxide::inflate::decompress_to_vec_zlib(data).ok()? } else { data.to_vec() };
                    Some((if typ == 0xF01A { "emf" } else { "wmf" }, bytes))
                }
                0xF01D | 0xF02A => Some(("jpeg", body.get(uid + 1..)?.to_vec())),
                0xF01E => Some(("png", body.get(uid + 1..)?.to_vec())),
                0xF01F => {
                    // A DIB: give it a BMP file header.
                    let dib = body.get(uid + 1..)?;
                    let mut bmp = b"BM".to_vec();
                    bmp.extend_from_slice(&((dib.len() + 14) as u32).to_le_bytes());
                    bmp.extend_from_slice(&[0, 0, 0, 0]);
                    let hdr = u32_at(dib, 0);
                    let bits = u16_at(dib, 14) as u32;
                    let colors = u32_at(dib, 32);
                    let pal = if bits <= 8 { (if colors == 0 { 1 << bits } else { colors }) * 4 } else { 0 };
                    bmp.extend_from_slice(&(14 + hdr + pal).to_le_bytes());
                    bmp.extend_from_slice(dib);
                    Some(("bmp", bmp))
                }
                0xF029 => Some(("tiff", body.get(uid + 1..)?.to_vec())),
                _ => None,
            };
        }
        i += 8 + len;
    }
    None
}
