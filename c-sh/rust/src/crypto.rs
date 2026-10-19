//! The cryptography SSH needs, from OpenSSL's libcrypto (3.x): the guest's static library, the one
//! python, curl and git link, so c-sh adds no implementation of its own. Declared here directly
//! (only what is used), with safe wrappers; objects OpenSSL allocates are freed on drop.
//!
//! No struct crosses the boundary by value: OSSL_PARAM arrays are built in place, and BIGNUM
//! parameters go through OSSL_PARAM_BLD.
#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_long, c_uint, c_void, CStr};
use std::ptr::{null, null_mut};

use crate::{err, Result};

#[repr(C)]
pub struct EVP_MD(c_void);
#[repr(C)]
pub struct EVP_MD_CTX(c_void);
#[repr(C)]
pub struct EVP_MAC(c_void);
#[repr(C)]
pub struct EVP_MAC_CTX(c_void);
#[repr(C)]
pub struct EVP_CIPHER(c_void);
#[repr(C)]
pub struct EVP_CIPHER_CTX(c_void);
#[repr(C)]
pub struct EVP_PKEY(c_void);
#[repr(C)]
pub struct EVP_PKEY_CTX(c_void);
#[repr(C)]
pub struct BIGNUM(c_void);
#[repr(C)]
pub struct BN_CTX(c_void);
#[repr(C)]
pub struct ECDSA_SIG(c_void);
#[repr(C)]
pub struct OSSL_PARAM_BLD(c_void);
#[repr(C)]
pub struct BIO(c_void);

/// `struct ossl_param_st`.
#[repr(C)]
pub struct OSSL_PARAM {
    key: *const c_char,
    data_type: c_uint,
    data: *mut c_void,
    data_size: usize,
    return_size: usize,
}

const OSSL_PARAM_UTF8_STRING: c_uint = 4;
const OSSL_PARAM_UNMODIFIED: usize = usize::MAX;

const EVP_PKEY_X25519: c_int = 1034;
const EVP_PKEY_ED25519: c_int = 1087;
const EVP_PKEY_RSA: c_int = 6;
const EVP_PKEY_EC: c_int = 408;
const EVP_PKEY_PUBLIC_KEY: c_int = 0x86;
const EVP_PKEY_KEYPAIR: c_int = 0x87;
const EVP_CTRL_AEAD_SET_IVLEN: c_int = 0x9;
const EVP_CTRL_AEAD_GET_TAG: c_int = 0x10;
const EVP_CTRL_AEAD_SET_TAG: c_int = 0x11;

