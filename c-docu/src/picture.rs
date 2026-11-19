//! Pictures: what an image file is and how big it is, and adding a picture to (or swapping the
//! picture of) a Word paragraph, a slide or a sheet — a new media part and a relationship, the
//! rest of the package untouched.

use crate::xml::{self, Document, Element, Node, A, P, PIC, R, S, W, WP, XDR};
use crate::{err, Result};

pub struct Image {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
    pub mime: &'static str,
    pub px: (u32, u32),
    pub dpi: (f64, f64),
}

fn be32(d: &[u8], i: usize) -> u32 {
    d.get(i..i + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0)
}
fn be16(d: &[u8], i: usize) -> u16 {
    d.get(i..i + 2).map(|b| u16::from_be_bytes([b[0], b[1]])).unwrap_or(0)
}
fn le16(d: &[u8], i: usize) -> u16 {
    d.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0)
}
fn le32(d: &[u8], i: usize) -> u32 {
    d.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0)
}

impl Image {
    pub fn load(path: &str) -> Result<Image> {
        let bytes = std::fs::read(path).map_err(|e| err(format!("{path}: {e}")))?;
        Image::from_bytes(bytes).map_err(|e| err(format!("{path}: {e}")))
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Image> {
        let d = &bytes;
        let mut dpi = (96.0, 96.0);
        let (ext, mime, px) = if d.starts_with(b"\x89PNG\r\n\x1a\n") {
            // pHYs: pixels per metre.
            let mut i = 8;
            while i + 12 <= d.len() {
                let len = be32(d, i) as usize;
                if &d[i + 4..i + 8] == b"pHYs" && d.get(i + 16) == Some(&1) {
                    let (x, y) = (be32(d, i + 8) as f64 * 0.0254, be32(d, i + 12) as f64 * 0.0254);
                    if x > 1.0 && y > 1.0 {
                        dpi = (x, y);
                    }
                }
                if &d[i + 4..i + 8] == b"IDAT" {
                    break;
                }
                i += 12 + len;
            }
            ("png", "image/png", (be32(d, 16), be32(d, 20)))
        } else if d.starts_with(&[0xFF, 0xD8]) {
            let mut i = 2;
            let mut size = None;
            while i + 9 < d.len() {
                if d[i] != 0xFF {
                    i += 1;
                    continue;
                }
                let m = d[i + 1];
                if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) || m == 0xFF {
                    i += if m == 0xFF { 1 } else { 2 };
                    continue;
                }
                let len = be16(d, i + 2) as usize;
                if m == 0xE0 && d.get(i + 4..i + 9) == Some(b"JFIF\0") {
                    let (unit, x, y) = (d[i + 11], be16(d, i + 12) as f64, be16(d, i + 14) as f64);
                    if x > 1.0 && y > 1.0 {
                        match unit {
                            1 => dpi = (x, y),
                            2 => dpi = (x * 2.54, y * 2.54),
                            _ => {}
                        }
                    }
                }
                if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
                    size = Some((be16(d, i + 7) as u32, be16(d, i + 5) as u32));
                    break;
                }
                i += 2 + len;
            }
            ("jpeg", "image/jpeg", size.ok_or_else(|| err("a JPEG without a frame header"))?)
        } else if d.starts_with(b"GIF8") {
            ("gif", "image/gif", (le16(d, 6) as u32, le16(d, 8) as u32))
        } else if d.starts_with(b"BM") {
            let (x, y) = (le32(d, 38) as f64 * 0.0254, le32(d, 42) as f64 * 0.0254);
            if x > 1.0 && y > 1.0 {
                dpi = (x, y);
            }
            ("bmp", "image/bmp", (le32(d, 18), (le32(d, 22) as i32).unsigned_abs()))
        } else if d.starts_with(b"II*\0") || d.starts_with(b"MM\0*") {
            let be = d[0] == b'M';
            let r16 = |i: usize| if be { be16(d, i) } else { le16(d, i) } as u32;
            let r32 = |i: usize| if be { be32(d, i) } else { le32(d, i) };
            let ifd = r32(4) as usize;
            let (mut w, mut h) = (0, 0);
            for k in 0..r16(ifd) as usize {
                let e = ifd + 2 + k * 12;
                let v = if r16(e + 2) == 3 { r16(e + 8) } else { r32(e + 8) };
                match r16(e) {
                    256 => w = v,
                    257 => h = v,
                    _ => {}
                }
            }
            ("tiff", "image/tiff", (w, h))
        } else if le32(d, 0) == 1 && d.get(40..44) == Some(b" EMF") {
            // The frame, in hundredths of a millimetre.
            let fw = le32(d, 32) as i32 - le32(d, 24) as i32;
            let fh = le32(d, 36) as i32 - le32(d, 28) as i32;
            dpi = (2540.0, 2540.0);
            ("emf", "image/x-emf", (fw.max(1) as u32, fh.max(1) as u32))
        } else if d.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) {
            let units = le16(d, 14).max(1) as f64;
            let w = (le16(d, 10) as i16 as i32 - le16(d, 6) as i16 as i32).unsigned_abs();
            let h = (le16(d, 12) as i16 as i32 - le16(d, 8) as i16 as i32).unsigned_abs();
            dpi = (units, units);
            ("wmf", "image/x-wmf", (w, h))
        } else {
            let head = String::from_utf8_lossy(&d[..d.len().min(256)]).to_lowercase();
            if head.contains("<svg") || head.contains("<?xml") {
                return Err(err("SVG pictures are not supported; use a PNG"));
            }
            return Err(err("not a picture docu knows (PNG, JPEG, GIF, BMP, TIFF, EMF, WMF)"));
        };
        if px.0 == 0 || px.1 == 0 {
            return Err(err("the picture has no size"));
        }
        Ok(Image { bytes, ext, mime, px, dpi })
    }

    /// Its size in EMU at its resolution.
    pub fn natural(&self) -> (i64, i64) {
        ((self.px.0 as f64 / self.dpi.0 * 914400.0) as i64, (self.px.1 as f64 / self.dpi.1 * 914400.0) as i64)
    }

    /// The size to show it at: as asked (one side keeps the proportions), else natural but no
    /// wider than `max_w`.
    pub fn size(&self, w: Option<i64>, h: Option<i64>, max_w: Option<i64>) -> (i64, i64) {
        let (nw, nh) = self.natural();
        let ratio = nh as f64 / nw as f64;
        match (w, h) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, (w as f64 * ratio) as i64),
            (None, Some(h)) => ((h as f64 / ratio) as i64, h),
            (None, None) => match max_w {
                Some(m) if nw > m => (m, (m as f64 * ratio) as i64),
                _ => (nw, nh),
            },
        }
    }
}

