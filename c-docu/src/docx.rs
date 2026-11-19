//! Word documents (.docx, .docm, .dotx, .dotm): WordprocessingML, edited in place. Paragraphs
//! are addressed in each story (the body, every header, footer, footnote, endnote and comment),
//! tables by their order, cells by row and column; a replacement or style change rewrites only
//! the runs it covers.

use crate::model::{Item, Match, Outline, RunInfo};
use crate::opc::{Package, Rels};
use crate::props::{Color, Edges, LineSpacing, Props, Vert};
use crate::text::{self, at, at_mut, Dialect, Finder};
use crate::xml::{self, put_in_order, Document, Element, Node, MC, R, W};
use crate::{err, Result};
use std::collections::HashMap;

pub struct Wml;

impl Dialect for Wml {
    fn is_run(&self, e: &Element) -> bool {
        e.is(W, "r")
    }
    fn is_text(&self, e: &Element) -> bool {
        e.is(W, "t")
    }
    fn atom(&self, e: &Element) -> Option<(char, Option<String>)> {
        if e.ns() == MC && e.local() == "AlternateContent" {
            return Some(('\u{FFFC}', Some(object_label(e))));
        }
        if e.ns() != W {
            return None;
        }
        Some(match e.local() {
            "tab" | "ptab" => ('\t', None),
            "br" => match e.own("type") {
                Some("page") => ('\u{c}', Some("\\f".into())),
                Some("column") => ('\u{c}', Some("\\f".into())),
                _ => ('\n', None),
            },
            "cr" => ('\n', None),
            "noBreakHyphen" => ('\u{2011}', None),
            "softHyphen" => ('\u{ad}', None),
            "sym" => {
                let c = e.own("char").and_then(|h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32).unwrap_or('\u{FFFD}');
                (c, None)
            }
            "drawing" | "pict" | "object" => ('\u{FFFC}', Some(object_label(e))),
            "footnoteReference" => ('\u{FFFC}', Some(format!("[^fn{}]", e.own("id").unwrap_or("?")))),
            "endnoteReference" => ('\u{FFFC}', Some(format!("[^en{}]", e.own("id").unwrap_or("?")))),
            _ => return None,
        })
    }
    fn descend(&self, e: &Element) -> bool {
        if e.ns() == MC {
            return matches!(e.local(), "AlternateContent" | "Choice");
        }
        e.ns() == W && matches!(e.local(), "hyperlink" | "smartTag" | "customXml" | "sdt" | "sdtContent" | "ins" | "moveTo" | "fldSimple" | "bdo" | "dir")
    }
    fn is_props(&self, e: &Element) -> bool {
        e.is(W, "rPr")
    }
    fn new_text(&self, run: &Element, text: &str) -> Element {
        let mut t = Element::new(&run.qualify("t"), W);
        t.set_text(text);
        t
    }
    fn new_run(&self, para: &Element, _text: &str) -> Element {
        let mut r = para.sibling("r");
        // An empty paragraph's text takes the paragraph mark's properties.
        if let Some(rpr) = para.child(W, "pPr").and_then(|p| p.child(W, "rPr")) {
            let mut c = rpr.clone();
            c.remove_children(W, "ins");
            c.remove_children(W, "del");
            c.remove_children(W, "rPrChange");
            c.touch();
            r.push(c);
        } else if let Some(first) = first_run(para) {
            if let Some(rpr) = first.child(W, "rPr") {
                r.push(rpr.clone());
            }
        }
        r
    }
    fn new_run_at(&self, para: &Element) -> usize {
        // After the paragraph properties, before a trailing bookmark end is fine: at the end.
        para.children.len()
    }
    fn fix_space(&self, t: &mut Element) {
        space(t);
    }
    fn settle(&self, parent: &mut Element, run: usize, text_el: usize) {
        let Some(r) = parent.children[run].as_element_mut() else { return };
        let Some(t) = r.children.get(text_el).and_then(|n| n.as_element()) else { return };
        let s = t.text();
        if s.contains(['\t', '\n', '\u{c}']) {
            // Tabs and breaks become their elements.
            let mut pieces: Vec<Element> = Vec::new();
            let mut cur = String::new();
            let flush = |cur: &mut String, pieces: &mut Vec<Element>| {
                if !cur.is_empty() {
                    let mut t = Element::new(&r_qual(r, "t"), W);
                    t.set_text(cur);
                    space(&mut t);
                    pieces.push(t);
                    cur.clear();
                }
            };
            for c in s.chars() {
                match c {
                    '\t' => {
                        flush(&mut cur, &mut pieces);
                        pieces.push(Element::new(&r_qual(r, "tab"), W));
                    }
                    '\n' => {
                        flush(&mut cur, &mut pieces);
                        pieces.push(Element::new(&r_qual(r, "br"), W));
                    }
                    '\u{c}' => {
                        flush(&mut cur, &mut pieces);
                        let mut b = Element::new(&r_qual(r, "br"), W);
                        b.set_own("type", "page");
                        pieces.push(b);
                    }
                    c => cur.push(c),
                }
            }
            flush(&mut cur, &mut pieces);
            r.children.splice(text_el..=text_el, pieces.into_iter().map(Node::Element));
        } else if let Some(t) = r.children[text_el].as_element_mut() {
            space(t);
        }
    }
}

fn r_qual(r: &Element, local: &str) -> String {
    r.qualify(local)
}

/// `xml:space="preserve"` where Word would drop spaces otherwise.
fn space(t: &mut Element) {
    let s = t.text();
    let needs = s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace) || s.contains("  ");
    if needs {
        t.set_attr("xml:space", xml::XML, "preserve");
    }
}

fn object_label(e: &Element) -> String {
    let d = e.descendants();
    if d.iter().any(|x| x.local() == "txbxContent") {
        "[text box]".into()
    } else if d.iter().any(|x| x.is(xml::C, "chart") || x.local() == "chart") {
        "[chart]".into()
    } else if d.iter().any(|x| x.local() == "blip" || x.local() == "imagedata") {
        "[image]".into()
    } else if e.local() == "object" || d.iter().any(|x| x.local() == "OLEObject") {
        "[object]".into()
    } else {
        "[drawing]".into()
    }
}

fn first_run(e: &Element) -> Option<&Element> {
    for c in e.elements() {
        if c.is(W, "r") {
            return Some(c);
        }
        if Wml.descend(c) {
            if let Some(r) = first_run(c) {
                return Some(r);
            }
        }
    }
    None
}

#[derive(Clone, Debug)]
pub struct Story {
    /// `body`, `hdr1`, `ftr2`, `fn3`, `en1`, `cmt0`
    pub id: String,
    pub label: String,
    pub part: usize,
    /// Path from the part's root element to the story's element.
    pub path: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct ParaLoc {
    pub story: usize,
    /// Path from the story element.
    pub path: Vec<usize>,
    /// 1-based in the story; 0 for the fallback copies of text boxes (edited, not listed).
    pub index: usize,
    pub cell: Option<(usize, usize, usize)>,
    pub textbox: bool,
}

#[derive(Clone, Debug)]
pub struct TableLoc {
    pub story: usize,
    pub path: Vec<usize>,
    pub index: usize,
    pub rows: Vec<RowLoc>,
    pub nested: bool,
}

#[derive(Clone, Debug)]
pub struct RowLoc {
    pub path: Vec<usize>,
    pub cells: Vec<Vec<usize>>,
}

pub struct Part {
    pub name: String,
    pub doc: Document,
}

pub struct StyleInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub based_on: Option<String>,
    /// The style a new paragraph after one of this style takes (`w:next`).
    pub next: Option<String>,
    pub default: bool,
}

pub struct Docx {
    pub pkg: Package,
    pub parts: Vec<Part>,
    pub stories: Vec<Story>,
    pub styles: Vec<StyleInfo>,
    pub rels: Rels,
    pub kind: &'static str,
    pub warnings: Vec<String>,
}

/// What an address names.
#[derive(Clone, Debug)]
pub enum Target {
    Paras { story: usize, paras: Vec<ParaLoc>, range: Option<(usize, usize)> },
    Table { story: usize, table: TableLoc },
    Row { story: usize, table: TableLoc, row: usize },
    Cell { story: usize, table: TableLoc, row: usize, col: usize },
    Story { story: usize },
}

impl Docx {
    pub fn open(pkg: Package, main: &str) -> Result<Docx> {
        let ct = pkg.content_type(main).unwrap_or_default();
        let kind = if ct.contains("template") {
            "dotx"
        } else if ct.contains("macroEnabled") {
            "docm"
        } else {
            "docx"
        };
        let doc = pkg.xml(main)?;
        let rels = pkg.rels(main)?;
        let mut d = Docx { pkg, parts: vec![Part { name: main.to_string(), doc }], stories: Vec::new(), styles: Vec::new(), rels, kind, warnings: Vec::new() };
        d.load_stories()?;
        d.load_styles();
        Ok(d)
    }

    fn part_index(&mut self, name: &str) -> Result<usize> {
        if let Some(i) = self.parts.iter().position(|p| p.name == name) {
            return Ok(i);
        }
        let doc = self.pkg.xml(name)?;
        self.parts.push(Part { name: name.to_string(), doc });
        Ok(self.parts.len() - 1)
    }

