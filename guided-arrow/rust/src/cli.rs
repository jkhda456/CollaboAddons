//! The command line: looking a command up, searching by task, listing, provenance, checking the
//! archive, and (on the build machine) packing one.

use crate::archive::{self, Archive, PageRef, KIND_LANG, KIND_SEE_ALSO, KIND_SHEET, KIND_TLDR, KIND_TOPIC};
use crate::codec::Effort;
use crate::json::{obj, s, Json};
use crate::page::{self, OptionForm, Sheet, Style};
use crate::{err, local, pack, search, Result};
use std::collections::HashMap;
use std::io::{IsTerminal, Write};

const USAGE: &str = "\
guided-arrow: verified command-line cheat sheets — tldr pages and cheat.sh's sheets — offline,
from one compressed archive. Examples come from reviewed upstream repositories (see --info), not
from a web search; sheets of one's own (--add) are marked [local].

Usage:
  guided-arrow NAME [WORD...]        a command's sheets: one's own first, then tldr, then cheat.sh's
                                     (cheat.sheets, cheat/cheatsheets). With WORDs (or NAME~WORD),
                                     only the examples that mention them. \"git commit\" finds
                                     git-commit.
  guided-arrow LANG/TOPIC            a programming language sheet (python/lambda, go/func);
                                     LANG/ lists a language's topics; LANG/some+words searches them
  guided-arrow -s WORD...            the best examples for a task, from every sheet
                                     (guided-arrow -s extract tar gz)
  guided-arrow -l [PREFIX]           page names, one per line
  guided-arrow --info [NAME]         sources, revisions, licenses, sizes; with NAME, where each of
                                     its pages came from (upstream URL at the revision, git blob id)
  guided-arrow --verify              decompress every block and check its checksum

Sheets of one's own (plain files, shown with the archive's as the source \"local\", before them):
  guided-arrow --add NAME DESCRIPTION COMMAND
                                     add an example to the sheet NAME (made if new); a COMMAND
                                     that starts with - goes after --
  guided-arrow --add NAME < FILE     add a sheet's text: # description lines over the commands
  guided-arrow --remove NAME [WORD...]
                                     remove that sheet, or only its examples that mention the words
  The folder: $GUIDED_ARROW_SHEETS (':'-separated; --add writes to the first), else the add-on's
  \"sheets\" setting (a folder the app keeps), else ~/.local/share/guided-arrow/sheets; also read:
  sheets/ beside the archive. --no-local leaves them out.

Options:
  -p, --platform NAME   tldr platform first looked in: linux (default here), osx, windows,
                        freebsd, ... then common, then any (said so in the header)
  -L, --language CODE   page language (default: $GUIDED_ARROW_LANG, $LANGUAGE/$LANG, else en);
                        English where a page has no translation or the archive no such
                        language (the default archive is English only)
      --from SOURCE     only tldr, cheat.sheets, cheatsheets or local (repeatable)
  -n, --limit N         how many examples -s shows (default 10)
  -S, --short-options   tldr option placeholders in short form (-C); -E/--long-options long
                        (the default: --directory); both: [-C|--directory]
      --json            the same, as JSON (pages, examples, upstream provenance)
      --raw             pages exactly as upstream wrote them
      --braces, --no-braces
                        keep {{...}} around values to fill in (default: kept, unless color is on)
      --color WHEN      auto (default: on a terminal, unless NO_COLOR), always, never
      --data FILE       the archive (default: $GUIDED_ARROW_DATA, else
                        ../share/guided-arrow/guided-arrow.gar from the program,
                        /usr/share/guided-arrow/guided-arrow.gar)
  -h, --help, -v, --version

Output: each page starts with \"=== NAME [SOURCE: PLATFORM] ===\", descriptions are # comments and
commands are plain lines. Exit status: 0 found, 1 not found (closest matches printed),
2 usage, 3 archive problem.

Building an archive (build machine):
  guided-arrow --pack -o FILE --source tldr=DIR --source cheat.sheets=DIR --source cheatsheets=DIR
               [--source NAME=DIR] (each: ,rev=COMMIT,date=YYYY-MM-DD) [--langs en (default)|en,ko,...|all]
               [--block-size BYTES (default 2 MiB)] [--no-dict] [--fast] [--created DATE] [--quiet]
";

struct Opts {
    platform: Option<String>,
    lang: Option<String>,
    from: Vec<String>,
    limit: usize,
    short: bool,
    long: bool,
    json: bool,
    raw: bool,
    color: Option<bool>,
    braces: Option<bool>,
    data: Option<String>,
    no_local: bool,
}

enum Mode {
    Lookup,
    Search,
    List,
    Info,
    Verify,
    Add,
    Remove,
    Pack,
    Help,
    Version,
}

pub fn main(args: Vec<String>) -> i32 {
    match run(args) {
        Ok(code) => code,
        Err(Fail::Usage(m)) => {
            eprintln!("guided-arrow: {m}\n(guided-arrow --help for usage)");
            2
        }
        Err(Fail::Archive(e)) => {
            eprintln!("guided-arrow: {e}");
            3
        }
    }
}

enum Fail {
    Usage(String),
    Archive(crate::Error),
}

impl From<crate::Error> for Fail {
    fn from(e: crate::Error) -> Fail {
        Fail::Archive(e)
    }
}

