//! A lossless XML tree. Every element keeps the text of its start and end tags as the source had
//! them, and every text node its raw form (entities, CDATA), so a document serializes back byte for
//! byte; only what an edit touched is written anew. Namespaces are resolved while parsing (each
//! element and attribute knows its namespace URI, strict OOXML URIs mapped to the transitional
//! ones), so matching does not depend on the prefixes a producer chose.

use crate::{err, Result};
use std::sync::RwLock;

/// An interned namespace URI.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Ns(pub u32);

macro_rules! namespaces {
    ($($id:ident = $n:expr, $uri:expr, $prefix:expr;)*) => {
        $(pub const $id: Ns = Ns($n);)*
        const KNOWN: &[(&str, &str)] = &[("", ""), $(($uri, $prefix),)*];
    };
}

pub const NONE: Ns = Ns(0);
namespaces! {
    W = 1, "http://schemas.openxmlformats.org/wordprocessingml/2006/main", "w";
    R = 2, "http://schemas.openxmlformats.org/officeDocument/2006/relationships", "r";
    A = 3, "http://schemas.openxmlformats.org/drawingml/2006/main", "a";
    P = 4, "http://schemas.openxmlformats.org/presentationml/2006/main", "p";
    S = 5, "http://schemas.openxmlformats.org/spreadsheetml/2006/main", "x";
    WP = 6, "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing", "wp";
    PIC = 7, "http://schemas.openxmlformats.org/drawingml/2006/picture", "pic";
    MC = 8, "http://schemas.openxmlformats.org/markup-compatibility/2006", "mc";
    WPS = 9, "http://schemas.microsoft.com/office/word/2010/wordprocessingShape", "wps";
    WPG = 10, "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup", "wpg";
    V = 11, "urn:schemas-microsoft-com:vml", "v";
    O = 12, "urn:schemas-microsoft-com:office:office", "o";
    C = 13, "http://schemas.openxmlformats.org/drawingml/2006/chart", "c";
    XDR = 14, "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing", "xdr";
    M = 15, "http://schemas.openxmlformats.org/officeDocument/2006/math", "m";
    REL = 16, "http://schemas.openxmlformats.org/package/2006/relationships", "";
    CT = 17, "http://schemas.openxmlformats.org/package/2006/content-types", "";
    CP = 18, "http://schemas.openxmlformats.org/package/2006/metadata/core-properties", "cp";
    DC = 19, "http://purl.org/dc/elements/1.1/", "dc";
    DCTERMS = 20, "http://purl.org/dc/terms/", "dcterms";
    XSI = 21, "http://www.w3.org/2001/XMLSchema-instance", "xsi";
    XML = 22, "http://www.w3.org/XML/1998/namespace", "xml";
    XMLNS = 23, "http://www.w3.org/2000/xmlns/", "xmlns";
    W14 = 24, "http://schemas.microsoft.com/office/word/2010/wordml", "w14";
    EP = 25, "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties", "";
    WPC = 26, "http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas", "wpc";
    DGM = 27, "http://schemas.openxmlformats.org/drawingml/2006/diagram", "dgm";
    X14AC = 28, "http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac", "x14ac";
    P14 = 29, "http://schemas.microsoft.com/office/powerpoint/2010/main", "p14";
    A14 = 30, "http://schemas.microsoft.com/office/drawing/2010/main", "a14";
    // OpenDocument
    OFFICE = 31, "urn:oasis:names:tc:opendocument:xmlns:office:1.0", "office";
    TEXT = 32, "urn:oasis:names:tc:opendocument:xmlns:text:1.0", "text";
    TABLE = 33, "urn:oasis:names:tc:opendocument:xmlns:table:1.0", "table";
    DRAW = 34, "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0", "draw";
    STYLE = 35, "urn:oasis:names:tc:opendocument:xmlns:style:1.0", "style";
    FO = 36, "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0", "fo";
    SVG = 37, "urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0", "svg";
    PRESENTATION = 38, "urn:oasis:names:tc:opendocument:xmlns:presentation:1.0", "presentation";
    XLINK = 39, "http://www.w3.org/1999/xlink", "xlink";
    NUMBER = 40, "urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0", "number";
    MANIFEST = 41, "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0", "manifest";
    META = 42, "urn:oasis:names:tc:opendocument:xmlns:meta:1.0", "meta";
    OF = 43, "urn:oasis:names:tc:opendocument:xmlns:of:1.2", "of";
    // HWPX (OWPML)
    HP = 44, "http://www.hancom.co.kr/hwpml/2011/paragraph", "hp";
    HS = 45, "http://www.hancom.co.kr/hwpml/2011/section", "hs";
    HH = 46, "http://www.hancom.co.kr/hwpml/2011/head", "hh";
    HC = 47, "http://www.hancom.co.kr/hwpml/2011/core", "hc";
    HA = 48, "http://www.hancom.co.kr/hwpml/2011/app", "ha";
    HP10 = 49, "http://www.hancom.co.kr/hwpml/2016/paragraph", "hp10";
    OPF = 50, "http://www.idpf.org/2007/opf/", "opf";
    OCF = 51, "urn:oasis:names:tc:opendocument:xmlns:container", "ocf";
    W15 = 52, "http://schemas.microsoft.com/office/word/2012/wordml", "w15";
    X14 = 53, "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main", "x14";
    PR = 54, "http://schemas.openxmlformats.org/officeDocument/2006/custom-properties", "";
    VT = 55, "http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes", "vt";
    ASVG = 56, "http://schemas.microsoft.com/office/drawing/2016/SVG/main", "asvg";
    WP14 = 57, "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing", "wp14";
    LOEXT = 58, "urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0", "loext";
    HM = 59, "http://www.hancom.co.kr/hwpml/2011/master-page", "hm";
    HHS = 60, "http://www.hancom.co.kr/hwpml/2011/history", "hhs";
}