fn emu(pt: f64) -> i64 {
    (pt * 12700.0).round() as i64
}

/// Writes the picture as a new media part and returns its name.
fn add_media(pkg: &mut crate::opc::Package, folder: &str, img: &Image) -> String {
    let part = pkg.free_name(&format!("{folder}/image{{}}.{}", img.ext));
    pkg.zip.write(&part, img.bytes.clone());
    pkg.set_content_type(&part, img.mime, true);
    part
}

const IMAGE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";

fn max_id(root: &Element, local: &str) -> u32 {
    root.descendants().iter().filter(|e| e.local() == local).filter_map(|e| e.attr_ns(xml::NONE, "id").and_then(|v| v.parse::<u32>().ok())).max().unwrap_or(0)
}

/// Rewrites the usual prefixes of a template to the ones the document uses.
fn retag(src: &str, doc: &mut Document, names: &[(&str, xml::Ns)]) -> String {
    let mut s = src.to_string();
    for (usual, ns) in names {
        let actual = doc.prefix_for(*ns);
        if actual != *usual {
            s = s.replace(&format!("<{usual}:"), &format!("<{actual}:")).replace(&format!("</{usual}:"), &format!("</{actual}:")).replace(&format!(" {usual}:"), &format!(" {actual}:"));
        }
    }
    s
}