fn run(args: Vec<String>) -> std::result::Result<i32, Fail> {
    let mut o = Opts {
        platform: None,
        lang: None,
        from: Vec::new(),
        limit: 10,
        short: false,
        long: false,
        json: false,
        raw: false,
        color: None,
        braces: None,
        data: None,
        no_local: false,
    };
    let mut mode = Mode::Lookup;
    let mut words = Vec::new();
    let mut pack_args = Vec::new();
    let mut it = args.into_iter();
    let usage = |m: String| Fail::Usage(m);
    while let Some(a) = it.next() {
        if matches!(mode, Mode::Pack) {
            pack_args.push(a);
            continue;
        }
        if a == "--" {
            words.extend(it.by_ref());
            break;
        }
        // --opt=value
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let mut value = |name: &str| -> std::result::Result<String, Fail> {
            inline.clone().or_else(|| it.next()).ok_or_else(|| usage(format!("{name} needs a value")))
        };
        match flag.as_str() {
            "-h" | "--help" => mode = Mode::Help,
            "-v" | "--version" => mode = Mode::Version,
            "-s" | "--search" => mode = Mode::Search,
            "-l" | "--list" => mode = Mode::List,
            "--info" => mode = Mode::Info,
            "--verify" => mode = Mode::Verify,
            "--add" => mode = Mode::Add,
            "--remove" => mode = Mode::Remove,
            "--no-local" => o.no_local = true,
            "--pack" => mode = Mode::Pack,
            "-p" | "--platform" => o.platform = Some(value(&flag)?.to_lowercase()),
            "-L" | "--language" => o.lang = Some(value(&flag)?),
            "--from" => o.from.push(value(&flag)?),
            "-n" | "--limit" => {
                o.limit = value(&flag)?.parse().map_err(|_| usage(format!("{flag}: not a number")))?;
            }
            "-S" | "--short-options" => o.short = true,
            "-E" | "--long-options" => o.long = true,
            "--json" => o.json = true,
            "--raw" => o.raw = true,
            "--braces" => o.braces = Some(true),
            "--no-braces" => o.braces = Some(false),
            "--color" | "--colour" => {
                o.color = match value(&flag)?.as_str() {
                    "auto" => None,
                    "always" | "yes" => Some(true),
                    "never" | "no" => Some(false),
                    v => return Err(usage(format!("--color {v}: auto, always or never"))),
                }
            }
            "--data" => o.data = Some(value(&flag)?),
            f if f.starts_with("-n") && f[2..].parse::<usize>().is_ok() => o.limit = f[2..].parse().unwrap(),
            f if f.starts_with('-') && f.len() > 1 && !f.starts_with("--") && f[1..].chars().all(|c| "SEhv".contains(c)) => {
                for c in f[1..].chars() {
                    match c {
                        'S' => o.short = true,
                        'E' => o.long = true,
                        'h' => mode = Mode::Help,
                        _ => mode = Mode::Version,
                    }
                }
            }
            f if f.starts_with('-') && f.len() > 1 && f != "-" => return Err(usage(format!("unknown option {f}"))),
            _ => words.push(a),
        }
    }
    match mode {
        Mode::Help => {
            print!("{USAGE}");
            return Ok(0);
        }
        Mode::Version => {
            println!("guided-arrow {}", env!("CARGO_PKG_VERSION"));
            if let Ok(ar) = open(&o) {
                let d = &ar.dir;
                let meta = |k: &str| d.meta.iter().find(|(a, _)| a == k).map_or("", |(_, v)| v.as_str()).to_string();
                println!("archive: {} ({} pages, created {})", ar.path, d.pages.len(), meta("created"));
                for src in &d.sources {
                    println!("  {} {} {}", src.id, short_rev(&src.revision), src.date);
                }
            }
            return Ok(0);
        }
        Mode::Pack => return pack_cmd(pack_args).map_err(usage),
        _ => {}
    }
    let mut out = String::new();
    if matches!(mode, Mode::Add | Mode::Remove) {
        // No archive needed: these change a folder of sheets.
        let f = local::folders(None);
        let code = if matches!(mode, Mode::Add) { add_cmd(&o, &f, &words, &mut out)? } else { remove_cmd(&o, &f, &words, &mut out)? };
        print!("{out}");
        return Ok(code);
    }
    let mut ar = open(&o)?;
    if !o.no_local {
        let f = local::folders(Some(&ar.path));
        local::load(&mut ar, &f)?;
    }
    let ar = ar;
    let cx = Context::new(&ar, &o).map_err(usage)?;
    let code = match mode {
        Mode::Verify => verify(&ar, &mut out)?,
        Mode::Info => info(&cx, &words, &mut out)?,
        Mode::List => list(&cx, &words, &mut out),
        Mode::Search => {
            if words.is_empty() {
                return Err(usage("-s needs words to search for".into()));
            }
            search_cmd(&cx, &words, &mut out)?
        }
        _ => {
            if words.is_empty() {
                print!("{USAGE}");
                return Ok(2);
            }
            lookup(&cx, &words, &mut out)?
        }
    };
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(out.as_bytes());
    let _ = stdout.flush();
    Ok(code)
}

fn short_rev(r: &str) -> &str {
    &r[..r.len().min(12)]
}

fn open(o: &Opts) -> Result<Archive> {
    let mut tried = Vec::new();
    let mut candidates = Vec::new();
    if let Some(d) = &o.data {
        return Archive::open(d);
    }
    if let Ok(d) = std::env::var("GUIDED_ARROW_DATA") {
        if !d.is_empty() {
            return Archive::open(&d);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("../share/guided-arrow/guided-arrow.gar"));
            candidates.push(dir.join("guided-arrow.gar"));
        }
    }
    candidates.push("/usr/share/guided-arrow/guided-arrow.gar".into());
    for c in candidates {
        if c.is_file() {
            let c = std::fs::canonicalize(&c).unwrap_or(c);
            return Archive::open(&c.to_string_lossy());
        }
        tried.push(c.display().to_string());
    }
    Err(err!("no archive found (tried {}); give one with --data or GUIDED_ARROW_DATA", tried.join(", ")))
}

/// What a request needs from the archive and the options.
struct Context<'a> {
    ar: &'a Archive,
    o: &'a Opts,
    /// Language indices, most wanted first, English last.
    langs: Vec<u16>,
    /// A language other than English was asked for.
    asked_lang: Option<String>,
    platforms: Vec<String>,
    by_key: HashMap<String, Vec<usize>>,
    style: Style,
    color: bool,
}

/// How names are compared: lower case, spaces as dashes ("git commit" is git-commit).
pub fn key(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join("-").to_lowercase()
}

fn host_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "osx"
    } else if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "freebsd") {
        "freebsd"
    } else {
        "linux"
    }
}

