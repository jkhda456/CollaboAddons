//! PowerPoint 97-2003 presentations (.ppt), edited in part. The PowerPoint Document stream is
//! read as a tree of records; an edit changes text atoms (and the style, special-info and
//! interactive runs that count their characters), and the stream is written again with every
//! other record's bytes as they were, the containers' lengths, the persist directories, the user
//! edits and the Current User stream pointing at the records' new places.

use crate::cfb::Cfb;
use crate::model::{Item, Match, Outline, RunInfo};
use crate::props::{Color, Props};
use crate::text::{self, Finder};
use crate::{err, Result};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Rec {
    pub ver_inst: u16,
    pub typ: u16,
    pub data: Vec<u8>,
    pub kids: Vec<Rec>,
    pub container: bool,
    /// Offset in the original stream (top-level records only matter).
    pub orig: usize,
}

const T_DOCUMENT: u16 = 0x03E8;
const T_SLIDE: u16 = 0x03EE;
const T_NOTES: u16 = 0x03F0;
const T_SLWT: u16 = 0x0FF0;
const T_SLIDE_PERSIST: u16 = 0x03F3;
const T_TEXT_HEADER: u16 = 0x0F9F;
const T_TEXT_CHARS: u16 = 0x0FA0;
const T_TEXT_BYTES: u16 = 0x0FA8;
const T_STYLE_TEXT_PROP: u16 = 0x0FA1;
const T_SPECIAL_INFO: u16 = 0x0FAA;
const T_MASTER_TEXT_PROP: u16 = 0x0FA2;
const T_INTERACTIVE_INFO: u16 = 0x0FDF;
const T_OUTLINE_REF: u16 = 0x0F9E;
const T_USER_EDIT: u16 = 0x0FF5;
const T_PERSIST_DIR: u16 = 0x1772;
const T_PPDRAWING: u16 = 0x040C;
const T_SP_CONTAINER: u16 = 0xF004;
const T_FSP: u16 = 0xF00A;
const T_CLIENT_TEXTBOX: u16 = 0xF00D;
const T_CLIENT_ANCHOR: u16 = 0xF010;
const T_CHILD_ANCHOR: u16 = 0xF00F;

fn u16_at(d: &[u8], i: usize) -> u16 {
    d.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0)
}
fn u32_at(d: &[u8], i: usize) -> u32 {
    d.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0)
}

fn parse(d: &[u8], start: usize, end: usize, depth: usize) -> Vec<Rec> {
    let mut out = Vec::new();
    let mut i = start;
    while i + 8 <= end {
        let ver_inst = u16_at(d, i);
        let typ = u16_at(d, i + 2);
        let len = u32_at(d, i + 4) as usize;
        let body_end = (i + 8 + len).min(end);
        let container = ver_inst & 0xF == 0xF && depth < 32;
        let rec = if container {
            Rec { ver_inst, typ, data: Vec::new(), kids: parse(d, i + 8, body_end, depth + 1), container: true, orig: i }
        } else {
            Rec { ver_inst, typ, data: d[i + 8..body_end].to_vec(), kids: Vec::new(), container: false, orig: i }
        };
        out.push(rec);
        i = body_end;
        if len == 0 && typ == 0 && ver_inst == 0 {
            break;
        }
    }
    out
}

impl Rec {
    fn size(&self) -> usize {
        8 + if self.container { self.kids.iter().map(|k| k.size()).sum() } else { self.data.len() }
    }
    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.ver_inst.to_le_bytes());
        out.extend_from_slice(&self.typ.to_le_bytes());
        out.extend_from_slice(&((self.size() - 8) as u32).to_le_bytes());
        if self.container {
            for k in &self.kids {
                k.write(out);
            }
        } else {
            out.extend_from_slice(&self.data);
        }
    }
    fn at(&self, path: &[usize]) -> &Rec {
        let mut r = self;
        for &i in path {
            r = &r.kids[i];
        }
        r
    }
    fn at_mut(&mut self, path: &[usize]) -> &mut Rec {
        let mut r = self;
        for &i in path {
            r = &mut r.kids[i];
        }
        r
    }
}

/// A piece of text: where its atom is (top-level record index, path), what it belongs to.
#[derive(Clone, Debug)]
pub struct TextLoc {
    pub top: usize,
    pub path: Vec<usize>,
    pub slide: usize,
    pub notes: bool,
    /// The shape (1-based, drawing order) it is in, if any.
    pub shape: Option<usize>,
    pub text_type: u32,
}

pub struct Ppt {
    cfb: Cfb,
    pub tops: Vec<Rec>,
    persist: HashMap<u32, usize>,
    /// Slides: persist id of the slide container; notes likewise.
    pub slides: Vec<(u32, Option<u32>)>,
    pub kind: &'static str,
    pub texts: Vec<TextLoc>,
}

pub fn text_of_atom(r: &Rec) -> String {
    if r.typ == T_TEXT_CHARS {
        let units: Vec<u16> = r.data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        r.data.iter().map(|&b| char::from_u32(cp1252(b)).unwrap_or('?')).collect()
    }
}