fn pic_xml(id: u32, name: &str, alt: &str, rid: &str, cx: i64, cy: i64, prefix_ns: &str) -> String {
    let descr = if alt.is_empty() { String::new() } else { format!(" descr=\"{}\"", xml::escape_attr_str(alt)) };
    format!(
        "<pic:pic{prefix_ns}><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"{}\"{descr}/><pic:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></pic:cNvPicPr></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>",
        xml::escape_attr_str(name)
    )
}

// ── Word ────────────────────────────────────────────────────────────────────────────────────

impl crate::docx::Docx {
    /// A new paragraph after (or before) `addr` holding the picture, or with `replace`, the
    /// picture of the paragraph(s) at `addr` (the `nth`, default the first) swapped for this one.
    pub fn insert_image(&mut self, addr: &str, after: bool, img: &Image, props: &crate::props::Props, replace: bool, nth: Option<usize>) -> Result<String> {
        let target = self.resolve(addr)?;
        let crate::docx::Target::Paras { story, paras, .. } = target else {
            return Err(err(format!("{addr}: pictures go after a paragraph (p3), or replace one in it (--replace)")));
        };
        let part_i = self.stories[story].part;
        let part_name = self.parts[part_i].name.clone();
        let media = add_media(&mut self.pkg, "word/media", img);
        let rid = if part_i == 0 {
            self.rels.add_part(IMAGE_REL, &media)
        } else {
            let mut rels = self.pkg.rels(&part_name)?;
            let id = rels.add_part(IMAGE_REL, &media);
            rels.save(&mut self.pkg);
            id
        };
        let max_w = self.text_width_twips() * 635;
        if replace {
            let want = nth.unwrap_or(1);
            let mut seen = 0;
            for p in &paras {
                let el = self.para_mut(p);
                let mut blips: Vec<Vec<usize>> = Vec::new();
                find_paths(el, &mut Vec::new(), &mut blips, &|e| e.local() == "blip");
                for path in blips {
                    seen += 1;
                    if seen != want {
                        continue;
                    }
                    // The drawing holding the blip: keep its width, take the new proportions.
                    let blip = crate::text::at_mut(el, &path);
                    blip.set_attr("r:embed", R, &rid);
                    // Its extensions (an SVG original, local DPI) were the old picture's.
                    blip.children.clear();
                    let (old_w, _) = drawing_extent(el, &path).unwrap_or((0, 0));
                    let w = props.width.map(emu).or(if old_w > 0 { Some(old_w) } else { None });
                    let (cx, cy) = img.size(w, props.height.map(emu), Some(max_w));
                    set_drawing_extent(el, &path, cx, cy);
                    return Ok(format!("replaced the picture in {}", self.para_addr(p)));
                }
            }
            return Err(err(format!("{addr} has {seen} picture{} (nothing written)", if seen == 1 { "" } else { "s" })));
        }
        let (cx, cy) = img.size(props.width.map(emu), props.height.map(emu), Some(max_w));
        let p = if after { paras.last().unwrap().clone() } else { paras[0].clone() };
        let doc = &mut self.parts[part_i].doc;
        let id = max_id(&doc.root, "docPr").max(max_id(&doc.root, "cNvPr")) + 1;
        let name = props.name.clone().unwrap_or_else(|| format!("Picture {id}"));
        let alt = props.alt.clone().unwrap_or_default();
        let descr = if alt.is_empty() { String::new() } else { format!(" descr=\"{}\"", xml::escape_attr_str(&alt)) };
        let src = format!(
            "<w:p><w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/><wp:docPr id=\"{id}\" name=\"{}\"{descr}/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">{}</a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>",
            xml::escape_attr_str(&name),
            pic_xml(0, &name, &alt, &rid, cx, cy, "")
        );
        let src = retag(&src, doc, &[("w", W), ("wp", WP), ("a", A), ("pic", PIC), ("r", R)]);
        let wp_p = doc.prefix_for(WP);
        let a_p = doc.prefix_for(A);
        let pic_p = doc.prefix_for(PIC);
        let r_p = doc.prefix_for(R);
        let w_p = doc.prefix_for(W);
        let el = xml::fragment(&src, &[(w_p.as_str(), W), (wp_p.as_str(), WP), (a_p.as_str(), A), (pic_p.as_str(), PIC), (r_p.as_str(), R)])?.remove(0);
        let story_el = self.story_el_mut(story);
        let (pp, pi) = p.path.split_at(p.path.len() - 1);
        let parent = crate::text::at_mut(story_el, pp);
        parent.children.insert(if after { pi[0] + 1 } else { pi[0] }, Node::Element(el));
        let new_addr = format!("{}p{}", self.story_prefix(story), if after { p.index + 1 } else { p.index });
        let mut rest = props.clone();
        rest.keys.retain(|k| !matches!(k.as_str(), "w" | "h" | "name" | "alt"));
        rest.width = None;
        rest.height = None;
        rest.name = None;
        rest.alt = None;
        if !rest.keys.is_empty() {
            self.style(&new_addr, &rest, None)?;
        }
        Ok(format!("inserted a picture ({:.1}x{:.1}cm) in {new_addr}", cx as f64 / 360000.0, cy as f64 / 360000.0))
    }
}