impl<'a> Context<'a> {
    fn new(ar: &'a Archive, o: &'a Opts) -> std::result::Result<Context<'a>, String> {
        let have = &ar.dir.langs;
        let en = have.iter().position(|l| l == "en");
        // tldr's client rules: --language, else $LANGUAGE (a list) then $LANG; C and POSIX mean none.
        let mut wanted: Vec<String> = Vec::new();
        if let Some(l) = &o.lang {
            wanted.push(l.clone());
        } else if let Some(l) = std::env::var("GUIDED_ARROW_LANG").ok().filter(|l| !l.is_empty()) {
            wanted.push(l);
        } else if let Ok(lang) = std::env::var("LANG") {
            if !lang.is_empty() {
                if let Ok(list) = std::env::var("LANGUAGE") {
                    wanted.extend(list.split(':').map(String::from));
                }
                wanted.push(lang);
            }
        }
        let mut langs: Vec<u16> = Vec::new();
        let mut asked_lang = None;
        for w in &wanted {
            let code = w.split(['.', '@']).next().unwrap_or("");
            if code.is_empty() || code == "C" || code == "POSIX" {
                continue;
            }
            let found = have
                .iter()
                .position(|h| h == code)
                .or_else(|| have.iter().position(|h| Some(h.as_str()) == code.split('_').next()));
            match found {
                Some(i) => {
                    if !langs.contains(&(i as u16)) {
                        langs.push(i as u16);
                    }
                    if have[i] != "en" && asked_lang.is_none() {
                        asked_lang = Some(have[i].clone());
                    }
                }
                // As tldr's clients do for a page without a translation: English, said so.
                None if o.lang.is_some() => {
                    eprintln!("guided-arrow: no pages in language {code:?} here (the archive has: {}); English is shown", have.join(", "));
                }
                None => {}
            }
        }
        if let Some(e) = en {
            if !langs.contains(&(e as u16)) {
                langs.push(e as u16);
            }
        }
        let mut platforms = vec![o.platform.clone().unwrap_or_else(|| host_platform().to_string())];
        for p in ["common", ""] {
            if !platforms.iter().any(|x| x == p) {
                platforms.push(p.to_string());
            }
        }
        if let Some(p) = &o.platform {
            if !ar.dir.platforms.contains(p) {
                let list: Vec<&str> = ar.dir.platforms.iter().filter(|p| !p.is_empty()).map(String::as_str).collect();
                return Err(format!("no platform {p:?} (there are: {})", list.join(", ")));
            }
        }
        for f in &o.from {
            if !ar.dir.sources.iter().any(|s| &s.id == f) {
                let list: Vec<&str> = ar.dir.sources.iter().map(|s| s.id.as_str()).collect();
                return Err(format!("--from {f}: no such source (there are: {})", list.join(", ")));
            }
        }
        let mut by_key: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, p) in ar.dir.pages.iter().enumerate() {
            by_key.entry(key(&p.name)).or_default().push(i);
        }
        let color = o.color.unwrap_or_else(|| {
            std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()) && !o.json
        });
        let options = match (o.short, o.long) {
            (true, true) => OptionForm::Both,
            (true, false) => OptionForm::Short,
            _ => OptionForm::Long,
        };
        let style = Style { braces: o.braces.unwrap_or(!color), color: color && !o.json, options };
        Ok(Context { ar, o, langs, asked_lang, platforms, by_key, style, color: color && !o.json })
    }

    fn source_ok(&self, p: &PageRef) -> bool {
        self.o.from.is_empty() || self.o.from.iter().any(|f| *f == self.ar.source(p).id)
    }

    fn lang_ok(&self, p: &PageRef) -> bool {
        self.langs.contains(&p.lang)
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Showing pages

fn kind_name(k: u8) -> &'static str {
    match k {
        KIND_TLDR => "tldr",
        KIND_SHEET => "sheet",
        KIND_LANG => "language-sheet",
        KIND_TOPIC => "topic",
        _ => "see-also",
    }
}

fn label(cx: &Context, p: &PageRef) -> String {
    let src = &cx.ar.source(p).id;
    let mut l = match p.kind {
        KIND_TLDR => format!("tldr: {}", cx.ar.platform(p)),
        KIND_TOPIC => format!("{src}: tools for a task"),
        _ => src.clone(),
    };
    let lang = cx.ar.lang(p);
    if lang != "en" {
        l.push_str(", ");
        l.push_str(lang);
    }
    l
}

fn header(cx: &Context, p: &PageRef, note: &str, out: &mut String) {
    let mut h = format!("=== {} [{}] ===", p.name, label(cx, p));
    if !note.is_empty() {
        h.push(' ');
        h.push_str(note);
    }
    out.push_str(&cx.paint("1;36", &h));
    out.push('\n');
}

fn code_line(cx: &Context, p: &PageRef, code: &str) -> String {
    if p.kind == KIND_TLDR {
        page::render_tldr_code(code, cx.style)
    } else {
        code.to_string()
    }
}

fn comment(cx: &Context, marker: &str, text: &str, out: &mut String) {
    for l in text.lines() {
        let line = if l.is_empty() { marker.to_string() } else { format!("{marker} {l}") };
        out.push_str(&cx.paint("32", &line));
        out.push('\n');
    }
}

/// A page as text: the whole of it, or only some examples (`only`).
fn render(cx: &Context, i: usize, sheet: &Sheet, only: Option<&[usize]>, note: &str, out: &mut String) -> Result<()> {
    let p = &cx.ar.dir.pages[i];
    header(cx, p, note, out);
    let marker = if sheet.marker.is_empty() { "#" } else { &sheet.marker };
    if cx.o.raw {
        out.push_str(&cx.ar.text(p)?);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        return Ok(());
    }
    // A whole cheat sheet is shown as upstream wrote it (without its front matter).
    if only.is_none() && matches!(p.kind, KIND_SHEET | KIND_LANG) {
        let text = cx.ar.text(p)?;
        let (_, body) = page::front_matter(&text);
        for l in body.trim_matches('\n').lines() {
            if l.trim_start().starts_with(marker) {
                out.push_str(&cx.paint("32", l));
            } else {
                out.push_str(l);
            }
            out.push('\n');
        }
        out.push('\n');
        return Ok(());
    }
    if only.is_none() {
        let mut head = sheet.summary.join("\n");
        if let Some(m) = &sheet.more {
            if !head.is_empty() {
                head.push('\n');
            }
            head.push_str("More information: ");
            head.push_str(m);
        }
        if !head.is_empty() {
            comment(cx, marker, &head, out);
            out.push('\n');
        }
    }
    let all: Vec<usize> = (0..sheet.examples.len()).collect();
    for &j in only.unwrap_or(&all) {
        let e = &sheet.examples[j];
        if !e.desc.is_empty() {
            comment(cx, marker, &e.desc, out);
        }
        for l in e.code.lines() {
            out.push_str(&code_line(cx, p, l));
            out.push('\n');
        }
        out.push('\n');
    }
    if only.is_none() && !sheet.see_also.is_empty() {
        comment(cx, marker, &format!("See also: {}", sheet.see_also.join(", ")), out);
        out.push('\n');
    }
    Ok(())
}

fn upstream_json(cx: &Context, p: &PageRef, with_blob: bool) -> Result<Json> {
    let src = cx.ar.source(p);
    let mut kv = vec![
        ("source", s(&src.id)),
        ("repository", s(&src.url)),
        ("path", s(&cx.ar.upstream_path(p))),
        ("revision", s(&src.revision)),
        ("date", s(&src.date)),
        ("url", s(&cx.ar.upstream_url(p))),
        ("license", s(&src.license)),
    ];
    if with_blob {
        kv.push(("git_blob", s(&archive::git_blob_id(cx.ar.text(p)?.as_bytes()))));
    }
    Ok(obj(kv))
}