extern "C" {
    fn RAND_bytes(buf: *mut u8, num: c_int) -> c_int;
    fn ERR_get_error() -> std::ffi::c_ulong;
    fn ERR_error_string_n(e: std::ffi::c_ulong, buf: *mut c_char, len: usize);
    fn ERR_clear_error();

    fn EVP_sha1() -> *const EVP_MD;
    fn EVP_sha256() -> *const EVP_MD;
    fn EVP_sha384() -> *const EVP_MD;
    fn EVP_sha512() -> *const EVP_MD;
    fn EVP_MD_CTX_new() -> *mut EVP_MD_CTX;
    fn EVP_MD_CTX_free(ctx: *mut EVP_MD_CTX);
    fn EVP_MD_CTX_copy_ex(out: *mut EVP_MD_CTX, input: *const EVP_MD_CTX) -> c_int;
    fn EVP_DigestInit_ex(ctx: *mut EVP_MD_CTX, md: *const EVP_MD, engine: *mut c_void) -> c_int;
    fn EVP_DigestUpdate(ctx: *mut EVP_MD_CTX, data: *const c_void, len: usize) -> c_int;
    fn EVP_DigestFinal_ex(ctx: *mut EVP_MD_CTX, out: *mut u8, len: *mut c_uint) -> c_int;
    fn EVP_DigestSignInit(
        ctx: *mut EVP_MD_CTX,
        pctx: *mut *mut EVP_PKEY_CTX,
        md: *const EVP_MD,
        engine: *mut c_void,
        key: *mut EVP_PKEY,
    ) -> c_int;
    fn EVP_DigestSign(ctx: *mut EVP_MD_CTX, sig: *mut u8, siglen: *mut usize, tbs: *const u8, tbslen: usize) -> c_int;
    fn EVP_DigestVerifyInit(
        ctx: *mut EVP_MD_CTX,
        pctx: *mut *mut EVP_PKEY_CTX,
        md: *const EVP_MD,
        engine: *mut c_void,
        key: *mut EVP_PKEY,
    ) -> c_int;
    fn EVP_DigestVerify(ctx: *mut EVP_MD_CTX, sig: *const u8, siglen: usize, tbs: *const u8, tbslen: usize) -> c_int;

    fn EVP_MAC_fetch(libctx: *mut c_void, algorithm: *const c_char, properties: *const c_char) -> *mut EVP_MAC;
    fn EVP_MAC_free(mac: *mut EVP_MAC);
    fn EVP_MAC_CTX_new(mac: *mut EVP_MAC) -> *mut EVP_MAC_CTX;
    fn EVP_MAC_CTX_free(ctx: *mut EVP_MAC_CTX);
    fn EVP_MAC_init(ctx: *mut EVP_MAC_CTX, key: *const u8, keylen: usize, params: *const OSSL_PARAM) -> c_int;
    fn EVP_MAC_update(ctx: *mut EVP_MAC_CTX, data: *const u8, len: usize) -> c_int;
    fn EVP_MAC_final(ctx: *mut EVP_MAC_CTX, out: *mut u8, outl: *mut usize, outsize: usize) -> c_int;

    fn EVP_aes_128_ctr() -> *const EVP_CIPHER;
    fn EVP_aes_192_ctr() -> *const EVP_CIPHER;
    fn EVP_aes_256_ctr() -> *const EVP_CIPHER;
    fn EVP_aes_128_cbc() -> *const EVP_CIPHER;
    fn EVP_aes_256_cbc() -> *const EVP_CIPHER;
    fn EVP_aes_128_gcm() -> *const EVP_CIPHER;
    fn EVP_aes_256_gcm() -> *const EVP_CIPHER;
    fn EVP_chacha20() -> *const EVP_CIPHER;
    fn EVP_CIPHER_CTX_new() -> *mut EVP_CIPHER_CTX;
    fn EVP_CIPHER_CTX_free(ctx: *mut EVP_CIPHER_CTX);
    fn EVP_CIPHER_CTX_set_padding(ctx: *mut EVP_CIPHER_CTX, pad: c_int) -> c_int;
    fn EVP_CIPHER_CTX_ctrl(ctx: *mut EVP_CIPHER_CTX, kind: c_int, arg: c_int, ptr: *mut c_void) -> c_int;
    fn EVP_CipherInit_ex(
        ctx: *mut EVP_CIPHER_CTX,
        cipher: *const EVP_CIPHER,
        engine: *mut c_void,
        key: *const u8,
        iv: *const u8,
        enc: c_int,
    ) -> c_int;
    fn EVP_CipherUpdate(ctx: *mut EVP_CIPHER_CTX, out: *mut u8, outl: *mut c_int, input: *const u8, inl: c_int) -> c_int;
    fn EVP_CipherFinal_ex(ctx: *mut EVP_CIPHER_CTX, out: *mut u8, outl: *mut c_int) -> c_int;

    fn EVP_PKEY_new_raw_private_key(kind: c_int, engine: *mut c_void, key: *const u8, len: usize) -> *mut EVP_PKEY;
    fn EVP_PKEY_new_raw_public_key(kind: c_int, engine: *mut c_void, key: *const u8, len: usize) -> *mut EVP_PKEY;
    fn EVP_PKEY_get_raw_public_key(key: *const EVP_PKEY, out: *mut u8, len: *mut usize) -> c_int;
    fn EVP_PKEY_get_raw_private_key(key: *const EVP_PKEY, out: *mut u8, len: *mut usize) -> c_int;
    fn EVP_PKEY_free(key: *mut EVP_PKEY);
    fn EVP_PKEY_get_base_id(key: *const EVP_PKEY) -> c_int;
    fn EVP_PKEY_get_octet_string_param(
        key: *const EVP_PKEY,
        name: *const c_char,
        buf: *mut u8,
        max: usize,
        out: *mut usize,
    ) -> c_int;
    fn EVP_PKEY_get_utf8_string_param(
        key: *const EVP_PKEY,
        name: *const c_char,
        buf: *mut c_char,
        max: usize,
        out: *mut usize,
    ) -> c_int;
    fn EVP_PKEY_get_bn_param(key: *const EVP_PKEY, name: *const c_char, bn: *mut *mut BIGNUM) -> c_int;
    fn EVP_PKEY_CTX_new(key: *mut EVP_PKEY, engine: *mut c_void) -> *mut EVP_PKEY_CTX;
    fn EVP_PKEY_CTX_new_from_name(libctx: *mut c_void, name: *const c_char, props: *const c_char) -> *mut EVP_PKEY_CTX;
    fn EVP_PKEY_CTX_free(ctx: *mut EVP_PKEY_CTX);
    fn EVP_PKEY_keygen_init(ctx: *mut EVP_PKEY_CTX) -> c_int;
    fn EVP_PKEY_CTX_set_group_name(ctx: *mut EVP_PKEY_CTX, name: *const c_char) -> c_int;
    fn EVP_PKEY_generate(ctx: *mut EVP_PKEY_CTX, key: *mut *mut EVP_PKEY) -> c_int;
    fn EVP_PKEY_derive_init(ctx: *mut EVP_PKEY_CTX) -> c_int;
    fn EVP_PKEY_derive_set_peer(ctx: *mut EVP_PKEY_CTX, peer: *mut EVP_PKEY) -> c_int;
    fn EVP_PKEY_derive(ctx: *mut EVP_PKEY_CTX, out: *mut u8, len: *mut usize) -> c_int;
    fn EVP_PKEY_fromdata_init(ctx: *mut EVP_PKEY_CTX) -> c_int;
    fn EVP_PKEY_fromdata(ctx: *mut EVP_PKEY_CTX, key: *mut *mut EVP_PKEY, selection: c_int, params: *mut OSSL_PARAM) -> c_int;

    fn OSSL_PARAM_BLD_new() -> *mut OSSL_PARAM_BLD;
    fn OSSL_PARAM_BLD_free(bld: *mut OSSL_PARAM_BLD);
    fn OSSL_PARAM_BLD_push_BN(bld: *mut OSSL_PARAM_BLD, key: *const c_char, bn: *const BIGNUM) -> c_int;
    fn OSSL_PARAM_BLD_push_utf8_string(bld: *mut OSSL_PARAM_BLD, key: *const c_char, s: *const c_char, len: usize) -> c_int;
    fn OSSL_PARAM_BLD_push_octet_string(bld: *mut OSSL_PARAM_BLD, key: *const c_char, s: *const c_void, len: usize) -> c_int;
    fn OSSL_PARAM_BLD_to_param(bld: *mut OSSL_PARAM_BLD) -> *mut OSSL_PARAM;
    fn OSSL_PARAM_free(params: *mut OSSL_PARAM);

    fn BN_new() -> *mut BIGNUM;
    fn BN_free(bn: *mut BIGNUM);
    fn BN_clear_free(bn: *mut BIGNUM);
    fn BN_bin2bn(s: *const u8, len: c_int, ret: *mut BIGNUM) -> *mut BIGNUM;
    fn BN_bn2bin(bn: *const BIGNUM, to: *mut u8) -> c_int;
    fn BN_num_bits(bn: *const BIGNUM) -> c_int;
    fn BN_cmp(a: *const BIGNUM, b: *const BIGNUM) -> c_int;
    fn BN_sub(r: *mut BIGNUM, a: *const BIGNUM, b: *const BIGNUM) -> c_int;
    fn BN_div(dv: *mut BIGNUM, rem: *mut BIGNUM, a: *const BIGNUM, d: *const BIGNUM, ctx: *mut BN_CTX) -> c_int;
    fn BN_value_one() -> *const BIGNUM;
    fn BN_mod_exp(r: *mut BIGNUM, a: *const BIGNUM, p: *const BIGNUM, m: *const BIGNUM, ctx: *mut BN_CTX) -> c_int;
    fn BN_get_rfc3526_prime_2048(bn: *mut BIGNUM) -> *mut BIGNUM;
    fn BN_get_rfc3526_prime_4096(bn: *mut BIGNUM) -> *mut BIGNUM;
    fn BN_get_rfc3526_prime_8192(bn: *mut BIGNUM) -> *mut BIGNUM;
    fn BN_CTX_new() -> *mut BN_CTX;
    fn BN_CTX_free(ctx: *mut BN_CTX);

    fn ECDSA_SIG_new() -> *mut ECDSA_SIG;
    fn ECDSA_SIG_free(sig: *mut ECDSA_SIG);
    fn ECDSA_SIG_set0(sig: *mut ECDSA_SIG, r: *mut BIGNUM, s: *mut BIGNUM) -> c_int;
    fn ECDSA_SIG_get0(sig: *const ECDSA_SIG, r: *mut *const BIGNUM, s: *mut *const BIGNUM);
    fn i2d_ECDSA_SIG(sig: *const ECDSA_SIG, out: *mut *mut u8) -> c_int;
    fn d2i_ECDSA_SIG(sig: *mut *mut ECDSA_SIG, input: *mut *const u8, len: c_long) -> *mut ECDSA_SIG;

    fn BIO_new_mem_buf(buf: *const c_void, len: c_int) -> *mut BIO;
    fn BIO_free(bio: *mut BIO) -> c_int;
    fn PEM_read_bio_PrivateKey(
        bio: *mut BIO,
        key: *mut *mut EVP_PKEY,
        cb: Option<extern "C" fn(*mut c_char, c_int, c_int, *mut c_void) -> c_int>,
        u: *mut c_void,
    ) -> *mut EVP_PKEY;
}

/// What OpenSSL says went wrong last, after `what`.
fn openssl_error(what: &str) -> crate::Error {
    let mut text = String::new();
    // SAFETY: the error queue is per thread; the buffer is written within its length.
    unsafe {
        loop {
            let code = ERR_get_error();
            if code == 0 {
                break;
            }
            let mut buf = [0 as c_char; 256];
            ERR_error_string_n(code, buf.as_mut_ptr(), buf.len());
            if text.is_empty() {
                text = CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
            }
        }
    }
    if text.is_empty() {
        err!("{what} failed (OpenSSL)")
    } else {
        err!("{what} failed (OpenSSL: {text})")
    }
}

fn check(ok: c_int, what: &str) -> Result<()> {
    if ok == 1 {
        Ok(())
    } else {
        Err(openssl_error(what))
    }
}

fn nonnull<T>(p: *mut T, what: &str) -> Result<*mut T> {
    if p.is_null() {
        Err(openssl_error(what))
    } else {
        Ok(p)
    }
}

