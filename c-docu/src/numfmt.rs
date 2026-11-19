//! Excel number formats: a value and a format code (`#,##0.00`, `0%`, `yyyy-mm-dd`,
//! `[Red]-0.0;0.0`, `"₩"#,##0`) give the text a cell shows.

/// The format codes of the built-in format ids (ECMA-376 18.8.30, with the CJK ones Excel uses
/// in Korean, Japanese and Chinese locales mapped to their usual codes).
pub fn builtin(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "\"$\"#,##0_);(\"$\"#,##0)",
        6 => "\"$\"#,##0_);[Red](\"$\"#,##0)",
        7 => "\"$\"#,##0.00_);(\"$\"#,##0.00)",
        8 => "\"$\"#,##0.00_);[Red](\"$\"#,##0.00)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "yyyy-mm-dd",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "yyyy-mm-dd h:mm",
        27..=31 => "yyyy\"년\" m\"월\" d\"일\"",
        32..=35 => "h\"시\" mm\"분\"",
        36 => "yyyy-mm-dd",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        41 => "_(* #,##0_);_(* (#,##0);_(* \"-\"_);_(@_)",
        42 => "_(\"$\"* #,##0_);_(\"$\"* (#,##0);_(\"$\"* \"-\"_);_(@_)",
        43 => "_(* #,##0.00_);_(* (#,##0.00);_(* \"-\"??_);_(@_)",
        44 => "_(\"$\"* #,##0.00_);_(\"$\"* (#,##0.00);_(\"$\"* \"-\"??_);_(@_)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        50..=58 => "yyyy-mm-dd",
        _ => return None,
    })
}

/// Whether a format shows dates or times (so a number in it is a serial date).
pub fn is_date(code: &str) -> bool {
    let mut in_quote = false;
    let mut prev = ' ';
    let mut bracket = false;
    for c in code.chars() {
        if in_quote {
            if c == '"' {
                in_quote = false;
            }
            continue;
        }
        match c {
            '"' => in_quote = true,
            '[' => bracket = true,
            ']' => bracket = false,
            _ if bracket => {
                if matches!(c.to_ascii_lowercase(), 'h' | 'm' | 's') && prev == '[' {
                    return true;
                }
            }
            '\\' | '_' | '*' => {}
            c if prev != '\\' && prev != '_' && prev != '*' && matches!(c.to_ascii_lowercase(), 'y' | 'd' | 'h' | 's') => return true,
            c if prev != '\\' && matches!(c, 'm' | 'M') => {
                // m alone is month or minute: a date either way.
                return true;
            }
            _ => {}
        }
        prev = c;
    }
    false
}

/// (year, month, day, hour, minute, second, fraction) of an Excel serial date.
pub fn serial_to_datetime(v: f64, date1904: bool) -> Option<(i64, u32, u32, u32, u32, u32, f64)> {
    if !(0.0..2958466.0).contains(&v) {
        return None;
    }
    let mut days = v.floor() as i64;
    let frac = v - v.floor();
    let mut secs_f = frac * 86400.0;
    // Rounding to the millisecond as Excel shows times.
    secs_f = (secs_f * 1000.0).round() / 1000.0;
    let mut secs = secs_f.floor() as i64;
    let sub = secs_f - secs as f64;
    if secs >= 86400 {
        secs -= 86400;
        days += 1;
    }
    let (y, m, d) = if date1904 {
        civil_from_days(days - 24107) // 1904-01-01
    } else if days == 60 {
        (1900, 2, 29) // Excel's fictitious leap day
    } else if days == 0 {
        (1900, 1, 0)
    } else {
        let d = if days < 60 { days } else { days - 1 };
        civil_from_days(d - 25568) // 1899-12-31 + d
    };
    Some((y, m, d, (secs / 3600) as u32, ((secs / 60) % 60) as u32, (secs % 60) as u32, sub))
}

/// Days since 1970-01-01 → (y, m, d).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

/// The serial number of a date (1900 system).
pub fn datetime_to_serial(y: i64, m: u32, d: u32, h: u32, mi: u32, s: f64) -> f64 {
    let days = days_from_civil(y, m, d) + 25568;
    let serial = if days >= 60 { days + 1 } else { days };
    serial as f64 + (h as f64 * 3600.0 + mi as f64 * 60.0 + s) / 86400.0
}

