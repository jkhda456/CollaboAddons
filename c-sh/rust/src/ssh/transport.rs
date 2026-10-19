//! The SSH transport layer over a byte stream: the version exchange, key exchange (driving
//! [super::kex]), the encrypted packet stream (through [super::packet]), host-key verification,
//! and rekeying. Above it sits [super::auth] and [super::channel].

use std::io::{BufReader, Read, Write};

use super::hostkey::PublicKey;
use super::packet::{self, Aead, Protection};
use super::wire::{Reader, Writer};
use super::{kex, msg};
use crate::crypto::{self, Cipher, Hmac};
use crate::{err, Result};

/// Decides whether to trust the server's host key (checked against known_hosts by the caller).
pub type HostKeyVerifier<'a> = Box<dyn FnMut(&PublicKey) -> Result<()> + Send + 'a>;

/// What c-sh calls itself in the version string.
pub const VERSION: &str = "SSH-2.0-collabo-csh_0.1";

pub struct Transport<'a> {
    reader: BufReader<Box<dyn Read + Send>>,
    writer: Box<dyn Write + Send>,
    send: Protection,
    recv: Protection,
    send_seq: u32,
    recv_seq: u32,
    client_version: String,
    server_version: String,
    session_id: Vec<u8>,
    verify: HostKeyVerifier<'a>,
    /// From the server's EXT_INFO: the signature algorithms it accepts for publickey auth.
    pub server_sig_algs: Vec<String>,
    /// The negotiated host-key algorithm (what the server signed the exchange with).
    pub host_key_alg: String,
    strict_kex: bool,
    first_kex: bool,
    bytes_since_kex: u64,
}

impl<'a> Transport<'a> {
    /// Connects: exchanges versions and runs the first key exchange, leaving the link encrypted
    /// and ready for authentication.
    pub fn start(read: Box<dyn Read + Send>, write: Box<dyn Write + Send>, verify: HostKeyVerifier<'a>) -> Result<Transport<'a>> {
        let mut t = Transport {
            reader: BufReader::new(read),
            writer: write,
            send: Protection::Plain,
            recv: Protection::Plain,
            send_seq: 0,
            recv_seq: 0,
            client_version: VERSION.to_string(),
            server_version: String::new(),
            session_id: Vec::new(),
            verify,
            server_sig_algs: Vec::new(),
            host_key_alg: String::new(),
            strict_kex: false,
            first_kex: true,
            bytes_since_kex: 0,
        };
        t.exchange_versions()?;
        t.key_exchange()?;
        Ok(t)
    }

    /// Bytes already read from the socket but not yet taken as packets (poll cannot see them).
    pub fn has_buffered(&self) -> bool {
        !self.reader.buffer().is_empty()
    }

    pub fn session_id(&self) -> &[u8] {
        &self.session_id
    }

    /// Lines up to and including the server's "SSH-2.0-..." banner, then sends ours.
    fn exchange_versions(&mut self) -> Result<()> {
        self.writer.write_all(format!("{}\r\n", self.client_version).as_bytes())?;
        self.writer.flush()?;
        // The server may send lines before its identification; skip them (RFC 4253 §4.2).
        for _ in 0..1000 {
            let mut line = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                self.reader.read_exact(&mut byte)?;
                if byte[0] == b'\n' {
                    break;
                }
                if byte[0] != b'\r' {
                    line.push(byte[0]);
                }
                if line.len() > 4096 {
                    return Err(err!("the server's identification line is too long"));
                }
            }
            if line.starts_with(b"SSH-") {
                self.server_version = String::from_utf8_lossy(&line).into_owned();
                if !self.server_version.starts_with("SSH-2.0-") && !self.server_version.starts_with("SSH-1.99-") {
                    return Err(err!("the server speaks {}, not SSH-2", self.server_version));
                }
                return Ok(());
            }
        }
        Err(err!("the server sent no SSH identification"))
    }

