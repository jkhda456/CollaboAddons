//! A session channel on top of the transport (RFC 4254): open it, ask for a pseudo-terminal, then
//! a shell, a command or a subsystem, and carry its bytes. The window is managed here.

use super::transport::Transport;
use super::wire::{Reader, Writer};
use super::msg;
use crate::{err, Result};

const INITIAL_WINDOW: u32 = 2 * 1024 * 1024;
const MAX_PACKET: u32 = 32 * 1024;

/// What to start on the server once the channel is open.
pub enum Command {
    Shell,
    Exec(String),
}

/// An open session channel. Data the server sends arrives through [Channel::read]; input goes
/// through [Channel::write]. The transport is owned here (one connection, one channel).
pub struct Channel<'a> {
    pub transport: Transport<'a>,
    remote_id: u32,
    /// How much the server may still send us before a window adjustment.
    local_window: u32,
    /// How much we may still send the server.
    remote_window: u32,
    remote_max_packet: u32,
    eof: bool,
    closed: bool,
    sent_close: bool,
    exit_status: Option<i32>,
    exit_signal: Option<String>,
    /// Output that arrived while reading for another purpose (e.g. the open handshake).
    pending: Vec<u8>,
}

impl<'a> Channel<'a> {
    /// Opens a session channel and starts `command`, with a pty of the given size when `pty` is
    /// set (its terminal type and the modes a line editor needs).
    pub fn open(mut transport: Transport<'a>, command: Command, pty: Option<(u16, u16, &str)>, env: &[(String, String)]) -> Result<Channel<'a>> {
        // Our only channel is number 0.
        let mut open = Writer::msg(msg::CHANNEL_OPEN);
        open.string("session").u32(0).u32(INITIAL_WINDOW).u32(MAX_PACKET);
        transport.send_packet(&open.finish())?;

        let (remote_id, remote_window, remote_max_packet) = loop {
            let p = transport.recv_packet()?;
            match p.first().copied() {
                Some(msg::CHANNEL_OPEN_CONFIRMATION) => {
                    let mut r = Reader::new(&p[1..]);
                    let _local = r.u32()?;
                    let remote = r.u32()?;
                    let window = r.u32()?;
                    let max = r.u32()?;
                    break (remote, window, max.max(1024));
                }
                Some(msg::CHANNEL_OPEN_FAILURE) => {
                    let mut r = Reader::new(&p[1..]);
                    let _local = r.u32()?;
                    let _code = r.u32()?;
                    return Err(err!("the server refused the session: {}", r.text().unwrap_or_default()));
                }
                _ => continue,
            }
        };

        let mut ch = Channel {
            transport,
            remote_id,
            local_window: INITIAL_WINDOW,
            remote_window,
            remote_max_packet,
            eof: false,
            closed: false,
            sent_close: false,
            exit_status: None,
            exit_signal: None,
            pending: Vec::new(),
        };

        for (name, value) in env {
            let mut w = Writer::msg(msg::CHANNEL_REQUEST);
            w.u32(ch.remote_id).string("env").bool(false).string(name).string(value);
            ch.transport.send_packet(&w.finish())?;
        }

        if let Some((cols, rows, term)) = pty {
            ch.request_pty(cols, rows, term)?;
        }

        let (kind, arg): (&str, Option<&str>) = match &command {
            Command::Shell => ("shell", None),
            Command::Exec(c) => ("exec", Some(c)),
        };
        let mut req = Writer::msg(msg::CHANNEL_REQUEST);
        req.u32(ch.remote_id).string(kind).bool(true);
        if let Some(a) = arg {
            req.string(a);
        }
        let payload = req.finish();
        ch.transport.send_packet(&payload)?;
        ch.await_request_reply(kind)?;
        Ok(ch)
    }

    fn request_pty(&mut self, cols: u16, rows: u16, term: &str) -> Result<()> {
        let mut w = Writer::msg(msg::CHANNEL_REQUEST);
        w.u32(self.remote_id).string("pty-req").bool(false).string(term);
        w.u32(cols as u32).u32(rows as u32).u32(0).u32(0);
        w.string(terminal_modes());
        self.transport.send_packet(&w.finish())?;
        Ok(())
    }

    /// Waits for the success/failure of the shell/exec request, keeping any data that arrives.
    fn await_request_reply(&mut self, what: &str) -> Result<()> {
        loop {
            let p = self.transport.recv_packet()?;
            match p.first().copied() {
                Some(msg::CHANNEL_SUCCESS) => return Ok(()),
                Some(msg::CHANNEL_FAILURE) => return Err(err!("the server would not start {what}")),
                _ => self.handle_other(&p)?,
            }
        }
    }

    /// A break of `ms` milliseconds (RFC 4335): the server passes it to the session's terminal.
    pub fn send_break(&mut self, ms: u32) -> Result<()> {
        let mut w = Writer::msg(msg::CHANNEL_REQUEST);
        w.u32(self.remote_id).string("break").bool(false).u32(ms);
        self.transport.send_packet(&w.finish())
    }

    /// Sets the remote terminal's size.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        let mut w = Writer::msg(msg::CHANNEL_REQUEST);
        w.u32(self.remote_id).string("window-change").bool(false);
        w.u32(cols as u32).u32(rows as u32).u32(0).u32(0);
        self.transport.send_packet(&w.finish())
    }

