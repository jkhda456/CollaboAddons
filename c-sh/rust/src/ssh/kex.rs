//! Algorithm negotiation (KEXINIT, RFC 4253 §7.1), the key exchange methods (curve25519, ECDH,
//! finite-field DH and group exchange), the exchange hash, and deriving the session keys. The
//! packets are sent by [super::transport]; the math is [crate::crypto].

use super::packet::{self, Aead};
use super::wire::{Reader, Writer};
use crate::crypto::{self, Curve, Dh, DhGroup, Ephemeral, Hash};
use crate::{err, Result};

/// The kex algorithms c-sh offers, best first.
pub const KEX_ALGS: &[&str] = &[
    "curve25519-sha256",
    "curve25519-sha256@libssh.org",
    "ecdh-sha2-nistp256",
    "ecdh-sha2-nistp384",
    "ecdh-sha2-nistp521",
    "diffie-hellman-group16-sha512",
    "diffie-hellman-group18-sha512",
    "diffie-hellman-group14-sha256",
    "diffie-hellman-group14-sha1",
];

/// The host-key (signature) algorithms c-sh accepts, best first.
pub const HOST_KEY_ALGS: &[&str] =
    &["ssh-ed25519", "ecdsa-sha2-nistp256", "ecdsa-sha2-nistp384", "ecdsa-sha2-nistp521", "rsa-sha2-512", "rsa-sha2-256", "ssh-rsa"];

/// What the two sides agreed on.
pub struct Negotiated {
    pub kex: String,
    pub host_key: String,
    pub cipher_c2s: &'static packet::CipherSpec,
    pub cipher_s2c: &'static packet::CipherSpec,
    pub mac_c2s: &'static packet::MacSpec,
    pub mac_s2c: &'static packet::MacSpec,
    /// Both sides asked for strict key exchange (OpenSSH): no non-kex packets before NEWKEYS.
    pub strict_kex: bool,
}

/// Our KEXINIT payload (without the leading message code). `cookie` is 16 random bytes.
pub fn build_kexinit(cookie: &[u8; 16], first: bool) -> Vec<u8> {
    let cipher_names: Vec<&str> = packet::ciphers().iter().map(|c| c.name).collect();
    let mac_names: Vec<&str> = packet::macs().iter().map(|m| m.name).collect();
    let mut kex: Vec<&str> = KEX_ALGS.to_vec();
    // RFC 8308 / OpenSSL strict-kex markers, as a client sends them.
    if first {
        kex.push("ext-info-c");
        kex.push("kex-strict-c-v00@openssh.com");
    }
    let mut w = Writer::new();
    w.raw(cookie);
    w.name_list(&kex);
    w.name_list(HOST_KEY_ALGS);
    w.name_list(&cipher_names);
    w.name_list(&cipher_names);
    w.name_list(&mac_names);
    w.name_list(&mac_names);
    w.name_list(&["none"]);
    w.name_list(&["none"]);
    w.name_list(&[]);
    w.name_list(&[]);
    w.bool(false); // first_kex_packet_follows
    w.u32(0);
    w.finish()
}

/// Picks the algorithms: our order decides, among those the server also offers.
pub fn negotiate(client: &[u8], server: &[u8]) -> Result<Negotiated> {
    let mut r = Reader::new(server);
    r.bytes(16)?; // cookie
    let s_kex = r.name_list()?;
    let s_host = r.name_list()?;
    let s_c2s_cipher = r.name_list()?;
    let s_s2c_cipher = r.name_list()?;
    let s_c2s_mac = r.name_list()?;
    let s_s2c_mac = r.name_list()?;

    let _ = client; // our offered lists are the constants below
    let pick = |ours: &[&str], theirs: &[String], what: &str| -> Result<String> {
        ours.iter()
            .find(|a| theirs.iter().any(|t| t == **a))
            .map(|a| a.to_string())
            .ok_or_else(|| err!("no common {what} (server: {})", theirs.join(", ")))
    };
    let cipher_names: Vec<&str> = packet::ciphers().iter().map(|c| c.name).collect();
    let mac_names: Vec<&str> = packet::macs().iter().map(|m| m.name).collect();

    let kex = pick(KEX_ALGS, &s_kex, "key exchange")?;
    let host_key = pick(HOST_KEY_ALGS, &s_host, "host key algorithm")?;
    let c2s_cipher = pick(&cipher_names, &s_c2s_cipher, "client-to-server cipher")?;
    let s2c_cipher = pick(&cipher_names, &s_s2c_cipher, "server-to-client cipher")?;
    let cipher_c2s = packet::ciphers().iter().find(|c| c.name == c2s_cipher).unwrap();
    let cipher_s2c = packet::ciphers().iter().find(|c| c.name == s2c_cipher).unwrap();
    // AEAD ciphers carry their own integrity, so a MAC is only needed for the others.
    let mac_c2s = if cipher_c2s.aead == Aead::No {
        let name = pick(&mac_names, &s_c2s_mac, "client-to-server MAC")?;
        packet::macs().iter().find(|m| m.name == name).unwrap()
    } else {
        &packet::macs()[0]
    };
    let mac_s2c = if cipher_s2c.aead == Aead::No {
        let name = pick(&mac_names, &s_s2c_mac, "server-to-client MAC")?;
        packet::macs().iter().find(|m| m.name == name).unwrap()
    } else {
        &packet::macs()[0]
    };

    Ok(Negotiated {
        kex,
        host_key,
        cipher_c2s,
        cipher_s2c,
        mac_c2s,
        mac_s2c,
        strict_kex: s_kex.iter().any(|a| a == "kex-strict-s-v00@openssh.com"),
    })
}

