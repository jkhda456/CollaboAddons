//! The archive file (`.gar`): compressed blocks of pages, then a compressed directory naming
//! every page, where it is, and where it came from.
//!
//! ```text
//! "GARROW\0" version(1)
//! block 0 .. block n-1                  codec output, back to back
//! directory                             codec output (no dictionary)
//! trailer: directory offset u64, compressed u32, raw u32, CRC-32 u32, "GAR!"   (all LE)
//! ```
//!
//! The directory lists the sources (upstream, revision, license), the languages and platforms,
//! the blocks (offset, sizes, CRC-32 of the raw text, the block whose text is the preset
//! dictionary, language, page count) and the pages in block order, column by column (names
//! front-coded, then sources, kinds, platforms). A block's text is its pages joined by NUL
//! bytes — page text never has one — so no lengths or offsets are stored.

use crate::codec::{self, Params};
use crate::{err, Result};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::rc::Rc;

pub const MAGIC: &[u8; 8] = b"GARROW\0\x01";
pub const TRAILER_MAGIC: &[u8; 4] = b"GAR!";
const TRAILER: usize = 24;

/// The kinds of page, which say how to read one (and where it was upstream).
pub const KIND_TLDR: u8 = 0; // a tldr page (Markdown)
pub const KIND_SHEET: u8 = 1; // a cheat sheet: comment lines and the commands under them
pub const KIND_LANG: u8 = 2; // a programming language's sheet (cheat.sheets' sheets/_LANG/TOPIC)
pub const KIND_TOPIC: u8 = 3; // the tools for a task (cheat.sheets' topics/*.json)
pub const KIND_SEE_ALSO: u8 = 4; // related commands (cheat.sheets' see_also/*)

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Source {
    /// tldr, cheat.sheets, cheatsheets, a folder packed under a name of its own, or `local`
    /// (sheets added with --add, local.rs)
    pub id: String,
    pub title: String,
    pub url: String,
    pub revision: String,
    pub date: String,
    pub license: String,
    /// A page's upstream file: `{rev}` and `{path}` are filled in.
    pub file_url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlockInfo {
    pub offset: u64,
    pub comp_len: u32,
    pub raw_len: u32,
    pub crc: u32,
    /// The block whose raw text comes before this one's (its preset dictionary).
    pub dict: Option<u32>,
    pub lang: u16,
    pub pages: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageRef {
    pub name: String,
    pub source: u16,
    pub kind: u8,
    pub platform: u16,
    pub lang: u16,
    pub block: u32,
    /// Which page of its block (the text between the index-th and the next NUL).
    pub index: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Directory {
    pub params: Params,
    pub meta: Vec<(String, String)>,
    pub sources: Vec<Source>,
    pub langs: Vec<String>,
    pub platforms: Vec<String>,
    pub blocks: Vec<BlockInfo>,
    pub pages: Vec<PageRef>,
}

pub struct Archive {
    pub path: String,
    pub size: u64,
    pub dir: Directory,
    /// How many of `dir`'s sources, blocks and pages are the file's own; the rest were added
    /// from folders ([`Archive::add_pages`]: sheets of one's own).
    pub own_sources: usize,
    pub own_blocks: usize,
    pub own_pages: usize,
    file: RefCell<File>,
    cache: RefCell<HashMap<u32, Rc<Block>>>,
}

/// A decompressed block and where each of its pages is.
pub struct Block {
    pub raw: Vec<u8>,
    pub spans: Vec<(usize, usize)>,
}

impl Archive {
    pub fn open(path: &str) -> Result<Archive> {
        let mut f = File::open(path).map_err(|e| err!("{path}: {e}"))?;
        let size = f.seek(SeekFrom::End(0))?;
        let bad = || err!("{path}: not a guided-arrow archive (or damaged)");
        if size < (MAGIC.len() + TRAILER) as u64 {
            return Err(bad());
        }
        let mut head = [0u8; 8];
        f.seek(SeekFrom::Start(0))?;
        f.read_exact(&mut head)?;
        if &head[..7] != &MAGIC[..7] {
            return Err(bad());
        }
        if head[7] != MAGIC[7] {
            return Err(err!("{path}: archive format version {} (this guided-arrow reads {})", head[7], MAGIC[7]));
        }
        let mut t = [0u8; TRAILER];
        f.seek(SeekFrom::End(-(TRAILER as i64)))?;
        f.read_exact(&mut t)?;
        if &t[20..24] != TRAILER_MAGIC {
            return Err(bad());
        }
        let off = u64::from_le_bytes(t[0..8].try_into().unwrap());
        let clen = u32::from_le_bytes(t[8..12].try_into().unwrap()) as u64;
        let rlen = u32::from_le_bytes(t[12..16].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(t[16..20].try_into().unwrap());
        if off < MAGIC.len() as u64 || off + clen + TRAILER as u64 != size {
            return Err(bad());
        }
        let mut comp = vec![0u8; clen as usize];
        f.seek(SeekFrom::Start(off))?;
        f.read_exact(&mut comp)?;
        let raw = codec::decompress(b"", &comp, rlen, Params::TEXT).map_err(|_| bad())?;
        if crc32(&raw) != crc {
            return Err(err!("{path}: the directory fails its checksum"));
        }
        let dir = Directory::decode(&raw).map_err(|e| err!("{path}: {e}"))?;
        for b in &dir.blocks {
            if b.offset + b.comp_len as u64 > off {
                return Err(bad());
            }
        }
        let (own_sources, own_blocks, own_pages) = (dir.sources.len(), dir.blocks.len(), dir.pages.len());
        Ok(Archive {
            path: path.to_string(),
            size,
            dir,
            own_sources,
            own_blocks,
            own_pages,
            file: RefCell::new(f),
            cache: RefCell::new(HashMap::new()),
        })
    }

    /// Pages from outside the file (`(name, kind, text)`, English, no platform) as one more
    /// source. They get a block of their own that is already in the cache, so everything reads
    /// them as it reads the archive's. A source with no pages is listed all the same.
    pub fn add_pages(&mut self, source: Source, pages: Vec<(String, u8, Vec<u8>)>) -> Result<()> {
        let src = self.dir.sources.len() as u16;
        self.dir.sources.push(source);
        if pages.is_empty() {
            return Ok(());
        }
        fn index(list: &mut Vec<String>, v: &str) -> u16 {
            match list.iter().position(|x| x == v) {
                Some(i) => i as u16,
                None => {
                    list.push(v.to_string());
                    (list.len() - 1) as u16
                }
            }
        }
        let lang = index(&mut self.dir.langs, "en");
        let platform = index(&mut self.dir.platforms, "");
        let block = self.dir.blocks.len() as u32;
        let (mut raw, mut spans) = (Vec::new(), Vec::new());
        for (i, (name, kind, text)) in pages.into_iter().enumerate() {
            if text.contains(&0) {
                return Err(err!("{name}: has a NUL byte"));
            }
            if i > 0 {
                raw.push(0);
            }
            spans.push((raw.len(), raw.len() + text.len()));
            raw.extend_from_slice(&text);
            self.dir.pages.push(PageRef { name, source: src, kind, platform, lang, block, index: i as u32 });
        }
        self.dir.blocks.push(BlockInfo {
            offset: 0,
            comp_len: 0,
            raw_len: raw.len() as u32,
            crc: crc32(&raw),
            dict: None,
            lang,
            pages: spans.len() as u32,
        });
        self.cache.borrow_mut().insert(block, Rc::new(Block { raw, spans }));
        Ok(())
    }

    /// A page added from a folder, not the archive's own.
    pub fn is_added(&self, p: &PageRef) -> bool {
        p.source as usize >= self.own_sources
    }

    /// A block's raw text, checked against its CRC-32.
    pub fn block(&self, i: u32) -> Result<Rc<Block>> {
        if let Some(b) = self.cache.borrow().get(&i) {
            return Ok(b.clone());
        }
        let info = self.dir.blocks.get(i as usize).ok_or_else(|| err!("no block {i}"))?.clone();
        let dict = match info.dict {
            Some(d) if d < i => Some(self.block(d)?),
            Some(_) => return Err(err!("{}: block {i} has a bad dictionary", self.path)),
            None => None,
        };
        let mut comp = vec![0u8; info.comp_len as usize];
        {
            let mut f = self.file.borrow_mut();
            f.seek(SeekFrom::Start(info.offset))?;
            f.read_exact(&mut comp)?;
        }
        let dict: &[u8] = dict.as_deref().map_or(b"", |d| d.raw.as_slice());
        let raw = codec::decompress(dict, &comp, info.raw_len as usize, self.dir.params)
            .map_err(|e| err!("{}: block {i}: {e}", self.path))?;
        if crc32(&raw) != info.crc {
            return Err(err!("{}: block {i} fails its checksum", self.path));
        }
        let mut spans = Vec::with_capacity(info.pages as usize);
        let mut start = 0;
        for (j, &b) in raw.iter().enumerate() {
            if b == 0 {
                spans.push((start, j));
                start = j + 1;
            }
        }
        spans.push((start, raw.len()));
        if spans.len() != info.pages as usize {
            return Err(err!("{}: block {i} holds {} pages, not {}", self.path, spans.len(), info.pages));
        }
        let b = Rc::new(Block { raw, spans });
        self.cache.borrow_mut().insert(i, b.clone());
        Ok(b)
    }

    pub fn text(&self, p: &PageRef) -> Result<String> {
        let b = self.block(p.block)?;
        let &(a, z) = b.spans.get(p.index as usize).ok_or_else(|| err!("{}: page {} is not in its block", self.path, p.name))?;
        String::from_utf8(b.raw[a..z].to_vec()).map_err(|_| err!("{}: page {} is not UTF-8", self.path, p.name))
    }

    pub fn lang(&self, p: &PageRef) -> &str {
        &self.dir.langs[p.lang as usize]
    }
    pub fn platform(&self, p: &PageRef) -> &str {
        &self.dir.platforms[p.platform as usize]
    }
    pub fn source(&self, p: &PageRef) -> &Source {
        &self.dir.sources[p.source as usize]
    }

    /// Where the page is in its upstream repository (for one added from a folder: its file).
    pub fn upstream_path(&self, p: &PageRef) -> String {
        if self.is_added(p) {
            return format!("{}/{}", self.source(p).url.trim_end_matches('/'), p.name);
        }
        upstream_path(&self.source(p).id, p.kind, self.lang(p), self.platform(p), &p.name)
    }

    pub fn upstream_url(&self, p: &PageRef) -> String {
        let s = self.source(p);
        if s.file_url.is_empty() {
            return String::new();
        }
        let path: String = self
            .upstream_path(p)
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b'~' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect();
        s.file_url.replace("{rev}", &s.revision).replace("{path}", &path)
    }
}

pub fn upstream_path(source: &str, kind: u8, lang: &str, platform: &str, name: &str) -> String {
    match kind {
        KIND_TLDR => {
            let dir = if lang == "en" { "pages".to_string() } else { format!("pages.{lang}") };
            format!("{dir}/{platform}/{name}.md")
        }
        KIND_LANG => match name.split_once('/') {
            Some((l, t)) => format!("sheets/_{l}/{t}"),
            None => format!("sheets/_{name}"),
        },
        KIND_TOPIC => format!("topics/{name}.json"),
        KIND_SEE_ALSO => format!("see_also/{name}"),
        _ if source == "cheat.sheets" => format!("sheets/{name}"),
        _ => name.to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// The directory's encoding: LEB128 numbers, strings as length + UTF-8.

pub struct Writer(pub Vec<u8>);

impl Writer {
    pub fn num(&mut self, mut v: u64) {
        loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                self.0.push(b);
                return;
            }
            self.0.push(b | 0x80);
        }
    }
    pub fn str(&mut self, s: &str) {
        self.num(s.len() as u64);
        self.0.extend_from_slice(s.as_bytes());
    }
}

pub struct Reader<'a> {
    pub buf: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn num(&mut self) -> Result<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = *self.buf.get(self.pos).ok_or_else(|| err!("truncated directory"))?;
            self.pos += 1;
            v |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(err!("bad number in the directory"))
    }
    pub fn small(&mut self, max: u64) -> Result<u64> {
        let v = self.num()?;
        if v > max {
            return Err(err!("bad directory (value {v} > {max})"));
        }
        Ok(v)
    }
    pub fn str(&mut self) -> Result<String> {
        let n = self.small(1 << 24)? as usize;
        let s = self.buf.get(self.pos..self.pos + n).ok_or_else(|| err!("truncated directory"))?;
        self.pos += n;
        String::from_utf8(s.to_vec()).map_err(|_| err!("bad string in the directory"))
    }
}

impl Directory {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer(Vec::new());
        w.0.push(self.params.to_byte());
        w.num(self.meta.len() as u64);
        for (k, v) in &self.meta {
            w.str(k);
            w.str(v);
        }
        w.num(self.sources.len() as u64);
        for s in &self.sources {
            for f in [&s.id, &s.title, &s.url, &s.revision, &s.date, &s.license, &s.file_url] {
                w.str(f);
            }
        }
        for list in [&self.langs, &self.platforms] {
            w.num(list.len() as u64);
            for s in list {
                w.str(s);
            }
        }
        w.num(self.blocks.len() as u64);
        for b in &self.blocks {
            w.num(b.offset);
            w.num(b.comp_len as u64);
            w.num(b.raw_len as u64);
            w.0.extend_from_slice(&b.crc.to_le_bytes());
            w.num(b.dict.map_or(0, |d| d as u64 + 1));
            w.num(b.lang as u64);
            w.num(b.pages as u64);
        }
        // Column by column: each compresses best next to its own kind.
        let mut prev = "";
        for p in &self.pages {
            let shared = prev.bytes().zip(p.name.bytes()).take_while(|(a, b)| a == b).count();
            let shared = (0..=shared).rev().find(|&i| p.name.is_char_boundary(i)).unwrap_or(0);
            w.num(shared as u64);
            w.str(&p.name[shared..]);
            prev = &p.name;
        }
        for p in &self.pages {
            w.num(p.source as u64);
        }
        for p in &self.pages {
            w.num(p.kind as u64);
        }
        for p in &self.pages {
            w.num(p.platform as u64);
        }
        w.0
    }

    pub fn decode(buf: &[u8]) -> Result<Directory> {
        let mut r = Reader { buf, pos: 0 };
        let params = buf.first().copied().and_then(Params::from_byte).ok_or_else(|| err!("bad codec parameters"))?;
        r.pos = 1;
        let mut meta = Vec::new();
        for _ in 0..r.small(1 << 16)? {
            meta.push((r.str()?, r.str()?));
        }
        let mut sources = Vec::new();
        for _ in 0..r.small(1 << 16)? {
            sources.push(Source {
                id: r.str()?,
                title: r.str()?,
                url: r.str()?,
                revision: r.str()?,
                date: r.str()?,
                license: r.str()?,
                file_url: r.str()?,
            });
        }
        let mut lists = [Vec::new(), Vec::new()];
        for list in &mut lists {
            for _ in 0..r.small(1 << 16)? {
                list.push(r.str()?);
            }
        }
        let [langs, platforms] = lists;
        let mut blocks = Vec::new();
        for i in 0..r.small(1 << 24)? {
            let offset = r.num()?;
            let comp_len = r.small(u32::MAX as u64)? as u32;
            let raw_len = r.small(u32::MAX as u64)? as u32;
            let crc = buf.get(r.pos..r.pos + 4).ok_or_else(|| err!("truncated directory"))?;
            let crc = u32::from_le_bytes(crc.try_into().unwrap());
            r.pos += 4;
            let dict = match r.small(i)? {
                0 => None,
                d => Some(d as u32 - 1),
            };
            let lang = r.small(langs.len().saturating_sub(1) as u64)? as u16;
            let pages = r.small(u32::MAX as u64)? as u32;
            if pages == 0 {
                return Err(err!("bad directory (an empty block)"));
            }
            blocks.push(BlockInfo { offset, comp_len, raw_len, crc, dict, lang, pages });
        }
        let n: u64 = blocks.iter().map(|b| b.pages as u64).sum();
        let mut pages = Vec::with_capacity(n.min(1 << 24) as usize);
        let mut prev = String::new();
        for (bi, b) in blocks.iter().enumerate() {
            for index in 0..b.pages {
                let shared = r.small(prev.len() as u64)? as usize;
                if !prev.is_char_boundary(shared) {
                    return Err(err!("bad page name in the directory"));
                }
                let name = format!("{}{}", &prev[..shared], r.str()?);
                prev = name.clone();
                pages.push(PageRef { name, source: 0, kind: 0, platform: 0, lang: b.lang, block: bi as u32, index });
            }
        }
        for p in &mut pages {
            p.source = r.small(sources.len().saturating_sub(1) as u64)? as u16;
        }
        for p in &mut pages {
            p.kind = r.small(KIND_SEE_ALSO as u64)? as u8;
        }
        for p in &mut pages {
            p.platform = r.small(platforms.len().saturating_sub(1) as u64)? as u16;
        }
        if r.pos != buf.len() {
            return Err(err!("bad directory (trailing bytes)"));
        }
        Ok(Directory { params, meta, sources, langs, platforms, blocks, pages })
    }
}

// ---------------------------------------------------------------------------------------------

/// CRC-32 (IEEE 802.3, as zlib).
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *v = c;
        }
        t
    });
    let mut c = !0u32;
    for &b in data {
        c = t[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

/// SHA-1, for the git blob id of a page (`git hash-object`): the same id names the file in
/// its upstream repository.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

pub fn git_blob_id(text: &[u8]) -> String {
    let mut obj = format!("blob {}\0", text.len()).into_bytes();
    obj.extend_from_slice(text);
    sha1(&obj).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums() {
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
        assert_eq!(git_blob_id(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
        assert_eq!(git_blob_id(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
        let abc: String = sha1(b"abc").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(abc, "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn directory_roundtrip() {
        let dir = Directory {
            params: Params::TEXT,
            meta: vec![("created".into(), "2026-10-28".into())],
            sources: vec![Source { id: "tldr".into(), revision: "abc".into(), ..Default::default() }],
            langs: vec!["en".into(), "ko".into()],
            platforms: vec!["".into(), "common".into()],
            blocks: vec![
                BlockInfo { offset: 8, comp_len: 10, raw_len: 7, crc: 1, dict: None, lang: 0, pages: 2 },
                BlockInfo { offset: 18, comp_len: 5, raw_len: 4, crc: 2, dict: Some(0), lang: 1, pages: 2 },
            ],
            pages: vec![
                PageRef { name: "tar".into(), source: 0, kind: 0, platform: 1, lang: 0, block: 0, index: 0 },
                PageRef { name: "tarsnap".into(), source: 0, kind: 2, platform: 1, lang: 0, block: 0, index: 1 },
                PageRef { name: "한글".into(), source: 0, kind: 0, platform: 1, lang: 1, block: 1, index: 0 },
                PageRef { name: "한국".into(), source: 0, kind: 0, platform: 0, lang: 1, block: 1, index: 1 },
            ],
        };
        assert_eq!(Directory::decode(&dir.encode()).unwrap(), dir);
        let enc = dir.encode();
        for n in 0..enc.len() {
            assert!(Directory::decode(&enc[..n]).is_err());
        }
    }
}
