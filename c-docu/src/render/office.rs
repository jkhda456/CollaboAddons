//! What Word, Excel and PowerPoint documents share for rendering: the theme (its colors and
//! fonts), DrawingML color transforms, and the fonts used in place of the ones a document names.

use crate::opc::Package;
use crate::render::world::Fonts;
use crate::xml::{Element, A, NONE};
use std::collections::HashMap;

#[derive(Clone, Default, Debug)]
pub struct Theme {
    pub colors: HashMap<String, String>,
    pub major_latin: String,
    pub minor_latin: String,
    pub major_ea: String,
    pub minor_ea: String,
    pub major_cs: String,
    pub minor_cs: String,
    /// Script fonts (`Hang`, `Jpan`, `Hans`, …) of the major and minor fonts.
    pub major_script: HashMap<String, String>,
    pub minor_script: HashMap<String, String>,
}

impl Theme {
    pub fn load(pkg: &Package, part: Option<String>) -> Theme {
        let mut t = Theme { minor_latin: "Calibri".into(), major_latin: "Calibri Light".into(), ..Default::default() };
        for (k, v) in [("dk1", "000000"), ("lt1", "FFFFFF"), ("dk2", "44546A"), ("lt2", "E7E6E6"), ("accent1", "4472C4"), ("accent2", "ED7D31"), ("accent3", "A5A5A5"), ("accent4", "FFC000"), ("accent5", "5B9BD5"), ("accent6", "70AD47"), ("hlink", "0563C1"), ("folHlink", "954F72")] {
            t.colors.insert(k.into(), v.into());
        }
        let Some(part) = part else { return t };
        let Ok(doc) = pkg.xml(&part) else { return t };
        let Some(el) = doc.root.child(A, "themeElements") else { return t };
        if let Some(cs) = el.child(A, "clrScheme") {
            for c in cs.elements() {
                let hex = c.elements().find_map(|x| match x.local() {
                    "srgbClr" => x.attr_ns(NONE, "val").map(str::to_string),
                    "sysClr" => x.attr_ns(NONE, "lastClr").map(str::to_string).or_else(|| Some(if x.attr_ns(NONE, "val") == Some("window") { "FFFFFF".into() } else { "000000".into() })),
                    _ => None,
                });
                if let Some(h) = hex {
                    t.colors.insert(c.local().to_string(), h.to_ascii_uppercase());
                }
            }
        }
        if let Some(fs) = el.child(A, "fontScheme") {
            for (which, latin, ea, cs, scripts) in [("majorFont", &mut t.major_latin, &mut t.major_ea, &mut t.major_cs, &mut t.major_script), ("minorFont", &mut t.minor_latin, &mut t.minor_ea, &mut t.minor_cs, &mut t.minor_script)] {
                if let Some(f) = fs.child(A, which) {
                    if let Some(v) = f.child(A, "latin").and_then(|e| e.attr_ns(NONE, "typeface")) {
                        *latin = v.to_string();
                    }
                    if let Some(v) = f.child(A, "ea").and_then(|e| e.attr_ns(NONE, "typeface")) {
                        *ea = v.to_string();
                    }
                    if let Some(v) = f.child(A, "cs").and_then(|e| e.attr_ns(NONE, "typeface")) {
                        *cs = v.to_string();
                    }
                    for s in f.children_named(A, "font") {
                        if let (Some(sc), Some(tf)) = (s.attr_ns(NONE, "script"), s.attr_ns(NONE, "typeface")) {
                            scripts.insert(sc.to_string(), tf.to_string());
                        }
                    }
                }
            }
        }
        t
    }

