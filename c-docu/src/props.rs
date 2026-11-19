//! Style properties as the command line gives them (`bold`, `color=red`, `size=12pt`,
//! `align=center`), parsed once and applied by each format in its own markup.

use crate::{err, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Color {
    Rgb(String), // six upper-case hex digits
    Auto,
    None,
}

impl Color {
    pub fn parse(s: &str) -> Result<Color> {
        let t = s.trim().trim_start_matches('#');
        let lower = t.to_ascii_lowercase();
        match lower.as_str() {
            "auto" => return Ok(Color::Auto),
            "none" | "off" | "no" | "transparent" => return Ok(Color::None),
            _ => {}
        }
        if let Some(hex) = named_color(&lower) {
            return Ok(Color::Rgb(hex.to_string()));
        }
        let hex = match t.len() {
            3 => t.chars().flat_map(|c| [c, c]).collect::<String>(),
            6 => t.to_string(),
            8 => t[2..].to_string(), // ARGB
            _ => return Err(err(format!("bad color {s:?} (use RRGGBB, #RGB or a name such as red)"))),
        };
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(err(format!("bad color {s:?}")));
        }
        Ok(Color::Rgb(hex.to_ascii_uppercase()))
    }
    pub fn hex(&self) -> Option<&str> {
        match self {
            Color::Rgb(h) => Some(h),
            _ => None,
        }
    }
}

pub fn named_color(name: &str) -> Option<&'static str> {
    Some(match name {
        "black" => "000000",
        "white" => "FFFFFF",
        "red" => "FF0000",
        "green" => "008000",
        "lime" => "00FF00",
        "blue" => "0000FF",
        "yellow" => "FFFF00",
        "cyan" | "aqua" => "00FFFF",
        "magenta" | "fuchsia" => "FF00FF",
        "gray" | "grey" => "808080",
        "silver" | "lightgray" | "lightgrey" => "C0C0C0",
        "darkgray" | "darkgrey" => "404040",
        "maroon" | "darkred" => "800000",
        "olive" | "darkyellow" => "808000",
        "navy" | "darkblue" => "000080",
        "purple" | "darkmagenta" => "800080",
        "teal" | "darkcyan" => "008080",
        "darkgreen" => "006400",
        "orange" => "FFA500",
        "pink" => "FFC0CB",
        "brown" => "A52A2A",
        "gold" => "FFD700",
        "indigo" => "4B0082",
        "violet" => "EE82EE",
        _ => return None,
    })
}

/// A length in points: `12pt`, `1cm`, `10mm`, `0.5in`, `720tw` (twips), `12700emu`, `2px` (at
/// 96 dpi), or a bare number (points).
pub fn length(s: &str) -> Result<f64> {
    let t = s.trim().to_ascii_lowercase();
    let (num, unit) = t.split_at(t.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(t.len()));
    let v: f64 = num.trim().parse().map_err(|_| err(format!("bad length {s:?} (use 12pt, 1cm, 10mm, 0.5in)")))?;
    Ok(match unit.trim() {
        "" | "pt" => v,
        "cm" => v * 72.0 / 2.54,
        "mm" => v * 72.0 / 25.4,
        "in" | "inch" => v * 72.0,
        "tw" | "twip" | "twips" | "dxa" => v / 20.0,
        "emu" => v / 12700.0,
        "px" => v * 0.75,
        "pc" => v * 12.0,
        u => return Err(err(format!("unknown unit {u:?} in {s:?}"))),
    })
}