    fn load_stories(&mut self) -> Result<()> {
        let body_idx = self.parts[0].doc.root.position(W, "body").ok_or_else(|| err("document.xml has no w:body"))?;
        self.stories.push(Story { id: "body".into(), label: "body".into(), part: 0, path: vec![body_idx] });
        // Headers and footers in the order the sections name them.
        let mut refs: Vec<(String, String, usize)> = Vec::new(); // (kind, rId, section)
        let body = self.parts[0].doc.root.children[body_idx].as_element().unwrap();
        let mut section = 1;
        for e in body.descendants() {
            if e.is(W, "sectPr") {
                for r in e.elements() {
                    if r.is(W, "headerReference") || r.is(W, "footerReference") {
                        let id = r.attr_ns(R, "id").unwrap_or("").to_string();
                        let t = r.own("type").unwrap_or("default").to_string();
                        refs.push((format!("{}|{}", r.local(), t), id, section));
                    }
                }
                section += 1;
            }
        }
        let (mut nh, mut nf) = (0, 0);
        let mut seen: Vec<String> = Vec::new();
        for (kind, id, section) in refs {
            let Some(part) = self.rels.part(&id) else { continue };
            let Some(part) = self.pkg.zip.real_name(&part) else { continue };
            if seen.contains(&part) {
                continue;
            }
            seen.push(part.clone());
            let pi = self.part_index(&part)?;
            let (k, t) = kind.split_once('|').unwrap();
            let sid = if k == "headerReference" {
                nh += 1;
                format!("hdr{nh}")
            } else {
                nf += 1;
                format!("ftr{nf}")
            };
            let label = format!("{} ({t}, section {section})", if k == "headerReference" { "header" } else { "footer" });
            self.stories.push(Story { id: sid, label, part: pi, path: vec![] });
        }
        for (kind, el, prefix) in [("footnotes", "footnote", "fn"), ("endnotes", "endnote", "en"), ("comments", "comment", "cmt")] {
            let Some(part) = self.rels.first_part(kind) else { continue };
            let Some(part) = self.pkg.zip.real_name(&part) else { continue };
            let pi = self.part_index(&part)?;
            let root = &self.parts[pi].doc.root;
            let mut found = Vec::new();
            for (i, n) in root.children.iter().enumerate() {
                if let Node::Element(e) = n {
                    if e.is(W, el) {
                        let t = e.own("type").unwrap_or("normal");
                        if t == "separator" || t == "continuationSeparator" || t == "continuationNotice" {
                            continue;
                        }
                        let id = e.own("id").unwrap_or("?").to_string();
                        let mut label = format!("{el} {id}");
                        if el == "comment" {
                            if let Some(a) = e.own("author") {
                                label = format!("comment {id} by {a}");
                            }
                        }
                        found.push(Story { id: format!("{prefix}{id}"), label, part: pi, path: vec![i] });
                    }
                }
            }
            self.stories.extend(found);
        }
        Ok(())
    }

    fn load_styles(&mut self) {
        let Some(part) = self.rels.first_part("styles") else { return };
        let Ok(doc) = self.pkg.xml(&part) else { return };
        for s in doc.root.children_named(W, "style") {
            self.styles.push(StyleInfo {
                id: s.own("styleId").unwrap_or("").to_string(),
                name: s.child(W, "name").and_then(|n| n.val()).unwrap_or("").to_string(),
                kind: s.own("type").unwrap_or("paragraph").to_string(),
                based_on: s.child(W, "basedOn").and_then(|n| n.val()).map(str::to_string),
                next: s.child(W, "next").and_then(|n| n.val()).map(str::to_string),
                default: matches!(s.own("default"), Some("1" | "true" | "on")),
            });
        }
    }

    pub fn style_name(&self, id: &str) -> String {
        self.styles.iter().find(|s| s.id == id).map(|s| s.name.clone()).unwrap_or_else(|| id.to_string())
    }

    /// A style id from a name or id (`Heading 1`, `heading1`, `제목 1`).
    pub fn style_id(&self, name: &str, kind: &str) -> Result<String> {
        let norm = |s: &str| s.to_lowercase().replace([' ', '-', '_'], "");
        let want = norm(name);
        let of_kind = |s: &&StyleInfo| s.kind == kind;
        if let Some(s) = self.styles.iter().filter(of_kind).find(|s| s.id == name) {
            return Ok(s.id.clone());
        }
        if let Some(s) = self.styles.iter().filter(of_kind).find(|s| norm(&s.name) == want || norm(&s.id) == want) {
            return Ok(s.id.clone());
        }
        let names: Vec<String> = self.styles.iter().filter(of_kind).map(|s| if s.name == s.id { s.id.clone() } else { format!("{} ({})", s.name, s.id) }).collect();
        Err(err(format!("no {kind} style {name:?}; the document has: {}", names.join(", "))))
    }

    pub fn story_el(&self, s: usize) -> &Element {
        let st = &self.stories[s];
        at(&self.parts[st.part].doc.root, &st.path)
    }

    pub fn story_el_mut(&mut self, s: usize) -> &mut Element {
        let st = self.stories[s].clone();
        at_mut(&mut self.parts[st.part].doc.root, &st.path)
    }

    /// Paragraphs and tables of a story, in document order.
    pub fn scan(&self, s: usize) -> (Vec<ParaLoc>, Vec<TableLoc>) {
        let mut paras = Vec::new();
        let mut tables = Vec::new();
        let mut path = Vec::new();
        scan_block(self.story_el(s), s, &mut path, &mut paras, &mut tables, None, false, false);
        let mut n = 0;
        for p in paras.iter_mut() {
            if p.index != 0 {
                n += 1;
                p.index = n;
            }
        }
        (paras, tables)
    }

    pub fn story_by_id(&self, id: &str) -> Option<usize> {
        self.stories.iter().position(|s| s.id == id)
    }

    /// Resolves an address: `p3`, `p3-7`, `p3[2:9]`, `t1`, `t1.r2`, `t1.r2.c3`, `t1.r2.c3.p2`,
    /// `hdr1.p1`, `fn2`, `body`.
    pub fn resolve(&self, addr: &str) -> Result<Target> {
        let addr = addr.trim();
        let (addr, range) = split_range(addr)?;
        let mut parts: Vec<&str> = addr.split('.').filter(|s| !s.is_empty()).collect();
        let mut story = 0;
        if let Some(first) = parts.first() {
            if let Some(i) = self.story_by_id(first) {
                story = i;
                parts.remove(0);
            } else if is_story_token(first) {
                let have: Vec<&str> = self.stories.iter().map(|s| s.id.as_str()).collect();
                return Err(err(format!("no story {first}; this document has: {}", have.join(", "))));
            }
        }
        let (paras, tables) = self.scan(story);
        let listed: Vec<&ParaLoc> = paras.iter().filter(|p| p.index != 0).collect();
        if parts.is_empty() {
            if range.is_some() {
                return Err(err("a character range needs a paragraph (p3[2:9])"));
            }
            return Ok(Target::Story { story });
        }
        let first = parts[0];
        if let Some(n) = first.strip_prefix('p') {
            if parts.len() > 1 {
                return Err(err(format!("nothing below a paragraph in {addr:?}")));
            }
            let (a, b) = num_range(n, addr)?;
            if a == 0 || b > listed.len() {
                return Err(err(format!("{addr}: the {} has {} paragraphs", self.stories[story].label, listed.len())));
            }
            if range.is_some() && a != b {
                return Err(err("a character range needs one paragraph"));
            }
            return Ok(Target::Paras { story, paras: listed[a - 1..b].iter().map(|p| (*p).clone()).collect(), range });
        }
        if let Some(n) = first.strip_prefix('t') {
            let ti: usize = n.parse().map_err(|_| err(format!("bad table number in {addr:?}")))?;
            let table = tables.get(ti.wrapping_sub(1)).cloned().ok_or_else(|| err(format!("{addr}: the {} has {} tables", self.stories[story].label, tables.len())))?;
            if parts.len() == 1 {
                return Ok(Target::Table { story, table });
            }
            let rn: usize = parts[1].strip_prefix('r').and_then(|n| n.parse().ok()).ok_or_else(|| err(format!("expected rN after the table in {addr:?}")))?;
            if rn == 0 || rn > table.rows.len() {
                return Err(err(format!("{addr}: table {ti} has {} rows", table.rows.len())));
            }
            if parts.len() == 2 {
                return Ok(Target::Row { story, table, row: rn - 1 });
            }
            let cn: usize = parts[2].strip_prefix('c').and_then(|n| n.parse().ok()).ok_or_else(|| err(format!("expected cN after the row in {addr:?}")))?;
            let ncells = table.rows[rn - 1].cells.len();
            if cn == 0 || cn > ncells {
                return Err(err(format!("{addr}: row {rn} of table {ti} has {ncells} cells")));
            }
            if parts.len() == 3 {
                if range.is_some() {
                    return Err(err("a character range needs a paragraph (t1.r1.c1.p1[0:4])"));
                }
                return Ok(Target::Cell { story, table, row: rn - 1, col: cn - 1 });
            }
            let pn = parts[3].strip_prefix('p').ok_or_else(|| err(format!("expected pN after the cell in {addr:?}")))?;
            let (a, b) = num_range(pn, addr)?;
            let cell_path = &table.rows[rn - 1].cells[cn - 1];
            let in_cell: Vec<&ParaLoc> = listed.iter().copied().filter(|p| p.path.starts_with(cell_path) && direct_in_cell(&p.path, cell_path, self.story_el(story))).collect();
            if a == 0 || b > in_cell.len() {
                return Err(err(format!("{addr}: the cell has {} paragraphs", in_cell.len())));
            }
            return Ok(Target::Paras { story, paras: in_cell[a - 1..b].iter().map(|p| (*p).clone()).collect(), range });
        }
        Err(err(format!("bad address {addr:?} (p3, p3-5, p3[0:5], t1, t1.r2, t1.r2.c3, t1.r2.c3.p1, hdr1.p1, fn1.p1)")))
    }

    pub fn para_el(&self, p: &ParaLoc) -> &Element {
        at(self.story_el(p.story), &p.path)
    }

    pub fn para_mut(&mut self, p: &ParaLoc) -> &mut Element {
        at_mut(self.story_el_mut(p.story), &p.path)
    }

    pub fn para_addr(&self, p: &ParaLoc) -> String {
        let s = &self.stories[p.story];
        if s.id == "body" {
            format!("p{}", p.index)
        } else {
            format!("{}.p{}", s.id, p.index)
        }
    }

    pub(crate) fn story_prefix(&self, s: usize) -> String {
        if self.stories[s].id == "body" {
            String::new()
        } else {
            format!("{}.", self.stories[s].id)
        }
    }