fn cp1252(b: u8) -> u32 {
    const HI: [u32; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161,
        0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    if (0x80..0xA0).contains(&b) {
        HI[(b - 0x80) as usize]
    } else {
        b as u32
    }
}

/// PowerPoint text: paragraphs separated by CR, line breaks as VT.
fn to_display(s: &str) -> String {
    s.replace('\u{b}', "\n")
}

impl Ppt {
    pub fn open(cfb: Cfb) -> Result<Ppt> {
        let data = cfb.stream("PowerPoint Document").ok_or_else(|| err("no PowerPoint Document stream"))?.to_vec();
        let tops = parse(&data, 0, data.len(), 0);
        let mut p = Ppt { cfb, tops, persist: HashMap::new(), slides: Vec::new(), kind: "ppt", texts: Vec::new() };
        p.read_persist(&data)?;
        p.index();
        Ok(p)
    }

    fn current_edit(&self) -> Option<usize> {
        let cu = self.cfb.stream("Current User")?;
        Some(u32_at(cu, 16) as usize)
    }

    fn read_persist(&mut self, data: &[u8]) -> Result<()> {
        // The chain of user edits, newest first; older entries do not override newer ones.
        let mut off = self.current_edit().unwrap_or(0);
        let mut guard = 0;
        let mut doc_ref = 0;
        while guard < 64 {
            guard += 1;
            let Some(ue) = self.tops.iter().find(|r| r.orig == off && r.typ == T_USER_EDIT) else { break };
            if doc_ref == 0 {
                doc_ref = u32_at(&ue.data, 16);
            }
            let pd_off = u32_at(&ue.data, 12) as usize;
            let last = u32_at(&ue.data, 8) as usize;
            if let Some(pd) = self.tops.iter().find(|r| r.orig == pd_off && r.typ == T_PERSIST_DIR) {
                let d = &pd.data;
                let mut i = 0;
                while i + 4 <= d.len() {
                    let head = u32_at(d, i);
                    let id = head & 0xFFFFF;
                    let n = (head >> 20) as usize;
                    for k in 0..n {
                        self.persist.entry(id + k as u32).or_insert(u32_at(d, i + 4 + k * 4) as usize);
                    }
                    i += 4 + n * 4;
                }
            }
            if last == 0 || last == off {
                break;
            }
            off = last;
        }
        let _ = data;
        if self.persist.is_empty() {
            return Err(err("a .ppt without a persist directory (damaged, or not PowerPoint 97 or later)"));
        }
        // The document container, and its slide lists.
        let doc_off = *self.persist.get(&doc_ref).ok_or_else(|| err("the document record is missing"))?;
        let doc = self.tops.iter().find(|r| r.orig == doc_off && r.typ == T_DOCUMENT).ok_or_else(|| err("the document record is missing"))?;
        let mut notes_by_id: HashMap<u32, u32> = HashMap::new();
        for lst in doc.kids.iter().filter(|r| r.typ == T_SLWT) {
            let inst = lst.ver_inst >> 4;
            for r in &lst.kids {
                if r.typ == T_SLIDE_PERSIST {
                    let psr = u32_at(&r.data, 0);
                    let slide_id = u32_at(&r.data, 12);
                    match inst {
                        0 => self.slides.push((psr, None)),
                        2 => {
                            notes_by_id.insert(slide_id, psr);
                        }
                        _ => {}
                    }
                }
            }
        }
        // A slide's notes: its SlideAtom names the notes' id.
        let slides = self.slides.clone();
        for (k, (psr, _)) in slides.iter().enumerate() {
            if let Some(t) = self.top_of(*psr) {
                if let Some(atom) = self.tops[t].kids.iter().find(|r| r.typ == 0x03EF) {
                    // SlideAtom: geometry, placeholder types, master id, notes id.
                    let notes_id = u32_at(&atom.data, 16);
                    if notes_id != 0 {
                        self.slides[k].1 = notes_by_id.get(&notes_id).copied();
                    }
                }
            }
        }
        Ok(())
    }

    pub fn top_of(&self, persist_id: u32) -> Option<usize> {
        let off = *self.persist.get(&persist_id)?;
        self.tops.iter().position(|r| r.orig == off)
    }

    /// Finds every text atom: the outline text of each slide (in the slide list after its
    /// SlidePersistAtom) and the text in each shape.
    fn index(&mut self) {
        let mut texts = Vec::new();
        let doc_top = self.tops.iter().position(|r| r.typ == T_DOCUMENT);
        let slide_pos: HashMap<u32, usize> = self.slides.iter().enumerate().map(|(i, s)| (s.0, i)).collect();
        let notes_pos: HashMap<u32, usize> = self.slides.iter().enumerate().filter_map(|(i, s)| s.1.map(|n| (n, i))).collect();
        // Outline texts: (slide, index in the list) → location.
        let mut outline: HashMap<(usize, u32), TextLoc> = HashMap::new();
        if let Some(dt) = doc_top {
            for (li, lst) in self.tops[dt].kids.iter().enumerate() {
                if lst.typ != T_SLWT || lst.ver_inst >> 4 != 0 {
                    continue;
                }
                let mut cur: Option<usize> = None;
                let mut n = 0u32;
                let mut text_type = 0;
                for (ri, r) in lst.kids.iter().enumerate() {
                    match r.typ {
                        T_SLIDE_PERSIST => {
                            cur = slide_pos.get(&u32_at(&r.data, 0)).copied();
                            n = 0;
                        }
                        T_TEXT_HEADER => text_type = u32_at(&r.data, 0),
                        T_TEXT_CHARS | T_TEXT_BYTES => {
                            if let Some(s) = cur {
                                outline.insert((s, n), TextLoc { top: dt, path: vec![li, ri], slide: s, notes: false, shape: None, text_type });
                                n += 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        let mut used_outline: Vec<(usize, u32)> = Vec::new();
        for (s, (psr, notes)) in self.slides.clone().iter().enumerate() {
            for (pid, is_notes) in [(Some(*psr), false), (*notes, true)] {
                let Some(pid) = pid else { continue };
                let Some(t) = self.top_of(pid) else { continue };
                let mut shape_n = 0;
                let mut path = Vec::new();
                walk_shapes(&self.tops[t], &mut path, &mut |path, sp| {
                    shape_n += 1;
                    let Some(tbi) = sp.kids.iter().position(|k| k.typ == T_CLIENT_TEXTBOX) else { return };
                    let tb = &sp.kids[tbi];
                    let mut text_type = 0;
                    for (k, r) in tb.kids.iter().enumerate() {
                        match r.typ {
                            T_TEXT_HEADER => text_type = u32_at(&r.data, 0),
                            T_TEXT_CHARS | T_TEXT_BYTES => {
                                // In notes, only the notes text (not the slide number's).
                                if is_notes && text_type != 2 {
                                    continue;
                                }
                                let mut p = path.to_vec();
                                p.push(tbi);
                                p.push(k);
                                texts.push(TextLoc { top: t, path: p, slide: s, notes: is_notes, shape: Some(shape_n), text_type });
                            }
                            T_OUTLINE_REF => {
                                let idx = u32_at(&r.data, 0);
                                if let Some(mut loc) = outline.get(&(s, idx)).cloned() {
                                    loc.shape = Some(shape_n);
                                    texts.push(loc);
                                    used_outline.push((s, idx));
                                }
                            }
                            _ => {}
                        }
                    }
                });
                let _ = notes_pos.len();
            }
        }
        // Outline text no shape refers to.
        let mut rest: Vec<_> = outline.into_iter().filter(|(k, _)| !used_outline.contains(k)).collect();
        rest.sort_by_key(|(k, _)| *k);
        for (_, loc) in rest {
            texts.push(loc);
        }
        texts.sort_by_key(|l| (l.slide, l.notes, l.shape.unwrap_or(usize::MAX)));
        self.texts = texts;
    }

    pub fn stream(&self, name: &str) -> Option<&[u8]> {
        self.cfb.stream(name)
    }

    /// The master a slide uses (top-level record index), by the slide atom's master id.
    pub fn master_of(&self, slide_top: usize) -> Option<usize> {
        let atom = self.tops[slide_top].kids.iter().find(|r| r.typ == 0x03EF)?;
        let master_id = u32_at(&atom.data, 12);
        let doc = self.tops.iter().find(|r| r.typ == T_DOCUMENT)?;
        for lst in doc.kids.iter().filter(|r| r.typ == T_SLWT && r.ver_inst >> 4 == 1) {
            for r in &lst.kids {
                if r.typ == T_SLIDE_PERSIST && u32_at(&r.data, 12) == master_id {
                    return self.top_of(u32_at(&r.data, 0));
                }
            }
        }
        // The first main master.
        self.tops.iter().position(|r| r.typ == 0x03F8)
    }

    pub fn atom(&self, l: &TextLoc) -> &Rec {
        self.tops[l.top].at(&l.path)
    }

    fn loc_addr(&self, l: &TextLoc) -> String {
        match (l.notes, l.shape) {
            (true, _) => format!("s{}.notes", l.slide + 1),
            (false, Some(sh)) => format!("s{}.sh{}", l.slide + 1, sh),
            (false, None) => format!("s{}.text", l.slide + 1),
        }
    }

    pub fn outline(&self, with_runs: bool, filter: &[String]) -> Result<Outline> {
        let mut items = Vec::new();
        for s in 0..self.slides.len() {
            items.push(Item { addr: format!("s{}", s + 1), kind: "slide".into(), ..Default::default() });
            for l in self.texts.iter().filter(|l| l.slide == s) {
                let addr = self.loc_addr(l);
                let t = text_of_atom(self.atom(l));
                items.push(Item { addr: addr.clone(), kind: if l.notes { "notes".into() } else { "shape".into() }, depth: 1, extra: vec![("type".into(), text_type_name(l.text_type).into())], ..Default::default() });
                let runs = if with_runs { self.char_runs(l) } else { Vec::new() };
                let mut pos = 0usize;
                for (pi, para) in t.split('\r').enumerate() {
                    let n = para.chars().count();
                    let mut it = Item { addr: format!("{addr}.p{}", pi + 1), kind: "p".into(), text: to_display(para), depth: 2, ..Default::default() };
                    if with_runs {
                        it.runs = runs.iter().filter(|r| r.start + r.text.chars().count() > pos && r.start < pos + n).map(|r| RunInfo { text: r.text.chars().skip(pos.saturating_sub(r.start)).take(n).collect(), props: r.props.clone(), start: r.start.saturating_sub(pos) }).collect();
                    }
                    pos += n + 1;
                    if !it.text.is_empty() || with_runs {
                        items.push(it);
                    }
                }
            }
        }
        Ok(crate::doc::filter_outline(Outline { items }, filter))
    }

    pub fn plain(&self) -> String {
        let mut s = String::new();
        for sl in 0..self.slides.len() {
            s.push_str(&format!("\n== s{}\n", sl + 1));
            for l in self.texts.iter().filter(|l| l.slide == sl) {
                let t = text_of_atom(self.atom(l));
                for p in t.split('\r') {
                    s.push_str(&to_display(p));
                    s.push('\n');
                }
            }
        }
        s
    }

    /// The paragraphs a scope names: (text loc index, paragraph index or all).
    fn scope(&self, scope: Option<&str>) -> Result<Vec<(usize, Option<usize>, Option<(usize, usize)>)>> {
        let Some(sc) = scope else { return Ok((0..self.texts.len()).map(|i| (i, None, None)).collect()) };
        let (addr, range) = crate::docx::split_range(sc)?;
        let parts: Vec<&str> = addr.split('.').collect();
        let sn: usize = parts[0].trim_start_matches('s').parse().map_err(|_| err(format!("bad address {sc:?} (s1, s1.sh2, s1.sh2.p1, s1.notes)")))?;
        if sn == 0 || sn > self.slides.len() {
            return Err(err(format!("there are {} slides", self.slides.len())));
        }
        let mut out = Vec::new();
        for (i, l) in self.texts.iter().enumerate() {
            if l.slide != sn - 1 {
                continue;
            }
            let a = self.loc_addr(l);
            if parts.len() == 1 {
                out.push((i, None, None));
                continue;
            }
            let base = parts[..parts.len().min(2)].join(".");
            if a != base {
                continue;
            }
            match parts.get(2) {
                Some(p) => {
                    let n: usize = p.trim_start_matches('p').parse().map_err(|_| err(format!("bad paragraph in {sc:?}")))?;
                    let count = text_of_atom(self.atom(l)).split('\r').count();
                    if n == 0 || n > count {
                        return Err(err(format!("{a} has {count} paragraphs")));
                    }
                    out.push((i, Some(n - 1), range));
                }
                None => out.push((i, None, None)),
            }
        }
        if out.is_empty() {
            return Err(err(format!("no text at {sc}")));
        }
        Ok(out)
    }

    pub fn find(&self, f: &Finder, scope: Option<&str>) -> Result<Vec<Match>> {
        let mut out = Vec::new();
        for (i, para, _) in self.scope(scope)? {
            let l = &self.texts[i];
            let t = text_of_atom(self.atom(l));
            for (pi, p) in t.split('\r').enumerate() {
                if para.map(|x| x != pi).unwrap_or(false) {
                    continue;
                }
                let d = to_display(p);
                for (a, b, _) in f.find(&d, None) {
                    out.push(Match { addr: format!("{}.p{}", self.loc_addr(l), pi + 1), start: a, end: b, text: d.chars().skip(a).take(b - a).collect(), context: text::escape_line(&d) });
                }
            }
        }
        Ok(out)
    }

    pub fn replace(&mut self, f: &Finder, with: &str, scope: Option<&str>, nth: Option<usize>) -> Result<Vec<String>> {
        let with = text::unescape(with);
        let mut done = Vec::new();
        let mut count = 0;
        for (i, para, range) in self.scope(scope)? {
            let l = self.texts[i].clone();
            let t = text_of_atom(self.atom(&l));
            // Matches by absolute character position in the atom.
            let mut edits: Vec<(usize, usize, String)> = Vec::new();
            let mut pos = 0;
            for (pi, p) in t.split('\r').enumerate() {
                let n = p.chars().count();
                if para.map(|x| x == pi).unwrap_or(true) {
                    let d = to_display(p);
                    for (a, b, rep) in f.find(&d, Some(&with)) {
                        if let Some((ra, rb)) = range {
                            if a < ra || b > rb {
                                continue;
                            }
                        }
                        count += 1;
                        if nth.map(|k| k == count).unwrap_or(true) {
                            edits.push((pos + a, pos + b, rep.replace('\n', "\u{b}")));
                            done.push(format!("{}.p{}", self.loc_addr(&l), pi + 1));
                        }
                    }
                }
                pos += n + 1;
            }
            for (a, b, rep) in edits.into_iter().rev() {
                self.splice(&l, a, b, &rep, None)?;
            }
        }
        Ok(done)
    }

    pub fn set_text(&mut self, addr: &str, value: &str) -> Result<String> {
        let value = text::unescape(value).replace('\n', "\u{b}");
        let targets = self.scope(Some(addr))?;
        if targets.len() != 1 {
            return Err(err("set takes one shape's text (s1.sh2), or one paragraph (s1.sh2.p1)"));
        }
        let (i, para, range) = targets[0];
        let l = self.texts[i].clone();
        let t = text_of_atom(self.atom(&l));
        let (a, b) = match para {
            Some(pi) => {
                let start: usize = t.split('\r').take(pi).map(|p| p.chars().count() + 1).sum();
                let len = t.split('\r').nth(pi).map(|p| p.chars().count()).unwrap_or(0);
                match range {
                    Some((ra, rb)) => (start + ra.min(len), start + rb.min(len)),
                    None => (start, start + len),
                }
            }
            // A whole shape: lines of the value become paragraphs.
            None => (0, t.chars().count()),
        };
        let v = if para.is_none() { value.replace('\u{b}', "\r") } else { value };
        self.splice(&l, a, b, &v, None)?;
        Ok(addr.to_string())
    }

    /// Character runs with their direct properties.
    fn char_runs(&self, l: &TextLoc) -> Vec<RunInfo> {
        let t: Vec<char> = text_of_atom(self.atom(l)).chars().collect();
        let Some(st) = self.style_atom(l) else { return vec![] };
        let (_, cf) = parse_style(&st.data, t.len() + 1);
        let mut out = Vec::new();
        let mut pos = 0;
        for (n, ex) in cf {
            let n = n as usize;
            let text: String = t.iter().skip(pos).take(n).collect();
            if !text.is_empty() {
                out.push(RunInfo { text: to_display(text.trim_end_matches('\r')), props: cf_summary(&ex), start: pos });
            }
            pos += n;
        }
        out
    }

    pub fn style_atom(&self, l: &TextLoc) -> Option<&Rec> {
        let (parent, idx) = l.path.split_at(l.path.len() - 1);
        let p = self.tops[l.top].at(parent);
        p.kids.iter().skip(idx[0] + 1).take_while(|r| !matches!(r.typ, T_TEXT_HEADER | T_SLIDE_PERSIST)).find(|r| r.typ == T_STYLE_TEXT_PROP)
    }

    /// Replaces characters [a, b) of a text atom by `with`, keeping the runs that count
    /// characters right; `style` gives the replaced text new character properties.
    fn splice(&mut self, l: &TextLoc, a: usize, b: usize, with: &str, style: Option<&Props>) -> Result<()> {
        let old: Vec<char> = text_of_atom(self.atom(l)).chars().collect();
        let n_new = with.chars().count();
        let mut chars = old.clone();
        chars.splice(a..b, with.chars());
        let new_text: String = chars.iter().collect();
        // The atom: bytes if every character fits in Latin-1, else UTF-16.
        let latin = new_text.chars().all(|c| (c as u32) < 256 && !(0x80..0xA0).contains(&(c as u32)));
        let (parent_path, idx) = l.path.split_at(l.path.len() - 1);
        let parent = self.tops[l.top].at_mut(parent_path);
        {
            let atom = &mut parent.kids[idx[0]];
            if latin && atom.typ == T_TEXT_BYTES {
                atom.data = new_text.chars().map(|c| c as u8).collect();
            } else {
                atom.typ = T_TEXT_CHARS;
                atom.data = new_text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
            }
        }
        // The records after it that count its characters.
        let delta = n_new as i64 - (b - a) as i64;
        let mut k = idx[0] + 1;
        while k < parent.kids.len() && !matches!(parent.kids[k].typ, T_TEXT_HEADER | T_SLIDE_PERSIST | T_TEXT_CHARS | T_TEXT_BYTES) {
            let r = &mut parent.kids[k];
            match r.typ {
                T_STYLE_TEXT_PROP => r.data = splice_style(&r.data, old.len(), &old, a, b, &chars, n_new, style),
                T_SPECIAL_INFO => r.data = splice_runs(&r.data, old.len() + 1, a, b, n_new, si_size),
                T_MASTER_TEXT_PROP => r.data = splice_runs(&r.data, old.len() + 1, a, b, n_new, |_| 2),
                T_INTERACTIVE_INFO => {
                    let (s, e) = (u32_at(&r.data, 0) as i64, u32_at(&r.data, 4) as i64);
                    let shift = |x: i64| if x >= b as i64 { x + delta } else if x > a as i64 { a as i64 + n_new as i64 } else { x };
                    let (ns, ne) = (shift(s), shift(e));
                    r.data[0..4].copy_from_slice(&(ns as u32).to_le_bytes());
                    r.data[4..8].copy_from_slice(&(ne as u32).to_le_bytes());
                }
                _ => {}
            }
            k += 1;
        }
        Ok(())
    }

    pub fn style(&mut self, addr: &str, props: &Props, matching: Option<(&Finder, Option<usize>)>) -> Result<Vec<String>> {
        props.only(&["bold", "italic", "underline", "size", "color", "clear"], ".ppt text")?;
        let mut done = Vec::new();
        let mut count = 0;
        for (i, para, range) in self.scope(Some(addr))? {
            let l = self.texts[i].clone();
            if self.style_atom(&l).is_none() {
                return Err(err(format!("{}: this text has no style runs to change", self.loc_addr(&l))));
            }
            let t = text_of_atom(self.atom(&l));
            let mut spans = Vec::new();
            let mut pos = 0;
            for (pi, p) in t.split('\r').enumerate() {
                let n = p.chars().count();
                if para.map(|x| x == pi).unwrap_or(true) {
                    match matching {
                        Some((f, nth)) => {
                            for (a, b, _) in f.find(&to_display(p), None) {
                                count += 1;
                                if nth.map(|k| k == count).unwrap_or(true) {
                                    spans.push((pos + a, pos + b));
                                }
                            }
                        }
                        None => {
                            let (ra, rb) = range.unwrap_or((0, n));
                            spans.push((pos + ra.min(n), pos + rb.min(n)));
                        }
                    }
                }
                pos += n + 1;
            }
            for (a, b) in spans {
                let seg: String = t.chars().skip(a).take(b - a).collect();
                self.splice(&l, a, b, &seg, Some(props))?;
                done.push(self.loc_addr(&l));
            }
        }
        if done.is_empty() {
            return Err(err("no text matched"));
        }
        Ok(done)
    }

    pub fn delete(&mut self, addr: &str) -> Result<String> {
        let targets = self.scope(Some(addr))?;
        let (i, para, range) = targets[0];
        let l = self.texts[i].clone();
        let t = text_of_atom(self.atom(&l));
        let total = t.chars().count();
        let (a, b) = match para {
            Some(pi) => {
                let start: usize = t.split('\r').take(pi).map(|p| p.chars().count() + 1).sum();
                let len = t.split('\r').nth(pi).map(|p| p.chars().count()).unwrap_or(0);
                match range {
                    Some((ra, rb)) => (start + ra.min(len), start + rb.min(len)),
                    // The paragraph and its break.
                    None if start + len < total => (start, start + len + 1),
                    None if start > 0 => (start - 1, start + len),
                    None => (start, start + len),
                }
            }
            None => (0, total),
        };
        self.splice(&l, a, b, "", None)?;
        Ok(format!("removed characters {a}..{b} of {}", self.loc_addr(&l)))
    }

    pub fn save(&mut self) -> Result<Vec<u8>> {
        // New places of the top-level records.
        let mut map: HashMap<usize, usize> = HashMap::new();
        let mut pos = 0;
        for r in &self.tops {
            map.insert(r.orig, pos);
            pos += r.size();
        }
        let remap = |v: u32| -> u32 { map.get(&(v as usize)).map(|n| *n as u32).unwrap_or(v) };
        for r in self.tops.iter_mut() {
            match r.typ {
                T_PERSIST_DIR => {
                    let mut i = 0;
                    while i + 4 <= r.data.len() {
                        let n = (u32_at(&r.data, i) >> 20) as usize;
                        for k in 0..n {
                            let at = i + 4 + k * 4;
                            let v = remap(u32_at(&r.data, at));
                            r.data[at..at + 4].copy_from_slice(&v.to_le_bytes());
                        }
                        i += 4 + n * 4;
                    }
                }
                T_USER_EDIT => {
                    for at in [8usize, 12] {
                        let old = u32_at(&r.data, at);
                        if old != 0 || at == 12 {
                            let v = remap(old);
                            r.data[at..at + 4].copy_from_slice(&v.to_le_bytes());
                        }
                    }
                }
                _ => {}
            }
        }
        let mut stream = Vec::with_capacity(pos);
        for r in &self.tops {
            r.write(&mut stream);
        }
        self.cfb.set_stream("PowerPoint Document", stream)?;
        if let Some(cu) = self.cfb.stream("Current User").map(|c| c.to_vec()) {
            let mut cu = cu;
            let v = remap(u32_at(&cu, 16));
            cu[16..20].copy_from_slice(&v.to_le_bytes());
            self.cfb.set_stream("Current User", cu)?;
        }
        Ok(self.cfb.save())
    }
}

fn walk_shapes(r: &Rec, path: &mut Vec<usize>, f: &mut dyn FnMut(&[usize], &Rec)) {
    for (i, k) in r.kids.iter().enumerate() {
        path.push(i);
        if k.typ == T_SP_CONTAINER {
            // A group's own shape record (the first in its group container) is not a shape.
            let is_group_head = k.kids.iter().any(|x| x.typ == T_FSP && u32_at(&x.data, 4) & 0x1 != 0);
            if !is_group_head {
                f(path, k);
            }
        } else if k.container {
            walk_shapes(k, path, f);
        }
        path.pop();
    }
}

fn text_type_name(t: u32) -> &'static str {
    match t {
        0 => "title",
        1 => "body",
        2 => "notes",
        4 => "other",
        5 => "center body",
        6 => "center title",
        7 => "half body",
        8 => "quarter body",
        _ => "text",
    }
}

// ── style runs ──────────────────────────────────────────────────────────────────────────────

pub fn pf_size(d: &[u8], at: usize) -> usize {
    let m = u32_at(d, at);
    let mut n = 4;
    if m & 0xF != 0 {
        n += 2;
    }
    for (bit, sz) in [(0x80, 2), (0x10, 2), (0x40, 2), (0x20, 4), (0x800, 2), (0x1000, 2), (0x2000, 2), (0x4000, 2), (0x100, 2), (0x400, 2), (0x8000, 2)] {
        if m & bit != 0 {
            n += sz;
        }
    }
    if m & 0x100000 != 0 {
        let cnt = u16_at(d, at + n) as usize;
        n += 2 + cnt * 4;
    }
    if m & 0x10000 != 0 {
        n += 2;
    }
    if m & 0xE0000 != 0 {
        n += 2;
    }
    if m & 0x200000 != 0 {
        n += 2;
    }
    n
}

pub fn cf_size(d: &[u8], at: usize) -> usize {
    let m = u32_at(d, at);
    let mut n = 4;
    if m & 0x3EB7 != 0 {
        n += 2;
    }
    for (bit, sz) in [(0x10000, 2), (0x200000, 2), (0x400000, 2), (0x800000, 2), (0x20000, 2), (0x40000, 4), (0x80000, 2), (0x100000, 4), (0x1000000, 2), (0x2000000, 2), (0x4000000, 4)] {
        if m & bit != 0 {
            n += sz;
        }
    }
    n
}

fn si_size(ex: &[u8]) -> usize {
    let m = u32_at(ex, 0);
    let mut n = 4;
    for (bit, sz) in [(0x1, 2), (0x2, 2), (0x4, 2), (0x20, 2), (0x40, 4)] {
        if m & bit != 0 {
            n += sz;
        }
    }
    if m & 0x200 != 0 {
        let cnt = u32_at(ex, n) as usize;
        n += 4 + cnt * 4;
    }
    n
}

/// Paragraph runs (count, indent level + exception bytes) and character runs (count, exception
/// bytes) of a StyleTextPropAtom for a text of `len` characters (counts sum to len + 1).
pub fn parse_style(d: &[u8], len_plus_one: usize) -> (Vec<(u32, Vec<u8>)>, Vec<(u32, Vec<u8>)>) {
    let mut pf = Vec::new();
    let mut cf = Vec::new();
    let mut i = 0;
    let mut covered = 0usize;
    while covered < len_plus_one && i + 6 <= d.len() {
        let n = u32_at(d, i);
        let size = 2 + pf_size(d, i + 6);
        pf.push((n, d[i + 4..(i + 4 + size).min(d.len())].to_vec()));
        i += 4 + size;
        covered += n as usize;
    }
    covered = 0;
    while covered < len_plus_one && i + 4 <= d.len() {
        let n = u32_at(d, i);
        let size = cf_size(d, i + 4);
        cf.push((n, d[i + 4..(i + 4 + size).min(d.len())].to_vec()));
        i += 4 + size;
        covered += n as usize;
    }
    (pf, cf)
}

/// Run-length list → one entry per character, edited, and back.
fn respan<T: Clone + PartialEq>(runs: &[(u32, T)], total: usize, a: usize, b: usize, n_new: usize, merge: bool) -> Vec<(u32, T)> {
    let mut per: Vec<T> = Vec::with_capacity(total + n_new);
    for (n, ex) in runs {
        for _ in 0..*n {
            per.push(ex.clone());
        }
    }
    if per.is_empty() {
        return runs.to_vec();
    }
    while per.len() < total {
        per.push(per.last().unwrap().clone());
    }
    per.truncate(total.max(1));
    let fill = per.get(a).or_else(|| per.last()).cloned().unwrap();
    let a2 = a.min(per.len());
    let b2 = b.min(per.len());
    per.splice(a2..b2, std::iter::repeat(fill).take(n_new));
    let mut out: Vec<(u32, T)> = Vec::new();
    for ex in per {
        match out.last_mut() {
            Some((n, e)) if merge && *e == ex => *n += 1,
            _ => out.push((1, ex)),
        }
    }
    out
}

fn splice_style(d: &[u8], len: usize, old: &[char], a: usize, b: usize, new_chars: &[char], n_new: usize, style: Option<&Props>) -> Vec<u8> {
    let (pf, cf) = parse_style(d, len + 1);
    // Paragraph runs follow the paragraphs (CRs): count each paragraph's characters again.
    let _ = old;
    let mut new_pf: Vec<(u32, Vec<u8>)> = Vec::new();
    {
        let pf_per = respan(&pf, len + 1, a, b, n_new, false);
        // Paragraph style per character, then one run per paragraph (up to and with its CR).
        let mut per: Vec<Vec<u8>> = Vec::new();
        for (n, ex) in &pf_per {
            for _ in 0..*n {
                per.push(ex.clone());
            }
        }
        let mut start = 0;
        for (i, c) in new_chars.iter().chain(std::iter::once(&'\r')).enumerate() {
            if *c == '\r' {
                let ex = per.get(start).or_else(|| per.last()).cloned().unwrap_or_default();
                new_pf.push(((i + 1 - start) as u32, ex));
                start = i + 1;
            }
        }
        if new_pf.is_empty() {
            new_pf.push((1, per.first().cloned().unwrap_or_default()));
        }
    }
    let mut new_cf = respan(&cf, len + 1, a, b, n_new, false);
    if let Some(p) = style {
        // The range [a, a + n_new) gets the new properties.
        let mut per: Vec<Vec<u8>> = Vec::new();
        for (n, ex) in &new_cf {
            for _ in 0..*n {
                per.push(ex.clone());
            }
        }
        for ex in per.iter_mut().skip(a).take(n_new) {
            *ex = set_cf(ex, p);
        }
        new_cf.clear();
        for ex in per {
            match new_cf.last_mut() {
                Some((n, e)) if *e == ex => *n += 1,
                _ => new_cf.push((1, ex)),
            }
        }
    } else {
        let mut merged: Vec<(u32, Vec<u8>)> = Vec::new();
        for (n, ex) in new_cf {
            match merged.last_mut() {
                Some((m, e)) if *e == ex => *m += n,
                _ => merged.push((n, ex)),
            }
        }
        new_cf = merged;
    }
    let mut out = Vec::new();
    for (n, ex) in new_pf.iter().chain(new_cf.iter()) {
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(ex);
    }
    // Whatever followed the runs (rare) stays.
    let consumed: usize = pf.iter().map(|(_, e)| 4 + e.len()).sum::<usize>() + cf.iter().map(|(_, e)| 4 + e.len()).sum::<usize>();
    if consumed < d.len() {
        out.extend_from_slice(&d[consumed..]);
    }
    out
}

/// Runs of (count u32, exception) for TextSpecialInfo and MasterTextProp atoms.
fn splice_runs(d: &[u8], total: usize, a: usize, b: usize, n_new: usize, size: impl Fn(&[u8]) -> usize) -> Vec<u8> {
    let mut runs = Vec::new();
    let mut i = 0;
    let mut covered = 0;
    while covered < total && i + 4 <= d.len() {
        let n = u32_at(d, i);
        let sz = size(&d[i + 4..]).min(d.len() - i - 4);
        runs.push((n, d[i + 4..i + 4 + sz].to_vec()));
        i += 4 + sz;
        covered += n as usize;
    }
    let new = respan(&runs, total, a, b, n_new, true);
    let mut out = Vec::new();
    for (n, ex) in new {
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&ex);
    }
    out.extend_from_slice(&d[i.min(d.len())..]);
    out
}

/// A TextCFException with bold/italic/underline, size or color set.
fn set_cf(ex: &[u8], p: &Props) -> Vec<u8> {
    let m = u32_at(ex, 0);
    // Read the present fields.
    let mut fields: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut at = 4;
    let mut style_flags: Option<u16> = None;
    if m & 0x3EB7 != 0 {
        style_flags = Some(u16_at(ex, at));
        at += 2;
    }
    for (bit, sz) in [(0x10000, 2), (0x200000, 2), (0x400000, 2), (0x800000, 2), (0x20000, 2), (0x40000, 4), (0x80000, 2), (0x100000, 4), (0x1000000, 2), (0x2000000, 2), (0x4000000, 4)] {
        if m & bit != 0 {
            fields.push((bit, ex[at..at + sz].to_vec()));
            at += sz;
        }
    }
    let mut mask = m;
    if p.clear {
        mask &= !0x3EB7;
        style_flags = None;
        fields.retain(|f| f.0 != 0x20000 && f.0 != 0x40000);
        mask &= !(0x20000 | 0x40000);
    }
    let mut flags = style_flags.unwrap_or(0);
    for (on, bit) in [(p.bold, 0x1u16), (p.italic, 0x2), (p.underline.as_ref().map(|u| u != "none"), 0x4)] {
        if let Some(v) = on {
            mask |= bit as u32;
            if v {
                flags |= bit;
            } else {
                flags &= !bit;
            }
        }
    }
    if let Some(sz) = p.size {
        mask |= 0x20000;
        fields.retain(|f| f.0 != 0x20000);
        fields.push((0x20000, (sz.round() as u16).to_le_bytes().to_vec()));
    }
    if let Some(Color::Rgb(h)) = &p.color {
        mask |= 0x40000;
        let v = u32::from_str_radix(h, 16).unwrap_or(0);
        // ColorIndexStruct: red, green, blue, index 0xFE (an RGB color).
        let bytes = vec![(v >> 16) as u8, (v >> 8) as u8, v as u8, 0xFE];
        fields.retain(|f| f.0 != 0x40000);
        fields.push((0x40000, bytes));
    }
    let order = [0x10000u32, 0x200000, 0x400000, 0x800000, 0x20000, 0x40000, 0x80000, 0x100000, 0x1000000, 0x2000000, 0x4000000];
    fields.sort_by_key(|f| order.iter().position(|o| *o == f.0).unwrap_or(99));
    let mut out = mask.to_le_bytes().to_vec();
    if mask & 0x3EB7 != 0 {
        out.extend_from_slice(&flags.to_le_bytes());
    }
    for (_, b) in fields {
        out.extend_from_slice(&b);
    }
    out
}

fn cf_summary(ex: &[u8]) -> Vec<String> {
    let m = u32_at(ex, 0);
    let mut v = Vec::new();
    let mut at = 4;
    if m & 0x3EB7 != 0 {
        let f = u16_at(ex, at);
        if m & 1 != 0 && f & 1 != 0 {
            v.push("bold".into());
        }
        if m & 2 != 0 && f & 2 != 0 {
            v.push("italic".into());
        }
        if m & 4 != 0 && f & 4 != 0 {
            v.push("underline".into());
        }
        at += 2;
    }
    for (bit, sz) in [(0x10000u32, 2usize), (0x200000, 2), (0x400000, 2), (0x800000, 2), (0x20000, 2), (0x40000, 4)] {
        if m & bit != 0 {
            if bit == 0x20000 {
                v.push(format!("size={}pt", u16_at(ex, at)));
            }
            if bit == 0x40000 && ex.get(at + 3) == Some(&0xFE) {
                v.push(format!("color={:02X}{:02X}{:02X}", ex[at], ex[at + 1], ex[at + 2]));
            }
            at += sz;
        }
    }
    v
}

#[allow(dead_code)]
const SHAPE_RECORDS: [u16; 5] = [T_SLIDE, T_NOTES, T_PPDRAWING, T_CLIENT_ANCHOR, T_CHILD_ANCHOR];