    /// Sends input to the server, respecting its window and packet size.
    pub fn write(&mut self, mut data: &[u8]) -> Result<()> {
        if self.closed {
            return Err(err!("the channel is closed"));
        }
        while !data.is_empty() {
            while self.remote_window == 0 {
                // Block until the server grows our window (it is reading our input).
                let p = self.transport.recv_packet()?;
                self.handle_other(&p)?;
                if self.closed {
                    return Err(err!("the channel closed while sending"));
                }
            }
            let chunk = data.len().min(self.remote_window as usize).min(self.remote_max_packet as usize);
            let mut w = Writer::msg(msg::CHANNEL_DATA);
            w.u32(self.remote_id).string(&data[..chunk]);
            self.transport.send_packet(&w.finish())?;
            self.remote_window -= chunk as u32;
            data = &data[chunk..];
        }
        Ok(())
    }

    /// Reads one transport packet and returns any channel output it carried (empty for control
    /// messages). Call it when the socket is readable. Returns an empty vector at EOF too; check
    /// [Channel::is_open].
    pub fn read(&mut self) -> Result<Vec<u8>> {
        if !self.pending.is_empty() {
            return Ok(std::mem::take(&mut self.pending));
        }
        if self.closed {
            return Ok(Vec::new());
        }
        let p = self.transport.recv_packet()?;
        self.take_data(&p)
    }

    /// Interprets a packet, returning channel data (CHANNEL_DATA / EXTENDED_DATA) or empty.
    fn take_data(&mut self, p: &[u8]) -> Result<Vec<u8>> {
        match p.first().copied() {
            Some(msg::CHANNEL_DATA) => {
                let mut r = Reader::new(&p[1..]);
                let _id = r.u32()?;
                let data = r.string()?.to_vec();
                self.consume_window(data.len() as u32)?;
                Ok(data)
            }
            Some(msg::CHANNEL_EXTENDED_DATA) => {
                let mut r = Reader::new(&p[1..]);
                let _id = r.u32()?;
                let _type = r.u32()?;
                let data = r.string()?.to_vec();
                self.consume_window(data.len() as u32)?;
                Ok(data) // stderr folded into the terminal stream, as a real terminal shows it
            }
            _ => {
                self.handle_other(p)?;
                Ok(Vec::new())
            }
        }
    }

