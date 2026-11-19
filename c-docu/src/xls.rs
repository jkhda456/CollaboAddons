//! Excel 97-2003 workbooks (.xls, BIFF8 in a compound file), edited in part. The Workbook
//! stream is read as records; every record the edit does not concern is written back as it was.
//! A sheet's cell table (its rows and cells) is laid out again, with its DBCELL and INDEX offsets
//! computed for the new places; the shared string table is written again (with its EXTSST) when
//! a string changed; BOUNDSHEET offsets follow the sheets. Values that formulas depend on are
//! recalculated and the formulas' cached results updated, and the sheets are marked for Excel
//! to recalculate on open.

use crate::biff;
use crate::cfb::Cfb;
use crate::formula::{self, Area, CellRef, Ctx, Val, XlErr};
use crate::model::{Item, Match, Outline};
use crate::numfmt;
use crate::text::{self, Finder};
use crate::{err, Result};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct Rec {
    pub typ: u16,
    pub data: Vec<u8>,
}

const BOF: u16 = 0x0809;
const EOF: u16 = 0x000A;
const CONTINUE: u16 = 0x003C;
const SST: u16 = 0x00FC;
const EXTSST: u16 = 0x00FF;
const BOUNDSHEET: u16 = 0x0085;
const ROW: u16 = 0x0208;
const DBCELL: u16 = 0x00D7;
const INDEX: u16 = 0x020B;
const DIMENSIONS: u16 = 0x0200;
const NUMBER: u16 = 0x0203;
const LABELSST: u16 = 0x00FD;
const LABEL: u16 = 0x0204;
const RK: u16 = 0x027E;
const MULRK: u16 = 0x00BD;
const BLANK: u16 = 0x0201;
const MULBLANK: u16 = 0x00BE;
const BOOLERR: u16 = 0x0205;
const FORMULA: u16 = 0x0006;
const STRING: u16 = 0x0207;
const SHRFMLA: u16 = 0x04BC;
const ARRAY: u16 = 0x0221;
const TABLE: u16 = 0x0236;
const RSTRING: u16 = 0x00D6;
const UNCALCED: u16 = 0x005E;
const DEFCOLWIDTH: u16 = 0x0055;

fn is_cell(t: u16) -> bool {
    matches!(t, NUMBER | LABELSST | LABEL | RK | MULRK | BLANK | MULBLANK | BOOLERR | FORMULA | RSTRING)
}

fn u16_at(d: &[u8], i: usize) -> u16 {
    d.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0)
}
fn u32_at(d: &[u8], i: usize) -> u32 {
    d.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0)
}

fn rk_value(rk: u32) -> f64 {
    let mut v = if rk & 2 != 0 { ((rk as i32) >> 2) as f64 } else { f64::from_bits(((rk & 0xFFFF_FFFC) as u64) << 32) };
    if rk & 1 != 0 {
        v /= 100.0;
    }
    v
}

/// One cell's records: the cell record (or a multi-cell record) and what follows it (a formula's
/// STRING, SHRFMLA, ARRAY or TABLE).
#[derive(Clone, Debug)]
pub struct Unit {
    pub recs: Vec<Rec>,
    pub last_col: u16,
}

#[derive(Clone, Debug)]
pub struct XSheet {
    pub name: String,
    pub hidden: bool,
    pub worksheet: bool,
    bs_index: usize,
    before: Vec<Rec>,
    rows: BTreeMap<u16, Vec<u8>>,
    units: BTreeMap<(u16, u16), Unit>,
    after: Vec<Rec>,
    changed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SstEntry {
    pub text: String,
    runs: Vec<(u16, u16)>,
    ext: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct FontInfo {
    pub name: String,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: u16,
}

#[derive(Clone, Debug, Default)]
pub struct XfInfo {
    pub font: u16,
    pub fmt: u16,
    pub halign: u8,
    pub valign: u8,
    pub wrap: bool,
    pub fill: Option<u16>,
    pub borders: [u8; 4],
    pub border_colors: [u16; 4],
    pub indent: u8,
    pub rotation: u8,
}

pub struct Xls {
    cfb: Cfb,
    stream: String,
    globals: Vec<Rec>,
    pub sheets: Vec<XSheet>,
    pub sst: Vec<SstEntry>,
    sst_dirty: bool,
    pub kind: &'static str,
    pub fonts: Vec<FontInfo>,
    pub xfs: Vec<XfInfo>,
    pub formats: HashMap<u16, String>,
    pub palette: Vec<String>,
    pub date1904: bool,
    xti: Vec<(u16, i16, i16)>,
    self_supbook: Option<u16>,
    names: Vec<String>,
    changed: HashSet<(usize, u32, u32)>,
    pub warnings: Vec<String>,
}

/// Records of a stream, with CONTINUE kept as records of their own.
fn records(d: &[u8]) -> Vec<(usize, Rec)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= d.len() {
        let typ = u16_at(d, i);
        let len = u16_at(d, i + 2) as usize;
        let end = (i + 4 + len).min(d.len());
        out.push((i, Rec { typ, data: d[i + 4..end].to_vec() }));
        i = end;
        if typ == 0 && len == 0 {
            break;
        }
    }
    out
}

/// Reads XLUnicodeString-like data across CONTINUE boundaries.
struct Reader<'a> {
    segs: Vec<&'a [u8]>,
    seg: usize,
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(segs: Vec<&'a [u8]>) -> Reader<'a> {
        Reader { segs, seg: 0, pos: 0 }
    }
    fn left(&self) -> usize {
        self.segs.get(self.seg).map(|s| s.len().saturating_sub(self.pos)).unwrap_or(0)
    }
    fn next_seg(&mut self) -> bool {
        if self.seg + 1 < self.segs.len() {
            self.seg += 1;
            self.pos = 0;
            true
        } else {
            false
        }
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            if self.left() == 0 && !self.next_seg() {
                break;
            }
            let take = (n - out.len()).min(self.left());
            out.extend_from_slice(&self.segs[self.seg][self.pos..self.pos + take]);
            self.pos += take;
        }
        out
    }
    fn u8(&mut self) -> u8 {
        self.bytes(1).first().copied().unwrap_or(0)
    }
    fn u16(&mut self) -> u16 {
        let b = self.bytes(2);
        u16_at(&b, 0)
    }
    fn u32(&mut self) -> u32 {
        let b = self.bytes(4);
        u32_at(&b, 0)
    }
    /// Characters of a string: when the record ends inside them, the next begins with a new
    /// high-byte flag.
    fn chars(&mut self, n: usize, mut high: bool) -> String {
        let mut units: Vec<u16> = Vec::with_capacity(n);
        while units.len() < n {
            if self.left() == 0 {
                if !self.next_seg() {
                    break;
                }
                high = self.segs[self.seg].first().map(|f| f & 1 != 0).unwrap_or(false);
                self.pos = 1;
            }
            if high {
                if self.left() < 2 {
                    self.pos += self.left();
                    continue;
                }
                units.push(u16_at(self.segs[self.seg], self.pos));
                self.pos += 2;
            } else {
                units.push(self.segs[self.seg][self.pos] as u16);
                self.pos += 1;
            }
        }
        String::from_utf16_lossy(&units)
    }
    fn rich_string(&mut self) -> SstEntry {
        let cch = self.u16() as usize;
        let flags = self.u8();
        let runs_n = if flags & 0x08 != 0 { self.u16() as usize } else { 0 };
        let ext_n = if flags & 0x04 != 0 { self.u32() as usize } else { 0 };
        let text = self.chars(cch, flags & 1 != 0);
        let mut runs = Vec::with_capacity(runs_n);
        for _ in 0..runs_n {
            let b = self.bytes(4);
            runs.push((u16_at(&b, 0), u16_at(&b, 2)));
        }
        let ext = self.bytes(ext_n);
        SstEntry { text, runs, ext }
    }
}

fn short_string(d: &[u8], i: usize) -> (String, usize) {
    // ShortXLUnicodeString: cch u8, flags u8, chars.
    let n = *d.get(i).unwrap_or(&0) as usize;
    let high = d.get(i + 1).map(|f| f & 1 != 0).unwrap_or(false);
    if high {
        let units: Vec<u16> = (0..n).map(|k| u16_at(d, i + 2 + k * 2)).collect();
        (String::from_utf16_lossy(&units), 2 + n * 2)
    } else {
        (d.get(i + 2..i + 2 + n).map(|b| b.iter().map(|&c| c as char).collect()).unwrap_or_default(), 2 + n)
    }
}