#[derive(Debug, Clone)]
enum Tok {
    Lit(String),
    Digit(char), // 0 # ?
    Point,
    Comma, // thousands separator (or scaling when trailing)
    Percent,
    Exp(String), // E+ E- e+ e-
    Date(String), // yyyy, mm, d, h, ss, AM/PM, [h], ...
    Text,         // @
    Fill(char),   // *x
    Skip(char),   // _x: a space as wide as x
    Slash,
}

struct Section {
    toks: Vec<Tok>,
    color: Option<String>,
    cond: Option<(String, f64)>,
}

fn tokenize(code: &str) -> Section {
    let mut toks = Vec::new();
    let mut color = None;
    let mut cond = None;
    let cs: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        match c {
            '"' => {
                let mut s = String::new();
                i += 1;
                while i < cs.len() && cs[i] != '"' {
                    s.push(cs[i]);
                    i += 1;
                }
                toks.push(Tok::Lit(s));
            }
            '\\' => {
                if i + 1 < cs.len() {
                    toks.push(Tok::Lit(cs[i + 1].to_string()));
                    i += 1;
                }
            }
            '_' => {
                if i + 1 < cs.len() {
                    toks.push(Tok::Skip(cs[i + 1]));
                    i += 1;
                }
            }
            '*' => {
                if i + 1 < cs.len() {
                    toks.push(Tok::Fill(cs[i + 1]));
                    i += 1;
                }
            }
            '[' => {
                let mut s = String::new();
                i += 1;
                while i < cs.len() && cs[i] != ']' {
                    s.push(cs[i]);
                    i += 1;
                }
                let low = s.to_ascii_lowercase();
                if low.starts_with('$') {
                    // [$€-407]: a currency symbol and locale.
                    let sym = s[1..].split('-').next().unwrap_or("").to_string();
                    if !sym.is_empty() {
                        toks.push(Tok::Lit(sym));
                    }
                } else if matches!(low.as_str(), "h" | "hh" | "m" | "mm" | "s" | "ss") || low.starts_with("h") && low.chars().all(|c| c == 'h') || low.chars().all(|c| c == 'm') || low.chars().all(|c| c == 's') {
                    toks.push(Tok::Date(format!("[{low}]")));
                } else if let Some(rest) = low.strip_prefix(">=").map(|r| (">=", r)).or_else(|| low.strip_prefix("<=").map(|r| ("<=", r))).or_else(|| low.strip_prefix("<>").map(|r| ("<>", r))).or_else(|| low.strip_prefix('>').map(|r| (">", r))).or_else(|| low.strip_prefix('<').map(|r| ("<", r))).or_else(|| low.strip_prefix('=').map(|r| ("=", r))) {
                    if let Ok(v) = rest.1.trim().parse::<f64>() {
                        cond = Some((rest.0.to_string(), v));
                    }
                } else if !low.starts_with("dbnum") && !low.starts_with("natnum") {
                    color = Some(s);
                }
            }
            '0' | '#' | '?' => toks.push(Tok::Digit(c)),
            '.' => toks.push(Tok::Point),
            ',' => toks.push(Tok::Comma),
            '%' => toks.push(Tok::Percent),
            '@' => toks.push(Tok::Text),
            '/' => toks.push(Tok::Slash),
            'E' | 'e' if i + 1 < cs.len() && (cs[i + 1] == '+' || cs[i + 1] == '-') => {
                toks.push(Tok::Exp(format!("{}{}", c, cs[i + 1])));
                i += 1;
            }
            _ => {
                let low = c.to_ascii_lowercase();
                if matches!(low, 'y' | 'm' | 'd' | 'h' | 's') {
                    let mut s = String::new();
                    while i < cs.len() && cs[i].to_ascii_lowercase() == low {
                        s.push(low);
                        i += 1;
                    }
                    i -= 1;
                    toks.push(Tok::Date(s));
                } else if (c == 'A' || c == 'a') && cs[i..].iter().collect::<String>().to_ascii_uppercase().starts_with("AM/PM") {
                    toks.push(Tok::Date("AM/PM".into()));
                    i += 4;
                } else if (c == 'A' || c == 'a') && cs[i..].iter().collect::<String>().to_ascii_uppercase().starts_with("A/P") {
                    toks.push(Tok::Date("A/P".into()));
                    i += 2;
                } else if c == '오' && cs[i..].iter().collect::<String>().starts_with("오전/오후") {
                    toks.push(Tok::Date("오전/오후".into()));
                    i += 4;
                } else {
                    toks.push(Tok::Lit(c.to_string()));
                }
            }
        }
        i += 1;
    }
    Section { toks, color, cond }
}