    /// One key exchange (the first, or a rekey): negotiate, agree a shared secret, verify the
    /// host key, and switch both directions to fresh keys.
    fn key_exchange(&mut self) -> Result<()> {
        let mut cookie = [0u8; 16];
        crypto::random(&mut cookie);
        let our_kexinit = {
            let mut p = vec![msg::KEXINIT];
            p.extend_from_slice(&kex::build_kexinit(&cookie, self.first_kex));
            p
        };
        self.write_packet(&our_kexinit)?;
        let their_kexinit = self.read_raw_expect(msg::KEXINIT)?;
        let neg = kex::negotiate(&our_kexinit, &their_kexinit[1..])?;
        if self.first_kex {
            self.strict_kex = neg.strict_kex;
        }
        self.host_key_alg = neg.host_key.clone();

        let method = kex::Method::start(&neg.kex)?;
        let client_pub = method.client_public().to_vec();
        let mpint = method.is_mpint();

        // curve25519/ECDH and fixed-group DH share message numbers 30/31.
        let mut init = Writer::msg(msg::KEX_ECDH_INIT);
        if mpint {
            init.mpint(&client_pub);
        } else {
            init.string(&client_pub);
        }
        self.write_packet(&init.finish())?;

        let reply = self.read_raw_expect(msg::KEX_ECDH_REPLY)?;
        let mut r = Reader::new(&reply[1..]);
        let host_key_blob = r.string()?.to_vec();
        let server_pub = if mpint { r.mpint()?.to_vec() } else { r.string()?.to_vec() };
        let signature = r.string()?.to_vec();

        let shared = method.shared(&server_pub)?;
        let k_mpint = super::wire::mpint(&shared);

        let hash = kex::kex_hash(&neg.kex);
        let exchange_hash = {
            let mut w = Writer::new();
            w.string(&self.client_version);
            w.string(&self.server_version);
            w.string(&our_kexinit);
            w.string(&their_kexinit);
            w.string(&host_key_blob);
            if mpint {
                w.mpint(&client_pub).mpint(&server_pub);
            } else {
                w.string(&client_pub).string(&server_pub);
            }
            w.raw(&k_mpint);
            crypto::hash(hash, &w.finish())
        };

        // Verify the server owns the host key, then that we trust the key.
        let host_key = PublicKey::parse(&host_key_blob)?;
        host_key.verify(&signature, &exchange_hash, Some(&neg.host_key))?;
        (self.verify)(&host_key)?;

        if self.session_id.is_empty() {
            self.session_id = exchange_hash.clone();
        }

        // NEWKEYS is the last cleartext packet. The send direction switches to the new keys once
        // we have sent ours; the receive direction only after the server's NEWKEYS, which is still
        // under the old keys — so read it before installing the receive side.
        let (send, recv) = self.derive_protections(&neg, hash, &k_mpint, &exchange_hash)?;
        self.write_packet(&[msg::NEWKEYS])?;
        self.send = send;
        if self.strict_kex {
            self.send_seq = 0;
        }
        self.read_raw_expect(msg::NEWKEYS)?;
        self.recv = recv;
        if self.strict_kex {
            self.recv_seq = 0;
        }
        self.first_kex = false;
        self.bytes_since_kex = 0;
        Ok(())
    }

    /// Derives the send and receive [Protection]s from the shared secret and exchange hash.
    fn derive_protections(&self, neg: &kex::Negotiated, hash: crypto::Hash, k_mpint: &[u8], h: &[u8]) -> Result<(Protection, Protection)> {
        let sid = &self.session_id;
        let mk = |letter: u8, need: usize| kex::derive_key(hash, k_mpint, h, letter, sid, need);
        // Client->server uses A,C,E; server->client B,D,F.
        let send = make_protection(neg.cipher_c2s, neg.mac_c2s, &mk(b'A', neg.cipher_c2s.iv_len), &mk(b'C', neg.cipher_c2s.key_len), &mk(b'E', neg.mac_c2s.hash.len()), true)?;
        let recv = make_protection(neg.cipher_s2c, neg.mac_s2c, &mk(b'B', neg.cipher_s2c.iv_len), &mk(b'D', neg.cipher_s2c.key_len), &mk(b'F', neg.mac_s2c.hash.len()), false)?;
        Ok((send, recv))
    }

    /// Writes a packet (raw, before encryption is handled by [packet::write]).
    fn write_packet(&mut self, payload: &[u8]) -> Result<()> {
        packet::write(&mut self.writer, &mut self.send, &mut self.send_seq, payload)
    }

    /// The agent-facing send: a normal transport message.
    pub fn send_packet(&mut self, payload: &[u8]) -> Result<()> {
        self.bytes_since_kex += payload.len() as u64 + 64;
        self.write_packet(payload)?;
        // Rekey after a gigabyte, as OpenSSH does.
        if self.bytes_since_kex > 1 << 30 {
            self.key_exchange()?;
        }
        Ok(())
    }

    /// Reads the next application packet, dealing with transport housekeeping (ignore, debug,
    /// rekey requests, ext-info) transparently.
    pub fn recv_packet(&mut self) -> Result<Vec<u8>> {
        loop {
            let p = packet::read(&mut self.reader, &mut self.recv, &mut self.recv_seq)?;
            match p.first().copied() {
                Some(msg::IGNORE) | Some(msg::DEBUG) | Some(msg::UNIMPLEMENTED) => continue,
                Some(msg::DISCONNECT) => {
                    let mut r = Reader::new(&p[1..]);
                    let _code = r.u32();
                    let reason = r.text().unwrap_or_default();
                    return Err(err!("the server closed the connection: {reason}"));
                }
                Some(msg::KEXINIT) => {
                    // The server wants to rekey: it has sent KEXINIT; run one (ours first).
                    self.rekey_from(p)?;
                    continue;
                }
                Some(msg::EXT_INFO) => {
                    self.read_ext_info(&p);
                    continue;
                }
                Some(msg::GLOBAL_REQUEST) => {
                    // Answer "want reply" global requests with a failure; ignore the rest.
                    let mut r = Reader::new(&p[1..]);
                    let _name = r.string();
                    if r.bool().unwrap_or(false) {
                        self.send_packet(&[msg::REQUEST_FAILURE])?;
                    }
                    continue;
                }
                _ => return Ok(p),
            }
        }
    }

