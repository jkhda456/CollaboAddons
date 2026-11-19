//! Inserting and deleting rows and columns, and adding, renaming and deleting sheets, with every
//! reference that points at them rewritten: formulas on every sheet, defined names, merged cells,
//! conditional formats, data validations, hyperlinks, filters, tables, drawing anchors and
//! chart series.

use crate::formula::{self, Area, CellRef, MAX_COL, MAX_ROW};
use crate::opc;
use crate::xlsx::{Raw, Sheet, Xlsx, WORKSHEET_ORDER};
use crate::xml::{self, put_in_order, Document, Element, Node, R, S, XDR};
use crate::{err, Result};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Axis {
    Row,
    Col,
}

/// Rows or columns inserted (delta > 0) or deleted (delta < 0) at `at` (0-based) on a sheet.
#[derive(Clone, Copy, Debug)]
pub struct Shift {
    pub sheet: usize,
    pub axis: Axis,
    pub at: u32,
    pub delta: i64,
}

impl Shift {
    fn limit(&self) -> u32 {
        if self.axis == Axis::Row {
            MAX_ROW
        } else {
            MAX_COL
        }
    }

    /// A coordinate after the shift (None: deleted).
    fn point(&self, x: u32) -> Option<u32> {
        if self.delta > 0 {
            if x >= self.at {
                Some((x as i64 + self.delta).min(self.limit() as i64) as u32)
            } else {
                Some(x)
            }
        } else {
            let n = (-self.delta) as u32;
            if x < self.at {
                Some(x)
            } else if x < self.at + n {
                None
            } else {
                Some(x - n)
            }
        }
    }

    /// A span [x0, x1] after the shift (None: all of it deleted).
    fn span(&self, x0: u32, x1: u32) -> Option<(u32, u32)> {
        if self.delta > 0 {
            let a = if x0 >= self.at { x0 as i64 + self.delta } else { x0 as i64 };
            let b = if x1 >= self.at { x1 as i64 + self.delta } else { x1 as i64 };
            return Some((a.min(self.limit() as i64) as u32, b.min(self.limit() as i64) as u32));
        }
        let n = (-self.delta) as u32;
        let end = self.at + n;
        let a = if x0 < self.at { x0 } else if x0 < end { self.at } else { x0 - n };
        let b = if x1 < self.at {
            x1
        } else if x1 < end {
            if self.at == 0 {
                return None;
            }
            self.at - 1
        } else {
            x1 - n
        };
        if b < a || (x0 >= self.at && x1 < end) {
            None
        } else {
            Some((a, b))
        }
    }

    /// An area on the shifted sheet after the shift: None when it does not change, Some(None)
    /// when it is gone (#REF!).
    pub fn area(&self, a: &Area) -> Option<Option<Area>> {
        let mut n = a.clone();
        let whole_other = match self.axis {
            Axis::Row => a.whole == Some('c'),
            Axis::Col => a.whole == Some('r'),
        };
        if whole_other {
            return None;
        }
        let (x0, x1) = if self.axis == Axis::Row { a.rows() } else { a.cols() };
        let single = a.single();
        let new = if single { self.point(x0).map(|x| (x, x)) } else { self.span(x0, x1) };
        let Some((y0, y1)) = new else { return Some(None) };
        if (y0, y1) == (x0, x1) {
            return None;
        }
        // Keep which endpoint was which.
        let (first_is_a, _) = if self.axis == Axis::Row { (a.a.row <= a.b.row, ()) } else { (a.a.col <= a.b.col, ()) };
        let (va, vb) = if first_is_a { (y0, y1) } else { (y1, y0) };
        if self.axis == Axis::Row {
            n.a.row = va;
            n.b.row = vb;
        } else {
            n.a.col = va;
            n.b.col = vb;
        }
        Some(Some(n))
    }
}

/// `sqref`-style lists: `A1:B3 D5`.
fn shift_sqref(s: &str, sh: &Shift) -> Option<String> {
    let mut out = Vec::new();
    let mut changed = false;
    for part in s.split_whitespace() {
        match formula::parse_area(part) {
            Some(a) => match sh.area(&a) {
                None => out.push(part.to_string()),
                Some(Some(n)) => {
                    out.push(n.text());
                    changed = true;
                }
                Some(None) => changed = true,
            },
            None => out.push(part.to_string()),
        }
    }
    if changed {
        Some(out.join(" "))
    } else {
        None
    }
}

