//! A .ppt as a .pptx: each slide's shapes (with the master's own) as DrawingML shapes at their
//! anchors, with their geometry type, fill and line (scheme colors resolved), pictures from the
//! picture store, and text whose runs carry every property resolved against the master's text
//! styles — for rendering and for `docu convert`.

use crate::ppt::{parse_style, text_of_atom, Ppt, Rec};
use crate::xml::escape;
use crate::zip::Zip;
use crate::Result;
use std::collections::HashMap;
use std::fmt::Write;

fn u16_at(d: &[u8], i: usize) -> u16 {
    d.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0)
}
fn i16_at(d: &[u8], i: usize) -> i16 {
    u16_at(d, i) as i16
}
fn u32_at(d: &[u8], i: usize) -> u32 {
    d.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0)
}

/// Master units (576 per inch) to EMU.
fn emu(v: i32) -> i64 {
    (v as i64) * 1587
}

fn attr(s: &str) -> String {
    let mut o = String::new();
    crate::xml::escape_attr(s, &mut o);
    o
}

#[derive(Clone, Default, Debug)]
struct Cf {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    font: Option<u16>,
    size: Option<u16>,
    color: Option<[u8; 4]>,
    position: Option<i16>,
}

#[derive(Clone, Default, Debug)]
struct Pf {
    bullet: Option<bool>,
    bullet_char: Option<u16>,
    bullet_font: Option<u16>,
    bullet_color: Option<[u8; 4]>,
    bullet_size: Option<i16>,
    align: Option<u16>,
    line: Option<i16>,
    before: Option<i16>,
    after: Option<i16>,
    margin: Option<i16>,
    indent: Option<i16>,
}

fn read_cf(d: &[u8]) -> Cf {
    let m = u32_at(d, 0);
    let mut c = Cf::default();
    let mut at = 4;
    if m & 0x3EB7 != 0 {
        let f = u16_at(d, at);
        if m & 1 != 0 {
            c.bold = Some(f & 1 != 0);
        }
        if m & 2 != 0 {
            c.italic = Some(f & 2 != 0);
        }
        if m & 4 != 0 {
            c.underline = Some(f & 4 != 0);
        }
        at += 2;
    }
    if m & 0x10000 != 0 {
        c.font = Some(u16_at(d, at));
        at += 2;
    }
    for bit in [0x200000, 0x400000, 0x800000] {
        if m & bit != 0 {
            at += 2;
        }
    }
    if m & 0x20000 != 0 {
        c.size = Some(u16_at(d, at));
        at += 2;
    }
    if m & 0x40000 != 0 {
        c.color = d.get(at..at + 4).map(|b| [b[0], b[1], b[2], b[3]]);
        at += 4;
    }
    if m & 0x80000 != 0 {
        c.position = Some(i16_at(d, at));
    }
    c
}

fn read_pf(d: &[u8]) -> Pf {
    let m = u32_at(d, 0);
    let mut p = Pf::default();
    let mut at = 4;
    if m & 0xF != 0 {
        let f = u16_at(d, at);
        if m & 1 != 0 {
            p.bullet = Some(f & 1 != 0);
        }
        at += 2;
    }
    if m & 0x80 != 0 {
        p.bullet_char = Some(u16_at(d, at));
        at += 2;
    }
    if m & 0x10 != 0 {
        p.bullet_font = Some(u16_at(d, at));
        at += 2;
    }
    if m & 0x40 != 0 {
        p.bullet_size = Some(i16_at(d, at));
        at += 2;
    }
    if m & 0x20 != 0 {
        p.bullet_color = d.get(at..at + 4).map(|b| [b[0], b[1], b[2], b[3]]);
        at += 4;
    }
    if m & 0x800 != 0 {
        p.align = Some(u16_at(d, at));
        at += 2;
    }
    if m & 0x1000 != 0 {
        p.line = Some(i16_at(d, at));
        at += 2;
    }
    if m & 0x2000 != 0 {
        p.before = Some(i16_at(d, at));
        at += 2;
    }
    if m & 0x4000 != 0 {
        p.after = Some(i16_at(d, at));
        at += 2;
    }
    if m & 0x100 != 0 {
        p.margin = Some(i16_at(d, at));
        at += 2;
    }
    if m & 0x400 != 0 {
        p.indent = Some(i16_at(d, at));
    }
    p
}