    /// A rekey the server began: it already sent `their_kexinit`, so finish the exchange.
    fn rekey_from(&mut self, their_kexinit: Vec<u8>) -> Result<()> {
        let mut cookie = [0u8; 16];
        crypto::random(&mut cookie);
        let our_kexinit = {
            let mut p = vec![msg::KEXINIT];
            p.extend_from_slice(&kex::build_kexinit(&cookie, false));
            p
        };
        self.write_packet(&our_kexinit)?;
        let neg = kex::negotiate(&our_kexinit, &their_kexinit[1..])?;
        self.host_key_alg = neg.host_key.clone();
        let method = kex::Method::start(&neg.kex)?;
        let client_pub = method.client_public().to_vec();
        let mpint = method.is_mpint();
        let mut init = Writer::msg(msg::KEX_ECDH_INIT);
        if mpint {
            init.mpint(&client_pub);
        } else {
            init.string(&client_pub);
        }
        self.write_packet(&init.finish())?;
        let reply = self.read_raw_expect(msg::KEX_ECDH_REPLY)?;
        let mut r = Reader::new(&reply[1..]);
        let host_key_blob = r.string()?.to_vec();
        let server_pub = if mpint { r.mpint()?.to_vec() } else { r.string()?.to_vec() };
        let signature = r.string()?.to_vec();
        let shared = method.shared(&server_pub)?;
        let k_mpint = super::wire::mpint(&shared);
        let hash = kex::kex_hash(&neg.kex);
        let mut w = Writer::new();
        w.string(&self.client_version).string(&self.server_version).string(&our_kexinit).string(&their_kexinit);
        w.string(&host_key_blob);
        if mpint {
            w.mpint(&client_pub).mpint(&server_pub);
        } else {
            w.string(&client_pub).string(&server_pub);
        }
        w.raw(&k_mpint);
        let exchange_hash = crypto::hash(hash, &w.finish());
        let host_key = PublicKey::parse(&host_key_blob)?;
        host_key.verify(&signature, &exchange_hash, Some(&neg.host_key))?;
        (self.verify)(&host_key)?;
        let (send, recv) = self.derive_protections(&neg, hash, &k_mpint, &exchange_hash)?;
        self.write_packet(&[msg::NEWKEYS])?;
        self.send = send;
        if self.strict_kex {
            self.send_seq = 0;
        }
        self.read_raw_expect(msg::NEWKEYS)?;
        self.recv = recv;
        if self.strict_kex {
            self.recv_seq = 0;
        }
        self.bytes_since_kex = 0;
        Ok(())
    }

    fn read_ext_info(&mut self, p: &[u8]) {
        let mut r = Reader::new(&p[1..]);
        let Ok(count) = r.u32() else { return };
        for _ in 0..count {
            let (Ok(name), Ok(value)) = (r.text(), r.text()) else { return };
            if name == "server-sig-algs" {
                self.server_sig_algs = value.split(',').map(str::to_string).collect();
            }
        }
    }

    /// Reads a packet during key exchange, where only the expected type (and ignore/debug) may
    /// appear. KEXINIT is read raw (not through recv_packet, which would recurse into rekey).
    fn read_raw_expect(&mut self, want: u8) -> Result<Vec<u8>> {
        loop {
            let p = packet::read(&mut self.reader, &mut self.recv, &mut self.recv_seq)?;
            match p.first().copied() {
                Some(msg::IGNORE) | Some(msg::DEBUG) if !self.strict_kex => continue,
                Some(msg::DISCONNECT) => {
                    let mut r = Reader::new(&p[1..]);
                    let _ = r.u32();
                    return Err(err!("the server closed the connection: {}", r.text().unwrap_or_default()));
                }
                Some(t) if t == want => return Ok(p),
                Some(t) => return Err(err!("expected SSH message {want} during key exchange, got {t}")),
                None => return Err(err!("an empty packet during key exchange")),
            }
        }
    }
}

/// Assembles a [Protection] for one direction from its IV, key and MAC key.
fn make_protection(cipher: &packet::CipherSpec, mac: &packet::MacSpec, iv: &[u8], key: &[u8], mac_key: &[u8], encrypt: bool) -> Result<Protection> {
    match cipher.aead {
        Aead::ChaChaPoly => {
            // The 64-byte key is K_2 (payload) || K_1 (length); each 32 bytes.
            let main_key: [u8; 32] = key[..32].try_into().unwrap();
            let header_key: [u8; 32] = key[32..64].try_into().unwrap();
            Ok(Protection::ChaChaPoly { header_key, main_key })
        }
        Aead::Gcm => {
            let cipher_impl = Cipher::new(cipher.kind.unwrap(), key, iv, encrypt)?;
            Ok(Protection::Gcm { cipher: cipher_impl, iv: iv.try_into().unwrap() })
        }
        Aead::No => {
            let cipher_impl = Cipher::new(cipher.kind.unwrap(), key, iv, encrypt)?;
            Ok(Protection::Mac { cipher: cipher_impl, mac: Hmac::new(mac.hash, mac_key)?, etm: mac.etm, block: cipher.block })
        }
    }
}