    /// Handles a non-data packet (window growth, requests, eof, close, exit status).
    fn handle_other(&mut self, p: &[u8]) -> Result<()> {
        match p.first().copied() {
            Some(msg::CHANNEL_WINDOW_ADJUST) => {
                let mut r = Reader::new(&p[1..]);
                let _id = r.u32()?;
                self.remote_window = self.remote_window.saturating_add(r.u32()?);
            }
            Some(msg::CHANNEL_DATA) | Some(msg::CHANNEL_EXTENDED_DATA) => {
                // Data arriving while we waited for something else; keep it for the next read.
                let extra = self.take_data(p)?;
                self.pending.extend_from_slice(&extra);
            }
            Some(msg::CHANNEL_EOF) => self.eof = true,
            Some(msg::CHANNEL_CLOSE) => {
                self.closed = true;
                if !self.sent_close {
                    let mut w = Writer::msg(msg::CHANNEL_CLOSE);
                    w.u32(self.remote_id);
                    let _ = self.transport.send_packet(&w.finish());
                    self.sent_close = true;
                }
            }
            Some(msg::CHANNEL_REQUEST) => {
                let mut r = Reader::new(&p[1..]);
                let _id = r.u32()?;
                let kind = r.text()?;
                let want_reply = r.bool().unwrap_or(false);
                match kind.as_str() {
                    "exit-status" => self.exit_status = Some(r.u32()? as i32),
                    "exit-signal" => self.exit_signal = Some(r.text()?),
                    _ => {}
                }
                let _ = want_reply; // channel requests to the client never need a reply here
            }
            _ => {}
        }
        Ok(())
    }

    /// Accounts for data the server sent and tops the window back up when it runs low.
    fn consume_window(&mut self, n: u32) -> Result<()> {
        self.local_window = self.local_window.saturating_sub(n);
        if self.local_window < INITIAL_WINDOW / 2 {
            let grow = INITIAL_WINDOW - self.local_window;
            let mut w = Writer::msg(msg::CHANNEL_WINDOW_ADJUST);
            w.u32(self.remote_id).u32(grow);
            self.transport.send_packet(&w.finish())?;
            self.local_window += grow;
        }
        Ok(())
    }


    /// Asks the server to close the channel.
    pub fn close(&mut self) -> Result<()> {
        if self.sent_close {
            return Ok(());
        }
        let mut w = Writer::msg(msg::CHANNEL_CLOSE);
        w.u32(self.remote_id);
        self.sent_close = true;
        self.transport.send_packet(&w.finish())
    }

    pub fn is_open(&self) -> bool {
        !self.closed
    }

    /// Data or packets waiting that a `poll` on the socket would not report.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty() || (!self.closed && self.transport.has_buffered())
    }


    /// The command's exit status, once it has ended (a signal is reported as 128 + its number is
    /// not known here, so a signalled exit gives 255 unless a status also came).
    pub fn exit_status(&self) -> Option<i32> {
        self.exit_status.or(self.exit_signal.as_ref().map(|_| 255))
    }


}

/// The terminal modes we ask for in a pty-req: sane defaults (echo and processing on, UTF-8-safe),
/// SSH's encoded opcode/value list ending with TTY_OP_END (0).
fn terminal_modes() -> Vec<u8> {
    // (opcode, value) pairs; opcodes from RFC 4254 §8.
    const TTY_OP_END: u8 = 0;
    const VINTR: u8 = 1;
    const VQUIT: u8 = 2;
    const VERASE: u8 = 3;
    const ECHO: u8 = 53;
    const ICANON: u8 = 52;
    const ISIG: u8 = 50;
    const ICRNL: u8 = 40;
    const OPOST: u8 = 70;
    const ONLCR: u8 = 72;
    const IUTF8: u8 = 42;
    let pairs: &[(u8, u32)] = &[
        (VINTR, 3),
        (VQUIT, 28),
        (VERASE, 127),
        (ISIG, 1),
        (ICANON, 1),
        (ECHO, 1),
        (ICRNL, 1),
        (OPOST, 1),
        (ONLCR, 1),
        (IUTF8, 1),
    ];
    let mut out = Vec::new();
    for (op, val) in pairs {
        out.push(*op);
        out.extend_from_slice(&val.to_be_bytes());
    }
    out.push(TTY_OP_END);
    out
}
