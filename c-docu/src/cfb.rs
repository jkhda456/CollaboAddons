//! Compound files (OLE2, the container of .doc, .xls, .ppt and .hwp): read into streams, written
//! back with every stream and directory entry kept (names, tree, colors, CLSIDs, timestamps,
//! state bits); only the sectors are laid out anew, so a stream nobody changed keeps its bytes.

use crate::{err, Result};

pub const MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const FREE: u32 = 0xFFFF_FFFF;
const END: u32 = 0xFFFF_FFFE;
const FATSECT: u32 = 0xFFFF_FFFD;
const DIFSECT: u32 = 0xFFFF_FFFC;
const NOSTREAM: u32 = 0xFFFF_FFFF;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    /// 1 storage, 2 stream, 5 root
    pub kind: u8,
    raw: [u8; 128],
    pub left: u32,
    pub right: u32,
    pub child: u32,
    pub data: Vec<u8>,
}

pub struct Cfb {
    pub entries: Vec<Entry>,
    clsid_header: [u8; 16],
}

fn u16_at(d: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([d[i], d[i + 1]])
}
fn u32_at(d: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
}

pub fn is_cfb(d: &[u8]) -> bool {
    d.starts_with(&MAGIC)
}

impl Cfb {
    pub fn open(d: &[u8]) -> Result<Cfb> {
        if d.len() < 512 || !is_cfb(d) {
            return Err(err("not a compound file"));
        }
        let shift = u16_at(d, 30);
        if shift != 9 && shift != 12 {
            return Err(err("compound file: bad sector size"));
        }
        let ss = 1usize << shift;
        let mini_shift = u16_at(d, 32);
        let mss = 1usize << mini_shift;
        let n_fat = u32_at(d, 44) as usize;
        let dir_start = u32_at(d, 48);
        let cutoff = u32_at(d, 56) as u64;
        let minifat_start = u32_at(d, 60);
        let difat_start = u32_at(d, 68);
        let n_difat = u32_at(d, 72) as usize;
        let sector = |s: u32| -> Option<&[u8]> {
            let off = (s as usize + 1) * ss;
            d.get(off..off + ss).or_else(|| d.get(off..d.len()))
        };
        // The FAT's sectors: 109 in the header, then the DIFAT chain.
        let mut fat_sectors: Vec<u32> = Vec::new();
        for i in 0..109 {
            let s = u32_at(d, 76 + i * 4);
            if s != FREE && s != END {
                fat_sectors.push(s);
            }
        }
        let mut ds = difat_start;
        let mut guard = 0;
        while ds != END && ds != FREE && guard < n_difat.max(1) + 10 {
            guard += 1;
            let Some(sec) = sector(ds) else { break };
            let n = ss / 4 - 1;
            for i in 0..n {
                let s = u32_at(sec, i * 4);
                if s != FREE && s != END {
                    fat_sectors.push(s);
                }
            }
            ds = u32_at(sec, n * 4);
        }
        fat_sectors.truncate(n_fat.max(fat_sectors.len().min(n_fat)));
        let mut fat: Vec<u32> = Vec::new();
        for s in &fat_sectors {
            if let Some(sec) = sector(*s) {
                for i in 0..sec.len() / 4 {
                    fat.push(u32_at(sec, i * 4));
                }
            }
        }
        let chain = |start: u32, fat: &[u32]| -> Vec<u32> {
            let mut v = Vec::new();
            let mut s = start;
            while s != END && s != FREE && (s as usize) < fat.len() && v.len() <= fat.len() {
                v.push(s);
                s = fat[s as usize];
            }
            v
        };
        let read_chain = |start: u32, size: Option<u64>| -> Vec<u8> {
            let mut out = Vec::new();
            for s in chain(start, &fat) {
                if let Some(sec) = sector(s) {
                    out.extend_from_slice(sec);
                }
            }
            if let Some(n) = size {
                out.truncate(n as usize);
            }
            out
        };
        // The directory.
        let dir = read_chain(dir_start, None);
        let mut entries = Vec::new();
        for i in 0..dir.len() / 128 {
            let e = &dir[i * 128..(i + 1) * 128];
            let nlen = (u16_at(e, 64) as usize).min(64);
            let units: Vec<u16> = (0..nlen.saturating_sub(2) / 2).map(|k| u16_at(e, k * 2)).collect();
            let name = String::from_utf16_lossy(&units);
            let mut raw = [0u8; 128];
            raw.copy_from_slice(e);
            entries.push(Entry { name, kind: e[66], raw, left: u32_at(e, 68), right: u32_at(e, 72), child: u32_at(e, 76), data: Vec::new() });
        }
        if entries.is_empty() || entries[0].kind != 5 {
            return Err(err("compound file without a root entry"));
        }
        // The mini stream (the root's data) and mini FAT.
        let root_start = u32_at(&entries[0].raw, 116);
        let root_size = u32_at(&entries[0].raw, 120) as u64;
        let mini_stream = read_chain(root_start, Some(root_size));
        let minifat_bytes = read_chain(minifat_start, None);
        let minifat: Vec<u32> = (0..minifat_bytes.len() / 4).map(|i| u32_at(&minifat_bytes, i * 4)).collect();
        for i in 1..entries.len() {
            if entries[i].kind != 2 {
                continue;
            }
            let start = u32_at(&entries[i].raw, 116);
            let mut size = u32_at(&entries[i].raw, 120) as u64;
            if shift == 9 {
                // Version 3: the high half of the size may be garbage.
            } else {
                size |= (u32_at(&entries[i].raw, 124) as u64) << 32;
            }
            entries[i].data = if size < cutoff {
                let mut out = Vec::with_capacity(size as usize);
                let mut s = start;
                let mut guard = 0;
                while s != END && s != FREE && (s as usize) < minifat.len() && guard <= minifat.len() {
                    guard += 1;
                    let off = s as usize * mss;
                    if let Some(b) = mini_stream.get(off..(off + mss).min(mini_stream.len())) {
                        out.extend_from_slice(b);
                    }
                    s = minifat[s as usize];
                }
                out.truncate(size as usize);
                out
            } else {
                read_chain(start, Some(size))
            };
        }
        let mut clsid_header = [0u8; 16];
        clsid_header.copy_from_slice(&d[8..24]);
        Ok(Cfb { entries, clsid_header })
    }

