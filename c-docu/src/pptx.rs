//! PowerPoint presentations (.pptx, .pptm, .ppsx, .potx): PresentationML, edited in place.
//! Slides are `s1`, `s2`, …; shapes on a slide `s1.sh2` (in drawing order; `s1.sh3.sh1` inside a
//! group), their paragraphs `s1.sh2.p1`, table cells `s1.sh4.r1.c2`, speaker notes `s1.notes`.

use crate::dml::{self, Dml};
use crate::model::{Item, Match, Outline, RunInfo};
use crate::opc::{self, Package, Rels};
use crate::props::{Color, Props};
use crate::text::{self, at, at_mut, Dialect, Finder};
use crate::xml::{self, put_in_order, Document, Element, Node, A, MC, NONE, P, R};
use crate::{err, Result};

pub struct Slide {
    pub part: String,
    pub doc: Document,
    pub rels: Rels,
    pub notes: Option<(String, Document)>,
    pub layout: Option<String>,
    pub rid: String,
}

pub struct Pptx {
    pub pkg: Package,
    pub kind: &'static str,
    pub pres_part: String,
    pub pres: Document,
    pub pres_rels: Rels,
    pub slides: Vec<Slide>,
    /// Parts removed (slides deleted): not to be written back.
    removed: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ShapeLoc {
    /// Path from the slide's root element.
    pub path: Vec<usize>,
    /// `sh2`, `sh3.sh1`
    pub id: String,
    pub kind: &'static str,
    pub name: String,
    pub placeholder: Option<(String, String)>,
}

#[derive(Clone, Debug)]
pub enum Target {
    Slide(usize),
    Shape(usize, ShapeLoc),
    /// Paragraphs of a text body: (slide, the body's path, paragraph paths, range); the notes'
    /// body has slide index and `notes` set.
    Paras { slide: usize, notes: bool, body: Vec<usize>, paras: Vec<Vec<usize>>, first: usize, range: Option<(usize, usize)> },
    Cell { slide: usize, shape: ShapeLoc, row: usize, col: usize, cell: Vec<usize> },
    Row { slide: usize, shape: ShapeLoc, row: usize, tr: Vec<usize> },
    Notes(usize),
}

impl Pptx {
    pub fn open(pkg: Package, main: &str) -> Result<Pptx> {
        let ct = pkg.content_type(main).unwrap_or_default();
        let kind = if ct.contains("template") {
            "potx"
        } else if ct.contains("slideshow") {
            "ppsx"
        } else if ct.contains("macroEnabled") {
            "pptm"
        } else {
            "pptx"
        };
        let pres = pkg.xml(main)?;
        let pres_rels = pkg.rels(main)?;
        let mut slides = Vec::new();
        if let Some(lst) = pres.root.child(P, "sldIdLst") {
            for s in lst.children_named(P, "sldId") {
                let rid = s.attr_ns(R, "id").unwrap_or("").to_string();
                let Some(part) = pres_rels.part(&rid).and_then(|p| pkg.zip.real_name(&p)) else { continue };
                let doc = pkg.xml(&part)?;
                let rels = pkg.rels(&part)?;
                let notes = match rels.first_part("notesSlide").and_then(|p| pkg.zip.real_name(&p)) {
                    Some(np) => Some((np.clone(), pkg.xml(&np)?)),
                    None => None,
                };
                let layout = rels.first_part("slideLayout").and_then(|p| pkg.zip.real_name(&p));
                slides.push(Slide { part, doc, rels, notes, layout, rid });
            }
        }
        Ok(Pptx { pkg, kind, pres_part: main.to_string(), pres, pres_rels, slides, removed: Vec::new() })
    }

    pub(crate) fn sp_tree_path(doc: &Document) -> Option<Vec<usize>> {
        let csld = doc.root.position(P, "cSld")?;
        let tree = doc.root.children[csld].as_element()?.position(P, "spTree")?;
        Some(vec![csld, tree])
    }

    /// The shapes of a slide (or notes), in drawing order, groups opened.
    pub fn shapes(doc: &Document) -> Vec<ShapeLoc> {
        let mut out = Vec::new();
        let Some(tree) = Self::sp_tree_path(doc) else { return out };
        let tree_el = at(&doc.root, &tree);
        collect_shapes(tree_el, &tree, "", &mut out);
        out
    }

    pub fn slide_count(&self) -> usize {
        self.slides.len()
    }

    /// The text body of a shape (`p:txBody`, or a table's cells have their own).
    fn text_body_path(shape: &Element) -> Option<usize> {
        shape.position(P, "txBody")
    }

    fn layout_name(&self, i: usize) -> Option<String> {
        let l = self.slides[i].layout.as_ref()?;
        let d = self.pkg.xml(l).ok()?;
        d.root.child(P, "cSld").and_then(|c| c.own("name").or_else(|| c.attr_ns(NONE, "name"))).map(str::to_string)
    }

    fn slide_doc(&self, i: usize, notes: bool) -> Option<&Document> {
        if notes {
            self.slides[i].notes.as_ref().map(|n| &n.1)
        } else {
            Some(&self.slides[i].doc)
        }
    }

    fn slide_doc_mut(&mut self, i: usize, notes: bool) -> Option<&mut Document> {
        if notes {
            self.slides[i].notes.as_mut().map(|n| &mut n.1)
        } else {
            Some(&mut self.slides[i].doc)
        }
    }

    /// The notes' text body path (the body placeholder).
    fn notes_body(&self, i: usize) -> Option<Vec<usize>> {
        let doc = self.slide_doc(i, true)?;
        for s in Self::shapes(doc) {
            if s.placeholder.as_ref().map(|p| p.0 == "body").unwrap_or(false) {
                let el = at(&doc.root, &s.path);
                let tb = Self::text_body_path(el)?;
                let mut p = s.path.clone();
                p.push(tb);
                return Some(p);
            }
        }
        None
    }

