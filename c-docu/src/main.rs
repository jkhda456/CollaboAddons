//! docu: office documents on the command line, edited in place and in part.

mod doc;
mod biff;
mod blank;
mod cfb;
mod chart_edit;
mod convert;
mod dml;
mod docx;
mod emf;
mod formula;
mod model;
mod numfmt;
mod opc;
mod ppt;
mod ppt_pptx;
mod picture;
mod pptx;
#[cfg(feature = "render")]
mod render;
mod props;
mod text;
mod word97;
mod word97_docx;
mod xls;
mod xlsx;
mod xlsx_ops;
mod xml;
mod zip;

use doc::AnyDoc;
use props::Props;
use serde_json::json;
use std::io::{Read, Write};
use text::Finder;

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn err(s: impl Into<String>) -> Error {
    Error(s.into())
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
docu — read, edit, create and render Word, Excel and PowerPoint files (docx xlsx pptx, and
the old doc xls ppt) in place, changing only what you name and keeping everything else.

usage: docu COMMAND FILE [ARGS] [OPTIONS]

look
  info FILE                      format, parts, counts
  read FILE [ADDR...] [--runs]   the content with an address for each piece; --runs shows each
                                 run's formatting; --json for the same as data
  text FILE                      plain text
  find FILE PATTERN              where text occurs: address, character range, context

edit (in place; -o OUT writes a copy instead, --dry-run writes nothing)
  replace FILE OLD NEW           replace text wherever it is (across runs), keeping its formatting
                                 --in ADDR  --nth N  --expect N  -i -w --regex ($1 in NEW)
  set FILE ADDR TEXT             the whole text of a paragraph, shape, cell or range; a cell
                                 takes numbers, TRUE, dates, =formulas; a range \"a|b;c|d\"
  style FILE ADDR PROP...        formatting: bold color=red size=14pt align=center fill=#FFF2CC
                                 --match TEXT (--nth N) styles only that text
  insert FILE ADDR TEXT [PROP..] new paragraph(s) after ADDR (--before), one per line;
                                 a row address (t1.r2, Sheet1!3:3, B:B) inserts rows/columns
  delete FILE ADDR...            paragraphs, characters (p3[0:5]), tables, rows, shapes, slides,
                                 rows/columns of a sheet (Sheet1!4:6), cell contents
  table FILE ADDR ROWS           new Word table after paragraph ADDR: \"a|b;c|d\" or a CSV file
  image FILE ADDR IMAGE [PROP..] a picture (PNG, JPEG, GIF, BMP, TIFF, EMF): after a paragraph,
                                 on a slide (s2), at a cell (Sheet1!B3); w= h= x= y= alt= name=;
                                 on a picture shape (s2.sh4) or with --replace (Word), swaps it
  textbox FILE SLIDE TEXT [PROP] a text box on a slide: x= y= w= h= size= color= ...
  chart FILE ADDR [title T|data TABLE]  a chart's series and the cells they read; its title;
                                 its data (Word, PowerPoint): \"Category|S1|S2;Q1|10|12;Q2|11|14\"
                                 (in a workbook, set the cells: the chart follows)
  slide FILE list|add [AFTER] [--layout NAME] [--title T]|dup N|delete N|move N TO|hide N|show N
  sheet FILE list|add NAME [AFTER]|rename OLD NEW|delete NAME|hide NAME|show NAME
  recalc FILE                    recalculate every formula of a workbook (edits do this too)
  batch FILE [SCRIPT]            several edits, one per line (or -e 'CMD ARGS' ...), saved once

make
  new FILE.docx|xlsx|pptx [TEXT [SUBTITLE]]   an empty document with the usual styles, a
                                 workbook with Sheet1, or a deck with a title slide (16:9)
  convert FILE OUT               doc → docx, xls → xlsx, ppt → pptx; any → pdf png svg txt md csv json
  render FILE -o OUT.pdf|png|svg lay it out as Office would: --pages 1,3-5  --ppi 96; several
                                 pages to PNG/SVG make OUT-1.png, OUT-2.png ... (or OUT-{p}.png)

addresses
  Word        p3  p3-7  p3[2:9] (characters 2..8)  t1  t1.r2  t1.r2.c3  t1.r2.c3.p1
              hdr1.p1  ftr1.p1  fn2.p1 (footnote 2)  en1  cmt0 (comment 0)  body
  Excel       Sheet1!B3  B3 (first sheet)  'My sheet'!A1:C9  Sheet1!B:B  Sheet1!3:5  Sheet1
  PowerPoint  s2  s2.sh3 (shape 3)  s2.sh3.p1  s2.sh3.sh1 (in a group)  s2.sh3.r1.c2 (table)
              s2.notes  s2.sh[Title 1] (by name)
  .doc        p3  hdr.p1  fn.p1  txbx.p2;  .ppt  s1.sh2.p1  s1.notes;  .xls  as Excel
Text: \\t is a tab. Setting one paragraph, \\n is a line break and a blank line (\\n\\n) starts
another paragraph; inserting, or setting a shape, cell or notes, each line is a paragraph.

`docu help style` lists the properties; `docu help formulas` the functions.";

struct Args {
    pos: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

const VALUE_FLAGS: &[&str] = &["-o", "--out", "--in", "--nth", "--expect", "--match", "-e", "--pages", "--dpi", "--format", "--sheet", "--like", "--count", "--layout", "--title", "--ppi"];
const BOOL_FLAGS: &[&str] = &["--force", "-f", "--replace", "--regex", "-r", "-i", "--ignore-case", "-w", "--word", "--json", "--dry-run", "-n", "--before", "--after", "--runs", "--all", "--header", "--plain", "-q", "--quiet", "--help", "-h", "--formulas", "--typst"];

impl Args {
    fn parse(v: &[String]) -> Result<Args> {
        let mut pos = Vec::new();
        let mut flags = Vec::new();
        let mut i = 0;
        let mut only_pos = false;
        while i < v.len() {
            let a = &v[i];
            i += 1;
            if only_pos || !a.starts_with('-') || a == "-" || a.parse::<f64>().is_ok() {
                pos.push(a.clone());
                continue;
            }
            if a == "--" {
                only_pos = true;
                continue;
            }
            let (name, inline) = match a.split_once('=') {
                Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
                _ => (a.clone(), None),
            };
            if VALUE_FLAGS.contains(&name.as_str()) {
                let val = match inline {
                    Some(v) => v,
                    None => {
                        let v2 = v.get(i).cloned().ok_or_else(|| err(format!("{name} needs a value")))?;
                        i += 1;
                        v2
                    }
                };
                flags.push((name, Some(val)));
            } else if BOOL_FLAGS.contains(&name.as_str()) {
                flags.push((name, None));
            } else {
                return Err(err(format!("unknown option {a} (docu help)")));
            }
        }
        Ok(Args { pos, flags })
    }
    fn has(&self, names: &[&str]) -> bool {
        self.flags.iter().any(|(n, _)| names.contains(&n.as_str()))
    }
    fn get(&self, names: &[&str]) -> Option<&str> {
        self.flags.iter().rev().find(|(n, _)| names.contains(&n.as_str())).and_then(|(_, v)| v.as_deref())
    }
    fn all(&self, names: &[&str]) -> Vec<String> {
        self.flags.iter().filter(|(n, _)| names.contains(&n.as_str())).filter_map(|(_, v)| v.clone()).collect()
    }
    fn num(&self, names: &[&str]) -> Result<Option<usize>> {
        self.get(names).map(|v| v.parse().map_err(|_| err(format!("{} needs a number", names[0])))).transpose()
    }
    fn need(&self, i: usize, what: &str) -> Result<&str> {
        self.pos.get(i).map(|s| s.as_str()).ok_or_else(|| err(format!("missing {what}")))
    }
}

fn finder(a: &Args, pattern: &str) -> Result<Finder> {
    let regex = a.has(&["--regex", "-r"]);
    let pat = if regex { pattern.to_string() } else { text::unescape(pattern) };
    Finder::new(&pat, regex, a.has(&["-i", "--ignore-case"]), a.has(&["-w", "--word"]))
}

/// What a command did to the document, for the report.
struct Outcome {
    lines: Vec<String>,
    json: serde_json::Value,
    changed: bool,
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&argv) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("docu: {e}");
            1
        }
    };
    let _ = std::io::stdout().flush();
    std::process::exit(code);
}

