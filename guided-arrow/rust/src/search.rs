//! Finding examples by what they do. There is no index to carry: the blocks of the languages
//! asked for are decompressed (fast) and every example — and every page's summary — is scored
//! against the words, BM25-style, with a word matching any word that starts with it.

use crate::archive::{Archive, KIND_SEE_ALSO};
use crate::page::{self, Sheet};
use crate::Result;
use std::collections::HashMap;

const STOP: &[&str] = &[
    "a", "an", "the", "to", "of", "in", "on", "for", "with", "and", "or", "how", "do", "does", "i", "is", "my", "me",
    "by", "into", "it", "its", "this", "that", "can", "you", "your", "using", "via", "from", "what", "which", "all",
];

/// The words of a text, lower case: runs of letters and digits (any script).
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            cur.extend(c.to_lowercase());
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The words of a description: tldr marks the letter an option stands for ("E[x]tract", "[c]reate"),
/// which must not split the word.
pub fn desc_tokens(text: &str) -> Vec<String> {
    tokens(&text.replace(['[', ']'], ""))
}

/// A word of a query as it is matched: lower case, a common English ending taken off.
pub fn query_terms(words: &[String]) -> Vec<String> {
    let all: Vec<String> = words.iter().flat_map(|w| tokens(w)).collect();
    let mut out: Vec<String> = Vec::new();
    for t in &all {
        if STOP.contains(&t.as_str()) && all.len() > 1 {
            continue;
        }
        let t = stem(t);
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// The start every form of an English word shares, about: files → file, boxes → box,
/// copies → copy, listing → list, modified → modifi.
fn stem(t: &str) -> String {
    if !t.is_ascii() || t.len() <= 4 {
        return t.to_string();
    }
    if let Some(s) = t.strip_suffix("ies") {
        return format!("{s}y");
    }
    if let Some(s) = t.strip_suffix("es") {
        if s.ends_with(['s', 'x', 'z']) || s.ends_with("ch") || s.ends_with("sh") {
            return s.to_string();
        }
    }
    for suf in ["ing", "ed"] {
        if let Some(s) = t.strip_suffix(suf) {
            if s.len() >= 3 {
                return s.to_string();
            }
        }
    }
    match t.strip_suffix('s') {
        Some(s) if !s.ends_with('s') => s.to_string(),
        _ => t.to_string(),
    }
}

#[inline]
fn matches(term: &str, tok: &str) -> bool {
    tok.starts_with(term)
}

/// How many of `terms` an example mentions (in its description, command or page name).
pub fn coverage(terms: &[String], texts: &[&str]) -> usize {
    let toks: Vec<String> = texts.iter().flat_map(|t| desc_tokens(t)).collect();
    terms.iter().filter(|t| toks.iter().any(|k| matches(t, k))).count()
}

pub struct Hit {
    pub page: usize,
    /// None: the page's summary (the tool itself fits).
    pub example: Option<usize>,
    pub score: f64,
}

pub struct Found {
    pub hits: Vec<Hit>,
    pub sheets: HashMap<usize, Sheet>,
}

const MAX_TERMS: usize = 8;

/// One description or command's term weights, while scoring.
#[derive(Clone, Copy)]
struct Tf {
    w: [f32; MAX_TERMS],
    len: u32,
}

impl Tf {
    fn new() -> Tf {
        Tf { w: [0.0; MAX_TERMS], len: 0 }
    }

    /// Adds the words of `text`: `weight` for a whole word, 0.7 of it for a word a term begins.
    fn add(&mut self, terms: &[String], text: &str, weight: f32, brackets: bool) {
        let mut tok = String::new();
        let mut chars = text.chars().peekable();
        loop {
            let c = chars.next();
            match c {
                Some(c) if c.is_alphanumeric() => tok.extend(c.to_lowercase()),
                Some('[' | ']') if brackets => {}
                _ => {
                    if !tok.is_empty() {
                        self.len += 1;
                        for (k, t) in terms.iter().enumerate() {
                            if tok.starts_with(t.as_str()) {
                                self.w[k] += weight * if tok.len() == t.len() { 1.0 } else { 0.7 };
                            }
                        }
                        tok.clear();
                    }
                    if c.is_none() {
                        break;
                    }
                }
            }
        }
    }

    fn any(&self) -> bool {
        self.w.iter().any(|&w| w > 0.0)
    }
}

struct Cand {
    page: usize,
    example: Option<usize>,
    tf: Tf,
    /// To show one command once, from whichever page (platform, translation) scores best.
    code_hash: u64,
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Scores every example of the pages `wanted` admits, best first. Blocks are read one after
/// another; a page none of whose text holds a term is not even parsed.
pub fn search(ar: &Archive, words: &[String], wanted: &dyn Fn(usize) -> bool, limit: usize) -> Result<Found> {
    let mut terms = query_terms(words);
    terms.truncate(MAX_TERMS);
    if terms.is_empty() {
        return Ok(Found { hits: Vec::new(), sheets: HashMap::new() });
    }
    let pages = &ar.dir.pages;
    let mut cands: Vec<Cand> = Vec::new();
    let mut df = [0f64; MAX_TERMS];
    let (mut ndocs, mut total_len, mut skipped) = (0u64, 0u64, 0u64);
    let mut i = 0;
    while i < pages.len() {
        let b = pages[i].block;
        let mut j = i;
        while j < pages.len() && pages[j].block == b {
            j += 1;
        }
        let todo: Vec<usize> = (i..j).filter(|&k| pages[k].kind != KIND_SEE_ALSO && wanted(k)).collect();
        i = j;
        if todo.is_empty() {
            continue;
        }
        let blk = ar.block(b)?;
        for k in todo {
            let p = &pages[k];
            let (a, z) = blk.spans[p.index as usize];
            let text = std::str::from_utf8(&blk.raw[a..z]).unwrap_or("");
            let low = text.to_lowercase().replace(['[', ']'], "");
            if !terms.iter().any(|t| low.contains(t.as_str())) {
                skipped += 1;
                continue;
            }
            let sheet = page::parse(p.kind, &p.name, text);
            let mut name = Tf::new();
            name.add(&terms, &p.name, 1.5, false);
            name.len = 0;
            let mut head = name;
            for l in sheet.summary.iter().chain(sheet.tags.iter()) {
                head.add(&terms, l, 1.0, true);
            }
            let mut docs = vec![(None, head, 0u64)];
            for (n, e) in sheet.examples.iter().enumerate() {
                let mut tf = name;
                tf.add(&terms, &e.desc, 1.0, true);
                tf.add(&terms, &e.code, 0.8, false);
                docs.push((Some(n), tf, hash(e.code.trim())));
            }
            for (example, tf, code_hash) in docs {
                ndocs += 1;
                total_len += tf.len as u64 + 2;
                if tf.any() {
                    for (t, d) in df.iter_mut().enumerate() {
                        if tf.w[t] > 0.0 {
                            *d += 1.0;
                        }
                    }
                    cands.push(Cand { page: k, example, tf, code_hash });
                }
            }
        }
    }
    // Pages skipped unread still count toward how common a word is (about 8 examples each).
    let n = (ndocs + skipped * 9) as f64;
    let avg = if ndocs > 0 { total_len as f64 / ndocs as f64 } else { 1.0 };
    let idf: Vec<f64> = df[..terms.len()].iter().map(|&f| ((n - f + 0.5) / (f + 0.5) + 1.0).ln()).collect();
    let (k1, b) = (1.2, 0.5);
    let mut hits: Vec<(f64, usize)> = cands
        .iter()
        .enumerate()
        .map(|(ci, c)| {
            let len = c.tf.len as f64 + 2.0;
            let mut score = 0.0;
            let mut covered = 0;
            for (t, &w) in c.tf.w[..terms.len()].iter().enumerate() {
                if w > 0.0 {
                    covered += 1;
                    let tf = w as f64;
                    score += idf[t] * tf * (k1 + 1.0) / (tf + k1 * (1.0 - b + b * len / avg));
                }
            }
            // Examples that mention every word first.
            let cov = covered as f64 / terms.len() as f64;
            score *= cov * cov;
            if c.example.is_none() {
                score *= 0.9;
            }
            (score, ci)
        })
        .collect();
    hits.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (score, ci) in hits {
        let c = &cands[ci];
        let key = match c.example {
            Some(_) => c.code_hash,
            None => hash(&format!("\0{}", pages[c.page].name)),
        };
        if seen.insert(key) {
            out.push(Hit { page: c.page, example: c.example, score });
            if out.len() >= limit {
                break;
            }
        }
    }
    let mut sheets = HashMap::new();
    for h in &out {
        if !sheets.contains_key(&h.page) {
            let p = &pages[h.page];
            sheets.insert(h.page, page::parse(p.kind, &p.name, &ar.text(p)?));
        }
    }
    Ok(Found { hits: out, sheets })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        assert_eq!(tokens("E[x]tract a .tar.gz file, 파일을 압축"), vec!["e", "x", "tract", "a", "tar", "gz", "file", "파일을", "압축"]);
        assert_eq!(desc_tokens("E[x]tract a [f]ile"), vec!["extract", "a", "file"]);
        assert_eq!(query_terms(&["how to extract files".into()]), vec!["extract", "file"]);
        assert_eq!(query_terms(&["copies".into(), "the".into()]), vec!["copy"]);
        assert_eq!(query_terms(&["boxes".into(), "matches".into(), "listing".into(), "class".into()]), vec!["box", "match", "list", "class"]);
        assert_eq!(query_terms(&["the".into()]), vec!["the"]);
        assert_eq!(coverage(&query_terms(&["extract".into(), "gz".into()]), &["Extract an archive", "tar xzf a.tar.gz"]), 2);
    }
}
