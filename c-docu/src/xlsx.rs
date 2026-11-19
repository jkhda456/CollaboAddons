//! Excel workbooks (.xlsx, .xlsm, .xltx): SpreadsheetML, edited in place. Cells are addressed as
//! `Sheet1!B3` (or `B3` on the first sheet), ranges as `A1:C9`, `B:B`, `3:5`. Setting a value
//! touches its cell (and the shared strings when it is text); formulas that depend on a changed
//! cell are recalculated and their cached values written, so readers that do not calculate see
//! the new numbers too; styles are added to styles.xml as new combinations, never by changing one
//! other cells use.

use crate::formula::{self, Area, CellRef, Ctx, Val, XlErr};
use crate::model::{Item, Match, Outline, RunInfo};
use crate::numfmt;
use crate::opc::{Package, Rels};
use crate::props::{Color, Edges, Props};
use crate::text::{self, at_mut, Dialect, Finder};
use crate::xml::{self, put_in_order, Document, Element, Node, R, S};
use crate::{err, Result};
use std::collections::{HashMap, HashSet};

/// SpreadsheetML rich text: `<r><rPr/><t/></r>` in `<si>` and `<is>`.
pub struct Sml;

impl Dialect for Sml {
    fn is_run(&self, e: &Element) -> bool {
        e.is(S, "r")
    }
    fn is_text(&self, e: &Element) -> bool {
        e.is(S, "t")
    }
    fn atom(&self, _e: &Element) -> Option<(char, Option<String>)> {
        None
    }
    fn descend(&self, _e: &Element) -> bool {
        false
    }
    fn is_props(&self, e: &Element) -> bool {
        e.is(S, "rPr")
    }
    fn new_text(&self, run: &Element, text: &str) -> Element {
        let mut t = Element::new(&run.qualify("t"), S);
        t.set_text(text);
        t
    }
    fn new_run(&self, para: &Element, _text: &str) -> Element {
        para.sibling("r")
    }
    fn fix_space(&self, t: &mut Element) {
        let s = t.text();
        if s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace) || s.contains('\n') {
            t.set_attr("xml:space", xml::XML, "preserve");
        }
    }
    fn settle(&self, parent: &mut Element, run: usize, text_el: usize) {
        if let Some(r) = parent.children[run].as_element_mut() {
            if let Some(t) = r.children.get_mut(text_el).and_then(|n| n.as_element_mut()) {
                self.fix_space(t);
            }
        }
    }
}

pub struct Sheet {
    pub name: String,
    pub part: String,
    pub doc: Option<Document>,
    pub hidden: bool,
    pub rid: String,
}

#[derive(Clone, Debug)]
pub enum Raw {
    Value(Val),
    /// A formula (its text as in the file, shared ones expanded) and its cached value.
    Formula(String, Val),
}

pub struct Xlsx {
    pub pkg: Package,
    pub kind: &'static str,
    pub wb_part: String,
    pub wb: Document,
    pub wb_rels: Rels,
    pub sheets: Vec<Sheet>,
    sst_part: Option<String>,
    pub sst: Option<Document>,
    sst_pos: Vec<usize>,
    styles_part: Option<String>,
    pub styles: Option<Document>,
    pub date1904: bool,
    /// Cells whose values changed (for recalculation): (sheet, row, col).
    changed: HashSet<(usize, u32, u32)>,
    pub warnings: Vec<String>,
}

/// A cell, as read.
#[derive(Clone, Debug)]
pub struct CellInfo {
    pub row: u32,
    pub col: u32,
    pub raw: Raw,
    pub style: u32,
}

impl Xlsx {
    pub fn open(pkg: Package, main: &str) -> Result<Xlsx> {
        let ct = pkg.content_type(main).unwrap_or_default();
        let kind = if ct.contains("template") {
            "xltx"
        } else if ct.contains("macroEnabled") {
            "xlsm"
        } else {
            "xlsx"
        };
        let wb = pkg.xml(main)?;
        let wb_rels = pkg.rels(main)?;
        let date1904 = wb.root.child(S, "workbookPr").and_then(|p| p.own("date1904")).map(|v| v == "1" || v == "true").unwrap_or(false);
        let mut sheets = Vec::new();
        if let Some(ss) = wb.root.child(S, "sheets") {
            for s in ss.children_named(S, "sheet") {
                let rid = s.attr_ns(R, "id").unwrap_or("").to_string();
                let part = wb_rels.part(&rid).and_then(|p| pkg.zip.real_name(&p)).unwrap_or_default();
                let is_ws = wb_rels.get(&rid).map(|r| r.kind() == "worksheet").unwrap_or(false);
                let doc = if is_ws && !part.is_empty() { Some(pkg.xml(&part)?) } else { None };
                sheets.push(Sheet {
                    name: s.own("name").unwrap_or("").to_string(),
                    part,
                    doc,
                    hidden: s.own("state").map(|v| v != "visible").unwrap_or(false),
                    rid,
                });
            }
        }
        let sst_part = wb_rels.first_part("sharedStrings").and_then(|p| pkg.zip.real_name(&p));
        let sst = match &sst_part {
            Some(p) => Some(pkg.xml(p)?),
            None => None,
        };
        let styles_part = wb_rels.first_part("styles").and_then(|p| pkg.zip.real_name(&p));
        let styles = match &styles_part {
            Some(p) => Some(pkg.xml(p)?),
            None => None,
        };
        let mut x = Xlsx { pkg, kind, wb_part: main.to_string(), wb, wb_rels, sheets, sst_part, sst, sst_pos: Vec::new(), styles_part, styles, date1904, changed: HashSet::new(), warnings: Vec::new() };
        x.index_sst();
        Ok(x)
    }

    fn index_sst(&mut self) {
        self.sst_pos.clear();
        if let Some(d) = &self.sst {
            for (i, n) in d.root.children.iter().enumerate() {
                if n.as_element().map(|e| e.is(S, "si")).unwrap_or(false) {
                    self.sst_pos.push(i);
                }
            }
        }
    }

    fn si(&self, i: usize) -> Option<&Element> {
        let d = self.sst.as_ref()?;
        d.root.children.get(*self.sst_pos.get(i)?)?.as_element()
    }

    /// The text of a shared string or inline string element (`si`, `is`): its runs' or its own
    /// `t`, without phonetic runs.
    pub fn string_text(e: &Element) -> String {
        let mut s = String::new();
        for c in e.elements() {
            if c.is(S, "t") {
                s.push_str(&decode_x(&c.text()));
            } else if c.is(S, "r") {
                if let Some(t) = c.child(S, "t") {
                    s.push_str(&decode_x(&t.text()));
                }
            }
        }
        s
    }