/// Strict OOXML (ISO 29500 strict) URIs, as the transitional ones they correspond to.
const STRICT: &[(&str, Ns)] = &[
    ("http://purl.oclc.org/ooxml/wordprocessingml/main", W),
    ("http://purl.oclc.org/ooxml/officeDocument/relationships", R),
    ("http://purl.oclc.org/ooxml/drawingml/main", A),
    ("http://purl.oclc.org/ooxml/presentationml/main", P),
    ("http://purl.oclc.org/ooxml/spreadsheetml/main", S),
    ("http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing", WP),
    ("http://purl.oclc.org/ooxml/drawingml/picture", PIC),
    ("http://purl.oclc.org/ooxml/drawingml/chart", C),
    ("http://purl.oclc.org/ooxml/drawingml/spreadsheetDrawing", XDR),
    ("http://purl.oclc.org/ooxml/officeDocument/math", M),
    ("http://purl.oclc.org/ooxml/drawingml/diagram", DGM),
    ("http://purl.oclc.org/ooxml/officeDocument/extendedProperties", EP),
    ("http://purl.oclc.org/ooxml/officeDocument/customProperties", PR),
    ("http://purl.oclc.org/ooxml/officeDocument/docPropsVTypes", VT),
    // HWPX documents of 2016+ name the same paragraph schema differently.
    ("http://www.hancom.co.kr/hwpml/2016/HwpUnitChar", HP10),
];

static INTERNED: RwLock<Vec<&'static str>> = RwLock::new(Vec::new());

impl Ns {
    pub fn intern(uri: &str) -> Ns {
        if let Some((_, ns)) = STRICT.iter().find(|(u, _)| *u == uri) {
            return *ns;
        }
        if let Some(i) = KNOWN.iter().position(|(u, _)| *u == uri) {
            return Ns(i as u32);
        }
        {
            let t = INTERNED.read().unwrap();
            if let Some(i) = t.iter().position(|u| *u == uri) {
                return Ns((KNOWN.len() + i) as u32);
            }
        }
        let mut t = INTERNED.write().unwrap();
        if let Some(i) = t.iter().position(|u| *u == uri) {
            return Ns((KNOWN.len() + i) as u32);
        }
        t.push(Box::leak(uri.to_string().into_boxed_str()));
        Ns((KNOWN.len() + t.len() - 1) as u32)
    }

