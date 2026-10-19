//! A client for an ssh-agent over a Unix socket — in the sandbox this is `$SSH_AUTH_SOCK`, which
//! the engine carries to the host's agent (collabo-sshagent → vsock 1082), so keys never leave the
//! user's computer. Only listing keys and signing are used, which is all that bridge passes.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use super::hostkey::PublicKey;
use super::wire::{Reader, Writer};
use crate::{err, Result};

const REQUEST_IDENTITIES: u8 = 11;
const IDENTITIES_ANSWER: u8 = 12;
const SIGN_REQUEST: u8 = 13;
const SIGN_RESPONSE: u8 = 14;

pub const RSA_SHA2_256: u32 = 2;
pub const RSA_SHA2_512: u32 = 4;

/// One key the agent holds.
pub struct AgentKey {
    pub blob: Vec<u8>,
    pub key: PublicKey,
    pub comment: String,
}

pub struct Agent {
    socket: UnixStream,
}

impl Agent {
    /// Connects to `$SSH_AUTH_SOCK`, or `path` when given.
    pub fn connect(path: Option<&str>) -> Result<Agent> {
        let path = match path {
            Some(p) => p.to_string(),
            None => std::env::var("SSH_AUTH_SOCK").map_err(|_| err!("no ssh-agent (SSH_AUTH_SOCK is not set)"))?,
        };
        let socket = UnixStream::connect(&path).map_err(|e| err!("cannot reach the ssh-agent at {path}: {e}"))?;
        Ok(Agent { socket })
    }

    fn roundtrip(&mut self, message: &[u8]) -> Result<Vec<u8>> {
        self.socket.write_all(&(message.len() as u32).to_be_bytes())?;
        self.socket.write_all(message)?;
        self.socket.flush()?;
        let mut len = [0u8; 4];
        self.socket.read_exact(&mut len)?;
        let n = u32::from_be_bytes(len) as usize;
        if n == 0 || n > 256 * 1024 {
            return Err(err!("the ssh-agent sent a {n}-byte reply"));
        }
        let mut body = vec![0u8; n];
        self.socket.read_exact(&mut body)?;
        Ok(body)
    }

    /// The keys the agent offers (ones c-sh understands).
    pub fn identities(&mut self) -> Result<Vec<AgentKey>> {
        let reply = self.roundtrip(&[REQUEST_IDENTITIES])?;
        let mut r = Reader::new(&reply);
        if r.u8()? != IDENTITIES_ANSWER {
            return Err(err!("the ssh-agent refused to list its keys"));
        }
        let count = r.u32()?;
        let mut keys = Vec::new();
        for _ in 0..count {
            let blob = r.string()?.to_vec();
            let comment = r.text()?;
            if let Ok(key) = PublicKey::parse(&blob) {
                keys.push(AgentKey { blob, key, comment });
            }
        }
        Ok(keys)
    }

    /// Signs `data` with the key, asking for `flags` (rsa-sha2 selection). Returns the SSH
    /// signature blob (algorithm + signature), as the agent formats it.
    pub fn sign(&mut self, key_blob: &[u8], data: &[u8], flags: u32) -> Result<Vec<u8>> {
        let mut w = Writer::msg(SIGN_REQUEST);
        w.string(key_blob).string(data).u32(flags);
        let reply = self.roundtrip(&w.finish())?;
        let mut r = Reader::new(&reply);
        if r.u8()? != SIGN_RESPONSE {
            return Err(err!("the ssh-agent refused to sign (the key may not be loaded, or the app denied it)"));
        }
        Ok(r.string()?.to_vec())
    }
}
