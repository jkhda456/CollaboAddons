//! Writing Typst code: string literals, lengths, colors, font lists. All document text goes in as
//! string literals (never as markup), so nothing in it can be read as Typst syntax.

use std::fmt::Write;

/// A Typst string literal.
pub fn s(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A length in points, short.
pub fn pt(v: f64) -> String {
    if !v.is_finite() {
        return "0pt".into();
    }
    let r = (v * 1000.0).round() / 1000.0;
    let mut t = format!("{r}");
    if t == "-0" {
        t = "0".into();
    }
    format!("{t}pt")
}

pub fn num(v: f64) -> String {
    let r = (v * 10000.0).round() / 10000.0;
    format!("{r}")
}

/// `rgb("#RRGGBB")`, or with alpha.
pub fn rgb(hex: &str) -> String {
    format!("rgb(\"#{}\")", hex)
}

pub fn rgba(hex: &str, alpha: f64) -> String {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("rgb(\"#{}{:02x}\")", hex, a)
}

/// A Typst array of font families.
pub fn fonts(list: &[String]) -> String {
    let mut out = String::from("(");
    for (i, f) in list.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&s(f));
    }
    if list.len() == 1 {
        out.push(',');
    }
    out.push(')');
    out
}

/// Helpers every generated document starts with.
pub const PRELUDE: &str = r#"
#let docu-tab(stops, default, lbl, off, leader-of, body) = context {
  // A tab: to the next stop after the current position (stops are measured from the paragraph's
  // origin: where its label is, less the first line's offset).
  let origin = locate(lbl).position().x - off
  let x = here().position().x - origin
  let next = none
  for st in stops {
    if st.pos > x + 0.5pt { next = st; break }
  }
  if next == none {
    let k = calc.floor(x / default) + 1
    next = (pos: k * default, kind: "left", leader: none)
  }
  let w = measure(body).width
  let gap = if next.kind == "right" { next.pos - x - w } else if next.kind == "center" { next.pos - x - w / 2 } else { next.pos - x }
  let gap = calc.max(gap, 0pt)
  if next.leader == none { h(gap) } else { box(width: gap, repeat(leader-of(next.leader))) }
  body
}
"#;

#[cfg(test)]
mod tests {
    #[test]
    fn strings() {
        assert_eq!(super::s("a\"b\\c\n#[x]"), "\"a\\\"b\\\\c\\n#[x]\"");
        assert_eq!(super::pt(12.0), "12pt");
        assert_eq!(super::pt(0.1234), "0.123pt");
    }
}