    pub fn uri(self) -> String {
        let i = self.0 as usize;
        if i < KNOWN.len() {
            KNOWN[i].0.to_string()
        } else {
            INTERNED.read().unwrap().get(i - KNOWN.len()).copied().unwrap_or("").to_string()
        }
    }

    /// The prefix producers usually give this namespace.
    pub fn usual_prefix(self) -> &'static str {
        KNOWN.get(self.0 as usize).map(|k| k.1).unwrap_or("ns")
    }
}

#[derive(Clone, Debug)]
pub struct Attr {
    pub name: String,
    pub ns: Ns,
    pub value: String,
}

impl Attr {
    pub fn local(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }
}

#[derive(Clone, Debug)]
pub struct Text {
    text: String,
    raw: Option<String>,
}

impl Text {
    pub fn new(text: &str) -> Text {
        Text { text: text.to_string(), raw: None }
    }
    pub fn get(&self) -> &str {
        &self.text
    }
    pub fn set(&mut self, text: &str) {
        if text != self.text {
            self.text = text.to_string();
            self.raw = None;
        }
    }
    pub fn is_whitespace(&self) -> bool {
        self.text.chars().all(|c| c.is_ascii_whitespace())
    }
}

#[derive(Clone, Debug)]
pub enum Node {
    Element(Element),
    Text(Text),
    /// A comment, processing instruction or doctype, as the source had it.
    Other(String),
}

