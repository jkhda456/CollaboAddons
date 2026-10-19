//! The binary packet protocol (RFC 4253 §6): framing a payload with its padding, and the cipher
//! modes a modern server offers — AES-CTR/CBC with HMAC (encrypt-and-mac or encrypt-then-mac),
//! AES-GCM (RFC 5647), and chacha20-poly1305@openssh.com. One direction's state is a [Protection].

use std::io::{Read, Write};

use crate::crypto::{self, Cipher, CipherKind, Hash, Hmac, Poly1305};
use crate::{err, Result};

/// The largest packet we accept (OpenSSH's limit), to bound a hostile server.
const MAX_PACKET: usize = 256 * 1024;

/// How one direction is protected after key exchange. Before it, [Protection::Plain].
pub enum Protection {
    /// No encryption and no MAC: only the version and the first KEXINIT go in the clear.
    Plain,
    /// A stream/block cipher with a separate MAC.
    Mac { cipher: Cipher, mac: Hmac, etm: bool, block: usize },
    /// AES-GCM: the length is clear and authenticated, a counter in the nonce.
    Gcm { cipher: Cipher, iv: [u8; 12] },
    /// chacha20-poly1305@openssh.com: one key for the length, one for the payload and the tag.
    ChaChaPoly { header_key: [u8; 32], main_key: [u8; 32] },
}

impl Protection {
    /// The cipher's block size, which bounds the padding (8 at least).
    fn block(&self) -> usize {
        match self {
            Protection::Plain => 8,
            Protection::Mac { block, .. } => *block,
            Protection::Gcm { .. } => 16,
            Protection::ChaChaPoly { .. } => 8,
        }
    }
}

/// Builds the GCM nonce from the IV: the last 8 bytes count up per packet (RFC 5647).
fn gcm_advance(iv: &mut [u8; 12]) {
    let mut carry = 1u64;
    for b in iv[4..12].iter_mut().rev() {
        let v = *b as u64 + (carry & 0xff);
        *b = v as u8;
        carry = v >> 8;
    }
}

/// The chacha20 IV for sequence `seq` and block counter `ctr` (openssh's 64-bit nonce in the last
/// eight bytes, big-endian; the 32-bit little-endian block counter first).
fn chacha_iv(seq: u32, ctr: u32) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[0..4].copy_from_slice(&ctr.to_le_bytes());
    iv[8..16].copy_from_slice(&(seq as u64).to_be_bytes());
    iv
}

fn padding_for(len: usize, block: usize) -> usize {
    let block = block.max(8);
    let mut pad = block - (len % block);
    if pad < 4 {
        pad += block;
    }
    pad
}

/// Writes one packet carrying `payload` through `out`, advancing `seq`.
pub fn write(out: &mut impl Write, prot: &mut Protection, seq: &mut u32, payload: &[u8]) -> Result<()> {
    let block = prot.block();
    let frame = match prot {
        Protection::Plain => {
            let pad = padding_for(5 + payload.len(), block);
            let mut p = Vec::with_capacity(5 + payload.len() + pad);
            p.extend_from_slice(&((1 + payload.len() + pad) as u32).to_be_bytes());
            p.push(pad as u8);
            p.extend_from_slice(payload);
            p.extend_from_slice(&crypto::random_vec(pad));
            p
        }
        Protection::Mac { cipher, mac, etm, block } => {
            if *etm {
                let pad = padding_for(1 + payload.len(), *block);
                let mut body = Vec::with_capacity(1 + payload.len() + pad);
                body.push(pad as u8);
                body.extend_from_slice(payload);
                body.extend_from_slice(&crypto::random_vec(pad));
                cipher.apply(&mut body)?;
                let len = ((1 + payload.len() + pad) as u32).to_be_bytes();
                let tag = mac.compute(&[&seq.to_be_bytes(), &len, &body]);
                let mut frame = Vec::with_capacity(4 + body.len() + tag.len());
                frame.extend_from_slice(&len);
                frame.extend_from_slice(&body);
                frame.extend_from_slice(&tag);
                frame
            } else {
                let pad = padding_for(5 + payload.len(), *block);
                let mut plain = Vec::with_capacity(5 + payload.len() + pad);
                plain.extend_from_slice(&((1 + payload.len() + pad) as u32).to_be_bytes());
                plain.push(pad as u8);
                plain.extend_from_slice(payload);
                plain.extend_from_slice(&crypto::random_vec(pad));
                let tag = mac.compute(&[&seq.to_be_bytes(), &plain]);
                cipher.apply(&mut plain)?;
                plain.extend_from_slice(&tag);
                plain
            }
        }
        Protection::Gcm { cipher, iv, .. } => {
            let pad = padding_for(1 + payload.len(), 16);
            let len = ((1 + payload.len() + pad) as u32).to_be_bytes();
            let mut body = Vec::with_capacity(1 + payload.len() + pad);
            body.push(pad as u8);
            body.extend_from_slice(payload);
            body.extend_from_slice(&crypto::random_vec(pad));
            cipher.reset_iv(iv)?;
            cipher.aad(&len)?;
            cipher.apply(&mut body)?;
            let tag = cipher.seal_tag()?;
            gcm_advance(iv);
            let mut frame = Vec::with_capacity(4 + body.len() + 16);
            frame.extend_from_slice(&len);
            frame.extend_from_slice(&body);
            frame.extend_from_slice(&tag);
            frame
        }
        Protection::ChaChaPoly { header_key, main_key } => {
            let pad = padding_for(1 + payload.len(), 8);
            let len = ((1 + payload.len() + pad) as u32).to_be_bytes();
            let mut enc_len = len;
            let mut hc = Cipher::new(CipherKind::ChaCha20, header_key, &chacha_iv(*seq, 0), true)?;
            hc.apply(&mut enc_len)?;
            let mut body = Vec::with_capacity(1 + payload.len() + pad);
            body.push(pad as u8);
            body.extend_from_slice(payload);
            body.extend_from_slice(&crypto::random_vec(pad));
            let poly_key = poly_key(main_key, *seq)?;
            let mut mc = Cipher::new(CipherKind::ChaCha20, main_key, &chacha_iv(*seq, 1), true)?;
            mc.apply(&mut body)?;
            let mut authed = enc_len.to_vec();
            authed.extend_from_slice(&body);
            let tag = Poly1305::new()?.tag(&poly_key, &authed)?;
            let mut frame = enc_len.to_vec();
            frame.extend_from_slice(&body);
            frame.extend_from_slice(&tag);
            frame
        }
    };
    out.write_all(&frame)?;
    out.flush()?;
    *seq = seq.wrapping_add(1);
    Ok(())
}

