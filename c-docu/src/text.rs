//! Editing the text of a paragraph in place, across the runs that hold it. A paragraph is a list
//! of segments (pieces of text nodes, and atoms such as tabs, breaks and pictures) with their
//! character offsets; a replacement rewrites only the text nodes a match covers, and a style
//! change splits the runs at the edges of its range, so every other run, attribute and byte of the
//! paragraph stays as it was. The dialects (WordprocessingML, DrawingML, SpreadsheetML rich text,
//! OWPML) say what a run, a text element and an atom are.

use crate::xml::{Element, Node};
use crate::{err, Result};

#[derive(Clone, Debug)]
pub struct Seg {
    /// Path (child indices) from the paragraph to the run.
    pub run: Vec<usize>,
    /// Path inside the run to the text node or atom element; empty when the run itself is the
    /// atom (a DrawingML `a:br`).
    pub inner: Vec<usize>,
    pub atom: bool,
    /// Offset and length in characters.
    pub start: usize,
    pub len: usize,
    pub text: String,
    /// How an atom is shown (`[image]`, `[^1]`); its `text` is what searches see.
    pub display: Option<String>,
}

impl Seg {
    pub fn end(&self) -> usize {
        self.start + self.len
    }
}

pub trait Dialect {
    /// A run: an element holding text elements and atoms under one set of properties.
    fn is_run(&self, e: &Element) -> bool;
    /// An element in a run that holds text (`w:t`).
    fn is_text(&self, e: &Element) -> bool;
    /// An element in a run that stands for a character (`w:tab`), with how it is shown.
    fn atom(&self, e: &Element) -> Option<(char, Option<String>)>;
    /// A run that is itself a character (DrawingML's `a:br`).
    fn run_atom(&self, _e: &Element) -> Option<char> {
        None
    }
    /// An element at paragraph level whose children are looked into (`w:hyperlink`).
    fn descend(&self, e: &Element) -> bool;
    /// Whether this child of a run is its properties element (copied to both halves of a split).
    fn is_props(&self, e: &Element) -> bool;
    /// A run that must not be split (a field): a style change takes it whole.
    fn unsplittable(&self, _run: &Element) -> bool {
        false
    }
    /// A text element for `text`, made like `like` (a sibling text element) when given.
    fn new_text(&self, run: &Element, text: &str) -> Element;
    /// A new run holding `text`, for an empty paragraph.
    fn new_run(&self, para: &Element, text: &str) -> Element;
    /// Where a new run goes in an empty paragraph (index in its children).
    fn new_run_at(&self, para: &Element) -> usize {
        para.children.len()
    }
    /// After the text of a text element changed: fix what the text needs (`xml:space`), and turn
    /// tabs and line breaks into the dialect's elements. Gets the run's parent and the run's
    /// index, and the index of the text element in the run.
    fn settle(&self, parent: &mut Element, run: usize, text_el: usize);
    /// After a text element's text changed in place: what the text needs to be kept as it is
    /// (`xml:space="preserve"` for spaces at its ends).
    fn fix_space(&self, _text_el: &mut Element) {}
    /// Whether a run holds nothing that shows (only properties) and can go.
    fn run_is_empty(&self, run: &Element) -> bool {
        run.elements().all(|c| self.is_props(c)) && run.children.iter().all(|n| !matches!(n, Node::Text(t) if !t.is_whitespace()))
    }
}

pub fn at<'a>(root: &'a Element, path: &[usize]) -> &'a Element {
    let mut e = root;
    for &i in path {
        e = e.children[i].as_element().expect("path to an element");
    }
    e
}

pub fn at_mut<'a>(root: &'a mut Element, path: &[usize]) -> &'a mut Element {
    let mut e = root;
    for &i in path {
        e = e.children[i].as_element_mut().expect("path to an element");
    }
    e
}

pub fn segments(d: &dyn Dialect, para: &Element) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut pos = 0;
    let mut path = Vec::new();
    collect(d, para, &mut path, &mut out, &mut pos);
    out
}

