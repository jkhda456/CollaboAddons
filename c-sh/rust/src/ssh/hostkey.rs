//! Public keys as SSH carries them (RFC 4253 6.6, RFC 5656, RFC 8709, RFC 8332): parsing key
//! blobs, checking signatures, fingerprints, and the known_hosts file that remembers servers.

use std::path::Path;

use super::wire::{Reader, Writer};
use crate::crypto::{self, Curve, Hash};
use crate::util::{base64, unbase64};
use crate::{err, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublicKey {
    Ed25519(Vec<u8>),
    Ecdsa(Curve, Vec<u8>),
    Rsa { e: Vec<u8>, n: Vec<u8> },
}

impl PublicKey {
    pub fn parse(blob: &[u8]) -> Result<PublicKey> {
        let mut r = Reader::new(blob);
        let kind = r.text()?;
        match kind.as_str() {
            "ssh-ed25519" => {
                let k = r.string()?;
                if k.len() != 32 {
                    return Err(err!("an ssh-ed25519 key of {} bytes", k.len()));
                }
                Ok(PublicKey::Ed25519(k.to_vec()))
            }
            "ssh-rsa" => {
                let e = r.mpint()?.to_vec();
                let n = r.mpint()?.to_vec();
                if n.len() * 8 < 1024 {
                    return Err(err!("an RSA key of {} bits is too small", n.len() * 8));
                }
                Ok(PublicKey::Rsa { e, n })
            }
            k if k.starts_with("ecdsa-sha2-") => {
                let curve_name = r.text()?;
                let curve = Curve::from_ssh_name(&curve_name).ok_or_else(|| err!("an ECDSA key on {curve_name}"))?;
                if k != format!("ecdsa-sha2-{curve_name}") {
                    return Err(err!("an ECDSA key named {k} on {curve_name}"));
                }
                Ok(PublicKey::Ecdsa(curve, r.string()?.to_vec()))
            }
            other => Err(err!("a key of type {other}, which c-sh does not know")),
        }
    }

    /// The key type as SSH names it.
    pub fn kind(&self) -> String {
        match self {
            PublicKey::Ed25519(_) => "ssh-ed25519".into(),
            PublicKey::Ecdsa(c, _) => format!("ecdsa-sha2-{}", c.ssh_name()),
            PublicKey::Rsa { .. } => "ssh-rsa".into(),
        }
    }

    pub fn blob(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.string(self.kind());
        match self {
            PublicKey::Ed25519(k) => w.string(k),
            PublicKey::Ecdsa(c, q) => w.string(c.ssh_name()).string(q),
            PublicKey::Rsa { e, n } => w.mpint(e).mpint(n),
        };
        w.finish()
    }

    /// `SHA256:…`, as `ssh-keygen -l` shows it.
    pub fn fingerprint(&self) -> String {
        format!("SHA256:{}", base64(&crypto::hash(Hash::Sha256, &self.blob()), false))
    }

    /// The signature algorithms this key can be checked with, in our order of preference.
    pub fn algorithms(&self) -> Vec<String> {
        match self {
            PublicKey::Rsa { .. } => vec!["rsa-sha2-512".into(), "rsa-sha2-256".into(), "ssh-rsa".into()],
            other => vec![other.kind()],
        }
    }

    /// Checks `sig` (an SSH signature blob: algorithm, then the signature) over `data`.
    /// `algorithm` is the one negotiated (or None: any this key allows).
    pub fn verify(&self, sig: &[u8], data: &[u8], algorithm: Option<&str>) -> Result<()> {
        let mut r = Reader::new(sig);
        let alg = r.text()?;
        let raw = r.string()?;
        if let Some(want) = algorithm {
            if alg != want {
                return Err(err!("the server signed with {alg}, not {want}"));
            }
        }
        if !self.algorithms().contains(&alg) {
            return Err(err!("a {} key cannot sign as {alg}", self.kind()));
        }
        let ok = match self {
            PublicKey::Ed25519(k) => crypto::verify_ed25519(k, data, raw),
            PublicKey::Ecdsa(curve, q) => {
                let mut s = Reader::new(raw);
                let (rr, ss) = (s.mpint()?, s.mpint()?);
                crypto::verify_ecdsa(*curve, q, data, rr, ss)
            }
            PublicKey::Rsa { e, n } => {
                let hash = match alg.as_str() {
                    "rsa-sha2-512" => Hash::Sha512,
                    "rsa-sha2-256" => Hash::Sha256,
                    _ => Hash::Sha1,
                };
                // A signature shorter than the modulus has lost leading zeros; OpenSSL wants
                // exactly the modulus' length.
                let mut padded = vec![0u8; n.len().saturating_sub(raw.len())];
                padded.extend_from_slice(raw);
                crypto::verify_rsa(n, e, hash, data, &padded)
            }
        };
        if ok {
            Ok(())
        } else {
            Err(err!("the {} signature does not verify", alg))
        }
    }

    /// `TYPE BASE64`, a line of authorized_keys or known_hosts.
    pub fn openssh(&self) -> String {
        format!("{} {}", self.kind(), base64(&self.blob(), true))
    }
}

/// What known_hosts says about a server's key.
#[derive(Debug, PartialEq, Eq)]
pub enum Known {
    /// The key is there for this host.
    Match,
    /// This host has another key of the same type: it changed (or someone is in the middle).
    Changed { line: usize },
    /// Marked @revoked.
    Revoked,
    /// No key of this type for this host (`other_types`: keys of other types are known).
    Unknown { other_types: bool },
}

/// The name a server goes by in known_hosts: `host`, or `[host]:port` off port 22.
pub fn host_entry(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

/// Does one comma-separated pattern list of a known_hosts line name `name`?
fn patterns_match(patterns: &str, name: &str) -> bool {
    if let Some(hashed) = patterns.strip_prefix("|1|") {
        let mut parts = hashed.splitn(2, '|');
        let (Some(salt), Some(want)) = (parts.next(), parts.next()) else { return false };
        let (Some(salt), Some(want)) = (unbase64(salt), unbase64(want)) else { return false };
        return crypto::hmac(Hash::Sha1, &salt, name.as_bytes()).is_ok_and(|h| crypto::equal(&h, &want));
    }
    let mut matched = false;
    for p in patterns.split(',') {
        if let Some(neg) = p.strip_prefix('!') {
            if glob(neg, name) {
                return false;
            }
        } else if glob(p, name) {
            matched = true;
        }
    }
    matched
}

/// `*` and `?` wildcards, case-insensitive (host names).
pub fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Looks `key` up for `names` (the host as given, and its address) in a known_hosts file.
pub fn check_known(path: &Path, names: &[String], key: &PublicKey) -> Known {
    let Ok(text) = std::fs::read_to_string(path) else { return Known::Unknown { other_types: false } };
    let (mut matched, mut changed, mut other) = (false, None, false);
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let mut first = fields.next().unwrap_or("");
        let marker = if first.starts_with('@') {
            let m = first;
            first = fields.next().unwrap_or("");
            Some(m)
        } else {
            None
        };
        let (Some(kind), Some(b64)) = (fields.next(), fields.next()) else { continue };
        if marker == Some("@cert-authority") || !names.iter().any(|n| patterns_match(first, n)) {
            continue;
        }
        let Some(blob) = unbase64(b64) else { continue };
        let same = blob == key.blob();
        if marker == Some("@revoked") {
            if same {
                return Known::Revoked;
            }
            continue;
        }
        if same {
            matched = true;
        } else if kind == key.kind() {
            changed.get_or_insert(index + 1);
        } else {
            other = true;
        }
    }
    if matched {
        Known::Match
    } else if let Some(line) = changed {
        Known::Changed { line }
    } else {
        Known::Unknown { other_types: other }
    }
}

/// Remembers a server's key (appends `name TYPE BASE64`).
pub fn add_known(path: &Path, name: &str, key: &PublicKey) -> Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    let ends_in_newline = std::fs::read(path).map(|t| t.is_empty() || t.ends_with(b"\n")).unwrap_or(true);
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    if !ends_in_newline {
        f.write_all(b"\n")?;
    }
    writeln!(f, "{name} {}", key.openssh())?;
    Ok(())
}