/// The one-time Poly1305 key for a chacha20-poly1305 packet (first 32 bytes of the keystream).
fn poly_key(main_key: &[u8; 32], seq: u32) -> Result<[u8; 32]> {
    let mut block = [0u8; 32];
    Cipher::new(CipherKind::ChaCha20, main_key, &chacha_iv(seq, 0), true)?.apply(&mut block)?;
    Ok(block)
}

/// Reads one packet's payload from `inp`, advancing `seq`.
pub fn read(inp: &mut impl Read, prot: &mut Protection, seq: &mut u32) -> Result<Vec<u8>> {
    let payload = match prot {
        Protection::Plain => {
            let mut len = [0u8; 4];
            inp.read_exact(&mut len)?;
            let total = u32::from_be_bytes(len) as usize;
            check_len(total)?;
            let mut rest = vec![0u8; total];
            inp.read_exact(&mut rest)?;
            let pad = rest[0] as usize;
            if pad + 1 > rest.len() {
                return Err(err!("bad padding"));
            }
            rest[1..rest.len() - pad].to_vec()
        }
        Protection::Mac { cipher, mac, etm, block } => {
            if *etm {
                let mut len = [0u8; 4];
                inp.read_exact(&mut len)?;
                let total = u32::from_be_bytes(len) as usize;
                check_len(total)?;
                let mut body = vec![0u8; total];
                inp.read_exact(&mut body)?;
                let mut tag = vec![0u8; mac.len()];
                inp.read_exact(&mut tag)?;
                let want = mac.compute(&[&seq.to_be_bytes(), &len, &body]);
                if !crypto::equal(&want, &tag) {
                    return Err(err!("the server's message failed its MAC (corrupt or tampered)"));
                }
                cipher.apply(&mut body)?;
                strip_padding(&body)?
            } else {
                let mut first = vec![0u8; *block];
                inp.read_exact(&mut first)?;
                cipher.apply(&mut first)?;
                let total = u32::from_be_bytes(first[..4].try_into().unwrap()) as usize;
                check_len(total)?;
                let mut plain = first;
                let already = plain.len() - 4;
                if total > already {
                    let mut more = vec![0u8; total - already];
                    inp.read_exact(&mut more)?;
                    cipher.apply(&mut more)?;
                    plain.extend_from_slice(&more);
                }
                let mut tag = vec![0u8; mac.len()];
                inp.read_exact(&mut tag)?;
                let want = mac.compute(&[&seq.to_be_bytes(), &plain]);
                if !crypto::equal(&want, &tag) {
                    return Err(err!("the server's message failed its MAC (corrupt or tampered)"));
                }
                strip_padding(&plain[4..])?
            }
        }
        Protection::Gcm { cipher, iv, .. } => {
            let mut len = [0u8; 4];
            inp.read_exact(&mut len)?;
            let total = u32::from_be_bytes(len) as usize;
            check_len(total)?;
            let mut body = vec![0u8; total];
            inp.read_exact(&mut body)?;
            let mut tag = [0u8; 16];
            inp.read_exact(&mut tag)?;
            cipher.reset_iv(iv)?;
            cipher.aad(&len)?;
            cipher.apply(&mut body)?;
            if !cipher.open_tag(&tag) {
                return Err(err!("the server's message failed its GCM tag (corrupt or tampered)"));
            }
            gcm_advance(iv);
            strip_padding(&body)?
        }
        Protection::ChaChaPoly { header_key, main_key } => {
            let mut enc_len = [0u8; 4];
            inp.read_exact(&mut enc_len)?;
            let poly = poly_key(main_key, *seq)?;
            let mut len = enc_len;
            Cipher::new(CipherKind::ChaCha20, header_key, &chacha_iv(*seq, 0), false)?.apply(&mut len)?;
            let total = u32::from_be_bytes(len) as usize;
            check_len(total)?;
            let mut body = vec![0u8; total];
            inp.read_exact(&mut body)?;
            let mut tag = [0u8; 16];
            inp.read_exact(&mut tag)?;
            let mut authed = enc_len.to_vec();
            authed.extend_from_slice(&body);
            let want = Poly1305::new()?.tag(&poly, &authed)?;
            if !crypto::equal(&want, &tag) {
                return Err(err!("the server's message failed its Poly1305 tag (corrupt or tampered)"));
            }
            Cipher::new(CipherKind::ChaCha20, main_key, &chacha_iv(*seq, 1), false)?.apply(&mut body)?;
            strip_padding(&body)?
        }
    };
    *seq = seq.wrapping_add(1);
    Ok(payload)
}

