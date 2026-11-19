//! Open Packaging Conventions: the parts of an OOXML package, their content types and the
//! relationships between them. XML parts are parsed on demand and written back only when their
//! text changed (the zip layer keeps unchanged entries as copies of the original).

use crate::xml::{self, Document, Element, Node};
use crate::zip::Zip;
use crate::{err, Result};

pub const RT_OFFICE_DOCUMENT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
pub const RT_STRICT_OFFICE_DOCUMENT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument";

#[derive(Clone, Debug)]
pub struct Rel {
    pub id: String,
    pub rtype: String,
    pub target: String,
    pub external: bool,
}

impl Rel {
    /// The relationship type's last path segment (`image`, `header`, `slideLayout`), the same for
    /// transitional and strict packages.
    pub fn kind(&self) -> &str {
        self.rtype.rsplit('/').next().unwrap_or("")
    }
}

/// The relationships of one part (its `_rels/<name>.rels`).
#[derive(Clone)]
pub struct Rels {
    pub source: String,
    pub path: String,
    pub list: Vec<Rel>,
    doc: Option<Document>,
    dirty: bool,
}

impl Rels {
    pub fn get(&self, id: &str) -> Option<&Rel> {
        self.list.iter().find(|r| r.id == id)
    }
    pub fn by_kind(&self, kind: &str) -> Vec<&Rel> {
        self.list.iter().filter(|r| r.kind() == kind).collect()
    }
    /// The part a relationship points at.
    pub fn part(&self, id: &str) -> Option<String> {
        let r = self.get(id)?;
        if r.external {
            return None;
        }
        Some(resolve(&self.source, &r.target))
    }
    pub fn first_part(&self, kind: &str) -> Option<String> {
        let r = self.list.iter().find(|r| r.kind() == kind && !r.external)?;
        Some(resolve(&self.source, &r.target))
    }

    /// Adds a relationship and returns its id (`rIdN`, the first unused).
    pub fn add(&mut self, rtype: &str, target: &str, external: bool) -> String {
        let mut n = self.list.len() + 1;
        while self.list.iter().any(|r| r.id == format!("rId{n}")) {
            n += 1;
        }
        let id = format!("rId{n}");
        self.list.push(Rel { id: id.clone(), rtype: rtype.to_string(), target: target.to_string(), external });
        let doc = self.doc.get_or_insert_with(|| {
            Document::parse("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>").unwrap()
        });
        let mut e = Element::new("Relationship", xml::REL);
        e.set_plain("Id", &id);
        e.set_plain("Type", rtype);
        e.set_plain("Target", target);
        if external {
            e.set_plain("TargetMode", "External");
        }
        doc.root.push(e);
        self.dirty = true;
        id
    }

    /// Adds a relationship to a part, by the part's name (the target is made relative).
    pub fn add_part(&mut self, rtype: &str, part: &str) -> String {
        let target = relative(&self.source, part);
        if let Some(r) = self.list.iter().find(|r| r.rtype == rtype && !r.external && resolve(&self.source, &r.target) == part) {
            return r.id.clone();
        }
        self.add(rtype, &target, false)
    }

    pub fn remove(&mut self, id: &str) {
        self.list.retain(|r| r.id != id);
        if let Some(doc) = &mut self.doc {
            doc.root.children.retain(|n| n.as_element().map(|e| e.attr("Id") != Some(id)).unwrap_or(true));
            self.dirty = true;
        }
    }

    pub fn rename_id(&mut self, from: &str, to: &str) {
        for r in self.list.iter_mut() {
            if r.id == from {
                r.id = to.to_string();
            }
        }
        if let Some(doc) = &mut self.doc {
            for e in doc.root.elements_mut() {
                if e.attr("Id") == Some(from) {
                    e.set_plain("Id", to);
                }
            }
            self.dirty = true;
        }
    }

    pub fn set_target(&mut self, id: &str, target: &str) {
        if let Some(r) = self.list.iter_mut().find(|r| r.id == id) {
            r.target = target.to_string();
        }
        if let Some(doc) = &mut self.doc {
            for e in doc.root.elements_mut() {
                if e.attr("Id") == Some(id) {
                    e.set_plain("Target", target);
                }
            }
            self.dirty = true;
        }
    }