impl Node {
    pub fn as_element(&self) -> Option<&Element> {
        match self {
            Node::Element(e) => Some(e),
            _ => None,
        }
    }
    pub fn as_element_mut(&mut self) -> Option<&mut Element> {
        match self {
            Node::Element(e) => Some(e),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Element {
    name: String,
    ns: Ns,
    attrs: Vec<Attr>,
    pub children: Vec<Node>,
    raw_open: Option<String>,
    raw_close: Option<String>,
}

impl Element {
    /// A new element; `name` is qualified (`w:p`).
    pub fn new(name: &str, ns: Ns) -> Element {
        Element { name: name.to_string(), ns, attrs: Vec::new(), children: Vec::new(), raw_open: None, raw_close: None }
    }

    /// A new element in the same namespace (and with the same prefix) as `self`.
    pub fn sibling(&self, local: &str) -> Element {
        Element::new(&self.qualify(local), self.ns)
    }

    pub fn qualify(&self, local: &str) -> String {
        match self.prefix() {
            "" => local.to_string(),
            p => format!("{p}:{local}"),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn local(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }
    pub fn prefix(&self) -> &str {
        match self.name.find(':') {
            Some(i) => &self.name[..i],
            None => "",
        }
    }
    pub fn ns(&self) -> Ns {
        self.ns
    }
    pub fn is(&self, ns: Ns, local: &str) -> bool {
        self.ns == ns && self.local() == local
    }

    pub fn attrs(&self) -> &[Attr] {
        &self.attrs
    }

    /// An attribute by its qualified name as written (`r:id`, `val`).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.name == name).map(|a| a.value.as_str())
    }

    /// An attribute by namespace and local name (NONE: an unprefixed attribute).
    pub fn attr_ns(&self, ns: Ns, local: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.ns == ns && a.local() == local).map(|a| a.value.as_str())
    }

    /// The `w:val`-style attribute: in the element's own namespace, or unprefixed.
    pub fn val(&self) -> Option<&str> {
        self.attr_ns(self.ns, "val").or_else(|| self.attr_ns(NONE, "val"))
    }

    /// An attribute in the element's own namespace (or unprefixed, as some producers write it).
    pub fn own(&self, local: &str) -> Option<&str> {
        self.attr_ns(self.ns, local).or_else(|| self.attr_ns(NONE, local))
    }

    /// Sets an attribute by namespace and local name, keeping its place when it exists; a new
    /// attribute takes `name` (qualified) and goes last.
    pub fn set_attr(&mut self, name: &str, ns: Ns, value: &str) {
        let local = name.rsplit(':').next().unwrap_or(name);
        if let Some(a) = self.attrs.iter_mut().find(|a| a.ns == ns && a.local() == local) {
            if a.value != value {
                a.value = value.to_string();
                self.raw_open = None;
            }
            return;
        }
        self.attrs.push(Attr { name: name.to_string(), ns, value: value.to_string() });
        self.raw_open = None;
    }

    /// Sets an attribute in the element's own namespace (`w:val` on `w:b`).
    pub fn set_own(&mut self, local: &str, value: &str) {
        if self.attr_ns(NONE, local).is_some() && self.attr_ns(self.ns, local).is_none() {
            self.set_attr(local, NONE, value);
        } else {
            let name = self.qualify(local);
            let ns = self.ns;
            self.set_attr(&name, ns, value);
        }
    }

    /// Sets an unprefixed attribute (`r` on a SpreadsheetML cell).
    pub fn set_plain(&mut self, name: &str, value: &str) {
        self.set_attr(name, NONE, value);
    }

    pub fn remove_attr(&mut self, ns: Ns, local: &str) -> bool {
        let before = self.attrs.len();
        self.attrs.retain(|a| !(a.ns == ns && a.local() == local));
        let removed = self.attrs.len() != before;
        if removed {
            self.raw_open = None;
        }
        removed
    }

    pub fn remove_own(&mut self, local: &str) -> bool {
        let ns = self.ns;
        self.remove_attr(ns, local) | self.remove_attr(NONE, local)
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| n.as_element())
    }
    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| n.as_element_mut())
    }
    pub fn child(&self, ns: Ns, local: &str) -> Option<&Element> {
        self.elements().find(|e| e.is(ns, local))
    }
    pub fn child_mut(&mut self, ns: Ns, local: &str) -> Option<&mut Element> {
        self.elements_mut().find(|e| e.is(ns, local))
    }
    pub fn children_named<'a>(&'a self, ns: Ns, local: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.elements().filter(move |e| e.is(ns, local))
    }
    /// The first element down a path of local names in one namespace.
    pub fn path(&self, ns: Ns, path: &[&str]) -> Option<&Element> {
        let mut e = self;
        for p in path {
            e = e.child(ns, p)?;
        }
        Some(e)
    }

    /// Index (in `children`) of the first child element with this name.
    pub fn position(&self, ns: Ns, local: &str) -> Option<usize> {
        self.children.iter().position(|n| n.as_element().map(|e| e.is(ns, local)).unwrap_or(false))
    }

    pub fn remove_children(&mut self, ns: Ns, local: &str) -> usize {
        let before = self.children.len();
        self.children.retain(|n| !n.as_element().map(|e| e.is(ns, local)).unwrap_or(false));
        before - self.children.len()
    }

    /// All text below this element, in order.
    pub fn text(&self) -> String {
        let mut s = String::new();
        fn walk(e: &Element, s: &mut String) {
            for n in &e.children {
                match n {
                    Node::Text(t) => s.push_str(&t.text),
                    Node::Element(c) => walk(c, s),
                    Node::Other(_) => {}
                }
            }
        }
        walk(self, &mut s);
        s
    }

    /// Replaces the children by one text node.
    pub fn set_text(&mut self, text: &str) {
        if self.children.len() == 1 {
            if let Node::Text(t) = &mut self.children[0] {
                t.set(text);
                return;
            }
        }
        self.children = if text.is_empty() { Vec::new() } else { vec![Node::Text(Text::new(text))] };
    }

    pub fn push(&mut self, e: Element) {
        self.children.push(Node::Element(e));
    }

    /// Every element below this one (depth first, document order), with `self` first.
    pub fn clear_attrs(&mut self) {
        if !self.attrs.is_empty() {
            self.attrs.clear();
            self.raw_open = None;
        }
    }

    pub fn descendants(&self) -> Vec<&Element> {
        let mut out = Vec::new();
        fn walk<'a>(e: &'a Element, out: &mut Vec<&'a Element>) {
            out.push(e);
            for c in e.elements() {
                walk(c, out);
            }
        }
        walk(self, &mut out);
        out
    }

    /// Visits every element below this one mutably (pre-order), self included.
    pub fn walk_mut(&mut self, f: &mut dyn FnMut(&mut Element)) {
        f(self);
        for c in self.elements_mut() {
            c.walk_mut(f);
        }
    }

    /// Whether this element serializes as it was parsed (nothing below it changed).
    pub fn pristine(&self) -> bool {
        self.raw_open.is_some()
            && (self.raw_close.is_some() || self.children.is_empty())
            && self.children.iter().all(|n| match n {
                Node::Element(e) => e.pristine(),
                Node::Text(t) => t.raw.is_some(),
                Node::Other(_) => true,
            })
    }

    /// Forgets the source text of the tags (they are written anew).
    pub fn touch(&mut self) {
        self.raw_open = None;
    }

    pub fn write(&self, out: &mut String) {
        let self_closing_raw = self.raw_open.as_deref().map(|r| r.ends_with("/>")).unwrap_or(false);
        match &self.raw_open {
            Some(raw) if !(self_closing_raw && !self.children.is_empty()) => out.push_str(raw),
            Some(raw) => {
                // Was <x/>, now has children.
                out.push_str(raw[..raw.len() - 2].trim_end());
                out.push('>');
            }
            None => {
                out.push('<');
                out.push_str(&self.name);
                for a in &self.attrs {
                    out.push(' ');
                    out.push_str(&a.name);
                    out.push_str("=\"");
                    escape_attr(&a.value, out);
                    out.push('"');
                }
                if self.children.is_empty() {
                    out.push_str("/>");
                    return;
                }
                out.push('>');
            }
        }
        if self.children.is_empty() && self.raw_open.as_deref().map(|r| r.ends_with("/>")).unwrap_or(false) {
            return;
        }
        for n in &self.children {
            match n {
                Node::Element(e) => e.write(out),
                Node::Text(t) => match &t.raw {
                    Some(r) => out.push_str(r),
                    None => escape_text(&t.text, out),
                },
                Node::Other(o) => out.push_str(o),
            }
        }
        match &self.raw_close {
            Some(r) => out.push_str(r),
            None => {
                out.push_str("</");
                out.push_str(&self.name);
                out.push('>');
            }
        }
    }

    pub fn to_xml(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }
}

