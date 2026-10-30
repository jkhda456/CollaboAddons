//! The archive's compression: LZ77 with a price-driven optimal parse, coded bit by bit with
//! adaptive binary models and a range coder. The design is the LZMA family's — a literal right
//! after a match is coded against the byte at the match's distance, four repeat distances, a
//! 12-state history of what came last — but the format and the code are our own. A block may
//! start from a preset dictionary (text the reader already has, e.g. the English pages a
//! translation repeats) that matches reach back into.
//!
//! A compressed block carries nothing but the range coder's bytes: the caller keeps its raw
//! length and parameters (the archive's block table).

use crate::{err, Result};

pub const MATCH_MIN: usize = 2;
pub const MATCH_MAX: usize = 273;

const STATES: usize = 12;
const POS_STATES: usize = 16; // 1 << pb, pb <= 4
const LEN_STATES: usize = 4;
const END_POS_MODEL: u32 = 14; // distance slots below this code their low bits with models
const FULL_DISTANCES: usize = 128; // 1 << (END_POS_MODEL / 2)
const ALIGN_BITS: u32 = 4;

const PROB_BITS: u32 = 11;
const PROB_ONE: u32 = 1 << PROB_BITS;
const PROB_INIT: u16 = (PROB_ONE / 2) as u16;
const MOVE_BITS: u32 = 5;
const TOP: u32 = 1 << 24;

/// Literal context bits (of the previous byte), literal position bits, position bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub lc: u32,
    pub lp: u32,
    pub pb: u32,
}

impl Params {
    /// What suits UTF-8 text: no position contexts, half of the previous byte.
    pub const TEXT: Params = Params { lc: 4, lp: 0, pb: 0 };

    pub fn to_byte(self) -> u8 {
        ((self.pb * 5 + self.lp) * 9 + self.lc) as u8
    }

    pub fn from_byte(b: u8) -> Option<Params> {
        let b = b as u32;
        let p = Params { lc: b % 9, lp: b / 9 % 5, pb: b / 45 };
        (p.pb <= 4).then_some(p)
    }
}

/// How hard the encoder looks: chain links followed per position, and the match length taken
/// at once without weighing the alternatives.
#[derive(Clone, Copy, Debug)]
pub struct Effort {
    pub depth: usize,
    pub nice: usize,
}

impl Effort {
    pub const BEST: Effort = Effort { depth: 512, nice: 128 };
    pub const FAST: Effort = Effort { depth: 16, nice: 32 };
}

#[derive(Clone)]
struct LenModel {
    choice: u16,
    choice2: u16,
    low: [[u16; 8]; POS_STATES],
    mid: [[u16; 8]; POS_STATES],
    high: [u16; 256],
}

impl LenModel {
    fn new() -> LenModel {
        LenModel {
            choice: PROB_INIT,
            choice2: PROB_INIT,
            low: [[PROB_INIT; 8]; POS_STATES],
            mid: [[PROB_INIT; 8]; POS_STATES],
            high: [PROB_INIT; 256],
        }
    }
}

/// Where each distance slot from 4 to 13 keeps the reverse tree of its footer bits.
const SPECIAL_AT: [usize; 14] = [0, 0, 0, 0, 0, 2, 4, 8, 12, 20, 28, 44, 60, 92];

#[derive(Clone)]
struct Model {
    p: Params,
    is_match: [u16; STATES * POS_STATES],
    is_rep: [u16; STATES],
    rep_g0: [u16; STATES],
    rep_g1: [u16; STATES],
    rep_g2: [u16; STATES],
    rep0_long: [u16; STATES * POS_STATES],
    slot: [[u16; 64]; LEN_STATES],
    special: [u16; 128],
    align: [u16; 16],
    len: LenModel,
    rep_len: LenModel,
    lit: Vec<u16>,
}

impl Model {
    fn new(p: Params) -> Model {
        Model {
            p,
            is_match: [PROB_INIT; STATES * POS_STATES],
            is_rep: [PROB_INIT; STATES],
            rep_g0: [PROB_INIT; STATES],
            rep_g1: [PROB_INIT; STATES],
            rep_g2: [PROB_INIT; STATES],
            rep0_long: [PROB_INIT; STATES * POS_STATES],
            slot: [[PROB_INIT; 64]; LEN_STATES],
            special: [PROB_INIT; 128],
            align: [PROB_INIT; 16],
            len: LenModel::new(),
            rep_len: LenModel::new(),
            lit: vec![PROB_INIT; 0x300 << (p.lc + p.lp)],
        }
    }

    #[inline]
    fn lit_base(&self, pos: usize, prev: u8) -> usize {
        let lp_mask = (1usize << self.p.lp) - 1;
        (((pos & lp_mask) << self.p.lc) + ((prev as usize) >> (8 - self.p.lc))) * 0x300
    }
}

