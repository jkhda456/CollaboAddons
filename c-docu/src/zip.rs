//! ZIP containers, read and written back in part: an entry nobody changed is copied byte for byte
//! (its local header, its data and its descriptor), so a document keeps everything this tool did
//! not touch. Changed entries are compressed again with the method they had; new ones are deflated
//! (stored when they are already compressed, as images are).

use crate::{err, Result};

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const EOCD64_SIG: u32 = 0x0606_4b50;
const EOCD64_LOC_SIG: u32 = 0x0706_4b50;
const DESCRIPTOR_SIG: u32 = 0x0807_4b50;

#[derive(Clone)]
pub struct Entry {
    pub name: String,
    method: u16,
    flags: u16,
    crc: u32,
    csize: u64,
    size: u64,
    time: u16,
    date: u16,
    /// Where the original's local record starts, and how long it is (header, data, descriptor).
    local: Option<(u64, u64)>,
    /// Where the data starts in the original.
    data_at: u64,
    /// The original central directory record, copied when the entry is unchanged.
    central: Vec<u8>,
    /// New (uncompressed) content.
    content: Option<Vec<u8>>,
}

impl Entry {
    pub fn size(&self) -> u64 {
        self.content.as_ref().map(|c| c.len() as u64).unwrap_or(self.size)
    }
    pub fn changed(&self) -> bool {
        self.content.is_some()
    }
}

pub struct Zip {
    data: Vec<u8>,
    entries: Vec<Entry>,
    comment: Vec<u8>,
}

fn u16_at(d: &[u8], i: usize) -> Result<u16> {
    d.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(|| err("truncated zip"))
}
fn u32_at(d: &[u8], i: usize) -> Result<u32> {
    d.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or_else(|| err("truncated zip"))
}
fn u64_at(d: &[u8], i: usize) -> Result<u64> {
    d.get(i..i + 8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| err("truncated zip"))
}

static CRC_TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();

pub fn crc32(data: &[u8]) -> u32 {
    let table = CRC_TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *v = c;
        }
        t
    });
    let mut c = !0u32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

pub fn is_zip(data: &[u8]) -> bool {
    data.len() >= 4 && u32::from_le_bytes([data[0], data[1], data[2], data[3]]) == LOCAL_SIG
}

impl Zip {
    pub fn new() -> Zip {
        Zip { data: Vec::new(), entries: Vec::new(), comment: Vec::new() }
    }

    pub fn open(data: Vec<u8>) -> Result<Zip> {
        let d = &data;
        // The end of central directory record: the last one in the final 64 KiB + 22 bytes.
        let min = d.len().saturating_sub(22 + 65535);
        let mut eocd = None;
        let mut i = d.len().saturating_sub(22);
        loop {
            if u32_at(d, i).ok() == Some(EOCD_SIG) {
                let clen = u16_at(d, i + 20)? as usize;
                if i + 22 + clen <= d.len() {
                    eocd = Some(i);
                    break;
                }
            }
            if i == min {
                break;
            }
            i -= 1;
        }
        let eocd = eocd.ok_or_else(|| err("not a zip archive (no end of central directory)"))?;
        let mut count = u16_at(d, eocd + 10)? as u64;
        let mut cd_size = u32_at(d, eocd + 12)? as u64;
        let mut cd_off = u32_at(d, eocd + 16)? as u64;
        let clen = u16_at(d, eocd + 20)? as usize;
        let comment = d[eocd + 22..eocd + 22 + clen].to_vec();
        if eocd >= 20 && u32_at(d, eocd - 20)? == EOCD64_LOC_SIG {
            let at = u64_at(d, eocd - 12)? as usize;
            if u32_at(d, at)? == EOCD64_SIG {
                count = u64_at(d, at + 32)?;
                cd_size = u64_at(d, at + 40)?;
                cd_off = u64_at(d, at + 48)?;
            }
        }
        // A file with something before the archive (a stub): offsets are relative to its start.
        let shift = (eocd as u64).saturating_sub(cd_off + cd_size);
        let mut entries = Vec::with_capacity(count as usize);
        let mut p = (cd_off + shift) as usize;
        for _ in 0..count {
            if u32_at(d, p)? != CENTRAL_SIG {
                return Err(err("damaged zip central directory"));
            }
            let flags = u16_at(d, p + 8)?;
            let method = u16_at(d, p + 10)?;
            let time = u16_at(d, p + 12)?;
            let date = u16_at(d, p + 14)?;
            let crc = u32_at(d, p + 16)?;
            let mut csize = u32_at(d, p + 20)? as u64;
            let mut size = u32_at(d, p + 24)? as u64;
            let nlen = u16_at(d, p + 28)? as usize;
            let xlen = u16_at(d, p + 30)? as usize;
            let mlen = u16_at(d, p + 32)? as usize;
            let mut local = u32_at(d, p + 42)? as u64;
            let raw_name = d.get(p + 46..p + 46 + nlen).ok_or_else(|| err("truncated zip"))?;
            let name = if flags & 0x800 != 0 {
                String::from_utf8_lossy(raw_name).into_owned()
            } else {
                // CP437 names are ASCII in documents; anything else is taken as UTF-8 anyway.
                String::from_utf8_lossy(raw_name).into_owned()
            };
            // ZIP64 extra field: the values that are 0xFFFFFFFF in the record, in order.
            let mut x = p + 46 + nlen;
            let xend = x + xlen;
            while x + 4 <= xend {
                let id = u16_at(d, x)?;
                let n = u16_at(d, x + 2)? as usize;
                if id == 1 {
                    let mut q = x + 4;
                    if size == 0xFFFF_FFFF {
                        size = u64_at(d, q)?;
                        q += 8;
                    }
                    if csize == 0xFFFF_FFFF {
                        csize = u64_at(d, q)?;
                        q += 8;
                    }
                    if local == 0xFFFF_FFFF {
                        local = u64_at(d, q)?;
                    }
                }
                x += 4 + n;
            }
            let rec_len = 46 + nlen + xlen + mlen;
            let central = d[p..p + rec_len].to_vec();
            p += rec_len;
            let lo = (local + shift) as usize;
            if u32_at(d, lo)? != LOCAL_SIG {
                return Err(err(format!("damaged zip entry {name}")));
            }
            let lnlen = u16_at(d, lo + 26)? as u64;
            let lxlen = u16_at(d, lo + 28)? as u64;
            let data_at = lo as u64 + 30 + lnlen + lxlen;
            let mut end = data_at + csize;
            if flags & 8 != 0 {
                let z64 = size > 0xFFFF_FFFE || csize > 0xFFFF_FFFE;
                let base = if z64 { 20 } else { 12 };
                end += if u32_at(d, end as usize).ok() == Some(DESCRIPTOR_SIG) { base + 4 } else { base };
            }
            if end as usize > d.len() {
                return Err(err(format!("truncated zip entry {name}")));
            }
            entries.push(Entry {
                name,
                method,
                flags,
                crc,
                csize,
                size,
                time,
                date,
                local: Some((lo as u64, end - lo as u64)),
                data_at,
                central,
                content: None,
            });
        }
        Ok(Zip { data, entries, comment })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn names(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.name.clone()).collect()
    }

