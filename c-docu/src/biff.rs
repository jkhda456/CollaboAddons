//! BIFF8 formulas (the parsed form .xls files keep): tokens to text and text to tokens, and the
//! built-in function table.

use crate::formula::{self, col_name, Expr, Val, XlErr};

/// (index, name, fixed argument count or -1 when variable)
pub const FUNCS: &[(u16, &str, i8)] = &[
    (0, "COUNT", -1), (1, "IF", -1), (2, "ISNA", 1), (3, "ISERROR", 1), (4, "SUM", -1), (5, "AVERAGE", -1), (6, "MIN", -1), (7, "MAX", -1),
    (8, "ROW", -1), (9, "COLUMN", -1), (10, "NA", 0), (11, "NPV", -1), (12, "STDEV", -1), (13, "DOLLAR", -1), (14, "FIXED", -1), (15, "SIN", 1),
    (16, "COS", 1), (17, "TAN", 1), (18, "ATAN", 1), (19, "PI", 0), (20, "SQRT", 1), (21, "EXP", 1), (22, "LN", 1), (23, "LOG10", 1), (24, "ABS", 1),
    (25, "INT", 1), (26, "SIGN", 1), (27, "ROUND", 2), (28, "LOOKUP", -1), (29, "INDEX", -1), (30, "REPT", 2), (31, "MID", 3), (32, "LEN", 1),
    (33, "VALUE", 1), (34, "TRUE", 0), (35, "FALSE", 0), (36, "AND", -1), (37, "OR", -1), (38, "NOT", 1), (39, "MOD", 2), (40, "DCOUNT", 3),
    (41, "DSUM", 3), (42, "DAVERAGE", 3), (43, "DMIN", 3), (44, "DMAX", 3), (45, "DSTDEV", 3), (46, "VAR", -1), (47, "DVAR", 3), (48, "TEXT", 2),
    (49, "LINEST", -1), (50, "TREND", -1), (51, "LOGEST", -1), (52, "GROWTH", -1), (56, "PV", -1), (57, "FV", -1), (58, "NPER", -1), (59, "PMT", -1),
    (60, "RATE", -1), (61, "MIRR", 3), (62, "IRR", -1), (63, "RAND", 0), (64, "MATCH", -1), (65, "DATE", 3), (66, "TIME", 3), (67, "DAY", 1),
    (68, "MONTH", 1), (69, "YEAR", 1), (70, "WEEKDAY", -1), (71, "HOUR", 1), (72, "MINUTE", 1), (73, "SECOND", 1), (74, "NOW", 0), (75, "AREAS", 1),
    (76, "ROWS", 1), (77, "COLUMNS", 1), (78, "OFFSET", -1), (82, "SEARCH", -1), (83, "TRANSPOSE", 1), (86, "TYPE", 1), (97, "ATAN2", 2), (98, "ASIN", 1),
    (99, "ACOS", 1), (100, "CHOOSE", -1), (101, "HLOOKUP", -1), (102, "VLOOKUP", -1), (105, "ISREF", 1), (109, "LOG", -1), (111, "CHAR", 1),
    (112, "LOWER", 1), (113, "UPPER", 1), (114, "PROPER", 1), (115, "LEFT", -1), (116, "RIGHT", -1), (117, "EXACT", 2), (118, "TRIM", 1),
    (119, "REPLACE", 4), (120, "SUBSTITUTE", -1), (121, "CODE", 1), (124, "FIND", -1), (125, "CELL", -1), (126, "ISERR", 1), (127, "ISTEXT", 1),
    (128, "ISNUMBER", 1), (129, "ISBLANK", 1), (130, "T", 1), (131, "N", 1), (140, "DATEVALUE", 1), (141, "TIMEVALUE", 1), (142, "SLN", 3),
    (143, "SYD", 4), (144, "DDB", -1), (148, "INDIRECT", -1), (162, "CLEAN", 1), (163, "MDETERM", 1), (164, "MINVERSE", 1), (165, "MMULT", 2),
    (167, "IPMT", -1), (168, "PPMT", -1), (169, "COUNTA", -1), (183, "PRODUCT", -1), (184, "FACT", 1), (189, "DPRODUCT", 3), (190, "ISNONTEXT", 1),
    (193, "STDEVP", -1), (194, "VARP", -1), (195, "DSTDEVP", 3), (196, "DVARP", 3), (197, "TRUNC", -1), (198, "ISLOGICAL", 1), (199, "DCOUNTA", 3),
    (204, "USDOLLAR", -1), (205, "FINDB", -1), (206, "SEARCHB", -1), (207, "REPLACEB", 4), (208, "LEFTB", -1), (209, "RIGHTB", -1), (210, "MIDB", 3),
    (211, "LENB", 1), (212, "ROUNDUP", 2), (213, "ROUNDDOWN", 2), (214, "ASC", 1), (215, "DBCS", 1), (216, "RANK", -1), (219, "ADDRESS", -1),
    (220, "DAYS360", -1), (221, "TODAY", 0), (222, "VDB", -1), (227, "MEDIAN", -1), (228, "SUMPRODUCT", -1), (229, "SINH", 1), (230, "COSH", 1),
    (231, "TANH", 1), (232, "ASINH", 1), (233, "ACOSH", 1), (234, "ATANH", 1), (235, "DGET", 3), (244, "INFO", 1), (247, "DB", -1), (252, "FREQUENCY", 2),
    (261, "ERROR.TYPE", 1), (269, "AVEDEV", -1), (270, "BETADIST", -1), (271, "GAMMALN", 1), (272, "BETAINV", -1), (273, "BINOMDIST", 4),
    (274, "CHIDIST", 2), (275, "CHIINV", 2), (276, "COMBIN", 2), (277, "CONFIDENCE", 3), (278, "CRITBINOM", 3), (279, "EVEN", 1), (280, "EXPONDIST", 3),
    (281, "FDIST", 3), (282, "FINV", 3), (283, "FISHER", 1), (284, "FISHERINV", 1), (285, "FLOOR", 2), (286, "GAMMADIST", 4), (287, "GAMMAINV", 3),
    (288, "CEILING", 2), (289, "HYPGEOMDIST", 4), (290, "LOGNORMDIST", 3), (291, "LOGINV", 3), (292, "NEGBINOMDIST", 3), (293, "NORMDIST", 4),
    (294, "NORMSDIST", 1), (295, "NORMINV", 3), (296, "NORMSINV", 1), (297, "STANDARDIZE", 3), (298, "ODD", 1), (299, "PERMUT", 2), (300, "POISSON", 3),
    (301, "TDIST", 3), (302, "WEIBULL", 4), (303, "SUMXMY2", 2), (304, "SUMX2MY2", 2), (305, "SUMX2PY2", 2), (306, "CHITEST", 2), (307, "CORREL", 2),
    (308, "COVAR", 2), (309, "FORECAST", 3), (310, "FTEST", 2), (311, "INTERCEPT", 2), (312, "PEARSON", 2), (313, "RSQ", 2), (314, "STEYX", 2),
    (315, "SLOPE", 2), (316, "TTEST", 4), (317, "PROB", -1), (318, "DEVSQ", -1), (319, "GEOMEAN", -1), (320, "HARMEAN", -1), (321, "SUMSQ", -1),
    (322, "KURT", -1), (323, "SKEW", -1), (324, "ZTEST", -1), (325, "LARGE", 2), (326, "SMALL", 2), (327, "QUARTILE", 2), (328, "PERCENTILE", 2),
    (329, "PERCENTRANK", -1), (330, "MODE", -1), (331, "TRIMMEAN", 2), (332, "TINV", 2), (336, "CONCATENATE", -1), (337, "POWER", 2), (342, "RADIANS", 1),
    (343, "DEGREES", 1), (344, "SUBTOTAL", -1), (345, "SUMIF", -1), (346, "COUNTIF", 2), (347, "COUNTBLANK", 1), (350, "ISPMT", 4), (351, "DATEDIF", 3),
    (352, "DATESTRING", 1), (353, "NUMBERSTRING", 2), (354, "ROMAN", -1), (358, "GETPIVOTDATA", -1), (359, "HYPERLINK", -1), (360, "PHONETIC", 1),
    (361, "AVERAGEA", -1), (362, "MAXA", -1), (363, "MINA", -1), (364, "STDEVPA", -1), (365, "VARPA", -1), (366, "STDEVA", -1), (367, "VARA", -1),
];