#[inline]
fn after_lit(s: usize) -> usize {
    if s < 4 {
        0
    } else if s < 10 {
        s - 3
    } else {
        s - 6
    }
}
#[inline]
fn after_match(s: usize) -> usize {
    if s < 7 { 7 } else { 10 }
}
#[inline]
fn after_rep(s: usize) -> usize {
    if s < 7 { 8 } else { 11 }
}
#[inline]
fn after_short(s: usize) -> usize {
    if s < 7 { 9 } else { 11 }
}

/// The slot of a coded distance (distance - 1): its bit length and the bit below the top one.
#[inline]
fn dist_slot(d: u32) -> u32 {
    if d < 4 {
        d
    } else {
        let n = 31 - d.leading_zeros();
        (n << 1) | ((d >> (n - 1)) & 1)
    }
}

// ---------------------------------------------------------------------------------------------
// Decoding

struct RcDec<'a> {
    src: &'a [u8],
    pos: usize,
    range: u32,
    code: u32,
}

impl<'a> RcDec<'a> {
    fn new(src: &'a [u8]) -> RcDec<'a> {
        let mut d = RcDec { src, pos: 0, range: 0xFFFF_FFFF, code: 0 };
        for _ in 0..4 {
            d.code = (d.code << 8) | d.byte() as u32;
        }
        d
    }

    #[inline]
    fn byte(&mut self) -> u8 {
        let b = self.src.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        b
    }

    #[inline]
    fn bit(&mut self, p: &mut u16) -> usize {
        let bound = (self.range >> PROB_BITS) * *p as u32;
        let bit = if self.code < bound {
            self.range = bound;
            *p += ((PROB_ONE - *p as u32) >> MOVE_BITS) as u16;
            0
        } else {
            self.range -= bound;
            self.code -= bound;
            *p -= *p >> MOVE_BITS;
            1
        };
        // A probability stays within [31, 2017], so one byte always brings the range back.
        if self.range < TOP {
            self.range <<= 8;
            self.code = (self.code << 8) | self.byte() as u32;
        }
        bit
    }

    fn direct(&mut self, bits: u32) -> u32 {
        let mut v = 0;
        for _ in 0..bits {
            self.range >>= 1;
            let b = if self.code >= self.range {
                self.code -= self.range;
                1
            } else {
                0
            };
            v = (v << 1) | b;
            if self.range < TOP {
                self.range <<= 8;
                self.code = (self.code << 8) | self.byte() as u32;
            }
        }
        v
    }

    #[inline]
    fn tree(&mut self, probs: &mut [u16], bits: u32) -> u32 {
        let mut m = 1usize;
        for _ in 0..bits {
            m = (m << 1) | self.bit(&mut probs[m]);
        }
        (m - (1 << bits)) as u32
    }

    fn rtree(&mut self, probs: &mut [u16], bits: u32) -> u32 {
        let mut m = 1usize;
        let mut v = 0u32;
        for i in 0..bits {
            let b = self.bit(&mut probs[m]);
            m = (m << 1) | b;
            v |= (b as u32) << i;
        }
        v
    }

    fn len(&mut self, lm: &mut LenModel, ps: usize) -> usize {
        2 + if self.bit(&mut lm.choice) == 0 {
            self.tree(&mut lm.low[ps], 3) as usize
        } else if self.bit(&mut lm.choice2) == 0 {
            8 + self.tree(&mut lm.mid[ps], 3) as usize
        } else {
            16 + self.tree(&mut lm.high, 8) as usize
        }
    }
}

/// Decompresses `src` into `raw_len` bytes, with `dict` as the text before the first one.
pub fn decompress(dict: &[u8], src: &[u8], raw_len: usize, p: Params) -> Result<Vec<u8>> {
    let corrupt = || err!("corrupt compressed block");
    let mut out = Vec::with_capacity(dict.len() + raw_len);
    out.extend_from_slice(dict);
    let end = dict.len() + raw_len;
    let mut m = Model::new(p);
    let mut rc = RcDec::new(src);
    let mut state = 0usize;
    let mut reps = [1u32; 4];
    let pb_mask = (1usize << p.pb) - 1;
    while out.len() < end {
        let pos = out.len();
        let ps = pos & pb_mask;
        if rc.bit(&mut m.is_match[state * POS_STATES + ps]) == 0 {
            let prev = if pos > 0 { out[pos - 1] } else { 0 };
            let base = m.lit_base(pos, prev);
            let probs = &mut m.lit[base..base + 0x300];
            let byte = if state < 7 {
                rc.tree(probs, 8) as u8
            } else {
                // After a match the next byte is likely the one at the same distance; its bits
                // pick the models until the first bit that differs.
                let mut mb = out[pos - reps[0] as usize] as usize;
                let mut offs = 0x100usize;
                let mut sym = 1usize;
                while sym < 0x100 {
                    mb <<= 1;
                    let mbit = mb & offs;
                    let b = rc.bit(&mut probs[offs + mbit + sym]);
                    sym = (sym << 1) | b;
                    offs &= if b == 0 { !mbit } else { mbit };
                }
                sym as u8
            };
            out.push(byte);
            state = after_lit(state);
            continue;
        }
        let len;
        if rc.bit(&mut m.is_rep[state]) == 0 {
            len = rc.len(&mut m.len, ps);
            let ls = (len - 2).min(LEN_STATES - 1);
            let slot = rc.tree(&mut m.slot[ls], 6);
            let d = if slot < 4 {
                slot
            } else {
                let fb = (slot >> 1) - 1;
                let base = (2 | (slot & 1)) << fb;
                if slot < END_POS_MODEL {
                    let at = SPECIAL_AT[slot as usize];
                    base + rc.rtree(&mut m.special[at..], fb)
                } else {
                    let hi = rc.direct(fb - ALIGN_BITS);
                    base + (hi << ALIGN_BITS) + rc.rtree(&mut m.align, ALIGN_BITS)
                }
            };
            let d = d.checked_add(1).ok_or_else(corrupt)?;
            reps = [d, reps[0], reps[1], reps[2]];
            state = after_match(state);
        } else {
            if rc.bit(&mut m.rep_g0[state]) == 0 {
                if rc.bit(&mut m.rep0_long[state * POS_STATES + ps]) == 0 {
                    let r0 = reps[0] as usize;
                    if r0 > pos {
                        return Err(corrupt());
                    }
                    out.push(out[pos - r0]);
                    state = after_short(state);
                    continue;
                }
            } else {
                let d;
                if rc.bit(&mut m.rep_g1[state]) == 0 {
                    d = reps[1];
                } else {
                    if rc.bit(&mut m.rep_g2[state]) == 0 {
                        d = reps[2];
                    } else {
                        d = reps[3];
                        reps[3] = reps[2];
                    }
                    reps[2] = reps[1];
                }
                reps[1] = reps[0];
                reps[0] = d;
            }
            len = rc.len(&mut m.rep_len, ps);
            state = after_rep(state);
        }
        let d = reps[0] as usize;
        if d > pos || len > end - pos {
            return Err(corrupt());
        }
        if d >= len {
            out.extend_from_within(pos - d..pos - d + len);
        } else {
            for i in 0..len {
                let b = out[pos - d + i];
                out.push(b);
            }
        }
    }
    if rc.pos > src.len() {
        return Err(corrupt());
    }
    out.drain(..dict.len());
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Encoding

struct RcEnc {
    low: u64,
    range: u32,
    cache: u8,
    cache_size: u64,
    out: Vec<u8>,
}

impl RcEnc {
    fn new() -> RcEnc {
        RcEnc { low: 0, range: 0xFFFF_FFFF, cache: 0, cache_size: 1, out: Vec::new() }
    }

    #[inline]
    fn bit(&mut self, p: &mut u16, bit: usize) {
        let bound = (self.range >> PROB_BITS) * *p as u32;
        if bit == 0 {
            self.range = bound;
            *p += ((PROB_ONE - *p as u32) >> MOVE_BITS) as u16;
        } else {
            self.low += bound as u64;
            self.range -= bound;
            *p -= *p >> MOVE_BITS;
        }
        while self.range < TOP {
            self.range <<= 8;
            self.shift_low();
        }
    }

    fn direct(&mut self, v: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.range >>= 1;
            if (v >> i) & 1 != 0 {
                self.low += self.range as u64;
            }
            while self.range < TOP {
                self.range <<= 8;
                self.shift_low();
            }
        }
    }

    fn shift_low(&mut self) {
        if (self.low as u32) < 0xFF00_0000 || (self.low >> 32) != 0 {
            let carry = (self.low >> 32) as u8;
            let mut b = self.cache;
            loop {
                self.out.push(b.wrapping_add(carry));
                b = 0xFF;
                self.cache_size -= 1;
                if self.cache_size == 0 {
                    break;
                }
            }
            self.cache = (self.low >> 24) as u8;
        }
        self.cache_size += 1;
        self.low = (self.low & 0x00FF_FFFF) << 8;
    }

    fn finish(mut self) -> Vec<u8> {
        for _ in 0..5 {
            self.shift_low();
        }
        // The first byte is the initial cache, always 0 (the interval never carries into it).
        self.out.remove(0);
        self.out
    }

    fn tree(&mut self, probs: &mut [u16], bits: u32, v: u32) {
        let mut m = 1usize;
        for i in (0..bits).rev() {
            let b = ((v >> i) & 1) as usize;
            self.bit(&mut probs[m], b);
            m = (m << 1) | b;
        }
    }

    fn rtree(&mut self, probs: &mut [u16], bits: u32, v: u32) {
        let mut m = 1usize;
        for i in 0..bits {
            let b = ((v >> i) & 1) as usize;
            self.bit(&mut probs[m], b);
            m = (m << 1) | b;
        }
    }

    fn len(&mut self, lm: &mut LenModel, len: usize, ps: usize) {
        let l = (len - 2) as u32;
        if l < 8 {
            self.bit(&mut lm.choice, 0);
            self.tree(&mut lm.low[ps], 3, l);
        } else if l < 16 {
            self.bit(&mut lm.choice, 1);
            self.bit(&mut lm.choice2, 0);
            self.tree(&mut lm.mid[ps], 3, l - 8);
        } else {
            self.bit(&mut lm.choice, 1);
            self.bit(&mut lm.choice2, 1);
            self.tree(&mut lm.high, 8, l - 16);
        }
    }
}

/// Bit prices in 1/16 bit, by probability >> 4.
struct PriceTable([u32; 128]);

impl PriceTable {
    fn new() -> PriceTable {
        let mut t = [0u32; 128];
        for (i, v) in t.iter_mut().enumerate() {
            let p = (i as f64 * 16.0 + 8.0) / PROB_ONE as f64;
            *v = (-p.log2() * 16.0).round() as u32;
        }
        PriceTable(t)
    }
    #[inline]
    fn p0(&self, p: u16) -> u32 {
        self.0[(p >> 4) as usize]
    }
    #[inline]
    fn p1(&self, p: u16) -> u32 {
        self.0[((PROB_ONE - p as u32) >> 4) as usize]
    }
    #[inline]
    fn bit(&self, p: u16, b: usize) -> u32 {
        if b == 0 { self.p0(p) } else { self.p1(p) }
    }
    fn tree(&self, probs: &[u16], bits: u32, v: u32) -> u32 {
        let mut m = 1usize;
        let mut sum = 0;
        for i in (0..bits).rev() {
            let b = ((v >> i) & 1) as usize;
            sum += self.bit(probs[m], b);
            m = (m << 1) | b;
        }
        sum
    }
    fn rtree(&self, probs: &[u16], bits: u32, v: u32) -> u32 {
        let mut m = 1usize;
        let mut sum = 0;
        for i in 0..bits {
            let b = ((v >> i) & 1) as usize;
            sum += self.bit(probs[m], b);
            m = (m << 1) | b;
        }
        sum
    }
}

const NONE: u32 = u32::MAX;
const HASH_BITS: u32 = 20;

/// Hash chains over 3-byte prefixes (and the last place of each 2-byte one): every position is
/// inserted once, in order; `find` lists the matches that are longer than all the closer ones.
struct MatchFinder<'a> {
    buf: &'a [u8],
    head2: Vec<u32>,
    head3: Vec<u32>,
    prev: Vec<u32>,
    effort: Effort,
    found: Vec<(u32, u32)>, // (length, distance), lengths increasing
}

