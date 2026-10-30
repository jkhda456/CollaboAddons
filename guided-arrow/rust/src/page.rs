//! Pages as text: reading each kind (tldr Markdown, cheat sheets, topic lists) into one shape —
//! a title, a summary, examples of a description and a command — and showing that shape.

use crate::archive::{KIND_LANG, KIND_SEE_ALSO, KIND_TLDR, KIND_TOPIC};
use crate::json::{self, Json};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Example {
    pub desc: String,
    pub code: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
    pub title: String,
    pub summary: Vec<String>,
    pub more: Option<String>,
    pub examples: Vec<Example>,
    pub syntax: String,
    pub tags: Vec<String>,
    pub see_also: Vec<String>,
    /// What starts a comment line in the sheet's language ("#" for commands).
    pub marker: String,
}

pub fn parse(kind: u8, name: &str, text: &str) -> Sheet {
    match kind {
        KIND_TLDR => parse_tldr(text),
        KIND_TOPIC => parse_topic(name, text),
        KIND_SEE_ALSO => Sheet {
            title: name.to_string(),
            marker: "#".into(),
            see_also: text.lines().map(str::trim).filter(|l| !l.is_empty() && *l != name).map(String::from).collect(),
            ..Default::default()
        },
        _ => parse_sheet(kind, name, text).0,
    }
}

/// A cheat sheet and where each of its examples is in `text`: from its first line to the next
/// example's (the blank lines after it included), so cutting a range out leaves the others as
/// they were written.
pub fn sheet_spans(kind: u8, name: &str, text: &str) -> (Sheet, Vec<(usize, usize)>) {
    parse_sheet(kind, name, text)
}

fn parse_tldr(text: &str) -> Sheet {
    let mut s = Sheet { syntax: "bash".into(), marker: "#".into(), ..Default::default() };
    for line in text.lines() {
        let line = line.trim_end();
        if let Some(t) = line.strip_prefix("# ") {
            if s.title.is_empty() {
                s.title = t.trim().to_string();
            }
        } else if let Some(d) = line.strip_prefix('>') {
            let d = d.trim();
            // "More information: <url>." (in every translation's own words)
            if let (Some(a), Some(b)) = (d.find("<http"), d.rfind('>')) {
                if a < b && s.more.is_none() {
                    s.more = Some(d[a + 1..b].to_string());
                    continue;
                }
            }
            s.summary.push(d.to_string());
        } else if let Some(d) = line.strip_prefix("- ") {
            s.examples.push(Example { desc: d.trim().to_string(), code: String::new() });
        } else if line.len() >= 2 && line.starts_with('`') && line.ends_with('`') {
            let code = &line[1..line.len() - 1];
            match s.examples.last_mut() {
                Some(e) if e.code.is_empty() => e.code = code.to_string(),
                Some(e) => {
                    e.code.push('\n');
                    e.code.push_str(code);
                }
                None => s.examples.push(Example { desc: String::new(), code: code.to_string() }),
            }
        }
    }
    s
}

fn parse_topic(name: &str, text: &str) -> Sheet {
    let mut s = Sheet { title: name.replace('-', " "), syntax: "bash".into(), marker: "#".into(), ..Default::default() };
    if let Ok(Json::Arr(items)) = json::parse(text) {
        for it in items {
            let field = |k: &str| it.get(k).and_then(Json::as_str).unwrap_or("").to_string();
            s.examples.push(Example { desc: field("description"), code: field("command") });
        }
    }
    s
}