pub fn func_by_index(i: u16) -> Option<(&'static str, i8)> {
    FUNCS.iter().find(|f| f.0 == i).map(|f| (f.1, f.2))
}

pub fn func_by_name(n: &str) -> Option<(u16, i8)> {
    FUNCS.iter().find(|f| f.1.eq_ignore_ascii_case(n)).map(|f| (f.0, f.2))
}

/// What formulas need from the workbook to be read: sheet names for 3-D references (by
/// EXTERNSHEET index) and defined names (by index).
pub trait Names {
    fn sheet_of_xti(&self, ixti: u16) -> Option<String>;
    fn name(&self, index: u32) -> Option<String>;
}

fn u16_at(d: &[u8], i: usize) -> u16 {
    d.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0)
}

fn cell_ref(rw: u16, col: u16, base: Option<(u32, u32)>) -> String {
    let row_rel = col & 0x8000 != 0;
    let col_rel = col & 0x4000 != 0;
    let c = (col & 0x3FFF) as i64;
    let r = rw as i64;
    let (r, c) = match base {
        // RefN/AreaN in shared formulas: relative parts are offsets from the cell.
        Some((br, bc)) => (
            if row_rel { (br as i64 + (r as i16) as i64).rem_euclid(65536) } else { r },
            if col_rel { (bc as i64 + ((c as i64) << 56 >> 56)).rem_euclid(256) } else { c },
        ),
        None => (r, c),
    };
    format!("{}{}{}{}", if col_rel { "" } else { "$" }, col_name(c as u32), if row_rel { "" } else { "$" }, r + 1)
}