    pub fn resolve(&self, addr: &str) -> Result<Target> {
        // A shape by name (s2.sh[Title 1]): its number, before anything else reads the address.
        let named;
        let addr = match addr.find("sh[") {
            Some(i) => {
                let close = addr[i..].find(']').map(|j| i + j).ok_or_else(|| err(format!("{addr:?}: sh[NAME] needs its ]")))?;
                let name = &addr[i + 3..close];
                let sn: usize = addr.strip_prefix('s').and_then(|r| r.split('.').next()).and_then(|n| n.parse().ok()).ok_or_else(|| err(format!("bad address {addr:?}")))?;
                let slide = self.slides.get(sn.wrapping_sub(1)).ok_or_else(|| err(format!("{addr}: the presentation has {} slides", self.slides.len())))?;
                let shapes = Self::shapes(&slide.doc);
                let sh = shapes.iter().find(|s| s.name == name).or_else(|| shapes.iter().find(|s| s.name.eq_ignore_ascii_case(name))).ok_or_else(|| {
                    let names: Vec<String> = shapes.iter().map(|s| format!("{} ({})", s.name, s.id)).collect();
                    err(format!("no shape named {name:?} on slide {sn}; it has: {}", names.join(", ")))
                })?;
                named = format!("{}{}{}", &addr[..i], sh.id, &addr[close + 1..]);
                named.as_str()
            }
            None => addr,
        };
        let (addr, range) = crate::docx::split_range(addr.trim())?;
        let parts: Vec<&str> = addr.split('.').collect();
        let sn: usize = parts[0].strip_prefix('s').and_then(|n| n.parse().ok()).ok_or_else(|| err(format!("bad address {addr:?} (s1, s1.sh2, s1.sh2.p1, s1.sh3.r1.c2, s1.notes)")))?;
        if sn == 0 || sn > self.slides.len() {
            return Err(err(format!("{addr}: the presentation has {} slides", self.slides.len())));
        }
        let si = sn - 1;
        if parts.len() == 1 {
            return Ok(Target::Slide(si));
        }
        if parts[1] == "notes" {
            let body = self.notes_body(si).ok_or_else(|| err(format!("slide {sn} has no notes")))?;
            let doc = self.slide_doc(si, true).unwrap();
            let paras = paragraphs_in(at(&doc.root, &body), &body);
            if parts.len() == 2 {
                if range.is_some() {
                    return Err(err("a character range needs a paragraph (s1.notes.p1[0:4])"));
                }
                return Ok(Target::Notes(si));
            }
            let (a, b) = para_range(parts[2], paras.len(), addr)?;
            return Ok(Target::Paras { slide: si, notes: true, body, paras: paras[a - 1..b].to_vec(), first: a, range });
        }
        // Shape path: sh2, sh3.sh1 (groups)
        let shapes = Self::shapes(&self.slides[si].doc);
        let mut k = 1;
        let mut id = String::new();
        while k < parts.len() && parts[k].starts_with("sh") {
            if !id.is_empty() {
                id.push('.');
            }
            id.push_str(parts[k]);
            k += 1;
        }
        let shape = shapes.iter().find(|s| s.id == id).cloned().ok_or_else(|| err(format!("{addr}: no shape {id} on slide {sn} (it has {})", shapes.iter().filter(|s| !s.id.contains('.')).count())))?;
        let slide_root = &self.slides[si].doc.root;
        let el = at(slide_root, &shape.path);
        if k == parts.len() {
            if range.is_some() {
                return Err(err("a character range needs a paragraph"));
            }
            return Ok(Target::Shape(si, shape));
        }
        let next = parts[k];
        if next.starts_with('p') {
            let tb = Self::text_body_path(el).ok_or_else(|| err(format!("{addr}: shape {id} has no text")))?;
            let mut body = shape.path.clone();
            body.push(tb);
            let paras = paragraphs_in(at(slide_root, &body), &body);
            let (a, b) = para_range(next, paras.len(), addr)?;
            return Ok(Target::Paras { slide: si, notes: false, body, paras: paras[a - 1..b].to_vec(), first: a, range });
        }
        if next.starts_with('r') {
            let tbl = table_path(el).ok_or_else(|| err(format!("{addr}: shape {id} is not a table")))?;
            let mut tpath = shape.path.clone();
            tpath.extend(tbl);
            let tbl_el = at(slide_root, &tpath);
            let rows: Vec<usize> = tbl_el.children.iter().enumerate().filter(|(_, n)| n.as_element().map(|e| e.is(A, "tr")).unwrap_or(false)).map(|(i, _)| i).collect();
            let rn: usize = next[1..].parse().map_err(|_| err(format!("bad row in {addr:?}")))?;
            if rn == 0 || rn > rows.len() {
                return Err(err(format!("{addr}: the table has {} rows", rows.len())));
            }
            let mut tr = tpath.clone();
            tr.push(rows[rn - 1]);
            if k + 1 == parts.len() {
                return Ok(Target::Row { slide: si, shape, row: rn - 1, tr });
            }
            let tr_el = at(slide_root, &tr);
            let cells: Vec<usize> = tr_el.children.iter().enumerate().filter(|(_, n)| n.as_element().map(|e| e.is(A, "tc")).unwrap_or(false)).map(|(i, _)| i).collect();
            let cn: usize = parts[k + 1].strip_prefix('c').and_then(|n| n.parse().ok()).ok_or_else(|| err(format!("expected cN in {addr:?}")))?;
            if cn == 0 || cn > cells.len() {
                return Err(err(format!("{addr}: row {rn} has {} cells", cells.len())));
            }
            let mut cell = tr.clone();
            cell.push(cells[cn - 1]);
            if k + 2 == parts.len() {
                return Ok(Target::Cell { slide: si, shape, row: rn - 1, col: cn - 1, cell });
            }
            let tc = at(slide_root, &cell);
            let tb = tc.position(A, "txBody").ok_or_else(|| err("the cell has no text body"))?;
            let mut body = cell.clone();
            body.push(tb);
            let paras = paragraphs_in(at(slide_root, &body), &body);
            let (a, b) = para_range(parts[k + 2], paras.len(), addr)?;
            return Ok(Target::Paras { slide: si, notes: false, body, paras: paras[a - 1..b].to_vec(), first: a, range });
        }
        Err(err(format!("bad address {addr:?}")))
    }

    /// Every text body of a slide (shapes, table cells, group members), with its address.
    fn bodies(&self, si: usize, notes: bool) -> Vec<(String, Vec<usize>)> {
        let mut out = Vec::new();
        if notes {
            if let Some(b) = self.notes_body(si) {
                out.push((format!("s{}.notes", si + 1), b));
            }
            return out;
        }
        let doc = &self.slides[si].doc;
        for sh in Self::shapes(doc) {
            let el = at(&doc.root, &sh.path);
            if let Some(tb) = Self::text_body_path(el) {
                let mut p = sh.path.clone();
                p.push(tb);
                out.push((format!("s{}.{}", si + 1, sh.id), p));
            }
            if let Some(t) = table_path(el) {
                let mut tp = sh.path.clone();
                tp.extend(t);
                let tbl = at(&doc.root, &tp);
                let mut ri = 0;
                for (i, n) in tbl.children.iter().enumerate() {
                    let Some(tr) = n.as_element().filter(|e| e.is(A, "tr")) else { continue };
                    ri += 1;
                    let mut ci = 0;
                    for (j, m) in tr.children.iter().enumerate() {
                        let Some(tc) = m.as_element().filter(|e| e.is(A, "tc")) else { continue };
                        ci += 1;
                        if let Some(tb) = tc.position(A, "txBody") {
                            let mut p = tp.clone();
                            p.extend([i, j, tb]);
                            out.push((format!("s{}.{}.r{ri}.c{ci}", si + 1, sh.id), p));
                        }
                    }
                }
            }
        }
        out
    }

    pub fn outline(&self, with_runs: bool, filter: &[String]) -> Result<Outline> {
        let mut items = Vec::new();
        for si in 0..self.slides.len() {
            let doc = &self.slides[si].doc;
            let mut it = Item { addr: format!("s{}", si + 1), kind: "slide".into(), ..Default::default() };
            if let Some(l) = self.layout_name(si) {
                it.style = Some(l);
            }
            if doc.root.own("show") == Some("0") || doc.root.attr_ns(NONE, "show") == Some("0") {
                it.extra.push(("hidden".into(), String::new()));
            }
            items.push(it);
            let bodies = self.bodies(si, false);
            for sh in Self::shapes(doc) {
                let el = at(&doc.root, &sh.path);
                let depth = 1 + sh.id.matches('.').count();
                let saddr = format!("s{}.{}", si + 1, sh.id);
                let mut it = Item { addr: saddr.clone(), kind: sh.kind.into(), depth, ..Default::default() };
                if let Some((t, idx)) = &sh.placeholder {
                    it.place = Some(if idx.is_empty() { t.clone() } else { format!("{t} {idx}") });
                }
                it.extra.push(("name".into(), sh.name.clone()));
                if let Some((x, y, w, h)) = self.geometry(si, &sh) {
                    it.extra.push(("box".into(), format!("{:.1},{:.1} {:.1}x{:.1}cm", x as f64 / 360000.0, y as f64 / 360000.0, w as f64 / 360000.0, h as f64 / 360000.0)));
                }
                if let Some(t) = table_path(el) {
                    let mut tp = sh.path.clone();
                    tp.extend(t);
                    let tbl = at(&doc.root, &tp);
                    let rows = tbl.children_named(A, "tr").count();
                    let cols = tbl.children_named(A, "tr").next().map(|r| r.children_named(A, "tc").count()).unwrap_or(0);
                    it.text = format!("{rows} rows x {cols} columns");
                }
                if sh.kind == "picture" {
                    if let Some(d) = el.descendants().iter().find(|e| e.local() == "cNvPr").and_then(|e| e.attr_ns(NONE, "descr")) {
                        it.extra.push(("alt".into(), d.to_string()));
                    }
                }
                items.push(it);
                for (addr, body) in bodies.iter().filter(|(a, _)| *a == saddr || a.starts_with(&format!("{saddr}.r"))) {
                    self.body_items(doc, addr, body, depth + 1, with_runs, &mut items);
                }
            }
            for (addr, body) in self.bodies(si, true) {
                items.push(Item { addr: addr.clone(), kind: "notes".into(), depth: 1, ..Default::default() });
                self.body_items(self.slide_doc(si, true).unwrap(), &addr, &body, 2, with_runs, &mut items);
            }
        }
        // Shapes named in the filter (s2.sh[Title 1].p1) by their numbers.
        let filter: Vec<String> = filter
            .iter()
            .map(|f| match (f.find("sh["), f.find(']')) {
                (Some(a), Some(b)) if b > a => match self.resolve(&f[..=b])? {
                    Target::Shape(si, loc) => Ok(format!("s{}.{}{}", si + 1, loc.id, &f[b + 1..])),
                    _ => Ok(f.clone()),
                },
                _ => Ok(f.clone()),
            })
            .collect::<Result<_>>()?;
        Ok(crate::doc::filter_outline(Outline { items }, &filter))
    }

    fn body_items(&self, d: &Document, addr: &str, body: &[usize], depth: usize, with_runs: bool, items: &mut Vec<Item>) {
        let b = at(&d.root, body);
        let mut n = 0;
        for p in b.elements() {
            if !p.is(A, "p") {
                continue;
            }
            n += 1;
            let t = text::display_of(&Dml, p);
            if t.is_empty() && !with_runs {
                continue;
            }
            let mut it = Item { addr: format!("{addr}.p{n}"), kind: "p".into(), text: t, depth, ..Default::default() };
            if let Some(l) = p.child(A, "pPr").and_then(|pp| pp.attr_ns(NONE, "lvl")).and_then(|l| l.parse::<u32>().ok()) {
                if l > 0 {
                    it.extra.push(("level".into(), (l + 1).to_string()));
                }
            }
            if with_runs {
                it.runs = runs_info(p);
            }
            items.push(it);
        }
    }

