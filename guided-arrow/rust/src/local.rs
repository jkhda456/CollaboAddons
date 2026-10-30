//! Sheets of one's own: plain files in a folder, in the cheat format (`# description` lines over
//! the commands under them), read at every run and shown with the archive's pages as the
//! source `local`. `--add` writes them, `--remove` takes them (or some of their examples) out.
//!
//! The folders, the first that is set:
//!
//! ```text
//! $GUIDED_ARROW_SHEETS        ':'-separated; --add writes to the first
//! the add-on's "sheets"       /etc/collabo/addons/guided-arrow.json: an app points it at a folder
//!                             it mounts, so the sheets outlive the sandbox
//! ${XDG_DATA_HOME:-~/.local/share}/guided-arrow/sheets
//! ```
//!
//! and, read only, `sheets/` beside the archive (`/usr/share/guided-arrow/sheets`: what the
//! image or another add-on puts there).

use crate::archive::{Archive, Source, KIND_SHEET};
use crate::{err, json, pack, Result};
use std::path::{Path, PathBuf};

/// The source the sheets are shown as.
pub const SOURCE: &str = "local";
const SETTINGS: &str = "/etc/collabo/addons/guided-arrow.json";
/// A bigger file in a sheet folder is not a sheet (it is skipped, said so).
const MAX_SHEET: u64 = 1 << 20;

pub struct Folders {
    /// Where --add writes (it need not exist yet).
    pub write: PathBuf,
    /// Every folder read, `write` first.
    pub read: Vec<PathBuf>,
    /// `write` is the default folder in a sandbox, which forgets it when it stops.
    pub ephemeral: bool,
}

pub fn folders(archive: Option<&str>) -> Folders {
    let mut read: Vec<PathBuf> = Vec::new();
    if let Some(list) = std::env::var("GUIDED_ARROW_SHEETS").ok().filter(|v| !v.is_empty()) {
        read.extend(list.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    }
    if read.is_empty() {
        read.extend(setting());
    }
    let mut ephemeral = false;
    if read.is_empty() {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let data = var("XDG_DATA_HOME")
            .or_else(|| var("HOME").map(|h| h.join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        read.push(data.join("guided-arrow/sheets"));
        ephemeral = Path::new(SETTINGS).is_file();
    }
    let write = read[0].clone();
    if let Some(dir) = archive.and_then(|a| Path::new(a).parent()) {
        let beside = dir.join("sheets");
        if !read.contains(&beside) {
            read.push(beside);
        }
    }
    Folders { write, read, ephemeral }
}

/// The add-on's "sheets" setting, from the app.
fn setting() -> Option<PathBuf> {
    let text = std::fs::read_to_string(SETTINGS).ok()?;
    let v = json::parse(&text).ok()?;
    v.get("sheets")?.as_str().filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// Adds every folder's sheets to the archive, one source each: the folder --add writes to even
/// when it has none yet (so `--from local` and `--info` know it), the others when they exist.
pub fn load(ar: &mut Archive, f: &Folders) -> Result<()> {
    let mut seen: Vec<PathBuf> = Vec::new();
    for (i, dir) in f.read.iter().enumerate() {
        let real = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
        if seen.contains(&real) || (i > 0 && !dir.is_dir()) {
            continue;
        }
        seen.push(real);
        let mut pages = Vec::new();
        if dir.is_dir() {
            let files = match pack::sheet_files(dir) {
                Ok(files) => files,
                Err(e) => {
                    eprintln!("guided-arrow: {e} (its sheets are left out)");
                    Vec::new()
                }
            };
            for (name, path) in files {
                if name.ends_with(".tmp") {
                    continue;
                }
                if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_SHEET) {
                    eprintln!("guided-arrow: {}: over {} KiB, not a sheet (left out)", path.display(), MAX_SHEET >> 10);
                    continue;
                }
                match pack::read_text(&path, false) {
                    Ok(t) if t.contains(&0) => eprintln!("guided-arrow: {}: has a NUL byte, not a sheet (left out)", path.display()),
                    Ok(t) => pages.push((name, KIND_SHEET, t)),
                    Err(e) => eprintln!("guided-arrow: {e} (left out)"),
                }
            }
        }
        let source = Source { id: SOURCE.into(), title: "sheets of one's own".into(), url: dir.display().to_string(), ..Default::default() };
        ar.add_pages(source, pages)?;
    }
    Ok(())
}

/// A sheet's name as its file is called: lower case, spaces as dashes (as names are looked up),
/// `/` between the parts of a name such as `python/walrus`.
pub fn sheet_name(raw: &str) -> std::result::Result<String, String> {
    let k = crate::cli::key(raw);
    let bad = k.is_empty()
        || k.len() > 200
        || k.contains('~')
        || k.split('/').any(|p| p.is_empty() || p.starts_with('.') || p.ends_with(".tmp") || p.contains('\\') || p.chars().any(char::is_control));
    if bad {
        return Err(format!(
            "{raw:?} is not a sheet name (a command's name, such as kubectl-secret or python/walrus: no part may be empty or start with a dot)"
        ));
    }
    Ok(k)
}

/// Writes a sheet whole (through a hidden temporary file, then a rename).
pub fn write_sheet(path: &Path, text: &str) -> Result<()> {
    let dir = path.parent().ok_or_else(|| err!("{}: no folder", path.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| err!("{}: {e}", dir.display()))?;
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{file}.tmp"));
    std::fs::write(&tmp, text).map_err(|e| err!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| err!("{}: {e}", path.display()))?;
    Ok(())
}

/// Removes a sheet's file, and the folders of its name that are left empty (not `root`).
pub fn delete_sheet(root: &Path, path: &Path) -> Result<()> {
    std::fs::remove_file(path).map_err(|e| err!("{}: {e}", path.display()))?;
    let mut dir = path.parent();
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) || std::fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(sheet_name("kubectl-secret"), Ok("kubectl-secret".into()));
        assert_eq!(sheet_name("Git Commit"), Ok("git-commit".into()));
        assert_eq!(sheet_name("python/walrus"), Ok("python/walrus".into()));
        for bad in ["", "../etc/passwd", "/abs", "a//b", "a/", ".hidden", "a/.b", "tar~list", "x.tmp", "a\\b"] {
            assert!(sheet_name(bad).is_err(), "{bad:?}");
        }
    }
}