    pub fn outline(&self, with_runs: bool) -> Outline {
        let mut items = Vec::new();
        for s in 0..self.stories.len() {
            let (paras, tables) = self.scan(s);
            let pre = self.story_prefix(s);
            if s > 0 {
                items.push(Item { addr: self.stories[s].id.clone(), kind: "story".into(), text: self.stories[s].label.clone(), depth: 0, ..Default::default() });
            }
            let depth0 = if s > 0 { 1 } else { 0 };
            // Tables are announced before their first paragraph.
            let mut announced = vec![false; tables.len()];
            for p in paras.iter().filter(|p| p.index != 0) {
                if let Some((ti, ri, ci)) = p.cell {
                    if !announced[ti] {
                        announced[ti] = true;
                        let t = &tables[ti];
                        let cols = t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
                        items.push(Item {
                            addr: format!("{pre}t{}", ti + 1),
                            kind: "table".into(),
                            text: format!("{} rows x {} columns", t.rows.len(), cols),
                            depth: depth0 + p.path.len().saturating_sub(4) / 4,
                            ..Default::default()
                        });
                    }
                    let _ = (ri, ci);
                }
                let el = self.para_el(p);
                let mut it = Item {
                    addr: format!("{pre}p{}", p.index),
                    kind: "p".into(),
                    text: text::display_of(&Wml, el),
                    depth: depth0 + if p.cell.is_some() { 1 } else { 0 } + if p.textbox { 1 } else { 0 },
                    ..Default::default()
                };
                if let Some((ti, ri, ci)) = p.cell {
                    it.place = Some(format!("{pre}t{}.r{}.c{}", ti + 1, ri + 1, ci + 1));
                }
                if p.textbox {
                    it.place = Some("text box".into());
                }
                if let Some(ppr) = el.child(W, "pPr") {
                    if let Some(st) = ppr.child(W, "pStyle").and_then(|s| s.val()) {
                        it.style = Some(self.style_name(st));
                    }
                    if let Some(num) = ppr.child(W, "numPr") {
                        let lvl = num.child(W, "ilvl").and_then(|e| e.val()).unwrap_or("0");
                        let id = num.child(W, "numId").and_then(|e| e.val()).unwrap_or("?");
                        if id != "0" {
                            it.extra.push(("list".into(), format!("{id}/{}", lvl.parse::<u32>().unwrap_or(0) + 1)));
                        }
                    }
                    if ppr.child(W, "sectPr").is_some() {
                        it.extra.push(("section-break".into(), "after".into()));
                    }
                }
                if with_runs {
                    it.runs = runs_info(el);
                }
                items.push(it);
            }
        }
        Outline { items }
    }

    /// Every paragraph a search looks at: listed ones and the fallback copies of text boxes.
    fn search_paras(&self, scope: &Option<Target>, all_stories: bool) -> Vec<ParaLoc> {
        match scope {
            Some(Target::Paras { paras, .. }) => paras.clone(),
            Some(Target::Story { story }) => self.scan(*story).0,
            Some(Target::Table { story, table }) => self.scan(*story).0.into_iter().filter(|p| p.path.starts_with(&table.path)).collect(),
            Some(Target::Row { story, table, row }) => self.scan(*story).0.into_iter().filter(|p| p.path.starts_with(&table.rows[*row].path)).collect(),
            Some(Target::Cell { story, table, row, col }) => {
                self.scan(*story).0.into_iter().filter(|p| p.path.starts_with(&table.rows[*row].cells[*col])).collect()
            }
            None => {
                let mut v = Vec::new();
                for s in 0..self.stories.len() {
                    if !all_stories && self.stories[s].id.starts_with("cmt") {
                        continue;
                    }
                    v.extend(self.scan(s).0);
                }
                v
            }
        }
    }

    pub fn find(&self, f: &Finder, scope: Option<&str>) -> Result<Vec<Match>> {
        let scope = scope.map(|s| self.resolve(s)).transpose()?;
        let mut out = Vec::new();
        for p in self.search_paras(&scope, true) {
            if p.index == 0 {
                continue;
            }
            let t = text::text_of(&Wml, self.para_el(&p));
            for (a, b, _) in f.find(&t, None) {
                out.push(Match { addr: self.para_addr(&p), start: a, end: b, text: t.chars().skip(a).take(b - a).collect(), context: context(&t, a, b) });
            }
        }
        Ok(out)
    }

    /// Replaces matches; `nth` picks one (1-based) of all matches in order. Returns the
    /// addresses changed, one per replacement.
    pub fn replace(&mut self, f: &Finder, with: &str, scope: Option<&str>, nth: Option<usize>) -> Result<Vec<String>> {
        let scope_t = scope.map(|s| self.resolve(s)).transpose()?;
        let range = match &scope_t {
            Some(Target::Paras { range, .. }) => *range,
            _ => None,
        };
        let paras = self.search_paras(&scope_t, scope_t.is_some());
        let mut done = Vec::new();
        let mut count = 0;
        // Fallback copies follow the listed paragraph they mirror: replace them the same way,
        // but count only listed ones.
        for p in paras {
            let t = text::text_of(&Wml, self.para_el(&p));
            let mut ms = f.find(&t, Some(with));
            if let Some((a, b)) = range {
                ms.retain(|(s, e, _)| *s >= a && *e <= b);
            }
            let mut chosen = Vec::new();
            for m in ms {
                if p.index != 0 {
                    count += 1;
                }
                if nth.map(|n| n == count && p.index != 0).unwrap_or(true) {
                    chosen.push(m);
                }
            }
            if chosen.is_empty() {
                continue;
            }
            let addr = self.para_addr(&p);
            let el = self.para_mut(&p);
            for (a, b, rep) in chosen.iter().rev() {
                text::replace_range(&Wml, el, *a, *b, &text::unescape(rep))?;
            }
            if p.index != 0 {
                for _ in &chosen {
                    done.push(addr.clone());
                }
            }
        }
        Ok(done)
    }

    /// Sets the whole text of paragraphs or a cell, keeping the first run's properties.
    pub fn set_text(&mut self, addr: &str, value: &str) -> Result<String> {
        let value = text::unescape(value);
        match self.resolve(addr)? {
            Target::Paras { paras, range, .. } => {
                if paras.len() != 1 {
                    return Err(err("set takes one paragraph (or a cell)"));
                }
                let p = &paras[0];
                let el = self.para_mut(p);
                let len = text::text_of(&Wml, el).chars().count();
                let (a, b) = range.unwrap_or((0, len));
                let lines: Vec<&str> = if range.is_none() { value.split("\n\n").collect() } else { vec![&value] };
                text::replace_range(&Wml, el, a, b.min(len), lines[0])?;
                if lines.len() > 1 {
                    // Blank lines start new paragraphs like this one.
                    let template = self.para_el(p).clone();
                    let story = self.story_el_mut(p.story);
                    let (pp, pi) = p.path.split_at(p.path.len() - 1);
                    let parent = at_mut(story, pp);
                    for (k, line) in lines[1..].iter().enumerate() {
                        let np = paragraph_like(&template, line);
                        parent.children.insert(pi[0] + 1 + k, Node::Element(np));
                    }
                }
                Ok(self.para_addr(p))
            }
            Target::Cell { story, table, row, col } => {
                let cell_path = table.rows[row].cells[col].clone();
                let cell = at_mut(self.story_el_mut(story), &cell_path);
                let continued = cell.child(W, "tcPr").and_then(|p| p.child(W, "vMerge")).map(|m| m.val() != Some("restart")).unwrap_or(false);
                set_cell_text(cell, &value);
                if continued {
                    self.warnings.push(format!("{addr} is merged into the cell above it (a vertical merge): Word does not show its text — set the first cell of the merge"));
                }
                Ok(format!("{}t{}.r{}.c{}", self.story_prefix(story), table.index, row + 1, col + 1))
            }
            _ => Err(err("set takes a paragraph (p3) or a table cell (t1.r2.c3)")),
        }
    }

    /// Applies properties to what an address names. With `matching`, only to the text that
    /// matches in it (each match, or the nth).
    pub fn style(&mut self, addr: &str, props: &Props, matching: Option<(&Finder, Option<usize>)>) -> Result<Vec<String>> {
        let target = self.resolve(addr)?;
        let mut changed = Vec::new();
        let char_props = props.has_char();
        let para_props = props.has_para();
        let mut other: Vec<&str> = props.keys.iter().map(|s| s.as_str()).filter(|k| !crate::props::CHAR_KEYS.contains(k) && !crate::props::PARA_KEYS.contains(k)).collect();
        let (paras, range) = match &target {
            Target::Paras { paras, range, .. } => (paras.clone(), *range),
            Target::Story { story } => (self.scan(*story).0.into_iter().filter(|p| p.index != 0).collect(), None),
            Target::Table { story, table } => {
                self.table_props(*story, table, None, props)?;
                other.retain(|k| !TABLE_KEYS.contains(k));
                changed.push(format!("{}t{}", self.story_prefix(*story), table.index));
                (self.scan(*story).0.into_iter().filter(|p| p.index != 0 && p.path.starts_with(&table.path)).collect(), None)
            }
            Target::Row { story, table, row } => {
                self.table_props(*story, table, Some((*row, None)), props)?;
                other.retain(|k| !TABLE_KEYS.contains(k));
                changed.push(format!("{}t{}.r{}", self.story_prefix(*story), table.index, row + 1));
                (self.scan(*story).0.into_iter().filter(|p| p.index != 0 && p.path.starts_with(&table.rows[*row].path)).collect(), None)
            }
            Target::Cell { story, table, row, col } => {
                self.table_props(*story, table, Some((*row, Some(*col))), props)?;
                other.retain(|k| !TABLE_KEYS.contains(k));
                changed.push(format!("{}t{}.r{}.c{}", self.story_prefix(*story), table.index, row + 1, col + 1));
                (self.scan(*story).0.into_iter().filter(|p| p.index != 0 && p.path.starts_with(&table.rows[*row].cells[*col])).collect(), None)
            }
        };
        if !other.is_empty() {
            return Err(err(format!(
                "{} not supported here (Word text takes: {}; tables, rows and cells also: {})",
                other.join(", "),
                [crate::props::CHAR_KEYS, crate::props::PARA_KEYS].concat().join(" "),
                TABLE_KEYS.join(" ")
            )));
        }
        let mut pstyle = props.para_style.as_ref().map(|n| self.style_id(n, "paragraph")).transpose()?;
        let list_id = match props.list.as_deref() {
            Some("none") | None => None,
            Some(kind) => {
                // List paragraphs take the List Paragraph style, as Word gives them.
                if pstyle.is_none() {
                    pstyle = self.style_id("List Paragraph", "paragraph").ok();
                }
                Some(self.list_num_id(kind)?)
            }
        };
        let cstyle = props.char_style.as_ref().map(|n| self.style_id(n, "character")).transpose()?;
        let link_rid = match &props.link {
            Some(url) if !url.is_empty() => Some(self.link_rel(paras.first().map(|p| p.story).unwrap_or(0), url)),
            _ => None,
        };
        let hyperlink_style = if props.link.is_some() { self.style_id("Hyperlink", "character").ok() } else { None };
        let mut count = 0;
        for p in &paras {
            let addr = self.para_addr(p);
            let el = self.para_mut(p);
            if para_props {
                let ppr = ensure_first(el, "pPr");
                apply_ppr(ppr, props, pstyle.as_deref(), list_id.as_deref())?;
            }
            if char_props {
                let len = text::text_of(&Wml, el).chars().count();
                let mut ranges: Vec<(usize, usize)> = match matching {
                    Some((f, nth)) => {
                        let t = text::text_of(&Wml, el);
                        let mut v = Vec::new();
                        for (a, b, _) in f.find(&t, None) {
                            if let Some((ra, rb)) = range {
                                if a < ra || b > rb {
                                    continue;
                                }
                            }
                            count += 1;
                            if nth.map(|n| n == count).unwrap_or(true) {
                                v.push((a, b));
                            }
                        }
                        v
                    }
                    None => vec![range.unwrap_or((0, len))],
                };
                ranges.retain(|(a, b)| a < b || len == 0);
                if ranges.is_empty() {
                    continue;
                }
                for (a, b) in ranges.iter().rev() {
                    let runs = text::runs_in(&Wml, el, *a, *b);
                    for rp in &runs {
                        let run = at_mut(el, rp);
                        let rpr = ensure_first(run, "rPr");
                        apply_rpr(rpr, props, cstyle.as_deref().or(if link_rid.is_some() { hyperlink_style.as_deref() } else { None }));
                        if rpr.children.is_empty() && props.clear {
                            run.remove_children(W, "rPr");
                        }
                    }
                    if let Some(rid) = &link_rid {
                        wrap_link(el, &runs, rid);
                    }
                    if *a == 0 && *b == len && matching.is_none() && range.is_none() {
                        // The paragraph mark too, as Word does when a whole paragraph is selected.
                        let ppr = ensure_first(el, "pPr");
                        let mark = ensure_in_order(ppr, "rPr", PPR_ORDER);
                        apply_rpr(mark, props, None);
                        if mark.children.is_empty() {
                            ppr.remove_children(W, "rPr");
                            if ppr.children.is_empty() {
                                el.remove_children(W, "pPr");
                            }
                        }
                    }
                }
                changed.push(addr);
            } else if para_props {
                changed.push(addr);
            }
        }
        if matching.is_some() && changed.is_empty() {
            return Err(err("no text matched"));
        }
        Ok(changed)
    }

