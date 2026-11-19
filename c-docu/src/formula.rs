//! Excel formulas: tokens (with their places in the text, so references can be rewritten without
//! touching the rest), a parser, an evaluator with the common functions, and the rewriting of
//! references when rows or columns move.

use crate::numfmt;

#[derive(Clone, Debug, PartialEq)]
pub enum XlErr {
    Null,
    Div0,
    Value,
    Ref,
    Name,
    Num,
    NA,
    Spill,
    Calc,
}

impl XlErr {
    pub fn text(&self) -> &'static str {
        match self {
            XlErr::Null => "#NULL!",
            XlErr::Div0 => "#DIV/0!",
            XlErr::Value => "#VALUE!",
            XlErr::Ref => "#REF!",
            XlErr::Name => "#NAME?",
            XlErr::Num => "#NUM!",
            XlErr::NA => "#N/A",
            XlErr::Spill => "#SPILL!",
            XlErr::Calc => "#CALC!",
        }
    }
    pub fn parse(s: &str) -> Option<XlErr> {
        Some(match s.to_ascii_uppercase().as_str() {
            "#NULL!" => XlErr::Null,
            "#DIV/0!" => XlErr::Div0,
            "#VALUE!" => XlErr::Value,
            "#REF!" => XlErr::Ref,
            "#NAME?" => XlErr::Name,
            "#NUM!" => XlErr::Num,
            "#N/A" => XlErr::NA,
            "#SPILL!" => XlErr::Spill,
            "#CALC!" => XlErr::Calc,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Num(f64),
    Str(String),
    Bool(bool),
    Err(XlErr),
    Empty,
    Arr(Vec<Vec<Val>>),
}

impl Val {
    pub fn first(self) -> Val {
        match self {
            Val::Arr(a) => a.into_iter().next().and_then(|r| r.into_iter().next()).unwrap_or(Val::Empty).first(),
            v => v,
        }
    }
    pub fn as_text(&self) -> String {
        match self {
            Val::Num(n) => numfmt::general(*n),
            Val::Str(s) => s.clone(),
            Val::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
            Val::Err(e) => e.text().into(),
            Val::Empty => String::new(),
            Val::Arr(_) => self.clone().first().as_text(),
        }
    }
}

/// A cell reference: 0-based row and column; whole rows or columns use the limits.
#[derive(Clone, Debug, PartialEq)]
pub struct CellRef {
    pub row: u32,
    pub col: u32,
    pub abs_row: bool,
    pub abs_col: bool,
}

pub const MAX_ROW: u32 = 1_048_575;
pub const MAX_COL: u32 = 16_383;

pub fn col_name(mut c: u32) -> String {
    let mut s = Vec::new();
    loop {
        s.push((b'A' + (c % 26) as u8) as char);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s.iter().rev().collect()
}

pub fn col_index(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 3 || !s.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut n: u32 = 0;
    for c in s.chars() {
        n = n * 26 + (c.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
    }
    if n == 0 || n - 1 > MAX_COL {
        return None;
    }
    Some(n - 1)
}

/// `B3`, `$B$3` → (row, col) 0-based.
pub fn parse_cell(s: &str) -> Option<CellRef> {
    let b = s.as_bytes();
    let mut i = 0;
    let abs_col = b.first() == Some(&b'$');
    if abs_col {
        i += 1;
    }
    let cs = i;
    while i < b.len() && b[i].is_ascii_alphabetic() {
        i += 1;
    }
    let col = col_index(&s[cs..i])?;
    let abs_row = b.get(i) == Some(&b'$');
    if abs_row {
        i += 1;
    }
    let rs = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i != b.len() || rs == i {
        return None;
    }
    let row: u32 = s[rs..i].parse().ok()?;
    if row == 0 || row - 1 > MAX_ROW {
        return None;
    }
    Some(CellRef { row: row - 1, col, abs_row, abs_col })
}

pub fn cell_name(row: u32, col: u32) -> String {
    format!("{}{}", col_name(col), row + 1)
}

fn ref_text(r: &CellRef) -> String {
    format!("{}{}{}{}", if r.abs_col { "$" } else { "" }, col_name(r.col), if r.abs_row { "$" } else { "" }, r.row + 1)
}

/// An area: `A1:B3`, `A:A` (whole columns), `3:5` (whole rows), a single cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Area {
    pub sheet: Option<String>,
    pub a: CellRef,
    pub b: CellRef,
    /// `A:C` or `2:4`: written without the other coordinate.
    pub whole: Option<char>,
}

impl Area {
    pub fn rows(&self) -> (u32, u32) {
        (self.a.row.min(self.b.row), self.a.row.max(self.b.row))
    }
    pub fn cols(&self) -> (u32, u32) {
        (self.a.col.min(self.b.col), self.a.col.max(self.b.col))
    }
    pub fn single(&self) -> bool {
        self.a == self.b && self.whole.is_none()
    }
    pub fn text(&self) -> String {
        let sheet = match &self.sheet {
            Some(s) => format!("{}!", quote_sheet(s)),
            None => String::new(),
        };
        match self.whole {
            Some('c') => format!("{sheet}{}{}:{}{}", if self.a.abs_col { "$" } else { "" }, col_name(self.a.col), if self.b.abs_col { "$" } else { "" }, col_name(self.b.col)),
            Some('r') => format!("{sheet}{}{}:{}{}", if self.a.abs_row { "$" } else { "" }, self.a.row + 1, if self.b.abs_row { "$" } else { "" }, self.b.row + 1),
            _ if self.single() => format!("{sheet}{}", ref_text(&self.a)),
            _ => format!("{sheet}{}:{}", ref_text(&self.a), ref_text(&self.b)),
        }
    }
}

pub fn quote_sheet(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && !s.chars().next().unwrap().is_ascii_digit()
        && parse_cell(s).is_none()
        && col_index(s).is_none();
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "''"))
    }
}

/// Parses `Sheet1!A1:B3`, `'My sheet'!C:C`, `B2`, `3:3`.
pub fn parse_area(s: &str) -> Option<Area> {
    let s = s.trim();
    let (sheet, rest) = split_sheet(s)?;
    let (x, y) = match rest.split_once(':') {
        Some((x, y)) => (x, Some(y)),
        None => (rest, None),
    };
    if let Some(a) = parse_cell(x) {
        let b = match y {
            Some(y) => parse_cell(y)?,
            None => a.clone(),
        };
        return Some(Area { sheet, a, b, whole: None });
    }
    let y = y?;
    // Whole columns.
    let strip = |t: &str| -> (bool, String) { (t.starts_with('$'), t.trim_start_matches('$').to_string()) };
    let (ax, xs) = strip(x);
    let (ay, ys) = strip(y);
    if let (Some(c1), Some(c2)) = (col_index(&xs), col_index(&ys)) {
        return Some(Area {
            sheet,
            a: CellRef { row: 0, col: c1, abs_row: false, abs_col: ax },
            b: CellRef { row: MAX_ROW, col: c2, abs_row: false, abs_col: ay },
            whole: Some('c'),
        });
    }
    if let (Ok(r1), Ok(r2)) = (xs.parse::<u32>(), ys.parse::<u32>()) {
        if r1 == 0 || r2 == 0 {
            return None;
        }
        return Some(Area {
            sheet,
            a: CellRef { row: r1 - 1, col: 0, abs_row: ax, abs_col: false },
            b: CellRef { row: r2 - 1, col: MAX_COL, abs_row: ay, abs_col: false },
            whole: Some('r'),
        });
    }
    None
}

fn split_sheet(s: &str) -> Option<(Option<String>, &str)> {
    if let Some(rest) = s.strip_prefix('\'') {
        // 'It''s'!A1
        let mut name = String::new();
        let mut it = rest.char_indices().peekable();
        while let Some((i, c)) = it.next() {
            if c == '\'' {
                if let Some((_, '\'')) = it.peek() {
                    name.push('\'');
                    it.next();
                    continue;
                }
                let after = &rest[i + 1..];
                return after.strip_prefix('!').map(|r| (Some(name), r));
            }
            name.push(c);
        }
        return None;
    }
    match s.rfind('!') {
        Some(i) => Some((Some(s[..i].to_string()), &s[i + 1..])),
        None => Some((None, s)),
    }
}

// ── tokens ──────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum TokKind {
    Num(f64),
    Str(String),
    Bool(bool),
    Err(XlErr),
    Area(Area),
    /// A reference that could not be one (`#REF!` in a sheet name, 3-D references…).
    BadRef,
    Name(String),
    Func(String),
    Op(String),
    LParen,
    RParen,
    Comma,
    Semi,
    LBrace,
    RBrace,
    Space,
}

#[derive(Clone, Debug)]
pub struct Tok {
    pub kind: TokKind,
    pub start: usize,
    pub end: usize,
}

pub fn tokenize(f: &str) -> Result<Vec<Tok>, String> {
    let s = f;
    let b = s.as_bytes();
    let mut i = 0;
    let mut out: Vec<Tok> = Vec::new();
    while i < b.len() {
        let c = b[i];
        let start = i;
        match c {
            b' ' | b'\n' | b'\r' | b'\t' => {
                while i < b.len() && matches!(b[i], b' ' | b'\n' | b'\r' | b'\t') {
                    i += 1;
                }
                out.push(Tok { kind: TokKind::Space, start, end: i });
                continue;
            }
            b'"' => {
                let mut v = String::new();
                i += 1;
                loop {
                    if i >= b.len() {
                        return Err("unterminated string".into());
                    }
                    if b[i] == b'"' {
                        if b.get(i + 1) == Some(&b'"') {
                            v.push('"');
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    let ch = s[i..].chars().next().unwrap();
                    v.push(ch);
                    i += ch.len_utf8();
                }
                out.push(Tok { kind: TokKind::Str(v), start, end: i });
                continue;
            }
            b'#' => {
                let rest = &s[i..];
                let errs = ["#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A", "#SPILL!", "#CALC!", "#GETTING_DATA"];
                if let Some(e) = errs.iter().find(|e| rest.to_ascii_uppercase().starts_with(*e)) {
                    i += e.len();
                    out.push(Tok { kind: TokKind::Err(XlErr::parse(e).unwrap_or(XlErr::NA)), start, end: i });
                    continue;
                }
                return Err(format!("unexpected # at {i}"));
            }
            b'(' => {
                i += 1;
                out.push(Tok { kind: TokKind::LParen, start, end: i });
                continue;
            }
            b')' => {
                i += 1;
                out.push(Tok { kind: TokKind::RParen, start, end: i });
                continue;
            }
            b',' => {
                i += 1;
                out.push(Tok { kind: TokKind::Comma, start, end: i });
                continue;
            }
            b';' => {
                i += 1;
                out.push(Tok { kind: TokKind::Semi, start, end: i });
                continue;
            }
            b'{' => {
                i += 1;
                out.push(Tok { kind: TokKind::LBrace, start, end: i });
                continue;
            }
            b'}' => {
                i += 1;
                out.push(Tok { kind: TokKind::RBrace, start, end: i });
                continue;
            }
            b'+' | b'-' | b'*' | b'/' | b'^' | b'&' | b'%' | b'=' | b':' => {
                i += 1;
                out.push(Tok { kind: TokKind::Op((c as char).to_string()), start, end: i });
                continue;
            }
            b'<' | b'>' => {
                i += 1;
                let mut op = (c as char).to_string();
                if i < b.len() && (b[i] == b'=' || (c == b'<' && b[i] == b'>')) {
                    op.push(b[i] as char);
                    i += 1;
                }
                out.push(Tok { kind: TokKind::Op(op), start, end: i });
                continue;
            }
            _ => {}
        }
        // Numbers (a leading digit or point), unless it is a row range (1:3) or ref.
        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).map(|d| d.is_ascii_digit()).unwrap_or(false)) {
            // A whole-row range: 3:5, $3:$5 handled with refs below.
            let mut j = i;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if b.get(j) == Some(&b':') {
                let mut k = j + 1;
                if b.get(k) == Some(&b'$') {
                    k += 1;
                }
                let ks = k;
                while k < b.len() && b[k].is_ascii_digit() {
                    k += 1;
                }
                if k > ks {
                    if let Some(a) = parse_area(&s[i..k]) {
                        out.push(Tok { kind: TokKind::Area(a), start, end: k });
                        i = k;
                        continue;
                    }
                }
            }
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'.') {
                j += 1;
            }
            if j < b.len() && (b[j] == b'e' || b[j] == b'E') && b.get(j + 1).map(|x| x.is_ascii_digit() || *x == b'+' || *x == b'-').unwrap_or(false) {
                j += 2;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
            }
            let v: f64 = s[i..j].parse().map_err(|_| format!("bad number {}", &s[i..j]))?;
            out.push(Tok { kind: TokKind::Num(v), start, end: j });
            i = j;
            continue;
        }
        // References, names and functions. A word runs to the next delimiter; a quoted sheet
        // name comes before its `!`.
        let mut j = i;
        if c == b'\'' {
            j += 1;
            loop {
                if j >= b.len() {
                    return Err("unterminated sheet name".into());
                }
                if b[j] == b'\'' {
                    if b.get(j + 1) == Some(&b'\'') {
                        j += 2;
                        continue;
                    }
                    j += 1;
                    break;
                }
                j += 1;
            }
            if b.get(j) != Some(&b'!') {
                return Err("a quoted name must be a sheet before !".into());
            }
            j += 1;
        }
        let word_end = |mut k: usize| {
            while k < b.len() {
                let ch = b[k];
                if ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'.' || ch == b'$' || ch == b'\\' || ch >= 0x80 || ch == b'?' {
                    k += 1;
                } else {
                    break;
                }
            }
            k
        };
        j = word_end(j);
        // Sheet!ref
        if b.get(j) == Some(&b'!') {
            j = word_end(j + 1);
            if b.get(j) == Some(&b'#') && s[j..].to_ascii_uppercase().starts_with("#REF!") {
                j += 5;
                out.push(Tok { kind: TokKind::BadRef, start, end: j });
                i = j;
                continue;
            }
        }
        // A range continues after ':'.
        let mut k = j;
        if b.get(k) == Some(&b':') {
            let k2 = word_end(k + 1);
            if k2 > k + 1 && parse_area(&s[start..k2]).is_some() {
                k = k2;
            }
        }
        if k > j {
            j = k;
        }
        if j == i {
            let ch = s[i..].chars().next().unwrap();
            return Err(format!("unexpected {ch:?}"));
        }
        let word = &s[i..j];
        // Structured references (Table1[Column]) are kept whole as names.
        if b.get(j) == Some(&b'[') {
            let mut depth = 0;
            let mut k = j;
            while k < b.len() {
                if b[k] == b'[' {
                    depth += 1;
                } else if b[k] == b']' {
                    depth -= 1;
                    if depth == 0 {
                        k += 1;
                        break;
                    }
                }
                k += 1;
            }
            out.push(Tok { kind: TokKind::Name(s[i..k].to_string()), start, end: k });
            i = k;
            continue;
        }
        if b.get(j) == Some(&b'(') {
            out.push(Tok { kind: TokKind::Func(word.to_string()), start, end: j });
        } else if word.eq_ignore_ascii_case("TRUE") {
            out.push(Tok { kind: TokKind::Bool(true), start, end: j });
        } else if word.eq_ignore_ascii_case("FALSE") {
            out.push(Tok { kind: TokKind::Bool(false), start, end: j });
        } else if let Some(a) = parse_area(word) {
            out.push(Tok { kind: TokKind::Area(a), start, end: j });
        } else if word.contains('!') {
            out.push(Tok { kind: TokKind::BadRef, start, end: j });
        } else {
            out.push(Tok { kind: TokKind::Name(word.to_string()), start, end: j });
        }
        i = j;
    }
    Ok(out)
}