    /// Position and size in EMU, the placeholder's (from the layout or master) when the shape has
    /// none of its own.
    pub fn geometry(&self, si: usize, sh: &ShapeLoc) -> Option<(i64, i64, i64, i64)> {
        let el = at(&self.slides[si].doc.root, &sh.path);
        if let Some(g) = xfrm_of(el) {
            return Some(g);
        }
        let (t, idx) = sh.placeholder.clone()?;
        let layout = self.slides[si].layout.as_ref()?;
        let ld = self.pkg.xml(layout).ok()?;
        if let Some(g) = placeholder_xfrm(&ld, &t, &idx) {
            return Some(g);
        }
        let lrels = self.pkg.rels(layout).ok()?;
        let master = lrels.first_part("slideMaster")?;
        let md = self.pkg.xml(&master).ok()?;
        placeholder_xfrm(&md, &t, &idx)
    }

    pub fn find(&self, f: &Finder, scope: Option<&str>) -> Result<Vec<Match>> {
        let mut out = Vec::new();
        for (si, notes, addr, body) in self.scope_bodies(scope)? {
            let d = self.slide_doc(si, notes).unwrap();
            let b = at(&d.root, &body);
            let mut n = 0;
            for p in b.elements() {
                if !p.is(A, "p") {
                    continue;
                }
                n += 1;
                let t = text::text_of(&Dml, p);
                for (a, e, _) in f.find(&t, None) {
                    out.push(Match { addr: format!("{addr}.p{n}"), start: a, end: e, text: t.chars().skip(a).take(e - a).collect(), context: text::escape_line(&t) });
                }
            }
        }
        Ok(out)
    }

    /// The text bodies a scope names: (slide, notes?, address, body path).
    fn scope_bodies(&self, scope: Option<&str>) -> Result<Vec<(usize, bool, String, Vec<usize>)>> {
        let all = |si: usize| -> Vec<(usize, bool, String, Vec<usize>)> {
            let mut v: Vec<_> = self.bodies(si, false).into_iter().map(|(a, b)| (si, false, a, b)).collect();
            v.extend(self.bodies(si, true).into_iter().map(|(a, b)| (si, true, a, b)));
            v
        };
        Ok(match scope {
            None => (0..self.slides.len()).flat_map(all).collect(),
            Some(s) => match self.resolve(s)? {
                Target::Slide(si) => all(si),
                Target::Notes(si) => self.bodies(si, true).into_iter().map(|(a, b)| (si, true, a, b)).collect(),
                Target::Shape(si, sh) => {
                    let prefix = format!("s{}.{}", si + 1, sh.id);
                    self.bodies(si, false).into_iter().filter(|(a, _)| a == &prefix || a.starts_with(&format!("{prefix}."))).map(|(a, b)| (si, false, a, b)).collect()
                }
                Target::Cell { slide, cell, .. } => self.bodies(slide, false).into_iter().filter(|(_, b)| b.starts_with(&cell)).map(|(a, b)| (slide, false, a, b)).collect(),
                Target::Row { slide, tr, .. } => self.bodies(slide, false).into_iter().filter(|(_, b)| b.starts_with(&tr)).map(|(a, b)| (slide, false, a, b)).collect(),
                Target::Paras { slide, notes, body, .. } => {
                    let a = if notes { format!("s{}.notes", slide + 1) } else { self.bodies(slide, false).into_iter().find(|(_, b)| *b == body).map(|(a, _)| a).unwrap_or_default() };
                    vec![(slide, notes, a, body)]
                }
            },
        })
    }

    pub fn replace(&mut self, f: &Finder, with: &str, scope: Option<&str>, nth: Option<usize>) -> Result<Vec<String>> {
        let with = text::unescape(with);
        let only: Option<(Vec<Vec<usize>>, Option<(usize, usize)>)> = match scope.map(|s| self.resolve(s)).transpose()? {
            Some(Target::Paras { paras, range, .. }) => Some((paras, range)),
            _ => None,
        };
        let mut done = Vec::new();
        let mut count = 0;
        for (si, notes, addr, body) in self.scope_bodies(scope)? {
            let d = self.slide_doc_mut(si, notes).unwrap();
            let b = at_mut(&mut d.root, &body);
            let mut n = 0;
            for k in 0..b.children.len() {
                let Some(p) = b.children[k].as_element_mut().filter(|e| e.is(A, "p")) else { continue };
                n += 1;
                if let Some((ps, _)) = &only {
                    let mut full = body.clone();
                    full.push(k);
                    if !ps.contains(&full) {
                        continue;
                    }
                }
                let t = text::text_of(&Dml, p);
                let mut ms = f.find(&t, Some(&with));
                if let Some((_, Some((ra, rb)))) = &only {
                    ms.retain(|(a, b, _)| a >= ra && b <= rb);
                }
                let mut chosen = Vec::new();
                for m in ms {
                    count += 1;
                    if nth.map(|x| x == count).unwrap_or(true) {
                        chosen.push(m);
                    }
                }
                for (a, e, rep) in chosen.iter().rev() {
                    text::replace_range(&Dml, p, *a, *e, rep)?;
                    done.push(format!("{addr}.p{n}"));
                }
            }
        }
        Ok(done)
    }

    pub fn set_text(&mut self, addr: &str, value: &str) -> Result<String> {
        let value = text::unescape(value);
        match self.resolve(addr)? {
            Target::Paras { slide, notes, paras, range, .. } => {
                if paras.len() != 1 {
                    return Err(err("set takes one paragraph, a shape, a cell or the notes"));
                }
                let d = self.slide_doc_mut(slide, notes).unwrap();
                let p = at_mut(&mut d.root, &paras[0]);
                let len = text::text_of(&Dml, p).chars().count();
                let (a, b) = range.unwrap_or((0, len));
                text::replace_range(&Dml, p, a, b.min(len), &value)?;
                Ok(addr.to_string())
            }
            Target::Shape(si, sh) => {
                let el = at(&self.slides[si].doc.root, &sh.path);
                let tb = Self::text_body_path(el).ok_or_else(|| err(format!("{addr} has no text (a picture or table?)")))?;
                let mut body = sh.path.clone();
                body.push(tb);
                let b = at_mut(&mut self.slides[si].doc.root, &body);
                set_body_text(b, &value);
                Ok(addr.to_string())
            }
            Target::Cell { slide, cell, .. } => {
                let tc = at_mut(&mut self.slides[slide].doc.root, &cell);
                if tc.child(A, "txBody").is_none() {
                    let tb = fragment_a(tc, "<a:txBody><a:bodyPr/><a:lstStyle/><a:p/></a:txBody>")?;
                    tc.children.insert(0, Node::Element(tb));
                }
                let b = tc.child_mut(A, "txBody").unwrap();
                set_body_text(b, &value);
                Ok(addr.to_string())
            }
            Target::Notes(si) => {
                let body = self.notes_body(si).unwrap();
                let d = self.slide_doc_mut(si, true).unwrap();
                set_body_text(at_mut(&mut d.root, &body), &value);
                Ok(addr.to_string())
            }
            Target::Slide(_) | Target::Row { .. } => Err(err("set takes a shape (s1.sh2), a paragraph, a table cell or the notes")),
        }
    }