    /// The list (numId) `list=bullet|number` puts paragraphs in: the first one whose top level
    /// is of that kind, or a new definition (and the numbering part, when there is none).
    fn list_num_id(&mut self, kind: &str) -> Result<String> {
        let bullet = match kind {
            "bullet" | "bullets" | "ul" => true,
            "number" | "numbers" | "numbered" | "ol" | "decimal" => false,
            k => return Err(err(format!("list={k}: bullet, number or none"))),
        };
        let part = match self.rels.first_part("numbering") {
            Some(p) => p,
            None => {
                let p = "word/numbering.xml".to_string();
                let doc = Document::parse("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"></w:numbering>")?;
                self.pkg.put_xml(&p, &doc);
                self.pkg.set_content_type(&p, "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml", false);
                self.rels.add_part("http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering", &p);
                p
            }
        };
        let mut doc = self.pkg.xml(&part)?;
        let mut kinds: HashMap<String, bool> = HashMap::new();
        let mut max_abs = -1i64;
        for a in doc.root.elements().filter(|e| e.local() == "abstractNum") {
            let id = a.own("abstractNumId").unwrap_or("").to_string();
            max_abs = max_abs.max(id.parse().unwrap_or(-1));
            let fmt = a.elements().find(|l| l.local() == "lvl" && l.own("ilvl") == Some("0")).and_then(|l| l.child(W, "numFmt")).and_then(|f| f.own("val")).unwrap_or("decimal");
            kinds.insert(id, fmt == "bullet");
        }
        let mut max_num = 0i64;
        for n in doc.root.elements().filter(|e| e.local() == "num") {
            max_num = max_num.max(n.own("numId").and_then(|v| v.parse().ok()).unwrap_or(0));
        }
        for n in doc.root.elements().filter(|e| e.local() == "num") {
            if n.child(W, "lvlOverride").is_some() {
                continue;
            }
            let abs = n.child(W, "abstractNumId").and_then(|a| a.own("val")).unwrap_or("");
            if kinds.get(abs) == Some(&bullet) {
                return Ok(n.own("numId").unwrap_or("1").to_string());
            }
        }
        let abs_id = max_abs + 1;
        let num_id = max_num + 1;
        let mut src = format!("<w:abstractNum w:abstractNumId=\"{abs_id}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>");
        for l in 0..9u32 {
            let (fmt, text, rpr) = if bullet {
                ("bullet", ["•", "◦", "▪"][l as usize % 3].to_string(), "<w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:hint=\"default\"/></w:rPr>")
            } else {
                (["decimal", "lowerLetter", "lowerRoman"][l as usize % 3], format!("%{}.", l + 1), "")
            };
            src.push_str(&format!("<w:lvl w:ilvl=\"{l}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"{text}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{}\" w:hanging=\"360\"/></w:pPr>{rpr}</w:lvl>", 720 + 720 * l));
        }
        src.push_str(&format!("</w:abstractNum><w:num w:numId=\"{num_id}\"><w:abstractNumId w:val=\"{abs_id}\"/></w:num>"));
        let mut els = xml::fragment(&src, &[("w", W)])?.into_iter();
        let (abs, num) = (els.next().unwrap(), els.next().unwrap());
        // Definitions come before the lists, and the lists before the clean-up marker.
        let first_num = doc.root.children.iter().position(|n| n.as_element().map(|e| e.local() == "num" || e.local() == "numIdMacAtCleanup").unwrap_or(false)).unwrap_or(doc.root.children.len());
        doc.root.children.insert(first_num, Node::Element(abs));
        let at = doc.root.children.iter().position(|n| n.as_element().map(|e| e.local() == "numIdMacAtCleanup").unwrap_or(false)).unwrap_or(doc.root.children.len());
        doc.root.children.insert(at, Node::Element(num));
        self.pkg.put_xml(&part, &doc);
        Ok(num_id.to_string())
    }

    fn link_rel(&mut self, story: usize, url: &str) -> String {
        let part = self.parts[self.stories[story].part].name.clone();
        if part == self.parts[0].name {
            return self.rels.add("http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink", url, true);
        }
        let mut rels = self.pkg.rels(&part).expect("rels");
        let id = rels.add("http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink", url, true);
        rels.save(&mut self.pkg);
        id
    }

    fn table_props(&mut self, story: usize, table: &TableLoc, which: Option<(usize, Option<usize>)>, props: &Props) -> Result<()> {
        let el = self.story_el_mut(story);
        let tbl = at_mut(el, &table.path);
        match which {
            None => {
                let tpr = ensure_first(tbl, "tblPr");
                if let Some(a) = &props.align {
                    let mut jc = tpr.sibling("jc");
                    jc.set_own("val", match a.as_str() { "left" => "left", "right" => "right", _ => "center" });
                    put_in_order(tpr, jc, TBLPR_ORDER);
                }
                if let Some(w) = props.width {
                    let mut tw = tpr.sibling("tblW");
                    tw.set_own("w", &twips(w).to_string());
                    tw.set_own("type", "dxa");
                    put_in_order(tpr, tw, TBLPR_ORDER);
                }
                if !props.borders.is_empty() {
                    let mut b = tpr.child(W, "tblBorders").cloned().unwrap_or_else(|| tpr.sibling("tblBorders"));
                    for (edges, border) in &props.borders {
                        let names: &[&str] = match edges {
                            Edges::All => &["top", "left", "bottom", "right", "insideH", "insideV"],
                            Edges::Outline => &["top", "left", "bottom", "right"],
                            Edges::Top => &["top"],
                            Edges::Bottom => &["bottom"],
                            Edges::Left => &["left"],
                            Edges::Right => &["right"],
                            Edges::Inside => &["insideH", "insideV"],
                            Edges::InsideH => &["insideH"],
                            Edges::InsideV => &["insideV"],
                        };
                        for n in names {
                            let be = border_el(&b, n, border);
                            put_in_order(&mut b, be, BORDER_ORDER);
                        }
                    }
                    b.touch();
                    put_in_order(tpr, b, TBLPR_ORDER);
                }
                if props.fill.is_some() {
                    // The table's cells.
                    for r in &table.rows {
                        for c in &r.cells {
                            let rel: Vec<usize> = c[table.path.len()..].to_vec();
                            let tc = at_mut(tbl, &rel);
                            cell_props(tc, props);
                        }
                    }
                }
            }
            Some((row, col)) => {
                let r = &table.rows[row];
                let cols: Vec<usize> = match col {
                    Some(c) => vec![c],
                    None => (0..r.cells.len()).collect(),
                };
                if let (Some(h), None) = (props.height, col) {
                    let rel: Vec<usize> = r.path[table.path.len()..].to_vec();
                    let tr = at_mut(tbl, &rel);
                    let trpr = ensure_first(tr, "trPr");
                    let mut th = trpr.sibling("trHeight");
                    th.set_own("val", &twips(h).to_string());
                    th.set_own("hRule", "atLeast");
                    put_in_order(trpr, th, TRPR_ORDER);
                }
                for c in cols {
                    let rel: Vec<usize> = r.cells[c][table.path.len()..].to_vec();
                    let tc = at_mut(tbl, &rel);
                    cell_props(tc, props);
                }
            }
        }
        Ok(())
    }

    /// Inserts paragraphs (one per line of `text`; `\n\n` or real newlines) before or after
    /// what `addr` names: a paragraph, a table (after it), or a row (a new row like it).
    pub fn insert(&mut self, addr: &str, after: bool, text_in: &str, props: Option<&Props>) -> Result<Vec<String>> {
        let value = text::unescape(text_in);
        let target = self.resolve(addr)?;
        match target {
            Target::Paras { story, paras, .. } => {
                let p = if after { paras.last().unwrap().clone() } else { paras[0].clone() };
                let mut template = self.para_el(&p).clone();
                if after {
                    // After a heading comes the style it names as next (Normal), as in Word.
                    let cur = template.child(W, "pPr").and_then(|x| x.child(W, "pStyle")).and_then(|x| x.val()).map(str::to_string);
                    let next = cur.as_ref().and_then(|c| self.styles.iter().find(|s| &s.id == c)).and_then(|s| s.next.clone()).filter(|n| Some(n) != cur.as_ref());
                    if let Some(n) = next {
                        let is_default = self.styles.iter().any(|s| s.id == n && s.default);
                        if let Some(ppr) = template.child_mut(W, "pPr") {
                            ppr.remove_children(W, "numPr");
                            ppr.remove_children(W, "outlineLvl");
                            if is_default {
                                ppr.remove_children(W, "pStyle");
                            } else if let Some(ps) = ppr.child_mut(W, "pStyle") {
                                ps.set_own("val", &n);
                            }
                        }
                    }
                }
                let lines: Vec<&str> = text::lines_of(&value);
                let story_el = self.story_el_mut(story);
                let (pp, pi) = p.path.split_at(p.path.len() - 1);
                let parent = at_mut(story_el, pp);
                let at_i = if after { pi[0] + 1 } else { pi[0] };
                for (k, line) in lines.iter().enumerate() {
                    let mut np = paragraph_like(&template, line);
                    // A section break stays with the original paragraph.
                    if let Some(ppr) = np.child_mut(W, "pPr") {
                        ppr.remove_children(W, "sectPr");
                    }
                    parent.children.insert(at_i + k, Node::Element(np));
                }
                let first_new = if after { p.index + 1 } else { p.index };
                let addrs: Vec<String> = (0..lines.len()).map(|k| format!("{}p{}", self.story_prefix(story), first_new + k)).collect();
                if let Some(pr) = props {
                    for a in &addrs {
                        self.style(a, pr, None)?;
                    }
                }
                Ok(addrs)
            }
            Target::Table { story, table } => {
                // A paragraph after (or before) the table.
                let story_el = self.story_el_mut(story);
                let (pp, ti) = table.path.split_at(table.path.len() - 1);
                let parent = at_mut(story_el, pp);
                let at_i = if after { ti[0] + 1 } else { ti[0] };
                let lines: Vec<&str> = text::lines_of(&value);
                for (k, line) in lines.iter().enumerate() {
                    let mut np = Element::new(&parent.qualify("p"), W);
                    if !line.is_empty() {
                        let mut r = np.sibling("r");
                        let t = Wml.new_text(&r, line);
                        r.push(t);
                        np.push(r);
                        let n = np.children.len() - 1;
                        Wml.settle(&mut np, n, 0);
                    }
                    parent.children.insert(at_i + k, Node::Element(np));
                }
                let (paras, _) = self.scan(story);
                let mut addrs = Vec::new();
                for p in paras.iter().filter(|p| p.index != 0) {
                    if p.path.len() == table.path.len() && p.path[..pp.len()] == *pp && p.path[pp.len()] >= at_i && p.path[pp.len()] < at_i + lines.len() {
                        addrs.push(self.para_addr(p));
                    }
                }
                Ok(addrs)
            }
            Target::Row { story, table, row } => {
                // A new row like this one; `text` gives its cells, separated by `|` or tabs.
                let cells: Vec<String> = if value.contains('|') { value.split('|').map(|s| s.trim().to_string()).collect() } else { value.split('\t').map(str::to_string).collect() };
                let story_el = self.story_el_mut(story);
                let tbl = at_mut(story_el, &table.path);
                let rel: Vec<usize> = table.rows[row].path[table.path.len()..].to_vec();
                let src = at(tbl, &rel).clone();
                let mut tr = src.clone();
                // Each cell keeps its first paragraph's look with the new text.
                let mut k = 0;
                for n in tr.children.iter_mut() {
                    if let Node::Element(tc) = n {
                        if tc.is(W, "tc") {
                            // A row below a vertical merge start continues nothing: restart it.
                            if let Some(tcpr) = tc.child_mut(W, "tcPr") {
                                tcpr.remove_children(W, "vMerge");
                            }
                            set_cell_text(tc, cells.get(k).map(|s| s.as_str()).unwrap_or(""));
                            k += 1;
                        }
                    }
                }
                let (pp, ri) = rel.split_at(rel.len() - 1);
                let parent = at_mut(tbl, pp);
                parent.children.insert(if after { ri[0] + 1 } else { ri[0] }, Node::Element(tr));
                let new_row = if after { row + 2 } else { row + 1 };
                Ok(vec![format!("{}t{}.r{}", self.story_prefix(story), table.index, new_row)])
            }
            Target::Cell { .. } => Err(err("insert into a cell's paragraph (t1.r2.c3.p1), or a row (t1.r2) for a new row")),
            Target::Story { story } => {
                // At the start or end of the story.
                let (paras, _) = self.scan(story);
                let listed: Vec<&ParaLoc> = paras.iter().filter(|p| p.index != 0 && p.path.len() == 1).collect();
                let p = if after { listed.last() } else { listed.first() }.ok_or_else(|| err("the story has no paragraphs"))?;
                let a = self.para_addr(p);
                self.insert(&a, after, text_in, props)
            }
        }
    }

    pub fn delete(&mut self, addr: &str) -> Result<String> {
        match self.resolve(addr)? {
            Target::Paras { story, paras, range } => {
                if let Some((a, b)) = range {
                    let el = self.para_mut(&paras[0]);
                    text::replace_range(&Wml, el, a, b, "")?;
                    return Ok(format!("{addr}: removed characters {a}..{b}"));
                }
                let n = paras.len();
                for p in paras.iter().rev() {
                    let story_el = self.story_el_mut(story);
                    let (pp, pi) = p.path.split_at(p.path.len() - 1);
                    let parent = at_mut(story_el, pp);
                    let siblings = parent.children.iter().filter(|c| c.as_element().map(|e| e.is(W, "p")).unwrap_or(false)).count();
                    let el = parent.children[pi[0]].as_element().unwrap().clone();
                    let sect = el.child(W, "pPr").and_then(|p| p.child(W, "sectPr")).cloned();
                    if parent.is(W, "tc") && siblings == 1 || (parent.is(W, "body") || parent.is(W, "hdr") || parent.is(W, "ftr")) && siblings == 1 {
                        // A cell or story keeps one (empty) paragraph.
                        let e = parent.children[pi[0]].as_element_mut().unwrap();
                        e.children.retain(|c| c.as_element().map(|x| x.is(W, "pPr")).unwrap_or(false));
                        continue;
                    }
                    parent.children.remove(pi[0]);
                    if let Some(sect) = sect {
                        // Keep the section: it moves to the paragraph before.
                        let prev = parent.children[..pi[0]].iter().rposition(|c| c.as_element().map(|e| e.is(W, "p")).unwrap_or(false));
                        if let Some(k) = prev {
                            let pe = parent.children[k].as_element_mut().unwrap();
                            let ppr = ensure_first(pe, "pPr");
                            if ppr.child(W, "sectPr").is_none() {
                                put_in_order(ppr, sect, PPR_ORDER);
                            }
                        }
                    }
                }
                Ok(format!("deleted {n} paragraph{}", if n == 1 { "" } else { "s" }))
            }
            Target::Table { story, table } => {
                let story_el = self.story_el_mut(story);
                let (pp, ti) = table.path.split_at(table.path.len() - 1);
                let parent = at_mut(story_el, pp);
                parent.children.remove(ti[0]);
                // A table must not end a cell: give it a paragraph.
                if parent.is(W, "tc") && !parent.elements().any(|e| e.is(W, "p")) {
                    let p = Element::new(&parent.qualify("p"), W);
                    parent.push(p);
                }
                Ok(format!("deleted table {}", table.index))
            }
            Target::Row { story, table, row } => {
                if table.rows.len() == 1 {
                    return self.delete(&format!("{}t{}", self.story_prefix(story), table.index));
                }
                let story_el = self.story_el_mut(story);
                let (pp, ri) = table.rows[row].path.split_at(table.rows[row].path.len() - 1);
                at_mut(story_el, pp).children.remove(ri[0]);
                Ok(format!("deleted row {} of table {}", row + 1, table.index))
            }
            Target::Cell { story, table, row, col } => {
                let p = table.rows[row].cells[col].clone();
                let cell = at_mut(self.story_el_mut(story), &p);
                set_cell_text(cell, "");
                Ok(format!("cleared cell t{}.r{}.c{}", table.index, row + 1, col + 1))
            }
            Target::Story { .. } => Err(err("delete takes paragraphs, tables or rows")),
        }
    }

    /// A table after (or before) a paragraph: rows of cells from `rows`.
    pub fn insert_table(&mut self, addr: &str, after: bool, rows: &[Vec<String>], header: bool) -> Result<String> {
        let Target::Paras { story, paras, .. } = self.resolve(addr)? else {
            return Err(err("a table goes before or after a paragraph"));
        };
        let p = if after { paras.last().unwrap().clone() } else { paras[0].clone() };
        let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
        // The text width of the section, shared out.
        let width = self.text_width_twips();
        let cw = width / cols as i64;
        let w = |s: &str| format!("w:{s}");
        let mut x = String::new();
        x.push_str(&format!("<{}>", w("tbl")));
        let style = self.style_id("Table Grid", "table").ok();
        x.push_str(&format!("<w:tblPr>{}<w:tblW w:w=\"0\" w:type=\"auto\"/>", style.as_ref().map(|s| format!("<w:tblStyle w:val=\"{}\"/>", xml::escape(s))).unwrap_or_default()));
        if style.is_none() {
            x.push_str("<w:tblBorders>");
            for e in ["top", "left", "bottom", "right", "insideH", "insideV"] {
                x.push_str(&format!("<w:{e} w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>"));
            }
            x.push_str("</w:tblBorders>");
        }
        x.push_str("<w:tblLook w:val=\"04A0\" w:firstRow=\"1\" w:lastRow=\"0\" w:firstColumn=\"1\" w:lastColumn=\"0\" w:noHBand=\"0\" w:noVBand=\"1\"/></w:tblPr><w:tblGrid>");
        for _ in 0..cols {
            x.push_str(&format!("<w:gridCol w:w=\"{cw}\"/>"));
        }
        x.push_str("</w:tblGrid>");
        for (ri, r) in rows.iter().enumerate() {
            x.push_str("<w:tr>");
            if ri == 0 && header {
                x.push_str("<w:trPr><w:tblHeader/></w:trPr>");
            }
            for ci in 0..cols {
                let t = r.get(ci).map(|s| s.as_str()).unwrap_or("");
                x.push_str(&format!("<w:tc><w:tcPr><w:tcW w:w=\"{cw}\" w:type=\"dxa\"/></w:tcPr><w:p>"));
                if !t.is_empty() {
                    let rpr = if ri == 0 && header { "<w:rPr><w:b/></w:rPr>" } else { "" };
                    x.push_str(&format!("<w:r>{rpr}<w:t xml:space=\"preserve\">{}</w:t></w:r>", xml::escape(t)));
                }
                x.push_str("</w:p></w:tc>");
            }
            x.push_str("</w:tr>");
        }
        x.push_str("</w:tbl>");
        let prefix = self.parts[self.stories[story].part].doc.prefix_for(W);
        let x = if prefix != "w" { x.replace("<w:", &format!("<{prefix}:")).replace("</w:", &format!("</{prefix}:")).replace(" w:", &format!(" {prefix}:")) } else { x };
        let tbl = xml::fragment(&x, &[(&prefix, W)])?.remove(0);
        let story_el = self.story_el_mut(story);
        let (pp, pi) = p.path.split_at(p.path.len() - 1);
        let parent = at_mut(story_el, pp);
        let at_i = if after { pi[0] + 1 } else { pi[0] };
        parent.children.insert(at_i, Node::Element(tbl));
        // A table must be followed by a paragraph in a cell or at the end of the body.
        let next_is_p = parent.children[at_i + 1..].iter().find_map(|n| n.as_element()).map(|e| e.is(W, "p")).unwrap_or(false);
        if !next_is_p {
            let np = Element::new(&parent.qualify("p"), W);
            parent.children.insert(at_i + 1, Node::Element(np));
        }
        let (_, tables) = self.scan(story);
        let mut tpath = pp.to_vec();
        tpath.push(at_i);
        let t = tables.iter().find(|t| t.path == tpath).map(|t| t.index).unwrap_or(0);
        Ok(format!("{}t{t}", self.story_prefix(story)))
    }

    pub(crate) fn text_width_twips(&self) -> i64 {
        let body = self.story_el(0);
        let sect = body.child(W, "sectPr");
        let w = sect.and_then(|s| s.child(W, "pgSz")).and_then(|p| p.own("w")).and_then(|v| v.parse::<i64>().ok()).unwrap_or(11906);
        let m = sect.and_then(|s| s.child(W, "pgMar"));
        let l = m.and_then(|p| p.own("left")).and_then(|v| v.parse::<i64>().ok()).unwrap_or(1440);
        let r = m.and_then(|p| p.own("right")).and_then(|v| v.parse::<i64>().ok()).unwrap_or(1440);
        (w - l - r).max(1440)
    }

    pub fn save(&mut self) -> Result<Vec<u8>> {
        for p in &self.parts {
            self.pkg.put_xml(&p.name, &p.doc);
        }
        self.rels.save(&mut self.pkg);
        self.pkg.save()
    }
}

pub const TABLE_KEYS: &[&str] = &["fill", "valign", "border", "w", "h", "align"];

fn is_story_token(s: &str) -> bool {
    let t = s.trim_end_matches(|c: char| c.is_ascii_digit());
    matches!(t, "hdr" | "ftr" | "fn" | "en" | "cmt" | "body")
}

fn num_range(n: &str, addr: &str) -> Result<(usize, usize)> {
    let bad = || err(format!("bad paragraph number in {addr:?}"));
    if let Some((a, b)) = n.split_once('-') {
        let a: usize = a.parse().map_err(|_| bad())?;
        let b: usize = b.trim_start_matches('p').parse().map_err(|_| bad())?;
        if b < a {
            return Err(bad());
        }
        Ok((a, b))
    } else {
        let a: usize = n.parse().map_err(|_| bad())?;
        Ok((a, a))
    }
}

/// `p3[2:9]` → (`p3`, Some((2, 9))); `[5:]` and `[:4]` work.
pub fn split_range(addr: &str) -> Result<(&str, Option<(usize, usize)>)> {
    if let Some(i) = addr.find('[') {
        let inner = addr[i + 1..].strip_suffix(']').ok_or_else(|| err(format!("bad range in {addr:?}")))?;
        let (a, b) = inner.split_once(':').ok_or_else(|| err(format!("range is [from:to] in {addr:?}")))?;
        let a: usize = if a.trim().is_empty() { 0 } else { a.trim().parse().map_err(|_| err("bad range start"))? };
        let b: usize = if b.trim().is_empty() { usize::MAX } else { b.trim().parse().map_err(|_| err("bad range end"))? };
        return Ok((&addr[..i], Some((a, b))));
    }
    Ok((addr, None))
}

fn direct_in_cell(p: &[usize], cell: &[usize], story: &Element) -> bool {
    // Not inside a nested table of the cell.
    let mut e = at(story, cell);
    for &i in &p[cell.len()..p.len() - 1] {
        e = e.children[i].as_element().unwrap();
        if e.is(W, "tbl") {
            return false;
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn scan_block(
    e: &Element,
    story: usize,
    path: &mut Vec<usize>,
    paras: &mut Vec<ParaLoc>,
    tables: &mut Vec<TableLoc>,
    cell: Option<(usize, usize, usize)>,
    textbox: bool,
    fallback: bool,
) {
    for (i, n) in e.children.iter().enumerate() {
        let Node::Element(c) = n else { continue };
        path.push(i);
        if c.is(W, "p") {
            paras.push(ParaLoc { story, path: path.clone(), index: if fallback { 0 } else { 1 }, cell, textbox });
            // Text boxes in the paragraph.
            scan_textboxes(c, story, path, paras, tables, fallback);
        } else if c.is(W, "tbl") {
            let ti = tables.len();
            tables.push(TableLoc { story, path: path.clone(), index: ti + 1, rows: Vec::new(), nested: cell.is_some() });
            let mut rows = Vec::new();
            collect_rows(c, path, &mut rows);
            for (ri, (rpath, cells)) in rows.iter().enumerate() {
                for (ci, cpath) in cells.iter().enumerate() {
                    let mut p2 = cpath.clone();
                    let tc = at(c, &cpath[path.len()..]);
                    scan_block(tc, story, &mut p2, paras, tables, Some((ti, ri, ci)), textbox, fallback);
                }
                tables[ti].rows.push(RowLoc { path: rpath.clone(), cells: cells.clone() });
            }
        } else if c.is(W, "sdt") || c.is(W, "sdtContent") || c.is(W, "customXml") || (c.ns() == MC && (c.local() == "AlternateContent" || c.local() == "Choice")) {
            scan_block(c, story, path, paras, tables, cell, textbox, fallback);
        } else if c.ns() == MC && c.local() == "Fallback" {
            scan_block(c, story, path, paras, tables, cell, textbox, true);
        }
        path.pop();
    }
}

fn collect_rows(tbl: &Element, base: &mut Vec<usize>, rows: &mut Vec<(Vec<usize>, Vec<Vec<usize>>)>) {
    for (i, n) in tbl.children.iter().enumerate() {
        let Node::Element(c) = n else { continue };
        base.push(i);
        if c.is(W, "tr") {
            let mut cells = Vec::new();
            collect_cells(c, base, &mut cells);
            rows.push((base.clone(), cells));
        } else if c.is(W, "sdt") || c.is(W, "sdtContent") || c.is(W, "customXml") {
            collect_rows(c, base, rows);
        }
        base.pop();
    }
}

fn collect_cells(tr: &Element, base: &mut Vec<usize>, cells: &mut Vec<Vec<usize>>) {
    for (i, n) in tr.children.iter().enumerate() {
        let Node::Element(c) = n else { continue };
        base.push(i);
        if c.is(W, "tc") {
            cells.push(base.clone());
        } else if c.is(W, "sdt") || c.is(W, "sdtContent") || c.is(W, "customXml") {
            collect_cells(c, base, cells);
        }
        base.pop();
    }
}

fn scan_textboxes(e: &Element, story: usize, path: &mut Vec<usize>, paras: &mut Vec<ParaLoc>, tables: &mut Vec<TableLoc>, fallback: bool) {
    for (i, n) in e.children.iter().enumerate() {
        let Node::Element(c) = n else { continue };
        if c.is(W, "p") || c.is(W, "tbl") {
            continue;
        }
        path.push(i);
        if c.is(W, "txbxContent") {
            scan_block(c, story, path, paras, tables, None, true, fallback);
        } else {
            let fb = fallback || (c.ns() == MC && c.local() == "Fallback");
            scan_textboxes(c, story, path, paras, tables, fb);
        }
        path.pop();
    }
}

fn context(t: &str, a: usize, b: usize) -> String {
    let chars: Vec<char> = t.chars().collect();
    let from = a.saturating_sub(30);
    let to = (b + 30).min(chars.len());
    let mut s = String::new();
    if from > 0 {
        s.push('…');
    }
    s.extend(&chars[from..a]);
    s.push('[');
    s.extend(&chars[a..b]);
    s.push(']');
    s.extend(&chars[b..to]);
    if to < chars.len() {
        s.push('…');
    }
    text::escape_line(&s)
}

pub const RPR_ORDER: &[&str] = &[
    "rStyle", "rFonts", "b", "bCs", "i", "iCs", "caps", "smallCaps", "strike", "dstrike", "outline", "shadow", "emboss", "imprint", "noProof",
    "snapToGrid", "vanish", "webHidden", "color", "spacing", "w", "kern", "position", "sz", "szCs", "highlight", "u", "effect", "bdr", "shd",
    "fitText", "vertAlign", "rtl", "cs", "em", "lang", "eastAsianLayout", "specVanish", "oMath", "rPrChange",
];
pub const PPR_ORDER: &[&str] = &[
    "pStyle", "keepNext", "keepLines", "pageBreakBefore", "framePr", "widowControl", "numPr", "suppressLineNumbers", "pBdr", "shd", "tabs",
    "suppressAutoHyphens", "kinsoku", "wordWrap", "overflowPunct", "topLinePunct", "autoSpaceDE", "autoSpaceDN", "bidi", "adjustRightInd",
    "snapToGrid", "spacing", "ind", "contextualSpacing", "mirrorIndents", "suppressOverlap", "jc", "textDirection", "textAlignment",
    "textboxTightWrap", "outlineLvl", "divId", "cnfStyle", "rPr", "sectPr", "pPrChange",
];
pub const TCPR_ORDER: &[&str] = &[
    "cnfStyle", "tcW", "gridSpan", "hMerge", "vMerge", "tcBorders", "shd", "noWrap", "tcMar", "textDirection", "tcFitText", "vAlign", "hideMark",
    "headers", "cellIns", "cellDel", "cellMerge", "tcPrChange",
];
pub const TBLPR_ORDER: &[&str] = &[
    "tblStyle", "tblpPr", "tblOverlap", "bidiVisual", "tblStyleRowBandSize", "tblStyleColBandSize", "tblW", "jc", "tblCellSpacing", "tblInd",
    "tblBorders", "shd", "tblLayout", "tblCellMar", "tblLook", "tblCaption", "tblDescription", "tblPrChange",
];
pub const TRPR_ORDER: &[&str] = &["cnfStyle", "divId", "gridBefore", "gridAfter", "wBefore", "wAfter", "cantSplit", "trHeight", "tblHeader", "tblCellSpacing", "jc", "hidden", "ins", "del", "trPrChange"];
const BORDER_ORDER: &[&str] = &["top", "start", "left", "bottom", "end", "right", "insideH", "insideV", "tl2br", "tr2bl"];

/// The child `local` of `e` (in W), made first among the children when missing (for pPr, rPr,
/// tcPr, tblPr, trPr: they come first in their parents).
pub fn ensure_first<'a>(e: &'a mut Element, local: &str) -> &'a mut Element {
    if let Some(i) = e.position(W, local) {
        return e.children[i].as_element_mut().unwrap();
    }
    let n = e.sibling(local);
    e.children.insert(0, Node::Element(n));
    e.children[0].as_element_mut().unwrap()
}

fn ensure_in_order<'a>(e: &'a mut Element, local: &str, order: &[&str]) -> &'a mut Element {
    if e.position(W, local).is_none() {
        let n = e.sibling(local);
        put_in_order(e, n, order);
    }
    let i = e.position(W, local).unwrap();
    e.children[i].as_element_mut().unwrap()
}

