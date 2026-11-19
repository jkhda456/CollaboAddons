//! Word 97-2003 documents (.doc), edited in part. Text is read through the piece table. An edit
//! is done as Word's own fast save does it: the new text is appended to the WordDocument stream
//! with pages of character and paragraph properties copied from where the text goes (and changed
//! for a style edit), the piece table gets a piece pointing at it, and the tables that count
//! characters of the main text (sections, fields, bookmarks, notes' references, shapes' anchors)
//! move by the change in length. Layout caches (page and line breaks Word computed) are dropped;
//! Word computes them again. Every other byte stays.

use crate::cfb::Cfb;
use crate::model::{Item, Match, Outline};
use crate::props::{Color, Props, Vert};
use crate::text::{self, Finder};
use crate::{err, Result};

fn u16_at(d: &[u8], i: usize) -> u16 {
    d.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0)
}
fn u32_at(d: &[u8], i: usize) -> u32 {
    d.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0)
}
fn put_u32(d: &mut [u8], i: usize, v: u32) {
    d[i..i + 4].copy_from_slice(&v.to_le_bytes());
}

const FC_BASE: usize = 154;
const PLCF_BTE_CHPX: usize = 12;
const PLCF_BTE_PAPX: usize = 13;
const CLX: usize = 33;

/// PLCs whose CPs are in the main text: (FcLcb index, size of each data element).
const MAIN_PLCS: &[(usize, usize)] = &[
    (2, 2),   // PlcffndRef
    (4, 30),  // PlcfandRef
    (6, 12),  // PlcfSed
    (16, 2),  // PlcfFldMom
    (22, 4),  // PlcfBkf
    (23, 0),  // PlcfBkl
    (40, 26), // PlcSpaMom
    (42, 4),  // PlcfAtnBkf
    (43, 0),  // PlcfAtnBkl
    (46, 2),  // PlcfendRef
    (116, 6), // PlcfBkfFactoid
    (118, 4), // PlcfBklFactoid
    (122, 6), // PlcfBkfFcc
    (123, 4), // PlcfBklFcc
    (125, 6), // PlcfbkfBPRepairs
    (126, 4), // PlcfbklBPRepairs
    (139, 6), // PlcfBkfSdt
    (140, 4), // PlcfBklSdt
    (143, 6), // PlcfBkfProt
    (144, 4), // PlcfBklProt
    (167, 6), // PlcfBkfMoveFrom
    (168, 4), // PlcfBklMoveFrom
    (170, 6), // PlcfBkfMoveTo
    (171, 4), // PlcfBklMoveTo
    (176, 6), // PlcfBkfArto
    (177, 4), // PlcfBklArto
];

/// The stories in CP order: (address prefix, offset of its length in the FIB, its PLCs with CPs
/// relative to its start).
const STORIES: &[(&str, usize, &[(usize, usize)])] = &[
    ("body", 76, MAIN_PLCS),
    ("fn", 80, &[(3, 0), (18, 2)]),
    ("hdr", 84, &[(11, 0), (17, 2)]),
    ("mcr", 88, &[]),
    ("cmt", 92, &[(5, 0), (19, 2)]),
    ("en", 96, &[(47, 0), (48, 2)]),
    ("txbx", 100, &[(56, 22), (57, 2), (75, 6)]),
    ("hdrtxbx", 104, &[(58, 22), (59, 2), (76, 6)]),
];

/// Caches of layout and proofing Word rebuilds: emptied after an edit.
const CACHES: &[usize] = &[
    7, 8, 34, 55, 63, 64, 65, 66, 67, 68, 86, 89, 91, 94, 103, 104, 105, 106, 107, 108, 110, 129, 130, 131, 134, 135, 136, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159,
    160, 161, 162, 163, 164,
];

#[derive(Clone, Debug)]
struct Piece {
    cp: u32,
    cp_end: u32,
    fc: u32,
    compressed: bool,
    raw: [u8; 8],
}

impl Piece {
    fn fc_of(&self, cp: u32) -> u32 {
        self.fc + (cp - self.cp) * if self.compressed { 1 } else { 2 }
    }
}

pub struct Doc {
    cfb: Cfb,
    wd: Vec<u8>,
    table: Vec<u8>,
    table_name: String,
    pieces: Vec<Piece>,
    prc: Vec<u8>,
    pub kind: &'static str,
    edited: bool,
}

fn story_label(i: usize) -> &'static str {
    match STORIES[i].0 {
        "fn" => "footnotes",
        "hdr" => "headers and footers",
        "cmt" => "comments",
        "en" => "endnotes",
        "txbx" => "text boxes",
        "hdrtxbx" => "text boxes in headers",
        _ => "",
    }
}