fn collect(d: &dyn Dialect, el: &Element, path: &mut Vec<usize>, out: &mut Vec<Seg>, pos: &mut usize) {
    for (i, n) in el.children.iter().enumerate() {
        let Node::Element(c) = n else { continue };
        path.push(i);
        if d.is_run(c) {
            collect_run(d, c, path, out, pos);
        } else if let Some(ch) = d.run_atom(c) {
            out.push(Seg { run: path.clone(), inner: Vec::new(), atom: true, start: *pos, len: 1, text: ch.to_string(), display: None });
            *pos += 1;
        } else if d.descend(c) {
            collect(d, c, path, out, pos);
        }
        path.pop();
    }
}

fn collect_run(d: &dyn Dialect, run: &Element, path: &[usize], out: &mut Vec<Seg>, pos: &mut usize) {
    for (k, n) in run.children.iter().enumerate() {
        let Node::Element(c) = n else { continue };
        if d.is_text(c) {
            for (j, m) in c.children.iter().enumerate() {
                match m {
                    Node::Text(t) => {
                        let len = t.get().chars().count();
                        if len > 0 {
                            out.push(Seg { run: path.to_vec(), inner: vec![k, j], atom: false, start: *pos, len, text: t.get().to_string(), display: None });
                            *pos += len;
                        }
                    }
                    Node::Element(a) => {
                        if let Some((ch, display)) = d.atom(a) {
                            out.push(Seg { run: path.to_vec(), inner: vec![k, j], atom: true, start: *pos, len: 1, text: ch.to_string(), display });
                            *pos += 1;
                        }
                    }
                    _ => {}
                }
            }
        } else if let Some((ch, display)) = d.atom(c) {
            out.push(Seg { run: path.to_vec(), inner: vec![k], atom: true, start: *pos, len: 1, text: ch.to_string(), display });
            *pos += 1;
        }
    }
}

pub fn text_of(d: &dyn Dialect, para: &Element) -> String {
    segments(d, para).iter().map(|s| s.text.as_str()).collect()
}

/// The text as `read` shows it: atoms by their display form.
pub fn display_of(d: &dyn Dialect, para: &Element) -> String {
    segments(d, para).iter().map(|s| s.display.as_deref().unwrap_or(&s.text)).collect()
}

fn char_slice(s: &str, from: usize, to: usize) -> String {
    s.chars().skip(from).take(to.saturating_sub(from)).collect()
}

/// Sets the text node a segment is in, and what its text element needs for it.
fn set_seg_text(d: &dyn Dialect, para: &mut Element, seg: &Seg, text: &str) {
    let run = at_mut(para, &seg.run);
    if seg.inner.len() != 2 {
        return;
    }
    let Some(te) = run.children[seg.inner[0]].as_element_mut() else { return };
    if let Node::Text(t) = &mut te.children[seg.inner[1]] {
        t.set(text);
    }
    d.fix_space(te);
}