/// The comment marker of a language's sheets.
fn comment_marker(lang: &str, text: &str) -> &'static str {
    match lang {
        "c" | "cpp" | "csharp" | "go" | "java" | "js" | "javascript" | "kotlin" | "rust" | "scala" | "swift" | "dart"
        | "php" | "solidity" | "typescript" | "ts" | "groovy" | "d" | "chapel" | "objective-c" | "zig" | "v" | "vala"
        | "fsharp" | "arduino" | "verilog" | "jq" | "protobuf" => "//",
        "sql" | "haskell" | "lua" | "elm" | "ada" | "vhdl" | "eiffel" | "idris" | "purescript" | "agda" | "psql"
        | "mysql" | "postgresql" | "sqlite" => "--",
        "lisp" | "clojure" | "elisp" | "scheme" | "racket" | "assembly" | "asm" | "clean" | "hy" | "fennel" => ";",
        "erlang" | "matlab" | "octave" | "prolog" | "latex" | "tex" | "mathematica" | "magma" => "%",
        "vim" | "vimscript" => "\"",
        "python" | "ruby" | "perl" | "bash" | "sh" | "zsh" | "fish" | "r" | "elixir" | "julia" | "nim" | "crystal"
        | "awk" | "sed" | "tcl" | "powershell" | "coffee" | "yaml" | "make" | "cmake" | "nix" | "raku" => "#",
        _ => {
            // Whichever marker starts the most lines.
            let count = |m: &str| text.lines().filter(|l| l.trim_start().starts_with(m)).count();
            ["#", "//", "--", ";", "%", "\""].into_iter().max_by_key(|m| count(m)).unwrap_or("#")
        }
    }
}

/// A sheet's YAML front matter (cheat/cheatsheets) and the text after it.
pub fn front_matter(text: &str) -> (&str, &str) {
    if let Some(rest) = text.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            return (&rest[..end], &rest[end + 5..]);
        }
        if let Some(head) = rest.strip_suffix("\n---") {
            return (head, "");
        }
    }
    ("", text)
}

fn parse_sheet(kind: u8, name: &str, text: &str) -> (Sheet, Vec<(usize, usize)>) {
    let mut s = Sheet::default();
    let (head, body) = front_matter(text);
    let base = text.len() - body.len();
    for line in head.lines() {
        if let Some(v) = line.strip_prefix("syntax:") {
            s.syntax = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("tags:") {
            let v = v.trim().trim_start_matches('[').trim_end_matches(']');
            s.tags = v.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
        }
    }
    let lang = if kind == KIND_LANG { name.split('/').next().unwrap_or("") } else { "" };
    let marker = if kind == KIND_LANG {
        comment_marker(lang, body)
    } else if s.syntax.is_empty() || s.syntax == "bash" || s.syntax == "sh" {
        "#"
    } else {
        comment_marker(&s.syntax, body)
    };
    if s.syntax.is_empty() {
        s.syntax = if kind == KIND_LANG { lang.to_string() } else { "bash".into() };
    }
    let is_comment = |l: &str| {
        let t = l.trim_start();
        t.starts_with(marker) && !(marker == "#" && (t.starts_with("#!") || t.starts_with("#include") || t.starts_with("#define")))
    };
    let strip = |l: &str| {
        let t = l.trim_start()[marker.len()..].trim_start_matches(marker.chars().next().unwrap_or('#'));
        t.strip_prefix(' ').unwrap_or(t).trim_end().to_string()
    };
    let mut cur = Example::default();
    // Where each example starts, and where the one being read did.
    let mut starts: Vec<usize> = Vec::new();
    let mut cur_start = 0;
    let mut flush = |cur: &mut Example, out: &mut Vec<Example>, at: usize| {
        if !cur.desc.is_empty() || !cur.code.is_empty() {
            out.push(std::mem::take(cur));
            starts.push(at);
        }
    };
    let push_line = |buf: &mut String, l: &str| {
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(l);
    };
    let mut at = base;
    for raw in body.split_inclusive('\n') {
        let line = raw.trim_end();
        let here = at;
        at += raw.len();
        if line.trim().is_empty() {
            flush(&mut cur, &mut s.examples, cur_start);
            continue;
        }
        if is_comment(line) && !cur.code.is_empty() {
            flush(&mut cur, &mut s.examples, cur_start);
        }
        if cur.desc.is_empty() && cur.code.is_empty() {
            cur_start = here;
        }
        if is_comment(line) {
            push_line(&mut cur.desc, &strip(line));
        } else {
            push_line(&mut cur.code, line);
        }
    }
    flush(&mut cur, &mut s.examples, cur_start);
    // cheat.sheets begins with "# name" and "# what it is".
    let short = name.rsplit('/').next().unwrap_or(name);
    if let Some(first) = s.examples.first() {
        let mut lines = first.desc.lines();
        if first.code.is_empty() && lines.next().is_some_and(|t| t.trim().eq_ignore_ascii_case(short) || t.trim().eq_ignore_ascii_case(name)) {
            s.summary = lines.map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
            s.examples.remove(0);
            starts.remove(0);
        }
    }
    s.title = name.to_string();
    s.marker = marker.to_string();
    let spans = (0..starts.len()).map(|i| (starts[i], starts.get(i + 1).copied().unwrap_or(text.len()))).collect();
    (s, spans)
}

// ---------------------------------------------------------------------------------------------
// tldr's placeholders: {{value}}, {{[-s|--long]}} option pairs, \{\{ escapes.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OptionForm {
    Long,
    Short,
    Both,
}

