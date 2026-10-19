//! Reading private keys a user signs with: the OpenSSH format (`-----BEGIN OPENSSH PRIVATE
//! KEY-----`, parsed here, bcrypt-encrypted ones decrypted with [bcrypt]) and the older PEM
//! formats (PKCS#1, SEC1, PKCS#8, which OpenSSL reads). The key itself lives in [crate::crypto].

use super::hostkey::PublicKey;
use super::wire::{Reader, Writer};
use crate::crypto::{self, Cipher, CipherKind, Curve, PrivateKey, PublicParts};
use crate::util::unbase64;
use crate::{err, Result};

mod bcrypt;

/// A private key, with the public half SSH advertises and a way to sign.
pub struct Identity {
    key: PrivateKey,
    pub public: PublicKey,
    pub comment: String,
}

impl Identity {
    /// Signs `data` and wraps it as an SSH signature blob, using `algorithm` (which must be one
    /// the key supports, e.g. rsa-sha2-256 for an RSA key).
    pub fn sign(&self, data: &[u8], algorithm: &str) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        w.string(algorithm);
        match &self.public {
            PublicKey::Ed25519(_) => {
                w.string(self.key.sign(None, data)?);
            }
            PublicKey::Ecdsa(curve, _) => {
                let (r, s) = self.key.sign_ecdsa(*curve, data)?;
                let mut inner = Writer::new();
                inner.mpint(&r).mpint(&s);
                w.string(inner.finish());
            }
            PublicKey::Rsa { .. } => {
                let hash = match algorithm {
                    "rsa-sha2-512" => crypto::Hash::Sha512,
                    "rsa-sha2-256" => crypto::Hash::Sha256,
                    "ssh-rsa" => crypto::Hash::Sha1,
                    other => return Err(err!("an RSA key cannot sign as {other}")),
                };
                w.string(self.key.sign(Some(hash), data)?);
            }
        }
        Ok(w.finish())
    }

    fn from_private(key: PrivateKey, comment: String) -> Result<Identity> {
        let public = match key.public_parts()? {
            PublicParts::Ed25519(k) => PublicKey::Ed25519(k),
            PublicParts::Ecdsa(c, q) => PublicKey::Ecdsa(c, q),
            PublicParts::Rsa { e, n } => PublicKey::Rsa { e, n },
        };
        Ok(Identity { key, public, comment })
    }
}

/// Reads every key in `pem` (a file may hold one). `passphrase` decrypts an encrypted key.
pub fn load(pem: &[u8], passphrase: Option<&str>) -> Result<Identity> {
    let text = String::from_utf8_lossy(pem);
    if text.contains("-----BEGIN OPENSSH PRIVATE KEY-----") {
        load_openssh(&text, passphrase)
    } else if text.contains("-----BEGIN ") {
        // PKCS#1 / SEC1 / PKCS#8, which OpenSSL parses (and decrypts).
        let key = PrivateKey::from_pem(pem, passphrase)?;
        Identity::from_private(key, String::new())
    } else {
        Err(err!("not a private key (no PEM header)"))
    }
}

/// True if the key is encrypted and needs a passphrase (so the caller can ask for one).
pub fn is_encrypted(pem: &[u8]) -> bool {
    let text = String::from_utf8_lossy(pem);
    if let Some(body) = between(&text, "-----BEGIN OPENSSH PRIVATE KEY-----", "-----END OPENSSH PRIVATE KEY-----") {
        if let Some(raw) = unbase64(body) {
            let mut r = Reader::new(&raw);
            if r.bytes(15).map(|m| m == b"openssh-key-v1\0").unwrap_or(false) {
                return r.string().map(|c| c != b"none").unwrap_or(false);
            }
        }
        false
    } else {
        text.contains("ENCRYPTED") || text.contains("Proc-Type: 4,ENCRYPTED")
    }
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let s = text.find(start)? + start.len();
    let e = text[s..].find(end)? + s;
    Some(text[s..e].trim())
}

/// The OpenSSH private key format (PROTOCOL.key):
///   "openssh-key-v1\0", ciphername, kdfname, kdfoptions, count=1, public key, encrypted block.
fn load_openssh(text: &str, passphrase: Option<&str>) -> Result<Identity> {
    let body = between(text, "-----BEGIN OPENSSH PRIVATE KEY-----", "-----END OPENSSH PRIVATE KEY-----")
        .ok_or_else(|| err!("the OPENSSH PRIVATE KEY block is not closed"))?;
    let raw = unbase64(body).ok_or_else(|| err!("the key is not valid base64"))?;
    let mut r = Reader::new(&raw);
    if r.bytes(15)? != b"openssh-key-v1\0" {
        return Err(err!("not an openssh-key-v1 key"));
    }
    let cipher = r.text()?;
    let kdf = r.text()?;
    let kdf_options = r.string()?.to_vec();
    if r.u32()? != 1 {
        return Err(err!("the key file holds more than one key, which c-sh does not expect"));
    }
    let _public = r.string()?;
    let mut blob = r.string()?.to_vec();

    if cipher != "none" {
        let pass = passphrase.ok_or_else(|| err!("the key is encrypted; a passphrase is needed"))?;
        decrypt_openssh(&cipher, &kdf, &kdf_options, pass, &mut blob)?;
    }

    let mut b = Reader::new(&blob);
    let (c1, c2) = (b.u32()?, b.u32()?);
    if c1 != c2 {
        return Err(err!("the key did not decrypt (wrong passphrase?)"));
    }
    let key = read_private_blob(&mut b)?;
    let comment = b.text().unwrap_or_default();
    Identity::from_private(key, comment)
}