// ── parse ───────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum Expr {
    Lit(Val),
    Area(Area),
    Name(String),
    Neg(Box<Expr>),
    Pct(Box<Expr>),
    Bin(String, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Array(Vec<Vec<Expr>>),
    Missing,
    Range(Box<Expr>, Box<Expr>),
}

struct P {
    t: Vec<Tok>,
    i: usize,
}

impl P {
    fn peek(&self) -> Option<&TokKind> {
        self.t.get(self.i).map(|t| &t.kind)
    }
    fn next(&mut self) -> Option<TokKind> {
        let k = self.t.get(self.i).map(|t| t.kind.clone());
        self.i += 1;
        k
    }
    fn expr(&mut self, min: u8) -> Result<Expr, String> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(TokKind::Op(o)) if o != "%" => o.clone(),
                _ => break,
            };
            let (prec, right) = match op.as_str() {
                "=" | "<" | ">" | "<=" | ">=" | "<>" => (1, false),
                "&" => (2, false),
                "+" | "-" => (3, false),
                "*" | "/" => (4, false),
                "^" => (5, false),
                ":" => (8, false),
                _ => return Err(format!("unknown operator {op}")),
            };
            if prec < min {
                break;
            }
            self.next();
            let rhs = self.expr(if right { prec } else { prec + 1 })?;
            lhs = if op == ":" { Expr::Range(Box::new(lhs), Box::new(rhs)) } else { Expr::Bin(op, Box::new(lhs), Box::new(rhs)) };
        }
        Ok(lhs)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(TokKind::Op(o)) if o == "-" => {
                self.next();
                // Negation binds tighter than ^ in Excel (-2^2 = 4).
                let e = self.unary()?;
                Ok(self.postfix(Expr::Neg(Box::new(e))))
            }
            Some(TokKind::Op(o)) if o == "+" => {
                self.next();
                self.unary()
            }
            _ => {
                let p = self.primary()?;
                Ok(self.postfix(p))
            }
        }
    }
    fn postfix(&mut self, mut e: Expr) -> Expr {
        while let Some(TokKind::Op(o)) = self.peek() {
            if o == "%" {
                self.next();
                e = Expr::Pct(Box::new(e));
            } else {
                break;
            }
        }
        e
    }
    fn primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(TokKind::Num(n)) => Ok(Expr::Lit(Val::Num(n))),
            Some(TokKind::Str(s)) => Ok(Expr::Lit(Val::Str(s))),
            Some(TokKind::Bool(b)) => Ok(Expr::Lit(Val::Bool(b))),
            Some(TokKind::Err(e)) => Ok(Expr::Lit(Val::Err(e))),
            Some(TokKind::BadRef) => Ok(Expr::Lit(Val::Err(XlErr::Ref))),
            Some(TokKind::Area(a)) => Ok(Expr::Area(a)),
            Some(TokKind::Name(n)) => Ok(Expr::Name(n)),
            Some(TokKind::LParen) => {
                let e = self.expr(0)?;
                match self.next() {
                    Some(TokKind::RParen) => Ok(e),
                    _ => Err("missing )".into()),
                }
            }
            Some(TokKind::Func(name)) => {
                self.next(); // (
                let mut args = Vec::new();
                if self.peek() == Some(&TokKind::RParen) {
                    self.next();
                    return Ok(Expr::Call(canonical_fn(&name), args));
                }
                loop {
                    match self.peek() {
                        Some(TokKind::Comma) => {
                            args.push(Expr::Missing);
                            self.next();
                            continue;
                        }
                        Some(TokKind::RParen) => {
                            args.push(Expr::Missing);
                            self.next();
                            break;
                        }
                        _ => {}
                    }
                    args.push(self.expr(0)?);
                    match self.next() {
                        Some(TokKind::Comma) => {
                            if self.peek() == Some(&TokKind::RParen) {
                                args.push(Expr::Missing);
                                self.next();
                                break;
                            }
                        }
                        Some(TokKind::RParen) => break,
                        _ => return Err(format!("missing ) after the arguments of {name}")),
                    }
                }
                Ok(Expr::Call(canonical_fn(&name), args))
            }
            Some(TokKind::LBrace) => {
                let mut rows = vec![Vec::new()];
                loop {
                    let e = self.unary()?;
                    rows.last_mut().unwrap().push(e);
                    match self.next() {
                        Some(TokKind::Comma) => {}
                        Some(TokKind::Semi) => rows.push(Vec::new()),
                        Some(TokKind::RBrace) => break,
                        _ => return Err("bad array constant".into()),
                    }
                }
                Ok(Expr::Array(rows))
            }
            Some(k) => Err(format!("unexpected {k:?}")),
            None => Err("formula ends too soon".into()),
        }
    }
}

/// Function names without the file format's prefixes (`_xlfn.`, `_xlws.`).
pub fn canonical_fn(n: &str) -> String {
    let mut s = n.to_ascii_uppercase();
    for p in ["_XLFN.", "_XLWS.", "_XLPM."] {
        while let Some(r) = s.strip_prefix(p) {
            s = r.to_string();
        }
    }
    s
}

pub fn parse(f: &str) -> Result<Expr, String> {
    let toks: Vec<Tok> = tokenize(f)?.into_iter().filter(|t| t.kind != TokKind::Space).collect();
    let mut p = P { t: toks, i: 0 };
    let e = p.expr(0)?;
    if p.i < p.t.len() {
        return Err(format!("unexpected text at {}", p.t[p.i].start));
    }
    Ok(e)
}

/// Functions newer than Excel 2007 must be written with `_xlfn.` in the file, or Excel shows
/// #NAME?.
const FUTURE: &[&str] = &[
    "ACOT", "ACOTH", "AGGREGATE", "ARABIC", "BASE", "BETA.DIST", "BETA.INV", "BINOM.DIST", "BINOM.DIST.RANGE", "BINOM.INV", "BITAND", "BITLSHIFT",
    "BITOR", "BITRSHIFT", "BITXOR", "CEILING.MATH", "CEILING.PRECISE", "CHISQ.DIST", "CHISQ.DIST.RT", "CHISQ.INV", "CHISQ.INV.RT", "CHISQ.TEST",
    "CHOOSECOLS", "CHOOSEROWS", "COMBINA", "CONCAT", "CONFIDENCE.NORM", "CONFIDENCE.T", "COT", "COTH", "COVARIANCE.P", "COVARIANCE.S", "CSC", "CSCH",
    "DAYS", "DECIMAL", "DROP", "ERF.PRECISE", "ERFC.PRECISE", "EXPAND", "EXPON.DIST", "F.DIST", "F.DIST.RT", "F.INV", "F.INV.RT", "F.TEST", "FILTER",
    "FILTERXML", "FLOOR.MATH", "FLOOR.PRECISE", "FORECAST.LINEAR", "FORMULATEXT", "GAMMA", "GAMMA.DIST", "GAMMA.INV", "GAMMALN.PRECISE", "GAUSS",
    "HSTACK", "HYPGEOM.DIST", "IFNA", "IFS", "IMAGE", "ISFORMULA", "ISOMITTED", "ISOWEEKNUM", "LAMBDA", "LET", "LOGNORM.DIST", "LOGNORM.INV", "MAKEARRAY",
    "MAP", "MAXIFS", "MINIFS", "MODE.MULT", "MODE.SNGL", "NEGBINOM.DIST", "NETWORKDAYS.INTL", "NORM.DIST", "NORM.INV", "NORM.S.DIST", "NORM.S.INV",
    "NUMBERVALUE", "PDURATION", "PERCENTILE.EXC", "PERCENTILE.INC", "PERCENTRANK.EXC", "PERCENTRANK.INC", "PERMUTATIONA", "PHI", "POISSON.DIST",
    "QUARTILE.EXC", "QUARTILE.INC", "RANDARRAY", "RANK.AVG", "RANK.EQ", "REDUCE", "RRI", "SCAN", "SEC", "SECH", "SEQUENCE", "SHEET", "SHEETS", "SKEW.P",
    "SORT", "SORTBY", "STDEV.P", "STDEV.S", "SWITCH", "T.DIST", "T.DIST.2T", "T.DIST.RT", "T.INV", "T.INV.2T", "T.TEST", "TAKE", "TEXTAFTER",
    "TEXTBEFORE", "TEXTJOIN", "TEXTSPLIT", "TOCOL", "TOROW", "UNICHAR", "UNICODE", "UNIQUE", "VALUETOTEXT", "ARRAYTOTEXT", "VAR.P", "VAR.S", "VSTACK",
    "WEBSERVICE", "WEIBULL.DIST", "WORKDAY.INTL", "WRAPCOLS", "WRAPROWS", "XLOOKUP", "XMATCH", "XOR", "Z.TEST",
];

/// The formula as the file stores it: no `=`, newer functions with their `_xlfn.` prefix.
pub fn to_file(f: &str) -> Result<String, String> {
    let f = f.trim_start_matches('=');
    let toks = tokenize(f)?;
    let mut out = String::with_capacity(f.len() + 8);
    let mut last = 0;
    for t in &toks {
        if let TokKind::Func(name) = &t.kind {
            let canon = canonical_fn(name);
            if FUTURE.contains(&canon.as_str()) && !name.to_ascii_uppercase().starts_with("_XLFN.") {
                out.push_str(&f[last..t.start]);
                let ws = if matches!(canon.as_str(), "FILTER" | "SORT" | "SORTBY" | "UNIQUE" | "SEQUENCE" | "RANDARRAY") { "_xlws." } else { "" };
                out.push_str(&format!("_xlfn.{ws}{canon}"));
                last = t.end;
            }
        }
    }
    out.push_str(&f[last..]);
    Ok(out)
}

/// The formula as people write it: `=` and no prefixes.
pub fn for_display(f: &str) -> String {
    let mut s = f.to_string();
    for p in ["_xlfn._xlws.", "_xlfn.", "_xlws.", "_xlpm."] {
        s = s.replace(p, "");
    }
    format!("={s}")
}

/// Rewrites the references of a formula: `map` gets each area (with the sheet it names, or None
/// for the formula's own sheet) and returns its new form (None: #REF!). Text outside references
/// stays as it was.
pub fn rewrite_refs(f: &str, map: &mut dyn FnMut(&Area) -> Option<Option<Area>>) -> Result<String, String> {
    let toks = tokenize(f)?;
    let mut out = String::with_capacity(f.len());
    let mut last = 0;
    for t in &toks {
        if let TokKind::Area(a) = &t.kind {
            if let Some(new) = map(a) {
                out.push_str(&f[last..t.start]);
                match new {
                    Some(n) => out.push_str(&n.text()),
                    None => {
                        let sheet = a.sheet.as_ref().map(|s| format!("{}!", quote_sheet(s))).unwrap_or_default();
                        out.push_str(&format!("{sheet}#REF!"));
                    }
                }
                last = t.end;
            }
        }
    }
    out.push_str(&f[last..]);
    Ok(out)
}

/// A shared formula's text for a cell `dr` rows and `dc` columns from its master.
pub fn shift(f: &str, dr: i64, dc: i64) -> String {
    rewrite_refs(f, &mut |a| {
        let mut n = a.clone();
        let mv = |r: &mut CellRef| -> bool {
            if !r.abs_row && a.whole != Some('c') {
                let v = r.row as i64 + dr;
                if !(0..=MAX_ROW as i64).contains(&v) {
                    return false;
                }
                r.row = v as u32;
            }
            if !r.abs_col && a.whole != Some('r') {
                let v = r.col as i64 + dc;
                if !(0..=MAX_COL as i64).contains(&v) {
                    return false;
                }
                r.col = v as u32;
            }
            true
        };
        if !mv(&mut n.a) || !mv(&mut n.b) {
            return Some(None);
        }
        Some(Some(n))
    })
    .unwrap_or_else(|_| f.to_string())
}

// ── evaluate ────────────────────────────────────────────────────────────────────────────────