pub fn boolean(s: &str) -> Result<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "1" | "true" | "on" | "yes" | "y" => Ok(true),
        "0" | "false" | "off" | "no" | "n" | "none" => Ok(false),
        _ => Err(err(format!("expected true or false, got {s:?}"))),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LineSpacing {
    /// A multiple of single spacing (1.5).
    Multiple(f64),
    /// Exactly this many points.
    Exact(f64),
    /// At least this many points.
    AtLeast(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Vert {
    Super,
    Sub,
    Baseline,
}

#[derive(Clone, Debug, Default)]
pub struct Border {
    pub style: String, // single, double, dashed, dotted, none, thick
    pub width: f64,    // points
    pub color: Option<Color>,
}

impl Border {
    pub fn parse(s: &str) -> Result<Border> {
        // "single 1pt red", "none", "1pt", "dashed"
        let mut b = Border { style: "single".into(), width: 0.5, color: None };
        for w in s.split(|c: char| c == ' ' || c == ',').filter(|w| !w.is_empty()) {
            let lw = w.to_ascii_lowercase();
            if matches!(lw.as_str(), "none" | "single" | "double" | "dashed" | "dotted" | "thick" | "hair" | "medium" | "thin" | "dashdot") {
                b.style = lw;
            } else if lw.chars().next().map(|c| c.is_ascii_digit() || c == '.').unwrap_or(false) {
                b.width = length(&lw)?;
            } else {
                b.color = Some(Color::parse(w)?);
            }
        }
        Ok(b)
    }
}

/// Which edges a border property names.
#[derive(Clone, Debug, PartialEq)]
pub enum Edges {
    All,
    Outline,
    Top,
    Bottom,
    Left,
    Right,
    Inside,
    InsideH,
    InsideV,
}

#[derive(Clone, Debug, Default)]
pub struct Props {
    // characters
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<String>,
    pub strike: Option<bool>,
    pub dstrike: Option<bool>,
    pub color: Option<Color>,
    pub size: Option<f64>,
    /// Every script's font.
    pub font: Option<String>,
    pub font_latin: Option<String>,
    pub font_ea: Option<String>,
    pub font_cs: Option<String>,
    pub highlight: Option<Color>,
    pub shade: Option<Color>,
    pub caps: Option<bool>,
    pub small_caps: Option<bool>,
    pub vert: Option<Vert>,
    pub char_style: Option<String>,
    pub hidden: Option<bool>,
    pub letter_spacing: Option<f64>,
    pub lang: Option<String>,
    pub link: Option<String>,
    pub clear: bool,
    // paragraphs
    pub align: Option<String>,
    pub para_style: Option<String>,
    pub indent_left: Option<f64>,
    pub indent_right: Option<f64>,
    pub indent_first: Option<f64>,
    pub indent_hanging: Option<f64>,
    pub space_before: Option<f64>,
    pub space_after: Option<f64>,
    pub line: Option<LineSpacing>,
    pub keep_next: Option<bool>,
    pub keep_lines: Option<bool>,
    pub page_break_before: Option<bool>,
    pub level: Option<u32>,
    pub bullet: Option<String>,
    pub list: Option<String>,
    // cells, shapes, tables
    pub fill: Option<Color>,
    pub number_format: Option<String>,
    pub valign: Option<String>,
    pub wrap: Option<bool>,
    pub borders: Vec<(Edges, Border)>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub rotation: Option<f64>,
    pub line_color: Option<Color>,
    pub line_width: Option<f64>,
    pub indent_level: Option<u32>,
    pub merge: Option<bool>,
    pub name: Option<String>,
    pub alt: Option<String>,
    pub autofit: Option<String>,
    pub column_width: Option<f64>,
    pub row_height: Option<f64>,
    pub protect: Option<bool>,
    /// The keys given, in order (for "not supported here" messages).
    pub keys: Vec<String>,
}

pub const HELP: &str = "\
characters: bold italic underline[=single|double|none|wave|dotted|dash] strike dstrike
            color=red|#RRGGBB size=12pt font=NAME (all scripts) font.latin= font.ea= (Korean,
            Chinese, Japanese) font.cs= (complex scripts)
            highlight=yellow bg=#RRGGBB (shading) caps smallcaps super sub baseline
            hidden spacing=1pt (letter spacing) lang=ko-KR cstyle=STYLE link=URL clear
paragraphs: align=left|center|right|justify|distribute style=STYLE indent=LEN indent.right=LEN
            indent.first=LEN indent.hanging=LEN before=LEN after=LEN line=1.5|exact:14pt|least:12pt
            keepnext keeplines pagebreak level=N (list or outline level) bullet=CHAR|none
            list=bullet|number|none
cells/shapes: fill=COLOR numfmt=FORMAT valign=top|middle|bottom wrap border=\"single 1pt red\"
            border.top= border.bottom= border.left= border.right= border.outline= border.inside=
            x= y= w= h= (lengths) rotate=DEG line=COLOR line.width=LEN name=TEXT alt=TEXT
            autofit=none|shrink|resize colwidth=CHARS rowheight=LEN indentlevel=N
Booleans take =false to turn off (bold=false). Lengths: 12pt 1cm 10mm 0.5in 720tw.";

/// The canonical name of a property key (aliases folded).
pub fn canon(k: &str) -> &str {
    match k {
        "b" | "bold" => "bold",
        "i" | "italic" => "italic",
        "u" | "underline" => "underline",
        "strike" | "s" | "strikethrough" => "strike",
        "dstrike" => "dstrike",
        "color" | "fg" | "colour" => "color",
        "size" | "sz" | "fontsize" => "size",
        "font" | "face" => "font",
        "font.latin" | "font.ascii" => "font.latin",
        "font.ea" | "font.eastasia" | "font.asian" | "font.ko" => "font.ea",
        "font.cs" | "font.complex" => "font.cs",
        "highlight" | "hl" | "mark" => "highlight",
        "bg" | "shade" | "shading" | "background" => "bg",
        "caps" | "allcaps" => "caps",
        "smallcaps" => "smallcaps",
        "super" | "sup" | "superscript" => "super",
        "sub" | "subscript" => "sub",
        "baseline" => "baseline",
        "cstyle" | "charstyle" | "rstyle" => "cstyle",
        "hidden" | "vanish" => "hidden",
        "spacing" | "letterspacing" | "tracking" => "spacing",
        "lang" | "language" => "lang",
        "link" | "href" | "hyperlink" => "link",
        "clear" | "plain" | "reset" => "clear",
        "align" | "alignment" | "jc" | "halign" => "align",
        "style" | "pstyle" | "parastyle" => "style",
        "indent" | "indent.left" | "ind" | "marl" => "indent",
        "indent.right" | "marr" => "indent.right",
        "indent.first" | "firstline" => "indent.first",
        "indent.hanging" | "hanging" => "indent.hanging",
        "before" | "space.before" | "spacebefore" => "before",
        "after" | "space.after" | "spaceafter" => "after",
        "line" | "linespacing" | "leading" => "line",
        "keepnext" | "keep-next" | "keep.next" => "keepnext",
        "keeplines" | "keep-lines" | "keep.lines" | "keeptogether" => "keeplines",
        "pagebreak" | "pagebreakbefore" | "page-break-before" => "pagebreak",
        "level" | "lvl" | "outline" => "level",
        "bullet" => "bullet",
        "list" | "numbering" => "list",
        "fill" | "bgcolor" => "fill",
        "numfmt" | "format" | "number" | "numberformat" => "numfmt",
        "valign" | "vertical" | "anchor" => "valign",
        "wrap" | "wraptext" => "wrap",
        "x" | "left" => "x",
        "y" | "top" => "y",
        "w" | "width" => "w",
        "h" | "height" => "h",
        "rotate" | "rotation" | "rot" => "rotate",
        "line.color" | "outline.color" | "stroke" | "linecolor" => "line.color",
        "line.width" | "stroke.width" | "linewidth" => "line.width",
        "indentlevel" | "cellindent" => "indentlevel",
        "merge" => "merge",
        "name" => "name",
        "alt" | "descr" | "description" => "alt",
        "autofit" => "autofit",
        "colwidth" | "columnwidth" => "colwidth",
        "rowheight" => "rowheight",
        "protect" | "locked" => "protect",
        k if k == "border" || k.starts_with("border.") => "border",
        _ => "",
    }
}

pub const CHAR_KEYS: &[&str] = &[
    "bold", "italic", "underline", "strike", "dstrike", "color", "size", "font", "font.latin", "font.ea", "font.cs", "highlight", "bg", "caps",
    "smallcaps", "super", "sub", "baseline", "cstyle", "hidden", "spacing", "lang", "link", "clear",
];
pub const PARA_KEYS: &[&str] = &[
    "align", "style", "indent", "indent.right", "indent.first", "indent.hanging", "before", "after", "line", "keepnext", "keeplines",
    "pagebreak", "level", "bullet", "list",
];

impl Props {
    pub fn parse(args: &[String]) -> Result<Props> {
        let mut p = Props::default();
        for a in args {
            let (k, v) = match a.find('=') {
                Some(i) => (a[..i].trim().to_ascii_lowercase(), a[i + 1..].to_string()),
                None => (a.trim().to_ascii_lowercase(), String::new()),
            };
            let (k, negated) = match k.strip_prefix("no-").or_else(|| k.strip_prefix("no.")) {
                Some(rest) if v.is_empty() => (rest.to_string(), true),
                _ => (k, false),
            };
            let flag = |v: &str| -> Result<bool> { Ok(if negated { false } else { boolean(v)? }) };
            let c = canon(&k);
            if c.is_empty() {
                return Err(err(format!("unknown property {k:?}\n{HELP}")));
            }
            match c {
                "bold" => p.bold = Some(flag(&v)?),
                "italic" => p.italic = Some(flag(&v)?),
                "underline" => {
                    p.underline = Some(if negated {
                        "none".into()
                    } else {
                        match v.to_ascii_lowercase().as_str() {
                            "" | "true" | "on" | "1" | "yes" => "single".into(),
                            "false" | "off" | "0" | "no" => "none".into(),
                            s => s.to_string(),
                        }
                    })
                }
                "strike" => p.strike = Some(flag(&v)?),
                "dstrike" => p.dstrike = Some(flag(&v)?),
                "color" => p.color = Some(Color::parse(&v)?),
                "size" => p.size = Some(length(&v)?),
                "font" => p.font = Some(v.clone()),
                "font.latin" => p.font_latin = Some(v.clone()),
                "font.ea" => p.font_ea = Some(v.clone()),
                "font.cs" => p.font_cs = Some(v.clone()),
                "highlight" => {
                    p.highlight = Some(if negated {
                        Color::None
                    } else if v.is_empty() {
                        Color::Rgb("FFFF00".into())
                    } else {
                        Color::parse(&v)?
                    })
                }
                "bg" => p.shade = Some(if negated { Color::None } else { Color::parse(&v)? }),
                "caps" => p.caps = Some(flag(&v)?),
                "smallcaps" => p.small_caps = Some(flag(&v)?),
                "super" => p.vert = Some(if flag(&v)? { Vert::Super } else { Vert::Baseline }),
                "sub" => p.vert = Some(if flag(&v)? { Vert::Sub } else { Vert::Baseline }),
                "baseline" => p.vert = Some(Vert::Baseline),
                "cstyle" => p.char_style = Some(v.clone()),
                "hidden" => p.hidden = Some(flag(&v)?),
                "spacing" => p.letter_spacing = Some(length(&v)?),
                "lang" => p.lang = Some(v.clone()),
                "link" => p.link = Some(v.clone()),
                "clear" => p.clear = true,
                "align" => {
                    let a = match v.to_ascii_lowercase().as_str() {
                        "l" | "left" | "start" => "left",
                        "c" | "center" | "centre" | "middle" => "center",
                        "r" | "right" | "end" => "right",
                        "j" | "justify" | "both" | "justified" => "justify",
                        "distribute" | "distributed" => "distribute",
                        "general" => "general",
                        "fill" => "fill",
                        _ => return Err(err(format!("align: {v:?} is not left, center, right, justify or distribute"))),
                    };
                    p.align = Some(a.into());
                }
                "style" => p.para_style = Some(v.clone()),
                "indent" => p.indent_left = Some(length(&v)?),
                "indent.right" => p.indent_right = Some(length(&v)?),
                "indent.first" => p.indent_first = Some(length(&v)?),
                "indent.hanging" => p.indent_hanging = Some(length(&v)?),
                "before" => p.space_before = Some(length(&v)?),
                "after" => p.space_after = Some(length(&v)?),
                "line" => {
                    let lv = v.to_ascii_lowercase();
                    let spacing = if let Some(x) = lv.strip_prefix("exact:").or_else(|| lv.strip_prefix("exactly:")) {
                        Some(LineSpacing::Exact(length(x)?))
                    } else if let Some(x) = lv.strip_prefix("least:").or_else(|| lv.strip_prefix("atleast:")) {
                        Some(LineSpacing::AtLeast(length(x)?))
                    } else if lv.parse::<f64>().is_ok() {
                        Some(LineSpacing::Multiple(lv.parse().unwrap()))
                    } else if lv == "single" {
                        Some(LineSpacing::Multiple(1.0))
                    } else if lv == "double" {
                        Some(LineSpacing::Multiple(2.0))
                    } else if let Ok(l) = length(&lv) {
                        Some(LineSpacing::Exact(l))
                    } else {
                        None
                    };
                    match spacing {
                        Some(s) => p.line = Some(s),
                        // `line=COLOR`: a shape's outline.
                        None => {
                            p.line_color = Some(Color::parse(&v).map_err(|_| err(format!("line: {v:?} is neither a spacing (1.5, exact:14pt) nor a color")))?);
                            p.keys.push("line.color".into());
                            continue;
                        }
                    }
                }
                "keepnext" => p.keep_next = Some(flag(&v)?),
                "keeplines" => p.keep_lines = Some(flag(&v)?),
                "pagebreak" => p.page_break_before = Some(flag(&v)?),
                "level" => p.level = Some(v.trim().parse().map_err(|_| err(format!("level: bad number {v:?}")))?),
                "bullet" => p.bullet = Some(if negated { "none".into() } else { v.clone() }),
                "list" => p.list = Some(if negated { "none".into() } else { v.to_ascii_lowercase() }),
                "fill" => p.fill = Some(if negated { Color::None } else { Color::parse(&v)? }),
                "numfmt" => p.number_format = Some(v.clone()),
                "valign" => {
                    let a = match v.to_ascii_lowercase().as_str() {
                        "t" | "top" => "top",
                        "m" | "middle" | "center" | "centre" | "c" | "ctr" => "middle",
                        "b" | "bottom" => "bottom",
                        _ => return Err(err(format!("valign: {v:?} is not top, middle or bottom"))),
                    };
                    p.valign = Some(a.into());
                }
                "wrap" => p.wrap = Some(flag(&v)?),
                "border" => {
                    let edges = match k.strip_prefix("border.").unwrap_or("all") {
                        "all" => Edges::All,
                        "outline" | "box" | "outside" => Edges::Outline,
                        "top" => Edges::Top,
                        "bottom" => Edges::Bottom,
                        "left" => Edges::Left,
                        "right" => Edges::Right,
                        "inside" | "inner" => Edges::Inside,
                        "inside.h" | "insideh" => Edges::InsideH,
                        "inside.v" | "insidev" => Edges::InsideV,
                        e => return Err(err(format!("border.{e}: not an edge (top, bottom, left, right, outline, inside)"))),
                    };
                    let b = if negated {
                        Border { style: "none".into(), width: 0.0, color: None }
                    } else {
                        Border::parse(if v.is_empty() { "single" } else { &v })?
                    };
                    p.borders.push((edges, b));
                }
                "x" => p.x = Some(length(&v)?),
                "y" => p.y = Some(length(&v)?),
                "w" => p.width = Some(length(&v)?),
                "h" => p.height = Some(length(&v)?),
                "rotate" => p.rotation = Some(v.trim().trim_end_matches("deg").parse().map_err(|_| err(format!("rotate: bad angle {v:?}")))?),
                "line.color" => p.line_color = Some(if negated { Color::None } else { Color::parse(&v)? }),
                "line.width" => p.line_width = Some(length(&v)?),
                "indentlevel" => p.indent_level = Some(v.trim().parse().map_err(|_| err(format!("indentlevel: bad number {v:?}")))?),
                "merge" => p.merge = Some(flag(&v)?),
                "name" => p.name = Some(v.clone()),
                "alt" => p.alt = Some(v.clone()),
                "autofit" => p.autofit = Some(v.to_ascii_lowercase()),
                "colwidth" => p.column_width = Some(v.trim().parse().map_err(|_| err(format!("colwidth: bad number {v:?}")))?),
                "rowheight" => p.row_height = Some(length(&v)?),
                "protect" => p.protect = Some(flag(&v)?),
                _ => unreachable!(),
            }
            p.keys.push(c.to_string());
        }
        Ok(p)
    }

    pub fn has_char(&self) -> bool {
        self.bold.is_some()
            || self.italic.is_some()
            || self.underline.is_some()
            || self.strike.is_some()
            || self.dstrike.is_some()
            || self.color.is_some()
            || self.size.is_some()
            || self.font.is_some()
            || self.font_latin.is_some()
            || self.font_ea.is_some()
            || self.font_cs.is_some()
            || self.highlight.is_some()
            || self.shade.is_some()
            || self.caps.is_some()
            || self.small_caps.is_some()
            || self.vert.is_some()
            || self.char_style.is_some()
            || self.hidden.is_some()
            || self.letter_spacing.is_some()
            || self.lang.is_some()
            || self.link.is_some()
            || self.clear
    }

    pub fn has_para(&self) -> bool {
        self.align.is_some()
            || self.para_style.is_some()
            || self.indent_left.is_some()
            || self.indent_right.is_some()
            || self.indent_first.is_some()
            || self.indent_hanging.is_some()
            || self.space_before.is_some()
            || self.space_after.is_some()
            || self.line.is_some()
            || self.keep_next.is_some()
            || self.keep_lines.is_some()
            || self.page_break_before.is_some()
            || self.level.is_some()
            || self.bullet.is_some()
            || self.list.is_some()
    }

    /// Fails naming the keys this target does not take.
    pub fn only(&self, allowed: &[&str], what: &str) -> Result<()> {
        let bad: Vec<&String> = self.keys.iter().filter(|k| !allowed.iter().any(|a| a == k)).collect();
        if bad.is_empty() {
            Ok(())
        } else {
            Err(err(format!(
                "{} not supported for {what} (supported: {})",
                bad.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
                allowed.join(" ")
            )))
        }
    }
}