#[derive(Clone, Copy, Debug)]
pub struct Style {
    /// Keep `{{...}}` around values to fill in (for agents: it marks what to replace).
    pub braces: bool,
    pub color: bool,
    pub options: OptionForm,
}

pub const UNDERLINE: &str = "\x1b[4m";
pub const NO_UNDERLINE: &str = "\x1b[24m";

/// A tldr command as shown: placeholders resolved by `style`.
pub fn render_tldr_code(code: &str, style: Style) -> String {
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("\\{\\{") {
            out.push_str("{{");
            rest = r;
        } else if let Some(r) = rest.strip_prefix("\\}\\}") {
            out.push_str("}}");
            rest = r;
        } else if let Some(r) = rest.strip_prefix("{{") {
            // The outer braces mark the placeholder; braces inside it are its own.
            let mut depth = 0i32;
            let mut end = None;
            let b = r.as_bytes();
            let mut i = 0;
            while i < b.len() {
                match b[i] {
                    b'{' => depth += 1,
                    b'}' if depth > 0 => depth -= 1,
                    b'}' if i + 1 < b.len() && b[i + 1] == b'}' => {
                        end = Some(i);
                        break;
                    }
                    _ => {}
                }
                i += 1;
            }
            let Some(end) = end else {
                out.push_str(rest);
                break;
            };
            // Trailing braces of the value itself: `{{stash@{0}}}` ends at the last "}}".
            let mut end = end;
            while r[end + 2..].starts_with('}') {
                end += 1;
            }
            let inner = &r[..end];
            rest = &r[end + 2..];
            if let Some(opt) = inner.strip_prefix('[').and_then(|v| v.strip_suffix(']')).filter(|v| v.contains('|')) {
                let (short, long) = opt.split_once('|').unwrap();
                match style.options {
                    OptionForm::Long => out.push_str(long),
                    OptionForm::Short => out.push_str(short),
                    OptionForm::Both => out.push_str(inner),
                }
            } else if style.color {
                out.push_str(UNDERLINE);
                out.push_str(inner);
                out.push_str(NO_UNDERLINE);
            } else if style.braces {
                out.push_str("{{");
                out.push_str(inner);
                out.push_str("}}");
            } else {
                out.push_str(inner);
            }
        } else {
            let ch = rest.chars().next().unwrap();
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEEP: Style = Style { braces: true, color: false, options: OptionForm::Long };
    const PLAIN: Style = Style { braces: false, color: false, options: OptionForm::Long };

    #[test]
    fn placeholders_by_the_client_spec() {
        assert_eq!(render_tldr_code("ping {{example.com}}", PLAIN), "ping example.com");
        assert_eq!(render_tldr_code("ping {{example.com}}", KEEP), "ping {{example.com}}");
        assert_eq!(
            render_tldr_code("docker inspect --format '\\{\\{range.NetworkSettings.Networks\\}\\}\\{\\{.IPAddress\\}\\}\\{\\{end\\}\\}' {{container}}", PLAIN),
            "docker inspect --format '{{range.NetworkSettings.Networks}}{{.IPAddress}}{{end}}' container"
        );
        assert_eq!(render_tldr_code("mount \\\\{{computer_name}}\\{{share_name}} Z:", PLAIN), "mount \\\\computer_name\\share_name Z:");
        assert_eq!(render_tldr_code("git stash show --patch {{stash@{0}}}", PLAIN), "git stash show --patch stash@{0}");
        assert_eq!(render_tldr_code("git add {{[-A|--all]}}", PLAIN), "git add --all");
        assert_eq!(render_tldr_code("git add {{[-A|--all]}}", Style { options: OptionForm::Short, ..PLAIN }), "git add -A");
        assert_eq!(render_tldr_code("git add {{[-A|--all]}}", Style { options: OptionForm::Both, ..PLAIN }), "git add [-A|--all]");
        assert_eq!(render_tldr_code("a {{b", PLAIN), "a {{b");
        assert_eq!(render_tldr_code("ünï {{cödé}}", KEEP), "ünï {{cödé}}");
    }

    #[test]
    fn tldr_page() {
        let s = parse(KIND_TLDR, "tar", "# tar\n\n> Archiving utility.\n> More information: <https://www.gnu.org/software/tar>.\n\n- [c]reate an archive:\n\n`tar cf {{target.tar}} {{file}}`\n\n- List:\n\n`tar tvf {{source.tar}}`\n");
        assert_eq!(s.title, "tar");
        assert_eq!(s.summary, vec!["Archiving utility."]);
        assert_eq!(s.more.as_deref(), Some("https://www.gnu.org/software/tar"));
        assert_eq!(s.examples.len(), 2);
        assert_eq!(s.examples[1], Example { desc: "List:".into(), code: "tar tvf {{source.tar}}".into() });
        let ko = parse(KIND_TLDR, "tar", "# tar\n\n> 아카이브 유틸리티.\n> 더 많은 정보: <https://www.gnu.org/software/tar>.\n");
        assert_eq!(ko.summary, vec!["아카이브 유틸리티."]);
        assert!(ko.more.is_some());
    }

    #[test]
    fn cheat_sheets() {
        let s = parse(1, "tar", "# tar\n# GNU version of tar\n\n# Extract\n# (verbose)\ntar -xvf a.tar\n\n# Create\ntar -cvf a.tar dir/\n# List\ntar -tvf a.tar\n");
        assert_eq!(s.summary, vec!["GNU version of tar"]);
        assert_eq!(s.examples.len(), 3);
        assert_eq!(s.examples[0], Example { desc: "Extract\n(verbose)".into(), code: "tar -xvf a.tar".into() });
        assert_eq!(s.examples[2].desc, "List");
        let c = parse(1, "git", "---\nsyntax: bash\ntags: [ vcs, development ]\n---\n# To stage all changes:\ngit add --all\n");
        assert_eq!(c.tags, vec!["vcs", "development"]);
        assert_eq!(c.examples, vec![Example { desc: "To stage all changes:".into(), code: "git add --all".into() }]);
        let go = parse(KIND_LANG, "go/hello", "// Hello world\npackage main\n\nfunc main() {}\n");
        assert_eq!(go.examples[0].desc, "Hello world");
        assert_eq!(go.examples[0].code, "package main");
        assert_eq!(go.syntax, "go");
    }

    #[test]
    fn example_spans() {
        let text = "---\nsyntax: bash\n---\n# tar\n# GNU tar\n\n# Extract\ntar -xvf a.tar\n\n\n# Create\ntar -cvf a.tar dir/\n# List\ntar -tvf a.tar";
        let (s, spans) = sheet_spans(1, "tar", text);
        assert_eq!(s.examples.len(), 3);
        let cut: Vec<&str> = spans.iter().map(|&(a, z)| &text[a..z]).collect();
        assert_eq!(cut, vec!["# Extract\ntar -xvf a.tar\n\n\n", "# Create\ntar -cvf a.tar dir/\n", "# List\ntar -tvf a.tar"]);
        // What comes before the first example (front matter, the "# name" header) is no example's.
        assert_eq!(&text[..spans[0].0], "---\nsyntax: bash\n---\n# tar\n# GNU tar\n\n");
    }

    #[test]
    fn topic_list() {
        let s = parse(KIND_TOPIC, "json-and-yaml", r#"[{"command": "jq", "description": "Process JSON."}, {"command": "yq", "description": "YAML."}]"#);
        assert_eq!(s.examples[0], Example { desc: "Process JSON.".into(), code: "jq".into() });
        assert_eq!(s.title, "json and yaml");
    }
}