impl<'a> MatchFinder<'a> {
    fn new(buf: &'a [u8], effort: Effort) -> MatchFinder<'a> {
        MatchFinder {
            buf,
            head2: vec![NONE; 1 << 16],
            head3: vec![NONE; 1 << HASH_BITS],
            prev: vec![NONE; buf.len()],
            effort,
            found: Vec::new(),
        }
    }

    #[inline]
    fn hash3(&self, pos: usize) -> usize {
        let b = &self.buf[pos..pos + 3];
        let v = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        (v.wrapping_mul(2654435761) >> (32 - HASH_BITS)) as usize
    }

    fn insert(&mut self, pos: usize) -> (u32, u32) {
        let rest = self.buf.len() - pos;
        let mut c2 = NONE;
        let mut c3 = NONE;
        if rest >= 2 {
            let h2 = (self.buf[pos] as usize) << 8 | self.buf[pos + 1] as usize;
            c2 = self.head2[h2];
            self.head2[h2] = pos as u32;
        }
        if rest >= 3 {
            let h3 = self.hash3(pos);
            c3 = self.head3[h3];
            self.head3[h3] = pos as u32;
            self.prev[pos] = c3;
        }
        (c2, c3)
    }

    fn skip(&mut self, from: usize, to: usize) {
        for pos in from..to {
            self.insert(pos);
        }
    }

    fn find(&mut self, pos: usize) {
        self.found.clear();
        let (c2, mut c) = self.insert(pos);
        let avail = (self.buf.len() - pos).min(MATCH_MAX);
        if avail < MATCH_MIN {
            return;
        }
        let buf = self.buf;
        let mut best = 1usize;
        if c2 != NONE && pos - c2 as usize <= 0xFFFF {
            let l = common(buf, c2 as usize, pos, avail);
            if l >= 2 {
                best = l;
                self.found.push((l as u32, (pos - c2 as usize) as u32));
            }
        }
        let mut depth = self.effort.depth;
        while c != NONE && depth > 0 && best < avail {
            let cand = c as usize;
            if buf[cand + best] == buf[pos + best] {
                let l = common(buf, cand, pos, avail);
                if l > best {
                    best = l;
                    let d = (pos - cand) as u32;
                    // A closer match of the same start was found by the 2-byte table: keep the
                    // list's lengths increasing.
                    if let Some(last) = self.found.last_mut() {
                        if last.1 >= d {
                            *last = (l as u32, d);
                        } else {
                            self.found.push((l as u32, d));
                        }
                    } else {
                        self.found.push((l as u32, d));
                    }
                    if l >= self.effort.nice {
                        break;
                    }
                }
            }
            c = self.prev[cand];
            depth -= 1;
        }
    }
}

#[inline]
fn common(buf: &[u8], a: usize, b: usize, max: usize) -> usize {
    let x = &buf[a..a + max];
    let y = &buf[b..b + max];
    x.iter().zip(y).take_while(|(p, q)| p == q).count()
}

const OPTS: usize = 4096;
const INF: u32 = u32::MAX;

const K_LIT: u8 = 0;
const K_MATCH: u8 = 1;
const K_REP: u8 = 2; // + index 0..3
const K_SHORT: u8 = 6;

#[derive(Clone, Copy)]
struct Node {
    price: u32,
    prev: u32,
    len: u16,
    kind: u8,
    state: u8,
    dist: u32,
    reps: [u32; 4],
}

const EMPTY: Node = Node { price: INF, prev: 0, len: 0, kind: 0, state: 0, dist: 0, reps: [0; 4] };

struct Encoder<'a> {
    buf: &'a [u8],
    m: Model,
    rc: RcEnc,
    state: usize,
    reps: [u32; 4],
    prices: PriceTable,
    len_prices: Vec<[u32; MATCH_MAX + 1]>,
    rep_len_prices: Vec<[u32; MATCH_MAX + 1]>,
    slot_prices: [[u32; 64]; LEN_STATES],
    dist_prices: [[u32; FULL_DISTANCES]; LEN_STATES],
    align_prices: [u32; 16],
    opt: Vec<Node>,
    cached: Option<(usize, Vec<(u32, u32)>)>,
    effort: Effort,
}