#[derive(Clone)]
pub struct Document {
    pub before: Vec<Node>,
    pub root: Element,
    pub after: Vec<Node>,
}

impl Document {
    pub fn parse(src: &str) -> Result<Document> {
        Parser { s: src, i: 0, scopes: Vec::new() }.document()
    }

    pub fn to_string(&self) -> String {
        let mut s = String::with_capacity(4096);
        for n in &self.before {
            write_node(n, &mut s);
        }
        self.root.write(&mut s);
        for n in &self.after {
            write_node(n, &mut s);
        }
        s
    }

    /// The prefix the root element declares for a namespace; declares the usual one when there
    /// is none.
    pub fn prefix_for(&mut self, ns: Ns) -> String {
        if self.root.ns == ns {
            return self.root.prefix().to_string();
        }
        for a in &self.root.attrs {
            if a.name.starts_with("xmlns:") && Ns::intern(&a.value) == ns {
                return a.name[6..].to_string();
            }
        }
        let p = ns.usual_prefix().to_string();
        let uri = ns.uri();
        self.root.set_attr(&format!("xmlns:{p}"), XMLNS, &uri);
        p
    }
}

fn write_node(n: &Node, s: &mut String) {
    match n {
        Node::Element(e) => e.write(s),
        Node::Text(t) => match &t.raw {
            Some(r) => s.push_str(r),
            None => escape_text(&t.text, s),
        },
        Node::Other(o) => s.push_str(o),
    }
}