    pub fn style(&mut self, addr: &str, props: &Props, matching: Option<(&Finder, Option<usize>)>) -> Result<Vec<String>> {
        let target = self.resolve(addr)?;
        const SHAPE_KEYS: &[&str] = &["fill", "line.color", "line.width", "x", "y", "w", "h", "rotate", "name", "alt", "valign", "autofit", "wrap"];
        let shape_props = props.keys.iter().any(|k| SHAPE_KEYS.contains(&k.as_str()));
        let text_props = props.has_char() || props.has_para();
        let mut done = Vec::new();
        // Shape (or cell) properties.
        if shape_props {
            match &target {
                Target::Shape(si, sh) => {
                    let geom = self.geometry(*si, sh);
                    let el = at_mut(&mut self.slides[*si].doc.root, &sh.path);
                    apply_shape(el, props, geom)?;
                    done.push(addr.to_string());
                }
                Target::Cell { slide, cell, .. } => {
                    let tc = at_mut(&mut self.slides[*slide].doc.root, cell);
                    apply_cell(tc, props)?;
                    done.push(addr.to_string());
                }
                Target::Row { slide, tr, .. } => {
                    let tr_el = at_mut(&mut self.slides[*slide].doc.root, tr);
                    if let Some(h) = props.height {
                        tr_el.set_attr("h", NONE, &dml::emu(h).to_string());
                    }
                    for tc in tr_el.elements_mut() {
                        if tc.is(A, "tc") {
                            apply_cell(tc, props)?;
                        }
                    }
                    done.push(addr.to_string());
                }
                _ => return Err(err(format!("{} apply to shapes and table cells (s1.sh2, s1.sh3.r1.c1)", props.keys.iter().filter(|k| SHAPE_KEYS.contains(&k.as_str())).cloned().collect::<Vec<_>>().join(", ")))),
            }
        }
        if !text_props {
            return Ok(done);
        }
        // Text: the paragraphs the target holds.
        let (slide, notes, paras, range): (usize, bool, Vec<Vec<usize>>, Option<(usize, usize)>) = match &target {
            Target::Paras { slide, notes, paras, range, .. } => (*slide, *notes, paras.clone(), *range),
            Target::Slide(si) => (*si, false, self.bodies(*si, false).into_iter().flat_map(|(_, b)| paragraphs_in(at(&self.slides[*si].doc.root, &b), &b)).collect(), None),
            Target::Notes(si) => {
                let b = self.notes_body(*si).unwrap();
                (*si, true, paragraphs_in(at(&self.slide_doc(*si, true).unwrap().root, &b), &b), None)
            }
            Target::Shape(si, sh) => {
                let prefix = format!("s{}.{}", si + 1, sh.id);
                let bodies: Vec<Vec<usize>> = self.bodies(*si, false).into_iter().filter(|(a, _)| a == &prefix || a.starts_with(&format!("{prefix}."))).map(|(_, b)| b).collect();
                (*si, false, bodies.iter().flat_map(|b| paragraphs_in(at(&self.slides[*si].doc.root, b), b)).collect(), None)
            }
            Target::Cell { slide, cell, .. } | Target::Row { slide, tr: cell, .. } => {
                let bodies: Vec<Vec<usize>> = self.bodies(*slide, false).into_iter().filter(|(_, b)| b.starts_with(cell)).map(|(_, b)| b).collect();
                (*slide, false, bodies.iter().flat_map(|b| paragraphs_in(at(&self.slides[*slide].doc.root, b), b)).collect(), None)
            }
        };
        let mut count = 0;
        let mut styled_any = false;
        for pp in &paras {
            let d = self.slide_doc_mut(slide, notes).unwrap();
            let p = at_mut(&mut d.root, pp);
            if props.has_para() {
                let ppr = ensure_ppr(p);
                dml::apply_ppr(ppr, props);
                styled_any = true;
            }
            if !props.has_char() {
                continue;
            }
            let t = text::text_of(&Dml, p);
            let len = t.chars().count();
            let ranges: Vec<(usize, usize)> = match matching {
                Some((f, nth)) => {
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
                None => vec![range.map(|(a, b)| (a, b.min(len))).unwrap_or((0, len))],
            };
            for (a, b) in ranges.iter().rev() {
                if a >= b && len > 0 {
                    continue;
                }
                let runs = text::runs_in(&Dml, p, *a, *b);
                for rp in &runs {
                    let run = at_mut(p, rp);
                    let rpr = ensure_rpr(run);
                    dml::apply_rpr(rpr, props);
                }
                if *a == 0 && *b == len && matching.is_none() {
                    // The paragraph's end mark too (what new text typed there gets).
                    if let Some(e) = p.child_mut(A, "endParaRPr") {
                        dml::apply_rpr(e, props);
                    }
                    if len == 0 && runs.is_empty() {
                        let mut e = p.child(A, "endParaRPr").cloned().unwrap_or_else(|| Element::new(&p.qualify("endParaRPr"), A));
                        dml::apply_rpr(&mut e, props);
                        put_in_order(p, e, &["pPr", "r", "br", "fld", "endParaRPr"]);
                    }
                }
                styled_any = true;
            }
        }
        if matching.is_some() && !styled_any {
            return Err(err("no text matched"));
        }
        done.push(addr.to_string());
        Ok(done)
    }

    pub fn insert(&mut self, addr: &str, after: bool, text_in: &str, props: Option<&Props>) -> Result<Vec<String>> {
        let value = text::unescape(text_in);
        match self.resolve(addr)? {
            Target::Paras { slide, notes, paras, first, .. } => {
                let pp = if after { paras.last().unwrap().clone() } else { paras[0].clone() };
                let d = self.slide_doc_mut(slide, notes).unwrap();
                let template = at(&d.root, &pp).clone();
                let (bp, pi) = pp.split_at(pp.len() - 1);
                let body = at_mut(&mut d.root, bp);
                let lines: Vec<&str> = text::lines_of(&value);
                let at_i = if after { pi[0] + 1 } else { pi[0] };
                for (k, line) in lines.iter().enumerate() {
                    body.children.insert(at_i + k, Node::Element(paragraph_like(&template, line)));
                }
                let base = addr.rsplit_once('.').map(|(a, _)| a).unwrap_or(addr);
                let base = crate::docx::split_range(base)?.0.to_string();
                let n0 = if after { first + paras.len() } else { first };
                let addrs: Vec<String> = (0..lines.len()).map(|k| format!("{base}.p{}", n0 + k)).collect();
                if let Some(pr) = props {
                    for a in &addrs {
                        self.style(a, pr, None)?;
                    }
                }
                Ok(addrs)
            }
            Target::Row { slide, tr, row, shape } => {
                let cells: Vec<String> = if value.contains('|') { value.split('|').map(|s| s.trim().to_string()).collect() } else { value.split('\t').map(str::to_string).collect() };
                let root = &mut self.slides[slide].doc.root;
                let mut new_tr = at(root, &tr).clone();
                let mut k = 0;
                for tc in new_tr.elements_mut() {
                    if tc.is(A, "tc") {
                        tc.remove_attr(NONE, "vMerge");
                        tc.remove_attr(NONE, "rowSpan");
                        if let Some(b) = tc.child_mut(A, "txBody") {
                            set_body_text(b, cells.get(k).map(|s| s.as_str()).unwrap_or(""));
                        }
                        k += 1;
                    }
                }
                let (tp, ri) = tr.split_at(tr.len() - 1);
                let tbl = at_mut(root, tp);
                tbl.children.insert(if after { ri[0] + 1 } else { ri[0] }, Node::Element(new_tr));
                // The frame grows by the row's height.
                let h: i64 = at(root, &tr).attr_ns(NONE, "h").and_then(|v| v.parse().ok()).unwrap_or(370840);
                let frame = at_mut(root, &shape.path);
                if let Some(ext) = frame.child_mut(P, "xfrm").and_then(|x| x.child_mut(A, "ext")) {
                    if let Some(cy) = ext.attr_ns(NONE, "cy").and_then(|v| v.parse::<i64>().ok()) {
                        ext.set_attr("cy", NONE, &(cy + h).to_string());
                    }
                }
                Ok(vec![format!("s{}.{}.r{}", slide + 1, shape.id, if after { row + 2 } else { row + 1 })])
            }
            Target::Shape(si, sh) => {
                // A paragraph at the end (or start) of the shape's text.
                let el = at(&self.slides[si].doc.root, &sh.path);
                let tb = Self::text_body_path(el).ok_or_else(|| err(format!("{addr} has no text")))?;
                let mut body = sh.path.clone();
                body.push(tb);
                let paras = paragraphs_in(at(&self.slides[si].doc.root, &body), &body);
                let n = if after { paras.len() } else { 1 };
                self.insert(&format!("{addr}.p{n}"), after, text_in, props)
            }
            _ => Err(err("insert takes a paragraph (s1.sh2.p1), a shape (s1.sh2, adds a paragraph) or a table row (s1.sh3.r2); for slides: docu slide FILE add")),
        }
    }

    pub fn delete(&mut self, addr: &str) -> Result<String> {
        match self.resolve(addr)? {
            Target::Slide(si) => {
                self.delete_slide(si)?;
                Ok(format!("deleted slide {}", si + 1))
            }
            Target::Shape(si, sh) => {
                let (pp, i) = sh.path.split_at(sh.path.len() - 1);
                at_mut(&mut self.slides[si].doc.root, pp).children.remove(i[0]);
                Ok(format!("deleted {addr} ({})", sh.name))
            }
            Target::Paras { slide, notes, paras, range, .. } => {
                let d = self.slide_doc_mut(slide, notes).unwrap();
                if let Some((a, b)) = range {
                    let p = at_mut(&mut d.root, &paras[0]);
                    text::replace_range(&Dml, p, a, b, "")?;
                    return Ok(format!("{addr}: removed characters {a}..{b}"));
                }
                let (bp, _) = paras[0].split_at(paras[0].len() - 1);
                let total = at(&d.root, bp).children_named(A, "p").count();
                for pp in paras.iter().rev() {
                    let (bp, pi) = pp.split_at(pp.len() - 1);
                    let body = at_mut(&mut d.root, bp);
                    if total == paras.len() && pp == &paras[0] {
                        // A text body keeps one paragraph.
                        let p = body.children[pi[0]].as_element_mut().unwrap();
                        p.children.retain(|n| n.as_element().map(|e| e.is(A, "pPr") || e.is(A, "endParaRPr")).unwrap_or(false));
                    } else {
                        body.children.remove(pi[0]);
                    }
                }
                Ok(format!("deleted {} paragraph{}", paras.len(), if paras.len() == 1 { "" } else { "s" }))
            }
            Target::Row { slide, tr, .. } => {
                let root = &mut self.slides[slide].doc.root;
                let h: i64 = at(root, &tr).attr_ns(NONE, "h").and_then(|v| v.parse().ok()).unwrap_or(0);
                let (tp, ri) = tr.split_at(tr.len() - 1);
                at_mut(root, tp).children.remove(ri[0]);
                let _ = h;
                Ok(format!("deleted {addr}"))
            }
            Target::Cell { slide, cell, .. } => {
                let tc = at_mut(&mut self.slides[slide].doc.root, &cell);
                if let Some(b) = tc.child_mut(A, "txBody") {
                    set_body_text(b, "");
                }
                Ok(format!("cleared {addr}"))
            }
            Target::Notes(si) => {
                let body = self.notes_body(si).unwrap();
                let d = self.slide_doc_mut(si, true).unwrap();
                set_body_text(at_mut(&mut d.root, &body), "");
                Ok(format!("cleared the notes of slide {}", si + 1))
            }
        }
    }

    // ── slides ──────────────────────────────────────────────────────────────────────────────

    fn layouts(&self) -> Vec<(String, String)> {
        // (part, name) of every layout of every master.
        let mut out = Vec::new();
        for m in self.pres_rels.by_kind("slideMaster") {
            let Some(mp) = self.pres_rels.part(&m.id) else { continue };
            let Ok(mrels) = self.pkg.rels(&mp) else { continue };
            for l in mrels.by_kind("slideLayout") {
                let Some(lp) = mrels.part(&l.id) else { continue };
                let name = self.pkg.xml(&lp).ok().and_then(|d| d.root.child(P, "cSld").and_then(|c| c.attr_ns(NONE, "name")).map(str::to_string)).unwrap_or_default();
                out.push((lp, name));
            }
        }
        out
    }

    /// A new slide from a layout (by name, or the one the slide before uses), after `after`.
    pub fn add_slide(&mut self, layout: Option<&str>, after: Option<usize>, title: Option<&str>) -> Result<usize> {
        let layouts = self.layouts();
        if layouts.is_empty() {
            return Err(err("the presentation has no slide layouts"));
        }
        let pos = after.map(|a| a + 1).unwrap_or(self.slides.len());
        let lp = match layout {
            Some(name) => {
                let norm = |s: &str| s.to_lowercase().replace([' ', '_', '-'], "");
                layouts.iter().find(|(_, n)| norm(n) == norm(name)).or_else(|| layouts.iter().find(|(_, n)| norm(n).contains(&norm(name)))).map(|(p, _)| p.clone()).ok_or_else(|| {
                    err(format!("no layout {name:?}; the layouts are: {}", layouts.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>().join(", ")))
                })?
            }
            None => {
                let near = if pos > 0 { self.slides.get(pos - 1).and_then(|s| s.layout.clone()) } else { None };
                near.or_else(|| layouts.iter().find(|(_, n)| n.to_lowercase().contains("content")).map(|(p, _)| p.clone())).unwrap_or_else(|| layouts[0].0.clone())
            }
        };
        let ld = self.pkg.xml(&lp)?;
        // The layout's placeholders, empty.
        let mut shapes = String::new();
        let mut id = 2;
        for sh in Self::shapes(&ld) {
            let el = at(&ld.root, &sh.path);
            let Some((t, idx)) = &sh.placeholder else { continue };
            if matches!(t.as_str(), "dt" | "ftr" | "sldNum") {
                continue;
            }
            let name = xml::escape(&sh.name);
            let ph = format!("<p:ph{}{}/>", if t == "body" && !idx.is_empty() { String::new() } else { format!(" type=\"{t}\"") }, if idx.is_empty() { String::new() } else { format!(" idx=\"{idx}\"") });
            let txt = if (t == "title" || t == "ctrTitle") && title.is_some() { format!("<a:r><a:rPr lang=\"en-US\" dirty=\"0\"/><a:t>{}</a:t></a:r>", xml::escape(title.unwrap())) } else { String::new() };
            if el.is(P, "sp") {
                shapes.push_str(&format!("<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p>{txt}<a:endParaRPr lang=\"en-US\" dirty=\"0\"/></a:p></p:txBody></p:sp>"));
                id += 1;
            }
        }
        let xml_text = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
        );
        let folder = format!("{}/slides", opc::folder(&self.pres_part));
        let part = self.pkg.free_name(&format!("{folder}/slide{{}}.xml"));
        let doc = Document::parse(&xml_text)?;
        let mut rels = self.pkg.rels(&part)?;
        rels.add_part("http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout", &lp);
        self.insert_slide(part, doc, rels, Some(lp), None, pos)?;
        Ok(pos)
    }