fn check_len(total: usize) -> Result<()> {
    if total == 0 || total > MAX_PACKET {
        return Err(err!("the server announced a {total}-byte packet"));
    }
    Ok(())
}

/// Drops the padding byte and the padding from a decrypted `padding_length || payload || pad`.
fn strip_padding(body: &[u8]) -> Result<Vec<u8>> {
    let pad = *body.first().ok_or_else(|| err!("empty packet"))? as usize;
    if 1 + pad > body.len() {
        return Err(err!("bad padding ({pad} of {} bytes)", body.len()));
    }
    Ok(body[1..body.len() - pad].to_vec())
}

/// How long a cipher's key and IV are, and how to make a [Protection] from derived material. The
/// MAC is added by the caller (it depends on the negotiated MAC algorithm) for the non-AEAD ones.
pub struct CipherSpec {
    pub name: &'static str,
    pub key_len: usize,
    pub iv_len: usize,
    pub block: usize,
    pub kind: Option<CipherKind>,
    pub aead: Aead,
}

#[derive(PartialEq, Clone, Copy)]
pub enum Aead {
    No,
    Gcm,
    ChaChaPoly,
}

/// The ciphers c-sh offers, best first.
pub fn ciphers() -> &'static [CipherSpec] {
    &[
        CipherSpec { name: "chacha20-poly1305@openssh.com", key_len: 64, iv_len: 0, block: 8, kind: None, aead: Aead::ChaChaPoly },
        CipherSpec { name: "aes256-gcm@openssh.com", key_len: 32, iv_len: 12, block: 16, kind: Some(CipherKind::Aes256Gcm), aead: Aead::Gcm },
        CipherSpec { name: "aes128-gcm@openssh.com", key_len: 16, iv_len: 12, block: 16, kind: Some(CipherKind::Aes128Gcm), aead: Aead::Gcm },
        CipherSpec { name: "aes256-ctr", key_len: 32, iv_len: 16, block: 16, kind: Some(CipherKind::Aes256Ctr), aead: Aead::No },
        CipherSpec { name: "aes192-ctr", key_len: 24, iv_len: 16, block: 16, kind: Some(CipherKind::Aes192Ctr), aead: Aead::No },
        CipherSpec { name: "aes128-ctr", key_len: 16, iv_len: 16, block: 16, kind: Some(CipherKind::Aes128Ctr), aead: Aead::No },
        CipherSpec { name: "aes256-cbc", key_len: 32, iv_len: 16, block: 16, kind: Some(CipherKind::Aes256Cbc), aead: Aead::No },
        CipherSpec { name: "aes128-cbc", key_len: 16, iv_len: 16, block: 16, kind: Some(CipherKind::Aes128Cbc), aead: Aead::No },
    ]
}

/// The MACs c-sh offers, best first: (name, hash, length, ETM).
pub struct MacSpec {
    pub name: &'static str,
    pub hash: Hash,
    pub etm: bool,
}

pub fn macs() -> &'static [MacSpec] {
    &[
        MacSpec { name: "hmac-sha2-256-etm@openssh.com", hash: Hash::Sha256, etm: true },
        MacSpec { name: "hmac-sha2-512-etm@openssh.com", hash: Hash::Sha512, etm: true },
        MacSpec { name: "hmac-sha2-256", hash: Hash::Sha256, etm: false },
        MacSpec { name: "hmac-sha2-512", hash: Hash::Sha512, etm: false },
        MacSpec { name: "hmac-sha1", hash: Hash::Sha1, etm: false },
    ]
}
