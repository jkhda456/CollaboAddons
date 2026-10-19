//! A backend is one live terminal connection, whatever carries it: SSH, telnet, rlogin, a raw TCP
//! socket, a local program in a pty, or a serial line. The session layer ([crate::session]) pumps
//! bytes between a backend and the terminal state, and sends it input — it does not care which.

pub mod local;
pub mod serial;
pub mod ssh;
pub mod tcp;

use std::os::fd::RawFd;

use crate::{err, Result};

pub use serial::LineOpts;

/// Line-level controls, for the backends that have them.
pub enum Control {
    /// A break of about this many milliseconds (serial; telnet's BRK; SSH's break request).
    Break(u32),
    /// Serial line settings and the modem lines; answered with the line's state.
    Line { line: LineOpts, dtr: Option<bool>, rts: Option<bool> },
}

/// One terminal connection. Reads and writes are the terminal's bytes (UTF-8 for display); the
/// session loop waits on [Backend::fd] and calls [Backend::read] when it is readable.
pub trait Backend: Send {
    /// A file descriptor that becomes readable when there is output (for `poll`).
    fn fd(&self) -> RawFd;

    /// Reads whatever output is ready, as terminal bytes. An empty result can mean a control
    /// message was handled; check [Backend::is_open]. Blocks only briefly (one protocol unit).
    fn read(&mut self) -> Result<Vec<u8>>;

    /// Sends input (keystrokes).
    fn write(&mut self, data: &[u8]) -> Result<()>;

    /// Tells the far end the terminal's new size, when it can.
    fn resize(&mut self, _cols: u16, _rows: u16) -> Result<()> {
        Ok(())
    }

    /// Whether the connection is still open.
    fn is_open(&self) -> bool;

    /// The remote command's exit status, once it has ended and the backend knows it.
    fn exit_status(&self) -> Option<i32> {
        None
    }

    /// Output already received but buffered inside the backend, which `poll` on [Backend::fd]
    /// would not report: the reader takes it before waiting again.
    fn pending(&self) -> bool {
        false
    }

    /// Line controls (`c-sh break`, `c-sh line`); a backend without them says so.
    fn control(&mut self, op: &Control) -> Result<String> {
        Err(match op {
            Control::Break(_) => err!("a {} session cannot send a break (serial, telnet and ssh can)", self.kind()),
            Control::Line { .. } => err!("line settings are for serial sessions, not {}", self.kind()),
        })
    }

    /// More about what the session reaches (a serial port's USB identity, ...), for `info`.
    fn detail(&self) -> String {
        String::new()
    }

    /// Called each loop of the reader, for backends that must poll something (reaping a local
    /// child, say). Default: nothing.
    fn tick(&mut self) {}

    /// Asks the far end to close; the session loop then drains any last output.
    fn close(&mut self) -> Result<()>;

    /// A short description of what carries the session, for `info`.
    fn kind(&self) -> &'static str;
}

/// How to reach something: parsed from the target string an agent gives `c-sh open`.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Ssh { user: Option<String>, host: String, port: u16 },
    Telnet { user: Option<String>, host: String, port: u16 },
    Rlogin { user: Option<String>, host: String, port: u16 },
    Raw { host: String, port: u16 },
    /// A device path, or `usb:VID:PID[:SERIAL]` (the host's, through the engine).
    Serial { path: String, line: LineOpts },
    /// A local program in a pty (empty argv means the login shell).
    Run { argv: Vec<String> },
}

impl Target {
    /// Parses a target. Forms:
    ///   ssh://[user@]host[:port], user@host, host (ssh by default)
    ///   telnet://host[:port], rlogin://[user@]host[:port]
    ///   raw://host:port, tcp://host:port
    ///   serial:PATH, serial:///dev/ttyUSB0?baud=115200
    ///   run:COMMAND  (a shell command line; run:-  or "run:" with argv given separately)
    pub fn parse(spec: &str, argv: &[String]) -> Result<Target> {
        if let Some(rest) = spec.strip_prefix("run:") {
            let line = rest.trim();
            let mut argv = argv.to_vec();
            if !line.is_empty() {
                argv = vec!["/bin/sh".into(), "-c".into(), line.into()];
            }
            return Ok(Target::Run { argv });
        }
        if spec == "run" {
            return Ok(Target::Run { argv: argv.to_vec() });
        }
        if let Some(rest) = spec.strip_prefix("serial:") {
            // serial:/dev/ttyUSB0?baud=9600&parity=even, serial:usb:0403:6001, serial:COM3,
            // serial:/dev/ttyUSB0,115200,8,n,1,N (PuTTY's -sercfg form after a comma).
            let rest = rest.strip_prefix("//").unwrap_or(rest);
            let (device, query) = rest.split_once('?').unwrap_or((rest, ""));
            let mut line = LineOpts::default();
            let device = match device.split_once(',') {
                Some((d, compact)) if !d.starts_with("usb:") || d.matches(':').count() >= 2 => {
                    line.set_compact(compact)?;
                    d
                }
                _ => device,
            };
            for kv in query.split('&').filter(|kv| !kv.is_empty()) {
                let (k, v) = kv.split_once('=').ok_or_else(|| err!("serial settings are KEY=VALUE (baud=9600&parity=even)"))?;
                line.set(k, v)?;
            }
            if device.is_empty() {
                return Err(err!("serial: needs a device, e.g. serial:/dev/ttyUSB0 or serial:usb:0403:6001"));
            }
            return Ok(Target::Serial { path: device.to_string(), line });
        }
        let (scheme, rest) = match spec.split_once("://") {
            Some((s, r)) => (Some(s), r),
            None => (None, spec),
        };
        // user@host:port
        let (user, hostport) = match rest.rsplit_once('@') {
            Some((u, h)) => (Some(u.to_string()), h),
            None => (None, rest),
        };
        let (host, port) = split_host_port(hostport)?;
        Ok(match scheme {
            None | Some("ssh") => Target::Ssh { user, host, port: port.unwrap_or(22) },
            Some("telnet") => Target::Telnet { user, host, port: port.unwrap_or(23) },
            Some("rlogin") => Target::Rlogin { user, host, port: port.unwrap_or(513) },
            Some("raw") | Some("tcp") => {
                Target::Raw { host, port: port.ok_or_else(|| err!("raw:// needs a port, e.g. raw://host:9000"))? }
            }
            Some(other) => return Err(err!("c-sh does not know the {other}:// scheme")),
        })
    }