    fn insert_slide(&mut self, part: String, doc: Document, mut rels: Rels, layout: Option<String>, notes: Option<(String, Document)>, pos: usize) -> Result<()> {
        self.pkg.set_content_type(&part, "application/vnd.openxmlformats-officedocument.presentationml.slide+xml", false);
        rels.save(&mut self.pkg);
        let rid = self.pres_rels.add_part("http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide", &part);
        if self.pres.root.child(P, "sldIdLst").is_none() {
            let e = Element::new(&self.pres.root.qualify("sldIdLst"), P);
            put_in_order(&mut self.pres.root, e, &["sldMasterIdLst", "notesMasterIdLst", "handoutMasterIdLst", "sldIdLst", "sldSz", "notesSz"]);
        }
        let rp = self.pres.prefix_for(R);
        let lst = self.pres.root.child_mut(P, "sldIdLst").unwrap();
        let next = lst.children_named(P, "sldId").filter_map(|e| e.attr_ns(NONE, "id").and_then(|v| v.parse::<u32>().ok())).max().unwrap_or(255).max(255) + 1;
        let mut e = Element::new(&lst.qualify("sldId"), P);
        e.set_attr("id", NONE, &next.to_string());
        e.set_attr(&format!("{rp}:id"), R, &rid);
        let mut k = 0;
        let mut at_i = lst.children.len();
        for (i, n) in lst.children.iter().enumerate() {
            if n.as_element().map(|e| e.is(P, "sldId")).unwrap_or(false) {
                if k == pos {
                    at_i = i;
                    break;
                }
                k += 1;
            }
        }
        lst.children.insert(at_i, Node::Element(e));
        let rels = self.pkg.rels(&part)?;
        self.slides.insert(pos, Slide { part, doc, rels, notes, layout, rid });
        Ok(())
    }

    /// A copy of slide `si` right after it (pictures shared; charts and notes copied).
    pub fn duplicate_slide(&mut self, si: usize) -> Result<usize> {
        let src_part = self.slides[si].part.clone();
        let layout = self.slides[si].layout.clone();
        let folder = opc::folder(&src_part).to_string();
        let part = self.pkg.free_name(&format!("{folder}/slide{{}}.xml"));
        let doc = Document::parse(&self.slides[si].doc.to_string())?;
        let src_rels = self.pkg.rels(&src_part)?;
        let mut rels = self.pkg.rels(&part)?;
        // Same ids, so the slide's r:id references stay right.
        let mut notes = None;
        for r in &src_rels.list {
            let target_part = if r.external { None } else { Some(opc::resolve(&src_part, &r.target)) };
            match (r.kind(), target_part) {
                ("notesSlide", Some(np)) => {
                    // A new notes slide pointing at the copy.
                    let nd = self.pkg.xml(&np)?;
                    let nfolder = opc::folder(&np).to_string();
                    let npart = self.pkg.free_name(&format!("{nfolder}/notesSlide{{}}.xml"));
                    let nrels_src = self.pkg.rels(&np)?;
                    let mut nrels = self.pkg.rels(&npart)?;
                    for nr in &nrels_src.list {
                        let tgt = if nr.kind() == "slide" { opc::relative(&npart, &part) } else if nr.external { nr.target.clone() } else { opc::relative(&npart, &opc::resolve(&np, &nr.target)) };
                        push_rel(&mut nrels, &nr.id, &nr.rtype, &tgt, nr.external);
                    }
                    nrels.save(&mut self.pkg);
                    self.pkg.set_content_type(&npart, "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml", false);
                    push_rel(&mut rels, &r.id, &r.rtype, &opc::relative(&part, &npart), false);
                    notes = Some((npart, Document::parse(&nd.to_string())?));
                }
                ("chart", Some(cp)) => {
                    let ncp = self.copy_part_tree(&cp, "chart{}.xml")?;
                    push_rel(&mut rels, &r.id, &r.rtype, &opc::relative(&part, &ncp), false);
                }
                (_, Some(tp)) => push_rel(&mut rels, &r.id, &r.rtype, &opc::relative(&part, &tp), false),
                (_, None) => push_rel(&mut rels, &r.id, &r.rtype, &r.target, true),
            }
        }
        self.insert_slide(part, doc, rels, layout, notes, si + 1)?;
        Ok(si + 1)
    }

