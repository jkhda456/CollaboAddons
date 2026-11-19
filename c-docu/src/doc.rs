//! Opening a file as whatever document it is, and the operations every format answers to.

use crate::docx::Docx;
use crate::ppt::Ppt;
use crate::pptx::Pptx;
use crate::word97::Doc;
use crate::xls::Xls;
use crate::xlsx::Xlsx;
use crate::model::{Match, Outline};
use crate::opc::Package;
use crate::props::Props;
use crate::text::Finder;
use crate::zip::{self, Zip};
use crate::{err, Result};

pub enum AnyDoc {
    Docx(Docx),
    Xlsx(Xlsx),
    Pptx(Pptx),
    Xls(Xls),
    Ppt(Ppt),
    Doc(Doc),
}

pub const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

impl AnyDoc {
    pub fn open(path: &str) -> Result<AnyDoc> {
        let data = std::fs::read(path).map_err(|e| err(format!("{path}: {e}")))?;
        Self::from_bytes(data, path)
    }

    pub fn from_bytes(data: Vec<u8>, path: &str) -> Result<AnyDoc> {
        if zip::is_zip(&data) {
            let z = Zip::open(data)?;
            let pkg = Package::open(z)?;
            if let Some(main) = pkg.main_part() {
                let ct = pkg.content_type(&main).unwrap_or_default();
                if ct.contains("wordprocessingml") || ct.contains("ms-word") {
                    return Ok(AnyDoc::Docx(Docx::open(pkg, &main)?));
                }
                if ct.contains("spreadsheetml") || ct.contains("ms-excel") {
                    return Ok(AnyDoc::Xlsx(Xlsx::open(pkg, &main)?));
                }
                if ct.contains("presentationml") || ct.contains("ms-powerpoint") {
                    return Ok(AnyDoc::Pptx(Pptx::open(pkg, &main)?));
                }
                return Err(err(format!("{path}: an OOXML package of a kind not supported yet ({ct})")));
            }
            return Err(err(format!("{path}: a zip archive, but not a document this tool knows")));
        }
        if data.starts_with(&CFB_MAGIC) {
            let c = crate::cfb::Cfb::open(&data)?;
            if c.stream("EncryptedPackage").is_some() {
                return Err(err(format!("{path}: password-protected (encrypted); not supported")));
            }
            if c.stream("Workbook").is_some() || c.stream("Book").is_some() {
                return Ok(AnyDoc::Xls(Xls::open(c)?));
            }
            if c.stream("PowerPoint Document").is_some() {
                return Ok(AnyDoc::Ppt(Ppt::open(c)?));
            }
            if c.stream("WordDocument").is_some() {
                return Ok(AnyDoc::Doc(Doc::open(c)?));
            }
            return Err(err(format!("{path}: a binary (OLE) document of a kind not supported yet ({})", c.names().join(", "))));
        }
        Err(err(format!("{path}: not a document this tool knows (docx, xlsx, pptx, doc, xls, ppt, odt, ods, odp, hwp, hwpx)")))
    }