/// Compresses `data`, with `dict` as text the decoder will have before it.
pub fn compress(dict: &[u8], data: &[u8], p: Params, effort: Effort) -> Vec<u8> {
    let mut buf = Vec::with_capacity(dict.len() + data.len());
    buf.extend_from_slice(dict);
    buf.extend_from_slice(data);
    let mut mf = MatchFinder::new(&buf, effort);
    mf.skip(0, dict.len());
    let pos_states = 1 << p.pb;
    let mut e = Encoder {
        buf: &buf,
        m: Model::new(p),
        rc: RcEnc::new(),
        state: 0,
        reps: [1; 4],
        prices: PriceTable::new(),
        len_prices: vec![[0; MATCH_MAX + 1]; pos_states],
        rep_len_prices: vec![[0; MATCH_MAX + 1]; pos_states],
        slot_prices: [[0; 64]; LEN_STATES],
        dist_prices: [[0; FULL_DISTANCES]; LEN_STATES],
        align_prices: [0; 16],
        opt: vec![EMPTY; OPTS + MATCH_MAX + 2],
        cached: None,
        effort,
    };
    let mut pos = dict.len();
    let mut path = Vec::new();
    // The price tables follow the models every few hundred bytes (they are costly to refresh).
    let mut priced = usize::MAX;
    while pos < buf.len() {
        if priced == usize::MAX || pos - priced >= 256 {
            e.update_prices();
            priced = pos;
        }
        e.parse(&mut mf, pos, &mut path);
        for &(kind, len, dist) in &path {
            e.emit(pos, kind, len as usize, dist);
            pos += len as usize;
        }
    }
    e.rc.finish()
}

