//! A serial line. Where the device node exists here (a native build, a device a guest has), the
//! tty is opened and put in raw mode directly. Otherwise — the sandbox has no serial devices of its
//! own — the host's port is reached through the engine's bridge (vsock port 1083, engine/src/
//! serial.rs), which opens it if the app shares it: a path (`/dev/ttyUSB0`, `COM3`) or a USB
//! identity (`usb:0403:6001[:SERIAL]`); `host:PATH` asks the host even where a node of that name
//! exists here. Both ways take the same line settings and controls.

use std::ffi::CString;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use super::{Backend, Control};
use crate::sys::{self, FdStream};
use crate::{err, Result};

/// The bridge's vsock port on the host (CID 2).
pub const BRIDGE_PORT: u32 = 1083;
const FRAME_DATA: u8 = 0;
const FRAME_CONTROL: u8 = 1;

/// Line settings, as the target's query or `open`'s options give them (unset: the defaults,
/// 115200 8N1 without flow control).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineOpts {
    pub baud: Option<u32>,
    pub data_bits: Option<u8>,
    pub parity: Option<String>,
    pub stop_bits: Option<u8>,
    pub flow: Option<String>,
}

impl LineOpts {
    /// `KEY=VALUE` settings: baud, data (or databits), parity, stop (or stopbits), flow.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        match key.to_ascii_lowercase().as_str() {
            "baud" | "speed" => self.baud = Some(value.parse().map_err(|_| err!("baud: a number, e.g. 115200"))?),
            "data" | "databits" | "data-bits" => {
                self.data_bits = Some(value.parse().ok().filter(|d| (5..=8).contains(d)).ok_or_else(|| err!("data bits: 5 to 8"))?)
            }
            "parity" => {
                let p = match value.to_ascii_lowercase().as_str() {
                    "n" | "none" => "none",
                    "e" | "even" => "even",
                    "o" | "odd" => "odd",
                    "m" | "mark" => "mark",
                    "s" | "space" => "space",
                    _ => return Err(err!("parity: none, even, odd, mark or space")),
                };
                self.parity = Some(p.into());
            }
            "stop" | "stopbits" | "stop-bits" => {
                self.stop_bits = Some(value.parse().ok().filter(|s| *s == 1 || *s == 2).ok_or_else(|| err!("stop bits: 1 or 2"))?)
            }
            "flow" => {
                let f = match value.to_ascii_lowercase().as_str() {
                    "none" | "n" | "off" => "none",
                    "rtscts" | "hardware" | "rts" | "hw" => "rtscts",
                    "xonxoff" | "software" | "xon" | "sw" => "xonxoff",
                    _ => return Err(err!("flow: none, rtscts or xonxoff")),
                };
                self.flow = Some(f.into());
            }
            other => return Err(err!("unknown line setting {other} (baud, data, parity, stop, flow)")),
        }
        Ok(())
    }

    /// PuTTY's compact form, `115200,8,n,1,N` (speed, data bits, parity, stop bits, flow N/X/R).
    pub fn set_compact(&mut self, spec: &str) -> Result<()> {
        let parts: Vec<&str> = spec.split(',').collect();
        if let Some(b) = parts.first().filter(|b| !b.is_empty()) {
            self.set("baud", b)?;
        }
        if let Some(d) = parts.get(1).filter(|d| !d.is_empty()) {
            self.set("data", d)?;
        }
        if let Some(p) = parts.get(2).filter(|p| !p.is_empty()) {
            self.set("parity", p)?;
        }
        if let Some(s) = parts.get(3).filter(|s| !s.is_empty()) {
            self.set("stop", s)?;
        }
        if let Some(f) = parts.get(4).filter(|f| !f.is_empty()) {
            let f = match f.to_ascii_uppercase().as_str() {
                "N" => "none",
                "X" => "xonxoff",
                "R" | "H" => "rtscts",
                _ => return Err(err!("flow in the compact form: N, X or R")),
            };
            self.set("flow", f)?;
        }
        Ok(())
    }

    fn json_fields(&self) -> String {
        let mut f = Vec::new();
        if let Some(b) = self.baud {
            f.push(format!("\"baud\":{b}"));
        }
        if let Some(d) = self.data_bits {
            f.push(format!("\"dataBits\":{d}"));
        }
        if let Some(p) = &self.parity {
            f.push(format!("\"parity\":\"{p}\""));
        }
        if let Some(s) = self.stop_bits {
            f.push(format!("\"stopBits\":{s}"));
        }
        if let Some(fl) = &self.flow {
            f.push(format!("\"flow\":\"{fl}\""));
        }
        f.join(",")
    }

    pub fn describe(&self) -> String {
        format!(
            "{} {}{}{} {}",
            self.baud.unwrap_or(115_200),
            self.data_bits.unwrap_or(8),
            self.parity.as_deref().map(|p| p[..1].to_uppercase()).unwrap_or_else(|| "N".into()),
            self.stop_bits.unwrap_or(1),
            self.flow.as_deref().unwrap_or("none")
        )
    }

    fn merge(&mut self, other: &LineOpts) {
        if other.baud.is_some() {
            self.baud = other.baud;
        }
        if other.data_bits.is_some() {
            self.data_bits = other.data_bits;
        }
        if other.parity.is_some() {
            self.parity = other.parity.clone();
        }
        if other.stop_bits.is_some() {
            self.stop_bits = other.stop_bits;
        }
        if other.flow.is_some() {
            self.flow = other.flow.clone();
        }
    }
}