fn toggle(rpr: &mut Element, local: &str, on: bool) {
    let mut b = rpr.sibling(local);
    if !on {
        b.set_own("val", "0");
    }
    put_in_order(rpr, b, RPR_ORDER);
}

fn twips(pt: f64) -> i64 {
    (pt * 20.0).round() as i64
}

const HIGHLIGHTS: &[(&str, &str)] = &[
    ("yellow", "FFFF00"),
    ("green", "00FF00"),
    ("cyan", "00FFFF"),
    ("magenta", "FF00FF"),
    ("blue", "0000FF"),
    ("red", "FF0000"),
    ("darkBlue", "000080"),
    ("darkCyan", "008080"),
    ("darkGreen", "008000"),
    ("darkMagenta", "800080"),
    ("darkRed", "800000"),
    ("darkYellow", "808000"),
    ("darkGray", "808080"),
    ("lightGray", "C0C0C0"),
    ("black", "000000"),
    ("white", "FFFFFF"),
];

pub fn highlight_rgb(name: &str) -> Option<&'static str> {
    HIGHLIGHTS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, h)| *h)
}

pub fn apply_rpr(rpr: &mut Element, p: &Props, cstyle: Option<&str>) {
    if p.clear {
        rpr.children.retain(|n| n.as_element().map(|e| e.is(W, "rPrChange")).unwrap_or(false));
    }
    if let Some(s) = cstyle {
        let mut e = rpr.sibling("rStyle");
        e.set_own("val", s);
        put_in_order(rpr, e, RPR_ORDER);
    }
    let fonts = [(&p.font, &["ascii", "hAnsi", "eastAsia", "cs"][..]), (&p.font_latin, &["ascii", "hAnsi"][..]), (&p.font_ea, &["eastAsia"][..]), (&p.font_cs, &["cs"][..])];
    for (font, attrs) in fonts {
        if let Some(f) = font {
            let mut e = rpr.child(W, "rFonts").cloned().unwrap_or_else(|| rpr.sibling("rFonts"));
            for a in attrs {
                e.set_own(a, f);
                // A theme font would win over the name.
                let theme = match *a {
                    "ascii" => "asciiTheme",
                    "hAnsi" => "hAnsiTheme",
                    "eastAsia" => "eastAsiaTheme",
                    _ => "cstheme",
                };
                e.remove_own(theme);
            }
            put_in_order(rpr, e, RPR_ORDER);
        }
    }
    if let Some(b) = p.bold {
        toggle(rpr, "b", b);
        toggle(rpr, "bCs", b);
    }
    if let Some(i) = p.italic {
        toggle(rpr, "i", i);
        toggle(rpr, "iCs", i);
    }
    if let Some(c) = p.caps {
        toggle(rpr, "caps", c);
    }
    if let Some(c) = p.small_caps {
        toggle(rpr, "smallCaps", c);
    }
    if let Some(s) = p.strike {
        toggle(rpr, "strike", s);
    }
    if let Some(s) = p.dstrike {
        toggle(rpr, "dstrike", s);
    }
    if let Some(h) = p.hidden {
        toggle(rpr, "vanish", h);
    }
    if let Some(c) = &p.color {
        let mut e = rpr.sibling("color");
        e.set_own("val", match c {
            Color::Rgb(h) => h,
            _ => "auto",
        });
        put_in_order(rpr, e, RPR_ORDER);
    }
    if let Some(s) = p.letter_spacing {
        let mut e = rpr.sibling("spacing");
        e.set_own("val", &twips(s).to_string());
        put_in_order(rpr, e, RPR_ORDER);
    }
    if let Some(sz) = p.size {
        let hp = ((sz * 2.0).round() as i64).max(1).to_string();
        let mut e = rpr.sibling("sz");
        e.set_own("val", &hp);
        put_in_order(rpr, e, RPR_ORDER);
        let mut e = rpr.sibling("szCs");
        e.set_own("val", &hp);
        put_in_order(rpr, e, RPR_ORDER);
    }
    if let Some(h) = &p.highlight {
        match h {
            Color::None | Color::Auto => {
                rpr.remove_children(W, "highlight");
            }
            Color::Rgb(hex) => {
                if let Some((name, _)) = HIGHLIGHTS.iter().find(|(_, x)| x == hex) {
                    let mut e = rpr.sibling("highlight");
                    e.set_own("val", name);
                    put_in_order(rpr, e, RPR_ORDER);
                } else {
                    // Not one of Word's highlight colors: shading shows it.
                    rpr.remove_children(W, "highlight");
                    put_in_order(rpr, shd(rpr, hex), RPR_ORDER);
                }
            }
        }
    }
    if let Some(u) = &p.underline {
        if u == "none" {
            rpr.remove_children(W, "u");
        } else {
            let v = match u.as_str() {
                "single" | "solid" => "single",
                "double" => "double",
                "thick" | "bold" => "thick",
                "dotted" | "dot" => "dotted",
                "dash" | "dashed" => "dash",
                "wave" | "wavy" => "wave",
                "words" => "words",
                other => other,
            };
            let mut e = rpr.sibling("u");
            e.set_own("val", v);
            put_in_order(rpr, e, RPR_ORDER);
        }
    }
    if let Some(c) = &p.shade {
        match c {
            Color::Rgb(hex) => put_in_order(rpr, shd(rpr, hex), RPR_ORDER),
            _ => {
                rpr.remove_children(W, "shd");
            }
        }
    }
    if let Some(v) = &p.vert {
        match v {
            Vert::Baseline => {
                rpr.remove_children(W, "vertAlign");
            }
            _ => {
                let mut e = rpr.sibling("vertAlign");
                e.set_own("val", if *v == Vert::Super { "superscript" } else { "subscript" });
                put_in_order(rpr, e, RPR_ORDER);
            }
        }
    }
    if let Some(l) = &p.lang {
        let mut e = rpr.child(W, "lang").cloned().unwrap_or_else(|| rpr.sibling("lang"));
        let low = l.to_ascii_lowercase();
        if low.starts_with("ko") || low.starts_with("ja") || low.starts_with("zh") {
            e.set_own("eastAsia", l);
        } else if low.starts_with("ar") || low.starts_with("he") || low.starts_with("fa") {
            e.set_own("bidi", l);
        } else {
            e.set_own("val", l);
        }
        put_in_order(rpr, e, RPR_ORDER);
    }
}

