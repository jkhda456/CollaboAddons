//! SSH's data types (RFC 4251 section 5): reading them out of a message and writing them into one.

use crate::{err, Result};

/// Reads SSH values from a message, front to back.
pub struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, at: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.at
    }


    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(err!("a message ended early (wanted {n} more bytes, {} left)", self.remaining()));
        }
        let r = &self.data[self.at..self.at + n];
        self.at += n;
        Ok(r)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.bytes(4)?.try_into().unwrap()))
    }


    /// A `string`: a length, then that many bytes.
    pub fn string(&mut self) -> Result<&'a [u8]> {
        let n = self.u32()? as usize;
        self.bytes(n)
    }

    /// A `string` that should be text (UTF-8; anything else is shown with replacements).
    pub fn text(&mut self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.string()?).into_owned())
    }

    /// An `mpint` as an unsigned big-endian number without leading zeros (negative ones refused).
    pub fn mpint(&mut self) -> Result<&'a [u8]> {
        let mut v = self.string()?;
        if v.first().is_some_and(|b| b & 0x80 != 0) {
            return Err(err!("a negative number where a positive one belongs"));
        }
        while v.first() == Some(&0) {
            v = &v[1..];
        }
        Ok(v)
    }

    pub fn name_list(&mut self) -> Result<Vec<String>> {
        let s = self.text()?;
        Ok(if s.is_empty() { Vec::new() } else { s.split(',').map(str::to_string).collect() })
    }
}

/// Builds an SSH message.
#[derive(Default, Clone)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Writer {
        Writer::default()
    }

    /// A message starting with its type.
    pub fn msg(kind: u8) -> Writer {
        Writer { buf: vec![kind] }
    }

    pub fn u8(&mut self, v: u8) -> &mut Writer {
        self.buf.push(v);
        self
    }

    pub fn bool(&mut self, v: bool) -> &mut Writer {
        self.buf.push(v as u8);
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Writer {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }


    pub fn raw(&mut self, v: &[u8]) -> &mut Writer {
        self.buf.extend_from_slice(v);
        self
    }

    pub fn string(&mut self, v: impl AsRef<[u8]>) -> &mut Writer {
        let v = v.as_ref();
        self.u32(v.len() as u32);
        self.buf.extend_from_slice(v);
        self
    }

    /// An unsigned big-endian number as an `mpint` (a zero byte in front when the top bit is set).
    pub fn mpint(&mut self, v: &[u8]) -> &mut Writer {
        let mut v = v;
        while v.first() == Some(&0) {
            v = &v[1..];
        }
        let pad = v.first().is_some_and(|b| b & 0x80 != 0);
        self.u32((v.len() + pad as usize) as u32);
        if pad {
            self.buf.push(0);
        }
        self.buf.extend_from_slice(v);
        self
    }

    pub fn name_list(&mut self, names: &[&str]) -> &mut Writer {
        self.string(names.join(","))
    }

    pub fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
}

pub fn mpint(v: &[u8]) -> Vec<u8> {
    Writer::new().mpint(v).finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4251_examples() {
        assert_eq!(mpint(&[]), [0, 0, 0, 0]);
        assert_eq!(mpint(&[0x09, 0xa3, 0x78, 0xf9, 0xb2, 0xe3, 0x32, 0xa7]), [0, 0, 0, 8, 0x09, 0xa3, 0x78, 0xf9, 0xb2, 0xe3, 0x32, 0xa7]);
        assert_eq!(mpint(&[0x80]), [0, 0, 0, 2, 0, 0x80]);
        assert_eq!(mpint(&[0, 0, 0x7f]), [0, 0, 0, 1, 0x7f]);
        let mut w = Writer::msg(20);
        w.name_list(&["zlib", "none"]).bool(true).u32(7).string(b"ab");
        let m = w.finish();
        let mut r = Reader::new(&m[1..]);
        assert_eq!(r.name_list().unwrap(), ["zlib", "none"]);
        assert!(r.bool().unwrap());
        assert_eq!(r.u32().unwrap(), 7);
        assert_eq!(r.string().unwrap(), b"ab");
        assert!(r.u8().is_err());
        let e = mpint(&[0x80]);
        assert_eq!(Reader::new(&e).mpint().unwrap(), [0x80]);
    }
}
