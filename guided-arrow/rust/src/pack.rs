//! Building an archive from the upstream repositories (on the build machine): tldr pages in
//! every language, cheat.sh's own sheets (chubin/cheat.sheets) and cheat/cheatsheets, or any
//! folder of sheets of one's own.
//!
//! Pages are grouped by language and sorted by name (so a command's pages from every source sit
//! together, which the compression likes), then cut into blocks of about `block_size`. A
//! translated block may take the English block(s) of the same names as its preset dictionary:
//! translations repeat most of the commands, URLs and option names.

use crate::archive::{
    crc32, BlockInfo, Directory, PageRef, Source, KIND_LANG, KIND_SEE_ALSO, KIND_SHEET, KIND_TLDR, KIND_TOPIC, MAGIC,
    TRAILER_MAGIC,
};
use crate::codec::{self, Effort, Params};
use crate::{err, Result};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct SourceSpec {
    pub id: String,
    pub dir: PathBuf,
    pub revision: String,
    pub date: String,
}

pub struct Options {
    pub out: String,
    pub sources: Vec<SourceSpec>,
    /// tldr languages to take (None: all of them).
    pub langs: Option<Vec<String>>,
    pub block_size: usize,
    pub effort: Effort,
    /// Give translated blocks the English text as a dictionary.
    pub dict: bool,
    pub created: String,
    pub quiet: bool,
}

struct RawPage {
    lang: String,
    name: String,
    source: u16,
    kind: u8,
    platform: String,
    text: Vec<u8>,
}

/// What we know of the upstreams by name.
fn known_source(spec: &SourceSpec) -> Source {
    let gh = |repo: &str| (format!("https://github.com/{repo}"), format!("https://github.com/{repo}/blob/{{rev}}/{{path}}"));
    let (title, (url, file_url), license) = match spec.id.as_str() {
        "tldr" => ("tldr pages", gh("tldr-pages/tldr"), "CC-BY-4.0"),
        "cheat.sheets" => ("cheat.sh's own cheat sheets (chubin/cheat.sheets)", gh("chubin/cheat.sheets"), "MIT"),
        "cheatsheets" => ("cheat/cheatsheets (cheat.sh's \"cheat\" source)", gh("cheat/cheatsheets"), "CC0-1.0"),
        _ => ("sheets packed from a folder", (String::new(), String::new()), ""),
    };
    Source {
        id: spec.id.clone(),
        title: title.to_string(),
        url,
        revision: spec.revision.clone(),
        date: spec.date.clone(),
        license: license.to_string(),
        file_url,
    }
}

/// A folder's entries by name; hidden ones (.git, .github) only when `hidden` (tldr has a page
/// for the `.` command: `..md`).
fn read_dir_sorted(dir: &Path, hidden: bool) -> Result<Vec<(String, PathBuf, std::fs::FileType)>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).map_err(|e| err!("{}: {e}", dir.display()))? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && !hidden {
            continue;
        }
        out.push((name, e.path(), e.file_type()?));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

pub fn read_text(path: &Path, quiet: bool) -> Result<Vec<u8>> {
    let b = std::fs::read(path).map_err(|e| err!("{}: {e}", path.display()))?;
    if std::str::from_utf8(&b).is_err() {
        if !quiet {
            eprintln!("guided-arrow: {}: not UTF-8, invalid bytes replaced", path.display());
        }
        return Ok(String::from_utf8_lossy(&b).into_owned().into_bytes());
    }
    Ok(b)
}

fn collect(spec: &SourceSpec, source: u16, langs: &Option<Vec<String>>, quiet: bool, out: &mut Vec<RawPage>) -> Result<()> {
    let page = |lang: &str, name: String, kind: u8, platform: &str, text: Vec<u8>| RawPage {
        lang: lang.to_string(),
        name,
        source,
        kind,
        platform: platform.to_string(),
        text,
    };
    match spec.id.as_str() {
        "tldr" => {
            for (dname, dpath, ft) in read_dir_sorted(&spec.dir, false)? {
                // pages.en is a link to pages.
                let lang = match dname.as_str() {
                    "pages" => "en".to_string(),
                    d => match d.strip_prefix("pages.") {
                        Some(l) if l != "en" => l.to_string(),
                        _ => continue,
                    },
                };
                if !ft.is_dir() || langs.as_ref().is_some_and(|ls| !ls.contains(&lang) && lang != "en") {
                    continue;
                }
                for (platform, ppath, ft) in read_dir_sorted(&dpath, false)? {
                    if !ft.is_dir() {
                        continue;
                    }
                    for (fname, fpath, ft) in read_dir_sorted(&ppath, true)? {
                        if let (true, Some(stem)) = (ft.is_file(), fname.strip_suffix(".md")) {
                            out.push(page(&lang, stem.to_string(), KIND_TLDR, &platform, read_text(&fpath, quiet)?));
                        }
                    }
                }
            }
        }
        "cheat.sheets" => {
            for (name, path, ft) in read_dir_sorted(&spec.dir.join("sheets"), false)? {
                if ft.is_file() {
                    out.push(page("en", name, KIND_SHEET, "", read_text(&path, quiet)?));
                } else if let (true, Some(lang)) = (ft.is_dir(), name.strip_prefix('_')) {
                    for (topic, tpath, ft) in read_dir_sorted(&path, false)? {
                        if ft.is_file() && !topic.starts_with('_') {
                            out.push(page("en", format!("{lang}/{topic}"), KIND_LANG, "", read_text(&tpath, quiet)?));
                        }
                    }
                }
            }
            if let Ok(list) = read_dir_sorted(&spec.dir.join("topics"), false) {
                for (name, path, ft) in list {
                    if let (true, Some(stem)) = (ft.is_file(), name.strip_suffix(".json")) {
                        out.push(page("en", stem.to_string(), KIND_TOPIC, "", read_text(&path, quiet)?));
                    }
                }
            }
            if let Ok(list) = read_dir_sorted(&spec.dir.join("see_also"), false) {
                for (name, path, ft) in list {
                    if ft.is_file() {
                        out.push(page("en", name, KIND_SEE_ALSO, "", read_text(&path, quiet)?));
                    }
                }
            }
        }
        _ => {
            // cheat/cheatsheets, or a folder of one's own: every file is a sheet, named by its path.
            for (name, path) in sheet_files(&spec.dir)? {
                out.push(page("en", name, KIND_SHEET, "", read_text(&path, quiet)?));
            }
        }
    }
    Ok(())
}

