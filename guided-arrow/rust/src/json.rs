//! Just enough JSON: reading cheat.sheets' topic lists, writing `--json` output.

use crate::{err, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    out.push_str(&format!("{}", *n as i64));
                } else {
                    out.push_str(&format!("{n}"));
                }
            }
            Json::Str(s) => quote(s, out),
            Json::Arr(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Json::Obj(kv) => {
                out.push('{');
                for (i, (k, v)) in kv.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    quote(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }

    pub fn to_string(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }
}

/// An object, built in order.
pub fn obj(kv: Vec<(&str, Json)>) -> Json {
    Json::Obj(kv.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

pub fn s(v: &str) -> Json {
    Json::Str(v.to_string())
}

fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn parse(text: &str) -> Result<Json> {
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(err!("JSON: trailing text at {}", p.i));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn bad<T>(&self) -> Result<T> {
        Err(err!("JSON: unexpected input at {}", self.i))
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json> {
        if depth > 64 {
            return self.bad();
        }
        self.ws();
        match self.s.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut kv = Vec::new();
                self.ws();
                if self.eat("}") {
                    return Ok(Json::Obj(kv));
                }
                loop {
                    self.ws();
                    let k = match self.value(depth + 1)? {
                        Json::Str(k) => k,
                        _ => return self.bad(),
                    };
                    self.ws();
                    if !self.eat(":") {
                        return self.bad();
                    }
                    kv.push((k, self.value(depth + 1)?));
                    self.ws();
                    if self.eat("}") {
                        return Ok(Json::Obj(kv));
                    }
                    if !self.eat(",") {
                        return self.bad();
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.eat("]") {
                    return Ok(Json::Arr(a));
                }
                loop {
                    a.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat("]") {
                        return Ok(Json::Arr(a));
                    }
                    if !self.eat(",") {
                        return self.bad();
                    }
                }
            }
            Some(b'"') => {
                self.i += 1;
                let mut out = String::new();
                loop {
                    let start = self.i;
                    while self.i < self.s.len() && self.s[self.i] != b'"' && self.s[self.i] != b'\\' {
                        self.i += 1;
                    }
                    out.push_str(std::str::from_utf8(&self.s[start..self.i]).map_err(|_| err!("JSON: bad UTF-8"))?);
                    match self.s.get(self.i) {
                        Some(b'"') => {
                            self.i += 1;
                            return Ok(Json::Str(out));
                        }
                        Some(b'\\') => {
                            self.i += 1;
                            let c = *self.s.get(self.i).ok_or_else(|| err!("JSON: truncated"))?;
                            self.i += 1;
                            match c {
                                b'n' => out.push('\n'),
                                b't' => out.push('\t'),
                                b'r' => out.push('\r'),
                                b'b' => out.push('\u{8}'),
                                b'f' => out.push('\u{c}'),
                                b'u' => {
                                    let mut cp = self.hex4()?;
                                    if (0xD800..0xDC00).contains(&cp) && self.eat("\\u") {
                                        let lo = self.hex4()?;
                                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                                    }
                                    out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                                }
                                c => out.push(c as char),
                            }
                        }
                        _ => return self.bad(),
                    }
                }
            }
            Some(b't') if self.eat("true") => Ok(Json::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(Json::Bool(false)),
            Some(b'n') if self.eat("null") => Ok(Json::Null),
            Some(c) if c.is_ascii_digit() || *c == b'-' => {
                let start = self.i;
                while self.i < self.s.len() && matches!(self.s[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).unwrap();
                t.parse().map(Json::Num).map_err(|_| err!("JSON: bad number {t}"))
            }
            _ => self.bad(),
        }
    }

    fn hex4(&mut self) -> Result<u32> {
        let h = self.s.get(self.i..self.i + 4).ok_or_else(|| err!("JSON: truncated"))?;
        self.i += 4;
        u32::from_str_radix(std::str::from_utf8(h).unwrap_or("x"), 16).map_err(|_| err!("JSON: bad escape"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let v = parse(r#" {"a": [1, 2.5, -3e2, true, false, null], "b\n": "x\"yé😀", "c": {}} "#).unwrap();
        assert_eq!(v.get("b\n").and_then(Json::as_str), Some("x\"yé😀"));
        assert_eq!(parse(&v.to_string()).unwrap(), v);
        assert!(parse("[1,]").is_err());
        assert!(parse("{\"a\" 1}").is_err());
        assert!(parse("\"abc").is_err());
    }
}