/// A JSON string literal (the bridge's requests are small; no JSON library needed).
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A field of a flat JSON reply: a string (unquoted), a number, true/false. Enough for the
/// bridge's answers, which c-sh only reads a few fields of.
fn json_field(json: &str, key: &str) -> Option<String> {
    let at = json.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = json[at..].trim_start();
    if let Some(s) = rest.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => return Some(out),
                '\\' => match chars.next()? {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'u' => {
                        let hex: String = chars.by_ref().take(4).collect();
                        out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                    }
                    other => out.push(other),
                },
                c => out.push(c),
            }
        }
        None
    } else {
        let end = rest.find([',', '}', ']']).unwrap_or(rest.len());
        Some(rest[..end].trim().to_string())
    }
}

enum Link {
    /// The tty itself.
    Local { port: FdStream },
    /// The host's port through the bridge: framed data and control messages.
    Bridge { socket: FdStream, inbox: Vec<u8>, data: Vec<u8>, answers: Vec<String> },
}

pub struct Serial {
    link: Link,
    open: bool,
    device: String,
    line: LineOpts,
    /// What the bridge said about the device (USB identity, ...), for `info`.
    pub detail: String,
}

impl Serial {
    /// Opens `device` (a path, or `usb:VID:PID[:SERIAL]`) with `line`.
    pub fn open(device: &str, line: &LineOpts) -> Result<Serial> {
        // `host:PATH` always means the host's device, even where this system has the same name.
        let (device, host_only) = match device.strip_prefix("host:") {
            Some(d) => (d, true),
            None => (device, false),
        };
        let local = !host_only && !device.starts_with("usb:") && is_char_device(device);
        if local {
            let port = open_local(device, line)?;
            return Ok(Serial { link: Link::Local { port }, open: true, device: device.to_string(), line: line.clone(), detail: "this system's own device".into() });
        }
        let socket = bridge_connect().map_err(|e| if host_only || device.starts_with("usb:") { e } else { err!("{device} is not a device here, and {e}") })?;
        let fields = line.json_fields();
        let request = format!("{{\"op\":\"open\",\"device\":{}{}{}}}\n", quote(device), if fields.is_empty() { "" } else { "," }, fields);
        let mut socket = socket;
        socket.write_all(request.as_bytes())?;
        let reply = read_line(&mut socket)?;
        if json_field(&reply, "ok").as_deref() != Some("true") {
            let why = json_field(&reply, "error").unwrap_or_else(|| reply.trim().to_string());
            return Err(err!("{why}"));
        }
        let path = json_field(&reply, "path").unwrap_or_else(|| device.to_string());
        let mut detail = format!("the host's {path}");
        if let (Some(vid), Some(pid)) = (json_field(&reply, "vid"), json_field(&reply, "pid")) {
            detail.push_str(&format!(" (USB {vid}:{pid}"));
            for k in ["manufacturer", "product"] {
                if let Some(v) = json_field(&reply, k).filter(|v| v != "null") {
                    detail.push(' ');
                    detail.push_str(&v);
                }
            }
            if let Some(s) = json_field(&reply, "serial").filter(|v| v != "null") {
                detail.push_str(&format!(", serial {s}"));
            }
            detail.push(')');
        }
        socket.set_nonblocking(true)?;
        Ok(Serial {
            link: Link::Bridge { socket, inbox: Vec::new(), data: Vec::new(), answers: Vec::new() },
            open: true,
            device: path,
            line: line.clone(),
            detail,
        })
    }