fn long_string(d: &[u8], i: usize) -> (String, usize) {
    // XLUnicodeString: cch u16, flags u8, chars.
    let n = u16_at(d, i) as usize;
    let high = d.get(i + 2).map(|f| f & 1 != 0).unwrap_or(false);
    if high {
        let units: Vec<u16> = (0..n).map(|k| u16_at(d, i + 3 + k * 2)).collect();
        (String::from_utf16_lossy(&units), 3 + n * 2)
    } else {
        (d.get(i + 3..i + 3 + n).map(|b| b.iter().map(|&c| c as char).collect()).unwrap_or_default(), 3 + n)
    }
}

fn encode_long_string(s: &str) -> Vec<u8> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut out = Vec::new();
    out.extend_from_slice(&(units.len() as u16).to_le_bytes());
    if units.iter().all(|u| *u < 256) {
        out.push(0);
        out.extend(units.iter().map(|u| *u as u8));
    } else {
        out.push(1);
        for u in units {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
    out
}

const DEFAULT_PALETTE: [&str; 56] = [
    "000000", "FFFFFF", "FF0000", "00FF00", "0000FF", "FFFF00", "FF00FF", "00FFFF", "800000", "008000", "000080", "808000", "800080", "008080", "C0C0C0", "808080", "9999FF", "993366", "FFFFCC",
    "CCFFFF", "660066", "FF8080", "0066CC", "CCCCFF", "000080", "FF00FF", "FFFF00", "00FFFF", "800080", "800000", "008080", "0000FF", "00CCFF", "CCFFFF", "CCFFCC", "FFFF99", "99CCFF", "FF99CC",
    "CC99FF", "FFCC99", "3366FF", "33CCCC", "99CC00", "FFCC00", "FF9900", "FF6600", "666699", "969696", "003366", "339966", "003300", "333300", "993300", "993366", "333399", "333333",
];

impl Xls {
    pub fn open(cfb: Cfb) -> Result<Xls> {
        let stream = if cfb.stream("Workbook").is_some() { "Workbook" } else { "Book" }.to_string();
        let data = cfb.stream(&stream).ok_or_else(|| err("no Workbook stream"))?.to_vec();
        let recs = records(&data);
        if recs.first().map(|r| r.1.typ) != Some(BOF) || u16_at(&recs[0].1.data, 0) != 0x0600 {
            return Err(err("an Excel file older than Excel 97 (BIFF5 or earlier): not supported"));
        }
        // Substreams: globals, then each BOF..EOF.
        let mut subs: Vec<(usize, Vec<Rec>)> = Vec::new();
        let mut cur: Vec<Rec> = Vec::new();
        let mut start = 0;
        let mut depth = 0;
        for (off, r) in recs {
            if r.typ == BOF {
                if depth == 0 {
                    start = off;
                }
                depth += 1;
            }
            let is_eof = r.typ == EOF;
            cur.push(r);
            if is_eof {
                depth -= 1;
                if depth <= 0 {
                    subs.push((start, std::mem::take(&mut cur)));
                    depth = 0;
                }
            }
        }
        if !cur.is_empty() {
            subs.push((start, cur));
        }
        let (_, globals) = subs.remove(0);
        let mut x = Xls {
            cfb,
            stream,
            globals,
            sheets: Vec::new(),
            sst: Vec::new(),
            sst_dirty: false,
            kind: "xls",
            fonts: Vec::new(),
            xfs: Vec::new(),
            formats: HashMap::new(),
            palette: DEFAULT_PALETTE.iter().map(|s| s.to_string()).collect(),
            date1904: false,
            xti: Vec::new(),
            self_supbook: None,
            names: Vec::new(),
            changed: HashSet::new(),
            warnings: Vec::new(),
        };
        x.read_globals();
        // BOUNDSHEETs point at their substreams by offset.
        let bounds: Vec<(usize, u32, String, bool, u8)> = x
            .globals
            .iter()
            .enumerate()
            .filter(|(_, r)| r.typ == BOUNDSHEET)
            .map(|(i, r)| {
                let (name, _) = short_string(&r.data, 6);
                (i, u32_at(&r.data, 0), name, r.data.get(4).map(|h| h & 3 != 0).unwrap_or(false), r.data.get(5).copied().unwrap_or(0))
            })
            .collect();
        for (bi, pos, name, hidden, dt) in bounds {
            let Some(k) = subs.iter().position(|(s, _)| *s == pos as usize) else {
                return Err(err(format!("sheet {name}: its substream is not where the workbook says")));
            };
            let recs = subs[k].1.clone();
            x.sheets.push(XSheet::parse(name, hidden, dt == 0, bi, recs));
        }
        Ok(x)
    }

    fn read_globals(&mut self) {
        let mut i = 0;
        while i < self.globals.len() {
            let r = &self.globals[i];
            match r.typ {
                SST => {
                    let mut segs: Vec<&[u8]> = vec![&r.data[8..]];
                    let mut j = i + 1;
                    while j < self.globals.len() && self.globals[j].typ == CONTINUE {
                        segs.push(&self.globals[j].data);
                        j += 1;
                    }
                    let n = u32_at(&r.data, 4) as usize;
                    let mut rd = Reader::new(segs);
                    let mut v = Vec::with_capacity(n);
                    for _ in 0..n {
                        if rd.left() == 0 && !rd.next_seg() {
                            break;
                        }
                        v.push(rd.rich_string());
                    }
                    self.sst = v;
                }
                0x0031 => {
                    let d = &r.data;
                    let (name, _) = short_string(d, 14);
                    self.fonts.push(FontInfo {
                        name,
                        size: u16_at(d, 0) as f64 / 20.0,
                        italic: u16_at(d, 2) & 0x02 != 0,
                        strike: u16_at(d, 2) & 0x08 != 0,
                        color: u16_at(d, 4),
                        bold: u16_at(d, 6) >= 700,
                        underline: d.get(10).copied().unwrap_or(0) != 0,
                    });
                }
                0x041E => {
                    let id = u16_at(&r.data, 0);
                    let (s, _) = long_string(&r.data, 2);
                    self.formats.insert(id, s);
                }
                0x00E0 => {
                    let d = &r.data;
                    let align = d.get(6).copied().unwrap_or(0);
                    let b1 = u32_at(d, 10);
                    let b2 = u32_at(d, 14);
                    let fill = u16_at(d, 18);
                    let pattern = (b2 >> 26) & 0x3F;
                    self.xfs.push(XfInfo {
                        font: u16_at(d, 0),
                        fmt: u16_at(d, 2),
                        halign: align & 7,
                        valign: (align >> 4) & 7,
                        wrap: align & 8 != 0,
                        fill: if pattern != 0 { Some(fill & 0x7F) } else { None },
                        borders: [((b1 >> 8) & 0xF) as u8, ((b1 >> 4) & 0xF) as u8, ((b1 >> 12) & 0xF) as u8, (b1 & 0xF) as u8],
                        border_colors: [(b2 & 0x7F) as u16, ((b1 >> 23) & 0x7F) as u16, ((b2 >> 7) & 0x7F) as u16, ((b1 >> 16) & 0x7F) as u16],
                        indent: d.get(8).map(|v| v & 0xF).unwrap_or(0),
                        rotation: d.get(7).copied().unwrap_or(0),
                    });
                }
                0x0092 => {
                    let n = u16_at(&r.data, 0) as usize;
                    for k in 0..n.min(56) {
                        let c = &r.data[2 + k * 4..];
                        if c.len() >= 3 {
                            self.palette[k] = format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
                        }
                    }
                }
                0x0022 => self.date1904 = u16_at(&r.data, 0) == 1,
                0x0017 => {
                    let n = u16_at(&r.data, 0) as usize;
                    for k in 0..n {
                        let o = 2 + k * 6;
                        self.xti.push((u16_at(&r.data, o), u16_at(&r.data, o + 2) as i16, u16_at(&r.data, o + 4) as i16));
                    }
                }
                0x01AE => {
                    // A SUPBOOK for this workbook ("internal") has cch 0x0401.
                    let idx = self.globals[..i].iter().filter(|x| x.typ == 0x01AE).count() as u16;
                    if u16_at(&r.data, 2) == 0x0401 {
                        self.self_supbook = Some(idx);
                    }
                }
                0x0018 => {
                    let d = &r.data;
                    let cch = d.get(3).copied().unwrap_or(0) as usize;
                    let builtin = u16_at(d, 0) & 0x20 != 0;
                    let high = d.get(14).map(|f| f & 1 != 0).unwrap_or(false);
                    let name = if builtin {
                        let code = d.get(15).copied().unwrap_or(0);
                        match code {
                            6 => "_xlnm.Print_Area".to_string(),
                            7 => "_xlnm.Print_Titles".to_string(),
                            13 => "_xlnm._FilterDatabase".to_string(),
                            c => format!("_xlnm.{c}"),
                        }
                    } else if high {
                        String::from_utf16_lossy(&(0..cch).map(|k| u16_at(d, 15 + k * 2)).collect::<Vec<_>>())
                    } else {
                        d.get(15..15 + cch).map(|b| b.iter().map(|&c| c as char).collect()).unwrap_or_default()
                    };
                    self.names.push(name);
                }
                _ => {}
            }
            i += 1;
        }
    }

    pub fn sheet_index(&self, name: &str) -> Option<usize> {
        self.sheets.iter().position(|s| s.name == name).or_else(|| self.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(name)))
    }

    pub fn sheet_names(&self) -> Vec<String> {
        self.sheets.iter().map(|s| s.name.clone()).collect()
    }

    pub fn number_format(&self, xf: u16) -> String {
        let id = self.xfs.get(xf as usize).map(|x| x.fmt).unwrap_or(0);
        if let Some(f) = self.formats.get(&id) {
            return f.clone();
        }
        numfmt::builtin(id as u32).unwrap_or("General").to_string()
    }

    pub fn font_of(&self, xf: u16) -> FontInfo {
        let f = self.xfs.get(xf as usize).map(|x| x.font as usize).unwrap_or(0);
        // Font 4 does not exist in BIFF: indexes above it are one further.
        let i = if f >= 4 { f - 1 } else { f };
        self.fonts.get(i).cloned().unwrap_or_default()
    }

    pub fn color(&self, icv: u16) -> Option<String> {
        match icv {
            8..=63 => self.palette.get((icv - 8) as usize).cloned(),
            0..=7 => DEFAULT_PALETTE.get(icv as usize).map(|s| s.to_string()),
            _ => None,
        }
    }

    pub fn display(&self, v: &Val, xf: u16) -> String {
        match v {
            Val::Num(n) => numfmt::format_number(*n, &self.number_format(xf), self.date1904),
            v => v.as_text(),
        }
    }

    // ── cells ───────────────────────────────────────────────────────────────────────────────

    /// The cells of a sheet: (row, col, value, formula text, xf).
    pub fn cells(&self, si: usize) -> Vec<(u32, u32, Val, Option<String>, u16)> {
        let sh = &self.sheets[si];
        let mut out = Vec::new();
        for ((row, col), u) in &sh.units {
            let r = &u.recs[0];
            let d = &r.data;
            match r.typ {
                MULRK => {
                    let n = (d.len() - 6) / 6;
                    for k in 0..n {
                        let xf = u16_at(d, 4 + k * 6);
                        let v = rk_value(u32_at(d, 6 + k * 6));
                        out.push((*row as u32, (*col as usize + k) as u32, Val::Num(v), None, xf));
                    }
                }
                MULBLANK => {
                    let n = (d.len() - 6) / 2;
                    for k in 0..n {
                        out.push((*row as u32, (*col as usize + k) as u32, Val::Empty, None, u16_at(d, 4 + k * 2)));
                    }
                }
                _ => {
                    let xf = u16_at(d, 4);
                    let (v, f) = self.unit_value(si, u);
                    out.push((*row as u32, *col as u32, v, f, xf));
                }
            }
        }
        out
    }

    fn unit_value(&self, si: usize, u: &Unit) -> (Val, Option<String>) {
        let r = &u.recs[0];
        let d = &r.data;
        match r.typ {
            NUMBER => (Val::Num(f64::from_le_bytes(d[6..14].try_into().unwrap_or([0; 8]))), None),
            RK => (Val::Num(rk_value(u32_at(d, 6))), None),
            LABELSST => (Val::Str(self.sst.get(u32_at(d, 6) as usize).map(|e| e.text.clone()).unwrap_or_default()), None),
            LABEL | RSTRING => (Val::Str(long_string(d, 6).0), None),
            BOOLERR => {
                if d.get(7) == Some(&1) {
                    (Val::Err(XlErr::parse(biff::err_text(d[6])).unwrap_or(XlErr::NA)), None)
                } else {
                    (Val::Bool(d.get(6) != Some(&0)), None)
                }
            }
            BLANK => (Val::Empty, None),
            FORMULA => {
                let v = if u16_at(d, 12) == 0xFFFF {
                    match d[6] {
                        0 => Val::Str(u.recs.iter().find(|x| x.typ == STRING).map(|s| long_string(&s.data, 0).0).unwrap_or_default()),
                        1 => Val::Bool(d[8] != 0),
                        2 => Val::Err(XlErr::parse(biff::err_text(d[8])).unwrap_or(XlErr::NA)),
                        _ => Val::Str(String::new()),
                    }
                } else {
                    Val::Num(f64::from_le_bytes(d[6..14].try_into().unwrap_or([0; 8])))
                };
                let f = self.formula_text(si, u);
                (v, Some(f))
            }
            _ => (Val::Empty, None),
        }
    }

    fn formula_text(&self, si: usize, u: &Unit) -> String {
        let d = &u.recs[0].data;
        let row = u16_at(d, 0) as u32;
        let col = u16_at(d, 2) as u32;
        let cce = u16_at(d, 20) as usize;
        let rgce = d.get(22..22 + cce).unwrap_or(&[]);
        match biff::decompile(rgce, (row, col), self) {
            Ok(t) => t,
            Err(e) if e.starts_with("shared:") => {
                // The shared formula of the anchor cell.
                let mut parts = e[7..].split(',');
                let ar: u16 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                let ac: u16 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                if let Some(anchor) = self.sheets[si].units.get(&(ar, ac)) {
                    if let Some(sh) = anchor.recs.iter().find(|r| r.typ == SHRFMLA || r.typ == ARRAY) {
                        let off = if sh.typ == SHRFMLA { 8 } else { 12 };
                        let cce = u16_at(&sh.data, off) as usize;
                        let rgce = sh.data.get(off + 2..off + 2 + cce).unwrap_or(&[]);
                        return biff::decompile(rgce, (row, col), self).unwrap_or_else(|e| format!("#formula?{e}"));
                    }
                }
                "#formula?".into()
            }
            Err(e) => format!("#formula?{e}"),
        }
    }

    pub fn addr_of(&self, si: usize, r: u32, c: u32) -> String {
        format!("{}!{}", formula::quote_sheet(&self.sheets[si].name), formula::cell_name(r, c))
    }

    pub fn resolve(&self, addr: &str) -> Result<(usize, Area)> {
        let a = addr.trim();
        if let Some(i) = self.sheet_index(a.trim_matches('\'')) {
            return Ok((i, Area { sheet: None, a: CellRef { row: 0, col: 0, abs_row: false, abs_col: false }, b: CellRef { row: 65535, col: 255, abs_row: false, abs_col: false }, whole: None }));
        }
        let area = formula::parse_area(a).ok_or_else(|| err(format!("bad cell address {addr:?} (Sheet1!B3, B3, A1:C9)")))?;
        let si = match &area.sheet {
            Some(s) => self.sheet_index(s).ok_or_else(|| err(format!("no sheet {s:?}; the sheets are: {}", self.sheet_names().join(", "))))?,
            None => self.sheets.iter().position(|s| s.worksheet && !s.hidden).unwrap_or(0),
        };
        if !self.sheets[si].worksheet {
            return Err(err(format!("{} is not a worksheet", self.sheets[si].name)));
        }
        let mut area = area;
        if area.whole.is_some() {
            // Whole rows or columns: as far as a sheet goes.
            area.b.row = area.b.row.min(65535);
            area.b.col = area.b.col.min(255);
        }
        let (r1, c1) = (area.rows().1, area.cols().1);
        if r1 > 65535 || c1 > 255 {
            return Err(err(format!("{addr}: an .xls sheet has 65536 rows and 256 columns")));
        }
        Ok((si, area))
    }

    pub fn outline(&self, _runs: bool, filter: &[String]) -> Result<Outline> {
        let mut items = Vec::new();
        let wanted: Vec<(usize, Option<Area>)> = if filter.is_empty() {
            (0..self.sheets.len()).map(|i| (i, None)).collect()
        } else {
            filter.iter().map(|f| self.resolve(f).map(|(s, a)| (s, if self.sheet_index(f.trim_matches('\'')).is_some() { None } else { Some(a) }))).collect::<Result<_>>()?
        };
        for (si, area) in wanted {
            let sh = &self.sheets[si];
            if !sh.worksheet {
                items.push(Item { addr: formula::quote_sheet(&sh.name), kind: "sheet".into(), text: "chart sheet".into(), ..Default::default() });
                continue;
            }
            let cells = self.cells(si);
            let (mr, mc) = cells.iter().filter(|c| !matches!(c.2, Val::Empty)).fold((0, 0), |a, c| (a.0.max(c.0), a.1.max(c.1)));
            let mut it = Item { addr: formula::quote_sheet(&sh.name), kind: "sheet".into(), text: format!("A1:{}", formula::cell_name(mr, mc)), ..Default::default() };
            if sh.hidden {
                it.extra.push(("hidden".into(), String::new()));
            }
            items.push(it);
            for (r, c, v, f, xf) in cells {
                if let Some(a) = &area {
                    let (r0, r1) = a.rows();
                    let (c0, c1) = a.cols();
                    if r < r0 || r > r1 || c < c0 || c > c1 {
                        continue;
                    }
                }
                if matches!(v, Val::Empty) && f.is_none() {
                    continue;
                }
                let shown = self.display(&v, xf);
                let mut it = Item { addr: self.addr_of(si, r, c), kind: "cell".into(), text: shown.clone(), depth: 1, ..Default::default() };
                if let Some(f) = f {
                    it.extra.push(("formula".into(), format!("={f}")));
                }
                let raw = match &v {
                    Val::Num(n) => numfmt::general(*n),
                    v => v.as_text(),
                };
                if raw != shown {
                    it.extra.push(("value".into(), raw));
                }
                items.push(it);
            }
        }
        Ok(Outline { items })
    }

    pub fn plain(&self) -> String {
        let mut s = String::new();
        for si in 0..self.sheets.len() {
            if !self.sheets[si].worksheet {
                continue;
            }
            s.push_str(&format!("== {}\n", self.sheets[si].name));
            let mut grid: HashMap<(u32, u32), String> = HashMap::new();
            let (mut er, mut ec) = (0, 0);
            for (r, c, v, _, xf) in self.cells(si) {
                if matches!(v, Val::Empty) {
                    continue;
                }
                er = er.max(r);
                ec = ec.max(c);
                grid.insert((r, c), self.display(&v, xf));
            }
            if grid.is_empty() {
                continue;
            }
            for r in 0..=er {
                let line: Vec<String> = (0..=ec).map(|c| grid.get(&(r, c)).cloned().unwrap_or_default().replace(['\t', '\n'], " ")).collect();
                s.push_str(line.join("\t").trim_end());
                s.push('\n');
            }
        }
        s
    }

    pub fn find(&self, f: &Finder, scope: Option<&str>, formulas: bool) -> Result<Vec<Match>> {
        let mut out = Vec::new();
        for (si, area) in self.scope(scope)? {
            for (r, c, v, fm, xf) in self.cells(si) {
                if let Some(a) = &area {
                    if !in_area(a, r, c) {
                        continue;
                    }
                }
                let t = match (&fm, formulas) {
                    (Some(f), true) => format!("={f}"),
                    _ => self.display(&v, xf),
                };
                for (a, b, _) in f.find(&t, None) {
                    out.push(Match { addr: self.addr_of(si, r, c), start: a, end: b, text: t.chars().skip(a).take(b - a).collect(), context: text::escape_line(&t) });
                }
            }
        }
        Ok(out)
    }

    fn scope(&self, scope: Option<&str>) -> Result<Vec<(usize, Option<Area>)>> {
        Ok(match scope {
            None => (0..self.sheets.len()).filter(|&i| self.sheets[i].worksheet).map(|i| (i, None)).collect(),
            Some(s) => {
                let (si, a) = self.resolve(s)?;
                vec![(si, if self.sheet_index(s.trim_matches('\'')).is_some() { None } else { Some(a) })]
            }
        })
    }

    /// Replaces text in string cells (the shared strings they use; forked when other cells
    /// outside the scope use the same string).
    pub fn replace(&mut self, f: &Finder, with: &str, scope: Option<&str>, nth: Option<usize>) -> Result<Vec<String>> {
        let with = text::unescape(with);
        let mut uses: HashMap<u32, usize> = HashMap::new();
        for sh in &self.sheets {
            for u in sh.units.values() {
                if u.recs[0].typ == LABELSST {
                    *uses.entry(u32_at(&u.recs[0].data, 6)).or_insert(0) += 1;
                }
            }
        }
        let mut targets: Vec<(usize, u16, u16, Option<usize>)> = Vec::new();
        let mut count = 0;
        for (si, area) in self.scope(scope)? {
            for (r, c, v, fm, _) in self.cells(si) {
                if fm.is_some() {
                    continue;
                }
                let Val::Str(t) = &v else { continue };
                if let Some(a) = &area {
                    if !in_area(a, r, c) {
                        continue;
                    }
                }
                let n = f.find(t, None).len();
                if n == 0 {
                    continue;
                }
                match nth {
                    Some(k) if count < k && k <= count + n => targets.push((si, r as u16, c as u16, Some(k - count))),
                    Some(_) => {}
                    None => targets.push((si, r as u16, c as u16, None)),
                }
                count += n;
            }
        }
        let mut mine: HashMap<u32, usize> = HashMap::new();
        for (si, r, c, _) in &targets {
            if let Some(u) = self.sheets[*si].units.get(&(*r, *c)) {
                if u.recs[0].typ == LABELSST {
                    *mine.entry(u32_at(&u.recs[0].data, 6)).or_insert(0) += 1;
                }
            }
        }
        let mut done = Vec::new();
        let mut edited: HashMap<u32, u32> = HashMap::new();
        for (si, r, c, local) in targets {
            let addr = self.addr_of(si, r as u32, c as u32);
            let u = self.sheets[si].units.get(&(r, c)).cloned().unwrap();
            match u.recs[0].typ {
                LABELSST => {
                    let i = u32_at(&u.recs[0].data, 6);
                    let new_index = if let Some(&ni) = edited.get(&i) {
                        ni
                    } else {
                        let old = self.sst[i as usize].text.clone();
                        let new = replace_text(f, &old, &with, local);
                        if mine.get(&i).copied().unwrap_or(0) >= uses.get(&i).copied().unwrap_or(0) && local.is_none() {
                            self.sst[i as usize] = SstEntry { text: new, ..Default::default() };
                            edited.insert(i, i);
                            i
                        } else {
                            self.sst.push(SstEntry { text: new, ..Default::default() });
                            let ni = (self.sst.len() - 1) as u32;
                            if local.is_none() {
                                edited.insert(i, ni);
                            }
                            ni
                        }
                    };
                    self.sst_dirty = true;
                    if new_index != i {
                        let sh = &mut self.sheets[si];
                        let unit = sh.units.get_mut(&(r, c)).unwrap();
                        unit.recs[0].data[6..10].copy_from_slice(&new_index.to_le_bytes());
                        sh.changed = true;
                    }
                }
                LABEL | RSTRING => {
                    let (old, _) = long_string(&u.recs[0].data, 6);
                    let new = replace_text(f, &old, &with, local);
                    let idx = self.add_string(&new);
                    let xf = u16_at(&u.recs[0].data, 4);
                    self.put_unit(si, r, c, label_sst(r, c, xf, idx));
                }
                _ => continue,
            }
            self.changed.insert((si, r as u32, c as u32));
            done.push(addr);
        }
        Ok(done)
    }

    fn add_string(&mut self, s: &str) -> u32 {
        self.sst_dirty = true;
        if let Some(i) = self.sst.iter().position(|e| e.text == s && e.runs.is_empty() && e.ext.is_empty()) {
            return i as u32;
        }
        self.sst.push(SstEntry { text: s.to_string(), ..Default::default() });
        (self.sst.len() - 1) as u32
    }

    /// Puts a cell's records at (r, c), splitting a multi-cell record that covers it.
    fn put_unit(&mut self, si: usize, r: u16, c: u16, unit: Unit) {
        let sh = &mut self.sheets[si];
        sh.changed = true;
        // A MULRK/MULBLANK in the row covering c: split it.
        let covering = sh.units.range((r, 0)..=(r, c)).rev().find(|(_, u)| u.last_col >= c).map(|(k, _)| *k);
        if let Some(k) = covering {
            if k.1 != c || sh.units[&k].last_col != c {
                let multi = sh.units.remove(&k).unwrap();
                for (col, part) in split_multi(&multi, k.0, k.1) {
                    if col != c {
                        sh.units.insert((r, col), part);
                    }
                }
            } else {
                sh.units.remove(&k);
            }
        }
        if unit.recs.is_empty() {
            sh.units.remove(&(r, c));
        } else {
            sh.units.insert((r, c), unit);
        }
        if !sh.rows.contains_key(&r) {
            let mut row = vec![0u8; 16];
            row[0..2].copy_from_slice(&r.to_le_bytes());
            row[6..8].copy_from_slice(&0x00FFu16.to_le_bytes());
            row[12..14].copy_from_slice(&0x0100u16.to_le_bytes());
            row[14..16].copy_from_slice(&0x000Fu16.to_le_bytes());
            sh.rows.insert(r, row);
        }
    }

    pub fn set(&mut self, addr: &str, value: &str) -> Result<String> {
        let (si, area) = self.resolve(addr)?;
        let rows: Vec<Vec<String>> = if value.contains(['|', ';', '\n']) && !value.starts_with('=') {
            value.split(['\n', ';']).map(|r| r.split('|').map(|c| c.to_string()).collect()).collect()
        } else {
            vec![vec![value.to_string()]]
        };
        let (r0, c0) = (area.rows().0, area.cols().0);
        let (r1, c1) = if area.single() { (r0 + rows.len() as u32 - 1, c0 + rows.iter().map(|r| r.len()).max().unwrap_or(1) as u32 - 1) } else { (area.rows().1, area.cols().1) };
        let one = rows.len() == 1 && rows[0].len() == 1;
        for r in r0..=r1 {
            for c in c0..=c1 {
                let v = if one { rows[0][0].clone() } else { match rows.get((r - r0) as usize).and_then(|x| x.get((c - c0) as usize)) { Some(v) => v.clone(), None => continue } };
                let v = if one && v.starts_with('=') && (r, c) != (r0, c0) { format!("={}", formula::shift(&v[1..], (r - r0) as i64, (c - c0) as i64)) } else { v };
                self.set_cell(si, r as u16, c as u16, &v)?;
            }
        }
        Ok(self.addr_of(si, r0, c0))
    }

    pub fn set_cell(&mut self, si: usize, r: u16, c: u16, value: &str) -> Result<()> {
        let xf = self.cells(si).iter().find(|x| x.0 == r as u32 && x.1 == c as u32).map(|x| x.4).unwrap_or(15);
        let unit = if value.is_empty() {
            Unit { recs: vec![Rec { typ: BLANK, data: [r.to_le_bytes(), c.to_le_bytes(), xf.to_le_bytes()].concat() }], last_col: c }
        } else if let Some(t) = value.strip_prefix('\'') {
            let i = self.add_string(t);
            label_sst(r, c, xf, i)
        } else if value.starts_with('=') && value.len() > 1 {
            let names: Vec<(String, u16)> = self.sheet_xti_map();
            let rgce = biff::compile(value, &|s: &str| names.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|(_, i)| *i)).map_err(|e| err(format!("{value}: {e}")))?;
            let mut d = Vec::new();
            d.extend_from_slice(&r.to_le_bytes());
            d.extend_from_slice(&c.to_le_bytes());
            d.extend_from_slice(&xf.to_le_bytes());
            d.extend_from_slice(&0f64.to_le_bytes());
            d.extend_from_slice(&0x0002u16.to_le_bytes());
            d.extend_from_slice(&0u32.to_le_bytes());
            d.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
            d.extend_from_slice(&rgce);
            Unit { recs: vec![Rec { typ: FORMULA, data: d }], last_col: c }
        } else if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false") {
            Unit { recs: vec![Rec { typ: BOOLERR, data: [&r.to_le_bytes()[..], &c.to_le_bytes(), &xf.to_le_bytes(), &[value.eq_ignore_ascii_case("true") as u8, 0]].concat() }], last_col: c }
        } else if let Some(n) = value.trim().parse::<f64>().ok().or_else(|| formula::parse_date_time(value).filter(|_| value.contains(['-', '/', ':']))) {
            Unit { recs: vec![Rec { typ: NUMBER, data: [&r.to_le_bytes()[..], &c.to_le_bytes(), &xf.to_le_bytes(), &n.to_le_bytes()].concat() }], last_col: c }
        } else {
            let i = self.add_string(value);
            label_sst(r, c, xf, i)
        };
        self.put_unit(si, r, c, unit);
        self.changed.insert((si, r as u32, c as u32));
        Ok(())
    }

    /// EXTERNSHEET indexes of this workbook's sheets (for 3-D references).
    fn sheet_xti_map(&self) -> Vec<(String, u16)> {
        let mut out = Vec::new();
        for (i, (sb, first, last)) in self.xti.iter().enumerate() {
            if Some(*sb) == self.self_supbook && first == last && *first >= 0 {
                if let Some(s) = self.sheets.get(*first as usize) {
                    out.push((s.name.clone(), i as u16));
                }
            }
        }
        out
    }

    pub fn delete(&mut self, addr: &str) -> Result<String> {
        let (si, area) = self.resolve(addr)?;
        let cells: Vec<(u32, u32)> = self.cells(si).iter().filter(|c| in_area(&area, c.0, c.1)).map(|c| (c.0, c.1)).collect();
        for (r, c) in &cells {
            self.set_cell(si, *r as u16, *c as u16, "")?;
        }
        Ok(format!("cleared {} cell{}", cells.len(), if cells.len() == 1 { "" } else { "s" }))
    }

    // ── calculation ─────────────────────────────────────────────────────────────────────────

    pub fn recalc(&mut self, all: bool) -> usize {
        if !all && self.changed.is_empty() {
            return 0;
        }
        let mut formulas: Vec<(usize, u32, u32, String)> = Vec::new();
        for si in 0..self.sheets.len() {
            if !self.sheets[si].worksheet {
                continue;
            }
            for (r, c, _, f, _) in self.cells(si) {
                if let Some(f) = f {
                    if !f.starts_with("#formula?") {
                        formulas.push((si, r, c, f));
                    }
                }
            }
        }
        let refs: Vec<Vec<(usize, Area)>> = formulas
            .iter()
            .map(|(si, _, _, f)| {
                formula::tokenize(f)
                    .map(|toks| {
                        toks.into_iter()
                            .filter_map(|t| match t.kind {
                                formula::TokKind::Area(a) => Some((a.sheet.as_ref().and_then(|n| self.sheet_index(n)).unwrap_or(*si), a)),
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect();
        let mut dirty: HashSet<(usize, u32, u32)> = HashSet::new();
        if all {
            dirty = formulas.iter().map(|f| (f.0, f.1, f.2)).collect();
        } else {
            let mut frontier: Vec<(usize, u32, u32)> = self.changed.iter().copied().collect();
            for f in &formulas {
                if self.changed.contains(&(f.0, f.1, f.2)) {
                    dirty.insert((f.0, f.1, f.2));
                }
            }
            let mut seen: HashSet<(usize, u32, u32)> = frontier.iter().copied().collect();
            while let Some((s, r, c)) = frontier.pop() {
                for (k, f) in formulas.iter().enumerate() {
                    if refs[k].iter().any(|(fs, a)| *fs == s && in_area(a, r, c)) {
                        let key = (f.0, f.1, f.2);
                        dirty.insert(key);
                        if seen.insert(key) {
                            frontier.push(key);
                        }
                    }
                }
            }
        }
        let mut book = XlsBook::new(self, dirty.clone());
        let mut results = Vec::new();
        let mut keys: Vec<_> = dirty.into_iter().collect();
        keys.sort();
        for (s, r, c) in keys {
            results.push((s, r, c, book.value(s, r, c)));
        }
        let unsupported = book.unsupported.clone();
        drop(book);
        let mut n = 0;
        for (s, r, c, v) in results {
            if v == Val::Err(XlErr::Name) && !unsupported.is_empty() {
                self.warnings.push(format!("{}: uses a function this tool does not calculate ({}); Excel recalculates on open", self.addr_of(s, r, c), unsupported.iter().cloned().collect::<Vec<_>>().join(", ")));
                continue;
            }
            self.write_cached(s, r as u16, c as u16, &v);
            n += 1;
        }
        // Excel recalculates sheets marked uncalculated when it opens them.
        for sh in self.sheets.iter_mut().filter(|s| s.changed && s.worksheet) {
            if !sh.before.iter().any(|r| r.typ == UNCALCED) {
                sh.before.insert(1, Rec { typ: UNCALCED, data: vec![0, 0] });
            }
        }
        self.changed.clear();
        n
    }

    fn write_cached(&mut self, si: usize, r: u16, c: u16, v: &Val) {
        let sh = &mut self.sheets[si];
        let Some(u) = sh.units.get_mut(&(r, c)) else { return };
        if u.recs[0].typ != FORMULA {
            return;
        }
        u.recs.retain(|x| x.typ != STRING);
        let mut bytes = [0u8; 8];
        let mut string = None;
        match v {
            Val::Num(n) => bytes = n.to_le_bytes(),
            Val::Str(s) => {
                bytes = [0, 0, 0, 0, 0, 0, 0xFF, 0xFF];
                if s.is_empty() {
                    bytes[0] = 3;
                } else {
                    string = Some(s.clone());
                }
            }
            Val::Bool(b) => bytes = [1, 0, *b as u8, 0, 0, 0, 0xFF, 0xFF],
            Val::Err(e) => bytes = [2, 0, biff::err_code(e), 0, 0, 0, 0xFF, 0xFF],
            Val::Empty => bytes = [3, 0, 0, 0, 0, 0, 0xFF, 0xFF],
            Val::Arr(_) => {}
        }
        let d = &mut u.recs[0].data;
        d[6..14].copy_from_slice(&bytes);
        if let Some(s) = string {
            let at = u.recs.iter().position(|x| x.typ == SHRFMLA || x.typ == ARRAY || x.typ == TABLE).map(|p| p + 1).unwrap_or(1);
            u.recs.insert(at, Rec { typ: STRING, data: encode_long_string(&s) });
        }
        sh.changed = true;
    }

    // ── writing ─────────────────────────────────────────────────────────────────────────────

    fn sst_records(&self, base: usize) -> Vec<Rec> {
        // SST and CONTINUE records of at most 8224 bytes; EXTSST with a bucket per 8 strings.
        const MAX: usize = 8224;
        let total: u32 = self.sheets.iter().flat_map(|s| s.units.values()).filter(|u| u.recs[0].typ == LABELSST).count() as u32;
        let mut recs: Vec<Vec<u8>> = vec![[total.to_le_bytes(), (self.sst.len() as u32).to_le_bytes()].concat()];
        let bucket = ((self.sst.len() / 128) + 1).max(8);
        let mut ext: Vec<u8> = (bucket as u16).to_le_bytes().to_vec();
        let mut abs_before: usize = base; // stream offset of the current record's start
        for (i, e) in self.sst.iter().enumerate() {
            let units: Vec<u16> = e.text.encode_utf16().collect();
            let high = units.iter().any(|u| *u > 255);
            let mut head = Vec::new();
            head.extend_from_slice(&(units.len() as u16).to_le_bytes());
            let mut flags = if high { 1u8 } else { 0 };
            if !e.runs.is_empty() {
                flags |= 0x08;
            }
            if !e.ext.is_empty() {
                flags |= 0x04;
            }
            head.push(flags);
            if !e.runs.is_empty() {
                head.extend_from_slice(&(e.runs.len() as u16).to_le_bytes());
            }
            if !e.ext.is_empty() {
                head.extend_from_slice(&(e.ext.len() as u32).to_le_bytes());
            }
            let w = if high { 2 } else { 1 };
            // The header and at least one character must fit in the record.
            if recs.last().unwrap().len() + head.len() + w > MAX {
                abs_before += 4 + recs.last().unwrap().len();
                recs.push(Vec::new());
            }
            if i % bucket == 0 {
                let cur = recs.last().unwrap();
                let ib = abs_before + 4 + cur.len();
                ext.extend_from_slice(&(ib as u32).to_le_bytes());
                ext.extend_from_slice(&((4 + cur.len()) as u16).to_le_bytes());
                ext.extend_from_slice(&0u16.to_le_bytes());
            }
            recs.last_mut().unwrap().extend_from_slice(&head);
            // Characters, continued in the next record with a new flag byte when they overflow.
            let mut k = 0;
            while k < units.len() {
                let room = (MAX - recs.last().unwrap().len()) / w;
                if room == 0 {
                    abs_before += 4 + recs.last().unwrap().len();
                    recs.push(vec![if high { 1 } else { 0 }]);
                    continue;
                }
                let n = room.min(units.len() - k);
                let cur = recs.last_mut().unwrap();
                for u in &units[k..k + n] {
                    if high {
                        cur.extend_from_slice(&u.to_le_bytes());
                    } else {
                        cur.push(*u as u8);
                    }
                }
                k += n;
            }
            let mut tail = Vec::new();
            for (ich, ifnt) in &e.runs {
                tail.extend_from_slice(&ich.to_le_bytes());
                tail.extend_from_slice(&ifnt.to_le_bytes());
            }
            tail.extend_from_slice(&e.ext);
            let mut t = 0;
            while t < tail.len() {
                let room = MAX - recs.last().unwrap().len();
                if room == 0 {
                    abs_before += 4 + recs.last().unwrap().len();
                    recs.push(Vec::new());
                    continue;
                }
                let n = room.min(tail.len() - t);
                recs.last_mut().unwrap().extend_from_slice(&tail[t..t + n]);
                t += n;
            }
        }
        let mut out: Vec<Rec> = recs.into_iter().enumerate().map(|(i, d)| Rec { typ: if i == 0 { SST } else { CONTINUE }, data: d }).collect();
        out.push(Rec { typ: EXTSST, data: ext });
        out
    }

    pub fn save(&mut self) -> Result<Vec<u8>> {
        self.recalc(false);
        // Globals, with the string table written again when it changed.
        let mut globals: Vec<Rec> = Vec::new();
        let mut bs_new: Vec<usize> = Vec::new();
        let mut i = 0;
        let mut pos = 0usize;
        while i < self.globals.len() {
            let r = &self.globals[i];
            if r.typ == SST && self.sst_dirty {
                let recs = self.sst_records(pos);
                for x in recs {
                    pos += 4 + x.data.len();
                    globals.push(x);
                }
                i += 1;
                while i < self.globals.len() && (self.globals[i].typ == CONTINUE || self.globals[i].typ == EXTSST) {
                    i += 1;
                }
                continue;
            }
            if r.typ == BOUNDSHEET {
                bs_new.push(globals.len());
            }
            pos += 4 + r.data.len();
            globals.push(r.clone());
            i += 1;
        }
        if self.sst_dirty && !self.globals.iter().any(|r| r.typ == SST) {
            // No table yet: one before the EOF.
            let at = globals.len() - 1;
            let base: usize = globals[..at].iter().map(|r| 4 + r.data.len()).sum();
            let recs = self.sst_records(base);
            for (k, x) in recs.into_iter().enumerate() {
                globals.insert(at + k, x);
            }
        }
        let globals_len: usize = globals.iter().map(|r| 4 + r.data.len()).sum();
        // Sheets.
        let mut out_sheets: Vec<Vec<u8>> = Vec::new();
        let mut offsets = Vec::new();
        let mut at = globals_len;
        for sh in &self.sheets {
            offsets.push(at);
            let bytes = sh.serialize(at);
            at += bytes.len();
            out_sheets.push(bytes);
        }
        // BOUNDSHEET positions.
        let mut bs_iter = bs_new.iter();
        for (k, sh) in self.sheets.iter().enumerate() {
            let _ = sh;
            if let Some(&gi) = bs_iter.next() {
                globals[gi].data[0..4].copy_from_slice(&(offsets[k] as u32).to_le_bytes());
            }
        }
        let mut stream = Vec::with_capacity(at);
        for r in &globals {
            stream.extend_from_slice(&r.typ.to_le_bytes());
            stream.extend_from_slice(&(r.data.len() as u16).to_le_bytes());
            stream.extend_from_slice(&r.data);
        }
        for s in out_sheets {
            stream.extend_from_slice(&s);
        }
        let name = self.stream.clone();
        self.cfb.set_stream(&name, stream)?;
        Ok(self.cfb.save())
    }
}

/// What a sheet's layout records say: column widths, row heights, merged cells.
pub struct Layout {
    /// (first col, last col, width in characters, hidden)
    pub cols: Vec<(u16, u16, f64, bool)>,
    /// row → (height in points if custom, hidden)
    pub rows: Vec<(u16, Option<f64>, bool)>,
    pub merges: Vec<(u16, u16, u16, u16)>,
    pub default_width: Option<f64>,
}

impl Xls {
    pub fn layout(&self, si: usize) -> Layout {
        let sh = &self.sheets[si];
        let mut l = Layout { cols: Vec::new(), rows: Vec::new(), merges: Vec::new(), default_width: None };
        for r in sh.before.iter().chain(sh.after.iter()) {
            let d = &r.data;
            match r.typ {
                0x007D => l.cols.push((u16_at(d, 0), u16_at(d, 2), u16_at(d, 4) as f64 / 256.0, u16_at(d, 8) & 1 != 0)),
                0x0055 => l.default_width = Some(u16_at(d, 0) as f64 + 0.71),
                0x0099 => l.default_width = Some(u16_at(d, 0) as f64 / 256.0),
                0x00E5 => {
                    let n = u16_at(d, 0) as usize;
                    for k in 0..n {
                        let o = 2 + k * 8;
                        l.merges.push((u16_at(d, o), u16_at(d, o + 2), u16_at(d, o + 4), u16_at(d, o + 6)));
                    }
                }
                _ => {}
            }
        }
        for (r, d) in &sh.rows {
            let h = u16_at(d, 6);
            let flags = u16_at(d, 12);
            let custom = flags & 0x40 != 0;
            l.rows.push((*r, if custom { Some((h & 0x7FFF) as f64 / 20.0) } else { None }, flags & 0x20 != 0));
        }
        l
    }
}

impl biff::Names for Xls {
    fn sheet_of_xti(&self, ixti: u16) -> Option<String> {
        let (sb, first, _) = self.xti.get(ixti as usize)?;
        if Some(*sb) == self.self_supbook {
            self.sheets.get(*first as usize).map(|s| s.name.clone())
        } else {
            Some(format!("[{sb}]Sheet{first}"))
        }
    }
    fn name(&self, index: u32) -> Option<String> {
        self.names.get(index.checked_sub(1)? as usize).cloned()
    }
}

fn label_sst(r: u16, c: u16, xf: u16, i: u32) -> Unit {
    Unit { recs: vec![Rec { typ: LABELSST, data: [&r.to_le_bytes()[..], &c.to_le_bytes(), &xf.to_le_bytes(), &i.to_le_bytes()].concat() }], last_col: c }
}

/// A MULRK or MULBLANK as single-cell records, by column.
fn split_multi(u: &Unit, r: u16, c0: u16) -> Vec<(u16, Unit)> {
    let d = &u.recs[0].data;
    let mut out = Vec::new();
    if u.recs[0].typ == MULRK {
        let n = (d.len() - 6) / 6;
        for k in 0..n {
            let c = c0 + k as u16;
            let mut nd = Vec::new();
            nd.extend_from_slice(&r.to_le_bytes());
            nd.extend_from_slice(&c.to_le_bytes());
            nd.extend_from_slice(&d[4 + k * 6..10 + k * 6]);
            out.push((c, Unit { recs: vec![Rec { typ: RK, data: nd }], last_col: c }));
        }
    } else if u.recs[0].typ == MULBLANK {
        let n = (d.len() - 6) / 2;
        for k in 0..n {
            let c = c0 + k as u16;
            let mut nd = Vec::new();
            nd.extend_from_slice(&r.to_le_bytes());
            nd.extend_from_slice(&c.to_le_bytes());
            nd.extend_from_slice(&d[4 + k * 2..6 + k * 2]);
            out.push((c, Unit { recs: vec![Rec { typ: BLANK, data: nd }], last_col: c }));
        }
    } else {
        out.push((c0, u.clone()));
    }
    out
}

fn replace_text(f: &Finder, s: &str, with: &str, nth: Option<usize>) -> String {
    let ms = f.find(s, Some(with));
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut last = 0;
    for (i, (a, b, rep)) in ms.iter().enumerate() {
        if nth.map(|n| n != i + 1).unwrap_or(false) {
            continue;
        }
        out.extend(&chars[last..*a]);
        out.push_str(rep);
        last = *b;
    }
    out.extend(&chars[last..]);
    out
}

fn in_area(a: &Area, r: u32, c: u32) -> bool {
    let (r0, r1) = a.rows();
    let (c0, c1) = a.cols();
    r >= r0 && r <= r1 && c >= c0 && c <= c1
}

impl XSheet {
    fn parse(name: String, hidden: bool, worksheet: bool, bs_index: usize, recs: Vec<Rec>) -> XSheet {
        let mut sh = XSheet { name, hidden, worksheet, bs_index, before: Vec::new(), rows: BTreeMap::new(), units: BTreeMap::new(), after: Vec::new(), changed: false };
        if !worksheet {
            sh.before = recs;
            return sh;
        }
        // before → cell table → after
        let mut state = 0;
        let mut last: Option<(u16, u16)> = None;
        for r in recs {
            let in_table = r.typ == ROW || is_cell(r.typ) || matches!(r.typ, STRING | SHRFMLA | ARRAY | TABLE | DBCELL);
            if state == 0 && (r.typ == ROW || is_cell(r.typ)) {
                state = 1;
            }
            if state == 1 && !in_table {
                // CONTINUE after a STRING belongs to it.
                if r.typ == CONTINUE {
                    if let Some(k) = last {
                        if let Some(u) = sh.units.get_mut(&k) {
                            u.recs.push(r);
                            continue;
                        }
                    }
                }
                state = 2;
            }
            match state {
                0 => sh.before.push(r),
                1 => match r.typ {
                    ROW => {
                        sh.rows.insert(u16_at(&r.data, 0), r.data);
                    }
                    DBCELL => {}
                    t if is_cell(t) => {
                        let row = u16_at(&r.data, 0);
                        let col = u16_at(&r.data, 2);
                        let last_col = match t {
                            MULRK | MULBLANK => u16_at(&r.data, r.data.len() - 2),
                            _ => col,
                        };
                        sh.units.insert((row, col), Unit { recs: vec![r], last_col });
                        last = Some((row, col));
                    }
                    _ => {
                        if let Some(k) = last {
                            if let Some(u) = sh.units.get_mut(&k) {
                                u.recs.push(r);
                            }
                        }
                    }
                },
                _ => sh.after.push(r),
            }
        }
        sh
    }

    /// The substream at stream offset `base`: records before the cell table (INDEX and
    /// DIMENSIONS updated), the cell table in blocks of 32 rows with their DBCELLs, the rest.
    fn serialize(&self, base: usize) -> Vec<u8> {
        let put = |out: &mut Vec<u8>, typ: u16, data: &[u8]| {
            out.extend_from_slice(&typ.to_le_bytes());
            out.extend_from_slice(&(data.len() as u16).to_le_bytes());
            out.extend_from_slice(data);
        };
        if !self.worksheet || (!self.changed && self.rows.is_empty() && self.units.is_empty()) {
            let mut out = Vec::new();
            for r in self.before.iter().chain(self.after.iter()) {
                put(&mut out, r.typ, &r.data);
            }
            return out;
        }
        // Rows in blocks of 32.
        let row_keys: Vec<u16> = self.rows.keys().copied().collect();
        let blocks: Vec<&[u16]> = row_keys.chunks(32).collect();
        // The cell table, laid out relative to its start, and the DBCELL offsets.
        let mut table = Vec::new();
        let mut dbcells = Vec::new();
        for b in &blocks {
            let first_row_at = table.len();
            for r in b.iter() {
                let mut d = self.rows[r].clone();
                // The row's cell span.
                let cols: Vec<(u16, u16)> = self.units.range((*r, 0)..=(*r, u16::MAX)).map(|(k, u)| (k.1, u.last_col)).collect();
                if let (Some(first), Some(last)) = (cols.first(), cols.last()) {
                    d[2..4].copy_from_slice(&first.0.to_le_bytes());
                    d[4..6].copy_from_slice(&(last.1 + 1).to_le_bytes());
                }
                put(&mut table, ROW, &d);
            }
            let rows_end = table.len();
            let mut offs: Vec<u16> = Vec::new();
            let mut prev = rows_end - if b.len() > 1 { 0 } else { 0 };
            // The first offset is from the start of the second ROW record (as Excel writes it).
            let mut first = true;
            for r in b.iter() {
                let start = table.len();
                let mut any = false;
                for (_, u) in self.units.range((*r, 0)..=(*r, u16::MAX)) {
                    for rec in &u.recs {
                        put(&mut table, rec.typ, &rec.data);
                    }
                    any = true;
                }
                if any {
                    let off = if first { start - (first_row_at + 20.min(rows_end - first_row_at)) } else { start - prev };
                    offs.push(off as u16);
                    prev = start;
                    first = false;
                }
            }
            let db_at = table.len();
            let mut d = Vec::new();
            d.extend_from_slice(&((db_at - first_row_at) as u32).to_le_bytes());
            for o in offs {
                d.extend_from_slice(&o.to_le_bytes());
            }
            dbcells.push(db_at);
            put(&mut table, DBCELL, &d);
        }
        // Before: INDEX (sized by the block count) and DIMENSIONS.
        let (mut rmin, mut rmax, mut cmin, mut cmax) = (u32::MAX, 0u32, u16::MAX, 0u16);
        for ((r, c), u) in &self.units {
            rmin = rmin.min(*r as u32);
            rmax = rmax.max(*r as u32 + 1);
            cmin = cmin.min(*c);
            cmax = cmax.max(u.last_col + 1);
        }
        if rmin == u32::MAX {
            rmin = 0;
            cmin = 0;
        }
        let mut before = Vec::new();
        let mut index_at = None;
        let mut defcol_at = None;
        for r in &self.before {
            match r.typ {
                INDEX => {
                    index_at = Some(before.len());
                    let n = blocks.len();
                    let mut d = vec![0u8; 16 + 4 * n];
                    d[4..8].copy_from_slice(&row_keys.first().map(|r| *r as u32).unwrap_or(0).to_le_bytes());
                    d[8..12].copy_from_slice(&row_keys.last().map(|r| *r as u32 + 1).unwrap_or(0).to_le_bytes());
                    put(&mut before, INDEX, &d);
                }
                DIMENSIONS => {
                    let mut d = r.data.clone();
                    if d.len() >= 12 {
                        d[0..4].copy_from_slice(&rmin.to_le_bytes());
                        d[4..8].copy_from_slice(&rmax.to_le_bytes());
                        d[8..10].copy_from_slice(&cmin.to_le_bytes());
                        d[10..12].copy_from_slice(&cmax.to_le_bytes());
                    }
                    put(&mut before, DIMENSIONS, &d);
                }
                DEFCOLWIDTH => {
                    defcol_at = Some(before.len());
                    put(&mut before, r.typ, &r.data);
                }
                _ => put(&mut before, r.typ, &r.data),
            }
        }
        if let Some(ix) = index_at {
            let table_base = base + before.len();
            let d = &mut before[ix + 4..];
            if let Some(dc) = defcol_at {
                d[12..16].copy_from_slice(&((base + dc) as u32).to_le_bytes());
            }
            for (k, db) in dbcells.iter().enumerate() {
                d[16 + k * 4..20 + k * 4].copy_from_slice(&((table_base + db) as u32).to_le_bytes());
            }
        }
        let mut out = before;
        out.extend_from_slice(&table);
        for r in &self.after {
            put(&mut out, r.typ, &r.data);
        }
        out
    }
}

/// The workbook as formulas see it.
struct XlsBook<'a> {
    x: &'a Xls,
    raws: Vec<Option<HashMap<(u32, u32), (Val, Option<String>)>>>,
    cache: HashMap<(usize, u32, u32), Val>,
    computing: HashSet<(usize, u32, u32)>,
    stack: Vec<(usize, u32, u32)>,
    dirty: HashSet<(usize, u32, u32)>,
    unsupported: HashSet<String>,
}

impl<'a> XlsBook<'a> {
    fn new(x: &'a Xls, dirty: HashSet<(usize, u32, u32)>) -> XlsBook<'a> {
        XlsBook { x, raws: vec![None; x.sheets.len()], cache: HashMap::new(), computing: HashSet::new(), stack: Vec::new(), dirty, unsupported: HashSet::new() }
    }
    fn raw(&mut self, si: usize) -> &HashMap<(u32, u32), (Val, Option<String>)> {
        if self.raws[si].is_none() {
            self.raws[si] = Some(self.x.cells(si).into_iter().map(|(r, c, v, f, _)| ((r, c), (v, f))).collect());
        }
        self.raws[si].as_ref().unwrap()
    }
    fn value(&mut self, si: usize, r: u32, c: u32) -> Val {
        if let Some(v) = self.cache.get(&(si, r, c)) {
            return v.clone();
        }
        let raw = self.raw(si).get(&(r, c)).cloned();
        let v = match raw {
            None => Val::Empty,
            Some((v, None)) => v,
            Some((cached, Some(f))) => {
                if !self.dirty.contains(&(si, r, c)) {
                    cached
                } else if !self.computing.insert((si, r, c)) {
                    Val::Num(0.0)
                } else {
                    self.stack.push((si, r, c));
                    let v = match formula::parse(&f) {
                        Ok(e) => {
                            collect_unknown(&e, &mut self.unsupported);
                            formula::eval(&e, self)
                        }
                        Err(_) => Val::Err(XlErr::Name),
                    };
                    self.stack.pop();
                    self.computing.remove(&(si, r, c));
                    v.first()
                }
            }
        };
        self.cache.insert((si, r, c), v.clone());
        v
    }
}

fn collect_unknown(e: &formula::Expr, out: &mut HashSet<String>) {
    use formula::Expr;
    match e {
        Expr::Call(n, args) => {
            if !crate::xlsx::KNOWN_FUNCTIONS.contains(&n.as_str()) {
                out.insert(n.clone());
            }
            for a in args {
                collect_unknown(a, out);
            }
        }
        Expr::Neg(a) | Expr::Pct(a) => collect_unknown(a, out),
        Expr::Bin(_, a, b) | Expr::Range(a, b) => {
            collect_unknown(a, out);
            collect_unknown(b, out);
        }
        _ => {}
    }
}

impl<'a> Ctx for XlsBook<'a> {
    fn cell(&mut self, sheet: Option<&str>, row: u32, col: u32) -> Val {
        let cur = self.stack.last().map(|s| s.0).unwrap_or(0);
        let si = match sheet {
            Some(s) => match self.x.sheet_index(s) {
                Some(i) => i,
                None => return Val::Err(XlErr::Ref),
            },
            None => cur,
        };
        self.value(si, row, col)
    }
    fn extent(&mut self, sheet: Option<&str>) -> (u32, u32) {
        let cur = self.stack.last().map(|s| s.0).unwrap_or(0);
        let si = sheet.and_then(|s| self.x.sheet_index(s)).unwrap_or(cur);
        self.raw(si).keys().fold((0, 0), |a, k| (a.0.max(k.0), a.1.max(k.1)))
    }
    fn name(&mut self, _name: &str) -> Option<String> {
        None
    }
    fn sheet_exists(&mut self, sheet: &str) -> bool {
        self.x.sheet_index(sheet).is_some()
    }
    fn here(&self) -> (u32, u32) {
        self.stack.last().map(|s| (s.1, s.2)).unwrap_or((0, 0))
    }
    fn date1904(&self) -> bool {
        self.x.date1904
    }
    fn now(&self) -> f64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0) / 86400.0 + 25569.0
    }
}