    pub fn save(&mut self, pkg: &mut Package) {
        if self.dirty {
            if let Some(doc) = &self.doc {
                pkg.zip.write(&self.path, doc.to_string().into_bytes());
            }
            self.dirty = false;
        }
    }
}

/// The folder of a part (`word/document.xml` → `word`).
pub fn folder(part: &str) -> &str {
    match part.rfind('/') {
        Some(i) => &part[..i],
        None => "",
    }
}

/// A relationship target, relative to the source part, as a part name (no leading slash).
pub fn resolve(source: &str, target: &str) -> String {
    let target = target.split('#').next().unwrap_or(target);
    let target = percent_decode(target);
    if let Some(abs) = target.strip_prefix('/') {
        return normalize(abs);
    }
    let base = folder(source);
    if base.is_empty() {
        normalize(&target)
    } else {
        normalize(&format!("{base}/{target}"))
    }
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// The target that reaches `part` from `source` (`word/document.xml`, `word/media/a.png` →
/// `media/a.png`).
pub fn relative(source: &str, part: &str) -> String {
    let base: Vec<&str> = folder(source).split('/').filter(|s| !s.is_empty()).collect();
    let to: Vec<&str> = part.split('/').collect();
    let mut common = 0;
    while common < base.len() && common < to.len() - 1 && base[common] == to[common] {
        common += 1;
    }
    let mut out = String::new();
    for _ in common..base.len() {
        out.push_str("../");
    }
    out.push_str(&to[common..].join("/"));
    out
}

pub fn rels_path(part: &str) -> String {
    match part.rfind('/') {
        Some(i) => format!("{}/_rels/{}.rels", &part[..i], &part[i + 1..]),
        None => format!("_rels/{part}.rels"),
    }
}

pub struct Package {
    pub zip: Zip,
    types: Option<Document>,
    types_dirty: bool,
}

impl Package {
    pub fn open(zip: Zip) -> Result<Package> {
        let types = match zip.read_string("[Content_Types].xml") {
            Ok(s) => Some(Document::parse(&s)?),
            Err(_) => None,
        };
        Ok(Package { zip, types, types_dirty: false })
    }

    pub fn xml(&self, part: &str) -> Result<Document> {
        let text = self.zip.read_string(part)?;
        Document::parse(&text).map_err(|e| err(format!("{part}: {e}")))
    }

    pub fn has(&self, part: &str) -> bool {
        self.zip.contains(part)
    }

    pub fn put_xml(&mut self, part: &str, doc: &Document) {
        self.zip.write(part, doc.to_string().into_bytes());
    }

    pub fn rels(&self, part: &str) -> Result<Rels> {
        let path = if part.is_empty() { "_rels/.rels".to_string() } else { rels_path(part) };
        let mut list = Vec::new();
        let doc = match self.zip.read_string(&path) {
            Ok(s) => {
                let d = Document::parse(&s).map_err(|e| err(format!("{path}: {e}")))?;
                for e in d.root.elements() {
                    if e.local() == "Relationship" {
                        list.push(Rel {
                            id: e.attr("Id").unwrap_or("").to_string(),
                            rtype: e.attr("Type").unwrap_or("").to_string(),
                            target: e.attr("Target").unwrap_or("").to_string(),
                            external: e.attr("TargetMode").map(|m| m.eq_ignore_ascii_case("External")).unwrap_or(false),
                        });
                    }
                }
                Some(d)
            }
            Err(_) => None,
        };
        Ok(Rels { source: part.to_string(), path, list, doc, dirty: false })
    }

    /// The main part (the package relationship of type officeDocument).
    pub fn main_part(&self) -> Option<String> {
        let rels = self.rels("").ok()?;
        rels.list
            .iter()
            .find(|r| r.rtype == RT_OFFICE_DOCUMENT || r.rtype == RT_STRICT_OFFICE_DOCUMENT || r.kind() == "officeDocument")
            .map(|r| resolve("", &r.target))
            .and_then(|p| self.zip.real_name(&p))
    }

    pub fn content_type(&self, part: &str) -> Option<String> {
        let t = self.types.as_ref()?;
        let want = format!("/{}", part.trim_start_matches('/'));
        for e in t.root.elements() {
            if e.local() == "Override" && e.attr("PartName").map(|p| p.eq_ignore_ascii_case(&want)).unwrap_or(false) {
                return e.attr("ContentType").map(str::to_string);
            }
        }
        let ext = part.rsplit('.').next()?.to_ascii_lowercase();
        for e in t.root.elements() {
            if e.local() == "Default" && e.attr("Extension").map(|x| x.eq_ignore_ascii_case(&ext)).unwrap_or(false) {
                return e.attr("ContentType").map(str::to_string);
            }
        }
        None
    }

    /// Declares the content type of a new part (an Override, or a Default for its extension when
    /// that is how the package does images).
    pub fn set_content_type(&mut self, part: &str, ctype: &str, by_extension: bool) {
        let Some(t) = self.types.as_mut() else { return };
        let ext = part.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        if by_extension {
            let has = t.root.elements().any(|e| e.local() == "Default" && e.attr("Extension").map(|x| x.eq_ignore_ascii_case(&ext)).unwrap_or(false));
            if !has {
                let mut e = Element::new("Default", xml::CT);
                e.set_plain("Extension", &ext);
                e.set_plain("ContentType", ctype);
                // Defaults go before the overrides.
                let at = t.root.children.iter().position(|n| n.as_element().map(|e| e.local() == "Override").unwrap_or(false)).unwrap_or(t.root.children.len());
                t.root.children.insert(at, Node::Element(e));
                self.types_dirty = true;
            }
            return;
        }
        let want = format!("/{}", part.trim_start_matches('/'));
        for e in t.root.elements_mut() {
            if e.local() == "Override" && e.attr("PartName").map(|p| p.eq_ignore_ascii_case(&want)).unwrap_or(false) {
                e.set_plain("ContentType", ctype);
                self.types_dirty = true;
                return;
            }
        }
        let mut e = Element::new("Override", xml::CT);
        e.set_plain("PartName", &want);
        e.set_plain("ContentType", ctype);
        t.root.push(e);
        self.types_dirty = true;
    }

    pub fn remove_content_type(&mut self, part: &str) {
        let Some(t) = self.types.as_mut() else { return };
        let want = format!("/{}", part.trim_start_matches('/'));
        let before = t.root.children.len();
        t.root.children.retain(|n| {
            !n.as_element().map(|e| e.local() == "Override" && e.attr("PartName").map(|p| p.eq_ignore_ascii_case(&want)).unwrap_or(false)).unwrap_or(false)
        });
        if t.root.children.len() != before {
            self.types_dirty = true;
        }
    }

    /// Removes a part, its relationships part and its content type override.
    pub fn remove_part(&mut self, part: &str) {
        self.zip.remove(part);
        self.zip.remove(&rels_path(part));
        self.remove_content_type(part);
    }

    /// A part name not in the package yet: `pattern` with `{}` replaced by 1, 2, ...
    pub fn free_name(&self, pattern: &str) -> String {
        let mut n = 1;
        loop {
            let name = pattern.replace("{}", &n.to_string());
            if !self.zip.contains(&name) {
                return name;
            }
            n += 1;
        }
    }

    pub fn save(&mut self) -> Result<Vec<u8>> {
        if self.types_dirty {
            if let Some(t) = &self.types {
                let s = t.to_string();
                self.zip.write("[Content_Types].xml", s.into_bytes());
            }
            self.types_dirty = false;
        }
        self.zip.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn targets() {
        assert_eq!(resolve("word/document.xml", "media/image1.png"), "word/media/image1.png");
        assert_eq!(resolve("ppt/slides/slide1.xml", "../slideLayouts/slideLayout1.xml"), "ppt/slideLayouts/slideLayout1.xml");
        assert_eq!(resolve("", "/word/document.xml"), "word/document.xml");
        assert_eq!(relative("word/document.xml", "word/media/a.png"), "media/a.png");
        assert_eq!(relative("ppt/slides/slide1.xml", "ppt/media/a.png"), "../media/a.png");
        assert_eq!(relative("", "word/document.xml"), "word/document.xml");
        assert_eq!(rels_path("word/document.xml"), "word/_rels/document.xml.rels");
    }
}