fn run(argv: &[String]) -> Result<i32> {
    let Some(cmd) = argv.first() else {
        println!("{USAGE}");
        return Ok(2);
    };
    let rest = &argv[1..];
    match cmd.as_str() {
        "help" | "-h" | "--help" => {
            match rest.first().map(|s| s.as_str()) {
                Some("style") | Some("props") | Some("properties") => println!("{}", props::HELP),
                Some("formulas") | Some("functions") | Some("formula") => {
                    let mut names: Vec<&str> = xlsx::KNOWN_FUNCTIONS.iter().copied().collect();
                    names.sort();
                    println!("Formulas are recalculated on save (and by `docu recalc`). Functions:\n{}", names.join(" "));
                }
                _ => println!("{USAGE}"),
            }
            return Ok(0);
        }
        "version" | "--version" | "-V" => {
            println!("docu {VERSION} (collabo-docu)");
            return Ok(0);
        }
        _ => {}
    }
    let a = Args::parse(rest)?;
    if a.has(&["--help", "-h"]) {
        println!("{USAGE}");
        return Ok(0);
    }
    let file = a.need(0, "FILE")?.to_string();
    match cmd.as_str() {
        "new" | "create" => {
            if std::path::Path::new(&file).exists() && !a.has(&["--force", "-f"]) {
                return Err(err(format!("{file} exists (--force replaces it)")));
            }
            let bytes = blank::new(&file)?;
            // Optional content: paragraphs, cells ("a|b;c|d") or the title slide's title.
            let mut d = AnyDoc::from_bytes(bytes, &file)?;
            if let Some(t) = a.pos.get(1) {
                match &mut d {
                    AnyDoc::Docx(_) => {
                        d.set("p1", t)?;
                    }
                    AnyDoc::Xlsx(_) => {
                        d.set("A1", t)?;
                    }
                    AnyDoc::Pptx(_) => {
                        d.set("s1.sh1", t)?;
                        if let Some(sub) = a.pos.get(2) {
                            d.set("s1.sh2", sub)?;
                        }
                    }
                    _ => {}
                }
            }
            write_atomic(&file, &d.save()?)?;
            println!("created {file}");
            return Ok(0);
        }
        "info" => {
            let d = AnyDoc::open(&file)?;
            let o = d.outline(false, &[])?;
            let mut counts: Vec<(String, usize)> = Vec::new();
            for it in &o.items {
                match counts.iter_mut().find(|(k, _)| *k == it.kind) {
                    Some((_, n)) => *n += 1,
                    None => counts.push((it.kind.clone(), 1)),
                }
            }
            if a.has(&["--json"]) {
                let c: serde_json::Map<String, serde_json::Value> = counts.iter().map(|(k, n)| (k.clone(), json!(n))).collect();
                println!("{}", json!({"file": file, "format": d.kind(), "counts": c}));
            } else {
                println!("{file}: {}", d.kind());
                for (k, n) in counts {
                    println!("  {k}: {n}");
                }
            }
            Ok(0)
        }
        "read" | "cat" | "show" => {
            let d = AnyDoc::open(&file)?;
            let o = d.outline(a.has(&["--runs"]), &a.pos[1..])?;
            if a.has(&["--json"]) {
                println!("{}", serde_json::to_string_pretty(&o.json()).unwrap());
            } else {
                let mut s = String::new();
                if a.has(&["--plain"]) {
                    o.plain(&mut s);
                } else {
                    o.print(&mut s);
                }
                print!("{s}");
            }
            Ok(0)
        }
        "text" => {
            let d = AnyDoc::open(&file)?;
            print!("{}", d.plain()?);
            Ok(0)
        }
        "find" | "grep" | "search" => {
            let d = AnyDoc::open(&file)?;
            let f = finder(&a, a.need(1, "PATTERN")?)?;
            let ms = d.find(&f, a.get(&["--in"]), a.has(&["--formulas"]))?;
            if a.has(&["--json"]) {
                let v: Vec<_> = ms.iter().map(|m| json!({"addr": m.addr, "start": m.start, "end": m.end, "text": m.text, "context": m.context})).collect();
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
            } else {
                for m in &ms {
                    println!("{}[{}:{}] {}", m.addr, m.start, m.end, m.context);
                }
                if !a.has(&["-q", "--quiet"]) {
                    eprintln!("{} match{}", ms.len(), if ms.len() == 1 { "" } else { "es" });
                }
            }
            Ok(if ms.is_empty() { 1 } else { 0 })
        }
        "render" | "export" | "print" => {
            #[cfg(feature = "render")]
            {
                let d = AnyDoc::open(&file)?;
                let out = match a.get(&["-o", "--out"]) {
                    Some(o) => o.to_string(),
                    None => {
                        let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(&file);
                        format!("{stem}.{}", a.pos.get(1).map(|s| s.as_str()).unwrap_or("pdf"))
                    }
                };
                let opts = render::Options {
                    pages: a.get(&["--pages"]).map(render::parse_pages).transpose()?,
                    ppi: a.get(&["--ppi", "--dpi"]).map(|v| v.parse::<f64>().map_err(|_| err("--ppi needs a number"))).transpose()?.unwrap_or(96.0),
                    keep_typst: a.has(&["--typst"]),
                };
                let (written, warnings) = render::render(&d, &out, &opts)?;
                for w in &warnings {
                    eprintln!("warning: {w}");
                }
                if a.has(&["--json"]) {
                    println!("{}", json!({"written": written, "warnings": warnings}));
                } else {
                    for w in &written {
                        println!("wrote {w}");
                    }
                }
                return Ok(0);
            }
            #[cfg(not(feature = "render"))]
            return Err(err("this docu was built without rendering"));
        }
        "convert" | "save-as" => {
            let d = AnyDoc::open(&file)?;
            let out = a.need(1, "OUT")?.to_string();
            let ext = out.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            let bytes: Vec<u8> = match (&d, ext.as_str()) {
                (AnyDoc::Xls(x), "xlsx") => convert::xls_to_xlsx(x)?,
                (AnyDoc::Doc(x), "docx") => convert::doc_to_docx(x)?,
                (AnyDoc::Ppt(x), "pptx") => ppt_pptx::ppt_to_pptx(x)?,
                (_, "txt") => d.plain()?.into_bytes(),
                (_, "md" | "markdown") => convert::markdown(&d)?.into_bytes(),
                (_, "csv") => convert::csv(&d, a.get(&["--sheet"]))?.into_bytes(),
                (_, "json") => serde_json::to_string_pretty(&d.outline(false, &[])?.json()).unwrap().into_bytes(),
                #[cfg(feature = "render")]
                (_, "pdf" | "png" | "svg") => {
                    let opts = render::Options { pages: a.get(&["--pages"]).map(render::parse_pages).transpose()?, ppi: 96.0, keep_typst: false };
                    let (written, _) = render::render(&d, &out, &opts)?;
                    for w in written {
                        println!("wrote {w}");
                    }
                    return Ok(0);
                }
                (_, e) => return Err(err(format!("cannot convert a {} to .{e} (doc→docx, xls→xlsx, ppt→pptx; any → pdf, png, svg, txt, md, csv, json)", d.kind()))),
            };
            write_atomic(&out, &bytes)?;
            println!("wrote {out}");
            Ok(0)
        }
        "batch" | "edit" => {
            let mut d = AnyDoc::open(&file)?;
            let mut lines: Vec<String> = a.all(&["-e"]);
            if let Some(script) = a.pos.get(1) {
                let src = if script == "-" {
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s).map_err(|e| err(e.to_string()))?;
                    s
                } else {
                    std::fs::read_to_string(script).map_err(|e| err(format!("{script}: {e}")))?
                };
                lines.extend(src.lines().map(str::to_string));
            } else if lines.is_empty() {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s).map_err(|e| err(e.to_string()))?;
                lines.extend(s.lines().map(str::to_string));
            }
            let mut changed = false;
            let mut report = Vec::new();
            for (n, line) in lines.iter().enumerate() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let words = split_words(line).map_err(|e| err(format!("line {}: {e}", n + 1)))?;
                let (c, r) = words.split_first().unwrap();
                let mut full = vec![file.clone()];
                full.extend(r.iter().cloned());
                let sub = Args::parse(&full).map_err(|e| err(format!("line {}: {e}", n + 1)))?;
                let o = edit(&mut d, c, &sub).map_err(|e| err(format!("line {} ({c}): {e}", n + 1)))?;
                changed |= o.changed;
                for l in o.lines {
                    report.push(format!("{c}: {l}"));
                }
            }
            finish(&mut d, &a, &file, changed, report, json!(null))
        }
        c => {
            let mut d = AnyDoc::open(&file)?;
            let o = edit(&mut d, c, &a)?;
            finish(&mut d, &a, &file, o.changed, o.lines, o.json)
        }
    }
}