/// Decrypts the OpenSSH key's private block in place.
fn decrypt_openssh(cipher: &str, kdf: &str, options: &[u8], passphrase: &str, blob: &mut [u8]) -> Result<()> {
    if kdf != "bcrypt" {
        return Err(err!("the key uses the {kdf} KDF, which c-sh does not support"));
    }
    let mut o = Reader::new(options);
    let salt = o.string()?;
    let rounds = o.u32()?;

    let (kind, key_len, iv_len) = match cipher {
        "aes256-ctr" => (CipherKind::Aes256Ctr, 32, 16),
        "aes256-cbc" => (CipherKind::Aes256Cbc, 32, 16),
        "aes128-ctr" => (CipherKind::Aes128Ctr, 16, 16),
        "aes128-cbc" => (CipherKind::Aes128Cbc, 16, 16),
        "aes256-gcm@openssh.com" => (CipherKind::Aes256Gcm, 32, 12),
        other => return Err(err!("the key is encrypted with {other}, which c-sh does not support")),
    };
    let mut material = vec![0u8; key_len + iv_len];
    bcrypt::pbkdf(passphrase.as_bytes(), salt, &mut material, rounds as usize)?;
    let (key, iv) = material.split_at(key_len);

    if matches!(kind, CipherKind::Aes256Gcm) {
        // The last 16 bytes are the tag; GCM authenticates the rest.
        if blob.len() < 16 {
            return Err(err!("the encrypted key is too short"));
        }
        let split = blob.len() - 16;
        let mut c = Cipher::new(kind, key, iv, false)?;
        c.apply(&mut blob[..split])?;
        if !c.open_tag(&blob[split..]) {
            return Err(err!("the key did not decrypt (wrong passphrase?)"));
        }
    } else {
        Cipher::new(kind, key, iv, false)?.apply(blob)?;
    }
    Ok(())
}

/// One private key out of an OpenSSH private block (the key type, then its numbers).
fn read_private_blob(r: &mut Reader) -> Result<PrivateKey> {
    let kind = r.text()?;
    match kind.as_str() {
        "ssh-ed25519" => {
            let _public = r.string()?;
            let secret = r.string()?; // 64 bytes: seed(32) || public(32)
            if secret.len() != 64 {
                return Err(err!("an ed25519 private key of {} bytes", secret.len()));
            }
            PrivateKey::ed25519(&secret[..32])
        }
        k if k.starts_with("ecdsa-sha2-") => {
            let curve_name = r.text()?;
            let curve = Curve::from_ssh_name(&curve_name).ok_or_else(|| err!("ECDSA on {curve_name}"))?;
            let point = r.string()?;
            let scalar = r.mpint()?;
            PrivateKey::ecdsa(curve, point, scalar)
        }
        "ssh-rsa" => {
            let n = r.mpint()?;
            let e = r.mpint()?;
            let d = r.mpint()?;
            let iqmp = r.mpint()?;
            let p = r.mpint()?;
            let q = r.mpint()?;
            PrivateKey::rsa(n, e, d, p, q, iqmp)
        }
        other => Err(err!("a private key of type {other}, which c-sh does not know")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn keygen(dir: &std::path::Path, kind: &str, pass: &str, extra: &[&str]) -> Vec<u8> {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let path = dir.join(format!("k{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let mut cmd = Command::new("ssh-keygen");
        cmd.args(["-q", "-t", kind.split(' ').next().unwrap(), "-N", pass, "-C", "tester", "-f"]).arg(&path);
        if kind.contains(' ') {
            cmd.args(["-b", kind.split(' ').nth(1).unwrap()]);
        }
        cmd.args(extra);
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "ssh-keygen {kind}: {}", String::from_utf8_lossy(&out.stderr));
        std::fs::read(&path).unwrap()
    }

    #[test]
    fn reads_openssh_keys() {
        if Command::new("ssh-keygen").arg("-?").output().is_err() {
            eprintln!("no ssh-keygen; skipping");
            return;
        }
        let dir = std::env::temp_dir().join(format!("c-sh-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for kind in ["ed25519", "ecdsa", "ecdsa 384", "ecdsa 521", "rsa 2048"] {
            let pem = keygen(&dir, kind, "", &[]);
            assert!(!is_encrypted(&pem), "{kind} should be clear");
            let id = load(&pem, None).unwrap();
            assert_eq!(id.comment, "tester", "{kind}");
            // Signing round-trips against our own verification.
            let alg = id.public.algorithms()[0].clone();
            let sig = id.sign(b"hello ssh", &alg).unwrap();
            id.public.verify(&sig, b"hello ssh", Some(&alg)).unwrap_or_else(|e| panic!("{kind}: {e}"));
        }
        // An encrypted key (bcrypt) needs the passphrase.
        let pem = keygen(&dir, "ed25519", "secret", &["-a", "8"]);
        assert!(is_encrypted(&pem));
        assert!(load(&pem, None).is_err());
        assert!(load(&pem, Some("wrong")).is_err());
        let id = load(&pem, Some("secret")).unwrap();
        let sig = id.sign(b"x", "ssh-ed25519").unwrap();
        id.public.verify(&sig, b"x", None).unwrap();
        // A classic PEM key (PKCS#8), via OpenSSL.
        let pem = keygen(&dir, "rsa 2048", "", &["-m", "PEM"]);
        load(&pem, None).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