impl Cf {
    fn over(&self, base: &Cf) -> Cf {
        Cf {
            bold: self.bold.or(base.bold),
            italic: self.italic.or(base.italic),
            underline: self.underline.or(base.underline),
            font: self.font.or(base.font),
            size: self.size.or(base.size),
            color: self.color.or(base.color),
            position: self.position.or(base.position),
        }
    }
}

impl Pf {
    fn over(&self, base: &Pf) -> Pf {
        Pf {
            bullet: self.bullet.or(base.bullet),
            bullet_char: self.bullet_char.or(base.bullet_char),
            bullet_font: self.bullet_font.or(base.bullet_font),
            bullet_color: self.bullet_color.or(base.bullet_color),
            bullet_size: self.bullet_size.or(base.bullet_size),
            align: self.align.or(base.align),
            line: self.line.or(base.line),
            before: self.before.or(base.before),
            after: self.after.or(base.after),
            margin: self.margin.or(base.margin),
            indent: self.indent.or(base.indent),
        }
    }
}

/// The master text styles: text type → levels of (paragraph, character) defaults.
fn master_styles(recs: &[&Rec]) -> HashMap<u16, Vec<(Pf, Cf)>> {
    let mut out = HashMap::new();
    for r in recs.iter().flat_map(|m| m.kids.iter()).filter(|r| r.typ == 0x0FA3) {
        let inst = r.ver_inst >> 4;
        let d = &r.data;
        let n = u16_at(d, 0) as usize;
        let mut at = 2;
        let mut levels = Vec::new();
        for _ in 0..n.min(5) {
            if inst >= 5 {
                at += 2;
            }
            let ps = crate::ppt::pf_size(d, at);
            let pf = read_pf(&d[at..(at + ps).min(d.len())]);
            at += ps;
            let cs = crate::ppt::cf_size(d, at);
            let cf = read_cf(&d[at..(at + cs).min(d.len())]);
            at += cs;
            levels.push((pf, cf));
        }
        out.insert(inst, levels);
    }
    out
}

/// The scheme colors of a slide (or its master): background, text, shadow, title, fill,
/// accent, accent and hyperlink, accent and followed hyperlink.
fn scheme(r: &Rec) -> Option<Vec<String>> {
    let s = r.kids.iter().find(|k| k.typ == 0x07F0)?;
    Some((0..8).map(|i| format!("{:02X}{:02X}{:02X}", s.data[i * 4], s.data[i * 4 + 1], s.data[i * 4 + 2])).collect())
}

fn color_of(c: [u8; 4], scheme: &[String]) -> Option<String> {
    match c[3] {
        0xFE | 0xFF => Some(format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2])),
        i if (i as usize) < scheme.len() => Some(scheme[i as usize].clone()),
        _ => None,
    }
}

/// OfficeArt property → (value, complex data).
fn opt(sp: &Rec) -> HashMap<u16, u32> {
    let mut m = HashMap::new();
    for o in sp.kids.iter().filter(|k| k.typ == 0xF00B || k.typ == 0xF122) {
        let n = (o.ver_inst >> 4) as usize;
        for k in 0..n {
            let id = u16_at(&o.data, k * 6) & 0x3FFF;
            m.insert(id, u32_at(&o.data, k * 6 + 2));
        }
    }
    m
}

fn art_color(v: u32, scheme: &[String]) -> Option<String> {
    let b = v.to_le_bytes();
    if b[3] & 0x08 != 0 {
        return scheme.get(b[0] as usize).cloned();
    }
    if b[3] & 0x10 != 0 {
        return None;
    }
    Some(format!("{:02X}{:02X}{:02X}", b[0], b[1], b[2]))
}

fn preset(t: u16) -> &'static str {
    match t {
        1 => "rect",
        2 => "roundRect",
        3 => "ellipse",
        4 => "diamond",
        5 => "triangle",
        6 => "rtTriangle",
        7 => "parallelogram",
        8 => "trapezoid",
        9 => "hexagon",
        10 => "octagon",
        11 => "plus",
        12 => "star5",
        13 => "rightArrow",
        15 => "homePlate",
        16 => "cube",
        20 | 32 => "line",
        21 => "plaque",
        22 => "can",
        23 => "donut",
        33 | 34 => "bentConnector3",
        37 | 38 => "curvedConnector3",
        55 => "chevron",
        56 => "pentagon",
        66 => "leftArrow",
        67 => "downArrow",
        68 => "upArrow",
        69 => "leftRightArrow",
        70 => "upDownArrow",
        73 => "lightningBolt",
        74 => "heart",
        96 => "smileyFace",
        _ => "rect",
    }
}