fn split_sections(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    let mut esc = false;
    let mut br = false;
    for c in code.chars() {
        if esc {
            cur.push(c);
            esc = false;
            continue;
        }
        match c {
            '\\' if !q => {
                esc = true;
                cur.push(c);
            }
            '"' => {
                q = !q;
                cur.push(c);
            }
            '[' if !q => {
                br = true;
                cur.push(c);
            }
            ']' if !q => {
                br = false;
                cur.push(c);
            }
            ';' if !q && !br => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

pub fn format_text(s: &str, code: &str) -> String {
    let secs = split_sections(code);
    let sec = if secs.len() >= 4 { &secs[3] } else if secs.len() == 1 && secs[0].contains('@') { &secs[0] } else { return s.to_string() };
    let t = tokenize(sec);
    let mut out = String::new();
    for tok in t.toks {
        match tok {
            Tok::Lit(l) => out.push_str(&l),
            Tok::Text => out.push_str(s),
            Tok::Skip(_) => out.push(' '),
            _ => {}
        }
    }
    out
}

/// The text Excel shows for a number in a format.
pub fn format_number(v: f64, code: &str, date1904: bool) -> String {
    if code.is_empty() || code.eq_ignore_ascii_case("general") {
        return general(v);
    }
    let secs = split_sections(code);
    let parsed: Vec<Section> = secs.iter().map(|s| tokenize(s)).collect();
    // Which section: conditions, else positive;negative;zero.
    let has_cond = parsed.iter().any(|s| s.cond.is_some());
    let (idx, use_abs) = if has_cond {
        let mut pick = None;
        for (i, s) in parsed.iter().enumerate() {
            if let Some((op, x)) = &s.cond {
                let ok = match op.as_str() {
                    ">" => v > *x,
                    ">=" => v >= *x,
                    "<" => v < *x,
                    "<=" => v <= *x,
                    "=" => v == *x,
                    "<>" => v != *x,
                    _ => false,
                };
                if ok {
                    pick = Some(i);
                    break;
                }
            }
        }
        match pick {
            Some(i) => (i, false),
            None => (parsed.iter().position(|s| s.cond.is_none()).unwrap_or(0), false),
        }
    } else if v < 0.0 && parsed.len() >= 2 {
        (1, true)
    } else if v == 0.0 && parsed.len() >= 3 {
        (2, true)
    } else {
        (0, false)
    };
    let sec = &parsed[idx.min(parsed.len() - 1)];
    let general_sec = secs[idx.min(secs.len() - 1)].trim().eq_ignore_ascii_case("general");
    if general_sec {
        return general(if use_abs { v.abs() } else { v });
    }
    let val = if use_abs { v.abs() } else { v };
    let is_dt = sec.toks.iter().any(|t| matches!(t, Tok::Date(_)));
    if is_dt {
        return format_date(val, sec, date1904);
    }
    let s = format_numeric(val, sec);
    // A negative number in a one-section format gets its minus sign.
    if v < 0.0 && !use_abs && !has_cond && !s.starts_with('-') {
        format!("-{}", s)
    } else {
        s
    }
}

/// `x` (not negative) with `n` decimals, rounded half away from zero on its 15 significant
/// digits, as Excel rounds.
pub fn fixed(x: f64, n: usize) -> String {
    if !x.is_finite() {
        return "#NUM!".into();
    }
    let x = x.abs();
    if x == 0.0 {
        return if n == 0 { "0".into() } else { format!("0.{}", "0".repeat(n)) };
    }
    // 15 significant digits.
    let e = format!("{:.14e}", x);
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: Vec<u8> = mant.bytes().filter(|b| b.is_ascii_digit()).map(|b| b - b'0').collect();
    // The value is 0.d1d2...d15 x 10^(exp+1). Keep digits up to position (exp + 1 + n).
    let keep = exp + 1 + n as i32;
    let mut int_digits: Vec<u8>;
    if keep < 0 {
        int_digits = vec![0];
    } else {
        let k = keep as usize;
        let mut d: Vec<u8> = digits.iter().copied().chain(std::iter::repeat(0)).take(k.max(1)).collect();
        if k == 0 {
            d = vec![0];
        }
        let round_up = digits.get(k).map(|&r| r >= 5).unwrap_or(false);
        if k == 0 {
            d[0] = if round_up { 1 } else { 0 };
        } else if round_up {
            let mut i = d.len();
            loop {
                if i == 0 {
                    d.insert(0, 1);
                    break;
                }
                i -= 1;
                if d[i] == 9 {
                    d[i] = 0;
                } else {
                    d[i] += 1;
                    break;
                }
            }
        }
        int_digits = d;
    }
    // int_digits is the number times 10^n, as digits.
    while int_digits.len() < n + 1 {
        int_digits.insert(0, 0);
    }
    let split = int_digits.len() - n;
    let ip: String = int_digits[..split].iter().map(|d| (b'0' + d) as char).collect();
    let ip = ip.trim_start_matches('0');
    let ip = if ip.is_empty() { "0" } else { ip };
    if n == 0 {
        ip.to_string()
    } else {
        let fp: String = int_digits[split..].iter().map(|d| (b'0' + d) as char).collect();
        format!("{ip}.{fp}")
    }
}

pub fn general(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    if !v.is_finite() {
        return "#NUM!".into();
    }
    let a = v.abs();
    if a >= 1e11 || a < 1e-9 {
        // Scientific with up to 6 significant digits.
        let s = format!("{:.5E}", v);
        let (m, e) = s.split_once('E').unwrap();
        let m = m.trim_end_matches('0').trim_end_matches('.');
        let e: i32 = e.parse().unwrap();
        return format!("{m}E{}{:02}", if e < 0 { "-" } else { "+" }, e.abs());
    }
    // Up to 11 characters of digits, as Excel's General does for a normal column width.
    let int_digits = if a >= 1.0 { (a.log10().floor() as i32) + 1 } else { 1 };
    let decimals = (10 - int_digits).max(0) as usize;
    let mut s = fixed(v, decimals);
    if v < 0.0 {
        s.insert(0, '-');
    }
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

fn format_numeric(v: f64, sec: &Section) -> String {
    let toks = &sec.toks;
    // Percent scales; trailing commas scale by 1000 each.
    let percents = toks.iter().filter(|t| matches!(t, Tok::Percent)).count();
    let mut val = v * 100f64.powi(percents as i32);
    let last_digit = toks.iter().rposition(|t| matches!(t, Tok::Digit(_)));
    let mut scale = 0;
    if let Some(ld) = last_digit {
        let mut k = ld + 1;
        while k < toks.len() && matches!(toks[k], Tok::Comma) {
            scale += 1;
            k += 1;
        }
    }
    val /= 1000f64.powi(scale);
    let point = toks.iter().position(|t| matches!(t, Tok::Point));
    let exp = toks.iter().position(|t| matches!(t, Tok::Exp(_)));
    let slash = toks.iter().position(|t| matches!(t, Tok::Slash));
    if let Some(sl) = slash {
        if toks[..sl].iter().any(|t| matches!(t, Tok::Digit(_))) {
            return format_fraction(val, sec, sl);
        }
    }
    let first_digit = toks.iter().position(|t| matches!(t, Tok::Digit(_)));
    let int_end = point.or(exp).unwrap_or(toks.len());
    let int_pat: Vec<char> = toks[..int_end].iter().filter_map(|t| if let Tok::Digit(c) = t { Some(*c) } else { None }).collect();
    let thousands = first_digit.is_some() && toks[first_digit.unwrap()..int_end].iter().any(|t| matches!(t, Tok::Comma)) && {
        // A comma between digits, not only trailing.
        let fd = first_digit.unwrap();
        toks[fd..int_end].iter().enumerate().any(|(i, t)| matches!(t, Tok::Comma) && toks[fd + i + 1..int_end].iter().any(|x| matches!(x, Tok::Digit(_))))
    };
    let frac_pat: Vec<char> = match point {
        Some(p) => toks[p + 1..exp.unwrap_or(toks.len())].iter().filter_map(|t| if let Tok::Digit(c) = t { Some(*c) } else { None }).collect(),
        None => vec![],
    };
    let mut exp_str = String::new();
    let mut mant = val.abs();
    if let Some(e) = exp {
        let exp_digits = toks[e + 1..].iter().filter(|t| matches!(t, Tok::Digit(_))).count().max(1);
        let int_len = int_pat.len().max(1) as i32;
        let mut ex = if mant == 0.0 { 0 } else { mant.log10().floor() as i32 };
        // Engineering-style when the integer pattern has several places (##0.0E+0).
        if int_len > 1 && int_pat.contains(&'#') {
            ex = ex.div_euclid(int_len) * int_len;
        } else {
            ex -= int_len - 1;
        }
        mant /= 10f64.powi(ex);
        let sign = if let Tok::Exp(s) = &toks[e] { s.chars().nth(1).unwrap() } else { '+' };
        let esign = if ex < 0 { "-" } else if sign == '+' { "+" } else { "" };
        exp_str = format!("E{esign}{:0width$}", ex.abs(), width = exp_digits);
    }
    let decimals = frac_pat.len();
    let rounded = fixed(mant, decimals);
    let (ip, fp) = rounded.split_once('.').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((rounded.clone(), String::new()));
    // Integer part against its pattern.
    let mut int_s = if ip == "0" && !int_pat.contains(&'0') { String::new() } else { ip.clone() };
    let min_int = int_pat.iter().filter(|c| **c == '0').count();
    while int_s.len() < min_int {
        int_s.insert(0, '0');
    }
    let q_int = int_pat.iter().take_while(|c| **c != '0').filter(|c| **c == '?').count();
    if thousands && !int_s.is_empty() {
        let digits: Vec<char> = int_s.chars().collect();
        let mut t = String::new();
        for (i, c) in digits.iter().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                t.push(',');
            }
            t.push(*c);
        }
        int_s = t;
    }
    let _ = q_int;
    // Fraction part: trailing # dropped, ? as spaces.
    let mut frac_s: Vec<char> = fp.chars().collect();
    let mut k = frac_s.len();
    while k > 0 {
        let p = frac_pat[k - 1];
        if p == '0' || frac_s[k - 1] != '0' {
            break;
        }
        frac_s[k - 1] = if p == '?' { ' ' } else { '\0' };
        k -= 1;
    }
    let frac_s: String = frac_s.into_iter().filter(|c| *c != '\0').collect();
    // Assemble: literals keep their places around the number.
    let mut out = String::new();
    let mut number_done = false;
    let mut fill = None;
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Digit(_) | Tok::Comma | Tok::Point => {
                if !number_done && Some(i) >= first_digit.or(point) {
                    out.push_str(&int_s);
                    if point.is_some() && (!frac_s.is_empty() || frac_pat.is_empty()) {
                        out.push('.');
                        out.push_str(&frac_s);
                    } else if point.is_some() && frac_pat.is_empty() {
                        out.push('.');
                    }
                    out.push_str(&exp_str);
                    number_done = true;
                }
            }
            Tok::Exp(_) | Tok::Slash => {}
            Tok::Lit(l) => {
                // Literals between digits (e.g. "0-0") are rare; they go where they are.
                out.push_str(l);
            }
            Tok::Percent => out.push('%'),
            Tok::Skip(_) => out.push(' '),
            Tok::Fill(c) => fill = Some(*c),
            Tok::Text => {}
            Tok::Date(d) => out.push_str(d),
        }
    }
    let _ = fill;
    if !number_done && first_digit.is_none() && point.is_none() {
        // A format with no digits (only text): the number is not shown.
    }
    // Digits that followed an exponent were consumed into exp_str: remove duplicates of them.
    out
}