    /// Copies a part and the parts it relates to that are its own (embedded workbooks, chart
    /// styles); returns the new part's name.
    fn copy_part_tree(&mut self, part: &str, pattern: &str) -> Result<String> {
        let folder = opc::folder(part).to_string();
        let np = self.pkg.free_name(&format!("{folder}/{pattern}"));
        let data = self.pkg.zip.read(part)?;
        self.pkg.zip.write(&np, data);
        if let Some(ct) = self.pkg.content_type(part) {
            self.pkg.set_content_type(&np, &ct, false);
        }
        let src_rels = self.pkg.rels(part)?;
        let mut rels = self.pkg.rels(&np)?;
        for r in &src_rels.list {
            if r.external {
                push_rel(&mut rels, &r.id, &r.rtype, &r.target, true);
                continue;
            }
            let tp = opc::resolve(part, &r.target);
            let ext = tp.rsplit('.').next().unwrap_or("bin").to_string();
            let base = tp.rsplit('/').next().unwrap_or("x").trim_end_matches(&format!(".{ext}")).trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
            let ntp = self.pkg.free_name(&format!("{}/{base}{{}}.{ext}", opc::folder(&tp)));
            let d = self.pkg.zip.read(&tp)?;
            self.pkg.zip.write(&ntp, d);
            if let Some(ct) = self.pkg.content_type(&tp) {
                self.pkg.set_content_type(&ntp, &ct, self.pkg.content_type(&tp).is_some() && !self.has_override(&tp));
            }
            push_rel(&mut rels, &r.id, &r.rtype, &opc::relative(&np, &ntp), false);
        }
        rels.save(&mut self.pkg);
        Ok(np)
    }

    fn has_override(&self, part: &str) -> bool {
        let ext = part.rsplit('.').next().unwrap_or("");
        // Media (images, workbooks) go by extension defaults.
        !matches!(ext.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "emf" | "wmf" | "xlsx" | "bin" | "svg" | "tif" | "tiff")
    }

    pub fn delete_slide(&mut self, si: usize) -> Result<()> {
        let s = self.slides.remove(si);
        self.pres_rels.remove(&s.rid);
        if let Some(lst) = self.pres.root.child_mut(P, "sldIdLst") {
            lst.children.retain(|n| n.as_element().map(|e| e.attr_ns(R, "id") != Some(s.rid.as_str())).unwrap_or(true));
        }
        self.pkg.remove_part(&s.part);
        self.removed.push(s.part.clone());
        if let Some((np, _)) = &s.notes {
            self.pkg.remove_part(np);
            self.removed.push(np.clone());
        }
        // Custom shows and sections name slides by id: drop the id from them.
        Ok(())
    }

    pub fn move_slide(&mut self, from: usize, to: usize) -> Result<()> {
        if to >= self.slides.len() {
            return Err(err(format!("there are {} slides", self.slides.len())));
        }
        let s = self.slides.remove(from);
        let rid = s.rid.clone();
        self.slides.insert(to, s);
        let lst = self.pres.root.child_mut(P, "sldIdLst").unwrap();
        let i = lst.children.iter().position(|n| n.as_element().map(|e| e.attr_ns(R, "id") == Some(rid.as_str())).unwrap_or(false)).unwrap();
        let node = lst.children.remove(i);
        let mut k = 0;
        let mut at_i = lst.children.len();
        for (j, n) in lst.children.iter().enumerate() {
            if n.as_element().map(|e| e.is(P, "sldId")).unwrap_or(false) {
                if k == to {
                    at_i = j;
                    break;
                }
                k += 1;
            }
        }
        lst.children.insert(at_i, node);
        Ok(())
    }

    pub fn set_hidden(&mut self, si: usize, hidden: bool) {
        let root = &mut self.slides[si].doc.root;
        if hidden {
            root.set_attr("show", NONE, "0");
        } else {
            root.remove_attr(NONE, "show");
        }
    }

    /// A text box on a slide.
    pub fn add_textbox(&mut self, si: usize, text_in: &str, props: &Props) -> Result<String> {
        let (cx, cy) = self.slide_size();
        let x = props.x.map(dml::emu).unwrap_or(cx / 10);
        let y = props.y.map(dml::emu).unwrap_or(cy / 3);
        let w = props.width.map(dml::emu).unwrap_or(cx * 8 / 10);
        let h = props.height.map(dml::emu).unwrap_or(cy / 8);
        let doc = &mut self.slides[si].doc;
        let tree = Self::sp_tree_path(doc).ok_or_else(|| err("the slide has no shape tree"))?;
        let max_id = doc.root.descendants().iter().filter(|e| e.local() == "cNvPr").filter_map(|e| e.attr_ns(NONE, "id").and_then(|v| v.parse::<u32>().ok())).max().unwrap_or(1);
        let id = max_id + 1;
        let paras: String = text::lines_of(&text::unescape(text_in)).into_iter().map(|l| format!("<a:p><a:r><a:rPr lang=\"en-US\" dirty=\"0\"/><a:t>{}</a:t></a:r></a:p>", xml::escape(l))).collect();
        let name = props.name.clone().unwrap_or_else(|| format!("TextBox {}", id - 1));
        let x_ = format!("<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap=\"square\" rtlCol=\"0\"><a:spAutoFit/></a:bodyPr><a:lstStyle/>{paras}</p:txBody></p:sp>", xml::escape(&name));
        let el = fragment_p(&doc.root, &x_)?;
        let tree_el = at_mut(&mut doc.root, &tree);
        let at_i = tree_el.position(P, "extLst").unwrap_or(tree_el.children.len());
        tree_el.children.insert(at_i, Node::Element(el));
        let shapes = Self::shapes(&self.slides[si].doc);
        let sh = shapes.iter().find(|s| s.name == name).map(|s| s.id.clone()).unwrap_or_default();
        let addr = format!("s{}.{sh}", si + 1);
        let mut rest = props.clone();
        rest.x = None;
        rest.y = None;
        rest.width = None;
        rest.height = None;
        rest.name = None;
        rest.keys.retain(|k| !matches!(k.as_str(), "x" | "y" | "w" | "h" | "name"));
        if !rest.keys.is_empty() {
            self.style(&addr, &rest, None)?;
        }
        Ok(addr)
    }

    pub fn slide_size(&self) -> (i64, i64) {
        let s = self.pres.root.child(P, "sldSz");
        let cx = s.and_then(|e| e.attr_ns(NONE, "cx")).and_then(|v| v.parse().ok()).unwrap_or(12192000);
        let cy = s.and_then(|e| e.attr_ns(NONE, "cy")).and_then(|v| v.parse().ok()).unwrap_or(6858000);
        (cx, cy)
    }

    pub fn save(&mut self) -> Result<Vec<u8>> {
        for s in &mut self.slides {
            self.pkg.put_xml(&s.part, &s.doc);
            s.rels.save(&mut self.pkg);
            if let Some((np, nd)) = &s.notes {
                self.pkg.put_xml(np, nd);
            }
        }
        self.pkg.put_xml(&self.pres_part, &self.pres);
        self.pres_rels.save(&mut self.pkg);
        self.pkg.save()
    }
}

fn push_rel(rels: &mut Rels, id: &str, rtype: &str, target: &str, external: bool) {
    // Rels::add picks its own id; keep the source's so the XML's r:id stay valid.
    let new = rels.add(rtype, target, external);
    if new != id {
        rels.rename_id(&new, id);
    }
}

fn collect_shapes(tree: &Element, base: &[usize], prefix: &str, out: &mut Vec<ShapeLoc>) {
    let mut n = 0;
    for (i, node) in tree.children.iter().enumerate() {
        let Some(e) = node.as_element() else { continue };
        let mut path = base.to_vec();
        path.push(i);
        let (e, path) = if e.ns() == MC && e.local() == "AlternateContent" {
            match e.child(MC, "Choice").and_then(|c| c.elements().next().map(|x| (c, x))) {
                Some((c, x)) => {
                    let ci = e.position(MC, "Choice").unwrap();
                    let xi = c.children.iter().position(|n| n.as_element().is_some()).unwrap();
                    let mut p = path.clone();
                    p.extend([ci, xi]);
                    (x, p)
                }
                None => continue,
            }
        } else {
            (e, path)
        };
        let kind: &'static str = match (e.ns(), e.local()) {
            (P, "sp") => {
                if e.path(P, &["nvSpPr", "cNvSpPr"]).and_then(|c| c.attr_ns(NONE, "txBox")) == Some("1") {
                    "textbox"
                } else {
                    "shape"
                }
            }
            (P, "pic") => "picture",
            (P, "grpSp") => "group",
            (P, "cxnSp") => "connector",
            (P, "graphicFrame") => {
                let uri = e.descendants().iter().find(|x| x.is(A, "graphicData")).and_then(|g| g.attr_ns(NONE, "uri")).unwrap_or("").to_string();
                if uri.ends_with("/table") {
                    "table"
                } else if uri.ends_with("/chart") || uri.contains("chart") {
                    "chart"
                } else if uri.ends_with("/diagram") {
                    "diagram"
                } else {
                    "object"
                }
            }
            (P, "contentPart") => "ink",
            _ => continue,
        };
        n += 1;
        let id = if prefix.is_empty() { format!("sh{n}") } else { format!("{prefix}.sh{n}") };
        let nv = e.elements().find(|c| c.local().starts_with("nv"));
        let name = nv.and_then(|v| v.child(P, "cNvPr")).and_then(|c| c.attr_ns(NONE, "name")).unwrap_or("").to_string();
        let ph = nv.and_then(|v| v.child(P, "nvPr")).and_then(|v| v.child(P, "ph")).map(|ph| (ph.attr_ns(NONE, "type").unwrap_or("body").to_string(), ph.attr_ns(NONE, "idx").unwrap_or("").to_string()));
        out.push(ShapeLoc { path: path.clone(), id: id.clone(), kind, name, placeholder: ph });
        if kind == "group" {
            collect_shapes(e, &path, &id, out);
        }
    }
}