fn page_json(cx: &Context, i: usize, sheet: &Sheet, only: Option<&[usize]>, note: &str) -> Result<Json> {
    let p = &cx.ar.dir.pages[i];
    let all: Vec<usize> = (0..sheet.examples.len()).collect();
    let examples = only
        .unwrap_or(&all)
        .iter()
        .map(|&j| {
            let e = &sheet.examples[j];
            let code: Vec<String> = e.code.lines().map(|l| code_line(cx, p, l)).collect();
            obj(vec![("description", s(&e.desc)), ("command", s(&code.join("\n")))])
        })
        .collect();
    let mut kv = vec![
        ("name", s(&p.name)),
        ("source", s(&cx.ar.source(p).id)),
        ("kind", s(kind_name(p.kind))),
        ("platform", s(cx.ar.platform(p))),
        ("lang", s(cx.ar.lang(p))),
        ("summary", Json::Arr(sheet.summary.iter().map(|l| s(l)).collect())),
        ("more_info", sheet.more.as_deref().map_or(Json::Null, s)),
        ("syntax", s(&sheet.syntax)),
        ("tags", Json::Arr(sheet.tags.iter().map(|l| s(l)).collect())),
        ("examples", Json::Arr(examples)),
        ("see_also", Json::Arr(sheet.see_also.iter().map(|l| s(l)).collect())),
        ("upstream", upstream_json(cx, p, false)?),
    ];
    if !note.is_empty() {
        kv.push(("note", s(note)));
    }
    if cx.o.raw {
        kv.push(("text", s(&cx.ar.text(p)?)));
    }
    Ok(obj(kv))
}

fn sheet(cx: &Context, i: usize) -> Result<Sheet> {
    let p = &cx.ar.dir.pages[i];
    Ok(page::parse(p.kind, &p.name, &cx.ar.text(p)?))
}

// ---------------------------------------------------------------------------------------------
// Looking a command up

struct Pick {
    page: usize,
    note: String,
}

/// The pages to show for a name: sheets of one's own (there for this machine), the best tldr
/// page (platform before language, as tldr's clients do), then each other source's sheet.
fn pick(cx: &Context, k: &str) -> (Vec<Pick>, Vec<String>) {
    let Some(list) = cx.by_key.get(k) else { return (Vec::new(), Vec::new()) };
    let pages = &cx.ar.dir.pages;
    let mut picks = Vec::new();
    let mut see_also = Vec::new();
    let tldr: Vec<usize> = list.iter().copied().filter(|&i| pages[i].kind == KIND_TLDR && cx.source_ok(&pages[i])).collect();
    if !tldr.is_empty() {
        let plat = |i: usize| cx.ar.platform(&pages[i]).to_string();
        let mut best = None;
        'outer: for pl in &cx.platforms {
            for &l in &cx.langs {
                if let Some(&i) = tldr.iter().find(|&&i| plat(i) == *pl && pages[i].lang == l) {
                    best = Some(i);
                    break 'outer;
                }
            }
        }
        // Not on this platform nor common: another platform's page, said so.
        let best = best.or_else(|| {
            cx.langs.iter().find_map(|&l| tldr.iter().copied().find(|&i| pages[i].lang == l))
        });
        if let Some(b) = best {
            let mut notes = Vec::new();
            let bp = plat(b);
            if !cx.platforms[..2].contains(&bp) {
                notes.push(format!("(no {} or common page)", cx.platforms[0]));
            }
            if let Some(al) = &cx.asked_lang {
                if cx.ar.lang(&pages[b]) != al {
                    notes.push(format!("(no {al} translation)"));
                }
            }
            let mut others: Vec<String> = tldr.iter().map(|&i| plat(i)).filter(|x| *x != bp).collect();
            others.sort();
            others.dedup();
            if !others.is_empty() {
                notes.push(format!("(also: {})", others.join(", ")));
            }
            picks.push(Pick { page: b, note: notes.join(" ") });
        }
    }
    let mut own = Vec::new();
    for &i in list {
        let p = &pages[i];
        if !cx.source_ok(p) {
            continue;
        }
        match p.kind {
            _ if cx.ar.is_added(p) => own.push(Pick { page: i, note: String::new() }),
            KIND_SHEET | KIND_LANG | KIND_TOPIC => picks.push(Pick { page: i, note: String::new() }),
            KIND_SEE_ALSO => see_also.push(i),
            _ => {}
        }
    }
    own.append(&mut picks);
    let picks = own;
    let mut names = Vec::new();
    for i in see_also {
        if let Ok(sh) = sheet(cx, i) {
            names.extend(sh.see_also);
        }
    }
    (picks, names)
}

fn lookup(cx: &Context, words: &[String], out: &mut String) -> Result<i32> {
    // NAME~WORD as cheat.sh writes it.
    let mut words: Vec<String> = words.to_vec();
    if let Some((n, rest)) = words[0].split_once('~').filter(|(n, r)| !n.is_empty() && !r.is_empty()) {
        let (n, rest) = (n.to_string(), rest.to_string());
        words.splice(0..1, [n, rest]);
    }
    if words[0].contains('/') && !cx.by_key.contains_key(&key(&words[0])) {
        return lang_sheet(cx, &words, out);
    }
    // The longest run of words that names a page ("git commit" → git-commit).
    let mut found = None;
    for n in (1..=words.len()).rev() {
        let k = key(&words[..n].join(" "));
        if cx.by_key.contains_key(&k) {
            let (picks, see) = pick(cx, &k);
            if !picks.is_empty() {
                found = Some((picks, see, n));
                break;
            }
        }
    }
    let Some((picks, see_also, n)) = found else {
        return not_found(cx, &words, out);
    };
    let filter = &words[n..];
    let mut sheets: Vec<Sheet> = Vec::new();
    for p in &picks {
        let mut sh = sheet(cx, p.page)?;
        if sh.see_also.is_empty() && cx.ar.dir.pages[p.page].kind != KIND_TLDR && cx.ar.source(&cx.ar.dir.pages[p.page]).id == "cheat.sheets" {
            sh.see_also = see_also.clone();
        }
        sheets.push(sh);
    }
    if !see_also.is_empty() && !sheets.iter().any(|s| !s.see_also.is_empty()) {
        if let Some(first) = sheets.first_mut() {
            first.see_also = see_also.clone();
        }
    }
    // With words: the examples that mention them all, else those that mention the most.
    let mut only: Vec<Option<Vec<usize>>> = vec![None; picks.len()];
    let mut note = String::new();
    if !filter.is_empty() {
        let terms = search::query_terms(filter);
        let cover = |sh: &Sheet, j: usize| {
            let e = &sh.examples[j];
            search::coverage(&terms, &[&e.desc, &e.code])
        };
        let best = sheets
            .iter()
            .flat_map(|sh| (0..sh.examples.len()).map(move |j| (sh, j)))
            .map(|(sh, j)| cover(sh, j))
            .max()
            .unwrap_or(0);
        if best == 0 {
            let name = &cx.ar.dir.pages[picks[0].page].name;
            eprintln!("guided-arrow: no example of {name} mentions {}", filter.join(" "));
            return search_cmd_with_hint(cx, &words, out, name);
        }
        if best < terms.len() {
            note = format!("(no example mentions all of: {}; the closest)", filter.join(" "));
        }
        for (k, sh) in sheets.iter().enumerate() {
            only[k] = Some((0..sh.examples.len()).filter(|&j| cover(sh, j) == best).collect());
        }
    }
    if cx.o.json {
        let mut list = Vec::new();
        for (k, p) in picks.iter().enumerate() {
            if only[k].as_ref().is_some_and(|v| v.is_empty()) {
                continue;
            }
            list.push(page_json(cx, p.page, &sheets[k], only[k].as_deref(), &p.note)?);
        }
        let mut kv = vec![("query", Json::Arr(words.iter().map(|w| s(w)).collect())), ("pages", Json::Arr(list))];
        if !note.is_empty() {
            kv.push(("note", s(&note)));
        }
        out.push_str(&obj(kv).to_string());
        out.push('\n');
        return Ok(0);
    }
    for (k, p) in picks.iter().enumerate() {
        if only[k].as_ref().is_some_and(|v| v.is_empty()) {
            continue;
        }
        let n = [p.note.as_str(), note.as_str()].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(" ");
        render(cx, p.page, &sheets[k], only[k].as_deref(), &n, out)?;
    }
    Ok(0)
}