fn find_paths(e: &Element, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>, pred: &dyn Fn(&Element) -> bool) {
    for (i, c) in e.children.iter().enumerate() {
        if let Node::Element(c) = c {
            path.push(i);
            if pred(c) {
                out.push(path.clone());
            }
            find_paths(c, path, out, pred);
            path.pop();
        }
    }
}

/// The extent of the drawing (wp:inline / wp:anchor) around the element at `path`.
fn drawing_extent(root: &Element, path: &[usize]) -> Option<(i64, i64)> {
    for k in (0..path.len()).rev() {
        let e = crate::text::at(root, &path[..k]);
        if e.local() == "inline" || e.local() == "anchor" {
            let x = e.child(WP, "extent")?;
            return Some((x.own("cx")?.parse().ok()?, x.own("cy")?.parse().ok()?));
        }
    }
    None
}

fn set_drawing_extent(root: &mut Element, path: &[usize], cx: i64, cy: i64) {
    for k in (0..path.len()).rev() {
        let e = crate::text::at_mut(root, &path[..k]);
        if e.local() == "inline" || e.local() == "anchor" {
            if let Some(x) = e.child_mut(WP, "extent") {
                x.set_own("cx", &cx.to_string());
                x.set_own("cy", &cy.to_string());
            }
            e.walk_mut(&mut |d| {
                if d.local() == "ext" && d.ns() == A && d.own("cx").is_some() {
                    d.set_own("cx", &cx.to_string());
                    d.set_own("cy", &cy.to_string());
                }
            });
            return;
        }
    }
}

// ── PowerPoint ──────────────────────────────────────────────────────────────────────────────