/// Fills `buf` from OpenSSL's random generator (seeded by the kernel's getrandom).
pub fn random(buf: &mut [u8]) {
    // SAFETY: buf is writable for its length. RAND_bytes fails only without entropy, which the
    // kernel always has; a failure must not go on with predictable bytes.
    let ok = unsafe { RAND_bytes(buf.as_mut_ptr(), buf.len() as c_int) };
    if ok != 1 {
        eprintln!("c-sh: no random numbers (OpenSSL RAND_bytes failed)");
        std::process::abort();
    }
}

pub fn random_vec(len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    random(&mut v);
    v
}

/// Compares in time independent of where the inputs differ.
pub fn equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---------------------------------------------------------------------------------------------
// Digests

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    fn md(self) -> *const EVP_MD {
        // SAFETY: these return static method tables.
        unsafe {
            match self {
                Hash::Sha1 => EVP_sha1(),
                Hash::Sha256 => EVP_sha256(),
                Hash::Sha384 => EVP_sha384(),
                Hash::Sha512 => EVP_sha512(),
            }
        }
    }

    pub fn len(self) -> usize {
        match self {
            Hash::Sha1 => 20,
            Hash::Sha256 => 32,
            Hash::Sha384 => 48,
            Hash::Sha512 => 64,
        }
    }

    fn name(self) -> &'static CStr {
        match self {
            Hash::Sha1 => c"SHA1",
            Hash::Sha256 => c"SHA256",
            Hash::Sha384 => c"SHA384",
            Hash::Sha512 => c"SHA512",
        }
    }
}

/// An incremental digest.
pub struct Hasher {
    ctx: *mut EVP_MD_CTX,
    hash: Hash,
}

// SAFETY: the context is owned by this value alone.
unsafe impl Send for Hasher {}

impl Hasher {
    pub fn new(hash: Hash) -> Hasher {
        // SAFETY: a fresh context, initialized with a static method table.
        unsafe {
            let ctx = EVP_MD_CTX_new();
            if ctx.is_null() || EVP_DigestInit_ex(ctx, hash.md(), null_mut()) != 1 {
                eprintln!("c-sh: OpenSSL cannot make a {:?} digest", hash);
                std::process::abort();
            }
            Hasher { ctx, hash }
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        // SAFETY: data is readable for its length.
        unsafe { EVP_DigestUpdate(self.ctx, data.as_ptr() as *const c_void, data.len()) };
    }

    pub fn finish(self) -> Vec<u8> {
        let mut out = vec![0u8; self.hash.len()];
        let mut len: c_uint = 0;
        // SAFETY: out has room for the digest.
        unsafe { EVP_DigestFinal_ex(self.ctx, out.as_mut_ptr(), &mut len) };
        out.truncate(len as usize);
        out
    }
}

impl Clone for Hasher {
    fn clone(&self) -> Hasher {
        // SAFETY: copying one live context into a fresh one.
        unsafe {
            let ctx = EVP_MD_CTX_new();
            if ctx.is_null() || EVP_MD_CTX_copy_ex(ctx, self.ctx) != 1 {
                eprintln!("c-sh: OpenSSL cannot copy a digest");
                std::process::abort();
            }
            Hasher { ctx, hash: self.hash }
        }
    }
}

impl Drop for Hasher {
    fn drop(&mut self) {
        // SAFETY: ours, freed once.
        unsafe { EVP_MD_CTX_free(self.ctx) };
    }
}

pub fn hash(hash: Hash, data: &[u8]) -> Vec<u8> {
    let mut h = Hasher::new(hash);
    h.update(data);
    h.finish()
}

// ---------------------------------------------------------------------------------------------
// MACs: HMAC and Poly1305 (EVP_MAC)

struct Mac {
    mac: *mut EVP_MAC,
    ctx: *mut EVP_MAC_CTX,
}

// SAFETY: owned by this value alone.
unsafe impl Send for Mac {}

impl Mac {
    fn new(algorithm: &CStr) -> Result<Mac> {
        // SAFETY: fetching from the default library context; the context belongs to the MAC.
        unsafe {
            let mac = nonnull(EVP_MAC_fetch(null_mut(), algorithm.as_ptr(), null()), "EVP_MAC_fetch")?;
            let ctx = EVP_MAC_CTX_new(mac);
            if ctx.is_null() {
                EVP_MAC_free(mac);
                return Err(openssl_error("EVP_MAC_CTX_new"));
            }
            Ok(Mac { mac, ctx })
        }
    }

    fn init(&mut self, key: Option<&[u8]>, digest: Option<Hash>) -> Result<()> {
        let name = digest.map(|d| d.name());
        let mut params = [
            OSSL_PARAM {
                key: c"digest".as_ptr(),
                data_type: OSSL_PARAM_UTF8_STRING,
                data: name.map_or(null_mut(), |n| n.as_ptr() as *mut c_void),
                data_size: name.map_or(0, |n| n.to_bytes().len()),
                return_size: OSSL_PARAM_UNMODIFIED,
            },
            OSSL_PARAM { key: null(), data_type: 0, data: null_mut(), data_size: 0, return_size: 0 },
        ];
        if name.is_none() {
            params.swap(0, 1);
        }
        let (k, klen) = key.map_or((null(), 0), |k| (k.as_ptr(), k.len()));
        // SAFETY: the parameters outlive the call; the key is copied by OpenSSL.
        check(unsafe { EVP_MAC_init(self.ctx, k, klen, params.as_ptr()) }, "EVP_MAC_init")
    }

    fn update(&mut self, data: &[u8]) {
        // SAFETY: data is readable for its length.
        unsafe { EVP_MAC_update(self.ctx, data.as_ptr(), data.len()) };
    }

    fn finish(&mut self, out: &mut [u8]) -> usize {
        let mut len = 0usize;
        // SAFETY: out has room for out.len() bytes.
        unsafe { EVP_MAC_final(self.ctx, out.as_mut_ptr(), &mut len, out.len()) };
        len
    }
}

impl Drop for Mac {
    fn drop(&mut self) {
        // SAFETY: ours, freed once.
        unsafe {
            EVP_MAC_CTX_free(self.ctx);
            EVP_MAC_free(self.mac);
        }
    }
}

/// HMAC with one key, computed again and again (one packet each time).
pub struct Hmac {
    mac: Mac,
    len: usize,
}

impl Hmac {
    pub fn new(hash: Hash, key: &[u8]) -> Result<Hmac> {
        let mut mac = Mac::new(c"HMAC")?;
        mac.init(Some(key), Some(hash))?;
        Ok(Hmac { mac, len: hash.len() })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// The MAC of the parts, one after the other.
    pub fn compute(&mut self, parts: &[&[u8]]) -> Vec<u8> {
        // NULL key: again with the key set at the start.
        let _ = self.mac.init(None, None);
        for p in parts {
            self.mac.update(p);
        }
        let mut out = vec![0u8; 64];
        let n = self.mac.finish(&mut out);
        out.truncate(n);
        out
    }
}

pub fn hmac(hash: Hash, key: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    Ok(Hmac::new(hash, key)?.compute(&[data]))
}

/// Poly1305, for chacha20-poly1305@openssh.com: a fresh one-time key per packet.
pub struct Poly1305 {
    mac: Mac,
}

impl Poly1305 {
    pub fn new() -> Result<Poly1305> {
        Ok(Poly1305 { mac: Mac::new(c"POLY1305")? })
    }