pub fn ppt_to_pptx(p: &Ppt) -> Result<Vec<u8>> {
    // Document: slide size, fonts, pictures.
    let doc = p.tops.iter().find(|r| r.typ == 0x03E8);
    let (sw, sh) = doc.and_then(|d| d.kids.iter().find(|k| k.typ == 0x03E9)).map(|a| (u32_at(&a.data, 0) as i32, u32_at(&a.data, 4) as i32)).unwrap_or((5760, 4320));
    let mut fonts: Vec<String> = Vec::new();
    if let Some(env) = doc.and_then(|d| d.kids.iter().find(|k| k.typ == 0x03F2)) {
        if let Some(fc) = env.kids.iter().find(|k| k.typ == 0x07D5) {
            for f in fc.kids.iter().filter(|k| k.typ == 0x0FB7) {
                let units: Vec<u16> = (0..32).map(|i| u16_at(&f.data, i * 2)).take_while(|u| *u != 0).collect();
                fonts.push(String::from_utf16_lossy(&units));
            }
        }
    }
    // The picture store: BLIPs by index (1-based pib).
    let mut blips: Vec<Option<(&'static str, Vec<u8>)>> = Vec::new();
    let pictures = p.stream("Pictures").unwrap_or(&[]);
    if let Some(dgg) = doc.and_then(|d| d.kids.iter().find(|k| k.typ == 0x040B)).and_then(|g| g.kids.iter().find(|k| k.typ == 0xF000)) {
        if let Some(bstore) = dgg.kids.iter().find(|k| k.typ == 0xF001) {
            for bse in &bstore.kids {
                let fo = u32_at(&bse.data, 28) as usize;
                let size = u32_at(&bse.data, 20) as usize;
                let b = if bse.data.len() > 36 {
                    crate::word97_docx::find_blip(&bse.data[36..])
                } else {
                    pictures.get(fo..(fo + size).min(pictures.len())).and_then(crate::word97_docx::find_blip)
                };
                blips.push(b);
            }
        }
    }
    let mut z = Zip::new();
    let mut slide_list = String::new();
    let mut pres_rels = String::from("<Relationship Id=\"rIdMaster\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>");
    let mut ct = String::new();
    let mut media_n = 0;
    let texts = p.texts.clone();
    for (si, (psr, _)) in p.slides.iter().enumerate() {
        let Some(top) = p.top_of(*psr) else { continue };
        let slide = &p.tops[top];
        let master_top = p.master_of(top);
        let master = master_top.map(|m| &p.tops[m]);
        // The document's styles (the "other" text's) under the master's.
        let env = doc.and_then(|d| d.kids.iter().find(|k| k.typ == 0x03F2));
        let styles = master_styles(&env.into_iter().chain(master).collect::<Vec<_>>());
        let colors = scheme(slide).or_else(|| master.and_then(scheme)).unwrap_or_else(|| ["FFFFFF", "000000", "808080", "000000", "BBE0E3", "333399", "009999", "99CC00"].iter().map(|s| s.to_string()).collect());
        let mut rels = String::from("<Relationship Id=\"rIdLayout\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>");
        let mut shapes = String::new();
        let mut id = 2;
        let ctx = Ctx { fonts: &fonts, colors: &colors, styles: &styles };
        // The master's own drawings first (unless the slide hides them).
        let follow_master = slide.kids.iter().find(|k| k.typ == 0x03EF).map(|a| u16_at(&a.data, 20) & 0x1 != 0).unwrap_or(true);
        let mut bg = None;
        for (layer, rec) in [(0, master), (1, Some(slide))] {
            let Some(rec) = rec else { continue };
            if layer == 0 && !follow_master {
                continue;
            }
            let Some(dg) = rec.kids.iter().find(|k| k.typ == 0x040C).and_then(|d| d.kids.iter().find(|k| k.typ == 0xF002)) else { continue };
            let mut shape_n = 0;
            walk(dg, &mut |sp: &Rec| {
                let fsp = sp.kids.iter().find(|k| k.typ == 0xF00A);
                let flags = fsp.map(|f| u32_at(&f.data, 4)).unwrap_or(0);
                let kind = fsp.map(|f| f.ver_inst >> 4).unwrap_or(0);
                let o = opt(sp);
                if flags & 0x1 != 0 {
                    // A group's own record (counted as ppt.rs counts shapes: not at all).
                    return;
                }
                shape_n += 1;
                if flags & 0x400 != 0 {
                    // The background shape.
                    if layer == 1 || bg.is_none() {
                        bg = o.get(&0x0181).and_then(|v| art_color(*v, &colors));
                    }
                    return;
                }
                // Placeholders on the master are templates, not drawings.
                let is_ph = sp.kids.iter().find(|k| k.typ == 0xF011).map(|cd| cd.kids.iter().any(|x| x.typ == 0x0BC3)).unwrap_or(false);
                if layer == 0 && is_ph {
                    return;
                }
                let anchor = sp.kids.iter().find(|k| k.typ == 0xF010 || k.typ == 0xF00F);
                let Some(a) = anchor else { return };
                let (x, y, w, h) = if a.data.len() >= 16 && a.typ == 0xF00F {
                    let (l, t, r, b) = (u32_at(&a.data, 0) as i32, u32_at(&a.data, 4) as i32, u32_at(&a.data, 8) as i32, u32_at(&a.data, 12) as i32);
                    (l, t, r - l, b - t)
                } else if a.data.len() >= 16 {
                    let (t, l, r, b) = (u32_at(&a.data, 0) as i32, u32_at(&a.data, 4) as i32, u32_at(&a.data, 8) as i32, u32_at(&a.data, 12) as i32);
                    (l, t, r - l, b - t)
                } else {
                    let (t, l, r, b) = (i16_at(&a.data, 0) as i32, i16_at(&a.data, 2) as i32, i16_at(&a.data, 4) as i32, i16_at(&a.data, 6) as i32);
                    (l, t, r - l, b - t)
                };
                let xf = format!("<a:xfrm{}{}><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>", if flags & 0x40 != 0 { " flipH=\"1\"" } else { "" }, if flags & 0x80 != 0 { " flipV=\"1\"" } else { "" }, emu(x), emu(y), emu(w.max(0)), emu(h.max(0)));
                id += 1;
                // A picture.
                if let Some(pib) = o.get(&0x0104) {
                    if let Some(Some((ext, bytes))) = blips.get((*pib as usize).wrapping_sub(1)) {
                        media_n += 1;
                        let name = format!("image{media_n}.{ext}");
                        z.write(&format!("ppt/media/{name}"), bytes.clone());
                        let rid = format!("rIdImg{media_n}");
                        let _ = write!(rels, "<Relationship Id=\"{rid}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"../media/{name}\"/>");
                        let _ = write!(shapes, "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"Picture {id}\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{xf}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>");
                        return;
                    }
                }
                // Fill and line.
                // Placeholders and text boxes are unfilled and unlined unless they say otherwise.
                let plain = is_ph || kind == 202 || kind == 0;
                let fill_on = o.get(&0x01BF).filter(|v| *v & 0x0010_0000 != 0).map(|v| v & 0x10 != 0).unwrap_or(!plain && kind != 20 && kind != 32);
                let line_on = o.get(&0x01FF).filter(|v| *v & 0x0008_0000 != 0).map(|v| v & 0x08 != 0).unwrap_or(!plain);
                let fill = if fill_on { o.get(&0x0181).and_then(|v| art_color(*v, &colors)).or_else(|| colors.get(4).cloned()) } else { None };
                let line = if line_on { o.get(&0x01C0).and_then(|v| art_color(*v, &colors)).or_else(|| colors.get(1).cloned()) } else { None };
                let lw = o.get(&0x01CB).copied().unwrap_or(9525);
                let mut sppr = format!("{xf}<a:prstGeom prst=\"{}\"><a:avLst/></a:prstGeom>", preset(kind));
                match &fill {
                    Some(c) => {
                        let _ = write!(sppr, "<a:solidFill><a:srgbClr val=\"{c}\"/></a:solidFill>");
                    }
                    None => sppr.push_str("<a:noFill/>"),
                }
                match &line {
                    Some(c) => {
                        let _ = write!(sppr, "<a:ln w=\"{lw}\"><a:solidFill><a:srgbClr val=\"{c}\"/></a:solidFill></a:ln>");
                    }
                    None => sppr.push_str("<a:ln><a:noFill/></a:ln>"),
                }
                // Text: the shape's own atoms or the slide's outline text it refers to.
                let txt = if layer == 1 { texts.iter().find(|t| t.slide == si && !t.notes && t.shape == Some(shape_n)).cloned() } else { None };
                let inline_tb = sp.kids.iter().find(|k| k.typ == 0xF00D);
                let mut body = String::new();
                let ins = |k: u16, d: i64| o.get(&k).map(|v| *v as i64).unwrap_or(d);
                let anchor_v = o.get(&0x0087).copied().unwrap_or(0);
                let anchor_t = match anchor_v {
                    1 | 4 => "ctr",
                    2 | 5 | 7 | 9 => "b",
                    _ => "t",
                };
                let fit = o.get(&0x00BF).map(|v| v & 0x0002_0000 != 0 && v & 0x2 != 0).unwrap_or(false);
                let bodypr = format!(
                    "<a:bodyPr lIns=\"{}\" tIns=\"{}\" rIns=\"{}\" bIns=\"{}\" anchor=\"{anchor_t}\"{} wrap=\"{}\">{}</a:bodyPr>",
                    ins(0x0081, 91440),
                    ins(0x0082, 45720),
                    ins(0x0083, 91440),
                    ins(0x0084, 45720),
                    if matches!(anchor_v, 3 | 4 | 5 | 8 | 9) { " anchorCtr=\"1\"" } else { "" },
                    if o.get(&0x0085).copied() == Some(2) { "none" } else { "square" },
                    if fit { "<a:spAutoFit/>" } else { "" }
                );
                let (atom, style, ttype): (Option<Rec>, Option<Rec>, u32) = match &txt {
                    Some(t) => (Some(p.atom(t).clone()), p.style_atom(t).cloned(), t.text_type),
                    None => match inline_tb {
                        Some(tb) => {
                            let a = tb.kids.iter().find(|k| k.typ == 0x0FA0 || k.typ == 0x0FA8).cloned();
                            let st = tb.kids.iter().find(|k| k.typ == 0x0FA1).cloned();
                            let tt = tb.kids.iter().find(|k| k.typ == 0x0F9F).map(|h| u32_at(&h.data, 0)).unwrap_or(4);
                            (a, st, tt)
                        }
                        None => (None, None, 4),
                    },
                };
                if let Some(atom) = atom {
                    body = text_body(&text_of_atom(&atom), style.as_ref(), ttype, &ctx);
                }
                let _ = write!(shapes, "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Shape {id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr>{sppr}</p:spPr>");
                if !body.is_empty() {
                    let _ = write!(shapes, "<p:txBody>{bodypr}<a:lstStyle/>{body}</p:txBody>");
                }
                shapes.push_str("</p:sp>");
            });
        }
        let bg_xml = bg.map(|c| format!("<p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"{c}\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg>")).unwrap_or_default();
        let n = si + 1;
        z.write(
            &format!("ppt/slides/slide{n}.xml"),
            format!("{HDR}<p:sld {NS}><p:cSld>{bg_xml}<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>").into_bytes(),
        );
        z.write(&format!("ppt/slides/_rels/slide{n}.xml.rels"), format!("{HDR}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rels}</Relationships>").into_bytes());
        let _ = write!(slide_list, "<p:sldId id=\"{}\" r:id=\"rIdS{n}\"/>", 255 + n);
        let _ = write!(pres_rels, "<Relationship Id=\"rIdS{n}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{n}.xml\"/>");
        let _ = write!(ct, "<Override PartName=\"/ppt/slides/slide{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>");
    }
    let tree0 = "<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree>";
    z.write("ppt/slideMasters/slideMaster1.xml", format!("{HDR}<p:sldMaster {NS}><p:cSld>{tree0}</p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/><p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rIdL\"/></p:sldLayoutIdLst></p:sldMaster>").into_bytes());
    z.write("ppt/slideMasters/_rels/slideMaster1.xml.rels", format!("{HDR}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdL\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/></Relationships>").into_bytes());
    z.write("ppt/slideLayouts/slideLayout1.xml", format!("{HDR}<p:sldLayout {NS}><p:cSld name=\"Blank\">{tree0}</p:cSld></p:sldLayout>").into_bytes());
    z.write("ppt/slideLayouts/_rels/slideLayout1.xml.rels", format!("{HDR}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdM\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/></Relationships>").into_bytes());
    z.write("ppt/presentation.xml", format!("{HDR}<p:presentation {NS}><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rIdMaster\"/></p:sldMasterIdLst><p:sldIdLst>{slide_list}</p:sldIdLst><p:sldSz cx=\"{}\" cy=\"{}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>", emu(sw), emu(sh)).into_bytes());
    z.write("ppt/_rels/presentation.xml.rels", format!("{HDR}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{pres_rels}</Relationships>").into_bytes());
    z.write("_rels/.rels", format!("{HDR}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/></Relationships>").into_bytes());
    z.write("[Content_Types].xml", format!("{HDR}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/><Default Extension=\"emf\" ContentType=\"image/x-emf\"/><Default Extension=\"wmf\" ContentType=\"image/x-wmf\"/><Default Extension=\"bmp\" ContentType=\"image/bmp\"/><Default Extension=\"tiff\" ContentType=\"image/tiff\"/><Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/><Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/><Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>{ct}</Types>").into_bytes());
    z.save()
}

const HDR: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const NS: &str = "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"";

struct Ctx<'a> {
    fonts: &'a [String],
    colors: &'a [String],
    styles: &'a HashMap<u16, Vec<(Pf, Cf)>>,
}

fn walk(r: &Rec, f: &mut dyn FnMut(&Rec)) {
    for k in &r.kids {
        if k.typ == 0xF004 {
            f(k);
        } else if k.container {
            walk(k, f);
        }
    }
}

/// Paragraphs of a text atom with every run's properties resolved.
fn text_body(text: &str, style: Option<&Rec>, text_type: u32, c: &Ctx) -> String {
    let chars: Vec<char> = text.chars().collect();
    let (pf_runs, cf_runs) = match style {
        Some(st) => parse_style(&st.data, chars.len() + 1),
        None => (Vec::new(), Vec::new()),
    };
    // Per character: paragraph run (indent level + exception), character exception.
    let mut per_pf: Vec<(u16, Pf)> = Vec::new();
    for (n, ex) in &pf_runs {
        let lvl = u16_at(ex, 0);
        let pf = read_pf(&ex[2..]);
        for _ in 0..*n {
            per_pf.push((lvl, pf.clone()));
        }
    }
    let mut per_cf: Vec<Cf> = Vec::new();
    for (n, ex) in &cf_runs {
        let cf = read_cf(ex);
        for _ in 0..*n {
            per_cf.push(cf.clone());
        }
    }
    // The master style for this kind of text.
    let inst = match text_type {
        0 | 6 => if text_type == 6 { 6 } else { 0 },
        1 | 5 | 7 | 8 => if c.styles.contains_key(&(text_type as u16)) { text_type as u16 } else { 1 },
        2 => 2,
        _ => 4,
    };
    let master = c.styles.get(&inst).or_else(|| if inst == 4 { None } else { c.styles.get(&(if inst == 6 { 0 } else { 1 })) });
    let mut out = String::new();
    let mut i = 0;
    for para in text.split('\r') {
        let n = para.chars().count();
        let (lvl, pf) = per_pf.get(i).cloned().unwrap_or_default();
        let (mpf, mcf) = master.and_then(|m| m.get(lvl as usize).or(m.first())).cloned().unwrap_or_default();
        let pf = pf.over(&mpf);
        let algn = match pf.align.unwrap_or(0) {
            1 => "ctr",
            2 => "r",
            3 => "just",
            4 => "dist",
            _ => "l",
        };
        let mar = pf.margin.unwrap_or(0) as i32;
        let ind = pf.indent.unwrap_or(0) as i32;
        let mut ppr = format!("<a:pPr lvl=\"{}\" algn=\"{algn}\" marL=\"{}\" indent=\"{}\">", lvl.min(8), emu(ind), emu(mar - ind));
        let spc = |v: i16| if v >= 0 { format!("<a:spcPct val=\"{}\"/>", v as i32 * 1000) } else { format!("<a:spcPts val=\"{}\"/>", (-(v as i32)) * 100 / 8) };
        if let Some(l) = pf.line {
            let _ = write!(ppr, "<a:lnSpc>{}</a:lnSpc>", spc(l));
        }
        if let Some(b) = pf.before {
            let _ = write!(ppr, "<a:spcBef>{}</a:spcBef>", spc(b));
        }
        if let Some(a) = pf.after {
            let _ = write!(ppr, "<a:spcAft>{}</a:spcAft>", spc(a));
        }
        if pf.bullet == Some(true) {
            let font = pf.bullet_font.and_then(|f| c.fonts.get(f as usize));
            let ch = pf.bullet_char.and_then(|u| char::from_u32(u as u32)).unwrap_or('•');
            let ch = if (font.is_none() && ('\u{F000}'..='\u{F0FF}').contains(&ch)) || (ch as u32) < 0x20 { '•' } else { ch };
            if let Some(col) = pf.bullet_color.and_then(|x| color_of(x, c.colors)) {
                let _ = write!(ppr, "<a:buClr><a:srgbClr val=\"{col}\"/></a:buClr>");
            }
            match pf.bullet_size {
                Some(v) if v > 0 => {
                    let _ = write!(ppr, "<a:buSzPct val=\"{}\"/>", v as i32 * 1000);
                }
                Some(v) if v < 0 => {
                    let _ = write!(ppr, "<a:buSzPts val=\"{}\"/>", -(v as i32) * 100);
                }
                _ => {}
            }
            if let Some(f) = font {
                let _ = write!(ppr, "<a:buFont typeface=\"{}\"/>", attr(f));
            }
            let _ = write!(ppr, "<a:buChar char=\"{}\"/>", attr(&ch.to_string()));
        } else {
            ppr.push_str("<a:buNone/>");
        }
        ppr.push_str("</a:pPr>");
        let _ = write!(out, "<a:p>{ppr}");
        // Runs: change where the character properties change.
        let mut k = 0;
        let pchars: Vec<char> = para.chars().collect();
        while k < n {
            let cf = per_cf.get(i + k).cloned().unwrap_or_default();
            let mut e = k + 1;
            while e < n && per_cf.get(i + e).map(|x| format!("{x:?}")) == Some(format!("{cf:?}")) {
                e += 1;
            }
            let rcf = cf.over(&mcf);
            let t: String = pchars[k..e].iter().collect::<String>().replace('\u{b}', "\n");
            let mut rpr = format!("<a:rPr lang=\"en-US\" sz=\"{}\"", rcf.size.unwrap_or(18) as u32 * 100);
            if rcf.bold == Some(true) {
                rpr.push_str(" b=\"1\"");
            }
            if rcf.italic == Some(true) {
                rpr.push_str(" i=\"1\"");
            }
            if rcf.underline == Some(true) {
                rpr.push_str(" u=\"sng\"");
            }
            if let Some(p) = rcf.position {
                if p != 0 {
                    let _ = write!(rpr, " baseline=\"{}\"", p as i32 * 1000);
                }
            }
            rpr.push('>');
            let col = rcf.color.and_then(|x| color_of(x, c.colors)).or_else(|| c.colors.get(if inst == 0 || inst == 6 { 3 } else { 1 }).cloned());
            if let Some(col) = col {
                let _ = write!(rpr, "<a:solidFill><a:srgbClr val=\"{col}\"/></a:solidFill>");
            }
            if let Some(f) = rcf.font.and_then(|f| c.fonts.get(f as usize)) {
                let _ = write!(rpr, "<a:latin typeface=\"{}\"/><a:ea typeface=\"{}\"/>", attr(f), attr(f));
            }
            rpr.push_str("</a:rPr>");
            for (j, line) in t.split('\n').enumerate() {
                if j > 0 {
                    let _ = write!(out, "<a:br>{rpr}</a:br>");
                }
                if !line.is_empty() {
                    let _ = write!(out, "<a:r>{rpr}<a:t>{}</a:t></a:r>", escape(line));
                }
            }
            k = e;
        }
        let rcf = per_cf.get(i).cloned().unwrap_or_default().over(&mcf);
        let _ = write!(out, "<a:endParaRPr lang=\"en-US\" sz=\"{}\"/></a:p>", rcf.size.unwrap_or(18) as u32 * 100);
        i += n + 1;
    }
    out
}