pub trait Ctx {
    /// The value of a cell (formulas evaluated).
    fn cell(&mut self, sheet: Option<&str>, row: u32, col: u32) -> Val;
    /// The used rows and columns of a sheet (for whole-column references).
    fn extent(&mut self, sheet: Option<&str>) -> (u32, u32);
    /// A defined name's formula.
    fn name(&mut self, name: &str) -> Option<String>;
    fn sheet_exists(&mut self, sheet: &str) -> bool;
    /// Where the formula is (for ROW(), COLUMN(), relative evaluation).
    fn here(&self) -> (u32, u32);
    fn date1904(&self) -> bool {
        false
    }
    fn now(&self) -> f64;
}

pub fn eval(e: &Expr, cx: &mut dyn Ctx) -> Val {
    match e {
        Expr::Lit(v) => v.clone(),
        Expr::Missing => Val::Empty,
        Expr::Area(a) => area_val(a, cx),
        Expr::Name(n) => match cx.name(n) {
            Some(f) => match parse(f.trim_start_matches('=')) {
                Ok(e) => eval(&e, cx),
                Err(_) => Val::Err(XlErr::Name),
            },
            None => Val::Err(XlErr::Name),
        },
        Expr::Neg(x) => map1(eval(x, cx), &|v| num(&v).map(|n| Val::Num(-n)).unwrap_or_else(Val::Err)),
        Expr::Pct(x) => map1(eval(x, cx), &|v| num(&v).map(|n| Val::Num(n / 100.0)).unwrap_or_else(Val::Err)),
        Expr::Bin(op, a, b) => {
            let x = eval(a, cx);
            let y = eval(b, cx);
            binop(op, x, y)
        }
        Expr::Array(rows) => Val::Arr(rows.iter().map(|r| r.iter().map(|e| eval(e, cx).first()).collect()).collect()),
        Expr::Range(a, b) => {
            // A range between two references (A1:INDEX(...)): only areas are supported.
            match (ref_of(a, cx), ref_of(b, cx)) {
                (Some(x), Some(y)) => {
                    let area = Area { sheet: x.sheet.clone(), a: CellRef { row: x.rows().0.min(y.rows().0), col: x.cols().0.min(y.cols().0), abs_row: false, abs_col: false }, b: CellRef { row: x.rows().1.max(y.rows().1), col: x.cols().1.max(y.cols().1), abs_row: false, abs_col: false }, whole: None };
                    area_val(&area, cx)
                }
                _ => Val::Err(XlErr::Ref),
            }
        }
        Expr::Call(name, args) => call(name, args, cx),
    }
}

fn area_val(a: &Area, cx: &mut dyn Ctx) -> Val {
    if let Some(s) = &a.sheet {
        if !cx.sheet_exists(s) {
            return Val::Err(XlErr::Ref);
        }
    }
    if a.single() {
        return cx.cell(a.sheet.as_deref(), a.a.row, a.a.col);
    }
    let (r0, mut r1) = a.rows();
    let (c0, mut c1) = a.cols();
    let (er, ec) = cx.extent(a.sheet.as_deref());
    r1 = r1.min(er.max(r0));
    c1 = c1.min(ec.max(c0));
    let mut rows = Vec::with_capacity((r1 - r0 + 1) as usize);
    for r in r0..=r1 {
        let mut row = Vec::with_capacity((c1 - c0 + 1) as usize);
        for c in c0..=c1 {
            row.push(cx.cell(a.sheet.as_deref(), r, c));
        }
        rows.push(row);
    }
    Val::Arr(rows)
}

/// The area an expression refers to (for ROW, COLUMN, OFFSET, INDEX as a reference).
fn ref_of(e: &Expr, cx: &mut dyn Ctx) -> Option<Area> {
    match e {
        Expr::Area(a) => Some(a.clone()),
        Expr::Name(n) => {
            let f = cx.name(n)?;
            match parse(f.trim_start_matches('=')).ok()? {
                Expr::Area(a) => Some(a),
                _ => None,
            }
        }
        Expr::Call(name, args) if name == "OFFSET" => offset_area(args, cx),
        Expr::Call(name, args) if name == "INDIRECT" => {
            let s = eval(args.first()?, cx).first().as_text();
            parse_area(&s)
        }
        Expr::Call(name, args) if name == "INDEX" => {
            let base = ref_of(args.first()?, cx)?;
            let r = args.get(1).map(|e| num(&eval(e, cx).first()).unwrap_or(0.0)).unwrap_or(0.0) as i64;
            let c = args.get(2).map(|e| num(&eval(e, cx).first()).unwrap_or(0.0)).unwrap_or(0.0) as i64;
            let (r0, r1) = base.rows();
            let (c0, c1) = base.cols();
            let (rr0, rr1) = if r > 0 { (r0 + r as u32 - 1, r0 + r as u32 - 1) } else { (r0, r1) };
            let (cc0, cc1) = if c > 0 { (c0 + c as u32 - 1, c0 + c as u32 - 1) } else if c1 == c0 { (c0, c0) } else if r > 0 && r1 == r0 { (c0 + (r as u32 - 1), c0 + (r as u32 - 1)) } else { (c0, c1) };
            Some(Area { sheet: base.sheet, a: CellRef { row: rr0, col: cc0, abs_row: false, abs_col: false }, b: CellRef { row: rr1, col: cc1, abs_row: false, abs_col: false }, whole: None })
        }
        _ => None,
    }
}

fn offset_area(args: &[Expr], cx: &mut dyn Ctx) -> Option<Area> {
    let base = ref_of(args.first()?, cx)?;
    let n = |i: usize, cx: &mut dyn Ctx| -> Option<f64> {
        match args.get(i) {
            Some(Expr::Missing) | None => None,
            Some(e) => num(&eval(e, cx).first()).ok(),
        }
    };
    let dr = n(1, cx).unwrap_or(0.0) as i64;
    let dc = n(2, cx).unwrap_or(0.0) as i64;
    let (r0, r1) = base.rows();
    let (c0, c1) = base.cols();
    let h = n(3, cx).map(|v| v as i64).unwrap_or((r1 - r0 + 1) as i64);
    let w = n(4, cx).map(|v| v as i64).unwrap_or((c1 - c0 + 1) as i64);
    let nr = r0 as i64 + dr;
    let nc = c0 as i64 + dc;
    if nr < 0 || nc < 0 || h < 1 || w < 1 {
        return None;
    }
    Some(Area {
        sheet: base.sheet,
        a: CellRef { row: nr as u32, col: nc as u32, abs_row: false, abs_col: false },
        b: CellRef { row: (nr + h - 1) as u32, col: (nc + w - 1) as u32, abs_row: false, abs_col: false },
        whole: None,
    })
}

fn map1(v: Val, f: &dyn Fn(Val) -> Val) -> Val {
    match v {
        Val::Arr(a) => Val::Arr(a.into_iter().map(|r| r.into_iter().map(f).collect()).collect()),
        v => f(v),
    }
}

fn map2(x: Val, y: Val, f: &dyn Fn(Val, Val) -> Val) -> Val {
    match (x, y) {
        (Val::Arr(a), Val::Arr(b)) => {
            let rows = a.len().max(b.len());
            let cols = a.iter().map(|r| r.len()).max().unwrap_or(0).max(b.iter().map(|r| r.len()).max().unwrap_or(0));
            let get = |m: &Vec<Vec<Val>>, r: usize, c: usize| -> Val {
                // Single rows/columns broadcast.
                let rr = if m.len() == 1 { 0 } else { r };
                let row = match m.get(rr) {
                    Some(x) => x,
                    None => return Val::Err(XlErr::NA),
                };
                let cc = if row.len() == 1 { 0 } else { c };
                row.get(cc).cloned().unwrap_or(Val::Err(XlErr::NA))
            };
            Val::Arr((0..rows).map(|r| (0..cols).map(|c| f(get(&a, r, c), get(&b, r, c))).collect()).collect())
        }
        (Val::Arr(a), y) => Val::Arr(a.into_iter().map(|r| r.into_iter().map(|v| f(v, y.clone())).collect()).collect()),
        (x, Val::Arr(b)) => Val::Arr(b.into_iter().map(|r| r.into_iter().map(|v| f(x.clone(), v)).collect()).collect()),
        (x, y) => f(x, y),
    }
}

/// A value as a number for arithmetic: text that reads as a number counts, empty is 0.
pub fn num(v: &Val) -> Result<f64, XlErr> {
    match v {
        Val::Num(n) => Ok(*n),
        Val::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        Val::Empty => Ok(0.0),
        Val::Str(s) => text_to_num(s).ok_or(XlErr::Value),
        Val::Err(e) => Err(e.clone()),
        Val::Arr(_) => num(&v.clone().first()),
    }
}

/// Text Excel reads as a number: `12`, `1,234.5`, `12%`, `$5`, `2024-01-15`, `3:30`.
pub fn text_to_num(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(v) = t.parse::<f64>() {
        return Some(v);
    }
    let (neg, t2) = if t.starts_with('(') && t.ends_with(')') { (true, &t[1..t.len() - 1]) } else { (false, t) };
    let mut u = t2.replace(',', "");
    let mut scale = 1.0;
    if let Some(x) = u.strip_suffix('%') {
        u = x.to_string();
        scale = 0.01;
    }
    for sym in ["$", "₩", "€", "£", "¥"] {
        if let Some(x) = u.strip_prefix(sym) {
            u = x.to_string();
        }
    }
    if let Ok(v) = u.trim().parse::<f64>() {
        return Some(if neg { -v } else { v } * scale);
    }
    parse_date_time(t)
}

/// `2024-01-15`, `2024/1/15`, `2024-01-15 13:30`, `13:30`, `1:30 PM` → a serial number.
pub fn parse_date_time(t: &str) -> Option<f64> {
    let t = t.trim();
    let (date_part, time_part) = match t.split_once(' ') {
        Some((a, b)) if a.contains(['-', '/', '.']) => (Some(a), Some(b.trim())),
        _ if t.contains(':') => (None, Some(t)),
        _ => (Some(t), None),
    };
    let mut serial = 0.0;
    if let Some(d) = date_part {
        let parts: Vec<&str> = d.split(['-', '/', '.']).collect();
        if parts.len() != 3 {
            return None;
        }
        let nums: Vec<i64> = parts.iter().map(|p| p.trim().parse::<i64>()).collect::<Result<_, _>>().ok()?;
        let (y, m, dd) = if parts[0].len() == 4 { (nums[0], nums[1], nums[2]) } else if parts[2].len() == 4 { (nums[2], nums[0], nums[1]) } else { return None };
        if !(1..=12).contains(&m) || !(1..=31).contains(&dd) || !(1900..=9999).contains(&y) {
            return None;
        }
        serial = numfmt::datetime_to_serial(y, m as u32, dd as u32, 0, 0, 0.0);
    }
    if let Some(tp) = time_part {
        let low = tp.to_ascii_lowercase();
        let (tp, pm, am) = if let Some(x) = low.strip_suffix("pm") { (x.trim().to_string(), true, false) } else if let Some(x) = low.strip_suffix("am") { (x.trim().to_string(), false, true) } else { (low.clone(), false, false) };
        let parts: Vec<&str> = tp.split(':').collect();
        if parts.len() < 2 || parts.len() > 3 {
            return None;
        }
        let mut h: f64 = parts[0].parse().ok()?;
        let m: f64 = parts[1].parse().ok()?;
        let s: f64 = parts.get(2).map(|x| x.parse().ok()).unwrap_or(Some(0.0))?;
        if pm && h < 12.0 {
            h += 12.0;
        }
        if am && h == 12.0 {
            h = 0.0;
        }
        serial += (h * 3600.0 + m * 60.0 + s) / 86400.0;
    }
    Some(serial)
}

fn text_of(v: &Val) -> Result<String, XlErr> {
    match v {
        Val::Err(e) => Err(e.clone()),
        Val::Arr(_) => text_of(&v.clone().first()),
        v => Ok(v.as_text()),
    }
}

fn truth(v: &Val) -> Result<bool, XlErr> {
    match v {
        Val::Bool(b) => Ok(*b),
        Val::Num(n) => Ok(*n != 0.0),
        Val::Empty => Ok(false),
        Val::Str(s) => match s.to_ascii_uppercase().as_str() {
            "TRUE" => Ok(true),
            "FALSE" => Ok(false),
            _ => Err(XlErr::Value),
        },
        Val::Err(e) => Err(e.clone()),
        Val::Arr(_) => truth(&v.clone().first()),
    }
}