    /// A theme font reference (`+mn-lt`, `minorHAnsi`, `majorEastAsia`) as a family name; `lang`
    /// picks the script font for East Asian slots.
    pub fn font(&self, r: &str, lang: Option<&str>) -> Option<String> {
        let (major, slot) = match r {
            "+mj-lt" | "majorHAnsi" | "majorAscii" => (true, "lt"),
            "+mn-lt" | "minorHAnsi" | "minorAscii" => (false, "lt"),
            "+mj-ea" | "majorEastAsia" => (true, "ea"),
            "+mn-ea" | "minorEastAsia" => (false, "ea"),
            "+mj-cs" | "majorBidi" => (true, "cs"),
            "+mn-cs" | "minorBidi" => (false, "cs"),
            _ => return None,
        };
        let (latin, ea, cs, scripts) = if major { (&self.major_latin, &self.major_ea, &self.major_cs, &self.major_script) } else { (&self.minor_latin, &self.minor_ea, &self.minor_cs, &self.minor_script) };
        let v = match slot {
            "lt" => latin.clone(),
            "ea" => {
                let script = match lang.map(|l| l.to_ascii_lowercase()) {
                    Some(l) if l.starts_with("ko") => Some("Hang"),
                    Some(l) if l.starts_with("ja") => Some("Jpan"),
                    Some(l) if l == "zh-tw" || l == "zh-hk" => Some("Hant"),
                    Some(l) if l.starts_with("zh") => Some("Hans"),
                    _ => None,
                };
                match script.and_then(|s| scripts.get(s)) {
                    Some(f) if ea.is_empty() => f.clone(),
                    _ if !ea.is_empty() => ea.clone(),
                    Some(f) => f.clone(),
                    None => String::new(),
                }
            }
            _ => cs.clone(),
        };
        if v.is_empty() {
            None
        } else {
            Some(v)
        }
    }

    pub fn color(&self, name: &str) -> Option<String> {
        let key = match name {
            "tx1" | "text1" | "dark1" => "dk1",
            "tx2" | "text2" | "dark2" => "dk2",
            "bg1" | "background1" | "light1" => "lt1",
            "bg2" | "background2" | "light2" => "lt2",
            "hyperlink" => "hlink",
            "followedHyperlink" => "folHlink",
            n => n,
        };
        self.colors.get(key).cloned()
    }
}

// ── colors ──────────────────────────────────────────────────────────────────────────────────

pub fn hex_rgb(h: &str) -> (f64, f64, f64) {
    let v = u32::from_str_radix(h.get(..6).unwrap_or("000000"), 16).unwrap_or(0);
    (((v >> 16) & 255) as f64 / 255.0, ((v >> 8) & 255) as f64 / 255.0, (v & 255) as f64 / 255.0)
}

pub fn rgb_hex(r: f64, g: f64, b: f64) -> String {
    let c = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("{:02X}{:02X}{:02X}", c(r), c(g), c(b))
}

fn to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