    pub fn tag(&mut self, key: &[u8; 32], data: &[u8]) -> Result<[u8; 16]> {
        self.mac.init(Some(key), None)?;
        self.mac.update(data);
        let mut out = [0u8; 16];
        if self.mac.finish(&mut out) != 16 {
            return Err(openssl_error("Poly1305"));
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------------------------
// Ciphers

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CipherKind {
    Aes128Ctr,
    Aes192Ctr,
    Aes256Ctr,
    Aes128Cbc,
    Aes256Cbc,
    Aes128Gcm,
    Aes256Gcm,
    ChaCha20,
}

impl CipherKind {
    fn evp(self) -> *const EVP_CIPHER {
        // SAFETY: static method tables.
        unsafe {
            match self {
                CipherKind::Aes128Ctr => EVP_aes_128_ctr(),
                CipherKind::Aes192Ctr => EVP_aes_192_ctr(),
                CipherKind::Aes256Ctr => EVP_aes_256_ctr(),
                CipherKind::Aes128Cbc => EVP_aes_128_cbc(),
                CipherKind::Aes256Cbc => EVP_aes_256_cbc(),
                CipherKind::Aes128Gcm => EVP_aes_128_gcm(),
                CipherKind::Aes256Gcm => EVP_aes_256_gcm(),
                CipherKind::ChaCha20 => EVP_chacha20(),
            }
        }
    }
}

/// A cipher context that keeps its state between calls (CTR's counter, CBC's chaining), as
/// SSH's packet stream needs.
pub struct Cipher {
    ctx: *mut EVP_CIPHER_CTX,
    encrypt: bool,
}

// SAFETY: owned by this value alone.
unsafe impl Send for Cipher {}

impl Cipher {
    pub fn new(kind: CipherKind, key: &[u8], iv: &[u8], encrypt: bool) -> Result<Cipher> {
        // SAFETY: a fresh context; key and iv are read during the call.
        unsafe {
            let ctx = nonnull(EVP_CIPHER_CTX_new(), "EVP_CIPHER_CTX_new")?;
            let cipher = Cipher { ctx, encrypt };
            if matches!(kind, CipherKind::Aes128Gcm | CipherKind::Aes256Gcm) {
                check(EVP_CipherInit_ex(ctx, kind.evp(), null_mut(), null(), null(), encrypt as c_int), "GCM init")?;
                check(EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_AEAD_SET_IVLEN, iv.len() as c_int, null_mut()), "GCM IV length")?;
                check(EVP_CipherInit_ex(ctx, null(), null_mut(), key.as_ptr(), iv.as_ptr(), encrypt as c_int), "GCM key")?;
            } else {
                check(
                    EVP_CipherInit_ex(ctx, kind.evp(), null_mut(), key.as_ptr(), iv.as_ptr(), encrypt as c_int),
                    "EVP_CipherInit_ex",
                )?;
            }
            EVP_CIPHER_CTX_set_padding(ctx, 0);
            Ok(cipher)
        }
    }

    /// Starts again with another IV, the key unchanged (GCM's per-packet nonce, ChaCha20's).
    pub fn reset_iv(&mut self, iv: &[u8]) -> Result<()> {
        // SAFETY: the context holds its key; iv is read during the call.
        check(
            unsafe { EVP_CipherInit_ex(self.ctx, null(), null_mut(), null(), iv.as_ptr(), self.encrypt as c_int) },
            "EVP_CipherInit_ex (IV)",
        )
    }

    /// Encrypts or decrypts `data` in place (whole blocks for CBC).
    pub fn apply(&mut self, data: &mut [u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let mut outl: c_int = 0;
        // SAFETY: OpenSSL's ciphers work in place when input and output are the same buffer.
        check(
            unsafe { EVP_CipherUpdate(self.ctx, data.as_mut_ptr(), &mut outl, data.as_ptr(), data.len() as c_int) },
            "EVP_CipherUpdate",
        )?;
        if outl as usize != data.len() {
            return Err(err!("cipher took {} of {} bytes", outl, data.len()));
        }
        Ok(())
    }

    /// GCM: additional data, authenticated but not encrypted (SSH's packet length).
    pub fn aad(&mut self, aad: &[u8]) -> Result<()> {
        let mut outl: c_int = 0;
        // SAFETY: a NULL output makes this the AAD.
        check(
            unsafe { EVP_CipherUpdate(self.ctx, null_mut(), &mut outl, aad.as_ptr(), aad.len() as c_int) },
            "GCM AAD",
        )
    }

    /// GCM, encrypting: finishes and returns the tag.
    pub fn seal_tag(&mut self) -> Result<[u8; 16]> {
        let mut outl: c_int = 0;
        let mut tag = [0u8; 16];
        // SAFETY: GCM writes no output at the end; the tag goes into our buffer.
        unsafe {
            check(EVP_CipherFinal_ex(self.ctx, tag.as_mut_ptr(), &mut outl), "GCM final")?;
            check(EVP_CIPHER_CTX_ctrl(self.ctx, EVP_CTRL_AEAD_GET_TAG, 16, tag.as_mut_ptr() as *mut c_void), "GCM tag")?;
        }
        Ok(tag)
    }

    /// GCM, decrypting: checks the tag. False: the packet is not authentic.
    pub fn open_tag(&mut self, tag: &[u8]) -> bool {
        let mut outl: c_int = 0;
        let mut tag = tag.to_vec();
        let mut scratch = [0u8; 16];
        // SAFETY: the tag is copied by OpenSSL; nothing is written past scratch.
        unsafe {
            EVP_CIPHER_CTX_ctrl(self.ctx, EVP_CTRL_AEAD_SET_TAG, tag.len() as c_int, tag.as_mut_ptr() as *mut c_void) == 1
                && EVP_CipherFinal_ex(self.ctx, scratch.as_mut_ptr(), &mut outl) == 1
        }
    }
}

impl Drop for Cipher {
    fn drop(&mut self) {
        // SAFETY: ours, freed once.
        unsafe { EVP_CIPHER_CTX_free(self.ctx) };
    }
}

// ---------------------------------------------------------------------------------------------
// Keys

/// An EVP_PKEY of ours.
pub struct PKey(*mut EVP_PKEY);

// SAFETY: OpenSSL keys may be used from any one thread at a time.
unsafe impl Send for PKey {}

impl Drop for PKey {
    fn drop(&mut self) {
        // SAFETY: ours, freed once.
        unsafe { EVP_PKEY_free(self.0) };
    }
}

struct Bn(*mut BIGNUM);

impl Bn {
    fn from_bytes(bytes: &[u8]) -> Result<Bn> {
        // SAFETY: bytes are read during the call; a NULL ret allocates a new BIGNUM.
        let p = unsafe { BN_bin2bn(bytes.as_ptr(), bytes.len() as c_int, null_mut()) };
        Ok(Bn(nonnull(p, "BN_bin2bn")?))
    }

    fn new() -> Result<Bn> {
        // SAFETY: allocation.
        Ok(Bn(nonnull(unsafe { BN_new() }, "BN_new")?))
    }

    fn to_bytes(&self) -> Vec<u8> {
        bn_bytes(self.0)
    }

    /// Gives up ownership (to OpenSSL, which frees it).
    fn into_raw(self) -> *mut BIGNUM {
        let p = self.0;
        std::mem::forget(self);
        p
    }
}

impl Drop for Bn {
    fn drop(&mut self) {
        // SAFETY: ours, freed once; cleared, as these may be secrets.
        unsafe { BN_clear_free(self.0) };
    }
}

fn bn_bytes(bn: *const BIGNUM) -> Vec<u8> {
    // SAFETY: the buffer is BN_num_bytes long, which BN_bn2bin writes exactly.
    unsafe {
        let len = (BN_num_bits(bn) as usize).div_ceil(8);
        let mut out = vec![0u8; len];
        BN_bn2bin(bn, out.as_mut_ptr());
        out
    }
}

struct BnCtx(*mut BN_CTX);

impl BnCtx {
    fn new() -> Result<BnCtx> {
        // SAFETY: allocation.
        Ok(BnCtx(nonnull(unsafe { BN_CTX_new() }, "BN_CTX_new")?))
    }
}

impl Drop for BnCtx {
    fn drop(&mut self) {
        // SAFETY: ours, freed once.
        unsafe { BN_CTX_free(self.0) };
    }
}

/// Parameters for EVP_PKEY_fromdata, through OSSL_PARAM_BLD. The builder keeps pointers to what
/// it is given until it builds the array, so the numbers and octets stay here until then (and the
/// numbers, which may be secret, are cleared when they go).
struct Params {
    bld: *mut OSSL_PARAM_BLD,
    numbers: Vec<Bn>,
    octets: Vec<Vec<u8>>,
}

impl Params {
    fn new() -> Result<Params> {
        // SAFETY: allocation.
        let bld = nonnull(unsafe { OSSL_PARAM_BLD_new() }, "OSSL_PARAM_BLD_new")?;
        Ok(Params { bld, numbers: Vec::new(), octets: Vec::new() })
    }

    fn bn(&mut self, key: &'static CStr, bytes: &[u8]) -> Result<()> {
        let bn = Bn::from_bytes(bytes)?;
        // SAFETY: the number lives in self.numbers until the array is built.
        check(unsafe { OSSL_PARAM_BLD_push_BN(self.bld, key.as_ptr(), bn.0) }, "OSSL_PARAM_BLD_push_BN")?;
        self.numbers.push(bn);
        Ok(())
    }

    fn text(&mut self, key: &'static CStr, value: &'static CStr) -> Result<()> {
        // SAFETY: a static string.
        check(
            unsafe { OSSL_PARAM_BLD_push_utf8_string(self.bld, key.as_ptr(), value.as_ptr(), 0) },
            "OSSL_PARAM_BLD_push_utf8_string",
        )
    }

    fn octets(&mut self, key: &'static CStr, value: &[u8]) -> Result<()> {
        let copy = value.to_vec();
        // SAFETY: the copy lives in self.octets (its heap buffer does not move) until the build.
        check(
            unsafe { OSSL_PARAM_BLD_push_octet_string(self.bld, key.as_ptr(), copy.as_ptr() as *const c_void, copy.len()) },
            "OSSL_PARAM_BLD_push_octet_string",
        )?;
        self.octets.push(copy);
        Ok(())
    }

    /// A key of `algorithm` from the parameters.
    fn build_key(self, algorithm: &CStr, selection: c_int) -> Result<PKey> {
        // SAFETY: the array is built while everything pushed is alive, used once and freed.
        unsafe {
            let params = nonnull(OSSL_PARAM_BLD_to_param(self.bld), "OSSL_PARAM_BLD_to_param")?;
            let ctx = EVP_PKEY_CTX_new_from_name(null_mut(), algorithm.as_ptr(), null());
            let mut key: *mut EVP_PKEY = null_mut();
            let ok = !ctx.is_null()
                && EVP_PKEY_fromdata_init(ctx) == 1
                && EVP_PKEY_fromdata(ctx, &mut key, selection, params) == 1;
            if !ctx.is_null() {
                EVP_PKEY_CTX_free(ctx);
            }
            OSSL_PARAM_free(params);
            if !ok || key.is_null() {
                return Err(openssl_error("EVP_PKEY_fromdata"));
            }
            Ok(PKey(key))
        }
    }
}

impl Drop for Params {
    fn drop(&mut self) {
        // SAFETY: ours, freed once (the numbers and octets after it, as fields drop).
        unsafe { OSSL_PARAM_BLD_free(self.bld) };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    P256,
    P384,
    P521,
}

impl Curve {
    fn group(self) -> &'static CStr {
        match self {
            Curve::P256 => c"P-256",
            Curve::P384 => c"P-384",
            Curve::P521 => c"P-521",
        }
    }

    pub fn ssh_name(self) -> &'static str {
        match self {
            Curve::P256 => "nistp256",
            Curve::P384 => "nistp384",
            Curve::P521 => "nistp521",
        }
    }

    pub fn from_ssh_name(name: &str) -> Option<Curve> {
        match name {
            "nistp256" => Some(Curve::P256),
            "nistp384" => Some(Curve::P384),
            "nistp521" => Some(Curve::P521),
            _ => None,
        }
    }

    /// The digest ecdsa-sha2-<curve> signs with.
    pub fn hash(self) -> Hash {
        match self {
            Curve::P256 => Hash::Sha256,
            Curve::P384 => Hash::Sha384,
            Curve::P521 => Hash::Sha512,
        }
    }

}

fn ec_public(curve: Curve, point: &[u8]) -> Result<PKey> {
    let mut p = Params::new()?;
    p.text(c"group", curve.group())?;
    p.octets(c"pub", point)?;
    p.build_key(c"EC", EVP_PKEY_PUBLIC_KEY)
}

fn derive(ours: &PKey, peer: &PKey) -> Result<Vec<u8>> {
    // SAFETY: both keys are alive for the call; the output is sized by the first call.
    unsafe {
        let ctx = nonnull(EVP_PKEY_CTX_new(ours.0, null_mut()), "EVP_PKEY_CTX_new")?;
        let mut len = 0usize;
        let ok = EVP_PKEY_derive_init(ctx) == 1
            && EVP_PKEY_derive_set_peer(ctx, peer.0) == 1
            && EVP_PKEY_derive(ctx, null_mut(), &mut len) == 1;
        let mut out = vec![0u8; len];
        let ok = ok && EVP_PKEY_derive(ctx, out.as_mut_ptr(), &mut len) == 1;
        EVP_PKEY_CTX_free(ctx);
        if !ok {
            return Err(openssl_error("key agreement"));
        }
        out.truncate(len);
        Ok(out)
    }
}

/// An ephemeral key for one key exchange: X25519, or ECDH on a NIST curve.
pub struct Ephemeral {
    key: PKey,
    curve: Option<Curve>,
    /// What goes to the server: the X25519 key, or the SEC1 point.
    pub public: Vec<u8>,
}

impl Ephemeral {
    pub fn x25519() -> Result<Ephemeral> {
        let mut secret = [0u8; 32];
        random(&mut secret);
        // SAFETY: the raw key is copied by OpenSSL.
        let key = PKey(nonnull(
            unsafe { EVP_PKEY_new_raw_private_key(EVP_PKEY_X25519, null_mut(), secret.as_ptr(), 32) },
            "X25519 key",
        )?);
        secret.fill(0);
        let mut public = vec![0u8; 32];
        let mut len = 32usize;
        // SAFETY: room for 32 bytes.
        check(unsafe { EVP_PKEY_get_raw_public_key(key.0, public.as_mut_ptr(), &mut len) }, "X25519 public key")?;
        Ok(Ephemeral { key, curve: None, public })
    }

    pub fn ecdh(curve: Curve) -> Result<Ephemeral> {
        // SAFETY: a key generated into `key`; the context freed after.
        unsafe {
            let ctx = nonnull(EVP_PKEY_CTX_new_from_name(null_mut(), c"EC".as_ptr(), null()), "EC context")?;
            let mut key: *mut EVP_PKEY = null_mut();
            let ok = EVP_PKEY_keygen_init(ctx) == 1
                && EVP_PKEY_CTX_set_group_name(ctx, curve.group().as_ptr()) == 1
                && EVP_PKEY_generate(ctx, &mut key) == 1;
            EVP_PKEY_CTX_free(ctx);
            if !ok || key.is_null() {
                return Err(openssl_error("EC key generation"));
            }
            let key = PKey(key);
            let public = octet_param(&key, c"encoded-pub-key")?;
            Ok(Ephemeral { key, curve: Some(curve), public })
        }
    }

    /// The shared secret with the server's public value: X25519's 32 bytes, or ECDH's x.
    pub fn agree(&self, peer: &[u8]) -> Result<Vec<u8>> {
        let peer = match self.curve {
            None => {
                if peer.len() != 32 {
                    return Err(err!("the server's X25519 key has {} bytes", peer.len()));
                }
                // SAFETY: the raw key is copied.
                PKey(nonnull(
                    unsafe { EVP_PKEY_new_raw_public_key(EVP_PKEY_X25519, null_mut(), peer.as_ptr(), 32) },
                    "X25519 peer key",
                )?)
            }
            Some(curve) => ec_public(curve, peer)?,
        };
        let shared = derive(&self.key, &peer)?;
        if self.curve.is_none() && shared.iter().all(|&b| b == 0) {
            return Err(err!("the server's X25519 key is a low-order point"));
        }
        Ok(shared)
    }
}

fn octet_param(key: &PKey, name: &CStr) -> Result<Vec<u8>> {
    let mut len = 0usize;
    // SAFETY: the first call sizes, the second writes within the buffer.
    unsafe {
        check(EVP_PKEY_get_octet_string_param(key.0, name.as_ptr(), null_mut(), 0, &mut len), "EVP_PKEY_get_octet_string_param")?;
        let mut out = vec![0u8; len];
        check(
            EVP_PKEY_get_octet_string_param(key.0, name.as_ptr(), out.as_mut_ptr(), len, &mut len),
            "EVP_PKEY_get_octet_string_param",
        )?;
        out.truncate(len);
        Ok(out)
    }
}

/// The fixed groups of diffie-hellman-group14, group16 and group18 (RFC 3526): their primes, as OpenSSL carries them. The generator is 2 in all three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DhGroup {
    Modp2048,
    Modp4096,
    Modp8192,
}

impl DhGroup {
    pub fn prime(self) -> Vec<u8> {
        // SAFETY: each allocates a new BIGNUM holding the constant; freed after it is copied.
        unsafe {
            let bn = match self {
                DhGroup::Modp2048 => BN_get_rfc3526_prime_2048(null_mut()),
                DhGroup::Modp4096 => BN_get_rfc3526_prime_4096(null_mut()),
                DhGroup::Modp8192 => BN_get_rfc3526_prime_8192(null_mut()),
            };
            if bn.is_null() {
                eprintln!("c-sh: OpenSSL has no {self:?} prime");
                std::process::abort();
            }
            let out = bn_bytes(bn);
            BN_free(bn);
            out
        }
    }
}

/// Finite-field Diffie-Hellman (diffie-hellman-group14/16/18, group-exchange).
pub struct Dh {
    p: Bn,
    x: Bn,
    /// e = g^x mod p, what goes to the server.
    pub public: Vec<u8>,
}

impl Dh {
    /// A private exponent of `bits` random bits (twice the security level, as RFC 8268 asks).
    pub fn new(p: &[u8], g: &[u8], bits: usize) -> Result<Dh> {
        let p = Bn::from_bytes(p)?;
        let g = Bn::from_bytes(g)?;
        let mut xb = vec![0u8; bits.div_ceil(8)];
        random(&mut xb);
        xb[0] |= 0x80; // the full length
        let x = Bn::from_bytes(&xb)?;
        xb.fill(0);
        let e = Bn::new()?;
        let ctx = BnCtx::new()?;
        // SAFETY: all operands alive; e is written.
        check(unsafe { BN_mod_exp(e.0, g.0, x.0, p.0, ctx.0) }, "BN_mod_exp")?;
        let public = e.to_bytes();
        Ok(Dh { p, x, public })
    }