fn shd(like: &Element, hex: &str) -> Element {
    let mut e = like.sibling("shd");
    e.set_own("val", "clear");
    e.set_own("color", "auto");
    e.set_own("fill", hex);
    e
}

fn apply_ppr(ppr: &mut Element, p: &Props, style: Option<&str>, num_id: Option<&str>) -> Result<()> {
    if let Some(s) = style {
        let mut e = ppr.sibling("pStyle");
        e.set_own("val", s);
        put_in_order(ppr, e, PPR_ORDER);
    }
    for (flag, name) in [(p.keep_next, "keepNext"), (p.keep_lines, "keepLines"), (p.page_break_before, "pageBreakBefore")] {
        if let Some(on) = flag {
            if on {
                put_in_order(ppr, ppr.sibling(name), PPR_ORDER);
            } else {
                let mut e = ppr.sibling(name);
                e.set_own("val", "0");
                put_in_order(ppr, e, PPR_ORDER);
            }
        }
    }
    if p.space_before.is_some() || p.space_after.is_some() || p.line.is_some() {
        let mut e = ppr.child(W, "spacing").cloned().unwrap_or_else(|| ppr.sibling("spacing"));
        if let Some(b) = p.space_before {
            e.set_own("before", &twips(b).to_string());
            e.remove_own("beforeLines");
            e.remove_own("beforeAutospacing");
        }
        if let Some(a) = p.space_after {
            e.set_own("after", &twips(a).to_string());
            e.remove_own("afterLines");
            e.remove_own("afterAutospacing");
        }
        if let Some(l) = &p.line {
            let (v, rule) = match l {
                LineSpacing::Multiple(m) => (((m * 240.0).round() as i64).to_string(), "auto"),
                LineSpacing::Exact(pt) => (twips(*pt).to_string(), "exact"),
                LineSpacing::AtLeast(pt) => (twips(*pt).to_string(), "atLeast"),
            };
            e.set_own("line", &v);
            e.set_own("lineRule", rule);
        }
        put_in_order(ppr, e, PPR_ORDER);
    }
    if p.indent_left.is_some() || p.indent_right.is_some() || p.indent_first.is_some() || p.indent_hanging.is_some() {
        let mut e = ppr.child(W, "ind").cloned().unwrap_or_else(|| ppr.sibling("ind"));
        if let Some(v) = p.indent_left {
            e.set_own("left", &twips(v).to_string());
            e.remove_own("start");
            e.remove_own("leftChars");
            e.remove_own("startChars");
        }
        if let Some(v) = p.indent_right {
            e.set_own("right", &twips(v).to_string());
            e.remove_own("end");
            e.remove_own("rightChars");
            e.remove_own("endChars");
        }
        if let Some(v) = p.indent_first {
            e.set_own("firstLine", &twips(v).to_string());
            e.remove_own("hanging");
            e.remove_own("firstLineChars");
            e.remove_own("hangingChars");
        }
        if let Some(v) = p.indent_hanging {
            e.set_own("hanging", &twips(v).to_string());
            e.remove_own("firstLine");
            e.remove_own("firstLineChars");
            e.remove_own("hangingChars");
        }
        put_in_order(ppr, e, PPR_ORDER);
    }
    if let Some(a) = &p.align {
        let mut e = ppr.sibling("jc");
        e.set_own("val", match a.as_str() {
            "justify" => "both",
            "distribute" => "distribute",
            "center" => "center",
            "right" => "right",
            _ => "left",
        });
        put_in_order(ppr, e, PPR_ORDER);
    }
    if let Some(l) = &p.list {
        if l == "none" {
            ppr.remove_children(W, "numPr");
            // Out of the list, out of its paragraph style too.
            if style.is_none() && ppr.child(W, "pStyle").and_then(|e| e.val()) == Some("ListParagraph") {
                ppr.remove_children(W, "pStyle");
            }
        } else if let Some(id) = num_id {
            let lvl = ppr.child(W, "numPr").and_then(|n| n.child(W, "ilvl")).and_then(|e| e.own("val")).unwrap_or("0").to_string();
            ppr.remove_children(W, "numPr");
            let mut num = ppr.sibling("numPr");
            let mut il = ppr.sibling("ilvl");
            il.set_own("val", &lvl);
            let mut ni = ppr.sibling("numId");
            ni.set_own("val", id);
            num.push(il);
            num.push(ni);
            put_in_order(ppr, num, PPR_ORDER);
        }
    }
    if let Some(l) = p.level {
        if l == 0 {
            return Err(err("level is 1 or more"));
        }
        if let Some(num) = ppr.child_mut(W, "numPr") {
            let mut e = num.sibling("ilvl");
            e.set_own("val", &(l - 1).to_string());
            put_in_order(num, e, &["ilvl", "numId"]);
        } else {
            let mut e = ppr.sibling("outlineLvl");
            e.set_own("val", &(l - 1).to_string());
            put_in_order(ppr, e, PPR_ORDER);
        }
    }
    if p.bullet.is_some() {
        return Err(err("bullet= is for slides; in Word use list=bullet|number (and level=N)"));
    }
    Ok(())
}