/// The text of a formula's tokens (`rgce`), for the cell at (row, col) (for shared formulas).
pub fn decompile(rgce: &[u8], at: (u32, u32), names: &dyn Names) -> Result<String, String> {
    let mut st: Vec<String> = Vec::new();
    let mut i = 0;
    let pop = |st: &mut Vec<String>| st.pop().ok_or_else(|| "formula stack underflow".to_string());
    while i < rgce.len() {
        let ptg = rgce[i];
        i += 1;
        match ptg {
            0x01 | 0x02 => {
                // A shared or array formula's anchor: the caller resolves it.
                return Err(format!("shared:{},{}", u16_at(rgce, i), u16_at(rgce, i + 2)));
            }
            0x03..=0x11 => {
                let b = pop(&mut st)?;
                let a = pop(&mut st)?;
                let op = ["+", "-", "*", "/", "^", "&", "<", "<=", "=", ">=", ">", "<>", " ", ",", ":"][(ptg - 3) as usize];
                st.push(format!("{a}{op}{b}"));
            }
            0x12 => {
                let a = pop(&mut st)?;
                st.push(format!("+{a}"));
            }
            0x13 => {
                let a = pop(&mut st)?;
                st.push(format!("-{a}"));
            }
            0x14 => {
                let a = pop(&mut st)?;
                st.push(format!("{a}%"));
            }
            0x15 => {
                let a = pop(&mut st)?;
                st.push(format!("({a})"));
            }
            0x16 => st.push(String::new()),
            0x17 => {
                let n = rgce[i] as usize;
                let flags = rgce[i + 1];
                i += 2;
                let s = if flags & 1 != 0 {
                    let units: Vec<u16> = (0..n).map(|k| u16_at(rgce, i + k * 2)).collect();
                    i += n * 2;
                    String::from_utf16_lossy(&units)
                } else {
                    let s: String = rgce[i..i + n].iter().map(|&b| b as char).collect();
                    i += n;
                    s
                };
                st.push(format!("\"{}\"", s.replace('"', "\"\"")));
            }
            0x18 => return Err("extended formula tokens".into()),
            0x19 => {
                let kind = rgce[i];
                i += 1;
                match kind {
                    0x04 => {
                        // CHOOSE jump table.
                        let n = u16_at(rgce, i) as usize;
                        i += 2 + (n + 1) * 2;
                    }
                    0x10 => {
                        let a = pop(&mut st)?;
                        st.push(format!("SUM({a})"));
                        i += 2;
                    }
                    _ => i += 2,
                }
            }
            0x1C => {
                let e = rgce[i];
                i += 1;
                st.push(err_text(e).to_string());
            }
            0x1D => {
                st.push(if rgce[i] != 0 { "TRUE".into() } else { "FALSE".into() });
                i += 1;
            }
            0x1E => {
                st.push(u16_at(rgce, i).to_string());
                i += 2;
            }
            0x1F => {
                let v = f64::from_le_bytes(rgce[i..i + 8].try_into().map_err(|_| "short number")?);
                st.push(crate::numfmt::general(v));
                i += 8;
            }
            p if p >= 0x20 => {
                let base = (p & 0x1F) | 0x20;
                match base {
                    0x20 => {
                        // Array constant: its values are after the tokens (rgcb); shown empty.
                        st.push("{}".into());
                        i += 7;
                    }
                    0x21 => {
                        let idx = u16_at(rgce, i);
                        i += 2;
                        let (name, n) = func_by_index(idx).unwrap_or(("UNKNOWN", 0));
                        let n = n.max(0) as usize;
                        let mut args = Vec::new();
                        for _ in 0..n {
                            args.push(pop(&mut st)?);
                        }
                        args.reverse();
                        st.push(format!("{name}({})", args.join(",")));
                    }
                    0x22 => {
                        let argc = (rgce[i] & 0x7F) as usize;
                        let idx = u16_at(rgce, i + 1) & 0x7FFF;
                        i += 3;
                        let mut args = Vec::new();
                        for _ in 0..argc {
                            args.push(pop(&mut st)?);
                        }
                        args.reverse();
                        if idx == 255 {
                            // An add-in or newer function: its name is the first argument.
                            let name = if args.is_empty() { "UNKNOWN".to_string() } else { args.remove(0) };
                            st.push(format!("{}({})", name.trim_start_matches("_xlfn."), args.join(",")));
                        } else {
                            let name = func_by_index(idx).map(|f| f.0).unwrap_or("UNKNOWN");
                            st.push(format!("{name}({})", args.join(",")));
                        }
                    }
                    0x23 => {
                        let idx = u32::from_le_bytes(rgce[i..i + 4].try_into().map_err(|_| "short name")?);
                        i += 4;
                        st.push(names.name(idx).unwrap_or_else(|| format!("NAME{idx}")));
                    }
                    0x24 => {
                        st.push(cell_ref(u16_at(rgce, i), u16_at(rgce, i + 2), None));
                        i += 4;
                    }
                    0x25 => {
                        let (r1, r2, c1, c2) = (u16_at(rgce, i), u16_at(rgce, i + 2), u16_at(rgce, i + 4), u16_at(rgce, i + 6));
                        st.push(format!("{}:{}", cell_ref(r1, c1, None), cell_ref(r2, c2, None)));
                        i += 8;
                    }
                    0x26 | 0x27 | 0x28 | 0x29 => {
                        // Mem tokens: their sub-expression follows; skip the header.
                        i += if base == 0x29 { 2 } else { 6 };
                    }
                    0x2A => {
                        st.push("#REF!".into());
                        i += 4;
                    }
                    0x2B => {
                        st.push("#REF!".into());
                        i += 8;
                    }
                    0x2C => {
                        st.push(cell_ref(u16_at(rgce, i), u16_at(rgce, i + 2), Some(at)));
                        i += 4;
                    }
                    0x2D => {
                        let (r1, r2, c1, c2) = (u16_at(rgce, i), u16_at(rgce, i + 2), u16_at(rgce, i + 4), u16_at(rgce, i + 6));
                        st.push(format!("{}:{}", cell_ref(r1, c1, Some(at)), cell_ref(r2, c2, Some(at))));
                        i += 8;
                    }
                    0x39 => {
                        let idx = u32::from_le_bytes(rgce[i + 2..i + 6].try_into().map_err(|_| "short namex")?);
                        i += 6;
                        st.push(names.name(idx).unwrap_or_else(|| format!("NAME{idx}")));
                    }
                    0x3A | 0x3B | 0x3C | 0x3D => {
                        let ixti = u16_at(rgce, i);
                        let sheet = names.sheet_of_xti(ixti).map(|s| format!("{}!", formula::quote_sheet(&s))).unwrap_or_default();
                        match base {
                            0x3A => st.push(format!("{sheet}{}", cell_ref(u16_at(rgce, i + 2), u16_at(rgce, i + 4), None))),
                            0x3B => {
                                let (r1, r2, c1, c2) = (u16_at(rgce, i + 2), u16_at(rgce, i + 4), u16_at(rgce, i + 6), u16_at(rgce, i + 8));
                                st.push(format!("{sheet}{}:{}", cell_ref(r1, c1, None), cell_ref(r2, c2, None)));
                            }
                            _ => st.push(format!("{sheet}#REF!")),
                        }
                        i += if base == 0x3A || base == 0x3C { 6 } else { 10 };
                    }
                    _ => return Err(format!("formula token {p:#x}")),
                }
            }
            p => return Err(format!("formula token {p:#x}")),
        }
    }
    st.pop().ok_or_else(|| "empty formula".into())
}