    /// The shared secret from the server's f (checked to be within 1 < f < p-1).
    pub fn agree(&self, f: &[u8]) -> Result<Vec<u8>> {
        let f = Bn::from_bytes(f)?;
        let pm1 = Bn::new()?;
        // SAFETY: plain arithmetic and comparisons on live numbers.
        unsafe {
            check(BN_sub(pm1.0, self.p.0, BN_value_one()), "BN_sub")?;
            if BN_cmp(f.0, BN_value_one()) <= 0 || BN_cmp(f.0, pm1.0) >= 0 {
                return Err(err!("the server's Diffie-Hellman value is out of range"));
            }
            let k = Bn::new()?;
            let ctx = BnCtx::new()?;
            check(BN_mod_exp(k.0, f.0, self.x.0, self.p.0, ctx.0), "BN_mod_exp")?;
            Ok(k.to_bytes())
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Signatures

fn verify_with(key: &PKey, md: *const EVP_MD, data: &[u8], sig: &[u8]) -> bool {
    // SAFETY: a fresh digest context bound to the live key for this call only.
    unsafe {
        let ctx = EVP_MD_CTX_new();
        if ctx.is_null() {
            return false;
        }
        let ok = EVP_DigestVerifyInit(ctx, null_mut(), md, null_mut(), key.0) == 1
            && EVP_DigestVerify(ctx, sig.as_ptr(), sig.len(), data.as_ptr(), data.len()) == 1;
        EVP_MD_CTX_free(ctx);
        ERR_clear_error();
        ok
    }
}

pub fn verify_ed25519(public: &[u8], data: &[u8], sig: &[u8]) -> bool {
    if public.len() != 32 || sig.len() != 64 {
        return false;
    }
    // SAFETY: the raw key is copied.
    let key = unsafe { EVP_PKEY_new_raw_public_key(EVP_PKEY_ED25519, null_mut(), public.as_ptr(), 32) };
    if key.is_null() {
        return false;
    }
    verify_with(&PKey(key), null(), data, sig)
}

/// ECDSA over `data` hashed with the curve's digest; r and s as big-endian integers.
pub fn verify_ecdsa(curve: Curve, point: &[u8], data: &[u8], r: &[u8], s: &[u8]) -> bool {
    let Ok(key) = ec_public(curve, point) else { return false };
    let Ok(der) = ecdsa_der(r, s) else { return false };
    verify_with(&key, curve.hash().md(), data, &der)
}

fn ecdsa_der(r: &[u8], s: &[u8]) -> Result<Vec<u8>> {
    // SAFETY: the SIG takes the two numbers; i2d sizes, then writes within the buffer.
    unsafe {
        let sig = nonnull(ECDSA_SIG_new(), "ECDSA_SIG_new")?;
        let (r, s) = (Bn::from_bytes(r)?, Bn::from_bytes(s)?);
        if ECDSA_SIG_set0(sig, r.into_raw(), s.into_raw()) != 1 {
            ECDSA_SIG_free(sig);
            return Err(openssl_error("ECDSA_SIG_set0"));
        }
        let len = i2d_ECDSA_SIG(sig, null_mut());
        if len <= 0 {
            ECDSA_SIG_free(sig);
            return Err(openssl_error("i2d_ECDSA_SIG"));
        }
        let mut out = vec![0u8; len as usize];
        let mut p = out.as_mut_ptr();
        i2d_ECDSA_SIG(sig, &mut p);
        ECDSA_SIG_free(sig);
        Ok(out)
    }
}

/// r and s of a DER ECDSA-Sig-Value.
fn ecdsa_from_der(der: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    // SAFETY: d2i reads within der; the numbers are copied out before the SIG is freed.
    unsafe {
        let mut p = der.as_ptr();
        let sig = nonnull(d2i_ECDSA_SIG(null_mut(), &mut p, der.len() as c_long), "d2i_ECDSA_SIG")?;
        let (mut r, mut s): (*const BIGNUM, *const BIGNUM) = (null(), null());
        ECDSA_SIG_get0(sig, &mut r, &mut s);
        let out = (bn_bytes(r), bn_bytes(s));
        ECDSA_SIG_free(sig);
        Ok(out)
    }
}

pub fn verify_rsa(n: &[u8], e: &[u8], hash: Hash, data: &[u8], sig: &[u8]) -> bool {
    let key = (|| {
        let mut p = Params::new()?;
        p.bn(c"n", n)?;
        p.bn(c"e", e)?;
        p.build_key(c"RSA", EVP_PKEY_PUBLIC_KEY)
    })();
    match key {
        Ok(key) => verify_with(&key, hash.md(), data, sig),
        Err(_) => false,
    }
}

/// A private key to sign with (a user's key for publickey authentication).
pub struct PrivateKey {
    key: PKey,
}

/// What kind of key a PrivateKey is, with the public parts SSH puts in a key blob.
pub enum PublicParts {
    Ed25519(Vec<u8>),
    Ecdsa(Curve, Vec<u8>),
    Rsa { e: Vec<u8>, n: Vec<u8> },
}

impl PrivateKey {
    pub fn ed25519(seed: &[u8]) -> Result<PrivateKey> {
        if seed.len() != 32 {
            return Err(err!("an Ed25519 key is 32 bytes, not {}", seed.len()));
        }
        // SAFETY: the raw key is copied.
        let key = nonnull(
            unsafe { EVP_PKEY_new_raw_private_key(EVP_PKEY_ED25519, null_mut(), seed.as_ptr(), 32) },
            "Ed25519 key",
        )?;
        Ok(PrivateKey { key: PKey(key) })
    }

    pub fn ecdsa(curve: Curve, point: &[u8], scalar: &[u8]) -> Result<PrivateKey> {
        let mut p = Params::new()?;
        p.text(c"group", curve.group())?;
        p.octets(c"pub", point)?;
        p.bn(c"priv", scalar)?;
        Ok(PrivateKey { key: p.build_key(c"EC", EVP_PKEY_KEYPAIR)? })
    }

    /// RSA from n, e, d and the primes; the CRT exponents are worked out here.
    pub fn rsa(n: &[u8], e: &[u8], d: &[u8], p: &[u8], q: &[u8], iqmp: &[u8]) -> Result<PrivateKey> {
        let (dmp1, dmq1) = {
            let (d, pb, qb) = (Bn::from_bytes(d)?, Bn::from_bytes(p)?, Bn::from_bytes(q)?);
            let (pm1, qm1, r1, r2) = (Bn::new()?, Bn::new()?, Bn::new()?, Bn::new()?);
            let ctx = BnCtx::new()?;
            // SAFETY: arithmetic on live numbers.
            unsafe {
                check(BN_sub(pm1.0, pb.0, BN_value_one()), "BN_sub")?;
                check(BN_sub(qm1.0, qb.0, BN_value_one()), "BN_sub")?;
                check(BN_div(null_mut(), r1.0, d.0, pm1.0, ctx.0), "BN_div")?;
                check(BN_div(null_mut(), r2.0, d.0, qm1.0, ctx.0), "BN_div")?;
            }
            (r1.to_bytes(), r2.to_bytes())
        };
        let mut b = Params::new()?;
        b.bn(c"n", n)?;
        b.bn(c"e", e)?;
        b.bn(c"d", d)?;
        b.bn(c"rsa-factor1", p)?;
        b.bn(c"rsa-factor2", q)?;
        b.bn(c"rsa-exponent1", &dmp1)?;
        b.bn(c"rsa-exponent2", &dmq1)?;
        b.bn(c"rsa-coefficient1", iqmp)?;
        Ok(PrivateKey { key: b.build_key(c"RSA", EVP_PKEY_KEYPAIR)? })
    }

    /// A PEM private key OpenSSL reads itself: PKCS#1 (`RSA PRIVATE KEY`), SEC1 (`EC PRIVATE
    /// KEY`), PKCS#8 (`PRIVATE KEY`, `ENCRYPTED PRIVATE KEY`), with the passphrase if it has one.
    pub fn from_pem(pem: &[u8], passphrase: Option<&str>) -> Result<PrivateKey> {
        let pass = std::ffi::CString::new(passphrase.unwrap_or("")).map_err(|_| err!("a passphrase with a NUL"))?;
        // SAFETY: the BIO reads pem during the call; the passphrase string outlives it (with no
        // callback, OpenSSL takes `u` as the passphrase).
        unsafe {
            let bio = nonnull(BIO_new_mem_buf(pem.as_ptr() as *const c_void, pem.len() as c_int), "BIO_new_mem_buf")?;
            let key = PEM_read_bio_PrivateKey(bio, null_mut(), None, pass.as_ptr() as *mut c_void);
            BIO_free(bio);
            let key = nonnull(key, "reading the PEM key")?;
            let key = PrivateKey { key: PKey(key) };
            key.public_parts()?;
            Ok(key)
        }
    }

    pub fn public_parts(&self) -> Result<PublicParts> {
        // SAFETY: reading parameters of a live key into buffers sized for them.
        unsafe {
            match EVP_PKEY_get_base_id(self.key.0) {
                EVP_PKEY_ED25519 => {
                    let mut public = vec![0u8; 32];
                    let mut len = 32usize;
                    check(EVP_PKEY_get_raw_public_key(self.key.0, public.as_mut_ptr(), &mut len), "Ed25519 public key")?;
                    Ok(PublicParts::Ed25519(public))
                }
                EVP_PKEY_EC => {
                    let mut name = [0 as c_char; 32];
                    let mut len = 0usize;
                    check(
                        EVP_PKEY_get_utf8_string_param(self.key.0, c"group".as_ptr(), name.as_mut_ptr(), name.len(), &mut len),
                        "EC group",
                    )?;
                    let curve = match CStr::from_ptr(name.as_ptr()).to_bytes() {
                        b"prime256v1" | b"P-256" => Curve::P256,
                        b"secp384r1" | b"P-384" => Curve::P384,
                        b"secp521r1" | b"P-521" => Curve::P521,
                        other => return Err(err!("an EC key on {} (SSH has P-256, P-384, P-521)", String::from_utf8_lossy(other))),
                    };
                    let point = octet_param(&self.key, c"encoded-pub-key")?;
                    Ok(PublicParts::Ecdsa(curve, point))
                }
                EVP_PKEY_RSA => {
                    let get = |name: &CStr| -> Result<Vec<u8>> {
                        let mut bn: *mut BIGNUM = null_mut();
                        check(EVP_PKEY_get_bn_param(self.key.0, name.as_ptr(), &mut bn), "RSA parameter")?;
                        let out = bn_bytes(bn);
                        BN_free(bn);
                        Ok(out)
                    };
                    Ok(PublicParts::Rsa { e: get(c"e")?, n: get(c"n")? })
                }
                other => Err(err!("a key of type {other}, which SSH does not use")),
            }
        }
    }

    /// Signs `data`: Ed25519 (no hash: the 64-byte signature), RSA PKCS#1 v1.5 with `hash`, or
    /// ECDSA as DER (`sign_ecdsa` gives r and s).
    pub fn sign(&self, hash: Option<Hash>, data: &[u8]) -> Result<Vec<u8>> {
        // SAFETY: a fresh digest context bound to the live key; sized, then written.
        unsafe {
            let ctx = nonnull(EVP_MD_CTX_new(), "EVP_MD_CTX_new")?;
            let md = hash.map_or(null(), |h| h.md());
            let mut len = 0usize;
            let ok = EVP_DigestSignInit(ctx, null_mut(), md, null_mut(), self.key.0) == 1
                && EVP_DigestSign(ctx, null_mut(), &mut len, data.as_ptr(), data.len()) == 1;
            let mut out = vec![0u8; len];
            let ok = ok && EVP_DigestSign(ctx, out.as_mut_ptr(), &mut len, data.as_ptr(), data.len()) == 1;
            EVP_MD_CTX_free(ctx);
            if !ok {
                return Err(openssl_error("signing"));
            }
            out.truncate(len);
            Ok(out)
        }
    }

    pub fn sign_ecdsa(&self, curve: Curve, data: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        ecdsa_from_der(&self.sign(Some(curve.hash()), data)?)
    }

    /// The raw Ed25519 seed (for writing a key out; not used for signing).
    #[allow(dead_code)]
    pub fn ed25519_seed(&self) -> Result<Vec<u8>> {
        let mut seed = vec![0u8; 32];
        let mut len = 32usize;
        // SAFETY: room for 32 bytes.
        check(unsafe { EVP_PKEY_get_raw_private_key(self.key.0, seed.as_mut_ptr(), &mut len) }, "Ed25519 seed")?;
        Ok(seed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn digests_and_hmac() {
        assert_eq!(hex(&hash(Hash::Sha256, b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&hash(Hash::Sha1, b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        let mut h = Hasher::new(Hash::Sha512);
        h.update(b"a");
        let copy = h.clone();
        h.update(b"bc");
        assert_eq!(&hex(&h.finish())[..16], "ddaf35a193617aba");
        assert_eq!(&hex(&copy.finish())[..16], "1f40fc92da241694");
        // RFC 4231 test case 2
        let mac = hmac(Hash::Sha256, b"Jefe", b"what do ya want for nothing?").unwrap();
        assert_eq!(hex(&mac), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        let mut m = Hmac::new(Hash::Sha256, b"Jefe").unwrap();
        assert_eq!(m.compute(&[b"what do ya ", b"want for nothing?"]), mac);
        assert_eq!(m.compute(&[b"what do ya want for nothing?"]), mac);
    }

    #[test]
    fn poly1305_rfc8439() {
        let key: [u8; 32] = unhex("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b").try_into().unwrap();
        let tag = Poly1305::new().unwrap().tag(&key, b"Cryptographic Forum Research Group").unwrap();
        assert_eq!(hex(&tag), "a8061dc1305136c6c22b8baf0c0127a9");
    }

    #[test]
    fn ciphers_keep_state() {
        let key = [7u8; 16];
        let iv = [9u8; 16];
        let mut whole = vec![0u8; 48];
        Cipher::new(CipherKind::Aes128Ctr, &key, &iv, true).unwrap().apply(&mut whole).unwrap();
        let mut parts = vec![0u8; 48];
        let mut c = Cipher::new(CipherKind::Aes128Ctr, &key, &iv, true).unwrap();
        c.apply(&mut parts[..16]).unwrap();
        c.apply(&mut parts[16..]).unwrap();
        assert_eq!(whole, parts);
        let mut d = Cipher::new(CipherKind::Aes128Ctr, &key, &iv, false).unwrap();
        d.apply(&mut parts).unwrap();
        assert_eq!(parts, vec![0u8; 48]);
        // CBC chains across calls
        let mut a = vec![1u8; 32];
        Cipher::new(CipherKind::Aes256Cbc, &[3u8; 32], &iv, true).unwrap().apply(&mut a).unwrap();
        let mut b = vec![1u8; 32];
        let mut c = Cipher::new(CipherKind::Aes256Cbc, &[3u8; 32], &iv, true).unwrap();
        c.apply(&mut b[..16]).unwrap();
        c.apply(&mut b[16..]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn gcm_round_trip() {
        let key = [5u8; 32];
        let mut iv = [1u8; 12];
        let mut enc = Cipher::new(CipherKind::Aes256Gcm, &key, &iv, true).unwrap();
        let mut dec = Cipher::new(CipherKind::Aes256Gcm, &key, &iv, false).unwrap();
        for round in 0..3 {
            iv[11] = round;
            enc.reset_iv(&iv).unwrap();
            dec.reset_iv(&iv).unwrap();
            let mut data = b"0123456789abcdef".to_vec();
            enc.aad(b"len!").unwrap();
            enc.apply(&mut data).unwrap();
            let tag = enc.seal_tag().unwrap();
            dec.aad(b"len!").unwrap();
            dec.apply(&mut data).unwrap();
            assert!(dec.open_tag(&tag));
            assert_eq!(&data, b"0123456789abcdef");
        }
        iv[11] = 9;
        dec.reset_iv(&iv).unwrap();
        let mut data = vec![0u8; 16];
        dec.aad(b"len!").unwrap();
        dec.apply(&mut data).unwrap();
        assert!(!dec.open_tag(&[0u8; 16]));
    }

    #[test]
    fn chacha20_block_counter() {
        // RFC 8439 2.4.2: key 00..1f, counter 1, nonce 000000000000004a00000000
        let key: Vec<u8> = (0u8..32).collect();
        let mut iv = [0u8; 16];
        iv[0] = 1;
        iv[4..16].copy_from_slice(&unhex("000000000000004a00000000"));
        let mut data = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.".to_vec();
        Cipher::new(CipherKind::ChaCha20, &key, &iv, true).unwrap().apply(&mut data).unwrap();
        assert_eq!(&hex(&data)[..32], "6e2e359a2568f98041ba0728dd0d6981");
    }

    #[test]
    fn key_agreement() {
        let a = Ephemeral::x25519().unwrap();
        let b = Ephemeral::x25519().unwrap();
        assert_eq!(a.agree(&b.public).unwrap(), b.agree(&a.public).unwrap());
        for curve in [Curve::P256, Curve::P384, Curve::P521] {
            let a = Ephemeral::ecdh(curve).unwrap();
            let b = Ephemeral::ecdh(curve).unwrap();
            let s = a.agree(&b.public).unwrap();
            assert_eq!(s, b.agree(&a.public).unwrap());
        }
        let p = DhGroup::Modp2048.prime();
        assert_eq!(p.len(), 256);
        assert_eq!(&hex(&p[..8]), "ffffffffffffffff");
        assert_eq!(&hex(&p[8..12]), "c90fdaa2");
        let a = Dh::new(&p, &[2], 512).unwrap();
        let b = Dh::new(&p, &[2], 512).unwrap();
        assert_eq!(a.agree(&b.public).unwrap(), b.agree(&a.public).unwrap());
        assert!(a.agree(&[1]).is_err());
        let mut pm1 = p.clone();
        *pm1.last_mut().unwrap() -= 1;
        assert!(a.agree(&pm1).is_err());
        assert_eq!(DhGroup::Modp8192.prime().len(), 1024);
    }

    #[test]
    fn signatures() {
        let ed = PrivateKey::ed25519(&[42u8; 32]).unwrap();
        let PublicParts::Ed25519(public) = ed.public_parts().unwrap() else { panic!() };
        let sig = ed.sign(None, b"message").unwrap();
        assert!(verify_ed25519(&public, b"message", &sig));
        assert!(!verify_ed25519(&public, b"messagf", &sig));
        let ec = Ephemeral::ecdh(Curve::P256).unwrap();
        let _ = ec;
    }
}