fn border_el(like: &Element, edge: &str, b: &crate::props::Border) -> Element {
    let mut e = like.sibling(edge);
    let style = match b.style.as_str() {
        "none" => "nil",
        "double" => "double",
        "dashed" => "dashed",
        "dotted" => "dotted",
        "thick" | "medium" => "thick",
        "dashdot" => "dotDash",
        _ => "single",
    };
    e.set_own("val", style);
    if style != "nil" {
        // Eighths of a point.
        e.set_own("sz", &((b.width * 8.0).round().max(2.0) as i64).to_string());
        e.set_own("space", "0");
        e.set_own("color", b.color.as_ref().and_then(|c| c.hex()).unwrap_or("auto"));
    }
    e
}

fn cell_props(tc: &mut Element, p: &Props) {
    let tcpr = ensure_first(tc, "tcPr");
    if let Some(f) = &p.fill {
        match f {
            Color::Rgb(hex) => put_in_order(tcpr, shd(tcpr, hex), TCPR_ORDER),
            _ => {
                tcpr.remove_children(W, "shd");
            }
        }
    }
    if let Some(v) = &p.valign {
        let mut e = tcpr.sibling("vAlign");
        e.set_own("val", match v.as_str() {
            "middle" => "center",
            "bottom" => "bottom",
            _ => "top",
        });
        put_in_order(tcpr, e, TCPR_ORDER);
    }
    if let Some(w) = p.width {
        let mut e = tcpr.sibling("tcW");
        e.set_own("w", &twips(w).to_string());
        e.set_own("type", "dxa");
        put_in_order(tcpr, e, TCPR_ORDER);
    }
    if !p.borders.is_empty() {
        let mut b = tcpr.child(W, "tcBorders").cloned().unwrap_or_else(|| tcpr.sibling("tcBorders"));
        for (edges, border) in &p.borders {
            let names: &[&str] = match edges {
                Edges::All | Edges::Outline => &["top", "left", "bottom", "right"],
                Edges::Top => &["top"],
                Edges::Bottom => &["bottom"],
                Edges::Left => &["left"],
                Edges::Right => &["right"],
                Edges::Inside | Edges::InsideH | Edges::InsideV => &[],
            };
            for n in names {
                let be = border_el(&b, n, border);
                            put_in_order(&mut b, be, BORDER_ORDER);
            }
        }
        b.touch();
        put_in_order(tcpr, b, TCPR_ORDER);
    }
    if p.align.is_some() {
        // Alignment of a cell is its paragraphs' (handled by the paragraph pass).
    }
}