impl crate::pptx::Pptx {
    /// A picture on slide `addr` (s2), at x/y/w/h or centred at its own size (no larger than the
    /// slide); on a picture shape (s2.sh4), its picture swapped, fitted into the same box.
    pub fn insert_image(&mut self, addr: &str, img: &Image, props: &crate::props::Props) -> Result<String> {
        let target = self.resolve(addr)?;
        let (sw, sh) = self.slide_size();
        match target {
            crate::pptx::Target::Slide(si) => {
                let media = add_media(&mut self.pkg, "ppt/media", img);
                let rid = self.slides[si].rels.add_part(IMAGE_REL, &media);
                let (cx, cy) = {
                    let (w, h) = img.size(props.width.map(emu), props.height.map(emu), None);
                    // No bigger than 90% of the slide unless asked.
                    if props.width.is_none() && props.height.is_none() && (w > sw * 9 / 10 || h > sh * 9 / 10) {
                        let s = (sw as f64 * 0.9 / w as f64).min(sh as f64 * 0.9 / h as f64);
                        ((w as f64 * s) as i64, (h as f64 * s) as i64)
                    } else {
                        (w, h)
                    }
                };
                let x = props.x.map(emu).unwrap_or((sw - cx) / 2);
                let y = props.y.map(emu).unwrap_or((sh - cy) / 2);
                let doc = &mut self.slides[si].doc;
                let id = max_id(&doc.root, "cNvPr") + 1;
                let name = props.name.clone().unwrap_or_else(|| format!("Picture {}", id - 1));
                let alt = props.alt.clone().unwrap_or_default();
                let descr = if alt.is_empty() { String::new() } else { format!(" descr=\"{}\"", xml::escape_attr_str(&alt)) };
                let src = format!(
                    "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"{}\"{descr}/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
                    xml::escape_attr_str(&name)
                );
                let src = retag(&src, doc, &[("p", P), ("a", A), ("r", R)]);
                let (pp, ap, rp) = (doc.prefix_for(P), doc.prefix_for(A), doc.prefix_for(R));
                let el = xml::fragment(&src, &[(pp.as_str(), P), (ap.as_str(), A), (rp.as_str(), R)])?.remove(0);
                let tree = crate::pptx::Pptx::sp_tree_path(doc).ok_or_else(|| err("the slide has no shape tree"))?;
                let tree_el = crate::text::at_mut(&mut doc.root, &tree);
                let at_i = tree_el.position(P, "extLst").unwrap_or(tree_el.children.len());
                tree_el.children.insert(at_i, Node::Element(el));
                let shapes = crate::pptx::Pptx::shapes(&self.slides[si].doc);
                let shid = shapes.iter().find(|s| s.name == name).map(|s| s.id.clone()).unwrap_or_default();
                Ok(format!("added s{}.{shid} (picture, {:.1}x{:.1}cm)", si + 1, cx as f64 / 360000.0, cy as f64 / 360000.0))
            }
            crate::pptx::Target::Shape(si, loc) => {
                let media = add_media(&mut self.pkg, "ppt/media", img);
                let rid = self.slides[si].rels.add_part(IMAGE_REL, &media);
                let el = crate::text::at_mut(&mut self.slides[si].doc.root, &loc.path);
                let mut blips = Vec::new();
                find_paths(el, &mut Vec::new(), &mut blips, &|e| e.local() == "blip");
                let Some(bp) = blips.first() else {
                    return Err(err(format!("{addr} is not a picture (add one to the slide: docu image FILE s{} IMG)", si + 1)));
                };
                let blip = crate::text::at_mut(el, bp);
                blip.set_attr("r:embed", R, &rid);
                blip.children.clear();
                // Fit into the old box, centred, keeping the new proportions (unless sizes given).
                let mut xfrms = Vec::new();
                find_paths(el, &mut Vec::new(), &mut xfrms, &|e| e.local() == "xfrm");
                if let Some(xp) = xfrms.first() {
                    let xf = crate::text::at_mut(el, xp);
                    let get = |xf: &Element, n: &str, a: &str| xf.child(A, n).and_then(|e| e.own(a)).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                    let (ox, oy, ow, oh) = (get(xf, "off", "x"), get(xf, "off", "y"), get(xf, "ext", "cx"), get(xf, "ext", "cy"));
                    let (cx, cy) = if props.width.is_some() || props.height.is_some() {
                        img.size(props.width.map(emu), props.height.map(emu), None)
                    } else {
                        let (nw, nh) = img.natural();
                        let s = (ow as f64 / nw as f64).min(oh as f64 / nh as f64);
                        ((nw as f64 * s) as i64, (nh as f64 * s) as i64)
                    };
                    let x = props.x.map(emu).unwrap_or(ox + (ow - cx) / 2);
                    let y = props.y.map(emu).unwrap_or(oy + (oh - cy) / 2);
                    if let Some(off) = xf.child_mut(A, "off") {
                        off.set_own("x", &x.to_string());
                        off.set_own("y", &y.to_string());
                    }
                    if let Some(ext) = xf.child_mut(A, "ext") {
                        ext.set_own("cx", &cx.to_string());
                        ext.set_own("cy", &cy.to_string());
                    }
                }
                // A cropped picture's crop belonged to the old one.
                el.walk_mut(&mut |e| {
                    if e.local() == "srcRect" {
                        e.clear_attrs();
                    }
                });
                Ok(format!("replaced the picture of {addr}"))
            }
            _ => Err(err(format!("{addr}: pictures go on a slide (s2) or replace a picture shape (s2.sh4)"))),
        }
    }
}