/// Replaces the characters [from, to) of the paragraph by `with`, which takes the properties of
/// the run where the range starts. Returns whether anything changed.
pub fn replace_range(d: &dyn Dialect, para: &mut Element, from: usize, to: usize, with: &str) -> Result<bool> {
    let segs = segments(d, para);
    let total: usize = segs.last().map(|s| s.end()).unwrap_or(0);
    if from > to || to > total {
        return Err(err(format!("character range {from}..{to} is outside the paragraph (length {total})")));
    }
    if from == to && with.is_empty() {
        return Ok(false);
    }
    if segs.is_empty() {
        let mut run = d.new_run(para, "");
        let t = d.new_text(&run, with);
        run.children.push(Node::Element(t));
        let k = run.children.len() - 1;
        let at = d.new_run_at(para);
        para.children.insert(at, Node::Element(run));
        d.settle(para, at, k);
        return Ok(true);
    }
    if from == to {
        // An insertion: into the text before the point (its properties), else the text or atom
        // after it.
        let target = segs
            .iter()
            .position(|s| !s.atom && s.start < from && from <= s.end())
            .or_else(|| segs.iter().position(|s| s.start == from));
        match target {
            Some(i) if !segs[i].atom => {
                let s = &segs[i];
                let k = from - s.start;
                let new = format!("{}{}{}", char_slice(&s.text, 0, k), with, char_slice(&s.text, k, s.len));
                set_seg_text(d, para, s, &new);
                settle_seg(d, para, &s.run, s.inner[0]);
            }
            Some(i) => insert_before_atom(d, para, &segs[i].clone(), with)?,
            None => insert_after_atom(d, para, &segs[segs.len() - 1].clone(), with)?,
        }
        return Ok(true);
    }
    let first = segs.iter().position(|s| s.start < to && s.end() > from).unwrap();
    let covered: Vec<usize> = (first + 1..segs.len()).filter(|&i| segs[i].start < to && segs[i].end() > from).collect();
    let mut runs: Vec<Vec<usize>> = Vec::new();
    let mut emptied: Vec<(Vec<usize>, usize)> = Vec::new();
    for &i in covered.iter().rev() {
        let s = &segs[i];
        runs.push(s.run.clone());
        if s.atom {
            remove_atom(para, s);
        } else {
            let keep_from = to.saturating_sub(s.start).min(s.len);
            let rest = char_slice(&s.text, keep_from, s.len);
            set_seg_text(d, para, s, &rest);
            if rest.is_empty() {
                emptied.push((s.run.clone(), s.inner[0]));
            }
        }
    }
    let f = segs[first].clone();
    let mut settle_first = None;
    if f.atom {
        if with.is_empty() {
            remove_atom(para, &f);
            runs.push(f.run.clone());
        } else {
            settle_first = Some(replace_atom_with_text(d, para, &f, with));
        }
    } else {
        let a = from.saturating_sub(f.start).min(f.len);
        let b = to.saturating_sub(f.start).min(f.len);
        let new = format!("{}{}{}", char_slice(&f.text, 0, a), with, char_slice(&f.text, b, f.len));
        set_seg_text(d, para, &f, &new);
        if new.is_empty() {
            emptied.push((f.run.clone(), f.inner[0]));
            runs.push(f.run.clone());
        } else {
            settle_first = Some((f.run.clone(), f.inner[0]));
        }
    }
    cleanup(d, para, runs, emptied);
    if let Some((run, k)) = settle_first {
        settle_seg(d, para, &run, k);
    }
    Ok(true)
}

fn settle_seg(d: &dyn Dialect, para: &mut Element, run: &[usize], text_el: usize) {
    let (pp, ri) = run.split_at(run.len() - 1);
    let parent = at_mut(para, pp);
    d.settle(parent, ri[0], text_el);
}

fn remove_atom(para: &mut Element, s: &Seg) {
    if s.inner.is_empty() {
        let (pp, ri) = s.run.split_at(s.run.len() - 1);
        let parent = at_mut(para, pp);
        parent.children[ri[0]] = Node::Other(String::new());
        return;
    }
    let run = at_mut(para, &s.run);
    if s.inner.len() == 1 {
        run.children[s.inner[0]] = Node::Other(String::new());
    } else if let Some(te) = run.children[s.inner[0]].as_element_mut() {
        te.children[s.inner[1]] = Node::Other(String::new());
    }
}

/// Puts text where an atom was (no indices move); returns where to settle it.
fn replace_atom_with_text(d: &dyn Dialect, para: &mut Element, s: &Seg, with: &str) -> (Vec<usize>, usize) {
    if s.inner.is_empty() {
        let mut run = d.new_run(para, "");
        let t = d.new_text(&run, with);
        run.children.push(Node::Element(t));
        let k = run.children.len() - 1;
        let (pp, ri) = s.run.split_at(s.run.len() - 1);
        at_mut(para, pp).children[ri[0]] = Node::Element(run);
        return (s.run.clone(), k);
    }
    let run = at_mut(para, &s.run);
    if s.inner.len() == 1 {
        let t = d.new_text(run, with);
        run.children[s.inner[0]] = Node::Element(t);
    } else if let Some(te) = run.children[s.inner[0]].as_element_mut() {
        te.children[s.inner[1]] = Node::Text(crate::xml::Text::new(with));
    }
    (s.run.clone(), s.inner[0])
}

fn insert_before_atom(d: &dyn Dialect, para: &mut Element, atom: &Seg, with: &str) -> Result<()> {
    insert_by_atom(d, para, atom, with, 0)
}

fn insert_after_atom(d: &dyn Dialect, para: &mut Element, atom: &Seg, with: &str) -> Result<()> {
    insert_by_atom(d, para, atom, with, 1)
}

