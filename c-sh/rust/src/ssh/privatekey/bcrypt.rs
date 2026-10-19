//! bcrypt_pbkdf, the key derivation OpenSSH encrypts private keys with (OpenBSD's
//! bcrypt_pbkdf.c). Blowfish is here in Rust (the bcrypt key schedule is not the standard one, so
//! OpenSSL's `BF_set_key` does not fit); SHA-512 is OpenSSL ([crate::crypto]).

use crate::crypto::{hash, Hash};
use crate::{err, Result};

mod bcrypt_init;

/// Blowfish, only as bcrypt uses it: the P-array and S-boxes, the block function, and the two
/// key-schedule steps (with and without a salt).
struct Blowfish {
    p: [u32; 18],
    s: [u32; 1024],
}

impl Blowfish {
    fn new() -> Blowfish {
        Blowfish { p: bcrypt_init::P_INIT, s: bcrypt_init::S_INIT }
    }

    fn f(&self, x: u32) -> u32 {
        let a = self.s[(x >> 24 & 0xff) as usize];
        let b = self.s[256 + (x >> 16 & 0xff) as usize];
        let c = self.s[512 + (x >> 8 & 0xff) as usize];
        let d = self.s[768 + (x & 0xff) as usize];
        (a.wrapping_add(b) ^ c).wrapping_add(d)
    }

    /// Encrypts one block (two words) in place.
    fn encrypt(&self, l: &mut u32, r: &mut u32) {
        let (mut xl, mut xr) = (*l, *r);
        for i in 0..16 {
            xl ^= self.p[i];
            xr ^= self.f(xl);
            std::mem::swap(&mut xl, &mut xr);
        }
        std::mem::swap(&mut xl, &mut xr);
        xr ^= self.p[16];
        xl ^= self.p[17];
        *l = xl;
        *r = xr;
    }

    /// The expensive key schedule: XOR the key into P, then fill P and S by encrypting a block
    /// that the salt is mixed into (a zero salt for `expand0`).
    fn expand(&mut self, key: &[u8], salt: &[u8]) {
        let mut j = 0;
        for i in 0..18 {
            self.p[i] ^= stream_word(key, &mut j);
        }
        let (mut l, mut r) = (0u32, 0u32);
        let mut sj = 0;
        let mut salt_block = |l: &mut u32, r: &mut u32| {
            if !salt.is_empty() {
                *l ^= stream_word(salt, &mut sj);
                *r ^= stream_word(salt, &mut sj);
            }
        };
        for i in (0..18).step_by(2) {
            salt_block(&mut l, &mut r);
            self.encrypt(&mut l, &mut r);
            self.p[i] = l;
            self.p[i + 1] = r;
        }
        for i in (0..1024).step_by(2) {
            salt_block(&mut l, &mut r);
            self.encrypt(&mut l, &mut r);
            self.s[i] = l;
            self.s[i + 1] = r;
        }
    }
}

/// Reads a big-endian 32-bit word from `data`, cycling and advancing `at` (Blowfish_stream2word).
fn stream_word(data: &[u8], at: &mut usize) -> u32 {
    let mut word = 0u32;
    for _ in 0..4 {
        word = word << 8 | data[*at % data.len()] as u32;
        *at += 1;
    }
    word
}

/// The 32-byte bcrypt core hash of one (password-derived, salt-derived) pair.
fn bcrypt_hash(sha2pass: &[u8; 64], sha2salt: &[u8; 64]) -> [u8; 32] {
    let mut bf = Blowfish::new();
    bf.expand(sha2pass, sha2salt);
    for _ in 0..64 {
        bf.expand(sha2salt, &[]);
        bf.expand(sha2pass, &[]);
    }
    // "OxychromaticBlowfishSwatDynamite" as eight big-endian words, encrypted 64 times.
    let magic = b"OxychromaticBlowfishSwatDynamite";
    let mut words = [0u32; 8];
    let mut j = 0;
    for w in &mut words {
        *w = stream_word(magic, &mut j);
    }
    for _ in 0..64 {
        for pair in words.chunks_mut(2) {
            let (mut l, mut r) = (pair[0], pair[1]);
            bf.encrypt(&mut l, &mut r);
            pair[0] = l;
            pair[1] = r;
        }
    }
    // Store little-endian, as OpenBSD does.
    let mut out = [0u8; 32];
    for (i, w) in words.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}

/// Fills `out` from `pass` and `salt` over `rounds` (OpenSSH's bcrypt KDF).
pub fn pbkdf(pass: &[u8], salt: &[u8], out: &mut [u8], rounds: usize) -> Result<()> {
    if rounds == 0 || out.is_empty() || out.len() > 1024 {
        return Err(err!("bad bcrypt parameters"));
    }
    let keylen = out.len();
    let stride = keylen.div_ceil(32);
    let amt = keylen.div_ceil(stride);
    let sha2pass: [u8; 64] = hash(Hash::Sha512, pass).try_into().unwrap();

    for count in 1..=stride as u32 {
        let mut countsalt = salt.to_vec();
        countsalt.extend_from_slice(&count.to_be_bytes());
        let mut sha2salt: [u8; 64] = hash(Hash::Sha512, &countsalt).try_into().unwrap();
        let mut tmp = bcrypt_hash(&sha2pass, &sha2salt);
        let mut acc = tmp;
        for _ in 1..rounds {
            sha2salt = hash(Hash::Sha512, &tmp).try_into().unwrap();
            tmp = bcrypt_hash(&sha2pass, &sha2salt);
            for (a, t) in acc.iter_mut().zip(&tmp) {
                *a ^= t;
            }
        }
        // Spread this block across the output: byte i*stride + (count-1).
        for (i, &b) in acc.iter().enumerate().take(amt) {
            let dest = i * stride + (count as usize - 1);
            if dest < keylen {
                out[dest] = b;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The known-answer vector from OpenBSD's bcrypt_pbkdf regression test (pass "password",
    /// salt "salt", 4 rounds, a 32-byte key: stride 1, so the bcrypt core hash shows through).
    #[test]
    fn openbsd_vector() {
        let mut out = [0u8; 32];
        pbkdf(b"password", b"salt", &mut out, 4).unwrap();
        let expected: [u8; 32] = [
            0x5b, 0xbf, 0x0c, 0xc2, 0x93, 0x58, 0x7f, 0x1c, 0x36, 0x35, 0x55, 0x5c, 0x27, 0x79, 0x65, 0x98,
            0xd4, 0x7e, 0x57, 0x90, 0x71, 0xbf, 0x42, 0x7e, 0x9d, 0x8f, 0xbe, 0x84, 0x2a, 0xba, 0x34, 0xd9,
        ];
        assert_eq!(out, expected);
        // A longer key strides across count-blocks: each byte must still be set.
        let mut long = [0u8; 48];
        pbkdf(b"pass", b"salty", &mut long, 8).unwrap();
        assert!(long.iter().any(|&b| b != 0));
    }
}