    fn index(&self, name: &str) -> Option<usize> {
        let name = name.trim_start_matches('/');
        self.entries
            .iter()
            .position(|e| e.name == name)
            .or_else(|| self.entries.iter().position(|e| e.name.eq_ignore_ascii_case(name)))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.index(name).is_some()
    }

    /// The real name of an entry (part names are case-insensitive).
    pub fn real_name(&self, name: &str) -> Option<String> {
        self.index(name).map(|i| self.entries[i].name.clone())
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let i = self.index(name).ok_or_else(|| err(format!("no part {name} in the package")))?;
        let e = &self.entries[i];
        if let Some(c) = &e.content {
            return Ok(c.clone());
        }
        if e.flags & 1 != 0 {
            return Err(err(format!("{} is encrypted", e.name)));
        }
        let raw = &self.data[e.data_at as usize..(e.data_at + e.csize) as usize];
        let out = match e.method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, 1 << 31)
                .map_err(|_| err(format!("{}: bad deflate data", e.name)))?,
            m => return Err(err(format!("{}: compression method {m} is not supported", e.name))),
        };
        Ok(out)
    }

    pub fn read_string(&self, name: &str) -> Result<String> {
        let bytes = self.read(name)?;
        decode_text(bytes).map_err(|_| err(format!("{name} is not UTF-8 text")))
    }

    /// Replaces an entry's content, or adds the entry at the end.
    pub fn write(&mut self, name: &str, content: Vec<u8>) {
        if let Some(i) = self.index(name) {
            let e = &mut self.entries[i];
            if e.content.is_none() && e.local.is_some() {
                // Unchanged content stays a copy of the original.
                if e.size == content.len() as u64 && crc32(&content) == e.crc {
                    return;
                }
            }
            e.content = Some(content);
        } else {
            let compressed = matches!(
                name.rsplit('.').next().map(|s| s.to_ascii_lowercase()).as_deref(),
                Some("png" | "jpg" | "jpeg" | "gif" | "zip" | "mp4" | "mp3" | "m4a" | "wdp" | "webp")
            );
            self.entries.push(Entry {
                name: name.trim_start_matches('/').to_string(),
                method: if compressed { 0 } else { 8 },
                flags: 0,
                crc: 0,
                csize: 0,
                size: 0,
                time: 0,
                date: 0x21, // 1980-01-01, as Office writes
                local: None,
                data_at: 0,
                central: Vec::new(),
                content: Some(content),
            });
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        if let Some(i) = self.index(name) {
            self.entries.remove(i);
            true
        } else {
            false
        }
    }

    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        let i = self.index(from).ok_or_else(|| err(format!("no part {from}")))?;
        let content = self.read(from)?;
        let e = &mut self.entries[i];
        e.name = to.trim_start_matches('/').to_string();
        e.content = Some(content);
        e.local = None;
        Ok(())
    }

    /// The whole archive: unchanged entries copied, changed ones written anew.
    pub fn save(&self) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(self.data.len() + 1024);
        let mut central = Vec::new();
        for e in &self.entries {
            let offset = out.len() as u64;
            if offset > 0xFFFF_FFFE {
                return Err(err("archive over 4 GB is not supported"));
            }
            match (&e.content, e.local) {
                (None, Some((at, len))) => {
                    out.extend_from_slice(&self.data[at as usize..(at + len) as usize]);
                    let mut rec = e.central.clone();
                    let local_field = u32::from_le_bytes(rec[42..46].try_into().unwrap());
                    if local_field == 0xFFFF_FFFF {
                        // The offset is in the ZIP64 extra field: write a plain record instead.
                        rec = central_record(e, offset as u32, e.crc, e.csize as u32, e.size as u32, e.method, e.flags);
                    } else {
                        rec[42..46].copy_from_slice(&(offset as u32).to_le_bytes());
                    }
                    central.extend_from_slice(&rec);
                }
                _ => {
                    let content = match &e.content {
                        Some(c) => c.clone(),
                        None => Vec::new(),
                    };
                    let crc = crc32(&content);
                    let method: u16 = if e.method == 0 { 0 } else { 8 };
                    let data = if method == 8 { miniz_oxide::deflate::compress_to_vec(&content, 6) } else { content.clone() };
                    if data.len() as u64 > 0xFFFF_FFFE || content.len() as u64 > 0xFFFF_FFFE {
                        return Err(err("entry over 4 GB is not supported"));
                    }
                    // No data descriptor, no encryption; keep the UTF-8 name flag.
                    let flags = (e.flags & 0x800) | if e.name.is_ascii() { 0 } else { 0x800 } | (e.flags & 6);
                    let name = e.name.as_bytes();
                    out.extend_from_slice(&LOCAL_SIG.to_le_bytes());
                    out.extend_from_slice(&20u16.to_le_bytes());
                    out.extend_from_slice(&flags.to_le_bytes());
                    out.extend_from_slice(&method.to_le_bytes());
                    out.extend_from_slice(&e.time.to_le_bytes());
                    out.extend_from_slice(&e.date.to_le_bytes());
                    out.extend_from_slice(&crc.to_le_bytes());
                    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
                    out.extend_from_slice(&(content.len() as u32).to_le_bytes());
                    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
                    out.extend_from_slice(&0u16.to_le_bytes());
                    out.extend_from_slice(name);
                    out.extend_from_slice(&data);
                    central.extend_from_slice(&central_record(
                        e,
                        offset as u32,
                        crc,
                        data.len() as u32,
                        content.len() as u32,
                        method,
                        flags,
                    ));
                }
            }
        }
        let cd_off = out.len() as u64;
        if cd_off > 0xFFFF_FFFE || self.entries.len() > 0xFFFE {
            return Err(err("archive too large"));
        }
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&(cd_off as u32).to_le_bytes());
        out.extend_from_slice(&(self.comment.len() as u16).to_le_bytes());
        out.extend_from_slice(&self.comment);
        Ok(out)
    }

    /// How many entries changed, were added or removed (for reports).
    pub fn changed_names(&self) -> Vec<String> {
        self.entries.iter().filter(|e| e.content.is_some()).map(|e| e.name.clone()).collect()
    }
}

