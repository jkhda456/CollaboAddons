//! TCP-stream backends: a raw socket (bytes straight through), telnet (RFC 854/855, with the
//! option handshake and NAWS window size), and rlogin (RFC 1282's initial handshake). All three
//! are one TCP connection; only the byte handling above it differs.

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::time::Duration;

use super::{Backend, Target};
use crate::sys::{self, FdStream};
use crate::{err, Result};

pub struct Tcp {
    socket: FdStream,
    mode: Mode,
    open: bool,
    cols: u16,
    rows: u16,
    /// Telnet responses / rlogin handshake waiting to go out.
    outbox: Vec<u8>,
}

enum Mode {
    Raw,
    Telnet(Telnet),
    Rlogin,
}

impl Tcp {
    /// `user_override`: the remote user for rlogin's handshake when given apart from the target.
    pub fn connect(target: &Target, cols: u16, rows: u16, term: &str, user_override: Option<&str>, timeout: Duration) -> Result<Tcp> {
        let (host, port) = match target {
            Target::Raw { host, port } | Target::Telnet { host, port, .. } | Target::Rlogin { host, port, .. } => (host.clone(), *port),
            _ => return Err(err!("not a TCP target")),
        };
        let stream = sys::tcp_connect(&host, port, timeout)?;
        stream.set_nodelay(true).ok();
        let socket = FdStream::new(sys::tcp_into_fd(stream));
        socket.set_nonblocking(true)?;
        let mut tcp = Tcp { socket, mode: Mode::Raw, open: true, cols, rows, outbox: Vec::new() };
        match target {
            Target::Raw { .. } => {}
            Target::Telnet { .. } => {
                tcp.mode = Mode::Telnet(Telnet::new());
                tcp.outbox = Telnet::greeting();
                tcp.flush_outbox()?;
            }
            Target::Rlogin { user, .. } => {
                let user = user_override.or(user.as_deref());
                tcp.mode = Mode::Rlogin;
                let local = std::env::var("USER").unwrap_or_else(|_| "root".into());
                let remote = user.map(str::to_string).unwrap_or_else(|| local.clone());
                // \0 localuser\0 remoteuser\0 term/speed\0
                let mut hs = vec![0u8];
                hs.extend_from_slice(local.as_bytes());
                hs.push(0);
                hs.extend_from_slice(remote.as_bytes());
                hs.push(0);
                hs.extend_from_slice(format!("{term}/38400").as_bytes());
                hs.push(0);
                tcp.outbox = hs;
                tcp.flush_outbox()?;
            }
            _ => return Err(err!("not a TCP target")),
        }
        Ok(tcp)
    }

    fn flush_outbox(&mut self) -> Result<()> {
        while !self.outbox.is_empty() {
            match self.socket.write(&self.outbox) {
                Ok(0) => return Err(err!("the connection closed while sending")),
                Ok(n) => {
                    self.outbox.drain(..n);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    sys::wait_readable(self.socket.as_raw_fd(), 20).ok();
                }
                Err(e) => return Err(err!("sending: {e}")),
            }
        }
        Ok(())
    }
}

impl Backend for Tcp {
    fn fd(&self) -> std::os::fd::RawFd {
        self.socket.as_raw_fd()
    }

    fn read(&mut self) -> Result<Vec<u8>> {
        let mut buf = [0u8; 16384];
        let raw = match self.socket.read(&mut buf) {
            Ok(0) => {
                self.open = false;
                return Ok(Vec::new());
            }
            Ok(n) => &buf[..n],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(Vec::new()),
            Err(e) => return Err(err!("reading: {e}")),
        };
        let out = match &mut self.mode {
            Mode::Raw | Mode::Rlogin => raw.to_vec(),
            Mode::Telnet(t) => {
                let (data, reply) = t.filter(raw, self.cols, self.rows);
                if !reply.is_empty() {
                    self.outbox.extend_from_slice(&reply);
                    self.flush_outbox()?;
                }
                data
            }
        };
        Ok(out)
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        // Telnet: a literal 0xFF in input must be doubled.
        let payload: Vec<u8> = if matches!(self.mode, Mode::Telnet(_)) && data.contains(&0xff) {
            let mut v = Vec::with_capacity(data.len() + 4);
            for &b in data {
                v.push(b);
                if b == 0xff {
                    v.push(0xff);
                }
            }
            v
        } else {
            data.to_vec()
        };
        self.outbox.extend_from_slice(&payload);
        self.flush_outbox()
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.cols = cols;
        self.rows = rows;
        if matches!(self.mode, Mode::Telnet(_)) {
            let naws = Telnet::naws(cols, rows);
            self.outbox.extend_from_slice(&naws);
            self.flush_outbox()?;
        }
        Ok(())
    }

    fn control(&mut self, op: &super::Control) -> Result<String> {
        match (op, &self.mode) {
            // Telnet's BRK (RFC 854): what a terminal's Break key sends.
            (super::Control::Break(_), Mode::Telnet(_)) => {
                self.outbox.extend_from_slice(&[IAC, BRK]);
                self.flush_outbox()?;
                Ok("sent telnet BRK".into())
            }
            (super::Control::Break(_), _) => Err(err!("a {} session cannot send a break (serial, telnet and ssh can)", self.kind())),
            (super::Control::Line { .. }, _) => Err(err!("line settings are for serial sessions, not {}", self.kind())),
        }
    }

