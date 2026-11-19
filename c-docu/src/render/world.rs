//! The Typst world the renderer compiles in: one generated source, the document's images as
//! virtual files, and the fonts of the font folders (loaded when a page needs them).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook, FontInfo};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};

struct Slot {
    path: PathBuf,
    index: u32,
    font: OnceLock<Option<Font>>,
}

pub struct Fonts {
    book: LazyHash<FontBook>,
    slots: Vec<Slot>,
}

/// The folders fonts come from: `DOCU_FONT_PATH` (colon-separated) first, then the system's
/// unless `DOCU_SYSTEM_FONTS=0`.
pub fn font_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var("DOCU_FONT_PATH").map(|v| v.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect()).unwrap_or_default();
    if std::env::var("DOCU_SYSTEM_FONTS").map(|v| v != "0").unwrap_or(true) {
        dirs.push("/usr/share/docu/fonts".into());
        dirs.push("/usr/share/fonts".into());
        dirs.push("/usr/local/share/fonts".into());
        if let Ok(h) = std::env::var("HOME") {
            dirs.push(PathBuf::from(&h).join(".local/share/fonts"));
            dirs.push(PathBuf::from(&h).join(".fonts"));
        }
    }
    dirs
}

impl Fonts {
    pub fn discover() -> Fonts {
        let mut book = FontBook::new();
        let mut slots = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut files = Vec::new();
        for d in font_dirs() {
            walk(&d, &mut files, 0);
        }
        for f in files {
            let canon = std::fs::canonicalize(&f).unwrap_or(f.clone());
            if !seen.insert(canon) {
                continue;
            }
            let Ok(data) = std::fs::read(&f) else { continue };
            let bytes = Bytes::new(data);
            for (i, info) in FontInfo::iter(&bytes).enumerate() {
                book.push(info);
                slots.push(Slot { path: f.clone(), index: i as u32, font: OnceLock::new() });
            }
        }
        Fonts { book: LazyHash::new(book), slots }
    }

    pub fn book(&self) -> &FontBook {
        &self.book
    }

    /// Whether a family is there (case-insensitive).
    pub fn has_family(&self, name: &str) -> bool {
        self.book.select_family(&name.to_lowercase()).next().is_some()
    }

    pub fn font(&self, index: usize) -> Option<Font> {
        let slot = self.slots.get(index)?;
        slot.font
            .get_or_init(|| {
                let data = std::fs::read(&slot.path).ok()?;
                Font::new(Bytes::new(data), slot.index)
            })
            .clone()
    }

    /// The first font of a family (for metrics).
    pub fn first_of(&self, family: &str) -> Option<Font> {
        let i = self.book.select_family(&family.to_lowercase()).next()?;
        self.font(i)
    }
}

fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, out, depth + 1);
        } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            if matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc") {
                out.push(p);
            }
        }
    }
}

pub struct DocuWorld<'a> {
    library: LazyHash<Library>,
    fonts: &'a Fonts,
    main: FileId,
    source: Source,
    files: HashMap<FileId, Bytes>,
    today: Option<Datetime>,
}

pub fn file_id(path: &str) -> FileId {
    FileId::new(RootedPath::new(VirtualRoot::Project, VirtualPath::new(path).expect("a virtual path")))
}

impl<'a> DocuWorld<'a> {
    pub fn new(fonts: &'a Fonts, text: String, files: Vec<(String, Vec<u8>)>) -> DocuWorld<'a> {
        let main = file_id("/main.typ");
        let source = Source::new(main, text);
        let files = files.into_iter().map(|(p, b)| (file_id(&format!("/{}", p.trim_start_matches('/'))), Bytes::new(b))).collect();
        let today = {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
            let (y, m, d) = crate::numfmt::civil_from_days(secs.div_euclid(86400));
            Datetime::from_ymd(y as i32, m as u8, d as u8)
        };
        DocuWorld { library: LazyHash::new(Library::default()), fonts, main, source, files, today }
    }

    pub fn source_text(&self) -> &str {
        self.source.text()
    }
}

impl World for DocuWorld<'_> {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &self.fonts.book
    }
    fn main(&self) -> FileId {
        self.main
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main {
            Ok(self.source.clone())
        } else {
            Err(FileError::NotFound(id.vpath().get_without_slash().into()))
        }
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.main {
            return Ok(Bytes::from_string(self.source.clone()));
        }
        self.files.get(&id).cloned().ok_or_else(|| FileError::NotFound(id.vpath().get_without_slash().into()))
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.font(index)
    }
    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        self.today
    }
}
