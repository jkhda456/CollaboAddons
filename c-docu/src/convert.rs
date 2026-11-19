//! Converting the older binary formats to their Open XML forms: for `docu convert`, and to render
//! them (through the Open XML renderers).

use crate::formula::{self, Val};
use crate::xls::Xls;
use crate::xml::escape;
use crate::zip::Zip;
use crate::Result;
use std::collections::HashMap;
use std::fmt::Write;

const HDR: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const SML: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";

fn attr(s: &str) -> String {
    let mut o = String::new();
    crate::xml::escape_attr(s, &mut o);
    o
}

/// An .xls workbook as .xlsx bytes: sheets, values, formulas (with their cached results),
/// shared strings, cell formats (fonts, fills, borders, number formats, alignment), column
/// widths, row heights and merged cells.
pub fn xls_to_xlsx(x: &Xls) -> Result<Vec<u8>> {
    let mut z = Zip::new();
    let mut strings: Vec<String> = Vec::new();
    let mut string_ix: HashMap<String, usize> = HashMap::new();
    let mut sheets_xml = String::new();
    let mut rels = String::new();
    let mut overrides = String::new();
    let mut n = 0;
    for (si, sh) in x.sheets.iter().enumerate() {
        if !sh.worksheet {
            continue;
        }
        n += 1;
        let _ = write!(sheets_xml, "<sheet name=\"{}\" sheetId=\"{n}\"{} r:id=\"rId{n}\"/>", attr(&sh.name), if sh.hidden { " state=\"hidden\"" } else { "" });
        let _ = write!(rels, "<Relationship Id=\"rId{n}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{n}.xml\"/>");
        let _ = write!(overrides, "<Override PartName=\"/xl/worksheets/sheet{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>");
        let lay = x.layout(si);
        let mut w = format!("{HDR}<worksheet {SML}>");
        if let Some(dw) = lay.default_width {
            let _ = write!(w, "<sheetFormatPr defaultColWidth=\"{dw}\" defaultRowHeight=\"12.75\"/>");
        } else {
            w.push_str("<sheetFormatPr defaultRowHeight=\"12.75\"/>");
        }
        if !lay.cols.is_empty() {
            w.push_str("<cols>");
            let mut cols = lay.cols.clone();
            cols.sort_by_key(|c| (c.0, c.1));
            for (a, b, width, hidden) in cols {
                let _ = write!(w, "<col min=\"{}\" max=\"{}\" width=\"{width}\" customWidth=\"1\"{}/>", a + 1, (b + 1).min(16384), if hidden { " hidden=\"1\"" } else { "" });
            }
            w.push_str("</cols>");
        }
        w.push_str("<sheetData>");
        let mut by_row: std::collections::BTreeMap<u32, Vec<(u32, Val, Option<String>, u16)>> = Default::default();
        for (r, c, v, f, xf) in x.cells(si) {
            by_row.entry(r).or_default().push((c, v, f, xf));
        }
        let heights: HashMap<u16, (Option<f64>, bool)> = lay.rows.iter().map(|(r, h, hid)| (*r, (*h, *hid))).collect();
        let mut all_rows: Vec<u32> = by_row.keys().copied().collect();
        for (r, (h, hid)) in &heights {
            if (h.is_some() || *hid) && !by_row.contains_key(&(*r as u32)) {
                all_rows.push(*r as u32);
            }
        }
        all_rows.sort();
        all_rows.dedup();
        for r in all_rows {
            let mut extra = String::new();
            if let Some((h, hid)) = heights.get(&(r as u16)) {
                if let Some(h) = h {
                    let _ = write!(extra, " ht=\"{h}\" customHeight=\"1\"");
                }
                if *hid {
                    extra.push_str(" hidden=\"1\"");
                }
            }
            let _ = write!(w, "<row r=\"{}\"{extra}>", r + 1);
            for (c, v, f, xf) in by_row.get(&r).cloned().unwrap_or_default() {
                let refn = formula::cell_name(r, c);
                let s = if xf > 0 { format!(" s=\"{xf}\"") } else { String::new() };
                let fx = f.as_ref().filter(|f| !f.starts_with("#formula?")).map(|f| format!("<f>{}</f>", escape(&formula::to_file(f).unwrap_or_else(|_| f.clone()))));
                match (&v, fx) {
                    (Val::Str(t), None) => {
                        let i = *string_ix.entry(t.clone()).or_insert_with(|| {
                            strings.push(t.clone());
                            strings.len() - 1
                        });
                        let _ = write!(w, "<c r=\"{refn}\"{s} t=\"s\"><v>{i}</v></c>");
                    }
                    (Val::Str(t), Some(fx)) => {
                        let _ = write!(w, "<c r=\"{refn}\"{s} t=\"str\">{fx}<v>{}</v></c>", escape(t));
                    }
                    (Val::Num(nv), fx) => {
                        let _ = write!(w, "<c r=\"{refn}\"{s}>{}<v>{}</v></c>", fx.unwrap_or_default(), crate::xlsx::num_text(*nv));
                    }
                    (Val::Bool(b), fx) => {
                        let _ = write!(w, "<c r=\"{refn}\"{s} t=\"b\">{}<v>{}</v></c>", fx.unwrap_or_default(), *b as u8);
                    }
                    (Val::Err(e), fx) => {
                        let _ = write!(w, "<c r=\"{refn}\"{s} t=\"e\">{}<v>{}</v></c>", fx.unwrap_or_default(), e.text());
                    }
                    (_, Some(fx)) => {
                        let _ = write!(w, "<c r=\"{refn}\"{s}>{fx}</c>");
                    }
                    _ => {
                        let _ = write!(w, "<c r=\"{refn}\"{s}/>");
                    }
                }
            }
            w.push_str("</row>");
        }
        w.push_str("</sheetData>");
        if !lay.merges.is_empty() {
            let _ = write!(w, "<mergeCells count=\"{}\">", lay.merges.len());
            for (r0, r1, c0, c1) in &lay.merges {
                let _ = write!(w, "<mergeCell ref=\"{}:{}\"/>", formula::cell_name(*r0 as u32, *c0 as u32), formula::cell_name(*r1 as u32, *c1 as u32));
            }
            w.push_str("</mergeCells>");
        }
        w.push_str("</worksheet>");
        z.write(&format!("xl/worksheets/sheet{n}.xml"), w.into_bytes());
    }
    // Styles from the XF, FONT and FORMAT records.
    let mut st = format!("{HDR}<styleSheet {SML}>");
    let mut custom: Vec<(&u16, &String)> = x.formats.iter().filter(|(id, _)| **id >= 164 || crate::numfmt::builtin(**id as u32).is_none()).collect();
    custom.sort();
    if !custom.is_empty() {
        let _ = write!(st, "<numFmts count=\"{}\">", custom.len());
        for (id, code) in &custom {
            let _ = write!(st, "<numFmt numFmtId=\"{id}\" formatCode=\"{}\"/>", attr(code));
        }
        st.push_str("</numFmts>");
    }
    let color = |icv: u16| x.color(icv).map(|h| format!("<color rgb=\"FF{h}\"/>")).unwrap_or_default();
    let _ = write!(st, "<fonts count=\"{}\">", x.fonts.len().max(1));
    if x.fonts.is_empty() {
        st.push_str("<font><sz val=\"10\"/><name val=\"Arial\"/></font>");
    }
    for f in &x.fonts {
        let _ = write!(
            st,
            "<font>{}{}{}{}<sz val=\"{}\"/>{}<name val=\"{}\"/></font>",
            if f.bold { "<b/>" } else { "" },
            if f.italic { "<i/>" } else { "" },
            if f.strike { "<strike/>" } else { "" },
            if f.underline { "<u/>" } else { "" },
            f.size,
            if f.color != 0x7FFF { color(f.color) } else { String::new() },
            attr(&f.name)
        );
    }
    st.push_str("</fonts>");
    let mut fills: Vec<String> = vec!["<fill><patternFill patternType=\"none\"/></fill>".into(), "<fill><patternFill patternType=\"gray125\"/></fill>".into()];
    let mut borders: Vec<String> = vec!["<border><left/><right/><top/><bottom/><diagonal/></border>".into()];
    let mut xfs = String::new();
    for xf in &x.xfs {
        let fill_id = match xf.fill.and_then(|c| x.color(c)) {
            Some(h) => {
                let f = format!("<fill><patternFill patternType=\"solid\"><fgColor rgb=\"FF{h}\"/><bgColor indexed=\"64\"/></patternFill></fill>");
                match fills.iter().position(|x| *x == f) {
                    Some(i) => i,
                    None => {
                        fills.push(f);
                        fills.len() - 1
                    }
                }
            }
            None => 0,
        };
        let line = |code: u8| match code {
            1 => "thin",
            2 => "medium",
            3 => "dashed",
            4 => "dotted",
            5 => "thick",
            6 => "double",
            7 => "hair",
            8 => "mediumDashed",
            9 => "dashDot",
            10 => "mediumDashDot",
            11 => "dashDotDot",
            12 => "mediumDashDotDot",
            13 => "slantDashDot",
            _ => "",
        };
        let side = |name: &str, k: usize| {
            let l = line(xf.borders[k]);
            if l.is_empty() {
                format!("<{name}/>")
            } else {
                format!("<{name} style=\"{l}\">{}</{name}>", color(xf.border_colors[k]))
            }
        };
        let b = format!("<border>{}{}{}{}<diagonal/></border>", side("left", 3), side("right", 1), side("top", 0), side("bottom", 2));
        let border_id = match borders.iter().position(|x| *x == b) {
            Some(i) => i,
            None => {
                borders.push(b);
                borders.len() - 1
            }
        };
        let font_id = if xf.font >= 4 { xf.font - 1 } else { xf.font };
        let h = ["general", "left", "center", "right", "fill", "justify", "centerContinuous", "distributed"].get(xf.halign as usize).copied().unwrap_or("general");
        let v = ["top", "center", "bottom", "justify", "distributed"].get(xf.valign as usize).copied().unwrap_or("bottom");
        let _ = write!(
            xfs,
            "<xf numFmtId=\"{}\" fontId=\"{font_id}\" fillId=\"{fill_id}\" borderId=\"{border_id}\" xfId=\"0\"><alignment horizontal=\"{h}\" vertical=\"{v}\"{}{}{}/></xf>",
            xf.fmt,
            if xf.wrap { " wrapText=\"1\"" } else { "" },
            if xf.indent > 0 { format!(" indent=\"{}\"", xf.indent) } else { String::new() },
            if xf.rotation > 0 { format!(" textRotation=\"{}\"", xf.rotation) } else { String::new() }
        );
    }
    if x.xfs.is_empty() {
        xfs.push_str("<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>");
    }
    let _ = write!(st, "<fills count=\"{}\">{}</fills>", fills.len(), fills.concat());
    let _ = write!(st, "<borders count=\"{}\">{}</borders>", borders.len(), borders.concat());
    st.push_str("<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>");
    let _ = write!(st, "<cellXfs count=\"{}\">{xfs}</cellXfs>", x.xfs.len().max(1));
    st.push_str("<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>");
    z.write("xl/styles.xml", st.into_bytes());
    let mut sst = format!("{HDR}<sst {SML} count=\"{}\" uniqueCount=\"{}\">", strings.len(), strings.len());
    for s in &strings {
        let _ = write!(sst, "<si><t xml:space=\"preserve\">{}</t></si>", escape(&crate::xlsx::encode_x(s)));
    }
    sst.push_str("</sst>");
    z.write("xl/sharedStrings.xml", sst.into_bytes());
    let k = n + 1;
    let _ = write!(rels, "<Relationship Id=\"rId{k}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings\" Target=\"sharedStrings.xml\"/>", k + 1);
    z.write("xl/_rels/workbook.xml.rels", format!("{HDR}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rels}</Relationships>").into_bytes());
    let date = if x.date1904 { "<workbookPr date1904=\"1\"/>" } else { "<workbookPr/>" };
    z.write("xl/workbook.xml", format!("{HDR}<workbook {SML}>{date}<sheets>{sheets_xml}</sheets><calcPr calcId=\"0\" fullCalcOnLoad=\"1\"/></workbook>").into_bytes());
    z.write("_rels/.rels", format!("{HDR}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>").into_bytes());
    z.write(
        "[Content_Types].xml",
        format!("{HDR}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>{overrides}<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/><Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/></Types>").into_bytes(),
    );
    // [Content_Types].xml first, as Office writes it.
    let mut ordered = Zip::new();
    let names = z.names();
    for nm in ["[Content_Types].xml", "_rels/.rels"].iter().map(|s| s.to_string()).chain(names.into_iter()) {
        if !ordered.contains(&nm) {
            ordered.write(&nm, z.read(&nm)?);
        }
    }
    ordered.save()
}