    pub fn kind(&self) -> &'static str {
        match self {
            AnyDoc::Docx(d) => d.kind,
            AnyDoc::Xlsx(d) => d.kind,
            AnyDoc::Pptx(d) => d.kind,
            AnyDoc::Xls(d) => d.kind,
            AnyDoc::Ppt(d) => d.kind,
            AnyDoc::Doc(d) => d.kind,
        }
    }

    /// The content with addresses; `filter` limits it to some addresses (ranges for sheets).
    pub fn outline(&self, runs: bool, filter: &[String]) -> Result<Outline> {
        Ok(match self {
            AnyDoc::Docx(d) => filter_outline(d.outline(runs), filter),
            AnyDoc::Xlsx(d) => d.outline(runs, filter)?,
            AnyDoc::Pptx(d) => d.outline(runs, filter)?,
            AnyDoc::Xls(d) => d.outline(runs, filter)?,
            AnyDoc::Ppt(d) => d.outline(runs, filter)?,
            AnyDoc::Doc(d) => d.outline(runs, filter)?,
        })
    }

    pub fn plain(&self) -> Result<String> {
        Ok(match self {
            AnyDoc::Xlsx(d) => d.plain(),
            AnyDoc::Xls(d) => d.plain(),
            AnyDoc::Ppt(d) => d.plain(),
            AnyDoc::Doc(d) => d.plain(),
            _ => {
                let mut s = String::new();
                self.outline(false, &[])?.plain(&mut s);
                s
            }
        })
    }

    pub fn find(&self, f: &Finder, scope: Option<&str>, formulas: bool) -> Result<Vec<Match>> {
        match self {
            AnyDoc::Docx(d) => d.find(f, scope),
            AnyDoc::Xlsx(d) => d.find(f, scope, formulas),
            AnyDoc::Pptx(d) => d.find(f, scope),
            AnyDoc::Xls(d) => d.find(f, scope, formulas),
            AnyDoc::Ppt(d) => d.find(f, scope),
            AnyDoc::Doc(d) => d.find(f, scope),
        }
    }

    pub fn replace(&mut self, f: &Finder, with: &str, scope: Option<&str>, nth: Option<usize>, formulas: bool) -> Result<Vec<String>> {
        match self {
            AnyDoc::Docx(d) => d.replace(f, with, scope, nth),
            AnyDoc::Xlsx(d) => d.replace(f, with, scope, nth, formulas),
            AnyDoc::Pptx(d) => d.replace(f, with, scope, nth),
            AnyDoc::Xls(d) => d.replace(f, with, scope, nth),
            AnyDoc::Ppt(d) => d.replace(f, with, scope, nth),
            AnyDoc::Doc(d) => d.replace(f, with, scope, nth),
        }
    }

    pub fn set(&mut self, addr: &str, value: &str) -> Result<String> {
        match self {
            AnyDoc::Docx(d) => d.set_text(addr, value),
            AnyDoc::Xlsx(d) => d.set(addr, value),
            AnyDoc::Pptx(d) => d.set_text(addr, value),
            AnyDoc::Xls(d) => d.set(addr, value),
            AnyDoc::Ppt(d) => d.set_text(addr, value),
            AnyDoc::Doc(d) => d.set_text(addr, value),
        }
    }

    pub fn style(&mut self, addr: &str, props: &Props, matching: Option<(&Finder, Option<usize>)>) -> Result<Vec<String>> {
        match self {
            AnyDoc::Docx(d) => d.style(addr, props, matching),
            AnyDoc::Xlsx(d) => d.style(addr, props, matching),
            AnyDoc::Pptx(d) => d.style(addr, props, matching),
            AnyDoc::Xls(_) => Err(err("styles in .xls files are not changed by this tool yet (convert to .xlsx)")),
            AnyDoc::Ppt(d) => d.style(addr, props, matching),
            AnyDoc::Doc(d) => d.style(addr, props, matching),
        }
    }

    pub fn insert(&mut self, addr: &str, after: bool, text: &str, props: Option<&Props>, count: usize) -> Result<Vec<String>> {
        match self {
            AnyDoc::Docx(d) => d.insert(addr, after, text, props),
            AnyDoc::Xlsx(d) => d.insert(addr, after, text, count),
            AnyDoc::Pptx(d) => d.insert(addr, after, text, props),
            AnyDoc::Xls(_) => Err(err("inserting rows in .xls files is not supported yet")),
            AnyDoc::Ppt(_) => Err(err("inserting into .ppt files is not supported (set or replace text instead)")),
            AnyDoc::Doc(d) => d.insert(addr, after, text),
        }
    }

    pub fn delete(&mut self, addr: &str) -> Result<String> {
        match self {
            AnyDoc::Docx(d) => d.delete(addr),
            AnyDoc::Xlsx(d) => d.delete(addr),
            AnyDoc::Pptx(d) => d.delete(addr),
            AnyDoc::Xls(d) => d.delete(addr),
            AnyDoc::Ppt(d) => d.delete(addr),
            AnyDoc::Doc(d) => d.delete(addr),
        }
    }

    pub fn insert_table(&mut self, addr: &str, after: bool, rows: &[Vec<String>], header: bool) -> Result<String> {
        match self {
            AnyDoc::Docx(d) => d.insert_table(addr, after, rows, header),
            _ => Err(err("table is for documents (in a sheet, set a range: docu set FILE A1 \"a|b;c|d\")")),
        }
    }

    pub fn recalc(&mut self) -> Result<(usize, Vec<String>)> {
        match self {
            AnyDoc::Xlsx(d) => {
                let n = d.recalc(true);
                Ok((n, std::mem::take(&mut d.warnings)))
            }
            AnyDoc::Xls(d) => {
                let n = d.recalc(true);
                Ok((n, std::mem::take(&mut d.warnings)))
            }
            _ => Err(err("recalc is for workbooks")),
        }
    }

    pub fn warnings(&mut self) -> Vec<String> {
        match self {
            AnyDoc::Xlsx(d) => std::mem::take(&mut d.warnings),
            AnyDoc::Xls(d) => std::mem::take(&mut d.warnings),
            AnyDoc::Docx(d) => std::mem::take(&mut d.warnings),
            _ => Vec::new(),
        }
    }

    pub fn save(&mut self) -> Result<Vec<u8>> {
        match self {
            AnyDoc::Docx(d) => d.save(),
            AnyDoc::Xlsx(d) => d.save(),
            AnyDoc::Pptx(d) => d.save(),
            AnyDoc::Xls(d) => d.save(),
            AnyDoc::Ppt(d) => d.save(),
            AnyDoc::Doc(d) => d.save(),
        }
    }
}

/// Keeps the items an address list names (and what is inside them).
pub fn filter_outline(o: Outline, addrs: &[String]) -> Outline {
    if addrs.is_empty() {
        return o;
    }
    let keep = |addr: &str| {
        addrs.iter().any(|a| {
            addr == a || addr.starts_with(&format!("{a}.")) || {
                if let (Some(n), Some((x, y))) = (addr.strip_prefix('p').and_then(|n| n.parse::<usize>().ok()), a.strip_prefix('p').and_then(|r| r.split_once('-'))) {
                    matches!((x.parse::<usize>(), y.parse::<usize>()), (Ok(x), Ok(y)) if n >= x && n <= y)
                } else {
                    false
                }
            }
        })
    };
    Outline { items: o.items.into_iter().filter(|it| keep(&it.addr) || it.place.as_deref().map(|p| keep(p)).unwrap_or(false)).collect() }
}