/// The hash a kex algorithm computes its exchange hash with.
pub fn kex_hash(name: &str) -> Hash {
    if name.ends_with("sha256") || name == "curve25519-sha256@libssh.org" {
        Hash::Sha256
    } else if name.ends_with("sha512") {
        Hash::Sha512
    } else if name.ends_with("sha1") {
        Hash::Sha1
    } else if name.starts_with("ecdh-sha2-nistp384") {
        Hash::Sha384
    } else if name.starts_with("ecdh-sha2-nistp521") {
        Hash::Sha512
    } else {
        Hash::Sha256
    }
}

/// A key exchange in progress: it has produced the client's public value and will finish into the
/// shared secret once the server's is known.
pub enum Method {
    Curve(Ephemeral),
    Ecdh(Ephemeral),
    Dh(Dh),
}

impl Method {
    /// Starts the chosen method and gives the client's public value (`e`, Q_C).
    pub fn start(kex: &str) -> Result<Method> {
        Ok(match kex {
            "curve25519-sha256" | "curve25519-sha256@libssh.org" => Method::Curve(Ephemeral::x25519()?),
            "ecdh-sha2-nistp256" => Method::Ecdh(Ephemeral::ecdh(Curve::P256)?),
            "ecdh-sha2-nistp384" => Method::Ecdh(Ephemeral::ecdh(Curve::P384)?),
            "ecdh-sha2-nistp521" => Method::Ecdh(Ephemeral::ecdh(Curve::P521)?),
            "diffie-hellman-group14-sha1" | "diffie-hellman-group14-sha256" => dh(DhGroup::Modp2048, 256)?,
            "diffie-hellman-group16-sha512" => dh(DhGroup::Modp4096, 256)?,
            "diffie-hellman-group18-sha512" => dh(DhGroup::Modp8192, 384)?,
            other => return Err(err!("c-sh cannot do the {other} key exchange")),
        })
    }

    pub fn client_public(&self) -> &[u8] {
        match self {
            Method::Curve(e) | Method::Ecdh(e) => &e.public,
            Method::Dh(d) => &d.public,
        }
    }

    /// Finite-field DH sends its public values as `mpint`; curve25519 and ECDH as `string`.
    pub fn is_mpint(&self) -> bool {
        matches!(self, Method::Dh(_))
    }

    /// The shared secret from the server's public value.
    pub fn shared(&self, server_public: &[u8]) -> Result<Vec<u8>> {
        match self {
            Method::Curve(e) | Method::Ecdh(e) => e.agree(server_public),
            Method::Dh(d) => d.agree(server_public),
        }
    }
}

/// A fixed-group DH (group14/16/18); `bits` is the private exponent's size.
fn dh(group: DhGroup, bits: usize) -> Result<Method> {
    Ok(Method::Dh(Dh::new(&group.prime(), &[2], bits)?))
}

/// Derives a key of `need` bytes for `letter` (b'A'..=b'F'), RFC 4253 §7.2.
pub fn derive_key(hash: Hash, k_mpint: &[u8], h: &[u8], letter: u8, session_id: &[u8], need: usize) -> Vec<u8> {
    let mut key = {
        let mut m = crypto::Hasher::new(hash);
        m.update(k_mpint);
        m.update(h);
        m.update(&[letter]);
        m.update(session_id);
        m.finish()
    };
    while key.len() < need {
        let mut m = crypto::Hasher::new(hash);
        m.update(k_mpint);
        m.update(h);
        m.update(&key);
        key.extend_from_slice(&m.finish());
    }
    key.truncate(need);
    key
}