// ── Excel ───────────────────────────────────────────────────────────────────────────────────

const DRAWING_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing";

impl crate::xlsx::Xlsx {
    /// A picture whose top left corner is at the cell `addr` (Sheet1!B3), at its own size or w/h.
    pub fn insert_image(&mut self, addr: &str, img: &Image, props: &crate::props::Props) -> Result<String> {
        let (si, area) = self.resolve(addr)?;
        let (r, c) = (area.a.row, area.a.col);
        let (cx, cy) = img.size(props.width.map(emu), props.height.map(emu), None);
        let sheet_part = self.sheets[si].part.clone();
        let mut sheet_rels = self.pkg.rels(&sheet_part)?;
        // The sheet's drawing, or a new one.
        let doc = self.sheets[si].doc.as_mut().ok_or_else(|| err("not a worksheet"))?;
        let existing = doc.root.child(S, "drawing").and_then(|d| d.attr_ns(R, "id")).map(str::to_string);
        let drawing_part = match existing.and_then(|id| sheet_rels.part(&id)) {
            Some(p) => p,
            None => {
                let p = self.pkg.free_name("xl/drawings/drawing{}.xml");
                let d = Document::parse("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"></xdr:wsDr>")?;
                self.pkg.put_xml(&p, &d);
                self.pkg.set_content_type(&p, "application/vnd.openxmlformats-officedocument.drawing+xml", false);
                let rid = sheet_rels.add_part(DRAWING_REL, &p);
                sheet_rels.save(&mut self.pkg);
                let rp = doc.prefix_for(R);
                let mut e = doc.root.sibling("drawing");
                e.set_attr(&format!("{rp}:id"), R, &rid);
                xml::put_in_order(&mut doc.root, e, crate::xlsx::WORKSHEET_ORDER);
                p
            }
        };
        let media = add_media(&mut self.pkg, "xl/media", img);
        let mut drels = self.pkg.rels(&drawing_part)?;
        let rid = drels.add_part(IMAGE_REL, &media);
        drels.save(&mut self.pkg);
        let mut d = self.pkg.xml(&drawing_part)?;
        let id = max_id(&d.root, "cNvPr") + 1;
        let name = props.name.clone().unwrap_or_else(|| format!("Picture {id}"));
        let alt = props.alt.clone().unwrap_or_default();
        let descr = if alt.is_empty() { String::new() } else { format!(" descr=\"{}\"", xml::escape_attr_str(&alt)) };
        let src = format!(
            "<xdr:oneCellAnchor><xdr:from><xdr:col>{c}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{r}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx=\"{cx}\" cy=\"{cy}\"/><xdr:pic><xdr:nvPicPr><xdr:cNvPr id=\"{id}\" name=\"{}\"{descr}/><xdr:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></xdr:cNvPicPr></xdr:nvPicPr><xdr:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic><xdr:clientData/></xdr:oneCellAnchor>",
            xml::escape_attr_str(&name)
        );
        let src = retag(&src, &mut d, &[("xdr", XDR), ("a", A), ("r", R)]);
        let (xp, ap, rp) = (d.prefix_for(XDR), d.prefix_for(A), d.prefix_for(R));
        let el = xml::fragment(&src, &[(xp.as_str(), XDR), (ap.as_str(), A), (rp.as_str(), R)])?.remove(0);
        d.root.push(el);
        self.pkg.put_xml(&drawing_part, &d);
        Ok(format!("added a picture at {} ({:.1}x{:.1}cm)", crate::formula::cell_name(r, c), cx as f64 / 360000.0, cy as f64 / 360000.0))
    }
}