fn xfrm_of(el: &Element) -> Option<(i64, i64, i64, i64)> {
    let x = el.child(P, "spPr").and_then(|s| s.child(A, "xfrm")).or_else(|| el.child(P, "xfrm")).or_else(|| el.child(P, "grpSpPr").and_then(|s| s.child(A, "xfrm")))?;
    let off = x.child(A, "off")?;
    let ext = x.child(A, "ext")?;
    let g = |e: &Element, a: &str| e.attr_ns(NONE, a).and_then(|v| v.parse::<i64>().ok());
    Some((g(off, "x")?, g(off, "y")?, g(ext, "cx")?, g(ext, "cy")?))
}

fn placeholder_xfrm(doc: &Document, t: &str, idx: &str) -> Option<(i64, i64, i64, i64)> {
    let shapes = Pptx::shapes(doc);
    let same_type = |a: &str, b: &str| a == b || (a == "ctrTitle" && b == "title") || (a == "title" && b == "ctrTitle") || (a == "subTitle" && b == "body") || (a == "obj" && b == "body") || (a == "body" && b == "obj");
    let by_idx = shapes.iter().find(|s| !idx.is_empty() && s.placeholder.as_ref().map(|p| p.1 == idx).unwrap_or(false));
    let by_type = shapes.iter().find(|s| s.placeholder.as_ref().map(|p| same_type(&p.0, t)).unwrap_or(false));
    for s in by_idx.into_iter().chain(by_type) {
        if let Some(g) = xfrm_of(at(&doc.root, &s.path)) {
            return Some(g);
        }
    }
    None
}

fn table_path(frame: &Element) -> Option<Vec<usize>> {
    let g = frame.position(A, "graphic")?;
    let ge = frame.children[g].as_element()?;
    let d = ge.position(A, "graphicData")?;
    let de = ge.children[d].as_element()?;
    let t = de.position(A, "tbl")?;
    Some(vec![g, d, t])
}

fn paragraphs_in(body: &Element, base: &[usize]) -> Vec<Vec<usize>> {
    body.children
        .iter()
        .enumerate()
        .filter(|(_, n)| n.as_element().map(|e| e.is(A, "p")).unwrap_or(false))
        .map(|(i, _)| {
            let mut p = base.to_vec();
            p.push(i);
            p
        })
        .collect()
}

fn para_range(tok: &str, n: usize, addr: &str) -> Result<(usize, usize)> {
    let t = tok.strip_prefix('p').ok_or_else(|| err(format!("expected pN in {addr:?}")))?;
    let (a, b) = match t.split_once('-') {
        Some((a, b)) => (a.parse().ok(), b.trim_start_matches('p').parse().ok()),
        None => (t.parse().ok(), t.parse().ok()),
    };
    let (Some(a), Some(b)) = (a, b) else { return Err(err(format!("bad paragraph number in {addr:?}"))) };
    if a == 0 || b < a || b > n {
        return Err(err(format!("{addr}: the text has {n} paragraphs")));
    }
    Ok((a, b))
}

/// The paragraphs of a text body become one per line of `text`, each with the look of the
/// paragraph that was at its place (the last one's for more).
pub fn set_body_text(body: &mut Element, text_in: &str) {
    let old: Vec<Element> = body.elements().filter(|e| e.is(A, "p")).cloned().collect();
    let template_default = Element::new(&body.qualify("p"), A);
    let at_i = body.children.iter().position(|n| n.as_element().map(|e| e.is(A, "p")).unwrap_or(false)).unwrap_or(body.children.len());
    body.children.retain(|n| !n.as_element().map(|e| e.is(A, "p")).unwrap_or(false));
    let lines: Vec<&str> = text::lines_of(text_in);
    for (k, line) in lines.iter().enumerate() {
        let t = old.get(k).or(old.last()).unwrap_or(&template_default);
        body.children.insert(at_i + k, Node::Element(paragraph_like(t, line)));
    }
}

/// A paragraph with the properties of `template` (its pPr, first run's rPr, end mark).
fn paragraph_like(template: &Element, line: &str) -> Element {
    let mut p = template.sibling("p");
    if let Some(ppr) = template.child(A, "pPr") {
        p.push(ppr.clone());
    }
    if !line.is_empty() {
        let mut r = Element::new(&template.qualify("r"), A);
        let rpr = template.elements().find(|e| e.is(A, "r")).and_then(|r| r.child(A, "rPr")).cloned().or_else(|| {
            template.child(A, "endParaRPr").map(|e| {
                let mut n = Element::new(&template.qualify("rPr"), A);
                for a in e.attrs() {
                    n.set_attr(&a.name, a.ns, &a.value);
                }
                n.children = e.children.clone();
                n
            })
        });
        if let Some(mut rpr) = rpr {
            rpr.remove_attr(NONE, "err");
            r.push(rpr);
        }
        let t = Dml.new_text(&r, line);
        r.push(t);
        p.push(r);
        let ri = p.children.len() - 1;
        let ti = p.children[ri].as_element().unwrap().children.len() - 1;
        Dml.settle(&mut p, ri, ti);
    }
    if let Some(e) = template.child(A, "endParaRPr") {
        p.push(e.clone());
    }
    p
}

fn ensure_ppr(p: &mut Element) -> &mut Element {
    if p.position(A, "pPr").is_none() {
        let e = Element::new(&p.qualify("pPr"), A);
        p.children.insert(0, Node::Element(e));
    }
    let i = p.position(A, "pPr").unwrap();
    p.children[i].as_element_mut().unwrap()
}

fn ensure_rpr(run: &mut Element) -> &mut Element {
    if run.position(A, "rPr").is_none() {
        let mut e = Element::new(&run.qualify("rPr"), A);
        e.set_attr("lang", NONE, "en-US");
        e.set_attr("dirty", NONE, "0");
        run.children.insert(0, Node::Element(e));
    }
    let i = run.position(A, "rPr").unwrap();
    run.children[i].as_element_mut().unwrap()
}