impl<'a> Encoder<'a> {
    fn pb_mask(&self) -> usize {
        (1usize << self.m.p.pb) - 1
    }

    fn emit(&mut self, pos: usize, kind: u8, len: usize, dist: u32) {
        let ps = pos & self.pb_mask();
        let s = self.state;
        if kind == K_LIT {
            self.rc.bit(&mut self.m.is_match[s * POS_STATES + ps], 0);
            let prev = if pos > 0 { self.buf[pos - 1] } else { 0 };
            let base = self.m.lit_base(pos, prev);
            let probs = &mut self.m.lit[base..base + 0x300];
            let byte = self.buf[pos] as usize;
            if s < 7 {
                self.rc.tree(probs, 8, byte as u32);
            } else {
                let mut mb = self.buf[pos - self.reps[0] as usize] as usize;
                let mut offs = 0x100usize;
                let mut sym = 1usize;
                for i in (0..8).rev() {
                    let b = (byte >> i) & 1;
                    mb <<= 1;
                    let mbit = mb & offs;
                    self.rc.bit(&mut probs[offs + mbit + sym], b);
                    sym = (sym << 1) | b;
                    offs &= if b == 0 { !mbit } else { mbit };
                }
            }
            self.state = after_lit(s);
            return;
        }
        self.rc.bit(&mut self.m.is_match[s * POS_STATES + ps], 1);
        if kind == K_MATCH {
            self.rc.bit(&mut self.m.is_rep[s], 0);
            self.rc.len(&mut self.m.len, len, ps);
            let d = dist - 1;
            let ls = (len - 2).min(LEN_STATES - 1);
            let slot = dist_slot(d);
            self.rc.tree(&mut self.m.slot[ls], 6, slot);
            if slot >= 4 {
                let fb = (slot >> 1) - 1;
                let rem = d - ((2 | (slot & 1)) << fb);
                if slot < END_POS_MODEL {
                    let at = SPECIAL_AT[slot as usize];
                    self.rc.rtree(&mut self.m.special[at..], fb, rem);
                } else {
                    self.rc.direct(rem >> ALIGN_BITS, fb - ALIGN_BITS);
                    self.rc.rtree(&mut self.m.align, ALIGN_BITS, rem & 15);
                }
            }
            self.reps = [dist, self.reps[0], self.reps[1], self.reps[2]];
            self.state = after_match(s);
            return;
        }
        self.rc.bit(&mut self.m.is_rep[s], 1);
        if kind == K_SHORT {
            self.rc.bit(&mut self.m.rep_g0[s], 0);
            self.rc.bit(&mut self.m.rep0_long[s * POS_STATES + ps], 0);
            self.state = after_short(s);
            return;
        }
        let idx = (kind - K_REP) as usize;
        if idx == 0 {
            self.rc.bit(&mut self.m.rep_g0[s], 0);
            self.rc.bit(&mut self.m.rep0_long[s * POS_STATES + ps], 1);
        } else {
            self.rc.bit(&mut self.m.rep_g0[s], 1);
            if idx == 1 {
                self.rc.bit(&mut self.m.rep_g1[s], 0);
            } else {
                self.rc.bit(&mut self.m.rep_g1[s], 1);
                self.rc.bit(&mut self.m.rep_g2[s], (idx == 3) as usize);
            }
        }
        self.reps = reps_after_rep(self.reps, idx);
        self.rc.len(&mut self.m.rep_len, len, ps);
        self.state = after_rep(s);
    }

