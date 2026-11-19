//! Rendering: a document becomes Typst markup (pages, positioned shapes, tables, runs with their
//! fonts), which the Typst library lays out and exports as PDF, PNG or SVG.

pub mod typ;
pub mod world;

mod chart;
mod docx;
mod geom;
mod images;
mod office;
mod pptx;
mod shapes;
mod xlsx;

use crate::doc::AnyDoc;
use crate::{err, Result};
use typst::layout::PageRanges;
use typst_layout::PagedDocument;

pub struct Options {
    /// 1-based page ranges.
    pub pages: Option<Vec<(usize, usize)>>,
    pub ppi: f64,
    /// Also write the generated Typst source next to the output.
    pub keep_typst: bool,
}

/// What a converter produces: Typst source and the files it refers to.
pub struct Job {
    pub source: String,
    pub files: Vec<(String, Vec<u8>)>,
    pub warnings: Vec<String>,
}

pub fn to_typst(doc: &AnyDoc, fonts: &world::Fonts) -> Result<Job> {
    match doc {
        AnyDoc::Docx(d) => docx::convert(d, fonts),
        AnyDoc::Xlsx(d) => xlsx::convert(d, fonts),
        AnyDoc::Pptx(d) => pptx::convert(d, fonts),
        AnyDoc::Xls(x) => {
            let bytes = crate::convert::xls_to_xlsx(x)?;
            match AnyDoc::from_bytes(bytes, "converted.xlsx")? {
                AnyDoc::Xlsx(d) => xlsx::convert(&d, fonts),
                _ => unreachable!(),
            }
        }
        AnyDoc::Doc(x) => {
            let bytes = crate::convert::doc_to_docx(x)?;
            match AnyDoc::from_bytes(bytes, "converted.docx")? {
                AnyDoc::Docx(d) => docx::convert(&d, fonts),
                _ => unreachable!(),
            }
        }
        AnyDoc::Ppt(x) => {
            let bytes = crate::ppt_pptx::ppt_to_pptx(x)?;
            match AnyDoc::from_bytes(bytes, "converted.pptx")? {
                AnyDoc::Pptx(d) => pptx::convert(&d, fonts),
                _ => unreachable!(),
            }
        }
    }
}

pub fn parse_pages(s: &str) -> Result<Vec<(usize, usize)>> {
    let mut v = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim().parse::<usize>(), if b.trim().is_empty() { Ok(usize::MAX) } else { b.trim().parse::<usize>() }),
            None => (part.parse::<usize>(), part.parse::<usize>()),
        };
        match (a, b) {
            (Ok(a), Ok(b)) if a >= 1 && b >= a => v.push((a, b)),
            _ => return Err(err(format!("bad page range {part:?} (1,3-5,7-)"))),
        }
    }
    Ok(v)
}