/// A folder of sheets: every file in it (not hidden ones), named by its path in the folder.
/// Links to files count; links to folders do not (no loops).
pub fn sheet_files(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
        for (name, path, ft) in read_dir_sorted(dir, false)? {
            let full = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if ft.is_dir() {
                walk(&path, &full, out)?;
            } else if ft.is_file() || (ft.is_symlink() && path.is_file()) {
                out.push((full, path));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out)?;
    Ok(out)
}

/// The order of platforms within a name: the ones a Linux guest wants first.
fn platform_rank(p: &str) -> (u8, &str) {
    let r = match p {
        "common" => 0,
        "linux" => 1,
        "" => 2,
        "osx" => 3,
        "windows" => 4,
        _ => 5,
    };
    (r, p)
}

pub fn pack(o: &Options) -> Result<Directory> {
    let mut pages = Vec::new();
    let mut sources = Vec::new();
    for (i, spec) in o.sources.iter().enumerate() {
        if !spec.dir.is_dir() {
            return Err(err!("{}: no such folder", spec.dir.display()));
        }
        if spec.id == crate::local::SOURCE {
            return Err(err!("--source {}: that name is for the sheets added with --add; pick another", spec.id));
        }
        sources.push(known_source(spec));
        collect(spec, i as u16, &o.langs, o.quiet, &mut pages)?;
    }
    if pages.is_empty() {
        return Err(err!("no pages found"));
    }
    // English first, then the others by code.
    let mut by_lang: BTreeMap<(bool, String), Vec<RawPage>> = BTreeMap::new();
    for p in pages {
        by_lang.entry((p.lang != "en", p.lang.clone())).or_default().push(p);
    }
    let mut langs = Vec::new();
    let mut platforms: Vec<String> = vec![String::new()];
    let mut groups = Vec::new();
    for ((_, lang), mut list) in by_lang {
        list.sort_by(|a, b| {
            (a.name.to_lowercase(), a.source, platform_rank(&a.platform), a.kind, &a.name)
                .cmp(&(b.name.to_lowercase(), b.source, platform_rank(&b.platform), b.kind, &b.name))
        });
        langs.push(lang);
        groups.push(list);
    }

    // Cut English into blocks of about block_size. A translation is cut where English is (each
    // part takes the English block of the same names as its dictionary), and by size.
    struct Plan {
        lang: u16,
        pages: Vec<RawPage>,
        dict: Option<u32>,
    }
    let mut plans: Vec<Plan> = Vec::new();
    let mut english_last: Vec<String> = Vec::new(); // the last name of each English block
    for (li, list) in groups.into_iter().enumerate() {
        let english = langs[li] == "en";
        let mut cur: Vec<RawPage> = Vec::new();
        let mut cur_dict = None;
        let mut size = 0;
        for p in list {
            let dict = if english || !o.dict || english_last.is_empty() {
                None
            } else {
                let n = p.name.to_lowercase();
                Some(english_last.iter().position(|l| n <= *l).unwrap_or(english_last.len() - 1) as u32)
            };
            if !cur.is_empty() && (size + p.text.len() > o.block_size || dict != cur_dict) {
                if english {
                    english_last.push(cur.last().unwrap().name.to_lowercase());
                }
                plans.push(Plan { lang: li as u16, pages: std::mem::take(&mut cur), dict: cur_dict });
                size = 0;
            }
            cur_dict = dict;
            size += p.text.len() + 1;
            cur.push(p);
        }
        if !cur.is_empty() {
            if english {
                english_last.push(cur.last().unwrap().name.to_lowercase());
            }
            plans.push(Plan { lang: li as u16, pages: cur, dict: cur_dict });
        }
    }
    let dicts: Vec<Option<u32>> = plans.iter().map(|pl| pl.dict).collect();

    // A block's text: its pages joined by NUL.
    let mut raws: Vec<Vec<u8>> = Vec::new();
    for pl in &plans {
        let mut raw = Vec::new();
        for (k, p) in pl.pages.iter().enumerate() {
            if p.text.contains(&0) {
                return Err(err!("{} ({}): has a NUL byte", p.name, langs[p_lang(pl)]));
            }
            if k > 0 {
                raw.push(0);
            }
            raw.extend_from_slice(&p.text);
        }
        raws.push(raw);
    }
    fn p_lang(pl: &Plan) -> usize {
        pl.lang as usize
    }
    let params = Params::TEXT;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(vec![Vec::new(); raws.len()]);
    let work = || loop {
        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if i >= raws.len() {
            break;
        }
        let dict: &[u8] = dicts[i].map_or(&[], |d| &raws[d as usize]);
        let c = codec::compress(dict, &raws[i], params, o.effort);
        if !o.quiet {
            eprintln!(
                "guided-arrow: block {i} ({}, {} pages{}): {} -> {} bytes",
                langs[plans[i].lang as usize],
                plans[i].pages.len(),
                dicts[i].map_or(String::new(), |d| format!(", dictionary: block {d}")),
                raws[i].len(),
                c.len()
            );
        }
        results.lock().unwrap()[i] = c;
    };
    std::thread::scope(|sc| {
        // Where threads cannot be made, this one does all the blocks.
        for _ in 1..threads.min(raws.len()) {
            if std::thread::Builder::new().spawn_scoped(sc, work).is_err() {
                break;
            }
        }
        work();
    });
    let comps = results.into_inner().unwrap();

    let mut file = Vec::new();
    file.extend_from_slice(MAGIC);
    let mut blocks = Vec::new();
    let mut refs = Vec::new();
    for (i, pl) in plans.iter().enumerate() {
        blocks.push(BlockInfo {
            offset: file.len() as u64,
            comp_len: comps[i].len() as u32,
            raw_len: raws[i].len() as u32,
            crc: crc32(&raws[i]),
            dict: dicts[i],
            lang: pl.lang,
            pages: pl.pages.len() as u32,
        });
        file.extend_from_slice(&comps[i]);
        for (index, p) in pl.pages.iter().enumerate() {
            let platform = match platforms.iter().position(|x| *x == p.platform) {
                Some(n) => n,
                None => {
                    platforms.push(p.platform.clone());
                    platforms.len() - 1
                }
            };
            refs.push(PageRef {
                name: p.name.clone(),
                source: p.source,
                kind: p.kind,
                platform: platform as u16,
                lang: pl.lang,
                block: i as u32,
                index: index as u32,
            });
        }
    }
    let raw_total: usize = raws.iter().map(Vec::len).sum();
    let dir = Directory {
        params,
        meta: vec![
            ("created".to_string(), o.created.clone()),
            ("packer".to_string(), format!("guided-arrow {}", env!("CARGO_PKG_VERSION"))),
            ("raw_bytes".to_string(), raw_total.to_string()),
        ],
        sources,
        langs,
        platforms,
        blocks,
        pages: refs,
    };
    let raw_dir = dir.encode();
    let comp_dir = codec::compress(b"", &raw_dir, Params::TEXT, o.effort);
    let dir_offset = file.len() as u64;
    file.extend_from_slice(&comp_dir);
    file.extend_from_slice(&dir_offset.to_le_bytes());
    file.extend_from_slice(&(comp_dir.len() as u32).to_le_bytes());
    file.extend_from_slice(&(raw_dir.len() as u32).to_le_bytes());
    file.extend_from_slice(&crc32(&raw_dir).to_le_bytes());
    file.extend_from_slice(TRAILER_MAGIC);
    let tmp = format!("{}.tmp", o.out);
    std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(&file))
        .map_err(|e| err!("{tmp}: {e}"))?;
    std::fs::rename(&tmp, &o.out).map_err(|e| err!("{}: {e}", o.out))?;
    if !o.quiet {
        eprintln!(
            "guided-arrow: {}: {} pages, {} blocks, {} -> {} bytes (directory {} -> {})",
            o.out,
            dir.pages.len(),
            dir.blocks.len(),
            raw_total,
            file.len(),
            raw_dir.len(),
            comp_dir.len()
        );
    }
    Ok(dir)
}

/// The commit a git checkout is at (HEAD, through a branch ref or packed-refs).
pub fn git_head(dir: &Path) -> Option<String> {
    let git = dir.join(".git");
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(r) = head.strip_prefix("ref: ") else {
        return Some(head.to_string());
    };
    if let Ok(v) = std::fs::read_to_string(git.join(r)) {
        return Some(v.trim().to_string());
    }
    let packed = std::fs::read_to_string(git.join("packed-refs")).ok()?;
    packed.lines().find_map(|l| l.strip_suffix(r).map(|h| h.trim().to_string()))
}