    /// Takes complete frames out of the bridge's input: data to `data`, control to `answers`.
    fn parse_frames(inbox: &mut Vec<u8>, data: &mut Vec<u8>, answers: &mut Vec<String>, open: &mut bool, detail: &mut String) {
        loop {
            if inbox.len() < 5 {
                return;
            }
            let len = u32::from_be_bytes(inbox[1..5].try_into().unwrap()) as usize;
            if inbox.len() < 5 + len {
                return;
            }
            let kind = inbox[0];
            let payload: Vec<u8> = inbox[5..5 + len].to_vec();
            inbox.drain(..5 + len);
            match kind {
                FRAME_DATA => data.extend_from_slice(&payload),
                FRAME_CONTROL => {
                    let text = String::from_utf8_lossy(&payload).into_owned();
                    if json_field(&text, "event").as_deref() == Some("closed") {
                        *open = false;
                        *detail = json_field(&text, "reason").unwrap_or_else(|| "the serial port went away".into());
                    } else {
                        answers.push(text);
                    }
                }
                _ => {}
            }
        }
    }

    /// Sends a control message through the bridge and waits for its answer.
    fn bridge_control(&mut self, json: &str) -> Result<String> {
        let Link::Bridge { socket, inbox, data, answers } = &mut self.link else {
            return Err(err!("not a bridged port"));
        };
        let mut frame = vec![FRAME_CONTROL];
        frame.extend_from_slice(&(json.len() as u32).to_be_bytes());
        frame.extend_from_slice(json.as_bytes());
        write_all(socket, &frame)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if !answers.is_empty() {
                let answer = answers.remove(0);
                if json_field(&answer, "ok").as_deref() == Some("false") {
                    return Err(err!("{}", json_field(&answer, "error").unwrap_or(answer)));
                }
                return Ok(answer);
            }
            if std::time::Instant::now() > deadline {
                return Err(err!("the host did not answer the serial control request"));
            }
            sys::wait_readable(socket.as_raw_fd(), 100)?;
            let mut buf = [0u8; 16384];
            match socket.read(&mut buf) {
                Ok(0) => {
                    self.open = false;
                    return Err(err!("the bridge closed"));
                }
                Ok(n) => inbox.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(err!("the bridge: {e}")),
            }
            let mut open = self.open;
            Serial::parse_frames(inbox, data, answers, &mut open, &mut self.detail);
            self.open = open;
        }
    }
}

fn write_all(s: &mut FdStream, mut data: &[u8]) -> Result<()> {
    while !data.is_empty() {
        match s.write(data) {
            Ok(0) => return Err(err!("the serial line is closed")),
            Ok(n) => data = &data[n..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let mut p = libc::pollfd { fd: s.as_raw_fd(), events: libc::POLLOUT, revents: 0 };
                // SAFETY: one valid pollfd.
                unsafe { libc::poll(&mut p, 1, 100) };
            }
            Err(e) => return Err(err!("writing to the serial line: {e}")),
        }
    }
    Ok(())
}

fn read_line(s: &mut FdStream) -> Result<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match s.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                line.push(byte[0]);
                if line.len() > 1 << 20 {
                    return Err(err!("the bridge's answer is too long"));
                }
            }
            Err(e) => return Err(err!("the bridge: {e}")),
        }
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