fn central_record(e: &Entry, offset: u32, crc: u32, csize: u32, size: u32, method: u16, flags: u16) -> Vec<u8> {
    let mut r = Vec::with_capacity(46 + e.name.len());
    let name = e.name.as_bytes();
    // Keep "version made by" and the external attributes of the original when there is one.
    let (made, ext) = if e.central.len() >= 46 {
        (u16::from_le_bytes([e.central[4], e.central[5]]), u32::from_le_bytes(e.central[38..42].try_into().unwrap()))
    } else {
        (20, 0)
    };
    r.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
    r.extend_from_slice(&made.to_le_bytes());
    r.extend_from_slice(&20u16.to_le_bytes());
    r.extend_from_slice(&flags.to_le_bytes());
    r.extend_from_slice(&method.to_le_bytes());
    r.extend_from_slice(&e.time.to_le_bytes());
    r.extend_from_slice(&e.date.to_le_bytes());
    r.extend_from_slice(&crc.to_le_bytes());
    r.extend_from_slice(&csize.to_le_bytes());
    r.extend_from_slice(&size.to_le_bytes());
    r.extend_from_slice(&(name.len() as u16).to_le_bytes());
    r.extend_from_slice(&0u16.to_le_bytes()); // extra
    r.extend_from_slice(&0u16.to_le_bytes()); // comment
    r.extend_from_slice(&0u16.to_le_bytes()); // disk
    r.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
    r.extend_from_slice(&ext.to_le_bytes());
    r.extend_from_slice(&offset.to_le_bytes());
    r.extend_from_slice(name);
    r
}

/// Text in a part: UTF-8 (with or without a byte order mark), or UTF-16 with one.
pub fn decode_text(bytes: Vec<u8>) -> std::result::Result<String, ()> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let be = bytes[0] == 0xFE;
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
            .collect();
        return String::from_utf16(&units).map_err(|_| ());
    }
    String::from_utf8(bytes).map_err(|_| ())
}