/// Takes every key for `name` out of known_hosts (plain or hashed entries); returns how many.
pub fn forget_known(path: &Path, name: &str) -> Result<usize> {
    let Ok(text) = std::fs::read_to_string(path) else { return Ok(0) };
    let mut kept = String::new();
    let mut removed = 0;
    for line in text.lines() {
        let first = line.split_whitespace().next().unwrap_or("");
        let pattern = if first.starts_with('@') { line.split_whitespace().nth(1).unwrap_or("") } else { first };
        let only_this = pattern.starts_with("|1|") || pattern.split(',').all(|p| p.eq_ignore_ascii_case(name));
        if !line.trim_start().starts_with('#') && only_this && patterns_match(pattern, name) {
            removed += 1;
        } else {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    if removed > 0 {
        std::fs::write(path, kept)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob("*.example.com", "a.example.com"));
        assert!(!glob("*.example.com", "example.com"));
        assert!(glob("10.0.?.*", "10.0.1.200"));
        assert!(patterns_match("a,b,!c*", "b"));
        assert!(!patterns_match("*,!secret", "secret"));
    }

    #[test]
    fn known_hosts_file() {
        let dir = std::env::temp_dir().join(format!("c-sh-kh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("known_hosts");
        let key = PublicKey::Ed25519(vec![1; 32]);
        let other = PublicKey::Ed25519(vec![2; 32]);
        let names = vec!["[h]:2222".to_string()];
        assert_eq!(check_known(&path, &names, &key), Known::Unknown { other_types: false });
        add_known(&path, "[h]:2222", &key).unwrap();
        assert_eq!(check_known(&path, &names, &key), Known::Match);
        assert_eq!(check_known(&path, &names, &other), Known::Changed { line: 1 });
        assert_eq!(check_known(&path, &["h".into()], &key), Known::Unknown { other_types: false });
        // a hashed entry, as ssh-keygen -H writes them
        let salt = [7u8; 20];
        let h = crypto::hmac(Hash::Sha1, &salt, b"hashed.host").unwrap();
        let line = format!("|1|{}|{} {}\n", base64(&salt, true), base64(&h, true), other.openssh());
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(&line);
        std::fs::write(&path, text).unwrap();
        assert_eq!(check_known(&path, &["hashed.host".into()], &other), Known::Match);
        assert_eq!(forget_known(&path, "hashed.host").unwrap(), 1);
        assert_eq!(forget_known(&path, "[h]:2222").unwrap(), 1);
        assert_eq!(check_known(&path, &names, &key), Known::Unknown { other_types: false });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_blob_round_trip() {
        let k = PublicKey::Rsa { e: vec![1, 0, 1], n: vec![0xc5; 256] };
        assert_eq!(PublicKey::parse(&k.blob()).unwrap(), k);
        let k = PublicKey::Ecdsa(Curve::P256, vec![4; 65]);
        assert_eq!(PublicKey::parse(&k.blob()).unwrap(), k);
        assert!(k.fingerprint().starts_with("SHA256:"));
    }
}