fn finish(d: &mut AnyDoc, a: &Args, file: &str, changed: bool, mut lines: Vec<String>, j: serde_json::Value) -> Result<i32> {
    let out = a.get(&["-o", "--out"]).unwrap_or(file).to_string();
    let dry = a.has(&["--dry-run", "-n"]);
    if changed && !dry {
        let bytes = d.save()?;
        write_atomic(&out, &bytes)?;
    }
    lines.extend(d.warnings().into_iter().map(|w| format!("warning: {w}")));
    if a.has(&["--json"]) {
        println!("{}", json!({"changed": changed, "saved": if changed && !dry { Some(&out) } else { None }, "result": j, "report": lines}));
    } else if !a.has(&["-q", "--quiet"]) {
        for l in &lines {
            println!("{l}");
        }
        if changed {
            if dry {
                println!("(dry run: {out} not written)");
            } else {
                println!("saved {out}");
            }
        }
    }
    Ok(0)
}

fn write_atomic(path: &str, bytes: &[u8]) -> Result<()> {
    let p = std::path::Path::new(path);
    let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "out".into());
    let tmp = dir.join(format!(".{name}.docu-{}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| err(format!("{}: {e}", tmp.display())))?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        err(format!("{path}: {e}"))
    })
}

/// Counts of addresses: `p2, p5 (x2)`.
fn summarize(addrs: &[String]) -> String {
    let mut out: Vec<(String, usize)> = Vec::new();
    for a in addrs {
        match out.iter_mut().find(|(x, _)| x == a) {
            Some((_, n)) => *n += 1,
            None => out.push((a.clone(), 1)),
        }
    }
    out.iter().map(|(a, n)| if *n > 1 { format!("{a} (x{n})") } else { a.clone() }).collect::<Vec<_>>().join(", ")
}