    fn update_prices(&mut self) {
        let pr = &self.prices;
        for (lm, table) in [(&self.m.len, &mut self.len_prices), (&self.m.rep_len, &mut self.rep_len_prices)] {
            for (ps, t) in table.iter_mut().enumerate() {
                let c0 = pr.p0(lm.choice);
                let c10 = pr.p1(lm.choice) + pr.p0(lm.choice2);
                let c11 = pr.p1(lm.choice) + pr.p1(lm.choice2);
                for len in MATCH_MIN..=MATCH_MAX {
                    let l = (len - 2) as u32;
                    t[len] = if l < 8 {
                        c0 + pr.tree(&lm.low[ps], 3, l)
                    } else if l < 16 {
                        c10 + pr.tree(&lm.mid[ps], 3, l - 8)
                    } else {
                        c11 + pr.tree(&lm.high, 8, l - 16)
                    };
                }
            }
        }
        for ls in 0..LEN_STATES {
            for slot in 0..64u32 {
                let mut v = pr.tree(&self.m.slot[ls], 6, slot);
                if slot >= END_POS_MODEL {
                    v += ((slot >> 1) - 1 - ALIGN_BITS) * 16;
                }
                self.slot_prices[ls][slot as usize] = v;
            }
            for d in 0..FULL_DISTANCES as u32 {
                let slot = dist_slot(d);
                let mut v = self.slot_prices[ls][slot as usize];
                if slot >= 4 {
                    let fb = (slot >> 1) - 1;
                    let rem = d - ((2 | (slot & 1)) << fb);
                    v += pr.rtree(&self.m.special[SPECIAL_AT[slot as usize]..], fb, rem);
                }
                self.dist_prices[ls][d as usize] = v;
            }
        }
        for a in 0..16u32 {
            self.align_prices[a as usize] = pr.rtree(&self.m.align, ALIGN_BITS, a);
        }
    }

    #[inline]
    fn dist_price(&self, dist: u32, len: usize) -> u32 {
        let d = dist - 1;
        let ls = (len - 2).min(LEN_STATES - 1);
        if (d as usize) < FULL_DISTANCES {
            self.dist_prices[ls][d as usize]
        } else {
            self.slot_prices[ls][dist_slot(d) as usize] + self.align_prices[(d & 15) as usize]
        }
    }

    fn lit_price(&self, pos: usize, state: usize, rep0: u32) -> u32 {
        let prev = if pos > 0 { self.buf[pos - 1] } else { 0 };
        let base = self.m.lit_base(pos, prev);
        let probs = &self.m.lit[base..base + 0x300];
        let byte = self.buf[pos] as usize;
        let pr = &self.prices;
        if state < 7 {
            return pr.tree(probs, 8, byte as u32);
        }
        let mut mb = self.buf[pos - rep0 as usize] as usize;
        let mut offs = 0x100usize;
        let mut sym = 1usize;
        let mut sum = 0;
        for i in (0..8).rev() {
            let b = (byte >> i) & 1;
            mb <<= 1;
            let mbit = mb & offs;
            sum += pr.bit(probs[offs + mbit + sym], b);
            sym = (sym << 1) | b;
            offs &= if b == 0 { !mbit } else { mbit };
        }
        sum
    }

    fn rep_price(&self, idx: usize, s: usize, ps: usize) -> u32 {
        let pr = &self.prices;
        let m = &self.m;
        pr.p1(m.is_rep[s])
            + match idx {
                0 => pr.p0(m.rep_g0[s]) + pr.p1(m.rep0_long[s * POS_STATES + ps]),
                1 => pr.p1(m.rep_g0[s]) + pr.p0(m.rep_g1[s]),
                2 => pr.p1(m.rep_g0[s]) + pr.p1(m.rep_g1[s]) + pr.p0(m.rep_g2[s]),
                _ => pr.p1(m.rep_g0[s]) + pr.p1(m.rep_g1[s]) + pr.p1(m.rep_g2[s]),
            }
    }