/// A cell holds one paragraph with `text` (the first paragraph's properties and first run's
/// look); `\n\n` makes more paragraphs.
pub fn set_cell_text(tc: &mut Element, text_in: &str) {
    let first_p = tc.elements().find(|e| e.is(W, "p")).cloned().unwrap_or_else(|| tc.sibling("p"));
    tc.children.retain(|n| n.as_element().map(|e| e.is(W, "tcPr")).unwrap_or(false));
    // One paragraph per line, as in the other containers (shapes, notes).
    for line in text::lines_of(text_in) {
        let p = paragraph_like(&first_p, line);
        tc.push(p);
    }
}

/// A new paragraph with the properties of `template` and its first run's properties.
pub fn paragraph_like(template: &Element, text_in: &str) -> Element {
    let mut p = template.sibling("p");
    for a in template.attrs() {
        // rsids and paraId would be duplicated: only keep nothing.
        let _ = a;
    }
    if let Some(ppr) = template.child(W, "pPr") {
        let mut c = ppr.clone();
        c.remove_children(W, "pPrChange");
        if let Some(rpr) = c.child_mut(W, "rPr") {
            rpr.remove_children(W, "ins");
            rpr.remove_children(W, "del");
        }
        p.push(c);
    }
    if !text_in.is_empty() {
        let mut r = p.sibling("r");
        if let Some(rpr) = first_run(template).and_then(|r| r.child(W, "rPr")) {
            let mut c = rpr.clone();
            c.remove_children(W, "rPrChange");
            r.push(c);
        } else if let Some(rpr) = template.child(W, "pPr").and_then(|p| p.child(W, "rPr")) {
            let mut c = rpr.clone();
            c.remove_children(W, "ins");
            c.remove_children(W, "del");
            c.touch();
            r.push(c);
        }
        let t = Wml.new_text(&r, text_in);
        r.push(t);
        let k = r.children.len() - 1;
        p.push(r);
        let ri = p.children.len() - 1;
        Wml.settle(&mut p, ri, k);
    }
    p
}

fn wrap_link(para: &mut Element, runs: &[Vec<usize>], rid: &str) {
    // Consecutive runs with the same parent go into one hyperlink.
    let mut groups: Vec<(Vec<usize>, usize, usize)> = Vec::new();
    for r in runs {
        let (pp, ri) = r.split_at(r.len() - 1);
        match groups.last_mut() {
            Some((gp, _, end)) if gp.as_slice() == pp && *end + 1 == ri[0] => *end = ri[0],
            _ => groups.push((pp.to_vec(), ri[0], ri[0])),
        }
    }
    for (pp, a, b) in groups.into_iter().rev() {
        let parent = at_mut(para, &pp);
        if parent.is(W, "hyperlink") {
            parent.set_attr("r:id", R, rid);
            continue;
        }
        let mut h = parent.sibling("hyperlink");
        h.set_attr("r:id", R, rid);
        h.set_own("history", "1");
        let moved: Vec<Node> = parent.children.drain(a..=b).collect();
        h.children = moved;
        parent.children.insert(a, Node::Element(h));
    }
}

/// The runs of a paragraph with their direct properties, for `read --runs`.
pub fn runs_info(p: &Element) -> Vec<RunInfo> {
    let segs = text::segments(&Wml, p);
    let mut out: Vec<RunInfo> = Vec::new();
    let mut last: Option<Vec<usize>> = None;
    for s in segs {
        let t = s.display.clone().unwrap_or(s.text.clone());
        if last.as_ref() == Some(&s.run) {
            out.last_mut().unwrap().text.push_str(&t);
            continue;
        }
        last = Some(s.run.clone());
        let run = at(p, &s.run);
        out.push(RunInfo { text: t, props: rpr_summary(run.child(W, "rPr")), start: s.start });
    }
    out
}

pub fn rpr_summary(rpr: Option<&Element>) -> Vec<String> {
    let mut v = Vec::new();
    let Some(rpr) = rpr else { return v };
    for e in rpr.elements() {
        if e.ns() != W {
            continue;
        }
        let off = matches!(e.val(), Some("0") | Some("false") | Some("off"));
        let name = match e.local() {
            "b" => "bold",
            "i" => "italic",
            "strike" => "strike",
            "dstrike" => "dstrike",
            "caps" => "caps",
            "smallCaps" => "smallcaps",
            "vanish" => "hidden",
            "rStyle" => {
                v.push(format!("cstyle={}", e.val().unwrap_or("")));
                continue;
            }
            "color" => {
                let mut c = e.val().unwrap_or("auto").to_string();
                if let Some(t) = e.own("themeColor") {
                    c = format!("{c}({t})");
                }
                v.push(format!("color={c}"));
                continue;
            }
            "sz" => {
                let hp: f64 = e.val().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                v.push(format!("size={}pt", hp / 2.0));
                continue;
            }
            "rFonts" => {
                let mut fonts = Vec::new();
                if let Some(a) = e.own("ascii") {
                    fonts.push(format!("font={a}"));
                } else if let Some(a) = e.own("asciiTheme") {
                    fonts.push(format!("font=theme:{a}"));
                }
                if let Some(a) = e.own("eastAsia") {
                    fonts.push(format!("font.ea={a}"));
                }
                v.extend(fonts);
                continue;
            }
            "highlight" => {
                v.push(format!("highlight={}", e.val().unwrap_or("")));
                continue;
            }
            "u" => {
                v.push(format!("underline={}", e.val().unwrap_or("single")));
                continue;
            }
            "shd" => {
                if let Some(f) = e.own("fill") {
                    if f != "auto" {
                        v.push(format!("bg={f}"));
                    }
                }
                continue;
            }
            "vertAlign" => {
                match e.val() {
                    Some("superscript") => v.push("super".into()),
                    Some("subscript") => v.push("sub".into()),
                    _ => {}
                }
                continue;
            }
            "spacing" => {
                let tw: f64 = e.val().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                v.push(format!("spacing={}pt", tw / 20.0));
                continue;
            }
            "lang" => continue,
            _ => continue,
        };
        v.push(if off { format!("{name}=false") } else { name.to_string() });
    }
    v
}