/// A .doc as .docx bytes.
pub fn doc_to_docx(d: &crate::word97::Doc) -> Result<Vec<u8>> {
    crate::word97_docx::Conv::new(d).docx()
}

/// The document as Markdown: headings from heading styles, paragraphs, tables as pipe tables,
/// cells of sheets as tables, slides as sections.
pub fn markdown(d: &crate::doc::AnyDoc) -> Result<String> {
    use crate::doc::AnyDoc;
    let o = d.outline(false, &[])?;
    let mut out = String::new();
    match d {
        AnyDoc::Xlsx(_) | AnyDoc::Xls(_) => {
            for sheet in d.plain()?.split("== ").filter(|s| !s.trim().is_empty()) {
                let mut lines = sheet.lines();
                let name = lines.next().unwrap_or("");
                let _ = writeln!(out, "## {name}\n");
                let rows: Vec<Vec<&str>> = lines.map(|l| l.split('\t').collect()).collect();
                let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
                for (i, r) in rows.iter().enumerate() {
                    let cells: Vec<String> = (0..cols).map(|k| r.get(k).copied().unwrap_or("").replace('|', "\\|")).collect();
                    let _ = writeln!(out, "| {} |", cells.join(" | "));
                    if i == 0 {
                        let _ = writeln!(out, "|{}|", vec!["---"; cols].join("|"));
                    }
                }
                out.push('\n');
            }
        }
        _ => {
            let mut table: Vec<(String, String)> = Vec::new();
            let flush = |table: &mut Vec<(String, String)>, out: &mut String| {
                if table.is_empty() {
                    return;
                }
                // Rows from the cells' places (t1.r2.c3).
                let mut rows: std::collections::BTreeMap<usize, Vec<String>> = Default::default();
                for (place, text) in table.iter() {
                    let parts: Vec<usize> = place.split('.').filter_map(|p| p.get(1..).and_then(|n| n.parse().ok())).collect();
                    if parts.len() >= 3 {
                        let row = rows.entry(parts[1]).or_default();
                        while row.len() < parts[2] {
                            row.push(String::new());
                        }
                        let cell = &mut row[parts[2] - 1];
                        if !cell.is_empty() {
                            cell.push_str("<br>");
                        }
                        cell.push_str(&text.replace('|', "\\|"));
                    }
                }
                let cols = rows.values().map(|r| r.len()).max().unwrap_or(0);
                for (i, r) in rows.values().enumerate() {
                    let cells: Vec<String> = (0..cols).map(|k| r.get(k).cloned().unwrap_or_default()).collect();
                    let _ = writeln!(out, "| {} |", cells.join(" | "));
                    if i == 0 {
                        let _ = writeln!(out, "|{}|", vec!["---"; cols].join("|"));
                    }
                }
                out.push('\n');
                table.clear();
            };
            for it in &o.items {
                if let Some(place) = it.place.as_ref().filter(|p| p.contains(".r")) {
                    table.push((place.clone(), it.text.clone()));
                    continue;
                }
                flush(&mut table, &mut out);
                match it.kind.as_str() {
                    "slide" => {
                        let _ = writeln!(out, "\n---\n\n<!-- {} -->\n", it.addr);
                    }
                    "story" => {
                        let _ = writeln!(out, "\n<!-- {} -->\n", it.text);
                    }
                    "p" => {
                        let t = it.text.replace('\u{FFFC}', "").replace('\n', "  \n");
                        if t.trim().is_empty() {
                            continue;
                        }
                        let style = it.style.as_deref().unwrap_or("").to_lowercase();
                        let level = if style == "title" {
                            Some(1)
                        } else if let Some(n) = style.strip_prefix("heading").and_then(|n| n.trim().parse::<usize>().ok()) {
                            Some(n + 1)
                        } else if style.contains("title") && it.addr.contains("sh1") {
                            Some(2)
                        } else {
                            None
                        };
                        let list = it.extra.iter().any(|(k, _)| k == "list");
                        match level {
                            Some(l) => {
                                let _ = writeln!(out, "{} {t}\n", "#".repeat(l.min(6)));
                            }
                            None if list => {
                                let _ = writeln!(out, "- {t}");
                            }
                            None => {
                                let _ = writeln!(out, "{t}\n");
                            }
                        }
                    }
                    _ => {}
                }
            }
            flush(&mut table, &mut out);
        }
    }
    Ok(out)
}

/// A sheet as CSV (the first, or the one named).
pub fn csv(d: &crate::doc::AnyDoc, sheet: Option<&str>) -> Result<String> {
    let text = d.plain()?;
    let mut out = String::new();
    for block in text.split("== ").filter(|s| !s.trim().is_empty()) {
        let mut lines = block.lines();
        let name = lines.next().unwrap_or("");
        if let Some(s) = sheet {
            if !name.eq_ignore_ascii_case(s) {
                continue;
            }
        }
        for l in lines {
            let cells: Vec<String> = l
                .split('\t')
                .map(|c| if c.contains([',', '"', '\n']) { format!("\"{}\"", c.replace('"', "\"\"")) } else { c.to_string() })
                .collect();
            out.push_str(&cells.join(","));
            out.push('\n');
        }
        break;
    }
    Ok(out)
}
