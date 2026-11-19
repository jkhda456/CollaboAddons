//! What `read`, `find` and the edit commands report, the same for every format: items with
//! addresses, text and properties, printed for people (one line each) or as JSON.

use serde_json::{json, Value};

#[derive(Clone, Debug, Default)]
pub struct RunInfo {
    pub text: String,
    pub props: Vec<String>,
    pub start: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Item {
    pub addr: String,
    /// `p`, `table`, `story`, `slide`, `shape`, `sheet`, `cell`, `row`, …
    pub kind: String,
    pub text: String,
    pub depth: usize,
    pub style: Option<String>,
    /// Where the item is (`t1.r2.c3`, `text box`).
    pub place: Option<String>,
    pub extra: Vec<(String, String)>,
    pub runs: Vec<RunInfo>,
}

#[derive(Clone, Debug, Default)]
pub struct Outline {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct Match {
    pub addr: String,
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub context: String,
}

impl Outline {
    pub fn print(&self, out: &mut String) {
        for it in &self.items {
            let pad = "  ".repeat(it.depth);
            let mut line = format!("{pad}{}", it.addr);
            match it.kind.as_str() {
                "p" | "cell" | "shape" | "notes" => {}
                k => line.push_str(&format!(" ({k})")),
            }
            if let Some(s) = &it.style {
                line.push_str(&format!(" [{s}]"));
            }
            if let Some(p) = &it.place {
                line.push_str(&format!(" <{p}>"));
            }
            for (k, v) in &it.extra {
                if k == "formula" {
                    line.push_str(&format!(" {{{v}}}"));
                } else if v.is_empty() {
                    line.push_str(&format!(" {{{k}}}"));
                } else {
                    line.push_str(&format!(" {{{k}={v}}}"));
                }
            }
            if !it.text.is_empty() {
                line.push_str(": ");
                line.push_str(&crate::text::escape_line(&it.text));
            }
            out.push_str(&line);
            out.push('\n');
            for (i, r) in it.runs.iter().enumerate() {
                out.push_str(&format!(
                    "{pad}    r{} @{} {:?}{}{}\n",
                    i + 1,
                    r.start,
                    r.text,
                    if r.props.is_empty() { "" } else { " " },
                    r.props.join(" ")
                ));
            }
        }
    }

    pub fn json(&self) -> Value {
        Value::Array(
            self.items
                .iter()
                .map(|it| {
                    let mut o = serde_json::Map::new();
                    o.insert("addr".into(), json!(it.addr));
                    o.insert("kind".into(), json!(it.kind));
                    o.insert("text".into(), json!(it.text));
                    if it.depth > 0 {
                        o.insert("depth".into(), json!(it.depth));
                    }
                    if let Some(s) = &it.style {
                        o.insert("style".into(), json!(s));
                    }
                    if let Some(p) = &it.place {
                        o.insert("place".into(), json!(p));
                    }
                    for (k, v) in &it.extra {
                        o.insert(k.clone(), json!(v));
                    }
                    if !it.runs.is_empty() {
                        o.insert(
                            "runs".into(),
                            Value::Array(it.runs.iter().map(|r| json!({"start": r.start, "text": r.text, "props": r.props})).collect()),
                        );
                    }
                    Value::Object(o)
                })
                .collect(),
        )
    }

    /// Plain text: one line per paragraph-like item.
    pub fn plain(&self, out: &mut String) {
        for it in &self.items {
            match it.kind.as_str() {
                "p" | "cell" | "notes" | "shape" => {
                    out.push_str(&it.text.replace('\u{FFFC}', ""));
                    out.push('\n');
                }
                "slide" | "sheet" | "story" => {
                    out.push_str(&format!("\n== {} {}\n", it.addr, it.text));
                }
                _ => {}
            }
        }
    }
}