/// A color with DrawingML transforms: (hex, alpha 0..1).
pub fn transform(hex: &str, mods: &[(String, f64)]) -> (String, f64) {
    let (mut r, mut g, mut b) = hex_rgb(hex);
    let mut alpha = 1.0;
    for (m, v) in mods {
        let v = *v / 100000.0;
        match m.as_str() {
            "lumMod" | "lumOff" | "satMod" | "satOff" | "hueMod" | "hueOff" => {
                let (mut h, mut s, mut l) = to_hsl(r, g, b);
                match m.as_str() {
                    "lumMod" => l *= v,
                    "lumOff" => l += v,
                    "satMod" => s *= v,
                    "satOff" => s += v,
                    "hueMod" => h = (h * v).fract(),
                    _ => h = (h + v / 360.0 * 100000.0 / 60000.0).fract(),
                }
                let c = from_hsl(h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
                r = c.0;
                g = c.1;
                b = c.2;
            }
            "tint" => {
                // Toward white, in linear light as Office does.
                let f = |c: f64| {
                    let lin = srgb_to_linear(c);
                    linear_to_srgb(lin * v + (1.0 - v))
                };
                r = f(r);
                g = f(g);
                b = f(b);
            }
            "shade" => {
                let f = |c: f64| linear_to_srgb(srgb_to_linear(c) * v);
                r = f(r);
                g = f(g);
                b = f(b);
            }
            "alpha" => alpha = v,
            "alphaMod" => alpha *= v,
            "alphaOff" => alpha += v,
            "inv" => {
                r = 1.0 - r;
                g = 1.0 - g;
                b = 1.0 - b;
            }
            "gray" => {
                let y = 0.3 * r + 0.59 * g + 0.11 * b;
                r = y;
                g = y;
                b = y;
            }
            "comp" => {
                let (h, s, l) = to_hsl(r, g, b);
                let c = from_hsl((h + 0.5).fract(), s, l);
                r = c.0;
                g = c.1;
                b = c.2;
            }
            _ => {}
        }
    }
    (rgb_hex(r, g, b), alpha.clamp(0.0, 1.0))
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f64) -> f64 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Word's themeTint/themeShade (hex bytes) on a theme color.
pub fn wml_tint_shade(hex: &str, tint: Option<&str>, shade: Option<&str>) -> String {
    let (r, g, b) = hex_rgb(hex);
    let (h, s, mut l) = to_hsl(r, g, b);
    if let Some(t) = tint.and_then(|v| u8::from_str_radix(v, 16).ok()) {
        l = l * (t as f64 / 255.0) + (1.0 - t as f64 / 255.0);
    }
    if let Some(sh) = shade.and_then(|v| u8::from_str_radix(v, 16).ok()) {
        l *= sh as f64 / 255.0;
    }
    let (r, g, b) = from_hsl(h, s, l.clamp(0.0, 1.0));
    rgb_hex(r, g, b)
}

/// A DrawingML color element's parent (`a:solidFill`, `a:fgClr`, …) as (hex, alpha), with
/// placeholder color `ph` for `phClr` and the slide's color map.
pub fn dml_color(parent: &Element, theme: &Theme, clr_map: &HashMap<String, String>, ph: Option<&(String, f64)>) -> Option<(String, f64)> {
    let c = parent.elements().find(|e| matches!(e.local(), "srgbClr" | "schemeClr" | "sysClr" | "prstClr" | "scrgbClr" | "hslClr"))?;
    color_el(c, theme, clr_map, ph)
}

pub fn color_el(c: &Element, theme: &Theme, clr_map: &HashMap<String, String>, ph: Option<&(String, f64)>) -> Option<(String, f64)> {
    let base: String = match c.local() {
        "srgbClr" => c.attr_ns(NONE, "val")?.to_ascii_uppercase(),
        "sysClr" => c.attr_ns(NONE, "lastClr").map(str::to_string).unwrap_or_else(|| if c.attr_ns(NONE, "val") == Some("window") { "FFFFFF".into() } else { "000000".into() }),
        "schemeClr" => {
            let v = c.attr_ns(NONE, "val")?;
            if v == "phClr" {
                ph.map(|p| p.0.clone()).unwrap_or_else(|| "000000".into())
            } else {
                let mapped = clr_map.get(v).map(|s| s.as_str()).unwrap_or(v);
                theme.color(mapped).or_else(|| theme.color(v)).unwrap_or_else(|| "000000".into())
            }
        }
        "prstClr" => crate::props::named_color(&c.attr_ns(NONE, "val")?.to_ascii_lowercase()).unwrap_or("000000").to_string(),
        "scrgbClr" => {
            let g = |a: &str| c.attr_ns(NONE, a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100000.0;
            rgb_hex(linear_to_srgb(g("r")), linear_to_srgb(g("g")), linear_to_srgb(g("b")))
        }
        "hslClr" => {
            let g = |a: &str| c.attr_ns(NONE, a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            let (r, gg, b) = from_hsl(g("hue") / 21600000.0, g("sat") / 100000.0, g("lum") / 100000.0);
            rgb_hex(r, gg, b)
        }
        _ => return None,
    };
    let mods: Vec<(String, f64)> = c.elements().filter_map(|m| m.attr_ns(NONE, "val").and_then(|v| v.parse::<f64>().ok()).map(|v| (m.local().to_string(), v))).collect();
    let (hex, mut alpha) = transform(&base, &mods);
    if c.attr_ns(NONE, "val") == Some("phClr") {
        if let Some(p) = ph {
            alpha *= p.1;
        }
    }
    Some((hex, alpha))
}

// ── fonts ───────────────────────────────────────────────────────────────────────────────────

/// The families to try for a font a document names, best first: the font itself when it is
/// installed, then its metric-compatible stand-in, then fallbacks for other scripts.
pub fn font_stack(fonts: &Fonts, latin: &str, ea: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let push = |f: &str, out: &mut Vec<String>| {
        if !f.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(f)) && fonts.has_family(f) {
            out.push(f.to_string());
        }
    };
    push(latin, &mut out);
    for s in substitutes(latin) {
        push(s, &mut out);
    }
    if let Some(e) = ea {
        push(e, &mut out);
        for s in substitutes(e) {
            push(s, &mut out);
        }
    }
    let serif = is_serif(latin) || ea.map(is_serif).unwrap_or(false);
    // Chinese characters and kana, which Nanum lacks, come from Droid Sans Fallback.
    let fallbacks: &[&str] = if serif { &["Liberation Serif", "NanumMyeongjo", "DejaVu Serif", "NanumGothic", "Droid Sans Fallback", "DejaVu Sans"] } else { &["Liberation Sans", "NanumGothic", "Droid Sans Fallback", "DejaVu Sans"] };
    for f in fallbacks {
        push(f, &mut out);
    }
    if out.is_empty() {
        out.push("DejaVu Sans".into());
    }
    out
}

pub fn is_serif(name: &str) -> bool {
    let n = name.to_lowercase();
    ["times", "serif", "cambria", "georgia", "garamond", "palatino", "book antiqua", "batang", "바탕", "명조", "myeongjo", "gungsuh", "궁서", "mincho", "song", "simsun", "minion", "caladea", "tinos", "century schoolbook", "baskerville"]
        .iter()
        .any(|k| n.contains(k))
        && !n.contains("sans")
}

fn substitutes(name: &str) -> &'static [&'static str] {
    let n = name.to_lowercase();
    let n = n.trim();
    match n {
        "calibri" | "calibri light" | "carlito" => &["Carlito"],
        "cambria" | "cambria math" | "caladea" => &["Caladea", "Liberation Serif"],
        "arial" | "helvetica" | "helvetica neue" | "arial unicode ms" | "arimo" | "liberation sans" => &["Liberation Sans"],
        "arial narrow" => &["Liberation Sans Narrow", "Liberation Sans"],
        "times new roman" | "times" | "tinos" | "liberation serif" => &["Liberation Serif"],
        "courier new" | "courier" | "cousine" | "liberation mono" => &["Liberation Mono"],
        "consolas" | "lucida console" | "menlo" | "monaco" | "source code pro" | "d2coding" | "나눔고딕코딩" | "nanumgothiccoding" => &["DejaVu Sans Mono", "Liberation Mono"],
        "segoe ui" | "segoe ui light" | "segoe ui semibold" | "tahoma" | "verdana" | "trebuchet ms" | "century gothic" | "gill sans" | "gill sans mt" | "franklin gothic" | "aptos" | "aptos display" | "aptos narrow" | "roboto" | "open sans" | "lato" | "noto sans" => &["Liberation Sans", "DejaVu Sans"],
        "georgia" | "garamond" | "book antiqua" | "palatino linotype" | "palatino" | "century schoolbook" | "constantia" | "baskerville old face" => &["Liberation Serif"],
        "symbol" | "wingdings" | "wingdings 2" | "wingdings 3" | "webdings" => &["DejaVu Sans"],
        _ => {
            if n.contains("고딕") || n.contains("gothic") || n.contains("굴림") || n.contains("gulim") || n.contains("돋움") || n.contains("dotum") || n.contains("malgun") || n.contains("맑은") || n.contains("sans kr") || n.contains("sans cjk") || n.contains("apple sd") || n.contains("pretendard") || n.contains("spoqa") || n.contains("hy견고딕") || n.contains("hy헤드라인") || n.contains("헤드라인") || n.contains("나눔스퀘어") {
                &["NanumGothic"]
            } else if n.contains("명조") || n.contains("바탕") || n.contains("궁서") || n.contains("batang") || n.contains("gungsuh") || n.contains("myeongjo") || n.contains("serif kr") || n.contains("한컴") || n.contains("hy신명조") || n.contains("함초롬") {
                &["NanumMyeongjo"]
            } else if n.contains("mono") || n.contains("code") || n.contains("courier") {
                &["DejaVu Sans Mono", "Liberation Mono"]
            } else if is_serif(n) {
                &["Liberation Serif"]
            } else {
                &[]
            }
        }
    }
}

/// (ascent, descent) in em of the first family of a stack that has a font: Windows metrics (what
/// Word and PowerPoint use for line heights).
pub fn line_metrics(fonts: &Fonts, stack: &[String]) -> (f64, f64) {
    for f in stack {
        if let Some(font) = fonts.first_of(f) {
            let Ok(face) = ttf_parser::Face::parse(font.data(), font.index()) else { continue };
            let upem = face.units_per_em() as f64;
            if let Some(os2) = face.tables().os2 {
                let a = os2.windows_ascender() as f64 / upem;
                let d = os2.windows_descender() as f64 / upem;
                if a > 0.0 {
                    return (a, d.abs());
                }
            }
            let a = face.ascender() as f64 / upem;
            let d = -(face.descender() as f64) / upem;
            let g = face.line_gap() as f64 / upem;
            return (a + g / 2.0, d + g / 2.0);
        }
    }
    (0.9, 0.25)
}

/// Characters of symbol fonts (Symbol, Wingdings: private use code points) as Unicode.
pub fn symbol_char(font: &str, c: char) -> char {
    let code = c as u32;
    let code = if (0xF000..=0xF0FF).contains(&code) { code - 0xF000 } else { code };
    let f = font.to_lowercase();
    if f.starts_with("wingdings") {
        return match code {
            0x9F | 0x6C => '●',
            0xA7 | 0x6E => '■',
            0xA8 => '◻',
            0x71 => '❑',
            0x75 => '◆',
            0x76 => '❖',
            0xD8 => '➢',
            0xE0 => '➔',
            0xE8 => '➔',
            0xF0 => '⇨',
            0xFC => '✓',
            0xFB => '✗',
            0xFD => '☒',
            0xFE => '☑',
            0x6F => '□',
            0x77 => '◆',
            0x4A => '☺',
            0x4C => '☹',
            0xA1 => '○',
            0x9E => '·',
            0x22 => '✂',
            0x28 => '☎',
            0x2A => '✉',
            0x3F => '✍',
            0x46 => '☞',
            0x49 => '✋',
            0x8C..=0x95 => char::from_u32(0x2460 + code - 0x8C).unwrap_or('•'),
            _ => '•',
        };
    }
    if f == "symbol" {
        return match code {
            0xB7 => '•',
            0xA8 => '♦',
            0xAB => '↔',
            0xAE => '→',
            0xAC => '←',
            0xAD => '↑',
            0xAF => '↓',
            0xB0 => '°',
            0xB1 => '±',
            0xB4 => '×',
            0xB8 => '÷',
            0xA3 => '≤',
            0xB3 => '≥',
            0xB9 => '≠',
            0xBB => '≈',
            0xA5 => '∞',
            0xD6 => '√',
            0xE5 => '∑',
            0xD5 => '∏',
            0x2D => '−',
            0x61..=0x7A => {
                const G: &str = "αβχδεφγηιϕκλμνοπθρστυϖωξψζ";
                G.chars().nth((code - 0x61) as usize).unwrap_or('?')
            }
            0x41..=0x5A => {
                const G: &str = "ΑΒΧΔΕΦΓΗΙϑΚΛΜΝΟΠΘΡΣΤΥςΩΞΨΖ";
                G.chars().nth((code - 0x41) as usize).unwrap_or('?')
            }
            c if c < 0x80 => char::from_u32(c).unwrap_or('•'),
            _ => '•',
        };
    }
    c
}

pub fn is_symbol_font(f: &str) -> bool {
    let f = f.to_lowercase();
    f == "symbol" || f.starts_with("wingdings") || f == "webdings"
}