/// One editing command on an open document.
fn edit(d: &mut AnyDoc, cmd: &str, a: &Args) -> Result<Outcome> {
    match cmd {
        "replace" | "sub" | "s" => {
            let old = a.need(1, "OLD text")?;
            let new = a.need(2, "NEW text")?;
            let f = finder(a, old)?;
            let done = d.replace(&f, new, a.get(&["--in"]), a.num(&["--nth"])?, a.has(&["--formulas"]))?;
            if let Some(n) = a.num(&["--expect"])? {
                if done.len() != n {
                    return Err(err(format!("expected {n} matches of {old:?}, found {} (nothing written)", done.len())));
                }
            }
            if done.is_empty() {
                return Err(err(format!("no match for {old:?} (nothing written)")));
            }
            Ok(Outcome {
                lines: vec![format!("replaced {} occurrence{} in {}", done.len(), if done.len() == 1 { "" } else { "s" }, summarize(&done))],
                json: json!({"replaced": done.len(), "where": done}),
                changed: true,
            })
        }
        "set" => {
            let addr = a.need(1, "ADDR")?;
            let value = a.need(2, "TEXT")?;
            let at = d.set(addr, value)?;
            Ok(Outcome { lines: vec![format!("set {at}")], json: json!({"set": at}), changed: true })
        }
        "style" | "format" => {
            let addr = a.need(1, "ADDR")?;
            let props = Props::parse(&a.pos[2..])?;
            if props.keys.is_empty() {
                return Err(err(format!("style needs properties\n{}", props::HELP)));
            }
            let m = a.get(&["--match"]).map(|t| finder(a, t)).transpose()?;
            let nth = a.num(&["--nth"])?;
            let done = d.style(addr, &props, m.as_ref().map(|f| (f, nth)))?;
            Ok(Outcome { lines: vec![format!("styled {}", summarize(&done))], json: json!({"styled": done}), changed: true })
        }
        "insert" | "add" => {
            let addr = a.need(1, "ADDR")?;
            let t = a.need(2, "TEXT")?;
            let props = if a.pos.len() > 3 { Some(Props::parse(&a.pos[3..])?) } else { None };
            let done = d.insert(addr, !a.has(&["--before"]), t, props.as_ref(), a.num(&["--count"])?.unwrap_or(1))?;
            Ok(Outcome { lines: vec![format!("inserted {}", done.join(", "))], json: json!({"inserted": done}), changed: true })
        }
        "delete" | "del" | "rm" => {
            if a.pos.len() < 2 {
                return Err(err("missing ADDR"));
            }
            let mut lines = Vec::new();
            // Last first, so the numbers of the others stay right.
            let mut addrs: Vec<&String> = a.pos[1..].iter().collect();
            addrs.sort_by_key(|s| std::cmp::Reverse(sort_key(s)));
            for addr in addrs {
                lines.push(d.delete(addr)?);
            }
            Ok(Outcome { lines, json: json!(null), changed: true })
        }
        "sheet" | "sheets" => {
            let op = a.pos.get(1).map(|s| s.as_str()).unwrap_or("list");
            match d {
                AnyDoc::Xlsx(x) => {
                    let name = a.pos.get(2).cloned();
                    let need = |w: &str| name.clone().ok_or_else(|| err(format!("sheet {op} needs {w}")));
                    let line = match op {
                        "list" | "ls" => {
                            let lines: Vec<String> = x.sheets.iter().enumerate().map(|(i, s)| format!("{} {}{}", i + 1, s.name, if s.hidden { " (hidden)" } else if s.doc.is_none() { " (chart)" } else { "" })).collect();
                            return Ok(Outcome { lines, json: json!(x.sheet_names()), changed: false });
                        }
                        "add" | "new" => {
                            let n = need("a NAME")?;
                            let after = a.pos.get(3).and_then(|s| x.sheet_index(s));
                            let i = x.add_sheet(&n, after)?;
                            format!("added sheet {n} (position {})", i + 1)
                        }
                        "rename" | "mv" => {
                            let n = need("OLD NEW")?;
                            let new = a.pos.get(3).ok_or_else(|| err("sheet rename needs OLD NEW"))?;
                            x.rename_sheet(&n, new)?;
                            format!("renamed {n} to {new}")
                        }
                        "delete" | "del" | "rm" => {
                            let n = need("a NAME")?;
                            x.delete_sheet(&n)?;
                            format!("deleted sheet {n}")
                        }
                        "hide" | "show" | "unhide" => {
                            let n = need("a NAME")?;
                            x.set_sheet_hidden(&n, op == "hide")?;
                            format!("{} sheet {n}", if op == "hide" { "hid" } else { "showed" })
                        }
                        _ => return Err(err("sheet list|add NAME|rename OLD NEW|delete NAME|hide NAME|show NAME")),
                    };
                    Ok(Outcome { lines: vec![line], json: json!(null), changed: true })
                }
                _ => Err(err("sheet is for workbooks")),
            }
        }
        "slide" | "slides" => {
            let AnyDoc::Pptx(p) = d else { return Err(err("slide is for presentations")) };
            let op = a.pos.get(1).map(|s| s.as_str()).unwrap_or("list");
            let num = |i: usize, what: &str| -> Result<usize> {
                let s = a.pos.get(i).ok_or_else(|| err(format!("slide {op} needs {what}")))?;
                let n: usize = s.trim_start_matches('s').parse().map_err(|_| err(format!("{s:?} is not a slide number")))?;
                if n == 0 || n > p.slide_count() {
                    return Err(err(format!("there are {} slides", p.slide_count())));
                }
                Ok(n - 1)
            };
            let line = match op {
                "list" | "ls" => {
                    let o = p.outline(false, &[])?;
                    let lines: Vec<String> = o.items.iter().filter(|i| i.kind == "slide").map(|i| format!("{} {}", i.addr, i.style.clone().unwrap_or_default())).collect();
                    return Ok(Outcome { lines, json: json!(null), changed: false });
                }
                "add" | "new" => {
                    let after = if a.pos.len() > 2 && a.pos[2].trim_start_matches('s').parse::<usize>().is_ok() { Some(num(2, "N")?) } else { None };
                    let layout = a.get(&["--layout"]);
                    let title = a.get(&["--title"]);
                    let i = p.add_slide(layout, after, title)?;
                    format!("added slide s{}", i + 1)
                }
                "dup" | "duplicate" | "copy" => {
                    let i = p.duplicate_slide(num(2, "N")?)?;
                    format!("added slide s{} (a copy)", i + 1)
                }
                "delete" | "del" | "rm" => {
                    let i = num(2, "N")?;
                    p.delete_slide(i)?;
                    format!("deleted slide s{}", i + 1)
                }
                "move" | "mv" => {
                    let from = num(2, "N TO")?;
                    let to = num(3, "N TO")?;
                    p.move_slide(from, to)?;
                    format!("moved slide s{} to s{}", from + 1, to + 1)
                }
                "hide" | "show" => {
                    let i = num(2, "N")?;
                    p.set_hidden(i, op == "hide");
                    format!("{} slide s{}", if op == "hide" { "hid" } else { "showed" }, i + 1)
                }
                _ => return Err(err("slide list|add [AFTER] [--layout NAME] [--title T]|dup N|delete N|move N TO|hide N|show N")),
            };
            Ok(Outcome { lines: vec![line], json: json!(null), changed: true })
        }
        "image" | "picture" | "img" => {
            let addr = a.need(1, "ADDR")?;
            let path = a.need(2, "IMAGE")?;
            let img = picture::Image::load(path)?;
            let props = Props::parse(&a.pos[3..])?;
            let line = match d {
                AnyDoc::Docx(x) => x.insert_image(addr, !a.has(&["--before"]), &img, &props, a.has(&["--replace"]), a.num(&["--nth"])?)?,
                AnyDoc::Pptx(x) => x.insert_image(addr, &img, &props)?,
                AnyDoc::Xlsx(x) => x.insert_image(addr, &img, &props)?,
                _ => return Err(err(format!("pictures go in .docx, .pptx and .xlsx files (docu convert this {} first)", d.kind()))),
            };
            Ok(Outcome { lines: vec![line], json: json!(null), changed: true })
        }
        "chart" | "charts" => {
            // docu chart FILE ADDR [title TEXT | title none | data TABLE]  (--nth N: which chart)
            let addr = a.need(1, "ADDR (a paragraph, a slide or shape, a sheet)")?;
            let parts = chart_edit::charts_at(d, addr)?;
            if parts.is_empty() {
                return Err(err(format!("no chart at {addr}")));
            }
            let nth = a.num(&["--nth"])?.unwrap_or(1);
            let part = parts.get(nth - 1).ok_or_else(|| err(format!("{addr} has {} chart{}", parts.len(), if parts.len() == 1 { "" } else { "s" })))?.clone();
            match a.pos.get(2).map(|s| s.as_str()) {
                None | Some("show") => {
                    let pkg = chart_edit::pkg_of(d).unwrap();
                    let info = chart_edit::info(pkg, &part)?;
                    let mut lines: Vec<String> = Vec::new();
                    if parts.len() > 1 {
                        lines.push(format!("chart {nth} of {} at {addr} (--nth N for another)", parts.len()));
                    }
                    lines.extend(info.print().lines().map(str::to_string));
                    Ok(Outcome { lines, json: info.json(), changed: false })
                }
                Some("title") => {
                    let t = a.need(3, "TITLE (or none)")?;
                    let t = text::unescape(t);
                    chart_edit::set_title(d, &part, if t == "none" { None } else { Some(&t) })?;
                    Ok(Outcome { lines: vec![format!("set the title of {part}")], json: json!({"chart": part}), changed: true })
                }
                Some("data") => {
                    let spec = a.need(3, "TABLE (\"Category|S1|S2;Q1|1|2\" or a CSV file)")?;
                    let rows: Vec<Vec<String>> = if std::path::Path::new(spec).is_file() {
                        read_csv(&std::fs::read_to_string(spec).map_err(|e| err(e.to_string()))?)
                    } else {
                        spec.split(';').map(|r| r.split('|').map(|c| text::unescape(c.trim())).collect()).collect()
                    };
                    let line = chart_edit::set_data(d, &part, &rows)?;
                    Ok(Outcome { lines: vec![format!("set the data: {line}")], json: json!({"chart": part}), changed: true })
                }
                Some(other) => Err(err(format!("chart FILE ADDR [show | title TEXT | title none | data \"Category|S1;Q1|1\"], not {other:?}"))),
            }
        }
        "textbox" => {
            let AnyDoc::Pptx(p) = d else { return Err(err("textbox is for presentations")) };
            let s = a.need(1, "SLIDE")?;
            let n: usize = s.trim_start_matches('s').parse().map_err(|_| err(format!("{s:?} is not a slide")))?;
            if n == 0 || n > p.slide_count() {
                return Err(err(format!("there are {} slides", p.slide_count())));
            }
            let t = a.need(2, "TEXT")?;
            let props = Props::parse(&a.pos[3..])?;
            let addr = p.add_textbox(n - 1, t, &props)?;
            Ok(Outcome { lines: vec![format!("added {addr}")], json: json!({"shape": addr}), changed: true })
        }
        "recalc" => {
            let (n, w) = d.recalc()?;
            let mut lines = vec![format!("recalculated {n} formula{}", if n == 1 { "" } else { "s" })];
            lines.extend(w.into_iter().map(|x| format!("warning: {x}")));
            Ok(Outcome { lines, json: json!({"recalculated": n}), changed: true })
        }
        "table" => {
            let addr = a.need(1, "ADDR")?;
            let spec = a.need(2, "ROWS")?;
            let rows: Vec<Vec<String>> = if std::path::Path::new(spec).is_file() {
                read_csv(&std::fs::read_to_string(spec).map_err(|e| err(e.to_string()))?)
            } else {
                spec.split(';').map(|r| r.split('|').map(|c| text::unescape(c.trim())).collect()).collect()
            };
            let t = d.insert_table(addr, !a.has(&["--before"]), &rows, a.has(&["--header"]))?;
            Ok(Outcome { lines: vec![format!("inserted table {t}")], json: json!({"table": t}), changed: true })
        }
        _ => Err(err(format!("unknown command {cmd:?} (docu help)"))),
    }
}