fn valid_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{FFFE}' && c != '\u{FFFF}')
}

pub fn escape_text(t: &str, out: &mut String) {
    for c in t.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if valid_char(c) => out.push(c),
            _ => {}
        }
    }
}

pub fn escape_attr(t: &str, out: &mut String) {
    for c in t.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if valid_char(c) => out.push(c),
            _ => {}
        }
    }
}

pub fn escape_attr_str(t: &str) -> String {
    let mut o = String::new();
    escape_attr(t, &mut o);
    o
}

pub fn escape(t: &str) -> String {
    let mut s = String::new();
    escape_text(t, &mut s);
    s
}

fn decode_entities(raw: &str) -> Result<String> {
    if !raw.contains('&') {
        return Ok(raw.to_string());
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').ok_or_else(|| err("XML: unterminated entity"))?;
        let ent = &rest[1..end];
        match ent {
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "amp" => out.push('&'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                let v = u32::from_str_radix(&ent[2..], 16).map_err(|_| err("XML: bad character reference"))?;
                out.push(char::from_u32(v).unwrap_or('\u{FFFD}'));
            }
            _ if ent.starts_with('#') => {
                let v: u32 = ent[1..].parse().map_err(|_| err("XML: bad character reference"))?;
                out.push(char::from_u32(v).unwrap_or('\u{FFFD}'));
            }
            // Undeclared entities (HTML ones in hand-made files): kept as written.
            _ => out.push_str(&rest[..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
    scopes: Vec<Vec<(String, Ns)>>,
}

impl<'a> Parser<'a> {
    fn resolve(&self, prefix: &str) -> Ns {
        match prefix {
            "xml" => return XML,
            "xmlns" => return XMLNS,
            _ => {}
        }
        for scope in self.scopes.iter().rev() {
            if let Some((_, ns)) = scope.iter().rev().find(|(p, _)| p == prefix) {
                return *ns;
            }
        }
        NONE
    }

    fn misc(&mut self, into: &mut Vec<Node>) -> Result<bool> {
        // Text (whitespace), comments, PIs and doctype outside the root. True when a `<` of an
        // element is next.
        loop {
            let rest = &self.s[self.i..];
            if rest.is_empty() {
                return Ok(false);
            }
            if rest.starts_with("<?") || rest.starts_with("<!--") || rest.starts_with("<!") {
                let n = self.other()?;
                into.push(Node::Other(n));
            } else if rest.starts_with('<') {
                return Ok(true);
            } else {
                let end = rest.find('<').unwrap_or(rest.len());
                into.push(Node::Other(rest[..end].to_string()));
                self.i += end;
            }
        }
    }

    fn other(&mut self) -> Result<String> {
        let rest = &self.s[self.i..];
        let end = if rest.starts_with("<?") {
            rest.find("?>").map(|e| e + 2)
        } else if rest.starts_with("<!--") {
            rest.find("-->").map(|e| e + 3)
        } else {
            // <!DOCTYPE ... [ ... ]>
            let mut depth = 0;
            let mut end = None;
            for (k, c) in rest.char_indices() {
                match c {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    '>' if depth == 0 => {
                        end = Some(k + 1);
                        break;
                    }
                    _ => {}
                }
            }
            end
        }
        .ok_or_else(|| err("XML: unterminated markup"))?;
        self.i += end;
        Ok(rest[..end].to_string())
    }

    fn document(mut self) -> Result<Document> {
        let mut before = Vec::new();
        if !self.misc(&mut before)? {
            return Err(err("XML: no root element"));
        }
        let root = self.element()?;
        let mut after = Vec::new();
        self.misc(&mut after)?;
        Ok(Document { before, root, after })
    }

    /// Parses the start tag at `self.i`: (qualified name, attributes, self-closing, raw).
    fn start_tag(&mut self) -> Result<(String, Vec<(String, String)>, bool, String)> {
        let s = self.s.as_bytes();
        let start = self.i;
        let mut i = self.i + 1;
        let name_start = i;
        while i < s.len() && !matches!(s[i], b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>') {
            i += 1;
        }
        let name = self.s[name_start..i].to_string();
        let mut attrs = Vec::new();
        loop {
            while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
            if i >= s.len() {
                return Err(err("XML: unterminated start tag"));
            }
            if s[i] == b'>' {
                i += 1;
                self.i = i;
                return Ok((name, attrs, false, self.s[start..i].to_string()));
            }
            if s[i] == b'/' && s.get(i + 1) == Some(&b'>') {
                i += 2;
                self.i = i;
                return Ok((name, attrs, true, self.s[start..i].to_string()));
            }
            let an = i;
            while i < s.len() && !matches!(s[i], b'=' | b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/') {
                i += 1;
            }
            let aname = self.s[an..i].to_string();
            while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
            if s.get(i) != Some(&b'=') {
                return Err(err(format!("XML: attribute {aname} without a value in <{name}>")));
            }
            i += 1;
            while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
            let q = *s.get(i).ok_or_else(|| err("XML: unterminated attribute"))?;
            if q != b'"' && q != b'\'' {
                return Err(err("XML: unquoted attribute value"));
            }
            i += 1;
            let vs = i;
            while i < s.len() && s[i] != q {
                i += 1;
            }
            if i >= s.len() {
                return Err(err("XML: unterminated attribute value"));
            }
            let raw = &self.s[vs..i];
            i += 1;
            // Attribute value normalization: literal whitespace characters become spaces.
            let v = decode_entities(&raw.replace(['\t', '\n', '\r'], " "))?;
            attrs.push((aname, v));
        }
    }

    fn element(&mut self) -> Result<Element> {
        let (name, raw_attrs, self_closing, raw_open) = self.start_tag()?;
        let mut scope = Vec::new();
        for (n, v) in &raw_attrs {
            if n == "xmlns" {
                scope.push((String::new(), Ns::intern(v)));
            } else if let Some(p) = n.strip_prefix("xmlns:") {
                scope.push((p.to_string(), Ns::intern(v)));
            }
        }
        self.scopes.push(scope);
        let prefix = name.find(':').map(|k| &name[..k]).unwrap_or("");
        let ns = self.resolve(prefix);
        let attrs = raw_attrs
            .into_iter()
            .map(|(n, value)| {
                let ns = if n == "xmlns" || n.starts_with("xmlns:") {
                    XMLNS
                } else if let Some(k) = n.find(':') {
                    self.resolve(&n[..k])
                } else {
                    NONE
                };
                Attr { name: n, ns, value }
            })
            .collect();
        let mut el = Element { name, ns, attrs, children: Vec::new(), raw_open: Some(raw_open), raw_close: None };
        if self_closing {
            self.scopes.pop();
            return Ok(el);
        }
        loop {
            let rest = &self.s[self.i..];
            if rest.is_empty() {
                return Err(err(format!("XML: <{}> is not closed", el.name)));
            }
            if rest.starts_with("</") {
                let end = rest.find('>').ok_or_else(|| err("XML: unterminated end tag"))?;
                let close = &rest[..=end];
                let cname = close[2..close.len() - 1].trim();
                if cname != el.name {
                    return Err(err(format!("XML: </{cname}> closes <{}>", el.name)));
                }
                el.raw_close = Some(close.to_string());
                self.i += end + 1;
                self.scopes.pop();
                return Ok(el);
            } else if rest.starts_with("<![CDATA[") {
                let end = rest.find("]]>").ok_or_else(|| err("XML: unterminated CDATA"))?;
                let raw = &rest[..end + 3];
                el.children.push(Node::Text(Text { text: rest[9..end].to_string(), raw: Some(raw.to_string()) }));
                self.i += end + 3;
            } else if rest.starts_with("<!--") || rest.starts_with("<?") {
                let o = self.other()?;
                el.children.push(Node::Other(o));
            } else if rest.starts_with('<') {
                let c = self.element()?;
                el.children.push(Node::Element(c));
            } else {
                let end = rest.find('<').unwrap_or(rest.len());
                let raw = &rest[..end];
                el.children.push(Node::Text(Text { text: decode_entities(raw)?, raw: Some(raw.to_string()) }));
                self.i += end;
            }
        }
    }
}

/// Parses a fragment of elements written with the given prefixes bound (for templates):
/// `fragment("<w:r><w:t>x</w:t></w:r>", &[("w", W)])`.
pub fn fragment(src: &str, bindings: &[(&str, Ns)]) -> Result<Vec<Element>> {
    let decls: String = bindings.iter().map(|(p, ns)| format!(" xmlns:{p}=\"{}\"", ns.uri())).collect();
    let wrapped = format!("<frag{decls}>{src}</frag>");
    let doc = Document::parse(&wrapped)?;
    let mut out = Vec::new();
    for n in doc.root.children {
        if let Node::Element(mut e) = n {
            // Written anew, so the declarations of the wrapper are not needed in the output.
            fn forget(e: &mut Element) {
                e.raw_open = None;
                e.raw_close = None;
                for c in e.elements_mut() {
                    forget(c);
                }
                for c in e.children.iter_mut() {
                    if let Node::Text(t) = c {
                        t.raw = None;
                    }
                }
            }
            forget(&mut e);
            out.push(e);
        }
    }
    Ok(out)
}

/// The order of child elements a schema sequence prescribes: inserts `el` before the first
/// existing child that comes later in `order` (or last), replacing a child of the same name.
pub fn put_in_order(parent: &mut Element, el: Element, order: &[&str]) {
    let local = el.local().to_string();
    let ns = el.ns();
    if let Some(i) = parent.children.iter().position(|n| n.as_element().map(|e| e.is(ns, &local)).unwrap_or(false)) {
        parent.children[i] = Node::Element(el);
        return;
    }
    let rank = |name: &str| order.iter().position(|o| *o == name);
    let mine = rank(&local).unwrap_or(order.len());
    let at = parent
        .children
        .iter()
        .position(|n| match n.as_element() {
            Some(e) => rank(e.local()).map(|r| r > mine).unwrap_or(false),
            None => false,
        })
        .unwrap_or(parent.children.len());
    parent.children.insert(at, Node::Element(el));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_is_exact() {
        let src = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" ><w:body><w:p w:rsidR='00A1'><w:r><w:t xml:space=\"preserve\"> a &amp; b &#x41;</w:t></w:r><!-- c --><w:r/></w:p></w:body ></w:document>";
        let d = Document::parse(src).unwrap();
        assert_eq!(d.to_string(), src);
        let p = d.root.child(W, "body").unwrap().child(W, "p").unwrap();
        assert_eq!(p.text(), " a & b A");
        assert_eq!(p.attr_ns(W, "rsidR"), Some("00A1"));
    }

    #[test]
    fn edits_touch_only_their_element() {
        let src = "<a:x xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><a:p  q='1'/><a:t>x</a:t></a:x>";
        let mut d = Document::parse(src).unwrap();
        d.root.children[1].as_element_mut().unwrap().set_text("y<");
        assert_eq!(d.to_string(), "<a:x xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><a:p  q='1'/><a:t>y&lt;</a:t></a:x>");
        d.root.children[0].as_element_mut().unwrap().set_own("q", "2");
        assert!(d.to_string().contains("<a:p q=\"2\"/>"));
    }

    #[test]
    fn strict_namespaces_resolve() {
        let src = "<w:document xmlns:w=\"http://purl.oclc.org/ooxml/wordprocessingml/main\"/>";
        let d = Document::parse(src).unwrap();
        assert!(d.root.is(W, "document"));
    }
}
