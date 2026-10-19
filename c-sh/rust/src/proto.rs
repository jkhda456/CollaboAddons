//! The tiny line protocol between the `c-sh` command and its daemon, over a Unix socket. A request
//! is a count of words, then each word as a decimal length and its bytes. A reply is
//! "STATUS LENGTH\n" then LENGTH bytes. The same shape the `web` and `gui` tools use.

use std::io::{Read, Write};

use crate::{err, Result};

pub fn write_request(out: &mut impl Write, words: &[String]) -> Result<()> {
    write!(out, "{}\n", words.len())?;
    for w in words {
        write!(out, "{}\n", w.len())?;
        out.write_all(w.as_bytes())?;
    }
    out.flush()?;
    Ok(())
}

pub fn read_request(inp: &mut impl Read) -> Result<Vec<String>> {
    let count: usize = read_line_number(inp)?;
    if count > 10000 {
        return Err(err!("too many words in a request"));
    }
    let mut words = Vec::with_capacity(count);
    for _ in 0..count {
        let len: usize = read_line_number(inp)?;
        if len > 64 * 1024 * 1024 {
            return Err(err!("a request word is too long"));
        }
        let mut buf = vec![0u8; len];
        inp.read_exact(&mut buf)?;
        words.push(String::from_utf8_lossy(&buf).into_owned());
    }
    Ok(words)
}

pub fn write_reply(out: &mut impl Write, status: i32, body: &[u8]) -> Result<()> {
    write!(out, "{} {}\n", status, body.len())?;
    out.write_all(body)?;
    out.flush()?;
    Ok(())
}

/// Reads a reply: (status, body).
pub fn read_reply(inp: &mut impl Read) -> Result<(i32, Vec<u8>)> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = inp.read(&mut byte)?;
        if n == 0 {
            return Err(err!("the daemon closed the connection"));
        }
        if byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
        if line.len() > 64 {
            return Err(err!("a malformed reply header"));
        }
    }
    let text = String::from_utf8_lossy(&line);
    let mut parts = text.split(' ');
    let status: i32 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| err!("bad reply status"))?;
    let len: usize = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| err!("bad reply length"))?;
    let mut body = vec![0u8; len];
    inp.read_exact(&mut body)?;
    Ok((status, body))
}

fn read_line_number(inp: &mut impl Read) -> Result<usize> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = inp.read(&mut byte)?;
        if n == 0 {
            return Err(err!("the connection ended mid-request"));
        }
        if byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
        if line.len() > 20 {
            return Err(err!("a malformed length"));
        }
    }
    String::from_utf8_lossy(&line).trim().parse().map_err(|_| err!("not a number: {:?}", String::from_utf8_lossy(&line)))
}