fn insert_by_atom(d: &dyn Dialect, para: &mut Element, atom: &Seg, with: &str, after: usize) -> Result<()> {
    let (pp, ri) = atom.run.split_at(atom.run.len() - 1);
    let ri = ri[0];
    if atom.inner.is_empty() {
        // A run-level atom: a new run beside it.
        let mut run = d.new_run(para, "");
        let t = d.new_text(&run, with);
        run.children.push(Node::Element(t));
        let k = run.children.len() - 1;
        let parent = at_mut(para, pp);
        parent.children.insert(ri + after, Node::Element(run));
        d.settle(parent, ri + after, k);
        return Ok(());
    }
    let run = at_mut(para, &atom.run);
    let k = atom.inner[0];
    if atom.inner.len() == 1 {
        let t = d.new_text(run, with);
        run.children.insert(k + after, Node::Element(t));
        let parent = at_mut(para, pp);
        d.settle(parent, ri, k + after);
    } else if let Some(te) = run.children[k].as_element_mut() {
        te.children.insert(atom.inner[1] + after, Node::Text(crate::xml::Text::new(with)));
        let parent = at_mut(para, pp);
        d.settle(parent, ri, k);
    }
    Ok(())
}

/// Drops what an edit emptied: the atoms it removed, the text elements it emptied, and the runs
/// (of those it touched) with nothing left in them.
fn cleanup(d: &dyn Dialect, para: &mut Element, mut runs: Vec<Vec<usize>>, emptied: Vec<(Vec<usize>, usize)>) {
    for (run, k) in &emptied {
        let r = at_mut(para, run);
        let empty = r.children[*k]
            .as_element()
            .map(|t| t.children.iter().all(|c| matches!(c, Node::Text(x) if x.get().is_empty()) || matches!(c, Node::Other(s) if s.is_empty())))
            .unwrap_or(false);
        if empty {
            r.children[*k] = Node::Other(String::new());
        }
    }
    runs.sort();
    runs.dedup();
    // Last first, so removing one does not move the others.
    runs.sort_by(|a, b| b.cmp(a));
    for path in &runs {
        let (pp, ri) = path.split_at(path.len() - 1);
        let parent = at_mut(para, pp);
        let Some(n) = parent.children.get_mut(ri[0]) else { continue };
        match n {
            Node::Other(s) if s.is_empty() => {
                parent.children.remove(ri[0]);
            }
            Node::Element(run) => {
                run.children.retain(|c| !matches!(c, Node::Other(s) if s.is_empty()));
                for c in run.elements_mut() {
                    if d.is_text(c) {
                        c.children.retain(|x| !matches!(x, Node::Other(s) if s.is_empty()));
                    }
                }
                if d.run_is_empty(run) {
                    parent.children.remove(ri[0]);
                }
            }
            _ => {}
        }
    }
}

/// Makes `at` a run boundary (splits the run that holds it); returns false when it already is
/// one or the run cannot be split.
pub fn split_at(d: &dyn Dialect, para: &mut Element, at_char: usize) -> bool {
    let segs = segments(d, para);
    let Some(s) = segs.iter().find(|s| s.start < at_char && at_char < s.end()).cloned().or_else(|| {
        // A boundary between two segments of the same run.
        let i = segs.iter().position(|s| s.start == at_char)?;
        if i > 0 && segs[i - 1].run == segs[i].run && !segs[i].inner.is_empty() {
            Some(segs[i].clone())
        } else {
            None
        }
    }) else {
        return false;
    };
    let (pp, ri) = s.run.split_at(s.run.len() - 1);
    let ri = ri[0];
    let parent = at_mut(para, pp);
    let run = parent.children[ri].as_element().unwrap();
    if d.unsplittable(run) || s.inner.is_empty() {
        return false;
    }
    let offset = at_char - s.start; // 0 when splitting before this segment
    let mut left = run.clone();
    let mut right = run.clone();
    let k = s.inner[0];
    // Left keeps children ..k (and part of k); right keeps the properties and k.. .
    left.children.truncate(k + 1);
    right.children = run
        .children
        .iter()
        .enumerate()
        .filter(|(i, n)| *i >= k || n.as_element().map(|e| d.is_props(e)).unwrap_or(false))
        .map(|(_, n)| n.clone())
        .collect();
    let kr = right.children.len() - (run.children.len() - k); // index of k in right
    if s.inner.len() == 2 {
        let j = s.inner[1];
        // Split text element k at node j, character `offset`.
        let te = run.children[k].as_element().unwrap();
        let mut lt = te.clone();
        let mut rt = te.clone();
        lt.children.truncate(j + 1);
        rt.children = te.children[j..].to_vec();
        if let Node::Text(t) = &mut lt.children[j] {
            let v: String = t.get().chars().take(offset).collect();
            t.set(&v);
        }
        if let Node::Text(t) = &mut rt.children[0] {
            let v: String = t.get().chars().skip(offset).collect();
            t.set(&v);
        }
        let l_empty = lt.children.iter().all(|c| matches!(c, Node::Text(x) if x.get().is_empty()));
        left.children[k] = Node::Element(lt);
        right.children[kr] = Node::Element(rt);
        if l_empty {
            left.children.remove(k);
        }
    } else {
        // An atom: it goes right.
        left.children.truncate(k);
    }
    parent.children[ri] = Node::Element(left);
    parent.children.insert(ri + 1, Node::Element(right));
    // Settle both texts (xml:space).
    for r in [ri, ri + 1] {
        let n = at(parent, &[r]).children.len();
        for k in 0..n {
            if at(parent, &[r]).children[k].as_element().map(|e| d.is_text(e)).unwrap_or(false) {
                d.settle(parent, r, k);
            }
        }
    }
    true
}