/// A formula on sheet `on` after the shift.
fn shift_formula(f: &str, on: usize, sh: &Shift, x: &Xlsx) -> Option<String> {
    let mut changed = false;
    let new = formula::rewrite_refs(f, &mut |a| {
        let target = match &a.sheet {
            Some(s) => x.sheet_index(s),
            None => Some(on),
        };
        if target != Some(sh.sheet) {
            return None;
        }
        let r = sh.area(a);
        if r.is_some() {
            changed = true;
        }
        r
    })
    .ok()?;
    if changed {
        Some(new)
    } else {
        None
    }
}

impl Xlsx {
    /// `Sheet1!5`, `5:7`, `C`, `C:E` → (sheet, axis, first, last), 0-based.
    fn rows_or_cols(&self, addr: &str) -> Result<(usize, Axis, u32, u32)> {
        let (sheet, rest) = match addr.rfind('!') {
            Some(i) => (Some(addr[..i].trim_matches('\'').replace("''", "'")), &addr[i + 1..]),
            None => (None, addr),
        };
        let si = match &sheet {
            Some(s) => self.sheet_index(s).ok_or_else(|| err(format!("no sheet {s:?}")))?,
            None => self.sheet_index(&self.sheet_names()[0]).unwrap_or(0).max(self.active()),
        };
        let rest = rest.replace('$', "");
        let (a, b) = rest.split_once(':').unwrap_or((&rest, &rest));
        if let (Ok(x), Ok(y)) = (a.parse::<u32>(), b.parse::<u32>()) {
            if x == 0 || y < x {
                return Err(err(format!("bad rows {addr:?}")));
            }
            return Ok((si, Axis::Row, x - 1, y - 1));
        }
        if let (Some(x), Some(y)) = (formula::col_index(a), formula::col_index(b)) {
            if y < x {
                return Err(err(format!("bad columns {addr:?}")));
            }
            return Ok((si, Axis::Col, x, y));
        }
        Err(err(format!("{addr:?} names neither rows (Sheet1!5, 5:7) nor columns (C, C:E)")))
    }