    /// The cheapest way, by the current models, to code what follows `start`, as far as all
    /// the ways considered meet again (or a long match makes looking further pointless).
    fn parse(&mut self, mf: &mut MatchFinder, start: usize, path: &mut Vec<(u8, u16, u32)>) {
        path.clear();
        let buf = self.buf;
        let matches = match self.cached.take() {
            Some((p, m)) if p == start => m,
            _ => {
                mf.find(start);
                mf.found.clone()
            }
        };
        let avail = (buf.len() - start).min(MATCH_MAX);
        // A long repeat or match is taken as it is.
        let mut best_rep = (0usize, 0usize);
        for (j, &r) in self.reps.iter().enumerate() {
            if r as usize <= start && avail >= MATCH_MIN {
                let l = common(buf, start - r as usize, start, avail);
                if l > best_rep.1 {
                    best_rep = (j, l);
                }
            }
        }
        if best_rep.1 >= self.effort.nice {
            path.push((K_REP + best_rep.0 as u8, best_rep.1 as u16, 0));
            mf.skip(start + 1, start + best_rep.1);
            return;
        }
        if let Some(&(l, d)) = matches.last() {
            if l as usize >= self.effort.nice {
                path.push((K_MATCH, l as u16, d));
                mf.skip(start + 1, start + l as usize);
                return;
            }
        }

        self.opt[0] = Node { price: 0, prev: 0, len: 0, kind: 0, state: self.state as u8, dist: 0, reps: self.reps };
        let mut end = 0usize;
        let mut cur = 0usize;
        let mut matches = matches;
        let pb_mask = self.pb_mask();
        loop {
            let pos = start + cur;
            let node = self.opt[cur];
            let s = node.state as usize;
            let r = node.reps;
            let ps = pos & pb_mask;
            let avail = (buf.len() - pos).min(MATCH_MAX);
            let frozen = cur >= OPTS;
            let relax = |opt: &mut Vec<Node>, end: &mut usize, at: usize, price: u32, kind: u8, len: usize, dist: u32| {
                if at > *end {
                    if frozen {
                        return;
                    }
                    for n in &mut opt[*end + 1..=at] {
                        n.price = INF;
                    }
                    *end = at;
                }
                if price < opt[at].price {
                    opt[at] = Node { price, prev: cur as u32, len: len as u16, kind, state: 0, dist, reps: [0; 4] };
                }
            };
            let pr = &self.prices;
            let im = self.m.is_match[s * POS_STATES + ps];
            let lit = node.price + pr.p0(im) + self.lit_price(pos, s, r[0]);
            relax(&mut self.opt, &mut end, cur + 1, lit, K_LIT, 1, 0);
            let r0 = r[0] as usize;
            if r0 <= pos && buf[pos] == buf[pos - r0] {
                let p = node.price
                    + pr.p1(im)
                    + pr.p1(self.m.is_rep[s])
                    + pr.p0(self.m.rep_g0[s])
                    + pr.p0(self.m.rep0_long[s * POS_STATES + ps]);
                relax(&mut self.opt, &mut end, cur + 1, p, K_SHORT, 1, 0);
            }
            if avail >= MATCH_MIN {
                let base = node.price + pr.p1(im);
                for j in 0..4 {
                    let d = r[j] as usize;
                    if d > pos {
                        continue;
                    }
                    let l = common(buf, pos - d, pos, avail);
                    if l < MATCH_MIN {
                        continue;
                    }
                    let p = base + self.rep_price(j, s, ps);
                    for len in MATCH_MIN..=l {
                        let price = p + self.rep_len_prices[ps][len];
                        relax(&mut self.opt, &mut end, cur + len, price, K_REP + j as u8, len, 0);
                    }
                }
                let p = base + pr.p0(self.m.is_rep[s]);
                let mut len = MATCH_MIN;
                for &(ml, d) in &matches {
                    while len <= ml as usize {
                        let price = p + self.len_prices[ps][len] + self.dist_price(d, len);
                        relax(&mut self.opt, &mut end, cur + len, price, K_MATCH, len, d);
                        len += 1;
                    }
                }
            }

            cur += 1;
            if cur >= end {
                break;
            }
            // The state and repeat distances on the cheapest way here.
            let n = self.opt[cur];
            let p = self.opt[n.prev as usize];
            let ps_ = p.state as usize;
            let (state, reps) = match n.kind {
                K_LIT => (after_lit(ps_), p.reps),
                K_MATCH => (after_match(ps_), [n.dist, p.reps[0], p.reps[1], p.reps[2]]),
                K_SHORT => (after_short(ps_), p.reps),
                k => (after_rep(ps_), reps_after_rep(p.reps, (k - K_REP) as usize)),
            };
            self.opt[cur].state = state as u8;
            self.opt[cur].reps = reps;
            mf.find(start + cur);
            matches.clear();
            matches.extend_from_slice(&mf.found);
            if let Some(&(l, _)) = matches.last() {
                if l as usize >= self.effort.nice {
                    self.cached = Some((start + cur, matches.clone()));
                    break;
                }
            }
        }
        let mut i = cur;
        while i > 0 {
            let n = self.opt[i];
            path.push((n.kind, n.len, n.dist));
            i = n.prev as usize;
        }
        path.reverse();
    }
}