fn cp1252(b: u8) -> char {
    const HI: [u32; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161,
        0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    let c = if (0x80..0xA0).contains(&b) { HI[(b - 0x80) as usize] } else { b as u32 };
    char::from_u32(c).unwrap_or('?')
}

/// A paragraph of the main text as shown: its text and the CP of each shown character.
#[derive(Clone, Debug)]
pub struct Para {
    pub cp: u32,
    pub cp_end: u32,
    pub text: String,
    pub cps: Vec<u32>,
    pub cell: bool,
}

impl Doc {
    pub fn open(cfb: Cfb) -> Result<Doc> {
        let wd = cfb.stream("WordDocument").ok_or_else(|| err("no WordDocument stream"))?.to_vec();
        if u16_at(&wd, 0) != 0xA5EC {
            return Err(err("not a Word 97 or later document"));
        }
        let flags = u16_at(&wd, 10);
        if flags & 0x0100 != 0 {
            return Err(err("password-protected (encrypted); not supported"));
        }
        let nfib = u16_at(&wd, 2);
        if nfib < 0x00C1 {
            return Err(err("a Word document older than Word 97: not supported"));
        }
        let table_name = if flags & 0x0200 != 0 { "1Table" } else { "0Table" }.to_string();
        let table = cfb.stream(&table_name).ok_or_else(|| err(format!("no {table_name} stream")))?.to_vec();
        let mut d = Doc { cfb, wd, table, table_name, pieces: Vec::new(), prc: Vec::new(), kind: "doc", edited: false };
        d.read_clx()?;
        Ok(d)
    }

    fn fclcb(&self, i: usize) -> (usize, usize) {
        let n = u16_at(&self.wd, 152) as usize;
        if i >= n {
            return (0, 0);
        }
        (u32_at(&self.wd, FC_BASE + i * 8) as usize, u32_at(&self.wd, FC_BASE + i * 8 + 4) as usize)
    }

    fn set_fclcb(&mut self, i: usize, fc: usize, lcb: usize) {
        let n = u16_at(&self.wd, 152) as usize;
        if i < n {
            put_u32(&mut self.wd, FC_BASE + i * 8, fc as u32);
            put_u32(&mut self.wd, FC_BASE + i * 8 + 4, lcb as u32);
        }
    }

    fn ccp_text(&self) -> u32 {
        u32_at(&self.wd, 64 + 12)
    }

    /// (start CP, length) of each story.
    fn stories(&self) -> Vec<(u32, u32)> {
        let mut at = 0;
        STORIES
            .iter()
            .map(|(_, off, _)| {
                let n = u32_at(&self.wd, *off);
                let r = (at, n);
                at += n;
                r
            })
            .collect()
    }

    fn story_of(&self, a: u32, b: u32) -> Option<usize> {
        self.stories().iter().position(|(s, n)| a >= *s && b <= s + n && (*n > 0 || a == *s))
    }

    fn read_clx(&mut self) -> Result<()> {
        let (fc, lcb) = self.fclcb(CLX);
        let clx = self.table.get(fc..fc + lcb).ok_or_else(|| err("damaged document (piece table)"))?.to_vec();
        let mut i = 0;
        while i < clx.len() && clx[i] == 1 {
            let cb = u16_at(&clx, i + 1) as usize;
            i += 3 + cb;
        }
        self.prc = clx[..i].to_vec();
        if clx.get(i) != Some(&2) {
            return Err(err("damaged document (no piece table)"));
        }
        let lcb_pcd = u32_at(&clx, i + 1) as usize;
        let plc = &clx[i + 5..(i + 5 + lcb_pcd).min(clx.len())];
        let n = (plc.len() - 4) / 12;
        for k in 0..n {
            let cp = u32_at(plc, k * 4);
            let cp_end = u32_at(plc, (k + 1) * 4);
            let pcd = &plc[(n + 1) * 4 + k * 8..(n + 1) * 4 + k * 8 + 8];
            let fcv = u32_at(pcd, 2);
            let compressed = fcv & 0x4000_0000 != 0;
            let fc = if compressed { (fcv & 0x3FFF_FFFF) / 2 } else { fcv };
            let mut raw = [0u8; 8];
            raw.copy_from_slice(pcd);
            self.pieces.push(Piece { cp, cp_end, fc, compressed, raw });
        }
        Ok(())
    }

    /// The characters [a, b) of the document's CP space.
    fn chars(&self, a: u32, b: u32) -> Vec<char> {
        let mut out = Vec::with_capacity((b - a) as usize);
        for p in &self.pieces {
            if p.cp_end <= a || p.cp >= b {
                continue;
            }
            let s = a.max(p.cp);
            let e = b.min(p.cp_end);
            if p.compressed {
                let start = p.fc_of(s) as usize;
                for k in 0..(e - s) as usize {
                    out.push(cp1252(*self.wd.get(start + k).unwrap_or(&0)));
                }
            } else {
                let start = p.fc_of(s) as usize;
                let units: Vec<u16> = (0..(e - s) as usize).map(|k| u16_at(&self.wd, start + k * 2)).collect();
                out.extend(char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')));
            }
        }
        out
    }

    /// A story's paragraphs, fields shown by their results.
    pub fn paragraphs(&self, story: usize) -> Vec<Para> {
        let (start, n) = self.stories()[story];
        if n == 0 {
            return Vec::new();
        }
        let chars = self.chars(start, start + n);
        let mut out = Vec::new();
        let mut cur = Para { cp: start, cp_end: start, text: String::new(), cps: Vec::new(), cell: false };
        // Field state: depth of field codes being hidden.
        let mut code_depth: Vec<bool> = Vec::new();
        for (i, &c) in chars.iter().enumerate() {
            let cp = start + i as u32;
            match c {
                '\r' | '\u{7}' => {
                    cur.cp_end = cp;
                    cur.cell = c == '\u{7}';
                    out.push(std::mem::replace(&mut cur, Para { cp: cp + 1, cp_end: cp + 1, text: String::new(), cps: Vec::new(), cell: false }));
                    continue;
                }
                '\u{13}' => {
                    code_depth.push(true);
                    continue;
                }
                '\u{14}' => {
                    if let Some(l) = code_depth.last_mut() {
                        *l = false;
                    }
                    continue;
                }
                '\u{15}' => {
                    code_depth.pop();
                    continue;
                }
                _ => {}
            }
            if code_depth.iter().any(|x| *x) {
                continue;
            }
            let shown = match c {
                '\u{b}' => Some('\n'),
                '\u{c}' => Some('\u{c}'),
                '\u{1}' | '\u{8}' => Some('\u{FFFC}'),
                // A note's reference mark (or, in the notes, its number).
                '\u{2}' => Some('\u{E002}'),
                '\u{5}' | '\u{1f}' => None,
                '\u{1e}' => Some('\u{2011}'),
                c if (c as u32) < 0x20 && c != '\t' => None,
                c => Some(c),
            };
            if let Some(s) = shown {
                cur.text.push(s);
                cur.cps.push(cp);
            }
        }
        if !cur.text.is_empty() || cur.cp < start + n {
            cur.cp_end = start + n;
            out.push(cur);
        }
        out
    }

    fn prefix(story: usize) -> String {
        if story == 0 {
            String::new()
        } else {
            format!("{}.", STORIES[story].0)
        }
    }

    pub fn outline(&self, _runs: bool, filter: &[String]) -> Result<Outline> {
        let mut items = Vec::new();
        for st in 0..STORIES.len() {
            let paras = self.paragraphs(st);
            if paras.iter().all(|p| p.text.trim().is_empty()) {
                continue;
            }
            let pre = Self::prefix(st);
            // Note references in the body: footnotes' and endnotes' positions.
            let refs = |idx: usize| -> Vec<u32> {
                let Some(plc) = self.table_slice(idx) else { return Vec::new() };
                let n = (plc.len().saturating_sub(4)) / 6;
                (0..n).map(|k| u32_at(plc, k * 4)).collect()
            };
            let (fn_refs, en_refs) = if st == 0 { (refs(2), refs(46)) } else { (Vec::new(), Vec::new()) };
            if st > 0 {
                items.push(Item { addr: STORIES[st].0.into(), kind: "story".into(), text: story_label(st).into(), ..Default::default() });
            }
            for (i, p) in paras.iter().enumerate() {
                if p.text.trim().is_empty() {
                    continue;
                }
                let mut text = String::new();
                for (k, c) in p.text.chars().enumerate() {
                    match c {
                        '\u{FFFC}' => text.push_str("[object]"),
                        '\u{E002}' => {
                            let cp = p.cps.get(k).copied().unwrap_or(0);
                            if let Some(n) = fn_refs.iter().position(|x| *x == cp) {
                                text.push_str(&format!("[^fn{}]", n + 1));
                            } else if let Some(n) = en_refs.iter().position(|x| *x == cp) {
                                text.push_str(&format!("[^en{}]", n + 1));
                            }
                        }
                        c => text.push(c),
                    }
                }
                let mut it = Item { addr: format!("{pre}p{}", i + 1), kind: "p".into(), text, depth: if st > 0 { 1 } else { 0 }, ..Default::default() };
                if p.cell {
                    it.extra.push(("cell".into(), String::new()));
                }
                items.push(it);
            }
        }
        Ok(crate::doc::filter_outline(Outline { items }, filter))
    }

    pub fn plain(&self) -> String {
        let mut s = String::new();
        for p in self.paragraphs(0) {
            s.push_str(&p.text.replace(['\u{FFFC}', '\u{E002}'], ""));
            s.push(if p.cell { '\t' } else { '\n' });
        }
        s
    }

    /// Paragraphs a scope names: (story, index, paragraph), and a character range.
    fn scope(&self, scope: Option<&str>) -> Result<(Vec<(usize, usize, Para)>, Option<(usize, usize)>)> {
        let Some(sc) = scope else {
            // Every story but comments.
            let mut all = Vec::new();
            for st in 0..STORIES.len() {
                if STORIES[st].0 == "cmt" {
                    continue;
                }
                all.extend(self.paragraphs(st).into_iter().enumerate().map(|(i, p)| (st, i, p)));
            }
            return Ok((all, None));
        };
        let (addr, range) = crate::docx::split_range(sc)?;
        let (story, rest) = match addr.split_once('.') {
            Some((s0, r)) => (STORIES.iter().position(|x| x.0 == s0).ok_or_else(|| err(format!("no story {s0:?} (.doc: body, hdr, fn, en, cmt, txbx, hdrtxbx)")))?, r),
            None => (0, addr),
        };
        let paras = self.paragraphs(story);
        if rest.is_empty() || rest == STORIES[story].0 {
            return Ok((paras.into_iter().enumerate().map(|(i, p)| (story, i, p)).collect(), None));
        }
        let t = rest.strip_prefix('p').ok_or_else(|| err(format!("bad address {sc:?} (.doc: p3, p3-5, p3[2:9], hdr.p1, fn.p2)")))?;
        let (a, b) = match t.split_once('-') {
            Some((a, b)) => (a.parse::<usize>().map_err(|_| err("bad paragraph"))?, b.trim_start_matches('p').parse::<usize>().map_err(|_| err("bad paragraph"))?),
            None => {
                let n = t.parse::<usize>().map_err(|_| err("bad paragraph"))?;
                (n, n)
            }
        };
        if a == 0 || b > paras.len() || b < a {
            return Err(err(format!("{sc}: the story has {} paragraphs", paras.len())));
        }
        Ok((paras.into_iter().enumerate().skip(a - 1).take(b - a + 1).map(|(i, p)| (story, i, p)).collect(), range))
    }

    pub fn find(&self, f: &Finder, scope: Option<&str>) -> Result<Vec<Match>> {
        let (paras, _) = self.scope(scope)?;
        let mut out = Vec::new();
        for (st, i, p) in paras {
            for (a, b, _) in f.find(&p.text, None) {
                out.push(Match { addr: format!("{}p{}", Self::prefix(st), i + 1), start: a, end: b, text: p.text.chars().skip(a).take(b - a).collect(), context: text::escape_line(&p.text) });
            }
        }
        Ok(out)
    }

    /// CP range of shown characters [a, b) of a paragraph.
    fn cp_range(p: &Para, a: usize, b: usize) -> (u32, u32) {
        let ca = p.cps.get(a).copied().unwrap_or(p.cp_end);
        let cb = if b == 0 { ca } else { p.cps.get(b - 1).map(|c| c + 1).unwrap_or(p.cp_end) };
        (ca, cb.max(ca))
    }

    pub fn replace(&mut self, f: &Finder, with: &str, scope: Option<&str>, nth: Option<usize>) -> Result<Vec<String>> {
        let with = text::unescape(with).replace('\n', "\u{b}");
        let (paras, range) = self.scope(scope)?;
        let mut edits: Vec<(u32, u32, String, String)> = Vec::new();
        let mut count = 0;
        for (st, i, p) in paras {
            for (a, b, rep) in f.find(&p.text, Some(&with)) {
                if let Some((ra, rb)) = range {
                    if a < ra || b > rb {
                        continue;
                    }
                }
                count += 1;
                if nth.map(|k| k == count).unwrap_or(true) {
                    let (ca, cb) = Self::cp_range(&p, a, b);
                    edits.push((ca, cb, rep, format!("{}p{}", Self::prefix(st), i + 1)));
                }
            }
        }
        let mut done = Vec::new();
        edits.sort_by(|x, y| y.0.cmp(&x.0));
        for (ca, cb, rep, addr) in edits {
            self.edit(ca, cb, &rep, None)?;
            done.push(addr);
        }
        done.reverse();
        Ok(done)
    }

    pub fn set_text(&mut self, addr: &str, value: &str) -> Result<String> {
        let value = text::unescape(value).replace('\n', "\u{b}");
        let (paras, range) = self.scope(Some(addr))?;
        if paras.len() != 1 {
            return Err(err("set takes one paragraph"));
        }
        let p = &paras[0].2;
        // The whole paragraph's characters (field codes too), before its mark.
        let (ca, cb) = match range {
            Some((a, b)) => Self::cp_range(p, a, b.min(p.text.chars().count())),
            None => (p.cp, p.cp_end),
        };
        self.edit(ca, cb, &value, None)?;
        Ok(addr.to_string())
    }

    pub fn insert(&mut self, addr: &str, after: bool, value: &str) -> Result<Vec<String>> {
        let value = text::unescape(value).replace('\n', "\r");
        let (paras, _) = self.scope(Some(addr))?;
        let (st, i, p) = if after { paras.last().unwrap().clone() } else { paras[0].clone() };
        // New paragraphs take the look of this one: a paragraph mark and text inserted at its
        // end make the new paragraph(s) after it.
        let n = value.split('\r').count();
        let pre = Self::prefix(st);
        if after {
            self.edit(p.cp_end, p.cp_end, &format!("\r{value}"), None)?;
            Ok((0..n).map(|k| format!("{pre}p{}", i + 2 + k)).collect())
        } else {
            self.edit(p.cp, p.cp, &format!("{value}\r"), None)?;
            Ok((0..n).map(|k| format!("{pre}p{}", i + 1 + k)).collect())
        }
    }

    pub fn delete(&mut self, addr: &str) -> Result<String> {
        let (paras, range) = self.scope(Some(addr))?;
        if let Some((a, b)) = range {
            let p = &paras[0].2;
            let (ca, cb) = Self::cp_range(p, a, b.min(p.text.chars().count()));
            self.edit(ca, cb, "", None)?;
            return Ok(format!("{addr}: removed characters {a}..{b}"));
        }
        let first = paras.first().unwrap().2.cp;
        let last = &paras.last().unwrap().2;
        let story_end = self.stories()[paras[0].0].0 + self.stories()[paras[0].0].1;
        if last.cell || paras.iter().any(|(_, _, p)| p.cell) {
            return Err(err("deleting table cells in .doc files is not supported (set their text instead)"));
        }
        // With the last one's mark, unless it is the document's last paragraph: then the mark
        // before it goes, when that is a plain paragraph mark (not a table's), else only the text.
        let (start, end) = if last.cp_end + 1 < story_end {
            (first, last.cp_end + 1)
        } else if first > 0 && self.chars(first - 1, first) == ['\r'] {
            (first - 1, last.cp_end)
        } else {
            (first, last.cp_end)
        };
        self.edit(start, end, "", None)?;
        Ok(format!("deleted {} paragraph{}", paras.len(), if paras.len() == 1 { "" } else { "s" }))
    }

    pub fn style(&mut self, addr: &str, props: &Props, matching: Option<(&Finder, Option<usize>)>) -> Result<Vec<String>> {
        props.only(&["bold", "italic", "underline", "strike", "size", "color", "highlight", "super", "sub", "baseline", "font", "font.latin", "caps", "smallcaps", "hidden"], ".doc text")?;
        let sprms = sprms_for(props)?;
        let (paras, range) = self.scope(Some(addr))?;
        let mut spans: Vec<(u32, u32, String)> = Vec::new();
        let mut count = 0;
        for (st, i, p) in &paras {
            let n = p.text.chars().count();
            let pre = Self::prefix(*st);
            match matching {
                Some((f, nth)) => {
                    for (a, b, _) in f.find(&p.text, None) {
                        count += 1;
                        if nth.map(|k| k == count).unwrap_or(true) {
                            let (ca, cb) = Self::cp_range(p, a, b);
                            spans.push((ca, cb, format!("{pre}p{}", i + 1)));
                        }
                    }
                }
                None => {
                    let (a, b) = range.unwrap_or((0, n));
                    let (ca, cb) = Self::cp_range(p, a.min(n), b.min(n));
                    spans.push((ca, cb, format!("{pre}p{}", i + 1)));
                }
            }
        }
        if spans.is_empty() {
            return Err(err("no text matched"));
        }
        let mut done = Vec::new();
        spans.sort_by(|x, y| y.0.cmp(&x.0));
        for (ca, cb, a) in spans {
            if ca >= cb {
                continue;
            }
            // The same characters again, as a piece with the new properties (runs of
            // different formatting are styled one by one, keeping theirs).
            let text: String = self.chars(ca, cb).iter().collect();
            self.edit(ca, cb, &text, Some(&sprms))?;
            done.push(a);
        }
        done.reverse();
        Ok(done)
    }

    // ── the edit ────────────────────────────────────────────────────────────────────────────

    fn find_piece(&self, cp: u32) -> Option<usize> {
        self.pieces.iter().position(|p| p.cp <= cp && cp < p.cp_end).or_else(|| if cp > 0 { self.pieces.iter().position(|p| p.cp < cp && cp <= p.cp_end) } else { None })
    }

    /// The FKP page and entry covering a byte of the WordDocument stream, from a bin table.
    fn fkp_entry(&self, plc_index: usize, fc: u32) -> Option<(Vec<u8>, usize)> {
        let (pfc, lcb) = self.fclcb(plc_index);
        let plc = self.table.get(pfc..pfc + lcb)?;
        let n = (lcb.checked_sub(4)?) / 8;
        let mut pn = None;
        for k in 0..n {
            let a = u32_at(plc, k * 4);
            let b = u32_at(plc, (k + 1) * 4);
            if a <= fc && fc < b {
                pn = Some(u32_at(plc, (n + 1) * 4 + k * 4) & 0x3F_FFFF);
            }
        }
        let pn = pn.or_else(|| if n > 0 { Some(u32_at(plc, (n + 1) * 4 + (n - 1) * 4) & 0x3F_FFFF) } else { None })? as usize;
        let page = self.wd.get(pn * 512..pn * 512 + 512)?.to_vec();
        let crun = page[511] as usize;
        let mut idx = 0;
        for k in 0..crun {
            if u32_at(&page, k * 4) <= fc {
                idx = k;
            }
        }
        Some((page, idx))
    }

    /// Replaces CPs [a, b) of the main text by `new` (with `sprms` added to its character
    /// properties).
    fn edit(&mut self, a: u32, b: u32, new: &str, sprms: Option<&[u8]>) -> Result<()> {
        let story = self.story_of(a, b).ok_or_else(|| err("an edit across stories"))?;
        let story_start = self.stories()[story].0;
        let units: Vec<u16> = new.encode_utf16().collect();
        let n = units.len() as u32;
        let delta = n as i64 - (b - a) as i64;
        // Properties of where the text goes: the character before (else at) a.
        let at_cp = if a > 0 && a == b { a - 1 } else { a };
        let pi = self.find_piece(at_cp).ok_or_else(|| err("damaged document (no piece for the text)"))?;
        let at_fc = self.pieces[pi].fc_of(at_cp.max(self.pieces[pi].cp));
        let mut new_pieces: Vec<Piece> = Vec::new();
        let mut new_fc = 0u32;
        if n > 0 {
            let (chp_page, chp_i) = self.fkp_entry(PLCF_BTE_CHPX, at_fc).ok_or_else(|| err("damaged document (character properties)"))?;
            let (pap_page, pap_i) = self.fkp_entry(PLCF_BTE_PAPX, at_fc).ok_or_else(|| err("damaged document (paragraph properties)"))?;
            // CHPX bytes (cb + grpprl), or none (default formatting).
            let chp_off = chp_page[(chp_page[511] as usize + 1) * 4 + chp_i] as usize * 2;
            let mut grpprl: Vec<u8> = if chp_off == 0 { Vec::new() } else { chp_page[chp_off + 1..chp_off + 1 + chp_page[chp_off] as usize].to_vec() };
            if let Some(extra) = sprms {
                grpprl.extend_from_slice(extra);
            }
            if grpprl.len() > 255 {
                return Err(err("too many character properties"));
            }
            let cpara = pap_page[511] as usize;
            let bx_at = (cpara + 1) * 4 + pap_i * 13;
            let pap_off = pap_page[bx_at] as usize * 2;
            let papx: Vec<u8> = if pap_off == 0 {
                vec![0, 0, 0]
            } else {
                let cb = pap_page[pap_off] as usize;
                let len = if cb == 0 { 2 + 2 * pap_page[pap_off + 1] as usize } else { 1 + 2 * cb - 1 };
                pap_page[pap_off..pap_off + len.min(512 - pap_off)].to_vec()
            };
            // Pages first (512-aligned), then the text.
            while self.wd.len() % 512 != 0 {
                self.wd.push(0);
            }
            let chp_pn = self.wd.len() / 512;
            let pap_pn = chp_pn + 1;
            new_fc = ((chp_pn + 2) * 512) as u32;
            let new_end = new_fc + n * 2;
            let mut chp = vec![0u8; 512];
            put_u32(&mut chp, 0, new_fc);
            put_u32(&mut chp, 4, new_end);
            let place = (511 - (1 + grpprl.len())) & !1;
            chp[8] = (place / 2) as u8;
            chp[place] = grpprl.len() as u8;
            chp[place + 1..place + 1 + grpprl.len()].copy_from_slice(&grpprl);
            chp[511] = 1;
            // Paragraph pages: a boundary after every CR in the new text; the last run goes past
            // the piece, so the paragraph continues into the text after it.
            let mut bounds: Vec<u32> = vec![new_fc];
            for (k, u) in units.iter().enumerate() {
                if *u == 13 {
                    bounds.push(new_fc + (k as u32 + 1) * 2);
                }
            }
            if *bounds.last().unwrap() < new_end || bounds.len() == 1 {
                bounds.push(new_end + 2);
            }
            let cpara = bounds.len() - 1;
            if cpara > 25 {
                return Err(err("too many paragraphs in one edit (split it)"));
            }
            let mut pap = vec![0u8; 512];
            for (k, f) in bounds.iter().enumerate() {
                put_u32(&mut pap, k * 4, *f);
            }
            let place = (511 - papx.len()) & !1;
            pap[place..place + papx.len()].copy_from_slice(&papx);
            for k in 0..cpara {
                pap[(cpara + 1) * 4 + k * 13] = (place / 2) as u8;
            }
            pap[511] = cpara as u8;
            self.wd.extend_from_slice(&chp);
            self.wd.extend_from_slice(&pap);
            for u in &units {
                self.wd.extend_from_slice(&u.to_le_bytes());
            }
            // The bin tables get an entry each for the new range.
            let bounds_end = *bounds.last().unwrap();
            self.append_bte(PLCF_BTE_CHPX, new_end, chp_pn as u32)?;
            self.append_bte(PLCF_BTE_PAPX, bounds_end, pap_pn as u32)?;
        }
        // The piece table.
        for p in &self.pieces {
            if p.cp_end <= a {
                new_pieces.push(p.clone());
            }
        }
        if let Some(i) = self.pieces.iter().position(|p| p.cp < a && a < p.cp_end) {
            let p = &self.pieces[i];
            new_pieces.push(Piece { cp: p.cp, cp_end: a, ..p.clone() });
        }
        if n > 0 {
            let prm = self.pieces.get(pi).map(|p| [p.raw[6], p.raw[7]]).unwrap_or([0, 0]);
            let mut raw = [0u8; 8];
            raw[0..2].copy_from_slice(&self.pieces.get(pi).map(|p| [p.raw[0], p.raw[1]]).unwrap_or([0, 0]));
            raw[2..6].copy_from_slice(&new_fc.to_le_bytes());
            raw[6..8].copy_from_slice(&prm);
            new_pieces.push(Piece { cp: a, cp_end: a + n, fc: new_fc, compressed: false, raw });
        }
        let shift = |cp: u32| (cp as i64 + delta) as u32;
        for p in &self.pieces {
            if p.cp < b && b < p.cp_end {
                // The rest of the piece the range ends in.
                let fc = p.fc_of(b);
                new_pieces.push(Piece { cp: shift(b), cp_end: shift(p.cp_end), fc, ..p.clone() });
            } else if p.cp >= b && p.cp_end > p.cp {
                if p.cp < a {
                    continue;
                }
                new_pieces.push(Piece { cp: shift(p.cp), cp_end: shift(p.cp_end), ..p.clone() });
            }
        }
        new_pieces.retain(|p| p.cp_end > p.cp);
        self.pieces = new_pieces;
        self.write_clx();
        // The story's length and the tables that count its characters (from its start).
        let off = STORIES[story].1;
        let ccp = (u32_at(&self.wd, off) as i64 + delta) as u32;
        put_u32(&mut self.wd, off, ccp);
        for &(idx, cb) in STORIES[story].2 {
            self.shift_plc(idx, cb, a - story_start, b - story_start, delta);
        }
        for &idx in CACHES {
            let (fc, lcb) = self.fclcb(idx);
            if lcb > 0 {
                self.set_fclcb(idx, fc, 0);
            }
        }
        let len = self.wd.len() as u32;
        put_u32(&mut self.wd, 64, len);
        self.edited = true;
        Ok(())
    }

    fn append_bte(&mut self, idx: usize, end_fc: u32, pn: u32) -> Result<()> {
        let (fc, lcb) = self.fclcb(idx);
        let plc = self.table.get(fc..fc + lcb).ok_or_else(|| err("damaged document (bin table)"))?.to_vec();
        let n = (lcb - 4) / 8;
        let mut fcs: Vec<u32> = (0..=n).map(|k| u32_at(&plc, k * 4)).collect();
        let mut pns: Vec<u32> = (0..n).map(|k| u32_at(&plc, (n + 1) * 4 + k * 4)).collect();
        fcs.push(end_fc.max(*fcs.last().unwrap_or(&0) + 1));
        pns.push(pn);
        let mut out = Vec::new();
        for f in fcs {
            out.extend_from_slice(&f.to_le_bytes());
        }
        for p in pns {
            out.extend_from_slice(&p.to_le_bytes());
        }
        let at = self.table.len();
        let l = out.len();
        self.table.extend_from_slice(&out);
        self.set_fclcb(idx, at, l);
        Ok(())
    }

    fn write_clx(&mut self) {
        let mut plc = Vec::new();
        for p in &self.pieces {
            plc.extend_from_slice(&p.cp.to_le_bytes());
        }
        plc.extend_from_slice(&self.pieces.last().map(|p| p.cp_end).unwrap_or(0).to_le_bytes());
        for p in &self.pieces {
            let mut raw = p.raw;
            let fcv = if p.compressed { (p.fc * 2) | 0x4000_0000 } else { p.fc };
            raw[2..6].copy_from_slice(&fcv.to_le_bytes());
            plc.extend_from_slice(&raw);
        }
        let mut clx = self.prc.clone();
        clx.push(2);
        clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
        clx.extend_from_slice(&plc);
        let at = self.table.len();
        let l = clx.len();
        self.table.extend_from_slice(&clx);
        self.set_fclcb(CLX, at, l);
    }

    fn shift_plc(&mut self, idx: usize, cb: usize, a: u32, b: u32, delta: i64) {
        let (fc, lcb) = self.fclcb(idx);
        if lcb < 4 || fc + lcb > self.table.len() {
            return;
        }
        let n = (lcb - 4) / (4 + cb);
        if (n + 1) * 4 + n * cb != lcb {
            return;
        }
        for k in 0..=n {
            let at = fc + k * 4;
            let cp = u32_at(&self.table, at);
            let new = if cp >= b {
                (cp as i64 + delta).max(0) as u32
            } else if cp > a {
                // Inside the replaced range: to its start.
                a
            } else {
                cp
            };
            if new != cp {
                put_u32(&mut self.table, at, new);
            }
        }
    }

    // ── for conversion ──────────────────────────────────────────────────────────────────────

    /// The data a FIB pair points at in the table stream.
    pub fn table_slice(&self, idx: usize) -> Option<&[u8]> {
        let (fc, lcb) = self.fclcb(idx);
        if lcb == 0 {
            return None;
        }
        self.table.get(fc..fc + lcb)
    }

    /// The table stream from where a FIB pair points to its end (for structures followed by
    /// data their length does not count, as the lists' levels).
    pub fn table_from(&self, idx: usize) -> Option<&[u8]> {
        let (fc, lcb) = self.fclcb(idx);
        if lcb == 0 {
            return None;
        }
        self.table.get(fc..)
    }

    pub fn wd_slice(&self, off: usize, len: usize) -> Option<&[u8]> {
        self.wd.get(off..off + len)
    }

    pub fn data_stream(&self) -> Option<&[u8]> {
        self.cfb.stream("Data")
    }

    pub fn story_start(&self, story: usize) -> u32 {
        self.stories()[story].0
    }

    pub fn story_len(&self, story: usize) -> u32 {
        self.stories()[story].1
    }

    pub fn fc_of_cp(&self, cp: u32) -> Option<u32> {
        let p = self.pieces.iter().find(|p| p.cp <= cp && cp < p.cp_end)?;
        Some(p.fc_of(cp))
    }

    /// Characters of a CP range with the stream offset each comes from.
    pub fn cp_chars(&self, a: u32, b: u32) -> Vec<(u32, char)> {
        let mut out = Vec::new();
        for p in &self.pieces {
            if p.cp_end <= a || p.cp >= b {
                continue;
            }
            let s = a.max(p.cp);
            let e = b.min(p.cp_end);
            for cp in s..e {
                let fc = p.fc_of(cp);
                let c = if p.compressed {
                    cp1252(*self.wd.get(fc as usize).unwrap_or(&0))
                } else {
                    char::from_u32(u16_at(&self.wd, fc as usize) as u32).unwrap_or('\u{FFFD}')
                };
                out.push((fc, c));
            }
        }
        out
    }

    /// The CP range (absolute) of the nth footnote's text, without its paragraph mark.
    pub fn footnote_range(&self, n: usize) -> Option<(u32, u32)> {
        let plc = self.table_slice(3)?;
        let count = plc.len() / 4;
        if n + 1 >= count {
            return None;
        }
        let start = self.story_start(1);
        let a = u32_at(plc, n * 4);
        let b = u32_at(plc, (n + 1) * 4);
        Some((start + a, start + b.saturating_sub(1).max(a)))
    }

    pub fn save(&mut self) -> Result<Vec<u8>> {
        if self.edited {
            let name = self.table_name.clone();
            self.cfb.set_stream("WordDocument", self.wd.clone())?;
            self.cfb.set_stream(&name, self.table.clone())?;
        }
        Ok(self.cfb.save())
    }
}

/// Character property modifiers (sprms) for a style change.
fn sprms_for(p: &Props) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    let toggle = |v: &mut Vec<u8>, op: u16, on: bool| {
        v.extend_from_slice(&op.to_le_bytes());
        v.push(on as u8);
    };
    if let Some(b) = p.bold {
        toggle(&mut v, 0x0835, b);
    }
    if let Some(i) = p.italic {
        toggle(&mut v, 0x0836, i);
    }
    if let Some(s) = p.strike {
        toggle(&mut v, 0x0837, s);
    }
    if let Some(c) = p.caps {
        toggle(&mut v, 0x083B, c);
    }
    if let Some(c) = p.small_caps {
        toggle(&mut v, 0x083A, c);
    }
    if let Some(h) = p.hidden {
        toggle(&mut v, 0x083C, h);
    }
    if let Some(u) = &p.underline {
        v.extend_from_slice(&0x2A3Eu16.to_le_bytes());
        v.push(match u.as_str() {
            "none" => 0,
            "double" => 3,
            "dotted" | "dot" => 4,
            "thick" => 6,
            "dash" | "dashed" => 7,
            "wave" | "wavy" => 11,
            "words" => 2,
            _ => 1,
        });
    }
    if let Some(sz) = p.size {
        v.extend_from_slice(&0x4A43u16.to_le_bytes());
        v.extend_from_slice(&((sz * 2.0).round() as u16).to_le_bytes());
        v.extend_from_slice(&0x4A61u16.to_le_bytes());
        v.extend_from_slice(&((sz * 2.0).round() as u16).to_le_bytes());
    }
    if let Some(c) = &p.color {
        match c {
            Color::Rgb(h) => {
                let x = u32::from_str_radix(h, 16).unwrap_or(0);
                // COLORREF: red, green, blue, 0.
                v.extend_from_slice(&0x6870u16.to_le_bytes());
                v.extend_from_slice(&[(x >> 16) as u8, (x >> 8) as u8, x as u8, 0]);
            }
            _ => {
                v.extend_from_slice(&0x2A42u16.to_le_bytes());
                v.push(0);
            }
        }
    }
    if let Some(h) = &p.highlight {
        let ico = match h.hex() {
            None => 0,
            Some("FFFF00") => 7,
            Some("00FF00") => 4,
            Some("00FFFF") => 3,
            Some("FF00FF") => 5,
            Some("0000FF") => 2,
            Some("FF0000") => 6,
            Some("000080") => 9,
            Some("008080") => 10,
            Some("008000") => 11,
            Some("800080") => 12,
            Some("800000") => 13,
            Some("808000") => 14,
            Some("808080") => 15,
            Some("C0C0C0") => 16,
            Some("000000") => 1,
            Some("FFFFFF") => 8,
            Some(_) => 7,
        };
        v.extend_from_slice(&0x2A0Cu16.to_le_bytes());
        v.push(ico);
    }
    if let Some(vert) = &p.vert {
        v.extend_from_slice(&0x2A48u16.to_le_bytes());
        v.push(match vert {
            Vert::Super => 1,
            Vert::Sub => 2,
            Vert::Baseline => 0,
        });
    }
    if p.font.is_some() || p.font_latin.is_some() {
        return Err(err("changing fonts in .doc files is not supported (it needs the font table); convert to .docx"));
    }
    Ok(v)
}