    fn active(&self) -> usize {
        self.wb.root.child(S, "bookViews").and_then(|b| b.child(S, "workbookView")).and_then(|v| v.own("activeTab")).and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    /// Inserts rows or columns (`count` of them, or as many as the address names) before (or
    /// after) the address; `text` fills the first new row or column (`a|b|c`).
    pub fn insert(&mut self, addr: &str, after: bool, text: &str, count: usize) -> Result<Vec<String>> {
        let (si, axis, a, b) = self.rows_or_cols(addr)?;
        let n = if count > 1 { count as u32 } else { b - a + 1 };
        let at = if after { b + 1 } else { a };
        self.shift(Shift { sheet: si, axis, at, delta: n as i64 })?;
        let name = formula::quote_sheet(&self.sheets[si].name);
        let what = match axis {
            Axis::Row => format!("{name}!{}:{}", at + 1, at + n),
            Axis::Col => format!("{name}!{}:{}", formula::col_name(at), formula::col_name(at + n - 1)),
        };
        if !text.is_empty() {
            let vals: Vec<&str> = text.split(['|', '\t']).collect();
            for (k, v) in vals.iter().enumerate() {
                let (r, c) = match axis {
                    Axis::Row => (at, k as u32),
                    Axis::Col => (k as u32, at),
                };
                // The new cells take the style of the row or column next to them.
                self.set_cell(si, r, c, &crate::text::unescape(v))?;
            }
        }
        // New cells copy the formats of the row/column before, as Excel does.
        if at > 0 {
            self.copy_formats(si, axis, at - 1, at, n);
        }
        Ok(vec![what])
    }

    fn copy_formats(&mut self, si: usize, axis: Axis, from: u32, at: u32, n: u32) {
        let cells = self.cells(si);
        let styled: Vec<(u32, u32)> = cells.iter().filter(|c| c.style != 0 && if axis == Axis::Row { c.row == from } else { c.col == from }).map(|c| (c.row, c.col)).collect();
        for (r, c) in styled {
            let s = self.find_cell(si, r, c).and_then(|e| e.own("s")).map(str::to_string);
            let Some(s) = s else { continue };
            for k in 0..n {
                let (nr, nc) = if axis == Axis::Row { (at + k, c) } else { (r, at + k) };
                let cell = self.cell_mut(si, nr, nc);
                if cell.own("s").is_none() {
                    cell.set_plain("s", &s);
                }
            }
        }
    }

    pub fn delete(&mut self, addr: &str) -> Result<String> {
        if let Ok((si, axis, a, b)) = self.rows_or_cols(addr) {
            let n = b - a + 1;
            self.shift(Shift { sheet: si, axis, at: a, delta: -(n as i64) })?;
            let name = formula::quote_sheet(&self.sheets[si].name);
            return Ok(match axis {
                Axis::Row => format!("deleted rows {name}!{}:{}", a + 1, b + 1),
                Axis::Col => format!("deleted columns {name}!{}:{}", formula::col_name(a), formula::col_name(b)),
            });
        }
        if self.sheet_index(addr.trim_matches('\'')).is_some() {
            return Err(err(format!("to delete a sheet: docu sheet FILE delete {addr}")));
        }
        // Cells: their contents (formats stay).
        let (si, area) = self.resolve(addr)?;
        let (r0, r1) = area.rows();
        let (c0, c1) = area.cols();
        let cells: Vec<(u32, u32)> = self.cells(si).iter().filter(|c| c.row >= r0 && c.row <= r1 && c.col >= c0 && c.col <= c1).map(|c| (c.row, c.col)).collect();
        for (r, c) in &cells {
            self.set_cell(si, *r, *c, "")?;
        }
        Ok(format!("cleared {} cell{}", cells.len(), if cells.len() == 1 { "" } else { "s" }))
    }

    /// Moves cells and rewrites every reference for a row or column insertion or deletion.
    pub fn shift(&mut self, sh: Shift) -> Result<()> {
        // Shared formulas on the sheet become explicit (their offsets would change).
        self.unshare_all(sh.sheet);
        // Formula texts on every sheet first (they need the sheet names, which stay).
        let names: Vec<String> = self.sheet_names();
        let mut new_formulas: Vec<(usize, u32, u32, String)> = Vec::new();
        for si in 0..self.sheets.len() {
            for c in self.cells(si) {
                if let Raw::Formula(f, _) = &c.raw {
                    if let Some(nf) = shift_formula(f, si, &sh, self) {
                        new_formulas.push((si, c.row, c.col, nf));
                    }
                }
            }
        }
        for (si, r, c, f) in new_formulas {
            let cell = self.cell_mut(si, r, c);
            if let Some(fe) = cell.child_mut(S, "f") {
                fe.set_text(&f);
            }
        }
        // The cells of the sheet.
        let doc = self.sheets[sh.sheet].doc.as_mut().ok_or_else(|| err("not a worksheet"))?;
        shift_cells(&mut doc.root, &sh);
        // Everything else on the sheet that names cells.
        shift_sheet_parts(&mut doc.root, &sh);
        // Formulas inside conditional formats and validations, on this sheet.
        let snapshot_names = names.clone();
        let _ = snapshot_names;
        let sheet = sh.sheet;
        let mut fixes: Vec<(usize, String, String)> = Vec::new();
        {
            let x: &Xlsx = self;
            if let Some(d) = &x.sheets[sheet].doc {
                for e in d.root.descendants() {
                    if (e.local() == "formula" || e.local() == "formula1" || e.local() == "formula2") && e.ns() == S {
                        let t = e.text();
                        if let Some(n) = shift_formula(&t, sheet, &sh, x) {
                            fixes.push((sheet, t, n));
                        }
                    }
                }
            }
        }
        if let Some(d) = self.sheets[sheet].doc.as_mut() {
            d.root.walk_mut(&mut |e| {
                if (e.local() == "formula" || e.local() == "formula1" || e.local() == "formula2") && e.ns() == S {
                    let t = e.text();
                    if let Some((_, _, n)) = fixes.iter().find(|(_, o, _)| *o == t) {
                        e.set_text(n);
                    }
                }
            });
        }
        // Defined names (print areas and others).
        let mut dn_fix: Vec<(usize, String)> = Vec::new();
        if let Some(dn) = self.wb.root.child(S, "definedNames") {
            for (i, n) in dn.children.iter().enumerate() {
                if let Some(e) = n.as_element() {
                    let on = e.own("localSheetId").and_then(|v| v.parse().ok()).unwrap_or(sheet);
                    if let Some(nf) = shift_formula(&e.text(), on, &sh, self) {
                        dn_fix.push((i, nf));
                    }
                }
            }
        }
        if let Some(dn) = self.wb.root.child_mut(S, "definedNames") {
            for (i, nf) in dn_fix {
                if let Some(e) = dn.children[i].as_element_mut() {
                    e.set_text(&nf);
                }
            }
        }
        // Tables, drawings and charts of the sheet.
        self.shift_related(&sh)?;
        Ok(())
    }

    fn unshare_all(&mut self, si: usize) {
        let cells = self.cells(si);
        let mut texts = Vec::new();
        for c in &cells {
            if let Raw::Formula(t, _) = &c.raw {
                if self.find_cell(si, c.row, c.col).and_then(|e| e.child(S, "f")).map(|f| f.own("t") == Some("shared")).unwrap_or(false) {
                    texts.push((c.row, c.col, t.clone()));
                }
            }
        }
        for (r, c, t) in texts {
            let cell = self.cell_mut(si, r, c);
            if let Some(fe) = cell.child_mut(S, "f") {
                let mut n = fe.sibling("f");
                n.set_text(&t);
                *fe = n;
            }
        }
    }

    fn shift_related(&mut self, sh: &Shift) -> Result<()> {
        let part = self.sheets[sh.sheet].part.clone();
        let rels = self.pkg.rels(&part)?;
        // Tables.
        for r in rels.by_kind("table") {
            let Some(tp) = rels.part(&r.id) else { continue };
            let Ok(mut d) = self.pkg.xml(&tp) else { continue };
            let mut changed = false;
            d.root.walk_mut(&mut |e| {
                if e.local() == "table" || e.local() == "autoFilter" || e.local() == "sortState" || e.local() == "sortCondition" {
                    if let Some(v) = e.own("ref").map(str::to_string) {
                        if let Some(n) = shift_sqref(&v, sh) {
                            e.set_plain("ref", &n);
                            changed = true;
                        }
                    }
                }
            });
            if changed {
                self.pkg.put_xml(&tp, &d);
            }
        }
        // Drawing anchors move with their cells.
        for r in rels.by_kind("drawing") {
            let Some(dp) = rels.part(&r.id) else { continue };
            let Ok(mut d) = self.pkg.xml(&dp) else { continue };
            let mut changed = false;
            for anchor in d.root.elements_mut() {
                for corner in ["from", "to"] {
                    if let Some(c) = anchor.child_mut(XDR, corner) {
                        let name = if sh.axis == Axis::Row { "row" } else { "col" };
                        if let Some(v) = c.child(XDR, name).map(|e| e.text()).and_then(|t| t.trim().parse::<u32>().ok()) {
                            let nv = match sh.point(v) {
                                Some(x) => x,
                                None => sh.at.min(v),
                            };
                            if nv != v {
                                c.child_mut(XDR, name).unwrap().set_text(&nv.to_string());
                                changed = true;
                            }
                        }
                    }
                }
            }
            if changed {
                self.pkg.put_xml(&dp, &d);
            }
        }
        // Charts anywhere name their sheets in their series.
        let charts: Vec<String> = self.pkg.zip.names().into_iter().filter(|n| n.contains("/charts/chart") && n.ends_with(".xml")).collect();
        for cp in charts {
            self.shift_chart(&cp, sh)?;
        }
        Ok(())
    }

    fn shift_chart(&mut self, cp: &str, sh: &Shift) -> Result<()> {
        let Ok(mut d) = self.pkg.xml(cp) else { return Ok(()) };
        let mut fixes = Vec::new();
        for e in d.root.descendants() {
            if e.is(xml::C, "f") {
                let t = e.text();
                // Chart formulas always name their sheet; resolve against sheet 0 otherwise.
                if let Some(n) = shift_formula(&t, usize::MAX, sh, self) {
                    fixes.push((t, n));
                }
            }
        }
        if fixes.is_empty() {
            return Ok(());
        }
        d.root.walk_mut(&mut |e| {
            if e.is(xml::C, "f") {
                let t = e.text();
                if let Some((_, n)) = fixes.iter().find(|(o, _)| *o == t) {
                    e.set_text(n);
                }
            }
        });
        self.pkg.put_xml(cp, &d);
        Ok(())
    }

    // ── sheets ──────────────────────────────────────────────────────────────────────────────

    pub fn add_sheet(&mut self, name: &str, after: Option<usize>) -> Result<usize> {
        check_sheet_name(name)?;
        if self.sheet_index(name).is_some() {
            return Err(err(format!("there is a sheet {name:?} already")));
        }
        let folder = format!("{}/worksheets", opc::folder(&self.wb_part));
        let part = self.pkg.free_name(&format!("{folder}/sheet{{}}.xml"));
        let doc = Document::parse("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><dimension ref=\"A1\"/><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews><sheetFormatPr defaultRowHeight=\"15\"/><sheetData/><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/></worksheet>").unwrap();
        self.pkg.set_content_type(&part, "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml", false);
        let rid = self.wb_rels.add_part("http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet", &part);
        let sheets_el = self.wb.root.child_mut(S, "sheets").ok_or_else(|| err("workbook has no sheets list"))?;
        let next_id = sheets_el.elements().filter_map(|e| e.own("sheetId").and_then(|v| v.parse::<u32>().ok())).max().unwrap_or(0) + 1;
        let mut e = sheets_el.sibling("sheet");
        e.set_plain("name", name);
        e.set_plain("sheetId", &next_id.to_string());
        let rp = self.wb.prefix_for(R);
        let sheets_el = self.wb.root.child_mut(S, "sheets").unwrap();
        e.set_attr(&format!("{rp}:id"), R, &rid);
        let pos = match after {
            Some(i) => i + 1,
            None => self.sheets.len(),
        };
        // The element's place among the sheet elements.
        let mut k = 0;
        let mut at = sheets_el.children.len();
        for (i, n) in sheets_el.children.iter().enumerate() {
            if n.as_element().map(|e| e.is(S, "sheet")).unwrap_or(false) {
                if k == pos {
                    at = i;
                    break;
                }
                k += 1;
            }
        }
        sheets_el.children.insert(at, Node::Element(e));
        self.sheets.insert(pos, Sheet { name: name.to_string(), part, doc: Some(doc), hidden: false, rid });
        // Sheet-local names refer to sheets by position.
        self.renumber_local_names(|i| if i >= pos { Some(i + 1) } else { Some(i) });
        Ok(pos)
    }

    pub fn rename_sheet(&mut self, old: &str, new: &str) -> Result<()> {
        check_sheet_name(new)?;
        let si = self.sheet_index(old).ok_or_else(|| err(format!("no sheet {old:?}")))?;
        if let Some(o) = self.sheet_index(new) {
            if o != si {
                return Err(err(format!("there is a sheet {new:?} already")));
            }
        }
        let old_name = self.sheets[si].name.clone();
        let rename = |f: &str| -> Option<String> {
            let mut changed = false;
            let r = formula::rewrite_refs(f, &mut |a| {
                if a.sheet.as_deref().map(|s| s.eq_ignore_ascii_case(&old_name)).unwrap_or(false) {
                    changed = true;
                    let mut n = a.clone();
                    n.sheet = Some(new.to_string());
                    Some(Some(n))
                } else {
                    None
                }
            })
            .ok()?;
            if changed {
                Some(r)
            } else {
                None
            }
        };
        for s in 0..self.sheets.len() {
            let cells = self.cells(s);
            for c in cells {
                if let Raw::Formula(f, _) = &c.raw {
                    if let Some(nf) = rename(f) {
                        let cell = self.cell_mut(s, c.row, c.col);
                        if let Some(fe) = cell.child_mut(S, "f") {
                            if !(fe.own("t") == Some("shared") && fe.text().is_empty()) {
                                fe.set_text(&nf);
                            }
                        }
                    }
                }
            }
        }
        if let Some(dn) = self.wb.root.child_mut(S, "definedNames") {
            for e in dn.elements_mut() {
                if let Some(nf) = rename(&e.text()) {
                    e.set_text(&nf);
                }
            }
        }
        // Charts anywhere name the sheet in their series.
        let charts: Vec<String> = self.pkg.zip.names().into_iter().filter(|n| n.contains("/charts/chart") && n.ends_with(".xml")).collect();
        for cp in charts {
            let Ok(mut d) = self.pkg.xml(&cp) else { continue };
            let mut changed = false;
            d.root.walk_mut(&mut |e| {
                if e.is(xml::C, "f") {
                    if let Some(nf) = rename(&e.text()) {
                        e.set_text(&nf);
                        changed = true;
                    }
                }
            });
            if changed {
                self.pkg.put_xml(&cp, &d);
            }
        }
        let rid = self.sheets[si].rid.clone();
        if let Some(sheets_el) = self.wb.root.child_mut(S, "sheets") {
            for e in sheets_el.elements_mut() {
                if e.attr_ns(R, "id") == Some(rid.as_str()) {
                    e.set_plain("name", new);
                }
            }
        }
        self.sheets[si].name = new.to_string();
        Ok(())
    }

    pub fn delete_sheet(&mut self, name: &str) -> Result<()> {
        let si = self.sheet_index(name).ok_or_else(|| err(format!("no sheet {name:?}")))?;
        let visible = self.sheets.iter().enumerate().filter(|(i, s)| *i != si && !s.hidden).count();
        if visible == 0 {
            return Err(err("a workbook keeps at least one visible sheet"));
        }
        let old_name = self.sheets[si].name.clone();
        // References to it become #REF!.
        let gone = |f: &str| -> Option<String> {
            let mut changed = false;
            let r = formula::rewrite_refs(f, &mut |a| {
                if a.sheet.as_deref().map(|s| s.eq_ignore_ascii_case(&old_name)).unwrap_or(false) {
                    changed = true;
                    Some(None)
                } else {
                    None
                }
            })
            .ok()?;
            if changed {
                Some(r)
            } else {
                None
            }
        };
        for s in 0..self.sheets.len() {
            if s == si {
                continue;
            }
            for c in self.cells(s) {
                if let Raw::Formula(f, _) = &c.raw {
                    if let Some(nf) = gone(f) {
                        let cell = self.cell_mut(s, c.row, c.col);
                        if let Some(fe) = cell.child_mut(S, "f") {
                            fe.set_text(&nf);
                        }
                        self.mark_changed(s, c.row, c.col);
                    }
                }
            }
        }
        let rid = self.sheets[si].rid.clone();
        let part = self.sheets[si].part.clone();
        if let Some(sheets_el) = self.wb.root.child_mut(S, "sheets") {
            sheets_el.children.retain(|n| n.as_element().map(|e| e.attr_ns(R, "id") != Some(rid.as_str())).unwrap_or(true));
        }
        self.wb_rels.remove(&rid);
        self.pkg.remove_part(&part);
        self.sheets.remove(si);
        // Names local to it go; later sheets move down one.
        self.renumber_local_names(|i| if i == si { None } else if i > si { Some(i - 1) } else { Some(i) });
        if let Some(bv) = self.wb.root.child_mut(S, "bookViews") {
            for v in bv.elements_mut() {
                for a in ["activeTab", "firstSheet"] {
                    if let Some(t) = v.own(a).and_then(|x| x.parse::<usize>().ok()) {
                        if t >= si && t > 0 {
                            v.set_plain(a, &(t - 1).to_string());
                        }
                    }
                }
            }
        }
        // Selected tabs on other sheets are fine; app.xml titles are informative only.
        Ok(())
    }

    pub fn set_sheet_hidden(&mut self, name: &str, hidden: bool) -> Result<()> {
        let si = self.sheet_index(name).ok_or_else(|| err(format!("no sheet {name:?}")))?;
        if hidden && self.sheets.iter().enumerate().filter(|(i, s)| *i != si && !s.hidden).count() == 0 {
            return Err(err("a workbook keeps at least one visible sheet"));
        }
        let rid = self.sheets[si].rid.clone();
        if let Some(sheets_el) = self.wb.root.child_mut(S, "sheets") {
            for e in sheets_el.elements_mut() {
                if e.attr_ns(R, "id") == Some(rid.as_str()) {
                    if hidden {
                        e.set_plain("state", "hidden");
                    } else {
                        e.remove_own("state");
                    }
                }
            }
        }
        self.sheets[si].hidden = hidden;
        Ok(())
    }

    fn renumber_local_names(&mut self, f: impl Fn(usize) -> Option<usize>) {
        if let Some(dn) = self.wb.root.child_mut(S, "definedNames") {
            dn.children.retain(|n| n.as_element().and_then(|e| e.own("localSheetId")).and_then(|v| v.parse::<usize>().ok()).map(|i| f(i).is_some()).unwrap_or(true));
            for e in dn.elements_mut() {
                if let Some(i) = e.own("localSheetId").and_then(|v| v.parse::<usize>().ok()) {
                    if let Some(j) = f(i) {
                        if i != j {
                            e.set_plain("localSheetId", &j.to_string());
                        }
                    }
                }
            }
            if dn.elements().count() == 0 {
                self.wb.root.remove_children(S, "definedNames");
            }
        }
    }
}

fn check_sheet_name(name: &str) -> Result<()> {
    if name.is_empty() || name.chars().count() > 31 || name.contains(['[', ']', ':', '*', '?', '/', '\\']) || name.starts_with('\'') || name.ends_with('\'') {
        return Err(err(format!("{name:?} is not a sheet name (1 to 31 characters, none of []:*?/\\, no ' at either end)")));
    }
    if name.eq_ignore_ascii_case("history") {
        return Err(err("History is reserved"));
    }
    Ok(())
}

/// Moves rows/cells in sheetData and the column definitions.
fn shift_cells(root: &mut Element, sh: &Shift) {
    if let Some(data) = root.child_mut(S, "sheetData") {
        let mut keep: Vec<Node> = Vec::new();
        for n in std::mem::take(&mut data.children) {
            let Node::Element(mut row) = n else {
                keep.push(n);
                continue;
            };
            if !row.is(S, "row") {
                keep.push(Node::Element(row));
                continue;
            }
            let r: u32 = row.own("r").and_then(|v| v.parse::<u32>().ok()).unwrap_or(1) - 1;
            let nr = if sh.axis == Axis::Row { sh.point(r) } else { Some(r) };
            let Some(nr) = nr else { continue };
            if nr != r {
                row.set_plain("r", &(nr + 1).to_string());
            }
            let mut cells: Vec<Node> = Vec::new();
            for cn in std::mem::take(&mut row.children) {
                let Node::Element(mut c) = cn else {
                    cells.push(cn);
                    continue;
                };
                if c.is(S, "c") {
                    if let Some(cr) = c.own("r").and_then(formula::parse_cell) {
                        let (mut rr, mut cc) = (cr.row, cr.col);
                        match sh.axis {
                            Axis::Row => rr = nr,
                            Axis::Col => match sh.point(cc) {
                                Some(x) => cc = x,
                                None => continue,
                            },
                        }
                        let name = formula::cell_name(rr, cc);
                        if c.own("r") != Some(name.as_str()) {
                            c.set_plain("r", &name);
                        }
                    }
                    // Shared and array formula ranges.
                    if let Some(f) = c.child_mut(S, "f") {
                        if let Some(v) = f.own("ref").map(str::to_string) {
                            match shift_sqref(&v, sh) {
                                Some(n) if !n.is_empty() => f.set_plain("ref", &n),
                                Some(_) => {
                                    f.remove_own("ref");
                                }
                                None => {}
                            }
                        }
                    }
                }
                cells.push(Node::Element(c));
            }
            row.children = cells;
            if sh.axis == Axis::Col {
                if let Some(sp) = row.own("spans").map(str::to_string) {
                    let _ = sp;
                    row.remove_own("spans");
                }
            }
            keep.push(Node::Element(row));
        }
        data.children = keep;
    }
    if sh.axis == Axis::Col {
        if let Some(cols) = root.child_mut(S, "cols") {
            let mut out = Vec::new();
            for n in std::mem::take(&mut cols.children) {
                let Node::Element(mut col) = n else {
                    out.push(n);
                    continue;
                };
                let min: u32 = col.own("min").and_then(|v| v.parse::<u32>().ok()).unwrap_or(1) - 1;
                let max: u32 = col.own("max").and_then(|v| v.parse::<u32>().ok()).unwrap_or(1) - 1;
                match sh.span(min, max) {
                    Some((a, b)) => {
                        if (a, b) != (min, max) {
                            col.set_plain("min", &(a + 1).to_string());
                            col.set_plain("max", &(b + 1).to_string());
                        }
                        out.push(Node::Element(col));
                    }
                    None => {}
                }
            }
            cols.children = out;
        }
    }
}

/// Merged cells, conditional formats, validations, hyperlinks, filters, the dimension, panes and
/// selections, page breaks.
fn shift_sheet_parts(root: &mut Element, sh: &Shift) {
    let fix_attr = |e: &mut Element, attr: &str| {
        if let Some(v) = e.own(attr).map(str::to_string) {
            if let Some(n) = shift_sqref(&v, sh) {
                if n.is_empty() {
                    return false;
                }
                e.set_plain(attr, &n);
            }
        }
        true
    };
    if let Some(mc) = root.child_mut(S, "mergeCells") {
        mc.children.retain_mut(|n| match n {
            Node::Element(e) => {
                let ok = fix_attr(e, "ref");
                // A merge down to one cell is no merge.
                ok && e.own("ref").map(|r| r.contains(':') && formula::parse_area(r).map(|a| !a.single()).unwrap_or(false)).unwrap_or(false)
            }
            _ => true,
        });
        let n = mc.elements().count();
        if n == 0 {
            root.remove_children(S, "mergeCells");
        } else {
            mc.set_plain("count", &n.to_string());
        }
    }
    root.children.retain_mut(|n| match n {
        Node::Element(e) if e.is(S, "conditionalFormatting") => fix_attr(e, "sqref"),
        _ => true,
    });
    if let Some(dv) = root.child_mut(S, "dataValidations") {
        dv.children.retain_mut(|n| match n {
            Node::Element(e) => fix_attr(e, "sqref"),
            _ => true,
        });
        let n = dv.elements().count();
        if n == 0 {
            root.remove_children(S, "dataValidations");
        } else {
            dv.set_plain("count", &n.to_string());
        }
    }
    if let Some(h) = root.child_mut(S, "hyperlinks") {
        h.children.retain_mut(|n| match n {
            Node::Element(e) => fix_attr(e, "ref"),
            _ => true,
        });
        if h.elements().count() == 0 {
            root.remove_children(S, "hyperlinks");
        }
    }
    if let Some(af) = root.child_mut(S, "autoFilter") {
        if !fix_attr(af, "ref") {
            root.remove_children(S, "autoFilter");
        }
    }
    if let Some(d) = root.child_mut(S, "dimension") {
        if !fix_attr(d, "ref") {
            d.set_plain("ref", "A1");
        }
    }
    if let Some(views) = root.child_mut(S, "sheetViews") {
        for v in views.elements_mut() {
            for e in v.elements_mut() {
                if e.local() == "selection" {
                    if !fix_attr(e, "sqref") {
                        e.set_plain("sqref", "A1");
                    }
                    if !fix_attr(e, "activeCell") {
                        e.set_plain("activeCell", "A1");
                    }
                }
            }
        }
    }
    let breaks = if sh.axis == Axis::Row { "rowBreaks" } else { "colBreaks" };
    if let Some(b) = root.child_mut(S, breaks) {
        b.children.retain_mut(|n| match n {
            Node::Element(e) => match e.own("id").and_then(|v| v.parse::<u32>().ok()) {
                Some(id) if id > 0 => match sh.point(id - 1) {
                    Some(x) => {
                        if x + 1 != id {
                            e.set_plain("id", &(x + 1).to_string());
                        }
                        true
                    }
                    None => false,
                },
                _ => true,
            },
            _ => true,
        });
    }
    let _ = (put_in_order, WORKSHEET_ORDER, CellRef { row: 0, col: 0, abs_row: false, abs_col: false });
}