fn reps_after_rep(r: [u32; 4], idx: usize) -> [u32; 4] {
    match idx {
        0 => r,
        1 => [r[1], r[0], r[2], r[3]],
        2 => [r[2], r[0], r[1], r[3]],
        _ => [r[3], r[0], r[1], r[2]],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(dict: &[u8], data: &[u8], p: Params, e: Effort) -> usize {
        let c = compress(dict, data, p, e);
        let d = decompress(dict, &c, data.len(), p).expect("decodes");
        assert!(d == data, "roundtrip differs ({} bytes, {:?})", data.len(), p);
        c.len()
    }

    fn noise(n: usize, seed: u64) -> Vec<u8> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    }

    #[test]
    fn small_and_edge_inputs() {
        for data in [&b""[..], b"a", b"ab", b"aaa", b"abcabcabcabc", &[0u8; 1000], &[255u8; 5000]] {
            for p in [Params::TEXT, Params { lc: 3, lp: 0, pb: 2 }, Params { lc: 8, lp: 2, pb: 4 }] {
                roundtrip(b"", data, p, Effort::BEST);
                roundtrip(b"abcaaa", data, p, Effort::FAST);
            }
        }
    }

    #[test]
    fn noise_and_text() {
        let n = noise(200_000, 7);
        let c = roundtrip(b"", &n, Params::TEXT, Effort::FAST);
        assert!(c < n.len() + n.len() / 50, "noise grew to {c}");
        let mut text = Vec::new();
        for i in 0..20_000 {
            text.extend_from_slice(format!("- Example {} of {}:\n\n`tool --flag {{{{path/to/file{}}}}}`\n\n", i % 97, i % 13, i % 1000).as_bytes());
        }
        let c = roundtrip(b"", &text, Params::TEXT, Effort::BEST);
        assert!(c * 20 < text.len(), "text: {} -> {}", text.len(), c);
        // Long distances (slots with direct bits) and mixes of noise and repeats.
        let mut mix = noise(300_000, 3);
        let copy = mix[1000..90_000].to_vec();
        mix.extend_from_slice(&copy);
        mix.extend_from_slice(&text[..50_000]);
        mix.extend_from_slice(&copy[..5_000]);
        roundtrip(&noise(70_000, 9), &mix, Params::TEXT, Effort::BEST);
    }

    #[test]
    fn dictionary_helps() {
        let dict: Vec<u8> = (0..400).flat_map(|i| format!("line {i}: the quick brown fox\n").into_bytes()).collect();
        let data: Vec<u8> = (100..300).flat_map(|i| format!("line {i}: the quick brown fox\n").into_bytes()).collect();
        let without = roundtrip(b"", &data, Params::TEXT, Effort::BEST);
        let with = roundtrip(&dict, &data, Params::TEXT, Effort::BEST);
        assert!(with * 4 < without, "with {with}, without {without}");
    }

    #[test]
    fn corrupt_input_is_an_error_not_a_panic() {
        let text: Vec<u8> = (0..3000).flat_map(|i| format!("{i} ").into_bytes()).collect();
        let c = compress(b"", &text, Params::TEXT, Effort::FAST);
        for i in (0..c.len()).step_by(7) {
            let mut bad = c.clone();
            bad[i] ^= 0x5A;
            let _ = decompress(b"", &bad, text.len(), Params::TEXT);
        }
        let _ = decompress(b"", &c[..c.len() / 2], text.len(), Params::TEXT);
        let _ = decompress(b"", &[], 100, Params::TEXT);
    }

    #[test]
    fn params_byte() {
        for lc in 0..=8 {
            for lp in 0..=4 {
                for pb in 0..=4 {
                    let p = Params { lc, lp, pb };
                    assert_eq!(Params::from_byte(p.to_byte()), Some(p));
                }
            }
        }
    }
}

#[cfg(test)]
mod measure {
    #[test]
    #[ignore]
    fn sample() {
        let f = std::env::var("GA_SAMPLE").unwrap();
        let data = std::fs::read(&f).unwrap();
        let t = std::time::Instant::now();
        let c = super::compress(b"", &data, super::Params::TEXT, super::Effort::BEST);
        let te = t.elapsed();
        let t = std::time::Instant::now();
        let d = super::decompress(b"", &c, data.len(), super::Params::TEXT).unwrap();
        let td = t.elapsed();
        assert!(d == data);
        println!("{f}: {} -> {} ({te:.2?} enc, {td:.2?} dec)", data.len(), c.len());
    }
}