pub fn err_text(e: u8) -> &'static str {
    match e {
        0x00 => "#NULL!",
        0x07 => "#DIV/0!",
        0x0F => "#VALUE!",
        0x17 => "#REF!",
        0x1D => "#NAME?",
        0x24 => "#NUM!",
        0x2A => "#N/A",
        _ => "#N/A",
    }
}

pub fn err_code(e: &XlErr) -> u8 {
    match e {
        XlErr::Null => 0x00,
        XlErr::Div0 => 0x07,
        XlErr::Value => 0x0F,
        XlErr::Ref => 0x17,
        XlErr::Name => 0x1D,
        XlErr::Num => 0x24,
        _ => 0x2A,
    }
}

/// Tokens for a formula's text: references (3-D through `sheet_xti`), constants, operators and
/// built-in functions.
pub fn compile(text: &str, sheet_xti: &dyn Fn(&str) -> Option<u16>) -> Result<Vec<u8>, String> {
    let e = formula::parse(text.trim_start_matches('='))?;
    let mut out = Vec::new();
    emit(&e, &mut out, sheet_xti, 0x40)?;
    Ok(out)
}

fn emit(e: &Expr, out: &mut Vec<u8>, xti: &dyn Fn(&str) -> Option<u16>, class: u8) -> Result<(), String> {
    let col_bits = |r: &formula::CellRef| -> u16 { (r.col as u16 & 0x3FFF) | if r.abs_row { 0 } else { 0x8000 } | if r.abs_col { 0 } else { 0x4000 } };
    match e {
        Expr::Lit(Val::Num(n)) => {
            if n.fract() == 0.0 && *n >= 0.0 && *n <= 65535.0 {
                out.push(0x1E);
                out.extend_from_slice(&(*n as u16).to_le_bytes());
            } else {
                out.push(0x1F);
                out.extend_from_slice(&n.to_le_bytes());
            }
        }
        Expr::Lit(Val::Str(s)) => {
            let units: Vec<u16> = s.encode_utf16().collect();
            if units.len() > 255 {
                return Err("a string in a formula is over 255 characters".into());
            }
            out.push(0x17);
            out.push(units.len() as u8);
            if units.iter().all(|u| *u < 256) {
                out.push(0);
                out.extend(units.iter().map(|u| *u as u8));
            } else {
                out.push(1);
                for u in units {
                    out.extend_from_slice(&u.to_le_bytes());
                }
            }
        }
        Expr::Lit(Val::Bool(b)) => {
            out.push(0x1D);
            out.push(*b as u8);
        }
        Expr::Lit(Val::Err(e)) => {
            out.push(0x1C);
            out.push(err_code(e));
        }
        Expr::Lit(_) | Expr::Missing => out.push(0x16),
        Expr::Area(a) => {
            let sheet = a.sheet.as_ref();
            let ixti = match sheet {
                Some(s) => Some(xti(s).ok_or_else(|| format!("no sheet {s}"))?),
                None => None,
            };
            let ref_class = if class == 0x40 && !a.single() { 0x20 } else { class };
            if a.single() {
                match ixti {
                    Some(x) => {
                        out.push(0x1A | ref_class);
                        out.extend_from_slice(&x.to_le_bytes());
                    }
                    None => out.push(0x04 | ref_class),
                }
                out.extend_from_slice(&(a.a.row as u16).to_le_bytes());
                out.extend_from_slice(&col_bits(&a.a).to_le_bytes());
            } else {
                match ixti {
                    Some(x) => {
                        out.push(0x1B | ref_class);
                        out.extend_from_slice(&x.to_le_bytes());
                    }
                    None => out.push(0x05 | ref_class),
                }
                let (r0, r1) = a.rows();
                let (c0, c1) = a.cols();
                let ra = formula::CellRef { row: r0, col: c0, abs_row: a.a.abs_row, abs_col: a.a.abs_col };
                let rb = formula::CellRef { row: r1.min(65535), col: c1.min(255), abs_row: a.b.abs_row, abs_col: a.b.abs_col };
                out.extend_from_slice(&(ra.row as u16).to_le_bytes());
                out.extend_from_slice(&(rb.row as u16).to_le_bytes());
                out.extend_from_slice(&col_bits(&ra).to_le_bytes());
                out.extend_from_slice(&col_bits(&rb).to_le_bytes());
            }
        }
        Expr::Neg(x) => {
            emit(x, out, xti, class)?;
            out.push(0x13);
        }
        Expr::Pct(x) => {
            emit(x, out, xti, class)?;
            out.push(0x14);
        }
        Expr::Bin(op, a, b) => {
            emit(a, out, xti, 0x40)?;
            emit(b, out, xti, 0x40)?;
            out.push(match op.as_str() {
                "+" => 0x03,
                "-" => 0x04,
                "*" => 0x05,
                "/" => 0x06,
                "^" => 0x07,
                "&" => 0x08,
                "<" => 0x09,
                "<=" => 0x0A,
                "=" => 0x0B,
                ">=" => 0x0C,
                ">" => 0x0D,
                "<>" => 0x0E,
                o => return Err(format!("operator {o}")),
            });
        }
        Expr::Range(a, b) => {
            emit(a, out, xti, 0x20)?;
            emit(b, out, xti, 0x20)?;
            out.push(0x11);
        }
        Expr::Call(name, args) => {
            let (idx, fixed) = func_by_name(name).ok_or_else(|| format!("{name} cannot be written in an .xls formula"))?;
            for a in args {
                // Functions take references for their range arguments.
                emit(a, out, xti, if matches!(a, Expr::Area(_)) { 0x20 } else { 0x40 })?;
            }
            if fixed >= 0 && fixed as usize == args.len() {
                out.push(0x41);
                out.extend_from_slice(&idx.to_le_bytes());
            } else {
                out.push(0x42);
                out.push(args.len() as u8);
                out.extend_from_slice(&idx.to_le_bytes());
            }
        }
        Expr::Name(n) => return Err(format!("names ({n}) cannot be written in an .xls formula")),
        Expr::Array(_) => return Err("array constants cannot be written in an .xls formula".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct N;
    impl Names for N {
        fn sheet_of_xti(&self, _: u16) -> Option<String> {
            Some("Data".into())
        }
        fn name(&self, _: u32) -> Option<String> {
            None
        }
    }
    #[test]
    fn roundtrip() {
        for f in ["SUM(A1:B2)+3", "IF(A1>5,\"big\",\"small\")", "Data!B2*2", "ROUND(A1/7,2)", "-$A$1%", "AVERAGE(A1,B1,C1)"] {
            let t = compile(f, &|_| Some(0)).unwrap();
            assert_eq!(decompile(&t, (0, 0), &N).unwrap(), f, "{f}");
        }
    }
}