/// Paths (from the paragraph) of the runs that cover [from, to) exactly, after splitting the runs
/// at its edges. Run-level atoms in the range are included.
pub fn runs_in(d: &dyn Dialect, para: &mut Element, from: usize, to: usize) -> Vec<Vec<usize>> {
    if from < to {
        split_at(d, para, to);
        split_at(d, para, from);
    }
    let segs = segments(d, para);
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for s in &segs {
        let inside = if from == to { s.start <= from && from < s.end() } else { s.start < to && s.end() > from };
        if inside && !runs.contains(&s.run) {
            runs.push(s.run.clone());
        }
    }
    runs
}

/// Finds `pattern` in `text`: (char start, char end, replacement) for each match.
pub struct Finder {
    re: regex::Regex,
    literal: Option<String>,
}

impl Finder {
    pub fn new(pattern: &str, regex: bool, ignore_case: bool, whole_word: bool) -> Result<Finder> {
        if pattern.is_empty() {
            return Err(err("empty search pattern"));
        }
        let mut p = if regex { pattern.to_string() } else { regex::escape(pattern) };
        if whole_word {
            p = format!(r"\b(?:{p})\b");
        }
        if ignore_case {
            p = format!("(?i){p}");
        }
        let re = regex::Regex::new(&p).map_err(|e| err(format!("bad pattern: {e}")))?;
        Ok(Finder { re, literal: if regex { None } else { Some(pattern.to_string()) } })
    }

    /// Matches as (char start, char end, the replacement for `with`: `$1` expanded for a regex).
    pub fn find(&self, text: &str, with: Option<&str>) -> Vec<(usize, usize, String)> {
        let mut out = Vec::new();
        for caps in self.re.captures_iter(text) {
            let m = caps.get(0).unwrap();
            if m.start() == m.end() {
                continue;
            }
            let a = text[..m.start()].chars().count();
            let b = a + text[m.start()..m.end()].chars().count();
            let rep = match with {
                Some(w) if self.literal.is_none() => {
                    let mut s = String::new();
                    caps.expand(w, &mut s);
                    s
                }
                Some(w) => w.to_string(),
                None => String::new(),
            };
            out.push((a, b, rep));
        }
        out
    }
}

/// Backslash escapes in command-line text: `\n`, `\t`, `\\` (others stay as written).
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.peek() {
                Some('n') => {
                    out.push('\n');
                    it.next();
                }
                Some('t') => {
                    out.push('\t');
                    it.next();
                }
                Some('\\') => {
                    out.push('\\');
                    it.next();
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Text for display on one line: tabs and breaks escaped.
pub fn escape_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            '\u{ad}' => out.push_str("\\-"),
            c => out.push(c),
        }
    }
    out
}

/// The paragraphs of inserted text: one per line, blank lines between them dropped (a lone empty
/// text is one empty paragraph).
pub fn lines_of(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.split('\n').filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        vec![""]
    } else {
        lines
    }
}