fn apply_shape(el: &mut Element, p: &Props, geom: Option<(i64, i64, i64, i64)>) -> Result<()> {
    let (props_name, is_frame) = if el.is(P, "graphicFrame") { ("xfrm", true) } else if el.is(P, "grpSp") { ("grpSpPr", false) } else { ("spPr", false) };
    if p.name.is_some() || p.alt.is_some() {
        if let Some(nv) = el.elements_mut().find(|c| c.local().starts_with("nv")) {
            if let Some(c) = nv.child_mut(P, "cNvPr") {
                if let Some(n) = &p.name {
                    c.set_attr("name", NONE, n);
                }
                if let Some(a) = &p.alt {
                    c.set_attr("descr", NONE, a);
                }
            }
        }
    }
    if p.x.is_some() || p.y.is_some() || p.width.is_some() || p.height.is_some() || p.rotation.is_some() {
        let (gx, gy, gw, gh) = geom.unwrap_or((0, 0, 914400, 914400));
        if is_frame {
            if el.child(P, "xfrm").is_none() {
                let x = fragment_p(el, &format!("<p:xfrm><a:off x=\"{gx}\" y=\"{gy}\"/><a:ext cx=\"{gw}\" cy=\"{gh}\"/></p:xfrm>"))?;
                let at_i = el.position(A, "graphic").unwrap_or(el.children.len());
                el.children.insert(at_i, Node::Element(x));
            }
        } else {
            if el.child(P, props_name).is_none() {
                let e = Element::new(&el.qualify(props_name), P);
                let at_i = el.children.iter().position(|n| n.as_element().map(|e| !e.local().starts_with("nv")).unwrap_or(false)).unwrap_or(el.children.len());
                el.children.insert(at_i, Node::Element(e));
            }
            let sp = el.child_mut(P, props_name).unwrap();
            if sp.child(A, "xfrm").is_none() {
                let x = fragment_a(sp, &format!("<a:xfrm><a:off x=\"{gx}\" y=\"{gy}\"/><a:ext cx=\"{gw}\" cy=\"{gh}\"/></a:xfrm>"))?;
                put_in_order(sp, x, dml::SPPR_ORDER);
            }
        }
        let xf = if is_frame { el.child_mut(P, "xfrm").unwrap() } else { el.child_mut(P, props_name).unwrap().child_mut(A, "xfrm").unwrap() };
        if let Some(r) = p.rotation {
            xf.set_attr("rot", NONE, &((r * 60000.0).round() as i64).to_string());
        }
        if let Some(off) = xf.child_mut(A, "off") {
            if let Some(x) = p.x {
                off.set_attr("x", NONE, &dml::emu(x).to_string());
            }
            if let Some(y) = p.y {
                off.set_attr("y", NONE, &dml::emu(y).to_string());
            }
        }
        if let Some(ext) = xf.child_mut(A, "ext") {
            if let Some(w) = p.width {
                ext.set_attr("cx", NONE, &dml::emu(w).to_string());
            }
            if let Some(h) = p.height {
                ext.set_attr("cy", NONE, &dml::emu(h).to_string());
            }
        }
    }
    if p.fill.is_some() || p.line_color.is_some() || p.line_width.is_some() {
        if is_frame {
            return Err(err("fill and line apply to shapes, not to tables or charts (style the cells: s1.sh3.r1.c1 fill=...)"));
        }
        if el.child(P, props_name).is_none() {
            let e = Element::new(&el.qualify(props_name), P);
            let at_i = el.children.iter().position(|n| n.as_element().map(|e| !e.local().starts_with("nv")).unwrap_or(false)).unwrap_or(el.children.len());
            el.children.insert(at_i, Node::Element(e));
        }
        let sp = el.child_mut(P, props_name).unwrap();
        if let Some(f) = &p.fill {
            for x in ["noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill"] {
                sp.remove_children(A, x);
            }
            match f {
                Color::Rgb(h) => {
                    let e = dml::solid_fill(sp, h);
                    put_in_order(sp, e, dml::SPPR_ORDER);
                }
                _ => {
                    let e = Element::new(&sp.qualify("noFill"), A);
                    put_in_order(sp, e, dml::SPPR_ORDER);
                }
            }
        }
        if p.line_color.is_some() || p.line_width.is_some() {
            let mut ln = sp.child(A, "ln").cloned().unwrap_or_else(|| Element::new(&sp.qualify("ln"), A));
            if let Some(w) = p.line_width {
                ln.set_attr("w", NONE, &dml::emu(w).to_string());
            }
            if let Some(c) = &p.line_color {
                for x in ["noFill", "solidFill", "gradFill", "pattFill"] {
                    ln.remove_children(A, x);
                }
                let e = match c {
                    Color::Rgb(h) => dml::solid_fill(&ln, h),
                    _ => Element::new(&ln.qualify("noFill"), A),
                };
                ln.children.insert(0, Node::Element(e));
            }
            put_in_order(sp, ln, dml::SPPR_ORDER);
        }
    }
    if p.valign.is_some() || p.autofit.is_some() || p.wrap.is_some() {
        let Some(tb) = el.child_mut(P, "txBody") else { return Err(err("valign, autofit and wrap need a shape with text")) };
        if tb.child(A, "bodyPr").is_none() {
            let e = Element::new(&tb.qualify("bodyPr"), A);
            tb.children.insert(0, Node::Element(e));
        }
        let bp = tb.child_mut(A, "bodyPr").unwrap();
        if let Some(v) = &p.valign {
            bp.set_attr("anchor", NONE, match v.as_str() {
                "middle" => "ctr",
                "bottom" => "b",
                _ => "t",
            });
        }
        if let Some(w) = p.wrap {
            bp.set_attr("wrap", NONE, if w { "square" } else { "none" });
        }
        if let Some(a) = &p.autofit {
            for x in ["noAutofit", "normAutofit", "spAutoFit"] {
                bp.remove_children(A, x);
            }
            let name = match a.as_str() {
                "shrink" | "normal" | "norm" => "normAutofit",
                "resize" | "shape" | "grow" => "spAutoFit",
                _ => "noAutofit",
            };
            let e = Element::new(&bp.qualify(name), A);
            put_in_order(bp, e, &["prstTxWarp", "noAutofit", "normAutofit", "spAutoFit", "scene3d", "sp3d", "flatTx", "extLst"]);
        }
    }
    Ok(())
}

fn apply_cell(tc: &mut Element, p: &Props) -> Result<()> {
    if tc.child(A, "tcPr").is_none() {
        let e = Element::new(&tc.qualify("tcPr"), A);
        put_in_order(tc, e, &["txBody", "tcPr", "extLst"]);
    }
    let pr = tc.child_mut(A, "tcPr").unwrap();
    const TCPR: &[&str] = &["lnL", "lnR", "lnT", "lnB", "lnTlToBr", "lnBlToTr", "cell3D", "noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill", "headers", "extLst"];
    if let Some(f) = &p.fill {
        for x in ["noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill"] {
            pr.remove_children(A, x);
        }
        let e = match f {
            Color::Rgb(h) => dml::solid_fill(pr, h),
            _ => Element::new(&pr.qualify("noFill"), A),
        };
        put_in_order(pr, e, TCPR);
    }
    if let Some(v) = &p.valign {
        pr.set_attr("anchor", NONE, match v.as_str() {
            "middle" => "ctr",
            "bottom" => "b",
            _ => "t",
        });
    }
    for (edges, b) in &p.borders {
        use crate::props::Edges;
        let names: &[&str] = match edges {
            Edges::All | Edges::Outline => &["lnL", "lnR", "lnT", "lnB"],
            Edges::Top => &["lnT"],
            Edges::Bottom => &["lnB"],
            Edges::Left => &["lnL"],
            Edges::Right => &["lnR"],
            _ => &[],
        };
        for n in names {
            let mut ln = Element::new(&pr.qualify(n), A);
            if b.style == "none" {
                ln.push(Element::new(&pr.qualify("noFill"), A));
            } else {
                ln.set_attr("w", NONE, &dml::emu(b.width).to_string());
                ln.push(dml::solid_fill(pr, b.color.as_ref().and_then(|c| c.hex()).unwrap_or("000000")));
                if b.style == "dashed" || b.style == "dotted" {
                    let mut d = Element::new(&pr.qualify("prstDash"), A);
                    d.set_attr("val", NONE, if b.style == "dashed" { "dash" } else { "sysDot" });
                    ln.push(d);
                }
            }
            put_in_order(pr, ln, TCPR);
        }
    }
    Ok(())
}

fn fragment_a(like: &Element, x: &str) -> Result<Element> {
    let pa = if like.ns() == A { like.prefix().to_string() } else { "a".to_string() };
    let x = if pa != "a" { x.replace("<a:", &format!("<{pa}:")).replace("</a:", &format!("</{pa}:")) } else { x.to_string() };
    Ok(xml::fragment(&x, &[(&pa, A)])?.remove(0))
}

fn fragment_p(like: &Element, x: &str) -> Result<Element> {
    let pp = if like.ns() == P { like.prefix().to_string() } else { "p".to_string() };
    let x = if pp != "p" { x.replace("<p:", &format!("<{pp}:")).replace("</p:", &format!("</{pp}:")) } else { x.to_string() };
    Ok(xml::fragment(&x, &[(&pp, P), ("a", A), ("r", R)])?.remove(0))
}

pub fn runs_info(p: &Element) -> Vec<RunInfo> {
    let segs = text::segments(&Dml, p);
    let mut out: Vec<RunInfo> = Vec::new();
    let mut last: Option<Vec<usize>> = None;
    for s in segs {
        if last.as_ref() == Some(&s.run) {
            out.last_mut().unwrap().text.push_str(&s.text);
            continue;
        }
        last = Some(s.run.clone());
        let run = at(p, &s.run);
        let mut props = dml::rpr_summary(run.child(A, "rPr"));
        if run.is(A, "fld") {
            props.insert(0, format!("field={}", run.attr_ns(NONE, "type").unwrap_or("")));
        }
        out.push(RunInfo { text: s.text.clone(), props, start: s.start });
    }
    out
}