    /// A one-line description for `list`/`info`.
    pub fn describe(&self) -> String {
        match self {
            Target::Ssh { user, host, port } => format!("ssh {}{host}:{port}", user.as_ref().map(|u| format!("{u}@")).unwrap_or_default()),
            Target::Telnet { user, host, port } => format!("telnet {}{host}:{port}", user.as_ref().map(|u| format!("{u}@")).unwrap_or_default()),
            Target::Rlogin { user, host, port } => format!("rlogin {}{host}:{port}", user.as_ref().map(|u| format!("{u}@")).unwrap_or_default()),
            Target::Raw { host, port } => format!("raw {host}:{port}"),
            Target::Serial { path, line } => format!("serial {path} {}", line.describe()),
            Target::Run { argv } => format!("run {}", if argv.is_empty() { "(shell)".into() } else { argv.join(" ") }),
        }
    }
}

fn split_host_port(s: &str) -> Result<(String, Option<u16>)> {
    // [v6]:port, host:port, or bare host.
    if let Some(rest) = s.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or_else(|| err!("unbalanced [ ] in {s}"))?;
        let port = after.strip_prefix(':').map(|p| p.parse()).transpose().map_err(|_| err!("bad port in {s}"))?;
        return Ok((host.to_string(), port));
    }
    match s.rsplit_once(':') {
        // A single colon with digits after is host:port; otherwise (e.g. a bare v6) keep it whole.
        Some((h, p)) if !h.contains(':') && p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
            Ok((h.to_string(), Some(p.parse().map_err(|_| err!("bad port in {s}"))?)))
        }
        _ => Ok((s.to_string(), None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_targets() {
        assert_eq!(Target::parse("ssh://bob@h:2222", &[]).unwrap(), Target::Ssh { user: Some("bob".into()), host: "h".into(), port: 2222 });
        assert_eq!(Target::parse("alice@example.com", &[]).unwrap(), Target::Ssh { user: Some("alice".into()), host: "example.com".into(), port: 22 });
        assert_eq!(Target::parse("host", &[]).unwrap(), Target::Ssh { user: None, host: "host".into(), port: 22 });
        assert_eq!(Target::parse("telnet://bbs:2323", &[]).unwrap(), Target::Telnet { user: None, host: "bbs".into(), port: 2323 });
        assert_eq!(Target::parse("raw://10.0.0.1:9000", &[]).unwrap(), Target::Raw { host: "10.0.0.1".into(), port: 9000 });
        assert!(Target::parse("raw://h", &[]).is_err());
        let Target::Serial { path, line } = Target::parse("serial:/dev/ttyUSB0?baud=9600&parity=even", &[]).unwrap() else { panic!() };
        assert_eq!((path.as_str(), line.baud, line.parity.as_deref()), ("/dev/ttyUSB0", Some(9600), Some("even")));
        let Target::Serial { path, line } = Target::parse("serial:usb:0403:6001:A50285BI?baud=57600", &[]).unwrap() else { panic!() };
        assert_eq!((path.as_str(), line.baud), ("usb:0403:6001:A50285BI", Some(57600)));
        let Target::Serial { path, line } = Target::parse("serial:COM3,9600,7,e,1,X", &[]).unwrap() else { panic!() };
        assert_eq!((path.as_str(), line.describe()), ("COM3", "9600 7E1 xonxoff".to_string()));
        assert!(Target::parse("serial:/dev/ttyS0?baud=fast", &[]).is_err());
        assert_eq!(Target::parse("rlogin://r@h", &[]).unwrap(), Target::Rlogin { user: Some("r".into()), host: "h".into(), port: 513 });
        match Target::parse("run:ls -la", &[]).unwrap() {
            Target::Run { argv } => assert_eq!(argv, ["/bin/sh", "-c", "ls -la"]),
            _ => panic!(),
        }
        assert_eq!(Target::parse("ssh://[::1]:22", &[]).unwrap(), Target::Ssh { user: None, host: "::1".into(), port: 22 });
    }
}