fn format_fraction(v: f64, sec: &Section, slash: usize) -> String {
    let toks = &sec.toks;
    let den_pat: Vec<char> = toks[slash + 1..].iter().take_while(|t| matches!(t, Tok::Digit(_))).filter_map(|t| if let Tok::Digit(c) = t { Some(*c) } else { None }).collect();
    let fixed_den: Option<u64> = {
        let lits: String = toks[slash + 1..].iter().filter_map(|t| if let Tok::Lit(l) = t { Some(l.clone()) } else { None }).collect();
        lits.trim().parse().ok()
    };
    // Is there an integer part (a digit group, then a space, before the numerator)?
    let num_start = toks[..slash].iter().rposition(|t| !matches!(t, Tok::Digit(_))).map(|p| p + 1).unwrap_or(0);
    let has_int = toks[..num_start].iter().any(|t| matches!(t, Tok::Digit(_)));
    let a = v.abs();
    let (int, frac) = if has_int { (a.floor(), a - a.floor()) } else { (0.0, a) };
    let max_den = 10u64.pow(den_pat.len().max(1) as u32) - 1;
    let (mut best_n, mut best_d) = (0u64, 1u64);
    if let Some(d) = fixed_den {
        best_d = d;
        best_n = (frac * d as f64).round() as u64;
    } else {
        let mut best_err = f64::MAX;
        for d in 1..=max_den {
            let n = (frac * d as f64).round() as u64;
            let e = (frac - n as f64 / d as f64).abs();
            if e < best_err - 1e-12 {
                best_err = e;
                best_n = n;
                best_d = d;
            }
        }
    }
    let mut s = String::new();
    if v < 0.0 {
        s.push('-');
    }
    let int_shown = has_int && (int > 0.0 || best_n == 0);
    if int_shown {
        s.push_str(&format!("{}", int as i64));
    }
    if best_n == 0 && has_int {
        return if int_shown { s } else { "0".into() };
    }
    if int_shown {
        s.push(' ');
    }
    s.push_str(&format!("{best_n}/{best_d}"));
    s
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

fn format_date(v: f64, sec: &Section, date1904: bool) -> String {
    let Some((y, mo, d, h, mi, s, sub)) = serial_to_datetime(v, date1904) else { return "#".repeat(8) };
    let ampm = sec.toks.iter().any(|t| matches!(t, Tok::Date(x) if x == "AM/PM" || x == "A/P" || x == "오전/오후"));
    let toks = &sec.toks;
    // Fractional seconds: "ss.00"
    let mut out = String::new();
    let mut i = 0;
    let dow = ((days_from_civil(y, mo, d) % 7 + 11) % 7) as usize; // 1970-01-01 was a Thursday
    while i < toks.len() {
        match &toks[i] {
            Tok::Date(code) => {
                let c = code.as_str();
                match c {
                    "yy" => out.push_str(&format!("{:02}", y.rem_euclid(100))),
                    "y" | "yyy" | "yyyy" => out.push_str(&format!("{:04}", y)),
                    _ if c.starts_with('y') => out.push_str(&format!("{:04}", y)),
                    "m" | "mm" => {
                        // Minutes after h or before s, else months.
                        let prev_h = toks[..i].iter().rev().find_map(|t| if let Tok::Date(x) = t { Some(x.clone()) } else { None }).map(|x| x.starts_with('h') || x.starts_with("[h")).unwrap_or(false);
                        let next_s = toks[i + 1..].iter().find_map(|t| if let Tok::Date(x) = t { Some(x.clone()) } else { None }).map(|x| x.starts_with('s')).unwrap_or(false);
                        if prev_h || next_s {
                            out.push_str(&if c == "mm" { format!("{:02}", mi) } else { mi.to_string() });
                        } else {
                            out.push_str(&if c == "mm" { format!("{:02}", mo) } else { mo.to_string() });
                        }
                    }
                    "mmm" => out.push_str(&MONTHS[(mo - 1) as usize][..3]),
                    "mmmmm" => out.push_str(&MONTHS[(mo - 1) as usize][..1]),
                    _ if c.starts_with("mmmm") => out.push_str(MONTHS[(mo - 1) as usize]),
                    "d" => out.push_str(&d.to_string()),
                    "dd" => out.push_str(&format!("{:02}", d)),
                    "ddd" => out.push_str(&DAYS[dow][..3]),
                    _ if c.starts_with("dddd") => out.push_str(DAYS[dow]),
                    "h" | "hh" => {
                        let hh = if ampm { if h % 12 == 0 { 12 } else { h % 12 } } else { h };
                        out.push_str(&if c == "hh" { format!("{:02}", hh) } else { hh.to_string() });
                    }
                    "s" | "ss" => {
                        out.push_str(&if c == "ss" { format!("{:02}", s) } else { s.to_string() });
                        // .0 / .00 after seconds
                        if matches!(toks.get(i + 1), Some(Tok::Point)) {
                            let n = toks[i + 2..].iter().take_while(|t| matches!(t, Tok::Digit('0'))).count();
                            if n > 0 {
                                let f = format!("{:.*}", n, sub);
                                out.push_str(&f[1..]);
                                i += 1 + n;
                            }
                        }
                    }
                    "AM/PM" => out.push_str(if h < 12 { "AM" } else { "PM" }),
                    "A/P" => out.push_str(if h < 12 { "A" } else { "P" }),
                    "오전/오후" => out.push_str(if h < 12 { "오전" } else { "오후" }),
                    _ if c.starts_with("[h") => out.push_str(&format!("{:0w$}", (v * 24.0).floor() as i64, w = c.len() - 2)),
                    _ if c.starts_with("[m") => out.push_str(&format!("{:0w$}", (v * 1440.0).floor() as i64, w = c.len() - 2)),
                    _ if c.starts_with("[s") => out.push_str(&format!("{:0w$}", (v * 86400.0).round() as i64, w = c.len() - 2)),
                    "e" => out.push_str(&y.to_string()),
                    _ => out.push_str(c),
                }
            }
            Tok::Lit(l) => out.push_str(l),
            Tok::Skip(_) => out.push(' '),
            Tok::Point => out.push('.'),
            Tok::Comma => out.push(','),
            Tok::Slash => out.push('/'),
            Tok::Digit(c) => out.push(*c),
            _ => {}
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers() {
        assert_eq!(format_number(1234.5, "#,##0.00", false), "1,234.50");
        assert_eq!(format_number(1234.5, "#,##0", false), "1,235");
        assert_eq!(format_number(-1234.5, "#,##0", false), "-1,235");
        assert_eq!(format_number(0.256, "0.0%", false), "25.6%");
        assert_eq!(format_number(5.0, "0.00", false), "5.00");
        assert_eq!(format_number(0.5, "#.##", false), ".5");
        assert_eq!(format_number(-5.0, "0.00;[Red](0.00)", false), "(5.00)");
        assert_eq!(format_number(0.0, "0.00;-0.00;\"-\"", false), "-");
        assert_eq!(format_number(12345.678, "0.00E+00", false), "1.23E+04");
        assert_eq!(format_number(1500000.0, "#,##0,,\"M\"", false), "2M");
        assert_eq!(format_number(1234.0, "\"₩\"#,##0", false), "₩1,234");
        assert_eq!(format_number(0.75, "# ?/?", false), "3/4");
        assert_eq!(format_number(1.5, "# ?/?", false), "1 1/2");
        assert_eq!(general(1234567.891), "1234567.891");
        assert_eq!(general(0.1 + 0.2), "0.3");
        assert_eq!(general(1e12), "1E+12");
        assert_eq!(format_number(42.0, "General", false), "42");
        assert_eq!(format_number(2.5, "0", false), "3");
        assert_eq!(format_number(1.005, "0.00", false), "1.01");
        assert_eq!(format_number(0.125, "0.00", false), "0.13");
        assert_eq!(fixed(999.996, 2), "1000.00");
    }
    #[test]
    fn dates() {
        assert_eq!(format_number(45306.0, "yyyy-mm-dd", false), "2024-01-15");
        assert_eq!(format_number(45306.75, "yyyy-mm-dd hh:mm", false), "2024-01-15 18:00");
        assert_eq!(format_number(45306.75, "h:mm AM/PM", false), "6:00 PM");
        assert_eq!(format_number(45306.0, "d-mmm-yy", false), "15-Jan-24");
        assert_eq!(format_number(45306.0, "dddd, mmmm d", false), "Monday, January 15");
        assert_eq!(format_number(1.5, "[h]:mm", false), "36:00");
        assert_eq!(format_number(60.0, "yyyy-mm-dd", false), "1900-02-29");
        assert!(is_date("yyyy-mm-dd"));
        assert!(!is_date("#,##0.00"));
        assert!(!is_date("\"day\"0"));
        assert_eq!(datetime_to_serial(2024, 1, 15, 0, 0, 0.0), 45306.0);
    }
}