/// Renders to `out` (by its extension: .pdf, .png, .svg, .typ); returns the files written.
pub fn render(doc: &AnyDoc, out: &str, opts: &Options) -> Result<(Vec<String>, Vec<String>)> {
    let ext = out.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let fonts = world::Fonts::discover();
    if fonts.book().families().next().is_none() {
        return Err(err("no fonts found (DOCU_FONT_PATH, /usr/share/fonts)"));
    }
    let job = to_typst(doc, &fonts)?;
    let mut written = Vec::new();
    let mut warnings = job.warnings.clone();
    if ext == "typ" || opts.keep_typst {
        let p = if ext == "typ" { out.to_string() } else { format!("{out}.typ") };
        std::fs::write(&p, &job.source).map_err(|e| err(format!("{p}: {e}")))?;
        written.push(p);
        if ext == "typ" {
            return Ok((written, warnings));
        }
    }
    let world = world::DocuWorld::new(&fonts, job.source, job.files);
    let result = typst::compile::<PagedDocument>(&world);
    for w in result.warnings.iter() {
        let m = w.message.to_string();
        if !m.contains("unknown font family") {
            warnings.push(format!("typst: {m}"));
        }
    }
    let document = result.output.map_err(|errs| {
        let msgs: Vec<String> = errs
            .iter()
            .map(|e| {
                let at = typst::WorldExt::range(&world, e.span).map(|r| {
                    let t = world.source_text();
                    let line = t[..r.start.min(t.len())].matches('\n').count() + 1;
                    let snippet: String = t[r.start.min(t.len())..r.end.min(t.len())].chars().take(80).collect();
                    format!(" (generated line {line}: {snippet})")
                });
                format!("{}{}", e.message, at.unwrap_or_default())
            })
            .collect();
        err(format!("layout failed: {}", msgs.join("; ")))
    })?;
    let n = document.pages().len();
    let wanted = |i: usize| match &opts.pages {
        None => true,
        Some(r) => r.iter().any(|(a, b)| i + 1 >= *a && i + 1 <= *b),
    };
    match ext.as_str() {
        "pdf" => {
            let ranges = opts.pages.as_ref().map(|r| {
                PageRanges::new(r.iter().map(|(a, b)| std::num::NonZeroUsize::new(*a)..=if *b == usize::MAX { None } else { std::num::NonZeroUsize::new(*b) }).collect())
            });
            let options = typst_pdf::PdfOptions { page_ranges: ranges, ..Default::default() };
            let pdf = typst_pdf::pdf(&document, &options).map_err(|e| err(format!("PDF export failed: {}", e.iter().map(|d| d.message.to_string()).collect::<Vec<_>>().join("; "))))?;
            std::fs::write(out, pdf).map_err(|e| err(format!("{out}: {e}")))?;
            written.push(out.to_string());
        }
        "png" | "svg" => {
            let pages: Vec<usize> = (0..n).filter(|i| wanted(*i)).collect();
            if pages.is_empty() {
                return Err(err(format!("no such pages (the document has {n})")));
            }
            for &i in &pages {
                let path = page_path(out, i + 1, pages.len() > 1 || out.contains("{p}"), n);
                let page = &document.pages()[i];
                if ext == "png" {
                    let pixmap = typst_render::render(page, &typst_render::RenderOptions { pixel_per_pt: typst::utils::Scalar::new(opts.ppi / 72.0), render_bleed: false });
                    let png = pixmap.encode_png().map_err(|e| err(format!("PNG encoding failed: {e}")))?;
                    std::fs::write(&path, png).map_err(|e| err(format!("{path}: {e}")))?;
                } else {
                    let svg = typst_svg::svg(page, &typst_svg::SvgOptions { render_bleed: false, pretty: false });
                    std::fs::write(&path, svg).map_err(|e| err(format!("{path}: {e}")))?;
                }
                written.push(path);
            }
        }
        _ => return Err(err(format!("render writes .pdf, .png, .svg or .typ, not {out:?}"))),
    }
    Ok((written, warnings))
}

/// `out.png` → `out-3.png` (or `{p}` in the name replaced); page numbers padded to the count's
/// width so they sort.
fn page_path(out: &str, page: usize, many: bool, total: usize) -> String {
    let width = total.to_string().len();
    let num = format!("{:0width$}", page);
    if out.contains("{p}") {
        return out.replace("{p}", &num);
    }
    if !many {
        return out.to_string();
    }
    match out.rfind('.') {
        Some(i) => format!("{}-{num}{}", &out[..i], &out[i..]),
        None => format!("{out}-{num}"),
    }
}

/// How many pages a document lays out to, and the generated source's size (for `info`).
pub fn page_count(doc: &AnyDoc) -> Result<usize> {
    let fonts = world::Fonts::discover();
    let job = to_typst(doc, &fonts)?;
    let world = world::DocuWorld::new(&fonts, job.source, job.files);
    let d = typst::compile::<PagedDocument>(&world).output.map_err(|_| err("layout failed"))?;
    Ok(d.pages().len())
}