/// LANG/TOPIC, LANG/ (its topics), LANG/words+to+search.
fn lang_sheet(cx: &Context, words: &[String], out: &mut String) -> Result<i32> {
    let (lang, topic) = words[0].split_once('/').unwrap();
    let lang = lang.to_lowercase();
    let prefix = format!("{lang}/");
    let pages = &cx.ar.dir.pages;
    let topics: Vec<usize> =
        (0..pages.len()).filter(|&i| pages[i].kind == KIND_LANG && pages[i].name.to_lowercase().starts_with(&prefix)).collect();
    if topics.is_empty() {
        let mut langs: Vec<&str> = pages.iter().filter(|p| p.kind == KIND_LANG).filter_map(|p| p.name.split('/').next()).collect();
        langs.dedup();
        eprintln!("guided-arrow: no sheets for the language {lang:?}; there are: {}", langs.join(" "));
        return Ok(1);
    }
    if topic.is_empty() || topic == ":list" {
        if cx.o.json {
            let names = topics.iter().map(|&i| s(&pages[i].name)).collect();
            out.push_str(&obj(vec![("topics", Json::Arr(names))]).to_string());
            out.push('\n');
        } else {
            for &i in &topics {
                out.push_str(&pages[i].name);
                out.push('\n');
            }
        }
        return Ok(0);
    }
    let mut q: Vec<String> = topic.split(['+', ' ']).filter(|w| !w.is_empty()).map(String::from).collect();
    q.extend(words[1..].iter().cloned());
    let wanted = |i: usize| topics.contains(&i);
    let found = search::search(cx.ar, &q, &wanted, cx.o.limit)?;
    if found.hits.is_empty() {
        eprintln!("guided-arrow: nothing in the {lang} sheets about {}", q.join(" "));
        return Ok(1);
    }
    show_hits(cx, &q, &found, out)?;
    Ok(0)
}