/// Sorts addresses so that later ones (in document order) come last: story, then numbers.
fn sort_key(a: &str) -> (String, Vec<usize>) {
    let mut story = String::new();
    let mut nums = Vec::new();
    for part in a.split('.') {
        let digits: String = part.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
        let head: String = part.chars().take_while(|c| !c.is_ascii_digit()).collect();
        if matches!(head.as_str(), "hdr" | "ftr" | "fn" | "en" | "cmt") && nums.is_empty() {
            story = part.to_string();
            continue;
        }
        nums.push(digits.parse().unwrap_or(0));
    }
    (story, nums)
}

fn read_csv(s: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match (c, quoted) {
            ('"', true) if it.peek() == Some(&'"') => {
                cell.push('"');
                it.next();
            }
            ('"', true) => quoted = false,
            ('"', false) if cell.is_empty() => quoted = true,
            (',', false) => row.push(std::mem::take(&mut cell)),
            ('\r', false) => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            (c, _) => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

/// Splits a batch line like a shell: quotes ('…', "…"), backslash escapes outside single
/// quotes (`\"`, `\\`; `\n` and `\t` are kept for the commands to read).
fn split_words(line: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match it.next() {
                        Some('\'') => break,
                        Some(c) => cur.push(c),
                        None => return Err(err("unterminated '")),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match it.next() {
                        Some('"') => break,
                        Some('\\') => match it.next() {
                            Some(c @ ('"' | '\\' | '$' | '`')) => cur.push(c),
                            Some(c) => {
                                cur.push('\\');
                                cur.push(c);
                            }
                            None => return Err(err("unterminated \"")),
                        },
                        Some(c) => cur.push(c),
                        None => return Err(err("unterminated \"")),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match it.next() {
                    Some(c @ (' ' | '"' | '\'' | '\\')) => cur.push(c),
                    Some(c) => {
                        cur.push('\\');
                        cur.push(c);
                    }
                    None => {}
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    Ok(out)
}