/// Excel's ordering: numbers < text < booleans; text without case.
fn compare(a: &Val, b: &Val) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let rank = |v: &Val| match v {
        Val::Num(_) | Val::Empty => 0,
        Val::Str(_) => 1,
        Val::Bool(_) => 2,
        _ => 3,
    };
    match (a, b) {
        (Val::Empty, Val::Str(s)) => "".cmp(&s.to_lowercase().as_str()),
        (Val::Str(s), Val::Empty) => s.to_lowercase().as_str().cmp(""),
        (Val::Empty, Val::Bool(x)) => false.cmp(x),
        (Val::Bool(x), Val::Empty) => x.cmp(&false),
        _ => {
            let (ra, rb) = (rank(a), rank(b));
            if ra != rb {
                return ra.cmp(&rb);
            }
            match (a, b) {
                (Val::Str(x), Val::Str(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
                (Val::Bool(x), Val::Bool(y)) => x.cmp(y),
                _ => {
                    let x = num(a).unwrap_or(0.0);
                    let y = num(b).unwrap_or(0.0);
                    x.partial_cmp(&y).unwrap_or(Equal)
                }
            }
        }
    }
}

fn binop(op: &str, x: Val, y: Val) -> Val {
    let op = op.to_string();
    map2(x, y, &move |a, b| {
        if let Val::Err(e) = &a {
            return Val::Err(e.clone());
        }
        if let Val::Err(e) = &b {
            return Val::Err(e.clone());
        }
        match op.as_str() {
            "&" => Val::Str(format!("{}{}", a.as_text(), b.as_text())),
            "=" | "<>" | "<" | ">" | "<=" | ">=" => {
                let o = compare(&a, &b);
                use std::cmp::Ordering::*;
                Val::Bool(match op.as_str() {
                    "=" => o == Equal,
                    "<>" => o != Equal,
                    "<" => o == Less,
                    ">" => o == Greater,
                    "<=" => o != Greater,
                    _ => o != Less,
                })
            }
            _ => {
                let (p, q) = match (num(&a), num(&b)) {
                    (Ok(p), Ok(q)) => (p, q),
                    (Err(e), _) | (_, Err(e)) => return Val::Err(e),
                };
                let r = match op.as_str() {
                    "+" => p + q,
                    "-" => p - q,
                    "*" => p * q,
                    "/" => {
                        if q == 0.0 {
                            return Val::Err(XlErr::Div0);
                        }
                        p / q
                    }
                    "^" => {
                        if p == 0.0 && q == 0.0 {
                            return Val::Err(XlErr::Num);
                        }
                        p.powf(q)
                    }
                    _ => return Val::Err(XlErr::Value),
                };
                if r.is_finite() {
                    Val::Num(r)
                } else {
                    Val::Err(XlErr::Num)
                }
            }
        }
    })
}

/// Every value in the arguments, flattened, with whether each came from a reference (where text
/// and booleans do not count for SUM-like functions).
fn flatten(args: &[Expr], cx: &mut dyn Ctx) -> Vec<(Val, bool)> {
    let mut out = Vec::new();
    for a in args {
        if matches!(a, Expr::Missing) {
            continue;
        }
        let from_ref = matches!(a, Expr::Area(_) | Expr::Name(_) | Expr::Range(_, _)) || matches!(a, Expr::Call(n, _) if n == "OFFSET" || n == "INDIRECT" || n == "INDEX");
        match eval(a, cx) {
            Val::Arr(rows) => {
                for r in rows {
                    for v in r {
                        out.push((v, true));
                    }
                }
            }
            v => out.push((v, from_ref)),
        }
    }
    out
}

/// The numbers in the arguments as SUM sees them; an error stops it.
fn numbers(args: &[Expr], cx: &mut dyn Ctx, count_text: bool) -> Result<Vec<f64>, XlErr> {
    let mut out = Vec::new();
    for (v, from_ref) in flatten(args, cx) {
        match v {
            Val::Num(n) => out.push(n),
            Val::Err(e) => return Err(e),
            Val::Bool(b) if !from_ref || count_text => out.push(if b { 1.0 } else { 0.0 }),
            Val::Str(s) if !from_ref => out.push(text_to_num(&s).ok_or(XlErr::Value)?),
            Val::Str(_) if count_text => out.push(0.0),
            _ => {}
        }
    }
    Ok(out)
}

fn arg(args: &[Expr], i: usize, cx: &mut dyn Ctx) -> Val {
    match args.get(i) {
        Some(e) => eval(e, cx),
        None => Val::Empty,
    }
}

fn argn(args: &[Expr], i: usize, cx: &mut dyn Ctx) -> Result<f64, XlErr> {
    num(&arg(args, i, cx).first())
}

fn argn_or(args: &[Expr], i: usize, cx: &mut dyn Ctx, d: f64) -> Result<f64, XlErr> {
    match args.get(i) {
        None | Some(Expr::Missing) => Ok(d),
        Some(_) => argn(args, i, cx),
    }
}

fn args_t(args: &[Expr], i: usize, cx: &mut dyn Ctx) -> Result<String, XlErr> {
    text_of(&arg(args, i, cx).first())
}

fn matrix(v: Val) -> Vec<Vec<Val>> {
    match v {
        Val::Arr(a) => a,
        v => vec![vec![v]],
    }
}

/// COUNTIF-style criteria: `>5`, `<>x`, `a*`, `=`, a number or text.
fn criterion(c: &Val) -> Box<dyn Fn(&Val) -> bool> {
    let (op, rhs): (&str, Val) = match c {
        Val::Str(s) => {
            let ops = ["<=", ">=", "<>", "<", ">", "="];
            let op = ops.iter().find(|o| s.starts_with(*o)).copied().unwrap_or("");
            let rest = &s[op.len()..];
            let v = if let Some(n) = text_to_num(rest) {
                Val::Num(n)
            } else if rest.eq_ignore_ascii_case("TRUE") {
                Val::Bool(true)
            } else if rest.eq_ignore_ascii_case("FALSE") {
                Val::Bool(false)
            } else {
                Val::Str(rest.to_string())
            };
            (if op.is_empty() { "=" } else { op }, v)
        }
        Val::Empty => ("=", Val::Str(String::new())),
        v => ("=", v.clone()),
    };
    let op = op.to_string();
    match rhs {
        Val::Str(pat) => {
            let wild = pat.contains(['*', '?']);
            let re = if wild { Some(wildcard(&pat)) } else { None };
            let low = pat.to_lowercase();
            Box::new(move |v: &Val| {
                let t = match v {
                    Val::Str(s) => Some(s.to_lowercase()),
                    Val::Empty => Some(String::new()),
                    _ => None,
                };
                let eq = match (&t, &re) {
                    (Some(t), Some(re)) => re.is_match(t),
                    (Some(t), None) => *t == low,
                    (None, _) => false,
                };
                match op.as_str() {
                    "=" => eq,
                    "<>" => !eq,
                    o => match &t {
                        Some(t) if !matches!(v, Val::Empty) => {
                            let ord = t.as_str().cmp(low.as_str());
                            match o {
                                "<" => ord.is_lt(),
                                ">" => ord.is_gt(),
                                "<=" => ord.is_le(),
                                _ => ord.is_ge(),
                            }
                        }
                        _ => false,
                    },
                }
            })
        }
        rhs => Box::new(move |v: &Val| {
            let x = match v {
                Val::Num(n) => *n,
                Val::Str(s) => match text_to_num(s) {
                    Some(n) if op == "=" => n,
                    _ => return op == "<>",
                },
                Val::Bool(b) => {
                    return match (&rhs, op.as_str()) {
                        (Val::Bool(r), "=") => b == r,
                        (Val::Bool(r), "<>") => b != r,
                        _ => op == "<>",
                    }
                }
                Val::Empty => return op == "<>",
                _ => return false,
            };
            let r = match &rhs {
                Val::Num(n) => *n,
                Val::Bool(_) => return op == "<>",
                _ => return false,
            };
            match op.as_str() {
                "=" => x == r,
                "<>" => x != r,
                "<" => x < r,
                ">" => x > r,
                "<=" => x <= r,
                _ => x >= r,
            }
        }),
    }
}

fn wildcard(p: &str) -> regex::Regex {
    let mut s = String::from("^(?s)");
    let mut it = p.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '~' => {
                if let Some(n) = it.next() {
                    s.push_str(&regex::escape(&n.to_string()));
                }
            }
            '*' => s.push_str(".*"),
            '?' => s.push('.'),
            c => s.push_str(&regex::escape(&c.to_lowercase().to_string())),
        }
    }
    s.push('$');
    regex::Regex::new(&s).unwrap_or_else(|_| regex::Regex::new("^$").unwrap())
}

fn flat(m: &[Vec<Val>]) -> Vec<Val> {
    m.iter().flat_map(|r| r.iter().cloned()).collect()
}

fn round_to(x: f64, d: i32, mode: i32) -> f64 {
    let f = 10f64.powi(d);
    let y = x * f;
    // Correct for binary representation before rounding (as Excel's 15 digits).
    let y = format!("{:.9}", y).parse::<f64>().unwrap_or(y);
    let r = match mode {
        0 => y.abs().round().copysign(y),
        1 => y.abs().ceil().copysign(y),
        _ => y.abs().floor().copysign(y),
    };
    r / f
}

fn ok(n: f64) -> Val {
    if n.is_finite() {
        Val::Num(n)
    } else {
        Val::Err(XlErr::Num)
    }
}

macro_rules! tryv {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return Val::Err(e),
        }
    };
}

fn ymd(serial: f64, cx: &dyn Ctx) -> Option<(i64, u32, u32)> {
    numfmt::serial_to_datetime(serial, cx.date1904()).map(|(y, m, d, ..)| (y, m, d))
}