fn not_found(cx: &Context, words: &[String], out: &mut String) -> Result<i32> {
    let k = key(&words[0]);
    // Close names: same start, or one or two edits away.
    let mut close: Vec<(usize, &str)> = cx
        .by_key
        .iter()
        .filter(|(n, v)| v.iter().any(|&i| cx.ar.dir.pages[i].kind != KIND_SEE_ALSO) && !n.contains('/'))
        .filter_map(|(n, _)| {
            let d = edit_distance(&k, n);
            let prefix = n.starts_with(&k) && k.len() >= 3;
            let near = d <= if k.chars().count() < 5 { 1 } else { 2 };
            (near || prefix).then_some((if prefix { 0 } else { d }, n.as_str()))
        })
        .collect();
    close.sort_by_key(|&(d, n)| (d, n.len(), n.to_string()));
    let close: Vec<&str> = close.into_iter().take(8).map(|(_, n)| n).collect();
    let mut msg = format!("guided-arrow: no page named {:?}", words[0]);
    if !close.is_empty() {
        msg.push_str(&format!(" (close: {})", close.join(", ")));
    }
    eprintln!("{msg}; the examples that best match the words follow");
    let wanted = |i: usize| cx.lang_ok(&cx.ar.dir.pages[i]) && cx.source_ok(&cx.ar.dir.pages[i]);
    let found = search::search(cx.ar, words, &wanted, cx.o.limit)?;
    if !found.hits.is_empty() {
        show_hits(cx, words, &found, out)?;
    } else if cx.o.json {
        out.push_str(&obj(vec![("query", Json::Arr(words.iter().map(|w| s(w)).collect())), ("results", Json::Arr(vec![]))]).to_string());
        out.push('\n');
    }
    Ok(1)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > 2 {
        return 3;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + (ca != cb) as usize).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

// ---------------------------------------------------------------------------------------------
// Searching

fn search_cmd(cx: &Context, words: &[String], out: &mut String) -> Result<i32> {
    let wanted = |i: usize| cx.lang_ok(&cx.ar.dir.pages[i]) && cx.source_ok(&cx.ar.dir.pages[i]) && platform_ok(cx, i);
    let found = search::search(cx.ar, words, &wanted, cx.o.limit)?;
    if found.hits.is_empty() {
        eprintln!("guided-arrow: nothing matches {}", words.join(" "));
        if cx.o.json {
            out.push_str(&obj(vec![("query", Json::Arr(words.iter().map(|w| s(w)).collect())), ("results", Json::Arr(vec![]))]).to_string());
            out.push('\n');
        }
        return Ok(1);
    }
    show_hits(cx, words, &found, out)?;
    Ok(0)
}

/// A page's examples did not mention the words: search all of that page's command family.
fn search_cmd_with_hint(cx: &Context, words: &[String], out: &mut String, _name: &str) -> Result<i32> {
    let code = search_cmd(cx, words, out)?;
    Ok(if code == 0 { 1 } else { code })
}

/// tldr pages of other platforms only when asked for one, or when none of ours has the name.
fn platform_ok(cx: &Context, i: usize) -> bool {
    let p = &cx.ar.dir.pages[i];
    if p.kind != KIND_TLDR {
        return true;
    }
    let pl = cx.ar.platform(p);
    if cx.platforms.iter().any(|x| x == pl) {
        return true;
    }
    // A page only another platform has (brew, winget) is still worth finding.
    let k = key(&p.name);
    !cx.by_key[&k].iter().any(|&j| {
        let q = &cx.ar.dir.pages[j];
        q.kind == KIND_TLDR && cx.platforms.iter().any(|x| x == cx.ar.platform(q))
    })
}

fn show_hits(cx: &Context, words: &[String], found: &search::Found, out: &mut String) -> Result<()> {
    if cx.o.json {
        let mut results = Vec::new();
        for h in &found.hits {
            let p = &cx.ar.dir.pages[h.page];
            let sh = &found.sheets[&h.page];
            let (desc, code) = match h.example {
                Some(j) => {
                    let e = &sh.examples[j];
                    let code: Vec<String> = e.code.lines().map(|l| code_line(cx, p, l)).collect();
                    (e.desc.clone(), code.join("\n"))
                }
                None => (sh.summary.join("\n"), String::new()),
            };
            results.push(obj(vec![
                ("name", s(&p.name)),
                ("source", s(&cx.ar.source(p).id)),
                ("kind", s(if h.example.is_some() { "example" } else { "page" })),
                ("platform", s(cx.ar.platform(p))),
                ("lang", s(cx.ar.lang(p))),
                ("description", s(&desc)),
                ("command", s(&code)),
                ("score", Json::Num((h.score * 1000.0).round() / 1000.0)),
                ("upstream", upstream_json(cx, p, false)?),
            ]));
        }
        out.push_str(&obj(vec![("query", Json::Arr(words.iter().map(|w| s(w)).collect())), ("results", Json::Arr(results))]).to_string());
        out.push('\n');
        return Ok(());
    }
    // Grouped by page, in the order of each page's best hit.
    let mut order: Vec<usize> = Vec::new();
    let mut by_page: HashMap<usize, (bool, Vec<usize>)> = HashMap::new();
    for h in &found.hits {
        if !by_page.contains_key(&h.page) {
            order.push(h.page);
        }
        let e = by_page.entry(h.page).or_default();
        match h.example {
            Some(j) => e.1.push(j),
            None => e.0 = true,
        }
    }
    for i in order {
        let (summary, examples) = &by_page[&i];
        let sh = &found.sheets[&i];
        let p = &cx.ar.dir.pages[i];
        header(cx, p, "", out);
        let marker = if sh.marker.is_empty() { "#" } else { &sh.marker };
        if *summary && !sh.summary.is_empty() {
            comment(cx, marker, &sh.summary.join("\n"), out);
            if examples.is_empty() {
                out.push('\n');
            }
        }
        for &j in examples {
            let e = &sh.examples[j];
            if !e.desc.is_empty() {
                comment(cx, marker, &e.desc, out);
            }
            for l in e.code.lines() {
                out.push_str(&code_line(cx, p, l));
                out.push('\n');
            }
            out.push('\n');
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Listing, provenance, checking

fn list(cx: &Context, words: &[String], out: &mut String) -> i32 {
    let prefix = words.first().map(|w| key(w)).unwrap_or_default();
    let mut names: Vec<(&str, Vec<String>)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for p in &cx.ar.dir.pages {
        if p.kind == KIND_SEE_ALSO || !cx.source_ok(p) || !cx.lang_ok(p) || !key(&p.name).starts_with(&prefix) {
            continue;
        }
        if p.kind == KIND_TLDR && cx.o.platform.is_some() && !cx.platforms.iter().any(|x| x == cx.ar.platform(p)) {
            continue;
        }
        let src = match p.kind {
            KIND_TLDR => format!("tldr/{}", cx.ar.platform(p)),
            _ => cx.ar.source(p).id.clone(),
        };
        let k = key(&p.name);
        match seen.get(&k) {
            Some(&n) => {
                if !names[n].1.contains(&src) {
                    names[n].1.push(src);
                }
            }
            None => {
                seen.insert(k, names.len());
                names.push((&p.name, vec![src]));
            }
        }
    }
    names.sort_by_key(|(n, _)| key(n));
    if cx.o.json {
        let list = names
            .iter()
            .map(|(n, srcs)| obj(vec![("name", s(n)), ("sources", Json::Arr(srcs.iter().map(|x| s(x)).collect()))]))
            .collect();
        out.push_str(&obj(vec![("pages", Json::Arr(list))]).to_string());
        out.push('\n');
    } else {
        for (n, _) in &names {
            out.push_str(n);
            out.push('\n');
        }
    }
    if names.is_empty() { 1 } else { 0 }
}

fn info(cx: &Context, words: &[String], out: &mut String) -> Result<i32> {
    let ar = cx.ar;
    let d = &ar.dir;
    if !words.is_empty() {
        let k = key(&words.join(" "));
        let Some(list) = cx.by_key.get(&k) else {
            eprintln!("guided-arrow: no page named {:?}", words.join(" "));
            return Ok(1);
        };
        let mut items = Vec::new();
        for &i in list {
            let p = &d.pages[i];
            if !cx.source_ok(p) || !cx.lang_ok(p) {
                continue;
            }
            if cx.o.json {
                items.push(obj(vec![
                    ("name", s(&p.name)),
                    ("kind", s(kind_name(p.kind))),
                    ("platform", s(ar.platform(p))),
                    ("lang", s(ar.lang(p))),
                    ("bytes", Json::Num(ar.text(p)?.len() as f64)),
                    ("upstream", upstream_json(cx, p, true)?),
                ]));
                continue;
            }
            let src = ar.source(p);
            out.push_str(&format!("{} [{}]\n", p.name, label(cx, p)));
            if src.url.is_empty() {
                out.push_str(&format!("  source:   {}\n", src.title));
            } else {
                out.push_str(&format!("  source:   {} ({})\n", src.title, src.url));
            }
            out.push_str(&format!("  path:     {}\n", ar.upstream_path(p)));
            if !src.revision.is_empty() {
                out.push_str(&format!("  revision: {} {}\n", src.revision, src.date));
            }
            let url = ar.upstream_url(p);
            if !url.is_empty() {
                out.push_str(&format!("  url:      {url}\n"));
            }
            if !src.license.is_empty() {
                out.push_str(&format!("  license:  {}\n", src.license));
            }
            let text = ar.text(p)?;
            out.push_str(&format!("  git blob: {} ({} bytes)\n", archive::git_blob_id(text.as_bytes()), text.len()));
        }
        if cx.o.json {
            out.push_str(&obj(vec![("pages", Json::Arr(items))]).to_string());
            out.push('\n');
        }
        return Ok(0);
    }
    let meta = |k: &str| d.meta.iter().find(|(a, _)| a == k).map_or("", |(_, v)| v.as_str()).to_string();
    // The archive's own figures; the folders of one's own sheets after them.
    let (own_pages, own_blocks) = (&d.pages[..ar.own_pages], &d.blocks[..ar.own_blocks]);
    let raw: u64 = own_blocks.iter().map(|b| b.raw_len as u64).sum();
    let folders: Vec<(&archive::Source, usize)> = (ar.own_sources..d.sources.len())
        .map(|i| (&d.sources[i], d.pages[ar.own_pages..].iter().filter(|p| p.source as usize == i).count()))
        .collect();
    let mut counts: Vec<(String, usize)> = Vec::new();
    for p in own_pages {
        let k = format!("{} {}", ar.source(p).id, kind_name(p.kind));
        match counts.iter_mut().find(|(n, _)| *n == k) {
            Some(c) => c.1 += 1,
            None => counts.push((k, 1)),
        }
    }
    let mut per_lang: Vec<(String, usize, u64)> = d.langs.iter().map(|l| (l.clone(), 0, 0)).collect();
    for p in own_pages {
        per_lang[p.lang as usize].1 += 1;
    }
    for b in own_blocks {
        per_lang[b.lang as usize].2 += b.comp_len as u64;
    }
    per_lang.retain(|(_, n, _)| *n > 0);
    if cx.o.json {
        let sources = d.sources[..ar.own_sources]
            .iter()
            .map(|x| {
                obj(vec![
                    ("id", s(&x.id)),
                    ("title", s(&x.title)),
                    ("url", s(&x.url)),
                    ("revision", s(&x.revision)),
                    ("date", s(&x.date)),
                    ("license", s(&x.license)),
                ])
            })
            .collect();
        let langs = per_lang
            .iter()
            .map(|(l, n, b)| obj(vec![("lang", s(l)), ("pages", Json::Num(*n as f64)), ("compressed_bytes", Json::Num(*b as f64))]))
            .collect();
        let local = folders
            .iter()
            .enumerate()
            .map(|(i, (x, n))| obj(vec![("folder", s(&x.url)), ("sheets", Json::Num(*n as f64)), ("add_writes_here", Json::Bool(i == 0))]))
            .collect();
        out.push_str(
            &obj(vec![
                ("archive", s(&ar.path)),
                ("bytes", Json::Num(ar.size as f64)),
                ("raw_bytes", Json::Num(raw as f64)),
                ("created", s(&meta("created"))),
                ("packer", s(&meta("packer"))),
                ("pages", Json::Num(own_pages.len() as f64)),
                ("blocks", Json::Num(own_blocks.len() as f64)),
                ("sources", Json::Arr(sources)),
                ("languages", Json::Arr(langs)),
                ("local", Json::Arr(local)),
            ])
            .to_string(),
        );
        out.push('\n');
        return Ok(0);
    }
    out.push_str(&format!("archive:  {}\n", ar.path));
    out.push_str(&format!(
        "size:     {} bytes for {} bytes of pages ({:.1}x), {} blocks\n",
        ar.size,
        raw,
        raw as f64 / ar.size.max(1) as f64,
        own_blocks.len()
    ));
    out.push_str(&format!("created:  {} by {}\n", meta("created"), meta("packer")));
    out.push_str("sources:\n");
    for x in &d.sources[..ar.own_sources] {
        out.push_str(&format!("  {:<13} {}\n", x.id, x.title));
        let mut at = x.url.clone();
        if !x.revision.is_empty() {
            at.push_str(&format!(" at {}", x.revision));
        }
        if !x.date.is_empty() {
            at.push_str(&format!(" ({})", x.date));
        }
        if !at.trim().is_empty() {
            out.push_str(&format!("  {:<13} {}\n", "", at.trim()));
        }
        if !x.license.is_empty() {
            out.push_str(&format!("  {:<13} license: {}\n", "", x.license));
        }
    }
    out.push_str("pages:\n");
    for (k, n) in &counts {
        out.push_str(&format!("  {k}: {n}\n"));
    }
    out.push_str("languages (pages, compressed bytes):\n  ");
    let langs: Vec<String> = per_lang.iter().map(|(l, n, b)| format!("{l} {n} {b}")).collect();
    out.push_str(&langs.join(", "));
    out.push('\n');
    if !folders.is_empty() {
        out.push_str("sheets of one's own (source local):\n");
        for (i, (x, n)) in folders.iter().enumerate() {
            let note = if i == 0 { "; --add writes here" } else { "" };
            out.push_str(&format!("  {} ({n} sheet{}{note})\n", x.url, if *n == 1 { "" } else { "s" }));
        }
    }
    Ok(0)
}

fn verify(ar: &Archive, out: &mut String) -> Result<i32> {
    let mut raw = 0u64;
    for i in 0..ar.own_blocks {
        raw += ar.block(i as u32)?.raw.len() as u64;
    }
    for p in &ar.dir.pages[..ar.own_pages] {
        ar.text(p)?;
    }
    out.push_str(&format!(
        "ok: {} blocks, {} pages, {} bytes of pages, every checksum matches\n",
        ar.own_blocks, ar.own_pages, raw
    ));
    // Sheets of one's own have no checksum; say which of them show nothing.
    let added = &ar.dir.pages[ar.own_pages..];
    if !added.is_empty() {
        let mut empty = Vec::new();
        for p in added {
            if page::parse(p.kind, &p.name, &ar.text(p)?).examples.is_empty() {
                empty.push(ar.upstream_path(p));
            }
        }
        out.push_str(&format!("and {} sheet{} of one's own (local)", added.len(), if added.len() == 1 { "" } else { "s" }));
        if empty.is_empty() {
            out.push('\n');
        } else {
            out.push_str(&format!("; with no examples: {}\n", empty.join(", ")));
        }
    }
    Ok(0)
}

// ---------------------------------------------------------------------------------------------
// Sheets of one's own: adding and removing

fn read_sheet(path: &std::path::Path) -> std::result::Result<String, Fail> {
    match std::fs::read(path) {
        Ok(b) => String::from_utf8(b).map_err(|_| Fail::Usage(format!("{}: not UTF-8; fix or remove it first", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(err!("{}: {e}", path.display()).into()),
    }
}

fn plural(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

/// --add NAME DESCRIPTION COMMAND (one example), or --add NAME with the text on stdin.
fn add_cmd(o: &Opts, f: &local::Folders, words: &[String], out: &mut String) -> std::result::Result<i32, Fail> {
    let form = "--add NAME DESCRIPTION COMMAND, or --add NAME with the sheet's text on stdin";
    let Some(raw) = words.first() else { return Err(Fail::Usage(format!("{form}"))) };
    let name = local::sheet_name(raw).map_err(Fail::Usage)?;
    let path = f.write.join(&name);
    let old = read_sheet(&path)?;
    let before = page::parse(KIND_SHEET, &name, &old).examples;
    // The example as it must read back (argument form).
    let mut want: Option<page::Example> = None;
    let addition = match words.len() {
        1 => {
            if std::io::stdin().is_terminal() {
                return Err(Fail::Usage(format!("{form} (here stdin is a terminal)")));
            }
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut std::io::stdin(), &mut b).map_err(|e| err!("stdin: {e}"))?;
            String::from_utf8(b).map_err(|_| Fail::Usage("the text on stdin is not UTF-8".into()))?
        }
        2 => return Err(Fail::Usage(format!("{form}: {raw} has a description but no command"))),
        _ => {
            let desc: Vec<&str> =
                words[1].lines().map(|l| l.trim().trim_start_matches('#').trim()).filter(|l| !l.is_empty()).collect();
            let joined = words[2..].join(" ");
            let code: Vec<&str> = joined.trim_matches('\n').lines().map(str::trim_end).collect();
            if desc.is_empty() || code.iter().all(|l| l.trim().is_empty()) {
                return Err(Fail::Usage(format!("{form}: the description and the command may not be empty")));
            }
            let e = page::Example { desc: desc.join("\n"), code: code.join("\n") };
            if let Some(same) = before.iter().find(|x| x.code.trim() == e.code.trim()) {
                out.push_str(&format!("already in {name} (as \"{}\"): {}\n", same.desc.replace('\n', " "), path.display()));
                return Ok(0);
            }
            let mut t: String = desc.iter().map(|l| format!("# {l}\n")).collect();
            t.push_str(&e.code);
            want = Some(e);
            t
        }
    };
    if addition.contains('\0') {
        return Err(Fail::Usage("a sheet may not hold a NUL byte".into()));
    }
    let addition = addition.trim_start_matches(['\n', '\r']).trim_end();
    let text = if old.trim().is_empty() { format!("{addition}\n") } else { format!("{}\n\n{addition}\n", old.trim_end()) };
    let after = page::parse(KIND_SHEET, &name, &text).examples;
    if after.len() <= before.len() {
        return Err(Fail::Usage(format!(
            "nothing to add to {name}: no example in it (an example is # description lines, then the command lines)"
        )));
    }
    if let Some(e) = &want {
        if after.last() != Some(e) || after.len() != before.len() + 1 {
            return Err(Fail::Usage(
                "the command does not read back as one example (a blank line in it, or a line starting with #?); give the sheet's text on stdin".into(),
            ));
        }
    }
    local::write_sheet(&path, &text)?;
    let added = after.len() - before.len();
    if o.json {
        let kv = vec![
            ("name", s(&name)),
            ("path", s(&path.display().to_string())),
            ("added", Json::Num(added as f64)),
            ("examples", Json::Num(after.len() as f64)),
        ];
        out.push_str(&obj(kv).to_string());
        out.push('\n');
    } else {
        let how = if old.trim().is_empty() { "a new sheet".to_string() } else { format!("{} in all", after.len()) };
        out.push_str(&format!("added {} to {name} ({how}): {}\n", plural(added, "example"), path.display()));
    }
    if f.ephemeral {
        eprintln!(
            "guided-arrow: {} is gone when the sandbox stops; to keep sheets, an app mounts a folder and names it in the add-on's \"sheets\" setting",
            f.write.display()
        );
    }
    Ok(0)
}

/// --remove NAME (the sheet) or --remove NAME WORD... (its examples that mention them all).
fn remove_cmd(o: &Opts, f: &local::Folders, words: &[String], out: &mut String) -> std::result::Result<i32, Fail> {
    let Some(raw) = words.first() else { return Err(Fail::Usage("--remove NAME [WORD...]".into())) };
    let name = local::sheet_name(raw).map_err(Fail::Usage)?;
    let path = f.write.join(&name);
    if !path.is_file() {
        eprintln!(
            "guided-arrow: no sheet {name} of one's own in {} (the archive's pages stay; guided-arrow -l --from local lists the sheets there are)",
            f.write.display()
        );
        return Ok(1);
    }
    let text = read_sheet(&path)?;
    let report = |out: &mut String, removed: usize, left: usize, gone: &str| {
        if o.json {
            let kv = vec![
                ("name", s(&name)),
                ("path", s(&path.display().to_string())),
                ("removed", Json::Num(removed as f64)),
                ("examples", Json::Num(left as f64)),
                ("text", s(gone)),
            ];
            out.push_str(&obj(kv).to_string());
            out.push('\n');
        } else if left == 0 {
            out.push_str(&format!("removed the sheet {name}: {}\n", path.display()));
        } else {
            out.push_str(&format!("removed {} from {name} ({} left): {}\n", plural(removed, "example"), left, path.display()));
        }
        if !o.json && !gone.trim().is_empty() {
            out.push('\n');
            out.push_str(gone.trim_end());
            out.push('\n');
        }
    };
    let (sheet, spans) = page::sheet_spans(KIND_SHEET, &name, &text);
    if words.len() == 1 {
        local::delete_sheet(&f.write, &path)?;
        report(out, sheet.examples.len(), 0, "");
        return Ok(0);
    }
    let terms = search::query_terms(&words[1..]);
    let hit: Vec<bool> = sheet.examples.iter().map(|e| search::coverage(&terms, &[&e.desc, &e.code]) == terms.len()).collect();
    let removed = hit.iter().filter(|&&h| h).count();
    if removed == 0 {
        eprintln!("guided-arrow: no example of {name} mentions {}", words[1..].join(" "));
        return Ok(1);
    }
    let (mut kept, mut gone) = (text[..spans[0].0].to_string(), String::new());
    for (j, &(a, z)) in spans.iter().enumerate() {
        if hit[j] { &mut gone } else { &mut kept }.push_str(&text[a..z]);
    }
    let left = sheet.examples.len() - removed;
    if left == 0 {
        local::delete_sheet(&f.write, &path)?;
    } else {
        local::write_sheet(&path, &format!("{}\n", kept.trim_end()))?;
    }
    report(out, removed, left, &gone);
    Ok(0)
}

// ---------------------------------------------------------------------------------------------

fn pack_cmd(args: Vec<String>) -> std::result::Result<i32, String> {
    let mut o = pack::Options {
        out: String::new(),
        sources: Vec::new(),
        langs: Some(vec!["en".to_string()]),
        block_size: 2 << 20,
        effort: Effort::BEST,
        dict: true,
        created: String::new(),
        quiet: false,
    };
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "-o" | "--output" => o.out = value()?,
            "--source" => {
                let v = value()?;
                let mut parts = v.split(',');
                let (id, dir) = parts.next().and_then(|p| p.split_once('=')).ok_or_else(|| format!("--source {v}: NAME=DIR"))?;
                let mut spec = pack::SourceSpec { id: id.to_string(), dir: dir.into(), revision: String::new(), date: String::new() };
                for p in parts {
                    match p.split_once('=') {
                        Some(("rev", r)) => spec.revision = r.to_string(),
                        Some(("date", d)) => spec.date = d.to_string(),
                        _ => return Err(format!("--source: unknown {p:?} (rev=, date=)")),
                    }
                }
                if spec.revision.is_empty() {
                    spec.revision = pack::git_head(&spec.dir).unwrap_or_default();
                }
                o.sources.push(spec);
            }
            "--langs" => {
                let v = value()?;
                o.langs = if v == "all" { None } else { Some(v.split(',').map(String::from).collect()) };
            }
            "--block-size" => o.block_size = value()?.parse().map_err(|_| "--block-size: a number of bytes".to_string())?,
            "--no-dict" => o.dict = false,
            "--fast" => o.effort = Effort::FAST,
            "--created" => o.created = value()?,
            "--quiet" | "-q" => o.quiet = true,
            _ => return Err(format!("--pack: unknown argument {a}")),
        }
    }
    if o.out.is_empty() || o.sources.is_empty() {
        return Err("--pack needs -o FILE and at least one --source NAME=DIR".into());
    }
    pack::pack(&o).map_err(|e| e.0)?;
    Ok(0)
}