    pub fn sheet_index(&self, name: &str) -> Option<usize> {
        self.sheets.iter().position(|s| s.name == name).or_else(|| self.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(name)))
    }

    // ── styles ──────────────────────────────────────────────────────────────────────────────

    fn style_list(&self, list: &str) -> Vec<&Element> {
        match &self.styles {
            Some(d) => d.root.child(S, list).map(|l| l.elements().collect()).unwrap_or_default(),
            None => Vec::new(),
        }
    }

    fn xf(&self, s: u32) -> Option<&Element> {
        self.style_list("cellXfs").into_iter().nth(s as usize)
    }

    pub fn number_format(&self, s: u32) -> String {
        let Some(xf) = self.xf(s) else { return "General".into() };
        let id: u32 = xf.own("numFmtId").and_then(|v| v.parse().ok()).unwrap_or(0);
        for nf in self.style_list("numFmts") {
            if nf.own("numFmtId").and_then(|v| v.parse::<u32>().ok()) == Some(id) {
                return nf.own("formatCode").unwrap_or("General").to_string();
            }
        }
        numfmt::builtin(id).unwrap_or("General").to_string()
    }

    /// What a cell shows.
    pub fn display(&self, v: &Val, s: u32) -> String {
        match v {
            Val::Num(n) => numfmt::format_number(*n, &self.number_format(s), self.date1904),
            Val::Str(t) => {
                let f = self.number_format(s);
                if f.contains('@') && f != "@" {
                    numfmt::format_text(t, &f)
                } else {
                    t.clone()
                }
            }
            v => v.as_text(),
        }
    }

    /// The direct formatting of a cell style, for `read --runs`.
    fn style_summary(&self, s: u32) -> Vec<String> {
        let mut v = Vec::new();
        let Some(xf) = self.xf(s) else { return v };
        let font_id: usize = xf.own("fontId").and_then(|x| x.parse().ok()).unwrap_or(0);
        if let Some(font) = self.style_list("fonts").get(font_id) {
            v.extend(sml_font_summary(font));
        }
        let fill_id: usize = xf.own("fillId").and_then(|x| x.parse().ok()).unwrap_or(0);
        if let Some(fill) = self.style_list("fills").get(fill_id) {
            if let Some(pf) = fill.child(S, "patternFill") {
                if pf.own("patternType") == Some("solid") {
                    if let Some(c) = pf.child(S, "fgColor").and_then(color_of) {
                        v.push(format!("fill={c}"));
                    }
                }
            }
        }
        let nf = self.number_format(s);
        if nf != "General" {
            v.push(format!("numfmt={nf}"));
        }
        if let Some(al) = xf.child(S, "alignment") {
            if let Some(h) = al.own("horizontal") {
                v.push(format!("align={h}"));
            }
            if let Some(h) = al.own("vertical") {
                v.push(format!("valign={h}"));
            }
            if al.own("wrapText") == Some("1") {
                v.push("wrap".into());
            }
        }
        let border_id: usize = xf.own("borderId").and_then(|x| x.parse().ok()).unwrap_or(0);
        if border_id > 0 {
            if let Some(b) = self.style_list("borders").get(border_id) {
                let edges: Vec<String> = b.elements().filter(|e| e.own("style").is_some()).map(|e| format!("{}:{}", e.local(), e.own("style").unwrap())).collect();
                if !edges.is_empty() {
                    v.push(format!("border={}", edges.join(",")));
                }
            }
        }
        v
    }

    // ── cells ───────────────────────────────────────────────────────────────────────────────

    /// Every cell of a sheet (in the order of the file), shared formulas expanded.
    pub fn cells(&self, si: usize) -> Vec<CellInfo> {
        let mut out = Vec::new();
        let Some(doc) = &self.sheets[si].doc else { return out };
        let Some(data) = doc.root.child(S, "sheetData") else { return out };
        let mut shared: HashMap<String, (String, u32, u32)> = HashMap::new();
        let mut next_row = 0u32;
        for row in data.children_named(S, "row") {
            let r = row.own("r").and_then(|v| v.parse::<u32>().ok()).map(|v| v - 1).unwrap_or(next_row);
            next_row = r + 1;
            let mut next_col = 0u32;
            for c in row.children_named(S, "c") {
                let (rr, cc) = match c.own("r").and_then(formula::parse_cell) {
                    Some(cr) => (cr.row, cr.col),
                    None => (r, next_col),
                };
                next_col = cc + 1;
                let style: u32 = c.own("s").and_then(|v| v.parse().ok()).unwrap_or(0);
                let value = self.cell_value(c);
                let raw = match c.child(S, "f") {
                    Some(f) => {
                        let text = f.text();
                        let t = f.own("t").unwrap_or("normal");
                        let ftext = if t == "shared" {
                            let sid = f.own("si").unwrap_or("").to_string();
                            if !text.is_empty() {
                                shared.insert(sid, (text.clone(), rr, cc));
                                text
                            } else if let Some((master, mr, mc)) = shared.get(&sid) {
                                formula::shift(master, rr as i64 - *mr as i64, cc as i64 - *mc as i64)
                            } else {
                                String::new()
                            }
                        } else {
                            text
                        };
                        if ftext.is_empty() && t != "shared" {
                            Raw::Value(value)
                        } else {
                            Raw::Formula(ftext, value)
                        }
                    }
                    None => Raw::Value(value),
                };
                out.push(CellInfo { row: rr, col: cc, raw, style });
            }
        }
        out
    }

    fn cell_value(&self, c: &Element) -> Val {
        let t = c.own("t").unwrap_or("n");
        if t == "inlineStr" {
            return match c.child(S, "is") {
                Some(is) => Val::Str(Self::string_text(is)),
                None => Val::Str(String::new()),
            };
        }
        let Some(v) = c.child(S, "v").map(|v| v.text()) else { return Val::Empty };
        match t {
            "s" => v.trim().parse::<usize>().ok().and_then(|i| self.si(i)).map(|e| Val::Str(Self::string_text(e))).unwrap_or(Val::Str(String::new())),
            "str" => Val::Str(decode_x(&v)),
            "b" => Val::Bool(v.trim() == "1" || v.trim().eq_ignore_ascii_case("true")),
            "e" => Val::Err(XlErr::parse(v.trim()).unwrap_or(XlErr::NA)),
            "d" => formula::parse_date_time(&v.replace('T', " ")).map(Val::Num).unwrap_or(Val::Str(v)),
            _ => v.trim().parse::<f64>().map(Val::Num).unwrap_or(Val::Str(v)),
        }
    }

    /// Resolves `Sheet1!A1:B2`, `A1`, `'My sheet'!C:C`, `Sheet1` (the used range).
    pub fn resolve(&self, addr: &str) -> Result<(usize, Area)> {
        let a = addr.trim();
        if let Some(i) = self.sheet_index(a.trim_matches('\'')) {
            let (r, c) = self.extent(i);
            return Ok((i, Area { sheet: None, a: CellRef { row: 0, col: 0, abs_row: false, abs_col: false }, b: CellRef { row: r, col: c, abs_row: false, abs_col: false }, whole: None }));
        }
        let area = formula::parse_area(a).ok_or_else(|| err(format!("bad cell address {addr:?} (Sheet1!B3, B3, A1:C9, B:B, 3:5)")))?;
        let si = match &area.sheet {
            Some(s) => self.sheet_index(s).ok_or_else(|| err(format!("no sheet {s:?}; the sheets are: {}", self.sheet_names().join(", "))))?,
            None => self.first_visible(),
        };
        if self.sheets[si].doc.is_none() {
            return Err(err(format!("{} is a chart sheet", self.sheets[si].name)));
        }
        Ok((si, area))
    }

    pub fn mark_changed(&mut self, si: usize, r: u32, c: u32) {
        self.changed.insert((si, r, c));
    }

    pub fn sheet_names(&self) -> Vec<String> {
        self.sheets.iter().map(|s| s.name.clone()).collect()
    }

    fn first_visible(&self) -> usize {
        // The active tab.
        let active: usize = self.wb.root.child(S, "bookViews").and_then(|b| b.child(S, "workbookView")).and_then(|v| v.own("activeTab")).and_then(|v| v.parse().ok()).unwrap_or(0);
        if active < self.sheets.len() && self.sheets[active].doc.is_some() {
            return active;
        }
        self.sheets.iter().position(|s| s.doc.is_some() && !s.hidden).unwrap_or(0)
    }

    /// (last row, last column) with content, 0-based.
    pub fn extent(&self, si: usize) -> (u32, u32) {
        let mut r = 0;
        let mut c = 0;
        for cell in self.cells(si) {
            if !matches!(cell.raw, Raw::Value(Val::Empty)) || cell.style != 0 {
                r = r.max(cell.row);
                c = c.max(cell.col);
            }
        }
        (r, c)
    }

    pub fn addr_of(&self, si: usize, row: u32, col: u32) -> String {
        format!("{}!{}", formula::quote_sheet(&self.sheets[si].name), formula::cell_name(row, col))
    }

    pub fn outline(&self, with_runs: bool, filter: &[String]) -> Result<Outline> {
        let mut items = Vec::new();
        let mut wanted: Vec<(usize, Option<Area>)> = Vec::new();
        if filter.is_empty() {
            wanted = (0..self.sheets.len()).map(|i| (i, None)).collect();
        } else {
            for f in filter {
                let (si, a) = self.resolve(f)?;
                wanted.push((si, if self.sheet_index(f.trim_matches('\'')).is_some() { None } else { Some(a) }));
            }
        }
        for (si, area) in wanted {
            let sh = &self.sheets[si];
            let Some(_) = &sh.doc else {
                items.push(Item { addr: sh.name.clone(), kind: "sheet".into(), text: "chart sheet".into(), ..Default::default() });
                continue;
            };
            let (er, ec) = self.extent(si);
            let mut it = Item { addr: formula::quote_sheet(&sh.name), kind: "sheet".into(), text: format!("A1:{}", formula::cell_name(er, ec)), ..Default::default() };
            if sh.hidden {
                it.extra.push(("hidden".into(), String::new()));
            }
            let merges = self.merges(si);
            if !merges.is_empty() {
                it.extra.push(("merged".into(), merges.iter().map(|m| m.text()).collect::<Vec<_>>().join(" ")));
            }
            items.push(it);
            let mut cells = self.cells(si);
            cells.sort_by_key(|c| (c.row, c.col));
            for c in cells {
                if let Some(a) = &area {
                    let (r0, r1) = a.rows();
                    let (c0, c1) = a.cols();
                    if c.row < r0 || c.row > r1 || c.col < c0 || c.col > c1 {
                        continue;
                    }
                }
                let (val, f) = match &c.raw {
                    Raw::Value(v) => (v.clone(), None),
                    Raw::Formula(f, v) => (v.clone(), Some(f.clone())),
                };
                if matches!(val, Val::Empty) && f.is_none() {
                    continue;
                }
                let shown = self.display(&val, c.style);
                let mut it = Item { addr: self.addr_of(si, c.row, c.col), kind: "cell".into(), text: shown.clone(), depth: 1, ..Default::default() };
                if let Some(f) = f {
                    it.extra.push(("formula".into(), formula::for_display(&f)));
                }
                let raw_text = match &val {
                    Val::Num(n) => numfmt::general(*n),
                    v => v.as_text(),
                };
                if raw_text != shown {
                    it.extra.push(("value".into(), raw_text));
                }
                if with_runs {
                    let props = self.style_summary(c.style);
                    let rich = self.rich_runs(si, c.row, c.col);
                    if rich.is_empty() {
                        it.runs = vec![RunInfo { text: shown, props, start: 0 }];
                    } else {
                        it.extra.push(("style".into(), props.join(" ")));
                        it.runs = rich;
                    }
                }
                items.push(it);
            }
        }
        Ok(Outline { items })
    }

    /// Tab-separated text of each sheet's used range.
    pub fn plain(&self) -> String {
        let mut s = String::new();
        for si in 0..self.sheets.len() {
            if self.sheets[si].doc.is_none() {
                continue;
            }
            s.push_str(&format!("== {}\n", self.sheets[si].name));
            let mut grid: HashMap<(u32, u32), String> = HashMap::new();
            let (mut er, mut ec) = (0, 0);
            for c in self.cells(si) {
                let v = match &c.raw {
                    Raw::Value(v) | Raw::Formula(_, v) => v.clone(),
                };
                if matches!(v, Val::Empty) {
                    continue;
                }
                er = er.max(c.row);
                ec = ec.max(c.col);
                grid.insert((c.row, c.col), self.display(&v, c.style));
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

    /// The string element (`si` or `is`) of a rich text cell.
    pub fn rich_string(&self, si: usize, r: u32, c: u32) -> Option<Element> {
        let el = match self.cell_sst_index(si, r, c) {
            Some(i) => self.si(i),
            None => self.find_cell(si, r, c).and_then(|e| e.child(S, "is")),
        }?;
        el.child(S, "r").is_some().then(|| el.clone())
    }

    /// The runs of a rich text cell, with their properties.
    fn rich_runs(&self, si: usize, r: u32, c: u32) -> Vec<RunInfo> {
        let el = match self.cell_sst_index(si, r, c) {
            Some(i) => self.si(i),
            None => self.find_cell(si, r, c).and_then(|e| e.child(S, "is")),
        };
        let Some(el) = el else { return vec![] };
        if el.child(S, "r").is_none() {
            return vec![];
        }
        let mut out = Vec::new();
        let mut pos = 0;
        for run in el.children_named(S, "r") {
            let t = run.child(S, "t").map(|t| decode_x(&t.text())).unwrap_or_default();
            let props = run.child(S, "rPr").map(sml_font_summary).unwrap_or_default();
            let n = t.chars().count();
            out.push(RunInfo { text: t, props, start: pos });
            pos += n;
        }
        out
    }

    fn merges(&self, si: usize) -> Vec<Area> {
        let Some(doc) = &self.sheets[si].doc else { return vec![] };
        doc.root.child(S, "mergeCells").map(|m| m.children_named(S, "mergeCell").filter_map(|c| c.own("ref").and_then(formula::parse_area)).collect()).unwrap_or_default()
    }

    // ── finding and replacing ───────────────────────────────────────────────────────────────

    fn scope_cells(&self, scope: Option<&str>) -> Result<Vec<(usize, Option<Area>)>> {
        Ok(match scope {
            None => (0..self.sheets.len()).filter(|&i| self.sheets[i].doc.is_some()).map(|i| (i, None)).collect(),
            Some(s) => {
                let (si, a) = self.resolve(s)?;
                vec![(si, if self.sheet_index(s.trim_matches('\'')).is_some() { None } else { Some(a) })]
            }
        })
    }

    pub fn find(&self, f: &Finder, scope: Option<&str>, formulas: bool) -> Result<Vec<Match>> {
        let mut out = Vec::new();
        for (si, area) in self.scope_cells(scope)? {
            let mut cells = self.cells(si);
            cells.sort_by_key(|c| (c.row, c.col));
            for c in cells {
                if let Some(a) = &area {
                    if !in_area(a, c.row, c.col) {
                        continue;
                    }
                }
                let text = match (&c.raw, formulas) {
                    (Raw::Formula(f, _), true) => formula::for_display(f),
                    (Raw::Value(v), _) | (Raw::Formula(_, v), false) => self.display(v, c.style),
                };
                for (a, b, _) in f.find(&text, None) {
                    out.push(Match { addr: self.addr_of(si, c.row, c.col), start: a, end: b, text: text.chars().skip(a).take(b - a).collect(), context: text::escape_line(&text) });
                }
            }
        }
        Ok(out)
    }

    /// Replaces text in text cells (shared and inline strings; with `formulas`, in formulas).
    pub fn replace(&mut self, f: &Finder, with: &str, scope: Option<&str>, nth: Option<usize>, formulas: bool) -> Result<Vec<String>> {
        let with = text::unescape(with);
        let scopes = self.scope_cells(scope)?;
        let refcount = self.sst_refcounts();
        // (sheet, row, col, which match in the cell when one is picked)
        let mut targets: Vec<(usize, u32, u32, Option<usize>)> = Vec::new();
        let mut count = 0;
        for (si, area) in &scopes {
            let mut cells = self.cells(*si);
            cells.sort_by_key(|c| (c.row, c.col));
            for c in cells {
                if let Some(a) = area {
                    if !in_area(a, c.row, c.col) {
                        continue;
                    }
                }
                let text = match (&c.raw, formulas) {
                    (Raw::Formula(f, _), true) => formula::for_display(f),
                    (Raw::Value(Val::Str(s)), false) => s.clone(),
                    _ => continue,
                };
                let n = f.find(&text, None).len();
                if n == 0 {
                    continue;
                }
                match nth {
                    Some(k) if count < k && k <= count + n => targets.push((*si, c.row, c.col, Some(k - count))),
                    Some(_) => {}
                    None => targets.push((*si, c.row, c.col, None)),
                }
                count += n;
            }
        }
        // Shared strings used only by targeted cells change in place, once.
        let mut uses: HashMap<usize, usize> = HashMap::new();
        for &(sh, r, c, _) in &targets {
            if let Some(i) = self.cell_sst_index(sh, r, c) {
                *uses.entry(i).or_insert(0) += 1;
            }
        }
        let mut edited: HashMap<usize, usize> = HashMap::new();
        let mut done = Vec::new();
        for &(sh, r, c, local) in &targets {
            let addr = self.addr_of(sh, r, c);
            if formulas {
                let cell = self.cell_mut(sh, r, c);
                let Some(fe) = cell.child_mut(S, "f") else { continue };
                if fe.own("t") == Some("shared") && fe.text().is_empty() {
                    // A cell of a shared group: it gets its own formula.
                    continue;
                }
                let disp = formula::for_display(&fe.text());
                let new = replace_all(f, &disp, &with, local);
                let file = formula::to_file(&new).map_err(|e| err(format!("{addr}: the new formula does not parse: {e}")))?;
                formula::parse(&file).map_err(|e| err(format!("{addr}: the new formula does not parse: {e}")))?;
                fe.set_text(&file);
                self.changed.insert((sh, r, c));
                done.push(addr);
                continue;
            }
            match self.cell_sst_index(sh, r, c) {
                Some(i) => {
                    let mine = uses.get(&i).copied().unwrap_or(0);
                    let total = refcount.get(&i).copied().unwrap_or(0);
                    if let Some(&ni) = edited.get(&i) {
                        if ni != i {
                            let cell = self.cell_mut(sh, r, c);
                            set_v(cell, &ni.to_string());
                        }
                    } else if mine >= total && local.is_none() {
                        self.edit_si(i, f, &with, None)?;
                        edited.insert(i, i);
                    } else {
                        let ni = self.fork_si(i);
                        self.edit_si(ni, f, &with, local)?;
                        if local.is_none() {
                            edited.insert(i, ni);
                        }
                        let cell = self.cell_mut(sh, r, c);
                        set_v(cell, &ni.to_string());
                    }
                }
                None => {
                    let cell = self.cell_mut(sh, r, c);
                    match cell.child_mut(S, "is") {
                        Some(is) => edit_string_el(is, f, &with, local)?,
                        None => continue,
                    }
                }
            }
            self.changed.insert((sh, r, c));
            done.push(addr);
        }
        Ok(done)
    }

    fn sst_refcounts(&self) -> HashMap<usize, usize> {
        let mut m = HashMap::new();
        for sh in &self.sheets {
            let Some(doc) = &sh.doc else { continue };
            let Some(data) = doc.root.child(S, "sheetData") else { continue };
            for row in data.children_named(S, "row") {
                for c in row.children_named(S, "c") {
                    if c.own("t") == Some("s") {
                        if let Some(i) = c.child(S, "v").and_then(|v| v.text().trim().parse::<usize>().ok()) {
                            *m.entry(i).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        m
    }

    fn cell_sst_index(&self, sh: usize, r: u32, c: u32) -> Option<usize> {
        let cell = self.find_cell(sh, r, c)?;
        if cell.own("t") != Some("s") {
            return None;
        }
        cell.child(S, "v")?.text().trim().parse().ok()
    }

    fn edit_si(&mut self, i: usize, f: &Finder, with: &str, nth: Option<usize>) -> Result<()> {
        let pos = self.sst_pos[i];
        let d = self.sst.as_mut().unwrap();
        let si = d.root.children[pos].as_element_mut().unwrap();
        edit_string_el(si, f, with, nth)
    }

    /// A copy of a shared string at the end of the table; its index.
    fn fork_si(&mut self, i: usize) -> usize {
        let pos = self.sst_pos[i];
        let d = self.sst.as_mut().unwrap();
        let copy = d.root.children[pos].as_element().unwrap().clone();
        d.root.push(copy);
        self.sst_pos.push(d.root.children.len() - 1);
        self.bump_sst_counts(1, 1);
        self.sst_pos.len() - 1
    }

    fn bump_sst_counts(&mut self, count: i64, unique: i64) {
        if let Some(d) = &mut self.sst {
            for (k, delta) in [("count", count), ("uniqueCount", unique)] {
                if let Some(v) = d.root.own(k).and_then(|v| v.parse::<i64>().ok()) {
                    d.root.set_plain(k, &(v + delta).max(0).to_string());
                }
            }
        }
    }

    /// The index of a shared string with this text (plain), added when there is none.
    fn shared_string(&mut self, text: &str) -> usize {
        if self.sst.is_none() {
            let part = format!("{}/sharedStrings.xml", crate::opc::folder(&self.wb_part));
            let doc = Document::parse("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<sst xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" count=\"0\" uniqueCount=\"0\"></sst>").unwrap();
            self.sst = Some(doc);
            self.wb_rels.add_part("http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings", &part);
            self.pkg.set_content_type(&part, "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml", false);
            self.sst_part = Some(part);
        }
        for i in 0..self.sst_pos.len() {
            let e = self.si(i).unwrap();
            if e.child(S, "r").is_none() && Self::string_text(e) == text && e.child(S, "rPh").is_none() {
                self.bump_sst_counts(1, 0);
                return i;
            }
        }
        let d = self.sst.as_mut().unwrap();
        let p = d.root.prefix().to_string();
        let q = |l: &str| if p.is_empty() { l.to_string() } else { format!("{p}:{l}") };
        let mut si = Element::new(&q("si"), S);
        let mut t = Element::new(&q("t"), S);
        t.set_text(&encode_x(text));
        Sml.fix_space(&mut t);
        si.push(t);
        d.root.push(si);
        self.sst_pos.push(d.root.children.len() - 1);
        self.bump_sst_counts(1, 1);
        self.sst_pos.len() - 1
    }

    pub fn find_cell(&self, sh: usize, r: u32, c: u32) -> Option<&Element> {
        let data = self.sheets[sh].doc.as_ref()?.root.child(S, "sheetData")?;
        let want = formula::cell_name(r, c);
        let mut next_row = 0;
        for row in data.children_named(S, "row") {
            let rr = row.own("r").and_then(|v| v.parse::<u32>().ok()).map(|v| v - 1).unwrap_or(next_row);
            next_row = rr + 1;
            if rr != r {
                continue;
            }
            let mut next_col = 0;
            for cell in row.children_named(S, "c") {
                let cc = cell.own("r").and_then(formula::parse_cell).map(|x| x.col).unwrap_or(next_col);
                next_col = cc + 1;
                if cc == c || cell.own("r") == Some(want.as_str()) {
                    return Some(cell);
                }
            }
            return None;
        }
        None
    }

    /// The cell element, made (in order, with the row's or column's style) when missing.
    pub fn cell_mut(&mut self, sh: usize, r: u32, c: u32) -> &mut Element {
        let col_style = self.column_style(sh, c);
        let doc = self.sheets[sh].doc.as_mut().expect("a worksheet");
        let root_prefix = doc.root.prefix().to_string();
        let q = |l: &str| if root_prefix.is_empty() { l.to_string() } else { format!("{root_prefix}:{l}") };
        if doc.root.child(S, "sheetData").is_none() {
            let sd = Element::new(&q("sheetData"), S);
            put_in_order(&mut doc.root, sd, WORKSHEET_ORDER);
        }
        let data = doc.root.child_mut(S, "sheetData").unwrap();
        // Rows and cells without `r` get one (their places are implicit otherwise).
        normalize_refs(data);
        let mut ri = None;
        let mut insert_at = data.children.len();
        for (i, n) in data.children.iter().enumerate() {
            if let Node::Element(row) = n {
                if !row.is(S, "row") {
                    continue;
                }
                let rr = row.own("r").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0).saturating_sub(1);
                if rr == r {
                    ri = Some(i);
                    break;
                }
                if rr > r {
                    insert_at = i;
                    break;
                }
            }
        }
        let ri = match ri {
            Some(i) => i,
            None => {
                let mut row = Element::new(&q("row"), S);
                row.set_plain("r", &(r + 1).to_string());
                data.children.insert(insert_at, Node::Element(row));
                insert_at
            }
        };
        let row = data.children[ri].as_element_mut().unwrap();
        let row_style = if row.own("customFormat") == Some("1") { row.own("s").map(str::to_string) } else { None };
        let want = formula::cell_name(r, c);
        let mut ci = None;
        let mut cat = row.children.len();
        for (i, n) in row.children.iter().enumerate() {
            if let Node::Element(cell) = n {
                if !cell.is(S, "c") {
                    continue;
                }
                let cc = cell.own("r").and_then(formula::parse_cell).map(|x| x.col).unwrap_or(0);
                if cc == c {
                    ci = Some(i);
                    break;
                }
                if cc > c {
                    cat = i;
                    break;
                }
            }
        }
        let ci = match ci {
            Some(i) => i,
            None => {
                // Before an extLst at the end of the row, if any.
                if cat == row.children.len() {
                    if let Some(p) = row.position(S, "extLst") {
                        cat = p;
                    }
                }
                let mut cell = Element::new(&q("c"), S);
                cell.set_plain("r", &want);
                if let Some(s) = row_style.or(col_style) {
                    if s != "0" {
                        cell.set_plain("s", &s);
                    }
                }
                row.children.insert(cat, Node::Element(cell));
                // The row's spans hint, when it has one.
                if let Some(sp) = row.own("spans").map(str::to_string) {
                    if let Some((a, b)) = sp.split_once(':') {
                        let (a, b): (u32, u32) = (a.parse().unwrap_or(1), b.parse().unwrap_or(1));
                        let (na, nb) = (a.min(c + 1), b.max(c + 1));
                        if (na, nb) != (a, b) {
                            row.set_plain("spans", &format!("{na}:{nb}"));
                        }
                    }
                }
                cat
            }
        };
        row.children[ci].as_element_mut().unwrap()
    }

    fn column_style(&self, sh: usize, c: u32) -> Option<String> {
        let doc = self.sheets[sh].doc.as_ref()?;
        for cols in doc.root.children_named(S, "cols") {
            for col in cols.children_named(S, "col") {
                let min: u32 = col.own("min")?.parse().ok()?;
                let max: u32 = col.own("max")?.parse().ok()?;
                if c + 1 >= min && c + 1 <= max {
                    return col.own("style").map(str::to_string);
                }
            }
        }
        None
    }

    /// Sets cells from text: numbers, TRUE/FALSE, `=formulas`, dates (into a date cell), text;
    /// `'text` is always text. A range takes values separated by `|` (columns) and `;` or
    /// newlines (rows), or one value for every cell.
    pub fn set(&mut self, addr: &str, value: &str) -> Result<String> {
        let (si, area) = self.resolve(addr)?;
        let rows: Vec<Vec<String>> = if value.contains(['|', ';', '\n']) && !value.starts_with('=') {
            value.split(['\n', ';']).map(|r| r.split('|').map(|c| c.to_string()).collect()).collect()
        } else {
            vec![vec![value.to_string()]]
        };
        let (r0, mut r1) = area.rows();
        let (c0, mut c1) = area.cols();
        if area.single() {
            r1 = r0 + rows.len() as u32 - 1;
            c1 = c0 + rows.iter().map(|r| r.len()).max().unwrap_or(1) as u32 - 1;
        }
        if area.whole.is_some() {
            return Err(err("set takes cells or a range, not whole rows or columns"));
        }
        let one = rows.len() == 1 && rows[0].len() == 1;
        for r in r0..=r1 {
            for c in c0..=c1 {
                let v = if one {
                    rows[0][0].clone()
                } else {
                    match rows.get((r - r0) as usize).and_then(|row| row.get((c - c0) as usize)) {
                        Some(v) => v.clone(),
                        None => continue,
                    }
                };
                // A formula with relative references, given for a range: shifted per cell.
                let v = if one && v.starts_with('=') && (r, c) != (r0, c0) { format!("={}", formula::shift(&v[1..], (r - r0) as i64, (c - c0) as i64)) } else { v };
                self.set_cell(si, r, c, &v)?;
            }
        }
        let a = if (r0, c0) == (r1, c1) { formula::cell_name(r0, c0) } else { format!("{}:{}", formula::cell_name(r0, c0), formula::cell_name(r1, c1)) };
        Ok(format!("{}!{a}", formula::quote_sheet(&self.sheets[si].name)))
    }

    pub fn set_cell(&mut self, si: usize, r: u32, c: u32, value: &str) -> Result<()> {
        self.unshare_if_master(si, r, c);
        let style = self.find_cell(si, r, c).and_then(|e| e.own("s")).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        let is_date_cell = numfmt::is_date(&self.number_format(style));
        enum V {
            Num(f64),
            Str(String),
            Bool(bool),
            Formula(String),
            Empty,
        }
        let v = if value.is_empty() {
            V::Empty
        } else if let Some(t) = value.strip_prefix('\'') {
            V::Str(t.to_string())
        } else if value.starts_with('=') && value.len() > 1 {
            let file = formula::to_file(value).map_err(|e| err(format!("bad formula {value:?}: {e}")))?;
            formula::parse(&file).map_err(|e| err(format!("bad formula {value:?}: {e}")))?;
            V::Formula(file)
        } else if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false") {
            V::Bool(value.eq_ignore_ascii_case("true"))
        } else if let Ok(n) = value.trim().parse::<f64>() {
            V::Num(n)
        } else if let Some(n) = formula::parse_date_time(value).filter(|_| value.contains(['-', '/', ':'])) {
            if !is_date_cell {
                // A date into a plain cell gets a date format.
                let fmt = if value.contains(':') && value.contains(['-', '/']) { "yyyy-mm-dd hh:mm" } else if value.contains(':') { "hh:mm" } else { "yyyy-mm-dd" };
                self.style_cells(si, &[(r, c)], &Props { number_format: Some(fmt.into()), keys: vec!["numfmt".into()], ..Default::default() })?;
            }
            V::Num(n)
        } else if let Some(n) = formula::text_to_num(value).filter(|_| value.trim().ends_with('%')) {
            if !self.number_format(style).contains('%') {
                self.style_cells(si, &[(r, c)], &Props { number_format: Some("0%".into()), keys: vec!["numfmt".into()], ..Default::default() })?;
            }
            V::Num(n)
        } else {
            V::Str(value.to_string())
        };
        let sst_index = if let V::Str(s) = &v { Some(self.shared_string(s)) } else { None };
        let cell = self.cell_mut(si, r, c);
        cell.remove_children(S, "is");
        cell.remove_children(S, "f");
        match v {
            V::Empty => {
                cell.remove_own("t");
                cell.remove_children(S, "v");
            }
            V::Num(n) => {
                cell.remove_own("t");
                set_v(cell, &num_text(n));
            }
            V::Bool(b) => {
                cell.set_plain("t", "b");
                set_v(cell, if b { "1" } else { "0" });
            }
            V::Str(_) => {
                cell.set_plain("t", "s");
                set_v(cell, &sst_index.unwrap().to_string());
            }
            V::Formula(f) => {
                cell.remove_own("t");
                cell.remove_children(S, "v");
                let mut fe = cell.sibling("f");
                fe.set_text(&f);
                cell.children.insert(0, Node::Element(fe));
            }
        }
        self.changed.insert((si, r, c));
        Ok(())
    }

    /// When a cell holds the master of a shared formula, its group gets explicit formulas so the
    /// others keep theirs when it changes.
    fn unshare_if_master(&mut self, si: usize, r: u32, c: u32) {
        let Some(cell) = self.find_cell(si, r, c) else { return };
        let Some(f) = cell.child(S, "f") else { return };
        if f.own("t") != Some("shared") || f.text().is_empty() {
            return;
        }
        let sid = f.own("si").unwrap_or("").to_string();
        let cells = self.cells(si);
        let mut texts: Vec<(u32, u32, String)> = Vec::new();
        for ci in &cells {
            if let Raw::Formula(t, _) = &ci.raw {
                if let Some(e) = self.find_cell(si, ci.row, ci.col).and_then(|e| e.child(S, "f")) {
                    if e.own("t") == Some("shared") && e.own("si") == Some(sid.as_str()) {
                        texts.push((ci.row, ci.col, t.clone()));
                    }
                }
            }
        }
        for (rr, cc, t) in texts {
            let cell = self.cell_mut(si, rr, cc);
            if let Some(fe) = cell.child_mut(S, "f") {
                let mut n = fe.sibling("f");
                n.set_text(&t);
                *fe = n;
            }
        }
    }

    // ── styles of cells ─────────────────────────────────────────────────────────────────────

    pub fn style(&mut self, addr: &str, props: &Props, matching: Option<(&Finder, Option<usize>)>) -> Result<Vec<String>> {
        let (si, area) = self.resolve(addr)?;
        const CELL_KEYS: &[&str] = &[
            "bold", "italic", "underline", "strike", "color", "size", "font", "font.latin", "font.ea", "super", "sub", "baseline", "fill", "numfmt",
            "align", "valign", "wrap", "border", "indentlevel", "rotate", "protect", "colwidth", "rowheight", "w", "h", "merge", "clear",
        ];
        props.only(CELL_KEYS, "cells")?;
        let (r0, r1) = area.rows();
        let (c0, c1) = area.cols();
        let mut done = Vec::new();
        if let Some((f, nth)) = matching {
            // Part of a cell's text: rich text runs.
            let mut count = 0;
            let mut cells = self.cells(si);
            cells.sort_by_key(|c| (c.row, c.col));
            for ci in cells {
                if ci.row < r0 || ci.row > r1 || ci.col < c0 || ci.col > c1 {
                    continue;
                }
                let Raw::Value(Val::Str(t)) = &ci.raw else { continue };
                let ms = f.find(t, None);
                let mut ranges = Vec::new();
                for (a, b, _) in ms {
                    count += 1;
                    if nth.map(|n| n == count).unwrap_or(true) {
                        ranges.push((a, b));
                    }
                }
                if ranges.is_empty() {
                    continue;
                }
                self.style_text_ranges(si, ci.row, ci.col, &ranges, props)?;
                done.push(self.addr_of(si, ci.row, ci.col));
            }
            if done.is_empty() {
                return Err(err("no text matched"));
            }
            return Ok(done);
        }
        if let Some(w) = props.column_width.or(props.width.map(|pt| pt / 5.25)) {
            self.set_column_width(si, c0, c1.min(c0 + 16383), w);
            done.push(format!("columns {}:{}", formula::col_name(c0), formula::col_name(c1)));
        }
        if let Some(h) = props.row_height.or(props.height) {
            for r in r0..=r1.min(r0 + 100_000) {
                self.set_row_height(si, r, h);
            }
            done.push(format!("rows {}:{}", r0 + 1, r1 + 1));
        }
        if let Some(m) = props.merge {
            self.set_merge(si, &area, m)?;
            done.push(format!("{} {}", if m { "merged" } else { "unmerged" }, area.text()));
        }
        let cell_props = props.keys.iter().any(|k| !matches!(k.as_str(), "colwidth" | "rowheight" | "w" | "h" | "merge"));
        if cell_props {
            if area.whole.is_some() {
                // Whole rows or columns: the cells that exist, and the column's or row's style.
                let cells: Vec<(u32, u32)> = self.cells(si).iter().filter(|c| in_area(&area, c.row, c.col)).map(|c| (c.row, c.col)).collect();
                self.style_cells(si, &cells, props)?;
            } else {
                let mut cells = Vec::new();
                for r in r0..=r1 {
                    for c in c0..=c1 {
                        cells.push((r, c));
                    }
                }
                if cells.len() > 2_000_000 {
                    return Err(err("range too large"));
                }
                self.style_cells(si, &cells, props)?;
            }
            done.push(format!("{}!{}", formula::quote_sheet(&self.sheets[si].name), area_text_local(&area)));
        }
        Ok(done)
    }

    fn style_text_ranges(&mut self, si: usize, r: u32, c: u32, ranges: &[(usize, usize)], props: &Props) -> Result<()> {
        // The cell's string becomes its own rich text (a fork when others share it).
        let style = self.find_cell(si, r, c).and_then(|e| e.own("s")).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        let base_font = self.font_of_style(style);
        let idx = match self.cell_sst_index(si, r, c) {
            Some(i) => {
                let rc = self.sst_refcounts().get(&i).copied().unwrap_or(0);
                if rc > 1 {
                    let ni = self.fork_si(i);
                    let cell = self.cell_mut(si, r, c);
                    set_v(cell, &ni.to_string());
                    ni
                } else {
                    i
                }
            }
            None => return Err(err("only text cells take styles on part of their text")),
        };
        let pos = self.sst_pos[idx];
        let d = self.sst.as_mut().unwrap();
        let el = d.root.children[pos].as_element_mut().unwrap();
        // Plain text becomes one run with the cell's font.
        if el.child(S, "r").is_none() {
            let t = el.child(S, "t").cloned();
            el.children.retain(|n| !n.as_element().map(|e| e.is(S, "t")).unwrap_or(false));
            let mut run = el.sibling("r");
            let mut rpr = font_to_rpr(&base_font, el);
            rpr.touch();
            run.push(rpr);
            if let Some(mut t) = t {
                t.touch();
                run.push(t);
            }
            el.children.insert(0, Node::Element(run));
        }
        for (a, b) in ranges.iter().rev() {
            let runs = text::runs_in(&Sml, el, *a, *b);
            for rp in runs {
                let run = at_mut(el, &rp);
                if run.child(S, "rPr").is_none() {
                    let rpr = font_to_rpr(&base_font, run);
                    run.children.insert(0, Node::Element(rpr));
                }
                let rpr = run.child_mut(S, "rPr").unwrap();
                apply_font(rpr, props, true);
            }
        }
        self.changed.insert((si, r, c));
        Ok(())
    }

    fn font_of_style(&self, s: u32) -> Element {
        let font_id: usize = self.xf(s).and_then(|x| x.own("fontId")).and_then(|x| x.parse().ok()).unwrap_or(0);
        self.style_list("fonts").get(font_id).map(|f| (*f).clone()).unwrap_or_else(|| Element::new("font", S))
    }

    /// Gives each cell a style like its own with the properties changed.
    pub fn style_cells(&mut self, si: usize, cells: &[(u32, u32)], props: &Props) -> Result<()> {
        self.ensure_styles();
        let mut memo: HashMap<u32, u32> = HashMap::new();
        for &(r, c) in cells {
            let cur: u32 = self.find_cell(si, r, c).and_then(|e| e.own("s")).and_then(|s| s.parse().ok()).or_else(|| self.column_style(si, c).and_then(|s| s.parse().ok())).unwrap_or(0);
            let new = match memo.get(&cur) {
                Some(n) => *n,
                None => {
                    let n = self.derive_xf(cur, props)?;
                    memo.insert(cur, n);
                    n
                }
            };
            let cell = self.cell_mut(si, r, c);
            if new == 0 {
                cell.remove_own("s");
            } else {
                cell.set_plain("s", &new.to_string());
            }
        }
        Ok(())
    }

    fn ensure_styles(&mut self) {
        if self.styles.is_some() {
            return;
        }
        let part = format!("{}/styles.xml", crate::opc::folder(&self.wb_part));
        let doc = Document::parse(DEFAULT_STYLES).unwrap();
        self.styles = Some(doc);
        self.wb_rels.add_part("http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles", &part);
        self.pkg.set_content_type(&part, "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml", false);
        self.styles_part = Some(part);
    }

    /// The index of a cell format like `base` with `props` applied (an existing one when the same
    /// exists, else a new one at the end).
    fn derive_xf(&mut self, base: u32, p: &Props) -> Result<u32> {
        let styles = self.styles.as_mut().unwrap();
        let root = &mut styles.root;
        for list in ["numFmts", "fonts", "fills", "borders", "cellXfs"] {
            if root.child(S, list).is_none() && list != "numFmts" {
                let e = root.sibling(list);
                put_in_order(root, e, STYLESHEET_ORDER);
            }
        }
        let xfs: Vec<Element> = root.child(S, "cellXfs").unwrap().elements().cloned().collect();
        let mut xf = xfs.get(base as usize).cloned().unwrap_or_else(|| {
            let mut x = root.sibling("xf");
            for a in ["numFmtId", "fontId", "fillId", "borderId", "xfId"] {
                x.set_plain(a, "0");
            }
            x
        });
        xf.touch();
        let get = |x: &Element, a: &str| -> usize { x.own(a).and_then(|v| v.parse().ok()).unwrap_or(0) };
        // Font
        if p.has_char() {
            let fonts = root.child(S, "fonts").unwrap();
            let mut font = fonts.elements().nth(get(&xf, "fontId")).cloned().unwrap_or_else(|| fonts.sibling("font"));
            font.touch();
            if p.clear {
                font = fonts.elements().next().cloned().unwrap_or_else(|| fonts.sibling("font"));
            }
            apply_font(&mut font, p, false);
            let id = add_unique(root.child_mut(S, "fonts").unwrap(), font);
            xf.set_plain("fontId", &id.to_string());
            xf.set_plain("applyFont", "1");
        }
        if let Some(fill) = &p.fill {
            let fills = root.child(S, "fills").unwrap();
            let mut f = fills.sibling("fill");
            let mut pf = fills.sibling("patternFill");
            match fill {
                Color::Rgb(hex) => {
                    pf.set_plain("patternType", "solid");
                    let mut fg = fills.sibling("fgColor");
                    fg.set_plain("rgb", &format!("FF{hex}"));
                    pf.push(fg);
                    let mut bg = fills.sibling("bgColor");
                    bg.set_plain("indexed", "64");
                    pf.push(bg);
                }
                _ => pf.set_plain("patternType", "none"),
            }
            f.push(pf);
            // Fills 0 and 1 are reserved (none, gray125): a new one goes after them.
            let fills = root.child_mut(S, "fills").unwrap();
            while fills.elements().count() < 2 {
                let mut e = fills.sibling("fill");
                let mut pf = fills.sibling("patternFill");
                pf.set_plain("patternType", if fills.elements().count() == 0 { "none" } else { "gray125" });
                e.push(pf);
                fills.push(e);
            }
            let id = if matches!(fill, Color::Rgb(_)) { add_unique(fills, f) } else { 0 };
            xf.set_plain("fillId", &id.to_string());
            xf.set_plain("applyFill", "1");
        }
        if !p.borders.is_empty() {
            let borders = root.child(S, "borders").unwrap();
            let mut b = borders.elements().nth(get(&xf, "borderId")).cloned().unwrap_or_else(|| borders.sibling("border"));
            b.touch();
            for (edges, border) in &p.borders {
                let names: &[&str] = match edges {
                    Edges::All | Edges::Outline => &["left", "right", "top", "bottom"],
                    Edges::Top => &["top"],
                    Edges::Bottom => &["bottom"],
                    Edges::Left => &["left"],
                    Edges::Right => &["right"],
                    _ => &[],
                };
                for n in names {
                    let mut e = b.sibling(n);
                    let style = match border.style.as_str() {
                        "none" => "",
                        "double" => "double",
                        "dashed" => "dashed",
                        "dotted" => "dotted",
                        "thick" => "thick",
                        "medium" => "medium",
                        "hair" => "hair",
                        "dashdot" => "dashDot",
                        _ if border.width >= 2.0 => "thick",
                        _ if border.width >= 1.0 => "medium",
                        _ => "thin",
                    };
                    if !style.is_empty() {
                        e.set_plain("style", style);
                        let mut col = b.sibling("color");
                        match border.color.as_ref().and_then(|c| c.hex()) {
                            Some(h) => col.set_plain("rgb", &format!("FF{h}")),
                            None => col.set_plain("auto", "1"),
                        }
                        e.push(col);
                    }
                    put_in_order(&mut b, e, &["start", "left", "end", "right", "top", "bottom", "diagonal", "vertical", "horizontal"]);
                }
            }
            for n in ["left", "right", "top", "bottom", "diagonal"] {
                if b.child(S, n).is_none() {
                    let e = b.sibling(n);
                    put_in_order(&mut b, e, &["start", "left", "end", "right", "top", "bottom", "diagonal", "vertical", "horizontal"]);
                }
            }
            let id = add_unique(root.child_mut(S, "borders").unwrap(), b);
            xf.set_plain("borderId", &id.to_string());
            xf.set_plain("applyBorder", "1");
        }
        if let Some(code) = &p.number_format {
            let code = match code.to_ascii_lowercase().as_str() {
                "general" => "General".to_string(),
                "number" | "comma" => "#,##0".into(),
                "number2" | "decimal" => "#,##0.00".into(),
                "percent" | "%" => "0%".into(),
                "percent2" => "0.00%".into(),
                "date" => "yyyy-mm-dd".into(),
                "datetime" => "yyyy-mm-dd hh:mm".into(),
                "time" => "hh:mm".into(),
                "text" | "@" => "@".into(),
                "currency" | "won" | "krw" => "\"₩\"#,##0".into(),
                "usd" | "dollar" => "\"$\"#,##0.00".into(),
                _ => code.clone(),
            };
            // Built-in ids only for the formats that do not depend on the reader's locale.
            let id = (0..=49).find(|&i| numfmt::builtin(i) == Some(code.as_str()) && !(14..=36).contains(&i) && !(45..=47).contains(&i)).map(|i| i as u32).unwrap_or_else(|| {
                if root.child(S, "numFmts").is_none() {
                    let e = root.sibling("numFmts");
                    put_in_order(root, e, STYLESHEET_ORDER);
                }
                let nfs = root.child_mut(S, "numFmts").unwrap();
                if let Some(e) = nfs.elements().find(|e| e.own("formatCode") == Some(code.as_str())) {
                    return e.own("numFmtId").and_then(|v| v.parse().ok()).unwrap_or(0);
                }
                let next = nfs.elements().filter_map(|e| e.own("numFmtId").and_then(|v| v.parse::<u32>().ok())).max().unwrap_or(163).max(163) + 1;
                let mut e = nfs.sibling("numFmt");
                e.set_plain("numFmtId", &next.to_string());
                e.set_plain("formatCode", &code);
                nfs.push(e);
                let n = nfs.elements().count();
                nfs.set_plain("count", &n.to_string());
                next
            });
            xf.set_plain("numFmtId", &id.to_string());
            xf.set_plain("applyNumberFormat", "1");
        }
        if p.align.is_some() || p.valign.is_some() || p.wrap.is_some() || p.indent_level.is_some() || p.rotation.is_some() {
            let mut al = xf.child(S, "alignment").cloned().unwrap_or_else(|| xf.sibling("alignment"));
            al.touch();
            if let Some(a) = &p.align {
                al.set_plain("horizontal", match a.as_str() {
                    "justify" => "justify",
                    "distribute" => "distributed",
                    "fill" => "fill",
                    "general" => "general",
                    "center" => "center",
                    "right" => "right",
                    _ => "left",
                });
            }
            if let Some(v) = &p.valign {
                al.set_plain("vertical", match v.as_str() {
                    "middle" => "center",
                    "top" => "top",
                    _ => "bottom",
                });
            }
            if let Some(w) = p.wrap {
                if w {
                    al.set_plain("wrapText", "1");
                } else {
                    al.remove_own("wrapText");
                }
            }
            if let Some(i) = p.indent_level {
                al.set_plain("indent", &i.to_string());
            }
            if let Some(r) = p.rotation {
                let v = if r < 0.0 { 90.0 - r } else { r };
                al.set_plain("textRotation", &(v.round() as i64).to_string());
            }
            put_in_order(&mut xf, al, &["alignment", "protection", "extLst"]);
            xf.set_plain("applyAlignment", "1");
        }
        if let Some(lock) = p.protect {
            let mut pr = xf.sibling("protection");
            pr.set_plain("locked", if lock { "1" } else { "0" });
            put_in_order(&mut xf, pr, &["alignment", "protection", "extLst"]);
            xf.set_plain("applyProtection", "1");
        }
        if p.clear {
            for a in ["fillId", "borderId", "numFmtId"] {
                xf.set_plain(a, "0");
            }
            xf.remove_children(S, "alignment");
        }
        // The same format may exist already.
        let xfs_el = root.child_mut(S, "cellXfs").unwrap();
        let key = xf.to_xml();
        if let Some(i) = xfs_el.elements().position(|e| e.to_xml() == key) {
            return Ok(i as u32);
        }
        xfs_el.push(xf);
        let n = xfs_el.elements().count();
        xfs_el.set_plain("count", &n.to_string());
        Ok((n - 1) as u32)
    }

    fn set_column_width(&mut self, si: usize, c0: u32, c1: u32, width: f64) {
        let doc = self.sheets[si].doc.as_mut().unwrap();
        if doc.root.child(S, "cols").is_none() {
            let e = doc.root.sibling("cols");
            put_in_order(&mut doc.root, e, WORKSHEET_ORDER);
        }
        let cols = doc.root.child_mut(S, "cols").unwrap();
        // Split existing ranges around [c0, c1] and set the width on the pieces inside.
        let mut out: Vec<Element> = Vec::new();
        let (a, b) = (c0 + 1, c1 + 1);
        let mut covered: Vec<(u32, u32)> = Vec::new();
        for col in cols.elements() {
            let min: u32 = col.own("min").and_then(|v| v.parse().ok()).unwrap_or(1);
            let max: u32 = col.own("max").and_then(|v| v.parse().ok()).unwrap_or(min);
            if max < a || min > b {
                out.push(col.clone());
                continue;
            }
            if min < a {
                let mut l = col.clone();
                l.set_plain("max", &(a - 1).to_string());
                out.push(l);
            }
            let mut m = col.clone();
            let (ma, mb) = (min.max(a), max.min(b));
            m.set_plain("min", &ma.to_string());
            m.set_plain("max", &mb.to_string());
            m.set_plain("width", &fmt_num(width));
            m.set_plain("customWidth", "1");
            covered.push((ma, mb));
            out.push(m);
            if max > b {
                let mut r = col.clone();
                r.set_plain("min", &(b + 1).to_string());
                out.push(r);
            }
        }
        // Columns no range covered get their own.
        let mut gaps = Vec::new();
        let mut x = a;
        covered.sort();
        for (ma, mb) in &covered {
            if *ma > x {
                gaps.push((x, ma - 1));
            }
            x = mb + 1;
        }
        if x <= b {
            gaps.push((x, b));
        }
        for (ga, gb) in gaps {
            let mut e = cols.sibling("col");
            e.set_plain("min", &ga.to_string());
            e.set_plain("max", &gb.to_string());
            e.set_plain("width", &fmt_num(width));
            e.set_plain("customWidth", "1");
            out.push(e);
        }
        out.sort_by_key(|e| e.own("min").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0));
        cols.children = out.into_iter().map(Node::Element).collect();
    }

    fn set_row_height(&mut self, si: usize, r: u32, pt: f64) {
        // Through a cell-less path: make the row exist.
        let doc = self.sheets[si].doc.as_mut().unwrap();
        let has = doc.root.child(S, "sheetData").map(|d| d.children_named(S, "row").any(|row| row.own("r").and_then(|v| v.parse::<u32>().ok()) == Some(r + 1))).unwrap_or(false);
        if !has {
            // Make the row by making a cell, then remove the cell.
            self.cell_mut(si, r, 0);
            let doc = self.sheets[si].doc.as_mut().unwrap();
            let data = doc.root.child_mut(S, "sheetData").unwrap();
            for row in data.elements_mut() {
                if row.own("r").and_then(|v| v.parse::<u32>().ok()) == Some(r + 1) {
                    row.children.retain(|n| !n.as_element().map(|c| c.is(S, "c") && c.children.is_empty() && c.own("s").is_none()).unwrap_or(false));
                }
            }
        }
        let doc = self.sheets[si].doc.as_mut().unwrap();
        let data = doc.root.child_mut(S, "sheetData").unwrap();
        for row in data.elements_mut() {
            if row.own("r").and_then(|v| v.parse::<u32>().ok()) == Some(r + 1) {
                row.set_plain("ht", &fmt_num(pt));
                row.set_plain("customHeight", "1");
            }
        }
    }

    fn set_merge(&mut self, si: usize, area: &Area, on: bool) -> Result<()> {
        let doc = self.sheets[si].doc.as_mut().unwrap();
        let text = area_text_local(area);
        if on {
            if doc.root.child(S, "mergeCells").is_none() {
                let e = doc.root.sibling("mergeCells");
                put_in_order(&mut doc.root, e, WORKSHEET_ORDER);
            }
            let mc = doc.root.child_mut(S, "mergeCells").unwrap();
            // Overlapping merges are not allowed.
            for m in mc.children_named(S, "mergeCell") {
                if let Some(o) = m.own("ref").and_then(formula::parse_area) {
                    if overlaps(&o, area) {
                        return Err(err(format!("{} overlaps the merged cells {}", text, o.text())));
                    }
                }
            }
            let mut e = mc.sibling("mergeCell");
            e.set_plain("ref", &text);
            mc.push(e);
            let n = mc.elements().count();
            mc.set_plain("count", &n.to_string());
        } else if let Some(mc) = doc.root.child_mut(S, "mergeCells") {
            mc.children.retain(|n| !n.as_element().and_then(|e| e.own("ref")).and_then(formula::parse_area).map(|o| overlaps(&o, area)).unwrap_or(false));
            let n = mc.elements().count();
            if n == 0 {
                doc.root.remove_children(S, "mergeCells");
            } else {
                mc.set_plain("count", &n.to_string());
            }
        }
        Ok(())
    }

    // ── calculation ─────────────────────────────────────────────────────────────────────────

    /// Recalculates formulas: those that depend on changed cells (all with `all`), writing their
    /// cached values. Returns how many were updated.
    pub fn recalc(&mut self, all: bool) -> usize {
        if !all && self.changed.is_empty() {
            return 0;
        }
        let mut book = Book::new(self);
        // Formula cells and what they refer to.
        let mut formulas: Vec<(usize, u32, u32, Vec<(usize, Area)>)> = Vec::new();
        for si in 0..book.x.sheets.len() {
            for (&(r, c), raw) in book.raw(si).clone().iter() {
                if let Raw::Formula(f, _) = raw {
                    let refs = refs_of(f, si, book.x);
                    formulas.push((si, r, c, refs));
                }
            }
        }
        let mut dirty: HashSet<(usize, u32, u32)> = if all { formulas.iter().map(|f| (f.0, f.1, f.2)).collect() } else { HashSet::new() };
        if !all {
            let mut frontier: Vec<(usize, u32, u32)> = book.x.changed.iter().copied().collect();
            let mut seen: HashSet<(usize, u32, u32)> = frontier.iter().copied().collect();
            // Changed formula cells themselves.
            for f in &formulas {
                if seen.contains(&(f.0, f.1, f.2)) {
                    dirty.insert((f.0, f.1, f.2));
                }
            }
            while let Some((s, r, c)) = frontier.pop() {
                for f in &formulas {
                    if f.3.iter().any(|(fs, a)| *fs == s && in_area(a, r, c)) {
                        let k = (f.0, f.1, f.2);
                        dirty.insert(k);
                        if seen.insert(k) {
                            frontier.push(k);
                        }
                    }
                }
            }
        }
        book.invalidate(&dirty);
        let mut results = Vec::new();
        let mut keys: Vec<_> = dirty.iter().copied().collect();
        keys.sort();
        for (s, r, c) in keys {
            let v = book.value(s, r, c);
            results.push((s, r, c, v));
        }
        let unsupported = book.unsupported.clone();
        drop(book);
        let mut n = 0;
        for (s, r, c, v) in results {
            let v = v.first();
            if v == Val::Err(XlErr::Name) && !unsupported.is_empty() {
                self.warnings.push(format!("{}: uses a function this tool does not calculate ({}); its old value is kept, and Excel recalculates on open", self.addr_of(s, r, c), unsupported.iter().cloned().collect::<Vec<_>>().join(", ")));
                continue;
            }
            let cell = self.cell_mut(s, r, c);
            write_cached(cell, &v);
            n += 1;
        }
        // Excel and LibreOffice recalculate on open too.
        if n > 0 || all {
            let wb = &mut self.wb.root;
            let mut cp = wb.child(S, "calcPr").cloned().unwrap_or_else(|| wb.sibling("calcPr"));
            if cp.own("fullCalcOnLoad") != Some("1") {
                cp.set_plain("fullCalcOnLoad", "1");
                put_in_order(wb, cp, WORKBOOK_ORDER);
            }
        }
        self.changed.clear();
        n
    }

    pub fn save(&mut self) -> Result<Vec<u8>> {
        let n = self.recalc(false);
        let _ = n;
        // Charts read cells: their cached values follow the edits.
        crate::chart_edit::refresh_xlsx(self);
        // Cached values computed by formulas are stale without a calc chain update: drop the chain
        // when formulas were added (Excel rebuilds it).
        for sh in &self.sheets {
            if let Some(d) = &sh.doc {
                if let Some(dim) = d.root.child(S, "dimension") {
                    let _ = dim;
                }
                self.pkg.put_xml(&sh.part, d);
            }
        }
        // The dimension of each sheet covers its cells.
        for si in 0..self.sheets.len() {
            if self.sheets[si].doc.is_none() {
                continue;
            }
            let (er, ec) = self.extent(si);
            let first = self.cells(si).iter().map(|c| (c.row, c.col)).fold((u32::MAX, u32::MAX), |a, b| (a.0.min(b.0), a.1.min(b.1)));
            let d = self.sheets[si].doc.as_mut().unwrap();
            if let Some(dim) = d.root.child_mut(S, "dimension") {
                let want = if first.0 == u32::MAX { "A1".to_string() } else if (first.0, first.1) == (er, ec) { formula::cell_name(er, ec) } else { format!("{}:{}", formula::cell_name(first.0, first.1), formula::cell_name(er, ec)) };
                if let Some(cur) = dim.own("ref").and_then(formula::parse_area) {
                    let (r0, r1) = cur.rows();
                    let (c0, c1) = cur.cols();
                    if er > r1 || ec > c1 || first.0 < r0 || first.1 < c0 {
                        dim.set_plain("ref", &want);
                    }
                }
            }
            let part = self.sheets[si].part.clone();
            let d = self.sheets[si].doc.as_ref().unwrap();
            self.pkg.put_xml(&part, d);
        }
        if let (Some(p), Some(d)) = (&self.sst_part, &self.sst) {
            self.pkg.put_xml(p, d);
        }
        if let (Some(p), Some(d)) = (&self.styles_part, &self.styles) {
            self.pkg.put_xml(p, d);
        }
        self.pkg.put_xml(&self.wb_part, &self.wb);
        self.wb_rels.save(&mut self.pkg);
        // A calculation chain naming cells that are no longer formulas makes Excel repair the
        // file: drop it (it is optional and rebuilt on save).
        if let Some(cc) = self.wb_rels.first_part("calcChain") {
            if self.pkg.zip.changed_names().iter().any(|n| self.sheets.iter().any(|s| &s.part == n)) {
                let id = self.wb_rels.list.iter().find(|r| r.kind() == "calcChain").map(|r| r.id.clone());
                if let Some(id) = id {
                    self.wb_rels.remove(&id);
                    self.wb_rels.save(&mut self.pkg);
                    self.pkg.remove_part(&cc);
                }
            }
        }
        self.pkg.save()
    }
}

/// Replaces in a plain string, all matches (or the nth).
fn replace_all(f: &Finder, s: &str, with: &str, nth: Option<usize>) -> String {
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

/// Edits a string element (`si`, `is`): plain `t`, or runs through the rich text dialect.
fn edit_string_el(el: &mut Element, f: &Finder, with: &str, nth: Option<usize>) -> Result<()> {
    if el.child(S, "r").is_some() {
        let t = text::text_of(&Sml, el);
        let ms = f.find(&t, Some(with));
        for (i, (a, b, rep)) in ms.iter().enumerate().rev() {
            if nth.map(|n| n != i + 1).unwrap_or(false) {
                continue;
            }
            text::replace_range(&Sml, el, *a, *b, rep)?;
        }
        return Ok(());
    }
    if let Some(t) = el.child_mut(S, "t") {
        let cur = decode_x(&t.text());
        let new = replace_all(f, &cur, with, nth);
        t.set_text(&encode_x(&new));
        Sml.fix_space(t);
    }
    Ok(())
}

/// `_x000D_`-style escapes of SpreadsheetML strings.
pub fn decode_x(s: &str) -> String {
    if !s.contains("_x") {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("_x") {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.len() >= 7 && tail.as_bytes()[6] == b'_' {
            if let Ok(v) = u32::from_str_radix(&tail[2..6], 16) {
                if let Some(c) = char::from_u32(v) {
                    out.push(c);
                    rest = &tail[7..];
                    continue;
                }
            }
        }
        out.push_str("_x");
        rest = &tail[2..];
    }
    out.push_str(rest);
    out
}

pub fn encode_x(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' {
            out.push_str(&format!("_x{:04X}_", c as u32));
        } else {
            out.push(c);
        }
    }
    // A literal `_xHHHH_` would be read as an escape: escape its underscore.
    let re = regex::Regex::new(r"_(x[0-9A-Fa-f]{4}_)").unwrap();
    re.replace_all(&out, "_x005F_$1").into_owned()
}

fn set_v(cell: &mut Element, v: &str) {
    match cell.child_mut(S, "v") {
        Some(e) => e.set_text(v),
        None => {
            let mut e = cell.sibling("v");
            e.set_text(v);
            // After f, before is/extLst.
            let at = cell.children.iter().position(|n| n.as_element().map(|e| e.is(S, "is") || e.is(S, "extLst")).unwrap_or(false)).unwrap_or(cell.children.len());
            cell.children.insert(at, Node::Element(e));
        }
    }
}

fn write_cached(cell: &mut Element, v: &Val) {
    match v {
        Val::Num(n) => {
            cell.remove_own("t");
            set_v(cell, &num_text(*n));
        }
        Val::Str(s) => {
            cell.set_plain("t", "str");
            set_v(cell, &encode_x(s));
        }
        Val::Bool(b) => {
            cell.set_plain("t", "b");
            set_v(cell, if *b { "1" } else { "0" });
        }
        Val::Err(e) => {
            cell.set_plain("t", "e");
            set_v(cell, e.text());
        }
        Val::Empty => {
            cell.remove_own("t");
            set_v(cell, "0");
        }
        Val::Arr(_) => write_cached(cell, &v.clone().first()),
    }
}

/// A number as SpreadsheetML writes it: the shortest form that reads back the same.
pub fn num_text(n: f64) -> String {
    if n == n.trunc() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

fn fmt_num(v: f64) -> String {
    let s = format!("{:.4}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn in_area(a: &Area, r: u32, c: u32) -> bool {
    let (r0, r1) = a.rows();
    let (c0, c1) = a.cols();
    r >= r0 && r <= r1 && c >= c0 && c <= c1
}

fn overlaps(a: &Area, b: &Area) -> bool {
    let (ar0, ar1) = a.rows();
    let (ac0, ac1) = a.cols();
    let (br0, br1) = b.rows();
    let (bc0, bc1) = b.cols();
    ar0 <= br1 && br0 <= ar1 && ac0 <= bc1 && bc0 <= ac1
}

fn area_text_local(a: &Area) -> String {
    let mut x = a.clone();
    x.sheet = None;
    x.text()
}

/// Gives every row and cell an `r` (needed before inserting among them).
fn normalize_refs(data: &mut Element) {
    let needs = data.children_named(S, "row").any(|r| r.own("r").is_none() || r.children_named(S, "c").any(|c| c.own("r").is_none()));
    if !needs {
        return;
    }
    let mut next_row = 0u32;
    for row in data.elements_mut() {
        if !row.is(S, "row") {
            continue;
        }
        let r = row.own("r").and_then(|v| v.parse::<u32>().ok()).map(|v| v - 1).unwrap_or(next_row);
        next_row = r + 1;
        if row.own("r").is_none() {
            row.set_plain("r", &(r + 1).to_string());
        }
        let mut next_col = 0u32;
        for c in row.elements_mut() {
            if !c.is(S, "c") {
                continue;
            }
            let cc = c.own("r").and_then(formula::parse_cell).map(|x| x.col).unwrap_or(next_col);
            next_col = cc + 1;
            if c.own("r").is_none() {
                c.set_plain("r", &formula::cell_name(r, cc));
            }
        }
    }
}

/// Appends `e` to a style list unless an equal element is there; its index. Updates `count`.
fn add_unique(list: &mut Element, e: Element) -> usize {
    let key = e.to_xml();
    if let Some(i) = list.elements().position(|x| x.to_xml() == key) {
        return i;
    }
    list.push(e);
    let n = list.elements().count();
    list.set_plain("count", &n.to_string());
    n - 1
}

const FONT_ORDER: &[&str] = &["b", "i", "strike", "condense", "extend", "outline", "shadow", "u", "vertAlign", "sz", "color", "name", "rFont", "family", "charset", "scheme"];

/// Applies character properties to a SpreadsheetML font (`font`, or a rich text `rPr`).
fn apply_font(font: &mut Element, p: &Props, rich: bool) {
    let set_flag = |font: &mut Element, name: &str, on: bool| {
        if on {
            let e = font.sibling(name);
            put_in_order(font, e, FONT_ORDER);
        } else {
            font.remove_children(S, name);
        }
    };
    if let Some(b) = p.bold {
        set_flag(font, "b", b);
    }
    if let Some(i) = p.italic {
        set_flag(font, "i", i);
    }
    if let Some(s) = p.strike {
        set_flag(font, "strike", s);
    }
    if let Some(u) = &p.underline {
        if u == "none" {
            font.remove_children(S, "u");
        } else {
            let mut e = font.sibling("u");
            if u == "double" {
                e.set_plain("val", "double");
            }
            put_in_order(font, e, FONT_ORDER);
        }
    }
    if let Some(v) = &p.vert {
        match v {
            crate::props::Vert::Baseline => {
                font.remove_children(S, "vertAlign");
            }
            v => {
                let mut e = font.sibling("vertAlign");
                e.set_plain("val", if *v == crate::props::Vert::Super { "superscript" } else { "subscript" });
                put_in_order(font, e, FONT_ORDER);
            }
        }
    }
    if let Some(sz) = p.size {
        let mut e = font.sibling("sz");
        e.set_plain("val", &fmt_num(sz));
        put_in_order(font, e, FONT_ORDER);
    }
    if let Some(c) = &p.color {
        let mut e = font.sibling("color");
        match c {
            Color::Rgb(h) => e.set_plain("rgb", &format!("FF{h}")),
            _ => e.set_plain("theme", "1"),
        }
        put_in_order(font, e, FONT_ORDER);
    }
    if let Some(f) = p.font.as_ref().or(p.font_latin.as_ref()).or(p.font_ea.as_ref()) {
        let mut e = font.sibling(if rich { "rFont" } else { "name" });
        e.set_plain("val", f);
        // Another name or font scheme would win.
        font.remove_children(S, if rich { "name" } else { "rFont" });
        font.remove_children(S, "scheme");
        put_in_order(font, e, FONT_ORDER);
    }
}

/// A rich text run's properties from a cell font (`name` becomes `rFont`).
fn font_to_rpr(font: &Element, like: &Element) -> Element {
    let mut r = like.sibling("rPr");
    for c in font.elements() {
        let mut c2 = c.clone();
        if c.local() == "name" {
            let mut n = like.sibling("rFont");
            if let Some(v) = c.own("val") {
                n.set_plain("val", v);
            }
            c2 = n;
        }
        c2.touch();
        r.push(c2);
    }
    r
}

fn color_of(e: &Element) -> Option<String> {
    if let Some(rgb) = e.own("rgb") {
        return Some(if rgb.len() == 8 { rgb[2..].to_string() } else { rgb.to_string() });
    }
    if let Some(t) = e.own("theme") {
        return Some(format!("theme{t}"));
    }
    e.own("indexed").map(|i| format!("indexed{i}"))
}

fn sml_font_summary(font: &Element) -> Vec<String> {
    let mut v = Vec::new();
    for c in font.elements() {
        match c.local() {
            "b" if c.own("val") != Some("0") => v.push("bold".into()),
            "i" if c.own("val") != Some("0") => v.push("italic".into()),
            "strike" if c.own("val") != Some("0") => v.push("strike".into()),
            "u" => v.push(format!("underline={}", c.own("val").unwrap_or("single"))),
            "sz" => v.push(format!("size={}pt", c.own("val").unwrap_or("?"))),
            "color" => {
                if let Some(col) = color_of(c) {
                    if col != "000000" && col != "theme1" {
                        v.push(format!("color={col}"));
                    }
                }
            }
            "name" | "rFont" => v.push(format!("font={}", c.own("val").unwrap_or("?"))),
            "vertAlign" => v.push(c.own("val").unwrap_or("").replace("superscript", "super").replace("subscript", "sub")),
            _ => {}
        }
    }
    v
}

pub const WORKSHEET_ORDER: &[&str] = &[
    "sheetPr", "dimension", "sheetViews", "sheetFormatPr", "cols", "sheetData", "sheetCalcPr", "sheetProtection", "protectedRanges", "scenarios",
    "autoFilter", "sortState", "dataConsolidate", "customSheetViews", "mergeCells", "phoneticPr", "conditionalFormatting", "dataValidations",
    "hyperlinks", "printOptions", "pageMargins", "pageSetup", "headerFooter", "rowBreaks", "colBreaks", "customProperties", "cellWatches",
    "ignoredErrors", "smartTags", "drawing", "legacyDrawing", "legacyDrawingHF", "drawingHF", "picture", "oleObjects", "controls", "webPublishItems",
    "tableParts", "extLst",
];
const STYLESHEET_ORDER: &[&str] = &["numFmts", "fonts", "fills", "borders", "cellStyleXfs", "cellXfs", "cellStyles", "dxfs", "tableStyles", "colors", "extLst"];
const WORKBOOK_ORDER: &[&str] = &[
    "fileVersion", "fileSharing", "workbookPr", "workbookProtection", "bookViews", "sheets", "functionGroups", "externalReferences", "definedNames",
    "calcPr", "oleSize", "customWorkbookViews", "pivotCaches", "smartTagPr", "smartTagTypes", "webPublishing", "fileRecoveryPr", "webPublishObjects",
    "extLst",
];

const DEFAULT_STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><fonts count=\"1\"><font><sz val=\"11\"/><color theme=\"1\"/><name val=\"Calibri\"/><family val=\"2\"/><scheme val=\"minor\"/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>";

/// The areas a formula refers to, with their sheets (names expanded).
fn refs_of(f: &str, si: usize, x: &Xlsx) -> Vec<(usize, Area)> {
    let mut out = Vec::new();
    let Ok(toks) = formula::tokenize(f) else { return out };
    for t in toks {
        match t.kind {
            formula::TokKind::Area(a) => {
                let s = a.sheet.as_ref().and_then(|n| x.sheet_index(n)).unwrap_or(si);
                out.push((s, a));
            }
            formula::TokKind::Name(n) => {
                if let Some(def) = x.defined_name(&n, si) {
                    out.extend(refs_of(def.trim_start_matches('='), si, x));
                }
            }
            formula::TokKind::Func(name) => {
                // INDIRECT and OFFSET can reach anywhere: treat them as depending on everything.
                let n = formula::canonical_fn(&name);
                if n == "INDIRECT" || n == "OFFSET" {
                    for s in 0..x.sheets.len() {
                        out.push((s, Area { sheet: None, a: CellRef { row: 0, col: 0, abs_row: false, abs_col: false }, b: CellRef { row: formula::MAX_ROW, col: formula::MAX_COL, abs_row: false, abs_col: false }, whole: None }));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

impl Xlsx {
    pub fn defined_name(&self, name: &str, sheet: usize) -> Option<String> {
        let dn = self.wb.root.child(S, "definedNames")?;
        let mut global = None;
        for d in dn.children_named(S, "definedName") {
            if !d.own("name").map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false) {
                continue;
            }
            match d.own("localSheetId").and_then(|v| v.parse::<usize>().ok()) {
                Some(s) if s == sheet => return Some(d.text()),
                Some(_) => {}
                None => global = Some(d.text()),
            }
        }
        global
    }
}

/// The workbook as formulas see it.
struct Book<'a> {
    x: &'a Xlsx,
    raws: Vec<Option<HashMap<(u32, u32), Raw>>>,
    extents: Vec<Option<(u32, u32)>>,
    cache: HashMap<(usize, u32, u32), Val>,
    computing: HashSet<(usize, u32, u32)>,
    stack: Vec<(usize, u32, u32)>,
    unsupported: HashSet<String>,
    now: f64,
    dirty: Option<HashSet<(usize, u32, u32)>>,
}

impl<'a> Book<'a> {
    fn new(x: &'a Xlsx) -> Book<'a> {
        let n = x.sheets.len();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0) / 86400.0 + 25569.0;
        Book { x, raws: (0..n).map(|_| None).collect(), extents: vec![None; n], cache: HashMap::new(), computing: HashSet::new(), stack: Vec::new(), unsupported: HashSet::new(), now, dirty: None }
    }
    fn raw(&mut self, si: usize) -> &HashMap<(u32, u32), Raw> {
        if self.raws[si].is_none() {
            let m: HashMap<(u32, u32), Raw> = self.x.cells(si).into_iter().map(|c| ((c.row, c.col), c.raw)).collect();
            self.raws[si] = Some(m);
        }
        self.raws[si].as_ref().unwrap()
    }
    fn invalidate(&mut self, keys: &HashSet<(usize, u32, u32)>) {
        for k in keys {
            self.cache.remove(k);
        }
        // Cells not dirty keep their cached values: formulas there are not evaluated again.
        self.dirty = Some(keys.clone());
    }
    fn value(&mut self, si: usize, r: u32, c: u32) -> Val {
        if let Some(v) = self.cache.get(&(si, r, c)) {
            return v.clone();
        }
        let raw = self.raw(si).get(&(r, c)).cloned();
        let v = match raw {
            None => Val::Empty,
            Some(Raw::Value(v)) => v,
            Some(Raw::Formula(f, cached)) => {
                let dirty = self.dirty.as_ref().map(|d| d.contains(&(si, r, c))).unwrap_or(true);
                if !dirty {
                    cached
                } else if !self.computing.insert((si, r, c)) {
                    // A circular reference: Excel keeps 0.
                    Val::Num(0.0)
                } else {
                    self.stack.push((si, r, c));
                    let v = match formula::parse(&f) {
                        Ok(e) => {
                            note_unsupported(&e, &mut self.unsupported);
                            formula::eval(&e, self)
                        }
                        Err(_) => Val::Err(XlErr::Name),
                    };
                    self.stack.pop();
                    self.computing.remove(&(si, r, c));
                    v
                }
            }
        };
        let v = match v {
            Val::Arr(_) => v.first(),
            v => v,
        };
        self.cache.insert((si, r, c), v.clone());
        v
    }
}

fn note_unsupported(e: &formula::Expr, out: &mut HashSet<String>) {
    use formula::Expr;
    match e {
        Expr::Call(n, args) => {
            if !KNOWN_FUNCTIONS.contains(&n.as_str()) {
                out.insert(n.clone());
            }
            for a in args {
                note_unsupported(a, out);
            }
        }
        Expr::Neg(a) | Expr::Pct(a) => note_unsupported(a, out),
        Expr::Bin(_, a, b) | Expr::Range(a, b) => {
            note_unsupported(a, out);
            note_unsupported(b, out);
        }
        Expr::Array(rows) => {
            for r in rows {
                for a in r {
                    note_unsupported(a, out);
                }
            }
        }
        _ => {}
    }
}

pub const KNOWN_FUNCTIONS: &[&str] = &[
    "IF", "IFS", "IFERROR", "IFNA", "AND", "OR", "XOR", "NOT", "TRUE", "FALSE", "SWITCH", "CHOOSE", "SUM", "PRODUCT", "SUMSQ", "SUMPRODUCT", "ABS",
    "ROUND", "ROUNDUP", "ROUNDDOWN", "INT", "TRUNC", "MOD", "QUOTIENT", "POWER", "SQRT", "EXP", "LN", "LOG10", "LOG", "PI", "SIGN", "SIN", "COS",
    "TAN", "ASIN", "ACOS", "ATAN", "ATAN2", "DEGREES", "RADIANS", "CEILING", "CEILING.MATH", "CEILING.PRECISE", "FLOOR", "FLOOR.MATH",
    "FLOOR.PRECISE", "MROUND", "EVEN", "ODD", "FACT", "GCD", "LCM", "RAND", "RANDBETWEEN", "AVERAGE", "AVERAGEA", "MIN", "MAX", "MINA", "MAXA",
    "COUNT", "COUNTA", "COUNTBLANK", "MEDIAN", "MODE", "MODE.SNGL", "STDEV", "STDEV.S", "STDEVP", "STDEV.P", "VAR", "VAR.S", "VARP", "VAR.P",
    "STDEVA", "VARA", "LARGE", "SMALL", "RANK", "RANK.EQ", "RANK.AVG", "PERCENTILE", "PERCENTILE.INC", "QUARTILE", "QUARTILE.INC", "GEOMEAN",
    "CORREL", "PEARSON", "COUNTIF", "SUMIF", "AVERAGEIF", "COUNTIFS", "SUMIFS", "AVERAGEIFS", "MAXIFS", "MINIFS", "VLOOKUP", "HLOOKUP", "MATCH",
    "XMATCH", "XLOOKUP", "LOOKUP", "INDEX", "ROW", "COLUMN", "ROWS", "COLUMNS", "OFFSET", "INDIRECT", "ADDRESS", "TRANSPOSE", "LEN", "LEFT", "RIGHT",
    "MID", "UPPER", "LOWER", "PROPER", "TRIM", "CLEAN", "CONCATENATE", "CONCAT", "TEXTJOIN", "TEXT", "VALUE", "NUMBERVALUE", "SUBSTITUTE", "REPLACE",
    "FIND", "SEARCH", "REPT", "EXACT", "CHAR", "UNICHAR", "CODE", "UNICODE", "T", "N", "FIXED", "TEXTBEFORE", "TEXTAFTER", "DATE", "TIME",
    "DATEVALUE", "TIMEVALUE", "TODAY", "NOW", "YEAR", "MONTH", "DAY", "HOUR", "MINUTE", "SECOND", "WEEKDAY", "WEEKNUM", "ISOWEEKNUM", "EDATE",
    "EOMONTH", "DAYS", "DATEDIF", "NETWORKDAYS", "WORKDAY", "YEARFRAC", "ISBLANK", "ISNUMBER", "ISTEXT", "ISNONTEXT", "ISLOGICAL", "ISERROR",
    "ISERR", "ISNA", "ISEVEN", "ISODD", "ISREF", "ISFORMULA", "NA", "TYPE", "ERROR.TYPE", "PMT", "IPMT", "PPMT", "PV", "FV", "NPER", "NPV", "IRR",
    "RATE", "UNIQUE", "SORT", "FILTER", "SEQUENCE",
];

impl<'a> Ctx for Book<'a> {
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
        if let Some(e) = self.extents[si] {
            return e;
        }
        let raw = self.raw(si);
        let e = raw.keys().fold((0, 0), |a, k| (a.0.max(k.0), a.1.max(k.1)));
        self.extents[si] = Some(e);
        e
    }
    fn name(&mut self, name: &str) -> Option<String> {
        let cur = self.stack.last().map(|s| s.0).unwrap_or(0);
        self.x.defined_name(name, cur)
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
        self.now
    }
}