fn date_serial(y: i64, m: i64, d: i64) -> f64 {
    // Months and days overflow into the next as Excel's DATE does.
    let total = y * 12 + (m - 1);
    let (yy, mm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let base = numfmt::datetime_to_serial(yy, mm as u32, 1, 0, 0, 0.0);
    base + (d - 1) as f64
}

fn days_in_month(y: i64, m: u32) -> u32 {
    let next = if m == 12 { numfmt::days_from_civil(y + 1, 1, 1) } else { numfmt::days_from_civil(y, m + 1, 1) };
    (next - numfmt::days_from_civil(y, m, 1)) as u32
}

fn call(name: &str, args: &[Expr], cx: &mut dyn Ctx) -> Val {
    let n = args.len();
    match name {
        // ── logic
        "IF" => {
            let c = arg(args, 0, cx);
            if let Val::Arr(_) = c {
                let t = arg(args, 1, cx);
                let f = if n > 2 { arg(args, 2, cx) } else { Val::Bool(false) };
                return map2(map2(c, t, &|c, t| Val::Arr(vec![vec![c, t]])), f, &|ct, f| {
                    if let Val::Arr(v) = ct {
                        match truth(&v[0][0]) {
                            Ok(true) => v[0][1].clone(),
                            Ok(false) => f,
                            Err(e) => Val::Err(e),
                        }
                    } else {
                        ct
                    }
                });
            }
            match truth(&c) {
                Ok(true) => if n > 1 && !matches!(args[1], Expr::Missing) { arg(args, 1, cx) } else if n > 1 { Val::Num(0.0) } else { Val::Bool(true) },
                Ok(false) => if n > 2 { if matches!(args[2], Expr::Missing) { Val::Num(0.0) } else { arg(args, 2, cx) } } else { Val::Bool(false) },
                Err(e) => Val::Err(e),
            }
        }
        "IFS" => {
            let mut i = 0;
            while i + 1 < n {
                match truth(&arg(args, i, cx).first()) {
                    Ok(true) => return arg(args, i + 1, cx),
                    Ok(false) => {}
                    Err(e) => return Val::Err(e),
                }
                i += 2;
            }
            Val::Err(XlErr::NA)
        }
        "IFERROR" => match arg(args, 0, cx) {
            Val::Err(_) => arg(args, 1, cx),
            v => v,
        },
        "IFNA" => match arg(args, 0, cx) {
            Val::Err(XlErr::NA) => arg(args, 1, cx),
            v => v,
        },
        "AND" | "OR" | "XOR" => {
            let vals = flatten(args, cx);
            let mut acc = name == "AND";
            let mut x = false;
            let mut any = false;
            for (v, from_ref) in vals {
                let b = match v {
                    Val::Bool(b) => b,
                    Val::Num(n) => n != 0.0,
                    Val::Err(e) => return Val::Err(e),
                    Val::Str(s) if !from_ref => match s.to_ascii_uppercase().as_str() {
                        "TRUE" => true,
                        "FALSE" => false,
                        _ => return Val::Err(XlErr::Value),
                    },
                    _ => continue,
                };
                any = true;
                match name {
                    "AND" => acc &= b,
                    "OR" => acc |= b,
                    _ => x ^= b,
                }
            }
            if !any {
                return Val::Err(XlErr::Value);
            }
            Val::Bool(if name == "XOR" { x } else { acc })
        }
        "NOT" => map1(arg(args, 0, cx), &|v| truth(&v).map(|b| Val::Bool(!b)).unwrap_or_else(Val::Err)),
        "TRUE" => Val::Bool(true),
        "FALSE" => Val::Bool(false),
        "SWITCH" => {
            let v = arg(args, 0, cx).first();
            let mut i = 1;
            while i + 1 < n {
                if compare(&v, &arg(args, i, cx).first()).is_eq() {
                    return arg(args, i + 1, cx);
                }
                i += 2;
            }
            if i < n {
                arg(args, i, cx)
            } else {
                Val::Err(XlErr::NA)
            }
        }
        "CHOOSE" => {
            let k = tryv!(argn(args, 0, cx)) as usize;
            if k < 1 || k >= n {
                return Val::Err(XlErr::Value);
            }
            arg(args, k, cx)
        }
        // ── math
        "SUM" => ok(tryv!(numbers(args, cx, false)).iter().sum()),
        "PRODUCT" => {
            let v = tryv!(numbers(args, cx, false));
            ok(if v.is_empty() { 0.0 } else { v.iter().product() })
        }
        "SUMSQ" => ok(tryv!(numbers(args, cx, false)).iter().map(|x| x * x).sum()),
        "SUMPRODUCT" => {
            let ms: Vec<Vec<Vec<Val>>> = args.iter().map(|a| matrix(eval(a, cx))).collect();
            if ms.is_empty() {
                return Val::Err(XlErr::Value);
            }
            let (r, c) = (ms[0].len(), ms[0].first().map(|x| x.len()).unwrap_or(0));
            if ms.iter().any(|m| m.len() != r || m.first().map(|x| x.len()).unwrap_or(0) != c) {
                return Val::Err(XlErr::Value);
            }
            let mut s = 0.0;
            for i in 0..r {
                for j in 0..c {
                    let mut p = 1.0;
                    for m in &ms {
                        match &m[i][j] {
                            Val::Num(x) => p *= x,
                            Val::Bool(b) => p *= if *b { 1.0 } else { 0.0 },
                            Val::Err(e) => return Val::Err(e.clone()),
                            _ => p = 0.0,
                        }
                    }
                    s += p;
                }
            }
            ok(s)
        }
        "ABS" => map1(arg(args, 0, cx), &|v| num(&v).map(|x| Val::Num(x.abs())).unwrap_or_else(Val::Err)),
        "ROUND" | "ROUNDUP" | "ROUNDDOWN" => {
            let x = tryv!(argn(args, 0, cx));
            let d = tryv!(argn_or(args, 1, cx, 0.0)) as i32;
            ok(round_to(x, d, match name { "ROUND" => 0, "ROUNDUP" => 1, _ => 2 }))
        }
        "INT" => ok(tryv!(argn(args, 0, cx)).floor()),
        "TRUNC" => {
            let x = tryv!(argn(args, 0, cx));
            let d = tryv!(argn_or(args, 1, cx, 0.0)) as i32;
            ok(round_to(x, d, 2))
        }
        "MOD" => {
            let a = tryv!(argn(args, 0, cx));
            let b = tryv!(argn(args, 1, cx));
            if b == 0.0 {
                return Val::Err(XlErr::Div0);
            }
            ok(a - b * (a / b).floor())
        }
        "QUOTIENT" => {
            let a = tryv!(argn(args, 0, cx));
            let b = tryv!(argn(args, 1, cx));
            if b == 0.0 {
                return Val::Err(XlErr::Div0);
            }
            ok((a / b).trunc())
        }
        "POWER" => binop("^", arg(args, 0, cx), arg(args, 1, cx)),
        "SQRT" => {
            let x = tryv!(argn(args, 0, cx));
            if x < 0.0 {
                Val::Err(XlErr::Num)
            } else {
                ok(x.sqrt())
            }
        }
        "EXP" => ok(tryv!(argn(args, 0, cx)).exp()),
        "LN" => {
            let x = tryv!(argn(args, 0, cx));
            if x <= 0.0 { Val::Err(XlErr::Num) } else { ok(x.ln()) }
        }
        "LOG10" => {
            let x = tryv!(argn(args, 0, cx));
            if x <= 0.0 { Val::Err(XlErr::Num) } else { ok(x.log10()) }
        }
        "LOG" => {
            let x = tryv!(argn(args, 0, cx));
            let b = tryv!(argn_or(args, 1, cx, 10.0));
            if x <= 0.0 || b <= 0.0 || b == 1.0 { Val::Err(XlErr::Num) } else { ok(x.ln() / b.ln()) }
        }
        "PI" => Val::Num(std::f64::consts::PI),
        "SIGN" => ok(tryv!(argn(args, 0, cx)).signum() * if tryv!(argn(args, 0, cx)) == 0.0 { 0.0 } else { 1.0 }),
        "SIN" => ok(tryv!(argn(args, 0, cx)).sin()),
        "COS" => ok(tryv!(argn(args, 0, cx)).cos()),
        "TAN" => ok(tryv!(argn(args, 0, cx)).tan()),
        "ASIN" => ok(tryv!(argn(args, 0, cx)).asin()),
        "ACOS" => ok(tryv!(argn(args, 0, cx)).acos()),
        "ATAN" => ok(tryv!(argn(args, 0, cx)).atan()),
        "ATAN2" => {
            let x = tryv!(argn(args, 0, cx));
            let y = tryv!(argn(args, 1, cx));
            if x == 0.0 && y == 0.0 { Val::Err(XlErr::Div0) } else { ok(y.atan2(x)) }
        }
        "DEGREES" => ok(tryv!(argn(args, 0, cx)).to_degrees()),
        "RADIANS" => ok(tryv!(argn(args, 0, cx)).to_radians()),
        "CEILING" | "CEILING.MATH" | "CEILING.PRECISE" | "FLOOR" | "FLOOR.MATH" | "FLOOR.PRECISE" | "MROUND" => {
            let x = tryv!(argn(args, 0, cx));
            let s = tryv!(argn_or(args, 1, cx, if x < 0.0 && !name.contains('.') && name != "MROUND" { -1.0 } else { 1.0 }));
            if s == 0.0 {
                return Val::Num(0.0);
            }
            let up = name.starts_with("CEILING");
            let q = x / s;
            let q = format!("{:.9}", q).parse::<f64>().unwrap_or(q);
            let r = if name == "MROUND" {
                if x * s < 0.0 {
                    return Val::Err(XlErr::Num);
                }
                q.round()
            } else if name.contains('.') {
                let s = s.abs();
                let q = x / s;
                let q = format!("{:.9}", q).parse::<f64>().unwrap_or(q);
                return ok(if up { q.ceil() } else { q.floor() } * s);
            } else if up {
                q.ceil()
            } else {
                q.floor()
            };
            ok(r * s)
        }
        "EVEN" | "ODD" => {
            let x = tryv!(argn(args, 0, cx));
            let a = x.abs().ceil();
            let r = if name == "EVEN" { if a % 2.0 == 0.0 { a } else { a + 1.0 } } else if a % 2.0 == 1.0 { a } else { a + 1.0 };
            ok(r.copysign(x))
        }
        "FACT" => {
            let x = tryv!(argn(args, 0, cx)).floor();
            if x < 0.0 {
                return Val::Err(XlErr::Num);
            }
            ok((1..=(x as u64)).map(|v| v as f64).product())
        }
        "GCD" | "LCM" => {
            let v = tryv!(numbers(args, cx, false));
            fn g(a: u64, b: u64) -> u64 {
                if b == 0 { a } else { g(b, a % b) }
            }
            let mut acc: u64 = if name == "GCD" { 0 } else { 1 };
            for x in v {
                if x < 0.0 {
                    return Val::Err(XlErr::Num);
                }
                let x = x.floor() as u64;
                acc = if name == "GCD" { g(acc, x) } else if x == 0 { 0 } else { acc / g(acc, x).max(1) * x };
            }
            Val::Num(acc as f64)
        }
        "RAND" => Val::Num(pseudo_random(cx)),
        "RANDBETWEEN" => {
            let a = tryv!(argn(args, 0, cx)).ceil();
            let b = tryv!(argn(args, 1, cx)).floor();
            Val::Num(a + (pseudo_random(cx) * (b - a + 1.0)).floor())
        }
        // ── statistics
        "AVERAGE" | "AVERAGEA" => {
            let v = tryv!(numbers(args, cx, name == "AVERAGEA"));
            if v.is_empty() {
                Val::Err(XlErr::Div0)
            } else {
                ok(v.iter().sum::<f64>() / v.len() as f64)
            }
        }
        "MIN" | "MAX" | "MINA" | "MAXA" => {
            let v = tryv!(numbers(args, cx, name.ends_with('A')));
            if v.is_empty() {
                return Val::Num(0.0);
            }
            ok(if name.starts_with("MIN") { v.iter().cloned().fold(f64::MAX, f64::min) } else { v.iter().cloned().fold(f64::MIN, f64::max) })
        }
        "COUNT" => Val::Num(flatten(args, cx).iter().filter(|(v, r)| matches!(v, Val::Num(_)) || (!r && matches!(v, Val::Bool(_))) || (!r && matches!(v, Val::Str(s) if text_to_num(s).is_some()))).count() as f64),
        "COUNTA" => Val::Num(flatten(args, cx).iter().filter(|(v, _)| !matches!(v, Val::Empty)).count() as f64),
        "COUNTBLANK" => Val::Num(flatten(args, cx).iter().filter(|(v, _)| matches!(v, Val::Empty) || matches!(v, Val::Str(s) if s.is_empty())).count() as f64),
        "MEDIAN" => {
            let mut v = tryv!(numbers(args, cx, false));
            if v.is_empty() {
                return Val::Err(XlErr::Num);
            }
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let m = v.len() / 2;
            ok(if v.len() % 2 == 1 { v[m] } else { (v[m - 1] + v[m]) / 2.0 })
        }
        "MODE" | "MODE.SNGL" => {
            let v = tryv!(numbers(args, cx, false));
            let mut best = None;
            let mut best_n = 1;
            for (i, x) in v.iter().enumerate() {
                let c = v.iter().filter(|y| *y == x).count();
                if c > best_n || (c == best_n && best.is_none() && c > 1) {
                    if c > best_n || best.is_none() {
                        best = Some(i);
                        best_n = c;
                    }
                }
            }
            match best {
                Some(i) => Val::Num(v[i]),
                None => Val::Err(XlErr::NA),
            }
        }
        "STDEV" | "STDEV.S" | "STDEVP" | "STDEV.P" | "VAR" | "VAR.S" | "VARP" | "VAR.P" | "STDEVA" | "VARA" => {
            let v = tryv!(numbers(args, cx, name.ends_with('A')));
            let pop = name.ends_with('P') || name.ends_with(".P");
            let n = v.len() as f64;
            if n < if pop { 1.0 } else { 2.0 } {
                return Val::Err(XlErr::Div0);
            }
            let mean = v.iter().sum::<f64>() / n;
            let ss: f64 = v.iter().map(|x| (x - mean).powi(2)).sum();
            let var = ss / if pop { n } else { n - 1.0 };
            ok(if name.starts_with("STDEV") { var.sqrt() } else { var })
        }
        "LARGE" | "SMALL" => {
            let mut v = tryv!(numbers(&args[..1.min(n)], cx, false));
            let k = tryv!(argn(args, 1, cx)) as usize;
            if k < 1 || k > v.len() {
                return Val::Err(XlErr::Num);
            }
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            if name == "LARGE" {
                v.reverse();
            }
            Val::Num(v[k - 1])
        }
        "RANK" | "RANK.EQ" | "RANK.AVG" => {
            let x = tryv!(argn(args, 0, cx));
            let v = tryv!(numbers(&args[1..2.min(n)], cx, false));
            let asc = tryv!(argn_or(args, 2, cx, 0.0)) != 0.0;
            let better = v.iter().filter(|y| if asc { **y < x } else { **y > x }).count();
            let same = v.iter().filter(|y| **y == x).count();
            if same == 0 {
                return Val::Err(XlErr::NA);
            }
            if name == "RANK.AVG" {
                ok(better as f64 + (same as f64 + 1.0) / 2.0)
            } else {
                Val::Num((better + 1) as f64)
            }
        }
        "PERCENTILE" | "PERCENTILE.INC" | "QUARTILE" | "QUARTILE.INC" => {
            let mut v = tryv!(numbers(&args[..1.min(n)], cx, false));
            let mut k = tryv!(argn(args, 1, cx));
            if name.starts_with("QUARTILE") {
                k = k.floor() / 4.0;
            }
            if v.is_empty() || !(0.0..=1.0).contains(&k) {
                return Val::Err(XlErr::Num);
            }
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let pos = k * (v.len() - 1) as f64;
            let lo = pos.floor() as usize;
            let hi = pos.ceil() as usize;
            ok(v[lo] + (v[hi] - v[lo]) * (pos - lo as f64))
        }
        "GEOMEAN" => {
            let v = tryv!(numbers(args, cx, false));
            if v.is_empty() || v.iter().any(|x| *x <= 0.0) {
                return Val::Err(XlErr::Num);
            }
            ok((v.iter().map(|x| x.ln()).sum::<f64>() / v.len() as f64).exp())
        }
        "CORREL" | "PEARSON" => {
            let a = flat(&matrix(arg(args, 0, cx)));
            let b = flat(&matrix(arg(args, 1, cx)));
            let pairs: Vec<(f64, f64)> = a.iter().zip(b.iter()).filter_map(|(x, y)| match (x, y) { (Val::Num(x), Val::Num(y)) => Some((*x, *y)), _ => None }).collect();
            let n = pairs.len() as f64;
            if n < 2.0 {
                return Val::Err(XlErr::Div0);
            }
            let mx = pairs.iter().map(|p| p.0).sum::<f64>() / n;
            let my = pairs.iter().map(|p| p.1).sum::<f64>() / n;
            let sxy: f64 = pairs.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
            let sx: f64 = pairs.iter().map(|p| (p.0 - mx).powi(2)).sum::<f64>().sqrt();
            let sy: f64 = pairs.iter().map(|p| (p.1 - my).powi(2)).sum::<f64>().sqrt();
            if sx == 0.0 || sy == 0.0 { Val::Err(XlErr::Div0) } else { ok(sxy / (sx * sy)) }
        }
        "COUNTIF" | "SUMIF" | "AVERAGEIF" => {
            let range = matrix(arg(args, 0, cx));
            let crit = criterion(&arg(args, 1, cx).first());
            let sum_range = if n > 2 { matrix(arg(args, 2, cx)) } else { range.clone() };
            let mut count = 0.0;
            let mut sum = 0.0;
            for (i, row) in range.iter().enumerate() {
                for (j, v) in row.iter().enumerate() {
                    if crit(v) {
                        count += 1.0;
                        if let Some(Val::Num(x)) = sum_range.get(i).and_then(|r| r.get(j)) {
                            sum += x;
                        } else if name == "AVERAGEIF" {
                            count -= 1.0;
                        }
                    }
                }
            }
            match name {
                "COUNTIF" => Val::Num(count),
                "SUMIF" => ok(sum),
                _ => if count == 0.0 { Val::Err(XlErr::Div0) } else { ok(sum / count) },
            }
        }
        "COUNTIFS" | "SUMIFS" | "AVERAGEIFS" | "MAXIFS" | "MINIFS" => {
            let (target, start) = if name == "COUNTIFS" { (None, 0) } else { (Some(matrix(arg(args, 0, cx))), 1) };
            let mut conds = Vec::new();
            let mut i = start;
            while i + 1 < n {
                conds.push((matrix(arg(args, i, cx)), criterion(&arg(args, i + 1, cx).first())));
                i += 2;
            }
            let shape = target.as_ref().or(conds.first().map(|c| &c.0));
            let Some(shape) = shape else { return Val::Err(XlErr::Value) };
            let mut hits = Vec::new();
            for (r, row) in shape.iter().enumerate() {
                for c in 0..row.len() {
                    if conds.iter().all(|(m, f)| m.get(r).and_then(|x| x.get(c)).map(|v| f(v)).unwrap_or(false)) {
                        hits.push((r, c));
                    }
                }
            }
            if name == "COUNTIFS" {
                return Val::Num(hits.len() as f64);
            }
            let t = target.unwrap();
            let vals: Vec<f64> = hits.iter().filter_map(|(r, c)| match &t[*r][*c] { Val::Num(x) => Some(*x), _ => None }).collect();
            match name {
                "SUMIFS" => ok(vals.iter().sum()),
                "AVERAGEIFS" => if vals.is_empty() { Val::Err(XlErr::Div0) } else { ok(vals.iter().sum::<f64>() / vals.len() as f64) },
                "MAXIFS" => Val::Num(vals.iter().cloned().fold(f64::MIN, f64::max).max(if vals.is_empty() { 0.0 } else { f64::MIN })),
                _ => Val::Num(if vals.is_empty() { 0.0 } else { vals.iter().cloned().fold(f64::MAX, f64::min) }),
            }
        }
        // ── lookup
        "VLOOKUP" | "HLOOKUP" => {
            let key = arg(args, 0, cx).first();
            let mut table = matrix(arg(args, 1, cx));
            if name == "HLOOKUP" {
                table = transpose(&table);
            }
            let idx = tryv!(argn(args, 2, cx)) as usize;
            let approx = if n > 3 { tryv!(truth(&arg(args, 3, cx).first())) } else { true };
            if idx < 1 || table.first().map(|r| idx > r.len()).unwrap_or(true) {
                return Val::Err(if idx < 1 { XlErr::Value } else { XlErr::Ref });
            }
            let col: Vec<Val> = table.iter().map(|r| r[0].clone()).collect();
            match lookup_pos(&key, &col, if approx { 1 } else { 0 }) {
                Some(i) => table[i][idx - 1].clone(),
                None => Val::Err(XlErr::NA),
            }
        }
        "MATCH" | "XMATCH" => {
            let key = arg(args, 0, cx).first();
            let list = flat(&matrix(arg(args, 1, cx)));
            let mode = if name == "MATCH" { tryv!(argn_or(args, 2, cx, 1.0)) as i32 } else { tryv!(argn_or(args, 2, cx, 0.0)) as i32 };
            let pos = if name == "XMATCH" { xlookup_pos(&key, &list, mode, tryv!(argn_or(args, 3, cx, 1.0)) as i32) } else { lookup_pos(&key, &list, mode) };
            match pos {
                Some(i) => Val::Num((i + 1) as f64),
                None => Val::Err(XlErr::NA),
            }
        }
        "XLOOKUP" => {
            let key = arg(args, 0, cx).first();
            let look = matrix(arg(args, 1, cx));
            let ret = matrix(arg(args, 2, cx));
            let vertical = look.len() > 1 || look.first().map(|r| r.len() == 1).unwrap_or(true);
            let list: Vec<Val> = flat(&look);
            let mode = tryv!(argn_or(args, 4, cx, 0.0)) as i32;
            let search = tryv!(argn_or(args, 5, cx, 1.0)) as i32;
            match xlookup_pos(&key, &list, mode, search) {
                Some(i) => {
                    if vertical {
                        match ret.get(i) {
                            Some(row) if row.len() == 1 => row[0].clone(),
                            Some(row) => Val::Arr(vec![row.clone()]),
                            None => Val::Err(XlErr::Ref),
                        }
                    } else {
                        let col: Vec<Vec<Val>> = ret.iter().filter_map(|r| r.get(i).cloned()).map(|v| vec![v]).collect();
                        if col.len() == 1 { col[0][0].clone() } else { Val::Arr(col) }
                    }
                }
                None => {
                    if n > 3 && !matches!(args[3], Expr::Missing) {
                        arg(args, 3, cx)
                    } else {
                        Val::Err(XlErr::NA)
                    }
                }
            }
        }
        "LOOKUP" => {
            let key = arg(args, 0, cx).first();
            let look = matrix(arg(args, 1, cx));
            let (keys, vals): (Vec<Val>, Vec<Val>) = if n > 2 {
                (flat(&look), flat(&matrix(arg(args, 2, cx))))
            } else if look.len() >= look.first().map(|r| r.len()).unwrap_or(0) {
                (look.iter().map(|r| r[0].clone()).collect(), look.iter().map(|r| r.last().unwrap().clone()).collect())
            } else {
                (look[0].clone(), look.last().unwrap().clone())
            };
            match lookup_pos(&key, &keys, 1) {
                Some(i) => vals.get(i).cloned().unwrap_or(Val::Err(XlErr::NA)),
                None => Val::Err(XlErr::NA),
            }
        }
        "INDEX" => {
            let m = matrix(arg(args, 0, cx));
            let r = tryv!(argn_or(args, 1, cx, 0.0)) as usize;
            let c = tryv!(argn_or(args, 2, cx, 0.0)) as usize;
            let rows = m.len();
            let cols = m.first().map(|x| x.len()).unwrap_or(0);
            let (r, c) = if n == 2 && rows == 1 { (1, r) } else { (r, c) };
            match (r, c) {
                (0, 0) => Val::Arr(m),
                (0, c) if c <= cols => Val::Arr(m.iter().map(|row| vec![row[c - 1].clone()]).collect()),
                (r, 0) if r <= rows => if cols == 1 { m[r - 1][0].clone() } else { Val::Arr(vec![m[r - 1].clone()]) },
                (r, c) if r <= rows && c <= cols => m[r - 1][c - 1].clone(),
                _ => Val::Err(XlErr::Ref),
            }
        }
        "ROW" | "COLUMN" => {
            if n == 0 || matches!(args[0], Expr::Missing) {
                let (r, c) = cx.here();
                return Val::Num(if name == "ROW" { r + 1 } else { c + 1 } as f64);
            }
            match ref_of(&args[0], cx) {
                Some(a) => Val::Num(if name == "ROW" { a.rows().0 + 1 } else { a.cols().0 + 1 } as f64),
                None => Val::Err(XlErr::Value),
            }
        }
        "ROWS" | "COLUMNS" => {
            if let Some(a) = args.first().and_then(|e| ref_of(e, cx)) {
                let (a0, a1) = if name == "ROWS" { a.rows() } else { a.cols() };
                return Val::Num((a1 - a0 + 1) as f64);
            }
            let m = matrix(arg(args, 0, cx));
            Val::Num(if name == "ROWS" { m.len() } else { m.first().map(|r| r.len()).unwrap_or(0) } as f64)
        }
        "OFFSET" => match offset_area(args, cx) {
            Some(a) => area_val(&a, cx),
            None => Val::Err(XlErr::Ref),
        },
        "INDIRECT" => {
            let s = tryv!(args_t(args, 0, cx));
            match parse_area(&s) {
                Some(a) => area_val(&a, cx),
                None => match cx.name(&s) {
                    Some(f) => match parse(f.trim_start_matches('=')) {
                        Ok(e) => eval(&e, cx),
                        Err(_) => Val::Err(XlErr::Ref),
                    },
                    None => Val::Err(XlErr::Ref),
                },
            }
        }
        "ADDRESS" => {
            let r = tryv!(argn(args, 0, cx)) as u32;
            let c = tryv!(argn(args, 1, cx)) as u32;
            let abs = tryv!(argn_or(args, 2, cx, 1.0)) as i32;
            if r < 1 || c < 1 {
                return Val::Err(XlErr::Value);
            }
            let (ar, ac) = match abs {
                1 => (true, true),
                2 => (true, false),
                3 => (false, true),
                _ => (false, false),
            };
            let mut s = format!("{}{}{}{}", if ac { "$" } else { "" }, col_name(c - 1), if ar { "$" } else { "" }, r);
            if n > 4 {
                s = format!("{}!{s}", quote_sheet(&tryv!(args_t(args, 4, cx))));
            }
            Val::Str(s)
        }
        "TRANSPOSE" => Val::Arr(transpose(&matrix(arg(args, 0, cx)))),
        // ── text
        "LEN" => map1(arg(args, 0, cx), &|v| text_of(&v).map(|s| Val::Num(s.chars().count() as f64)).unwrap_or_else(Val::Err)),
        "LEFT" | "RIGHT" => {
            let s = tryv!(args_t(args, 0, cx));
            let k = tryv!(argn_or(args, 1, cx, 1.0));
            if k < 0.0 {
                return Val::Err(XlErr::Value);
            }
            let k = k as usize;
            let cs: Vec<char> = s.chars().collect();
            Val::Str(if name == "LEFT" { cs.iter().take(k).collect() } else { cs[cs.len().saturating_sub(k)..].iter().collect() })
        }
        "MID" => {
            let s = tryv!(args_t(args, 0, cx));
            let a = tryv!(argn(args, 1, cx));
            let k = tryv!(argn(args, 2, cx));
            if a < 1.0 || k < 0.0 {
                return Val::Err(XlErr::Value);
            }
            Val::Str(s.chars().skip(a as usize - 1).take(k as usize).collect())
        }
        "UPPER" => Val::Str(tryv!(args_t(args, 0, cx)).to_uppercase()),
        "LOWER" => Val::Str(tryv!(args_t(args, 0, cx)).to_lowercase()),
        "PROPER" => {
            let s = tryv!(args_t(args, 0, cx));
            let mut out = String::new();
            let mut prev_alpha = false;
            for c in s.chars() {
                if c.is_alphabetic() {
                    if prev_alpha {
                        out.extend(c.to_lowercase());
                    } else {
                        out.extend(c.to_uppercase());
                    }
                    prev_alpha = true;
                } else {
                    out.push(c);
                    prev_alpha = false;
                }
            }
            Val::Str(out)
        }
        "TRIM" => Val::Str(tryv!(args_t(args, 0, cx)).split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")),
        "CLEAN" => Val::Str(tryv!(args_t(args, 0, cx)).chars().filter(|c| *c as u32 >= 32).collect()),
        "CONCATENATE" => {
            let mut s = String::new();
            for i in 0..n {
                s.push_str(&tryv!(args_t(args, i, cx)));
            }
            Val::Str(s)
        }
        "CONCAT" => {
            let mut s = String::new();
            for (v, _) in flatten(args, cx) {
                s.push_str(&tryv!(text_of(&v)));
            }
            Val::Str(s)
        }
        "TEXTJOIN" => {
            let d = tryv!(args_t(args, 0, cx));
            let skip = tryv!(truth(&arg(args, 1, cx).first()));
            let mut parts = Vec::new();
            for (v, _) in flatten(&args[2.min(n)..], cx) {
                let t = tryv!(text_of(&v));
                if skip && t.is_empty() {
                    continue;
                }
                parts.push(t);
            }
            Val::Str(parts.join(&d))
        }
        "TEXT" => {
            let v = arg(args, 0, cx).first();
            let f = tryv!(args_t(args, 1, cx));
            match &v {
                Val::Err(e) => Val::Err(e.clone()),
                Val::Str(s) => match text_to_num(s) {
                    Some(x) => Val::Str(numfmt::format_number(x, &f, cx.date1904())),
                    None => Val::Str(numfmt::format_text(s, &f)),
                },
                v => Val::Str(numfmt::format_number(num(v).unwrap_or(0.0), &f, cx.date1904())),
            }
        }
        "VALUE" | "NUMBERVALUE" => {
            let v = arg(args, 0, cx).first();
            match v {
                Val::Num(x) => Val::Num(x),
                Val::Empty => Val::Num(0.0),
                Val::Err(e) => Val::Err(e),
                v => match text_to_num(&v.as_text()) {
                    Some(x) => Val::Num(x),
                    None => Val::Err(XlErr::Value),
                },
            }
        }
        "SUBSTITUTE" => {
            let s = tryv!(args_t(args, 0, cx));
            let old = tryv!(args_t(args, 1, cx));
            let new = tryv!(args_t(args, 2, cx));
            if old.is_empty() {
                return Val::Str(s);
            }
            if n > 3 {
                let k = tryv!(argn(args, 3, cx)) as usize;
                if k < 1 {
                    return Val::Err(XlErr::Value);
                }
                let mut out = String::new();
                let mut rest = s.as_str();
                let mut i = 0;
                while let Some(p) = rest.find(&old) {
                    i += 1;
                    out.push_str(&rest[..p]);
                    out.push_str(if i == k { &new } else { &old });
                    rest = &rest[p + old.len()..];
                }
                out.push_str(rest);
                Val::Str(out)
            } else {
                Val::Str(s.replace(&old, &new))
            }
        }
        "REPLACE" => {
            let s: Vec<char> = tryv!(args_t(args, 0, cx)).chars().collect();
            let a = tryv!(argn(args, 1, cx)) as usize;
            let k = tryv!(argn(args, 2, cx)) as usize;
            let new = tryv!(args_t(args, 3, cx));
            if a < 1 {
                return Val::Err(XlErr::Value);
            }
            let a = (a - 1).min(s.len());
            let b = (a + k).min(s.len());
            Val::Str(format!("{}{}{}", s[..a].iter().collect::<String>(), new, s[b..].iter().collect::<String>()))
        }
        "FIND" | "SEARCH" => {
            let needle = tryv!(args_t(args, 0, cx));
            let hay = tryv!(args_t(args, 1, cx));
            let start = tryv!(argn_or(args, 2, cx, 1.0)) as usize;
            let chars: Vec<char> = hay.chars().collect();
            if start < 1 || start > chars.len() + 1 {
                return Val::Err(XlErr::Value);
            }
            let tail: String = chars[start - 1..].iter().collect();
            let pos = if name == "FIND" {
                tail.find(&needle).map(|b| tail[..b].chars().count())
            } else {
                let re = wildcard(&format!("*{needle}*").to_lowercase());
                let _ = re;
                let low = tail.to_lowercase();
                if needle.contains(['*', '?']) {
                    let pat = wildcard(&needle.to_lowercase());
                    let src = pat.as_str().trim_start_matches("^(?s)").trim_end_matches('$').to_string();
                    regex::Regex::new(&src).ok().and_then(|r| r.find(&low)).map(|m| low[..m.start()].chars().count())
                } else {
                    low.find(&needle.to_lowercase()).map(|b| low[..b].chars().count())
                }
            };
            match pos {
                Some(p) => Val::Num((p + start) as f64),
                None => Val::Err(XlErr::Value),
            }
        }
        "REPT" => {
            let s = tryv!(args_t(args, 0, cx));
            let k = tryv!(argn(args, 1, cx));
            if k < 0.0 {
                return Val::Err(XlErr::Value);
            }
            Val::Str(s.repeat(k as usize))
        }
        "EXACT" => Val::Bool(tryv!(args_t(args, 0, cx)) == tryv!(args_t(args, 1, cx))),
        "CHAR" | "UNICHAR" => {
            let k = tryv!(argn(args, 0, cx)) as u32;
            match char::from_u32(k) {
                Some(c) if k > 0 => Val::Str(c.to_string()),
                _ => Val::Err(XlErr::Value),
            }
        }
        "CODE" | "UNICODE" => match tryv!(args_t(args, 0, cx)).chars().next() {
            Some(c) => Val::Num(c as u32 as f64),
            None => Val::Err(XlErr::Value),
        },
        "T" => match arg(args, 0, cx).first() {
            Val::Str(s) => Val::Str(s),
            Val::Err(e) => Val::Err(e),
            _ => Val::Str(String::new()),
        },
        "N" => match arg(args, 0, cx).first() {
            Val::Num(x) => Val::Num(x),
            Val::Bool(b) => Val::Num(if b { 1.0 } else { 0.0 }),
            Val::Err(e) => Val::Err(e),
            _ => Val::Num(0.0),
        },
        "FIXED" => {
            let x = tryv!(argn(args, 0, cx));
            let d = tryv!(argn_or(args, 1, cx, 2.0)) as i32;
            let no_commas = n > 2 && tryv!(truth(&arg(args, 2, cx).first()));
            let code = format!("{}{}", if no_commas { "0" } else { "#,##0" }, if d > 0 { format!(".{}", "0".repeat(d as usize)) } else { String::new() });
            Val::Str(numfmt::format_number(x, &code, false))
        }
        "TEXTBEFORE" | "TEXTAFTER" => {
            let s = tryv!(args_t(args, 0, cx));
            let d = tryv!(args_t(args, 1, cx));
            let k = tryv!(argn_or(args, 2, cx, 1.0)) as i64;
            let idx: Vec<usize> = s.match_indices(&d).map(|(i, _)| i).collect();
            let pick = if k > 0 { idx.get(k as usize - 1) } else { idx.len().checked_sub((-k) as usize).and_then(|i| idx.get(i)) };
            match pick {
                Some(&i) => Val::Str(if name == "TEXTBEFORE" { s[..i].to_string() } else { s[i + d.len()..].to_string() }),
                None => if n > 5 { arg(args, 5, cx) } else { Val::Err(XlErr::NA) },
            }
        }
        // ── dates
        "DATE" => {
            let y = tryv!(argn(args, 0, cx)) as i64;
            let m = tryv!(argn(args, 1, cx)) as i64;
            let d = tryv!(argn(args, 2, cx)) as i64;
            let y = if y < 1900 { y + 1900 } else { y };
            ok(date_serial(y, m, d))
        }
        "TIME" => {
            let h = tryv!(argn(args, 0, cx));
            let m = tryv!(argn(args, 1, cx));
            let s = tryv!(argn(args, 2, cx));
            ok(((h * 3600.0 + m * 60.0 + s) / 86400.0).rem_euclid(1.0))
        }
        "DATEVALUE" | "TIMEVALUE" => match parse_date_time(&tryv!(args_t(args, 0, cx))) {
            Some(v) => Val::Num(if name == "DATEVALUE" { v.floor() } else { v - v.floor() }),
            None => Val::Err(XlErr::Value),
        },
        "TODAY" => Val::Num(cx.now().floor()),
        "NOW" => Val::Num(cx.now()),
        "YEAR" | "MONTH" | "DAY" => {
            let v = tryv!(argn(args, 0, cx));
            match ymd(v, cx) {
                Some((y, m, d)) => Val::Num(match name { "YEAR" => y as f64, "MONTH" => m as f64, _ => d as f64 }),
                None => Val::Err(XlErr::Num),
            }
        }
        "HOUR" | "MINUTE" | "SECOND" => {
            let v = tryv!(argn(args, 0, cx));
            match numfmt::serial_to_datetime(v, cx.date1904()) {
                Some((_, _, _, h, mi, s, _)) => Val::Num(match name { "HOUR" => h, "MINUTE" => mi, _ => s } as f64),
                None => Val::Err(XlErr::Num),
            }
        }
        "WEEKDAY" => {
            let v = tryv!(argn(args, 0, cx)).floor();
            let t = tryv!(argn_or(args, 1, cx, 1.0)) as i64;
            // Serial 1 (1900-01-01) was a Sunday in Excel's count.
            let dow = ((v as i64 - 1).rem_euclid(7)) as i64; // 0 = Sunday
            Val::Num(match t {
                2 => ((dow + 6) % 7 + 1) as f64,
                3 => ((dow + 6) % 7) as f64,
                _ => (dow + 1) as f64,
            })
        }
        "WEEKNUM" | "ISOWEEKNUM" => {
            let v = tryv!(argn(args, 0, cx)).floor();
            let Some((y, _, _)) = ymd(v, cx) else { return Val::Err(XlErr::Num) };
            if name == "ISOWEEKNUM" {
                let dow = (v as i64 - 2).rem_euclid(7); // 0 = Monday
                let thursday = v as i64 - dow + 3;
                let (ty, _, _) = ymd(thursday as f64, cx).unwrap_or((y, 1, 1));
                let jan1 = date_serial(ty, 1, 1) as i64;
                return Val::Num(((thursday - jan1) / 7 + 1) as f64);
            }
            let jan1 = date_serial(y, 1, 1);
            let dow1 = ((jan1 as i64 - 1).rem_euclid(7)) as f64;
            Val::Num(((v - jan1 + dow1) / 7.0).floor() + 1.0)
        }
        "EDATE" | "EOMONTH" => {
            let v = tryv!(argn(args, 0, cx));
            let k = tryv!(argn(args, 1, cx)) as i64;
            let Some((y, m, d)) = ymd(v, cx) else { return Val::Err(XlErr::Num) };
            let total = y * 12 + (m as i64 - 1) + k;
            let (ny, nm) = (total.div_euclid(12), (total.rem_euclid(12) + 1) as u32);
            let dim = days_in_month(ny, nm);
            let nd = if name == "EOMONTH" { dim } else { d.min(dim) };
            ok(numfmt::datetime_to_serial(ny, nm, nd, 0, 0, 0.0))
        }
        "DAYS" => ok(tryv!(argn(args, 0, cx)).floor() - tryv!(argn(args, 1, cx)).floor()),
        "DATEDIF" => {
            let a = tryv!(argn(args, 0, cx)).floor();
            let b = tryv!(argn(args, 1, cx)).floor();
            let unit = tryv!(args_t(args, 2, cx)).to_ascii_uppercase();
            if a > b {
                return Val::Err(XlErr::Num);
            }
            let (Some((y1, m1, d1)), Some((y2, m2, d2))) = (ymd(a, cx), ymd(b, cx)) else { return Val::Err(XlErr::Num) };
            let months = (y2 - y1) * 12 + m2 as i64 - m1 as i64 - if d2 < d1 { 1 } else { 0 };
            Val::Num(match unit.as_str() {
                "D" => b - a,
                "M" => months as f64,
                "Y" => (months / 12) as f64,
                "YM" => (months % 12) as f64,
                "MD" => {
                    if d2 >= d1 {
                        (d2 - d1) as f64
                    } else {
                        let (py, pm) = if m2 == 1 { (y2 - 1, 12) } else { (y2, m2 - 1) };
                        (days_in_month(py, pm) - d1 + d2) as f64
                    }
                }
                "YD" => {
                    let anniv = date_serial(y2, m1 as i64, d1 as i64);
                    let anniv = if anniv > b { date_serial(y2 - 1, m1 as i64, d1 as i64) } else { anniv };
                    b - anniv
                }
                _ => return Val::Err(XlErr::Num),
            })
        }
        "NETWORKDAYS" | "WORKDAY" => {
            let a = tryv!(argn(args, 0, cx)).floor();
            let holidays: Vec<f64> = if n > 2 { tryv!(numbers(&args[2..3], cx, false)).into_iter().map(|x| x.floor()).collect() } else { vec![] };
            let workday = |s: f64| -> bool {
                let dow = (s as i64 - 1).rem_euclid(7);
                dow != 0 && dow != 6 && !holidays.contains(&s)
            };
            if name == "NETWORKDAYS" {
                let b = tryv!(argn(args, 1, cx)).floor();
                let (lo, hi, sign) = if a <= b { (a, b, 1.0) } else { (b, a, -1.0) };
                let mut c = 0.0;
                let mut s = lo;
                while s <= hi {
                    if workday(s) {
                        c += 1.0;
                    }
                    s += 1.0;
                }
                Val::Num(c * sign)
            } else {
                let mut k = tryv!(argn(args, 1, cx)) as i64;
                let step = if k >= 0 { 1.0 } else { -1.0 };
                let mut s = a;
                while k != 0 {
                    s += step;
                    if workday(s) {
                        k -= step as i64;
                    }
                }
                Val::Num(s)
            }
        }
        "YEARFRAC" => {
            let a = tryv!(argn(args, 0, cx)).floor();
            let b = tryv!(argn(args, 1, cx)).floor();
            let basis = tryv!(argn_or(args, 2, cx, 0.0)) as i32;
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let (Some((y1, m1, mut d1)), Some((y2, m2, mut d2))) = (ymd(lo, cx), ymd(hi, cx)) else { return Val::Err(XlErr::Num) };
            ok(match basis {
                1 => (hi - lo) / 365.25,
                2 => (hi - lo) / 360.0,
                3 => (hi - lo) / 365.0,
                _ => {
                    if d1 == 31 {
                        d1 = 30;
                    }
                    if d2 == 31 && d1 >= 30 {
                        d2 = 30;
                    }
                    ((y2 - y1) as f64 * 360.0 + (m2 as f64 - m1 as f64) * 30.0 + (d2 as f64 - d1 as f64)) / 360.0
                }
            })
        }
        // ── information
        "ISBLANK" => Val::Bool(matches!(arg(args, 0, cx).first(), Val::Empty)),
        "ISNUMBER" => map1(arg(args, 0, cx), &|v| Val::Bool(matches!(v, Val::Num(_)))),
        "ISTEXT" => map1(arg(args, 0, cx), &|v| Val::Bool(matches!(v, Val::Str(_)))),
        "ISNONTEXT" => Val::Bool(!matches!(arg(args, 0, cx).first(), Val::Str(_))),
        "ISLOGICAL" => Val::Bool(matches!(arg(args, 0, cx).first(), Val::Bool(_))),
        "ISERROR" => map1(arg(args, 0, cx), &|v| Val::Bool(matches!(v, Val::Err(_)))),
        "ISERR" => Val::Bool(matches!(arg(args, 0, cx).first(), Val::Err(e) if e != XlErr::NA)),
        "ISNA" => Val::Bool(matches!(arg(args, 0, cx).first(), Val::Err(XlErr::NA))),
        "ISEVEN" | "ISODD" => {
            let x = tryv!(argn(args, 0, cx)).trunc().abs();
            Val::Bool((x % 2.0 == 0.0) == (name == "ISEVEN"))
        }
        "ISREF" => Val::Bool(args.first().map(|a| ref_of(a, cx).is_some()).unwrap_or(false)),
        "ISFORMULA" => Val::Bool(false),
        "NA" => Val::Err(XlErr::NA),
        "TYPE" => Val::Num(match arg(args, 0, cx) {
            Val::Num(_) | Val::Empty => 1.0,
            Val::Str(_) => 2.0,
            Val::Bool(_) => 4.0,
            Val::Err(_) => 16.0,
            Val::Arr(_) => 64.0,
        }),
        "ERROR.TYPE" => match arg(args, 0, cx).first() {
            Val::Err(e) => Val::Num(match e {
                XlErr::Null => 1.0,
                XlErr::Div0 => 2.0,
                XlErr::Value => 3.0,
                XlErr::Ref => 4.0,
                XlErr::Name => 5.0,
                XlErr::Num => 6.0,
                XlErr::NA => 7.0,
                _ => 8.0,
            }),
            _ => Val::Err(XlErr::NA),
        },
        // ── finance
        "PMT" | "IPMT" | "PPMT" | "PV" | "FV" | "NPER" => {
            let rate = tryv!(argn(args, 0, cx));
            fn pmt(r: f64, n: f64, pv: f64, fv: f64, t: f64) -> f64 {
                if r == 0.0 {
                    -(pv + fv) / n
                } else {
                    -(r * (fv + pv * (1.0 + r).powf(n))) / ((1.0 + r * t) * ((1.0 + r).powf(n) - 1.0))
                }
            }
            fn fvf(r: f64, n: f64, p: f64, pv: f64, t: f64) -> f64 {
                if r == 0.0 {
                    -(pv + p * n)
                } else {
                    -(pv * (1.0 + r).powf(n) + p * (1.0 + r * t) * ((1.0 + r).powf(n) - 1.0) / r)
                }
            }
            match name {
                "PMT" => {
                    let nper = tryv!(argn(args, 1, cx));
                    let pv = tryv!(argn(args, 2, cx));
                    let fv = tryv!(argn_or(args, 3, cx, 0.0));
                    let t = tryv!(argn_or(args, 4, cx, 0.0));
                    ok(pmt(rate, nper, pv, fv, t))
                }
                "IPMT" | "PPMT" => {
                    let per = tryv!(argn(args, 1, cx));
                    let nper = tryv!(argn(args, 2, cx));
                    let pv = tryv!(argn(args, 3, cx));
                    let fv = tryv!(argn_or(args, 4, cx, 0.0));
                    let t = tryv!(argn_or(args, 5, cx, 0.0));
                    let p = pmt(rate, nper, pv, fv, t);
                    // As LibreOffice's ScGetIpmt.
                    let ip = if per == 1.0 {
                        if t == 1.0 { 0.0 } else { -pv }
                    } else if t == 1.0 {
                        fvf(rate, per - 2.0, p, pv, 1.0) - p
                    } else {
                        fvf(rate, per - 1.0, p, pv, 0.0)
                    } * rate;
                    ok(if name == "IPMT" { ip } else { p - ip })
                }
                "PV" => {
                    let nper = tryv!(argn(args, 1, cx));
                    let p = tryv!(argn(args, 2, cx));
                    let fv = tryv!(argn_or(args, 3, cx, 0.0));
                    let t = tryv!(argn_or(args, 4, cx, 0.0));
                    ok(if rate == 0.0 { -(fv + p * nper) } else { -(fv + p * (1.0 + rate * t) * ((1.0 + rate).powf(nper) - 1.0) / rate) / (1.0 + rate).powf(nper) })
                }
                "FV" => {
                    let nper = tryv!(argn(args, 1, cx));
                    let p = tryv!(argn(args, 2, cx));
                    let pv = tryv!(argn_or(args, 3, cx, 0.0));
                    let t = tryv!(argn_or(args, 4, cx, 0.0));
                    ok(fvf(rate, nper, p, pv, t))
                }
                _ => {
                    let p = tryv!(argn(args, 1, cx));
                    let pv = tryv!(argn(args, 2, cx));
                    let fv = tryv!(argn_or(args, 3, cx, 0.0));
                    let t = tryv!(argn_or(args, 4, cx, 0.0));
                    if rate == 0.0 {
                        if p == 0.0 { return Val::Err(XlErr::Num) }
                        ok(-(pv + fv) / p)
                    } else {
                        let z = p * (1.0 + rate * t) / rate;
                        ok(((-fv + z) / (pv + z)).ln() / (1.0 + rate).ln())
                    }
                }
            }
        }
        "NPV" => {
            let rate = tryv!(argn(args, 0, cx));
            let v = tryv!(numbers(&args[1..], cx, false));
            ok(v.iter().enumerate().map(|(i, x)| x / (1.0 + rate).powi(i as i32 + 1)).sum())
        }
        "IRR" => {
            let v = tryv!(numbers(&args[..1.min(n)], cx, false));
            let mut r = tryv!(argn_or(args, 1, cx, 0.1));
            for _ in 0..100 {
                let f: f64 = v.iter().enumerate().map(|(i, x)| x / (1.0 + r).powi(i as i32)).sum();
                let df: f64 = v.iter().enumerate().map(|(i, x)| -(i as f64) * x / (1.0 + r).powi(i as i32 + 1)).sum();
                if df == 0.0 {
                    break;
                }
                let nr = r - f / df;
                if (nr - r).abs() < 1e-10 {
                    return ok(nr);
                }
                r = nr;
            }
            Val::Err(XlErr::Num)
        }
        "RATE" => {
            let nper = tryv!(argn(args, 0, cx));
            let p = tryv!(argn(args, 1, cx));
            let pv = tryv!(argn(args, 2, cx));
            let fv = tryv!(argn_or(args, 3, cx, 0.0));
            let t = tryv!(argn_or(args, 4, cx, 0.0));
            let mut r = tryv!(argn_or(args, 5, cx, 0.1));
            let f = |r: f64| -> f64 {
                if r.abs() < 1e-12 {
                    pv + p * nper + fv
                } else {
                    pv * (1.0 + r).powf(nper) + p * (1.0 + r * t) * ((1.0 + r).powf(nper) - 1.0) / r + fv
                }
            };
            for _ in 0..100 {
                let y = f(r);
                let d = (f(r + 1e-7) - y) / 1e-7;
                if d == 0.0 {
                    break;
                }
                let nr = r - y / d;
                if (nr - r).abs() < 1e-10 {
                    return ok(nr);
                }
                r = nr;
            }
            Val::Err(XlErr::Num)
        }
        // ── dynamic arrays (results as arrays; a cell shows the first value)
        "UNIQUE" => {
            let m = matrix(arg(args, 0, cx));
            let mut out: Vec<Vec<Val>> = Vec::new();
            for r in m {
                if !out.iter().any(|o| o.len() == r.len() && o.iter().zip(&r).all(|(a, b)| compare(a, b).is_eq())) {
                    out.push(r);
                }
            }
            Val::Arr(out)
        }
        "SORT" => {
            let mut m = matrix(arg(args, 0, cx));
            let idx = tryv!(argn_or(args, 1, cx, 1.0)) as usize;
            let desc = tryv!(argn_or(args, 2, cx, 1.0)) < 0.0;
            m.sort_by(|a, b| {
                let o = compare(a.get(idx - 1).unwrap_or(&Val::Empty), b.get(idx - 1).unwrap_or(&Val::Empty));
                if desc { o.reverse() } else { o }
            });
            Val::Arr(m)
        }
        "FILTER" => {
            let m = matrix(arg(args, 0, cx));
            let inc = flat(&matrix(arg(args, 1, cx)));
            let out: Vec<Vec<Val>> = m.into_iter().zip(inc).filter(|(_, k)| truth(k).unwrap_or(false)).map(|(r, _)| r).collect();
            if out.is_empty() {
                if n > 2 { arg(args, 2, cx) } else { Val::Err(XlErr::Calc) }
            } else {
                Val::Arr(out)
            }
        }
        "SEQUENCE" => {
            let r = tryv!(argn(args, 0, cx)) as usize;
            let c = tryv!(argn_or(args, 1, cx, 1.0)) as usize;
            let s = tryv!(argn_or(args, 2, cx, 1.0));
            let st = tryv!(argn_or(args, 3, cx, 1.0));
            Val::Arr((0..r).map(|i| (0..c).map(|j| Val::Num(s + st * (i * c + j) as f64)).collect()).collect())
        }
        _ => Val::Err(XlErr::Name),
    }
}

fn transpose(m: &[Vec<Val>]) -> Vec<Vec<Val>> {
    let cols = m.iter().map(|r| r.len()).max().unwrap_or(0);
    (0..cols).map(|c| m.iter().map(|r| r.get(c).cloned().unwrap_or(Val::Empty)).collect()).collect()
}

/// MATCH semantics: 0 exact (wildcards in text), 1 largest ≤ key (sorted ascending), -1
/// smallest ≥ key (sorted descending).
fn lookup_pos(key: &Val, list: &[Val], mode: i32) -> Option<usize> {
    if mode == 0 {
        if let Val::Str(s) = key {
            if s.contains(['*', '?', '~']) {
                let re = wildcard(&s.to_lowercase());
                return list.iter().position(|v| matches!(v, Val::Str(t) if re.is_match(&t.to_lowercase())));
            }
        }
        return list.iter().position(|v| !matches!(v, Val::Empty) && same_type(v, key) && compare(v, key).is_eq());
    }
    let mut best = None;
    for (i, v) in list.iter().enumerate() {
        if matches!(v, Val::Empty) || !same_type(v, key) {
            continue;
        }
        let o = compare(v, key);
        if mode > 0 {
            if o.is_le() {
                best = Some(i);
            } else {
                break;
            }
        } else if o.is_ge() {
            best = Some(i);
        } else {
            break;
        }
    }
    best
}

fn same_type(a: &Val, b: &Val) -> bool {
    matches!((a, b), (Val::Num(_), Val::Num(_)) | (Val::Str(_), Val::Str(_)) | (Val::Bool(_), Val::Bool(_)))
}

/// XLOOKUP/XMATCH: match mode 0 exact, -1 exact or next smaller, 1 exact or next larger, 2
/// wildcards; search mode 1 first to last, -1 last to first.
fn xlookup_pos(key: &Val, list: &[Val], mode: i32, search: i32) -> Option<usize> {
    let order: Vec<usize> = if search < 0 { (0..list.len()).rev().collect() } else { (0..list.len()).collect() };
    if mode == 2 {
        let pat = wildcard(&key.as_text().to_lowercase());
        return order.into_iter().find(|&i| matches!(&list[i], Val::Str(t) if pat.is_match(&t.to_lowercase())));
    }
    if let Some(i) = order.iter().copied().find(|&i| same_type(&list[i], key) && compare(&list[i], key).is_eq()) {
        return Some(i);
    }
    if mode == 0 {
        return None;
    }
    let mut best: Option<usize> = None;
    for &i in &order {
        let v = &list[i];
        if !same_type(v, key) {
            continue;
        }
        let o = compare(v, key);
        let better = |b: usize| if mode < 0 { compare(v, &list[b]).is_gt() } else { compare(v, &list[b]).is_lt() };
        if (mode < 0 && o.is_lt()) || (mode > 0 && o.is_gt()) {
            if best.map(better).unwrap_or(true) {
                best = Some(i);
            }
        }
    }
    best
}

fn pseudo_random(cx: &dyn Ctx) -> f64 {
    // Deterministic per cell and day: recalculation gives the same file each time.
    let (r, c) = cx.here();
    let mut x = (r as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (c as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ (cx.now().floor() as u64);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct T(HashMap<(u32, u32), Val>);
    impl Ctx for T {
        fn cell(&mut self, _s: Option<&str>, r: u32, c: u32) -> Val {
            self.0.get(&(r, c)).cloned().unwrap_or(Val::Empty)
        }
        fn extent(&mut self, _s: Option<&str>) -> (u32, u32) {
            (20, 10)
        }
        fn name(&mut self, n: &str) -> Option<String> {
            (n == "Rate").then(|| "0.1".to_string())
        }
        fn sheet_exists(&mut self, _s: &str) -> bool {
            true
        }
        fn here(&self) -> (u32, u32) {
            (0, 0)
        }
        fn now(&self) -> f64 {
            45306.5
        }
    }

    fn ev(f: &str) -> Val {
        let mut m = HashMap::new();
        for (i, v) in [10.0, 20.0, 30.0].iter().enumerate() {
            m.insert((i as u32, 0), Val::Num(*v));
        }
        m.insert((0, 1), Val::Str("apple".into()));
        m.insert((1, 1), Val::Str("banana".into()));
        m.insert((2, 1), Val::Str("cherry".into()));
        let mut t = T(m);
        eval(&parse(f).unwrap(), &mut t).first()
    }

    #[test]
    fn evaluates() {
        assert_eq!(ev("1+2*3"), Val::Num(7.0));
        assert_eq!(ev("-2^2"), Val::Num(4.0));
        assert_eq!(ev("SUM(A1:A3)"), Val::Num(60.0));
        assert_eq!(ev("AVERAGE(A1:A3)*Rate"), Val::Num(2.0));
        assert_eq!(ev("IF(A1>5,\"big\",\"small\")"), Val::Str("big".into()));
        assert_eq!(ev("VLOOKUP(20,A1:B3,2,FALSE)"), Val::Str("banana".into()));
        assert_eq!(ev("INDEX(B1:B3,MATCH(30,A1:A3,0))"), Val::Str("cherry".into()));
        assert_eq!(ev("_xlfn.XLOOKUP(\"cherry\",B1:B3,A1:A3)"), Val::Num(30.0));
        assert_eq!(ev("COUNTIF(A1:A3,\">15\")"), Val::Num(2.0));
        assert_eq!(ev("SUMIFS(A1:A3,B1:B3,\"*an*\")"), Val::Num(20.0));
        assert_eq!(ev("TEXT(1234.5,\"#,##0.00\")"), Val::Str("1,234.50".into()));
        assert_eq!(ev("\"a\"&1.5&TRUE"), Val::Str("a1.5TRUE".into()));
        assert_eq!(ev("1/0"), Val::Err(XlErr::Div0));
        assert_eq!(ev("ROUND(2.345,2)"), Val::Num(2.35));
        assert_eq!(ev("DATE(2024,1,15)"), Val::Num(45306.0));
        assert_eq!(ev("EOMONTH(DATE(2024,1,15),1)"), Val::Num(45351.0));
        assert_eq!(ev("SUMPRODUCT(A1:A3,A1:A3)"), Val::Num(1400.0));
        assert_eq!(ev("SUM(A1:A3*2)"), Val::Num(120.0));
        assert_eq!(ev("LEFT(B2,3)&UPPER(MID(B3,2,2))"), Val::Str("banHE".into()));
        assert_eq!(ev("ROUND(PMT(0.05/12,360,200000),2)"), Val::Num(-1073.64));
        assert_eq!(ev("NOSUCHFN(1)"), Val::Err(XlErr::Name));
    }

    #[test]
    fn rewrites() {
        assert_eq!(shift("A1+$B$2+Sheet2!C3", 1, 2), "C2+$B$2+Sheet2!E4");
        assert_eq!(shift("SUM(A:A)", 0, 1), "SUM(B:B)");
        assert_eq!(to_file("=XLOOKUP(A1,B:B,C:C)").unwrap(), "_xlfn.XLOOKUP(A1,B:B,C:C)");
        assert_eq!(to_file("=SUM(A1)").unwrap(), "SUM(A1)");
        assert_eq!(for_display("_xlfn.XLOOKUP(A1,B:B,C:C)"), "=XLOOKUP(A1,B:B,C:C)");
        assert_eq!(parse_area("'My Sheet'!B2:C3").unwrap().sheet.as_deref(), Some("My Sheet"));
        assert_eq!(quote_sheet("My Sheet"), "'My Sheet'");
        assert_eq!(quote_sheet("Data"), "Data");
    }
}