    fn is_open(&self) -> bool {
        self.open
    }

    fn close(&mut self) -> Result<()> {
        self.open = false;
        // SAFETY: shutting down our own socket.
        unsafe { libc::shutdown(self.socket.as_raw_fd(), libc::SHUT_RDWR) };
        Ok(())
    }

    fn kind(&self) -> &'static str {
        match self.mode {
            Mode::Raw => "raw",
            Mode::Telnet(_) => "telnet",
            Mode::Rlogin => "rlogin",
        }
    }
}

// --- Telnet ---------------------------------------------------------------------------------

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const BRK: u8 = 243;
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_TTYPE: u8 = 24;
const OPT_NAWS: u8 = 31;

#[derive(PartialEq)]
enum TState {
    Data,
    Iac,
    Opt(u8),
    Sub,
    SubIac,
}

/// The telnet option state machine: strips commands from the byte stream and answers them.
struct Telnet {
    state: TState,
    sub: Vec<u8>,
    naws_agreed: bool,
}

impl Telnet {
    fn new() -> Telnet {
        Telnet { state: TState::Data, sub: Vec::new(), naws_agreed: false }
    }

    /// What we offer as soon as we connect: suppress go-ahead both ways, and that we will send a
    /// terminal type and a window size.
    fn greeting() -> Vec<u8> {
        vec![IAC, WILL, OPT_SGA, IAC, DO, OPT_SGA, IAC, WILL, OPT_TTYPE, IAC, WILL, OPT_NAWS]
    }

    fn naws(cols: u16, rows: u16) -> Vec<u8> {
        let [wh, wl] = cols.to_be_bytes();
        let [hh, hl] = rows.to_be_bytes();
        // Any 0xff inside must be doubled, but sizes rarely contain it; handle anyway.
        let mut body = vec![wh, wl, hh, hl];
        let mut escaped = Vec::new();
        for b in body.drain(..) {
            escaped.push(b);
            if b == IAC {
                escaped.push(IAC);
            }
        }
        let mut out = vec![IAC, SB, OPT_NAWS];
        out.extend_from_slice(&escaped);
        out.extend_from_slice(&[IAC, SE]);
        out
    }

    /// Runs the input through the machine; returns (terminal data, bytes to send back).
    fn filter(&mut self, input: &[u8], cols: u16, rows: u16) -> (Vec<u8>, Vec<u8>) {
        let mut data = Vec::with_capacity(input.len());
        let mut reply = Vec::new();
        for &b in input {
            match self.state {
                TState::Data => {
                    if b == IAC {
                        self.state = TState::Iac;
                    } else {
                        data.push(b);
                    }
                }
                TState::Iac => match b {
                    IAC => {
                        data.push(IAC);
                        self.state = TState::Data;
                    }
                    SB => {
                        self.sub.clear();
                        self.state = TState::Sub;
                    }
                    DO | DONT | WILL | WONT => self.state = TState::Opt(b),
                    _ => self.state = TState::Data, // other commands (GA, NOP, ...) ignored
                },
                TState::Opt(cmd) => {
                    self.answer(cmd, b, cols, rows, &mut reply);
                    self.state = TState::Data;
                }
                TState::Sub => {
                    if b == IAC {
                        self.state = TState::SubIac;
                    } else {
                        self.sub.push(b);
                    }
                }
                TState::SubIac => {
                    if b == SE {
                        self.subnegotiation(cols, rows, &mut reply);
                        self.state = TState::Data;
                    } else {
                        // IAC IAC inside a subnegotiation is a literal 0xff.
                        self.sub.push(b);
                        self.state = TState::Sub;
                    }
                }
            }
        }
        (data, reply)
    }

    fn answer(&mut self, cmd: u8, opt: u8, cols: u16, rows: u16, reply: &mut Vec<u8>) {
        match cmd {
            DO => {
                let ok = matches!(opt, OPT_SGA | OPT_TTYPE | OPT_NAWS);
                reply.extend_from_slice(&[IAC, if ok { WILL } else { WONT }, opt]);
                if ok && opt == OPT_NAWS {
                    self.naws_agreed = true;
                    reply.extend_from_slice(&Telnet::naws(cols, rows));
                }
            }
            WILL => {
                // Let the server echo and suppress go-ahead; refuse the rest.
                let ok = matches!(opt, OPT_ECHO | OPT_SGA);
                reply.extend_from_slice(&[IAC, if ok { DO } else { DONT }, opt]);
            }
            WONT => reply.extend_from_slice(&[IAC, DONT, opt]),
            DONT => reply.extend_from_slice(&[IAC, WONT, opt]),
            _ => {}
        }
    }

    fn subnegotiation(&mut self, _cols: u16, _rows: u16, reply: &mut Vec<u8>) {
        // TTYPE SEND (option 24, command 1): answer with our terminal type.
        if self.sub.len() >= 2 && self.sub[0] == OPT_TTYPE && self.sub[1] == 1 {
            let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".into());
            reply.extend_from_slice(&[IAC, SB, OPT_TTYPE, 0]); // 0 = IS
            reply.extend_from_slice(term.as_bytes());
            reply.extend_from_slice(&[IAC, SE]);
        }
    }
}