/// A connection to the engine's serial bridge.
fn bridge_connect() -> Result<FdStream> {
    let fd = sys::vsock_connect(libc::VMADDR_CID_HOST, BRIDGE_PORT)
        .map_err(|e| err!("no engine bridge to the host's serial ports ({e}); the app shares them with its `serial` setting, the engine with --serial"))?;
    Ok(FdStream::new(fd))
}

/// This system's own serial ports: ttys with hardware behind them (sysfs), less the legacy 8250
/// ports a board does not have (UART type 0).
pub fn list_local() -> Vec<String> {
    let read = |p: std::path::PathBuf| std::fs::read_to_string(p).ok().map(|s| s.trim().to_string());
    let mut out: Vec<String> = std::fs::read_dir("/sys/class/tty")
        .map(|d| {
            d.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let dir = e.path();
                    if std::fs::metadata(dir.join("device")).is_err() {
                        return None;
                    }
                    if name.starts_with("ttyS") && read(dir.join("type")).as_deref() == Some("0") {
                        return None;
                    }
                    let path = format!("/dev/{name}");
                    is_char_device(&path).then_some(path)
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// The host's serial devices, as the bridge lists them (a JSON line).
pub fn list_host() -> Result<String> {
    let mut socket = bridge_connect()?;
    socket.write_all(b"{\"op\":\"list\"}\n")?;
    read_line(&mut socket)
}

fn is_char_device(path: &str) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(path).map(|m| m.file_type().is_char_device()).unwrap_or(false)
}

fn open_local(path: &str, line: &LineOpts) -> Result<FdStream> {
    let c_path = CString::new(path).map_err(|_| err!("a NUL in the device path"))?;
    // SAFETY: open with O_NOCTTY so the tty does not become our controlling terminal.
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK) };
    if fd < 0 {
        return Err(err!("cannot open {path}: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: fd is a valid descriptor we now own.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    configure_local(owned.as_raw_fd(), line)?;
    let port = FdStream::new(owned);
    port.set_nonblocking(true)?;
    Ok(port)
}

/// Raw mode at the line's settings.
fn configure_local(fd: i32, line: &LineOpts) -> Result<()> {
    // SAFETY: tcgetattr/tcsetattr on an open tty; the termios struct is written by the first.
    unsafe {
        let mut tio: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut tio) != 0 {
            return Err(err!("not a terminal device: {}", std::io::Error::last_os_error()));
        }
        libc::cfmakeraw(&mut tio);
        tio.c_cflag |= libc::CLOCAL | libc::CREAD;
        tio.c_cflag &= !libc::CSIZE;
        tio.c_cflag |= match line.data_bits.unwrap_or(8) {
            5 => libc::CS5,
            6 => libc::CS6,
            7 => libc::CS7,
            _ => libc::CS8,
        };
        if line.stop_bits == Some(2) {
            tio.c_cflag |= libc::CSTOPB;
        }
        match line.parity.as_deref() {
            Some("even") => tio.c_cflag |= libc::PARENB,
            Some("odd") => tio.c_cflag |= libc::PARENB | libc::PARODD,
            Some("mark") => tio.c_cflag |= libc::PARENB | libc::PARODD | libc::CMSPAR,
            Some("space") => tio.c_cflag |= libc::PARENB | libc::CMSPAR,
            _ => {}
        }
        match line.flow.as_deref() {
            Some("rtscts") => tio.c_cflag |= libc::CRTSCTS,
            Some("xonxoff") => tio.c_iflag |= libc::IXON | libc::IXOFF,
            _ => {}
        }
        tio.c_cc[libc::VMIN] = 1;
        tio.c_cc[libc::VTIME] = 0;
        let baud = line.baud.unwrap_or(115_200);
        let speed = baud_constant(baud).ok_or_else(|| err!("unsupported baud rate {baud}"))?;
        libc::cfsetispeed(&mut tio, speed);
        libc::cfsetospeed(&mut tio, speed);
        if libc::tcsetattr(fd, libc::TCSANOW, &tio) != 0 {
            return Err(err!("cannot configure the serial line: {}", std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

fn baud_constant(baud: u32) -> Option<libc::speed_t> {
    Some(match baud {
        300 => libc::B300,
        1200 => libc::B1200,
        2400 => libc::B2400,
        4800 => libc::B4800,
        9600 => libc::B9600,
        19200 => libc::B19200,
        38400 => libc::B38400,
        57600 => libc::B57600,
        115200 => libc::B115200,
        230400 => libc::B230400,
        460800 => libc::B460800,
        500000 => libc::B500000,
        921600 => libc::B921600,
        1000000 => libc::B1000000,
        1500000 => libc::B1500000,
        2000000 => libc::B2000000,
        3000000 => libc::B3000000,
        4000000 => libc::B4000000,
        _ => return None,
    })
}

impl Backend for Serial {
    fn fd(&self) -> std::os::fd::RawFd {
        match &self.link {
            Link::Local { port } => port.as_raw_fd(),
            Link::Bridge { socket, .. } => socket.as_raw_fd(),
        }
    }

    fn read(&mut self) -> Result<Vec<u8>> {
        let mut buf = [0u8; 16384];
        match &mut self.link {
            Link::Local { port } => match port.read(&mut buf) {
                Ok(0) => Ok(Vec::new()),
                Ok(n) => Ok(buf[..n].to_vec()),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(Vec::new()),
                Err(e) if e.raw_os_error() == Some(libc::EIO) => {
                    self.open = false;
                    Ok(Vec::new())
                }
                Err(e) => Err(err!("reading {}: {e}", self.device)),
            },
            Link::Bridge { socket, inbox, data, answers } => {
                match socket.read(&mut buf) {
                    Ok(0) => {
                        self.open = false;
                    }
                    Ok(n) => inbox.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(err!("the serial bridge: {e}")),
                }
                let mut open = self.open;
                Serial::parse_frames(inbox, data, answers, &mut open, &mut self.detail);
                self.open = open;
                Ok(std::mem::take(data))
            }
        }
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        match &mut self.link {
            Link::Local { port } => write_all(port, bytes),
            Link::Bridge { socket, .. } => {
                for chunk in bytes.chunks(32 * 1024) {
                    let mut frame = vec![FRAME_DATA];
                    frame.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
                    frame.extend_from_slice(chunk);
                    write_all(socket, &frame)?;
                }
                Ok(())
            }
        }
    }

    fn pending(&self) -> bool {
        match &self.link {
            Link::Bridge { inbox, data, .. } => !data.is_empty() || inbox.len() >= 5 && inbox.len() >= 5 + u32::from_be_bytes(inbox[1..5].try_into().unwrap()) as usize,
            Link::Local { .. } => false,
        }
    }

    fn control(&mut self, op: &Control) -> Result<String> {
        match op {
            Control::Break(ms) => {
                match &self.link {
                    Link::Local { port } => {
                        // SAFETY: tcsendbreak on our tty (its duration is the system's).
                        if unsafe { libc::tcsendbreak(port.as_raw_fd(), 0) } != 0 {
                            return Err(err!("sending a break: {}", std::io::Error::last_os_error()));
                        }
                    }
                    Link::Bridge { .. } => {
                        self.bridge_control(&format!("{{\"op\":\"break\",\"ms\":{ms}}}"))?;
                    }
                }
                Ok(format!("sent a break ({ms} ms) on {}", self.device))
            }
            Control::Line { line, dtr, rts } => {
                let mut new = self.line.clone();
                new.merge(line);
                match &self.link {
                    Link::Local { port } => {
                        let fd = port.as_raw_fd();
                        configure_local(fd, &new)?;
                        self.line = new;
                        for (want, bit) in [(dtr, libc::TIOCM_DTR), (rts, libc::TIOCM_RTS)] {
                            if let Some(on) = want {
                                let request = if *on { libc::TIOCMBIS } else { libc::TIOCMBIC };
                                // SAFETY: the modem bit by reference, as the ioctl wants.
                                if unsafe { libc::ioctl(fd, request as _, &bit) } != 0 {
                                    let e = std::io::Error::last_os_error();
                                    return Err(err!("{}: now {}, but the modem lines were not set: {e}", self.device, self.line.describe()));
                                }
                            }
                        }
                        Ok(format!("{}: {}", self.device, self.line.describe()))
                    }
                    Link::Bridge { .. } => {
                        let fields = line.json_fields();
                        if !fields.is_empty() {
                            self.bridge_control(&format!("{{\"op\":\"config\",{fields}}}"))?;
                        }
                        self.line = new;
                        let mut set = Vec::new();
                        if let Some(on) = dtr {
                            set.push(format!("\"dtr\":{on}"));
                        }
                        if let Some(on) = rts {
                            set.push(format!("\"rts\":{on}"));
                        }
                        if !set.is_empty() {
                            if let Err(e) = self.bridge_control(&format!("{{\"op\":\"set\",{}}}", set.join(","))) {
                                return Err(err!("{}: now {}, but the modem lines were not set: {e}", self.device, self.line.describe()));
                            }
                        }
                        let status = self.bridge_control("{\"op\":\"status\"}")?;
                        let flag = |k: &str| json_field(&status, k).map(|v| if v == "true" { "on" } else { "off" }).unwrap_or("?");
                        Ok(format!(
                            "{}: {} — modem lines CTS {} DSR {} DCD {} RI {}",
                            self.device,
                            self.line.describe(),
                            flag("cts"),
                            flag("dsr"),
                            flag("dcd"),
                            flag("ri")
                        ))
                    }
                }
            }
        }
    }

    fn is_open(&self) -> bool {
        self.open
    }

    fn close(&mut self) -> Result<()> {
        self.open = false;
        if matches!(self.link, Link::Bridge { .. }) {
            // Ask the bridge to close and wait for it to say so: it answers once the host's port
            // is closed, so an `open` right after finds it free.
            let _ = self.bridge_control("{\"op\":\"close\"}");
            if let Link::Bridge { socket, .. } = &self.link {
                // SAFETY: shutting down our own socket.
                unsafe { libc::shutdown(socket.as_raw_fd(), libc::SHUT_RDWR) };
            }
        }
        Ok(())
    }

    fn kind(&self) -> &'static str {
        match self.link {
            Link::Local { .. } => "serial",
            Link::Bridge { .. } => "serial (the host's, through the engine)",
        }
    }

    fn detail(&self) -> String {
        format!("{} at {}", self.detail, self.line.describe())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_settings() {
        let mut l = LineOpts::default();
        l.set_compact("9600,7,e,2,R").unwrap();
        assert_eq!(l, LineOpts { baud: Some(9600), data_bits: Some(7), parity: Some("even".into()), stop_bits: Some(2), flow: Some("rtscts".into()) });
        assert_eq!(l.describe(), "9600 7E2 rtscts");
        assert!(l.set("data", "9").is_err());
        assert_eq!(l.json_fields(), "\"baud\":9600,\"dataBits\":7,\"parity\":\"even\",\"stopBits\":2,\"flow\":\"rtscts\"");
    }

    #[test]
    fn reads_bridge_json() {
        let reply = r#"{"ok":true,"device":{"path":"/dev/ttyUSB0","usb":{"vid":"0403","pid":"6001","serial":"A5\"0","manufacturer":null}}}"#;
        assert_eq!(json_field(reply, "ok").as_deref(), Some("true"));
        assert_eq!(json_field(reply, "path").as_deref(), Some("/dev/ttyUSB0"));
        assert_eq!(json_field(reply, "serial").as_deref(), Some("A5\"0"));
        assert_eq!(json_field(reply, "manufacturer").as_deref(), Some("null"));
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn frames() {
        let mut inbox = vec![0u8, 0, 0, 0, 2, b'h', b'i', 1, 0, 0, 0, 2, b'{', b'}', 0, 0, 0];
        let (mut data, mut answers, mut open, mut detail) = (Vec::new(), Vec::new(), true, String::new());
        Serial::parse_frames(&mut inbox, &mut data, &mut answers, &mut open, &mut detail);
        assert_eq!(data, b"hi");
        assert_eq!(answers, ["{}"]);
        assert_eq!(inbox, [0, 0, 0]); // an incomplete header waits
    }
}