    /// The index of an entry by its path (`WordDocument`, `ObjectPool/_123/Ole`), names compared
    /// without case.
    pub fn find(&self, path: &str) -> Option<usize> {
        let mut cur = 0usize;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            let mut found = None;
            let mut stack = vec![self.entries[cur].child];
            while let Some(i) = stack.pop() {
                if i == NOSTREAM || i as usize >= self.entries.len() {
                    continue;
                }
                let e = &self.entries[i as usize];
                if e.name.eq_ignore_ascii_case(part) {
                    found = Some(i as usize);
                    break;
                }
                stack.push(e.left);
                stack.push(e.right);
            }
            cur = found?;
        }
        Some(cur)
    }

    pub fn stream(&self, path: &str) -> Option<&[u8]> {
        let i = self.find(path)?;
        (self.entries[i].kind == 2).then(|| self.entries[i].data.as_slice())
    }

    pub fn set_stream(&mut self, path: &str, data: Vec<u8>) -> Result<()> {
        let i = self.find(path).ok_or_else(|| err(format!("no stream {path}")))?;
        self.entries[i].data = data;
        Ok(())
    }

    /// The names of the streams and storages directly in the root.
    pub fn names(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![self.entries[0].child];
        while let Some(i) = stack.pop() {
            if i == NOSTREAM || i as usize >= self.entries.len() {
                continue;
            }
            let e = &self.entries[i as usize];
            out.push(e.name.clone());
            stack.push(e.left);
            stack.push(e.right);
        }
        out
    }

    /// The compound file, version 3 (512-byte sectors), with every entry as it was but for the
    /// streams' places and sizes.
    pub fn save(&self) -> Vec<u8> {
        const SS: usize = 512;
        const MSS: usize = 64;
        const CUTOFF: usize = 4096;
        // Mini stream: the small streams, 64-byte sectors.
        let mut mini = Vec::new();
        let mut minifat: Vec<u32> = Vec::new();
        let mut starts: Vec<(u32, u64)> = vec![(END, 0); self.entries.len()];
        for (i, e) in self.entries.iter().enumerate() {
            if e.kind != 2 {
                continue;
            }
            if e.data.len() < CUTOFF {
                if e.data.is_empty() {
                    starts[i] = (END, 0);
                    continue;
                }
                let first = minifat.len() as u32;
                let n = e.data.len().div_ceil(MSS);
                for k in 0..n {
                    minifat.push(if k + 1 == n { END } else { first + k as u32 + 1 });
                }
                mini.extend_from_slice(&e.data);
                mini.resize(minifat.len() * MSS, 0);
                starts[i] = (first, e.data.len() as u64);
            }
        }
        // Sectors: [big streams][mini stream][minifat][directory][FAT][DIFAT]
        let mut sectors: Vec<Vec<u8>> = Vec::new();
        let mut fat: Vec<u32> = Vec::new();
        let put = |data: &[u8], sectors: &mut Vec<Vec<u8>>, fat: &mut Vec<u32>| -> u32 {
            if data.is_empty() {
                return END;
            }
            let first = sectors.len() as u32;
            let n = data.len().div_ceil(SS);
            for k in 0..n {
                let mut s = data[k * SS..((k + 1) * SS).min(data.len())].to_vec();
                s.resize(SS, 0);
                sectors.push(s);
                fat.push(if k + 1 == n { END } else { first + k as u32 + 1 });
            }
            first
        };
        for (i, e) in self.entries.iter().enumerate() {
            if e.kind == 2 && e.data.len() >= CUTOFF {
                let s = put(&e.data, &mut sectors, &mut fat);
                starts[i] = (s, e.data.len() as u64);
            }
        }
        let mini_start = put(&mini, &mut sectors, &mut fat);
        let mut minifat_bytes: Vec<u8> = minifat.iter().flat_map(|v| v.to_le_bytes()).collect();
        if !minifat_bytes.is_empty() {
            let pad = minifat_bytes.len().div_ceil(SS) * SS;
            minifat_bytes.resize(pad, 0xFF);
        }
        let minifat_start = put(&minifat_bytes, &mut sectors, &mut fat);
        let n_minifat = minifat_bytes.len() / SS;
        // Directory entries with their new places.
        let mut dir = Vec::with_capacity(self.entries.len() * 128);
        for (i, e) in self.entries.iter().enumerate() {
            let mut raw = e.raw;
            if i == 0 {
                raw[116..120].copy_from_slice(&mini_start.to_le_bytes());
                raw[120..124].copy_from_slice(&(mini.len() as u32).to_le_bytes());
                raw[124..128].copy_from_slice(&0u32.to_le_bytes());
            } else if e.kind == 2 {
                raw[116..120].copy_from_slice(&starts[i].0.to_le_bytes());
                raw[120..124].copy_from_slice(&(starts[i].1 as u32).to_le_bytes());
                raw[124..128].copy_from_slice(&0u32.to_le_bytes());
            }
            dir.extend_from_slice(&raw);
        }
        // Unused directory slots are empty entries.
        while dir.len() % SS != 0 {
            let mut empty = [0u8; 128];
            empty[68..80].copy_from_slice(&[0xFF; 12]);
            dir.extend_from_slice(&empty);
        }
        let dir_start = put(&dir, &mut sectors, &mut fat);
        // The FAT itself (and DIFAT when there are more than 109 FAT sectors).
        let entries_per = SS / 4;
        let mut n_fat_sectors = 0;
        let mut n_difat = 0;
        loop {
            let total = sectors.len() + n_fat_sectors + n_difat;
            let need = total.div_ceil(entries_per);
            let need_difat = if need > 109 { (need - 109).div_ceil(entries_per - 1) } else { 0 };
            if need == n_fat_sectors && need_difat == n_difat {
                break;
            }
            n_fat_sectors = need;
            n_difat = need_difat;
        }
        let fat_first = sectors.len() as u32;
        for _ in 0..n_fat_sectors {
            fat.push(FATSECT);
        }
        let difat_first = fat.len() as u32;
        for _ in 0..n_difat {
            fat.push(DIFSECT);
        }
        fat.resize(n_fat_sectors * entries_per, FREE);
        let mut fat_bytes: Vec<u8> = fat.iter().flat_map(|v| v.to_le_bytes()).collect();
        fat_bytes.resize(n_fat_sectors * SS, 0xFF);
        for k in 0..n_fat_sectors {
            sectors.push(fat_bytes[k * SS..(k + 1) * SS].to_vec());
        }
        let fat_ids: Vec<u32> = (0..n_fat_sectors as u32).map(|k| fat_first + k).collect();
        for k in 0..n_difat {
            let mut s = Vec::with_capacity(SS);
            for j in 0..entries_per - 1 {
                let idx = 109 + k * (entries_per - 1) + j;
                s.extend_from_slice(&fat_ids.get(idx).copied().unwrap_or(FREE).to_le_bytes());
            }
            let next = if k + 1 < n_difat { difat_first + k as u32 + 1 } else { END };
            s.extend_from_slice(&next.to_le_bytes());
            sectors.push(s);
        }
        // Header.
        let mut h = vec![0u8; SS];
        h[0..8].copy_from_slice(&MAGIC);
        h[8..24].copy_from_slice(&self.clsid_header);
        h[24..26].copy_from_slice(&0x3Eu16.to_le_bytes());
        h[26..28].copy_from_slice(&3u16.to_le_bytes());
        h[28..30].copy_from_slice(&0xFFFEu16.to_le_bytes());
        h[30..32].copy_from_slice(&9u16.to_le_bytes());
        h[32..34].copy_from_slice(&6u16.to_le_bytes());
        h[44..48].copy_from_slice(&(n_fat_sectors as u32).to_le_bytes());
        h[48..52].copy_from_slice(&dir_start.to_le_bytes());
        h[56..60].copy_from_slice(&(CUTOFF as u32).to_le_bytes());
        h[60..64].copy_from_slice(&(if n_minifat > 0 { minifat_start } else { END }).to_le_bytes());
        h[64..68].copy_from_slice(&(n_minifat as u32).to_le_bytes());
        h[68..72].copy_from_slice(&(if n_difat > 0 { difat_first } else { END }).to_le_bytes());
        h[72..76].copy_from_slice(&(n_difat as u32).to_le_bytes());
        for i in 0..109 {
            let v = fat_ids.get(i).copied().unwrap_or(FREE);
            h[76 + i * 4..80 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        let mut out = h;
        for s in sectors {
            out.extend_from_slice(&s);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_keeps_streams() {
        // A file made by save() reads back with the same streams.
        let mut root = [0u8; 128];
        let name: Vec<u8> = "Root Entry".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        root[..name.len()].copy_from_slice(&name);
        root[64..66].copy_from_slice(&((name.len() + 2) as u16).to_le_bytes());
        root[66] = 5;
        root[68..72].copy_from_slice(&NOSTREAM.to_le_bytes());
        root[72..76].copy_from_slice(&NOSTREAM.to_le_bytes());
        root[76..80].copy_from_slice(&1u32.to_le_bytes());
        let mk = |n: &str, left: u32, right: u32| {
            let mut r = [0u8; 128];
            let name: Vec<u8> = n.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
            r[..name.len()].copy_from_slice(&name);
            r[64..66].copy_from_slice(&((name.len() + 2) as u16).to_le_bytes());
            r[66] = 2;
            r[68..72].copy_from_slice(&left.to_le_bytes());
            r[72..76].copy_from_slice(&right.to_le_bytes());
            r[76..80].copy_from_slice(&NOSTREAM.to_le_bytes());
            r
        };
        let c = Cfb {
            entries: vec![
                Entry { name: "Root Entry".into(), kind: 5, raw: root, left: NOSTREAM, right: NOSTREAM, child: 1, data: vec![] },
                Entry { name: "Big".into(), kind: 2, raw: mk("Big", NOSTREAM, 2), left: NOSTREAM, right: 2, child: NOSTREAM, data: (0..10000u32).map(|i| (i % 251) as u8).collect() },
                Entry { name: "Small".into(), kind: 2, raw: mk("Small", NOSTREAM, NOSTREAM), left: NOSTREAM, right: NOSTREAM, child: NOSTREAM, data: b"hello".to_vec() },
            ],
            clsid_header: [0; 16],
        };
        let bytes = c.save();
        let d = Cfb::open(&bytes).unwrap();
        assert_eq!(d.stream("Big").unwrap(), c.entries[1].data.as_slice());
        assert_eq!(d.stream("small").unwrap(), b"hello");
        assert_eq!(d.save(), bytes);
    }
}
