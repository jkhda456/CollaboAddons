//! The `c-sh` command: parses what the agent typed, starts the daemon if it is not running, sends
//! the request over the socket and prints the answer. `help`, `version` and `daemon` run here.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use crate::{daemon, proto};

pub const HELP: &str = "\
c-sh (collabo sh): terminal sessions that stay connected. A daemon holds each connection open
between commands, so an agent opens once and then sends input and reads output as often as it
needs — no reconnect per command.

usage: c-sh COMMAND [ARG...]

Sessions
  open NAME TARGET [options]   connect and keep the session open
  list                         the sessions: name, target, state, age
  info NAME                    size, cursor, title, bytes, idle time, host key
  close NAME                   end a session

Driving
  run NAME COMMAND [--timeout S]   (POSIX shells: sh, bash, zsh, ash) send COMMAND, wait for
                               it to finish, print its output; exit status is the command's
  run NAME COMMAND --prompt RE     anything else (a router CLI, a REPL): until the last line
                               matches RE, e.g. --prompt '[>#] ?$'; or --idle MS: until quiet
  enter NAME [TEXT...]         send TEXT then Enter (a line of input)
  send NAME TEXT...            send TEXT as typed; \\n \\r \\t \\e escapes work
  key NAME KEY...              keys: Enter Tab Esc Up Down Left Right Home End PageUp
                               PageDown Backspace Delete F1..F12, ctrl+c, alt+x, C-d ...
  resize NAME WxH              the terminal's size

Reading
  screen NAME                  the screen as a person would see it now (curses apps too)
  read NAME [--since N] [--timeout MS]
                               raw output from byte offset N (default 0: everything kept);
                               --timeout waits for new output
  wait NAME [text STR | regex RE | idle MS | closed] [--timeout S]
                               until the output shows it (default: idle 500 ms, 30 s)
  logs NAME                    the whole kept transcript (the last MiB)

TARGET
  ssh://[user@]host[:port]  user@host  host       SSH (the default)
  telnet://host[:port]   rlogin://[user@]host[:port]   raw://host:port (tcp://)
  serial:/dev/ttyUSB0?baud=9600&parity=even       a serial line: this system's, else the host's
  serial:usb:0403:6001[:SERIAL]                    (through the engine; c-sh ports lists them) — a
  serial:COM3,115200,8,n,1,N                       USB adapter by identity, wherever it is plugged
  serial:host:/dev/ttyS0                           the host's device even if one is here too
  run:COMMAND  or  run -- PROGRAM ARG...         a local program in a terminal
  run                                             the local login shell

open options
  --size WxH (80x24)  --term NAME (xterm-256color)  --env KEY=VALUE  --connect-timeout S
  --cwd DIR: where a local program starts (default: the folder c-sh open is run in)
  serial: --baud N --data-bits 5-8 --parity none|even|odd|mark|space --stop-bits 1|2
       --flow none|rtscts|xonxoff, or --line 115200,8,n,1,N
  telnet, rlogin, serial, raw: --user NAME --password PW answer the login and password
       prompts (telnet://user@host works too)
  Secrets: --password-env VAR / --password-file FILE (and --passphrase-env/-file) keep the
       value out of the command line that `ps` shows; prefer them to --password
  SSH: --user NAME  --password PW  --identity FILE  --passphrase PW  --no-agent
       --agent SOCKET  --command CMD (run one command instead of a shell)  --no-pty
       --known-hosts FILE  --accept-new (default: trust a first-seen key, refuse a changed
       one)  --strict (only keys already known)  --insecure (accept any key)
  Keys: the ssh-agent ($SSH_AUTH_SOCK: the host's agent in the sandbox), then --identity
  files or ~/.ssh/id_{ed25519,ecdsa,rsa}, then the password.

Lines
  ports [--json]               the host's serial devices (paths, USB identities) and whether
                               this sandbox may open each
  break NAME [MS]              a break: serial (MS, default 250), telnet BRK, ssh break
  line NAME [baud=N] [data=N] [parity=P] [stop=N] [flow=F] [dtr=on|off] [rts=on|off]
                               change a serial line's settings and modem lines; shows them

Other
  forget HOST[:PORT]           remove a host's keys from known_hosts
  shutdown                     stop the daemon (and every session)
  version, help

Exit status: 0 done (run: the command's status), 1 failed, 2 usage, 3 timed out,
4 no such session, 5 could not reach the daemon.

Example
  c-sh open web ssh://deploy@10.0.0.5
  c-sh run web 'systemctl status nginx --no-pager'
  c-sh enter web 'sudo -s'; c-sh wait web text 'password'; c-sh enter web \"$PW\"
  c-sh open top run -- top; c-sh screen top; c-sh key top q
";

/// Runs the command line; returns the exit status.
pub fn main(args: Vec<String>) -> i32 {
    let Some(command) = args.first().map(String::as_str) else {
        eprint!("{HELP}");
        return 2;
    };
    match command {
        "help" | "-h" | "--help" => {
            print!("{HELP}");
            0
        }
        "version" | "--version" => {
            println!("c-sh {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "ports" => ports(args.iter().any(|a| a == "--json")),
        "daemon" => {
            let detach = args.iter().any(|a| a == "--detach");
            match daemon::run(detach) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("c-sh: {e}");
                    1
                }
            }
        }
        _ => request(&args),
    }
}

/// `c-sh ports`: the serial devices the host has and whether the guest may open each (the
/// engine's bridge), and any this system has itself.
fn ports(json: bool) -> i32 {
    let local = crate::backend::serial::list_local();
    let host = crate::backend::serial::list_host();
    if json {
        match &host {
            Ok(line) => println!("{line}"),
            Err(e) => println!("{{\"ok\":false,\"error\":{:?}}}", e.0),
        }
        return if host.is_ok() { 0 } else { 1 };
    }
    let mut status = 0;
    match host {
        Ok(line) => print!("{}", describe_ports(&line)),
        Err(e) => {
            println!("the host's serial ports: {e}");
            status = 1;
        }
    }
    for l in &local {
        println!("{l}\tthis system's own");
    }
    status
}

/// The bridge's listing as lines: PATH, access, bus and USB identity, the stable name.
fn describe_ports(json: &str) -> String {
    let field = |obj: &str, key: &str| -> Option<String> {
        let at = obj.find(&format!("\"{key}\":"))? + key.len() + 3;
        let rest = obj[at..].trim_start();
        if let Some(s) = rest.strip_prefix('"') {
            s.find('"').map(|e| s[..e].to_string())
        } else {
            let end = rest.find([',', '}']).unwrap_or(rest.len());
            Some(rest[..end].trim().to_string()).filter(|v| v != "null")
        }
    };
    let mode = field(json, "mode").unwrap_or_default();
    let mut out = format!("the host shares serial ports: {}\n", match mode.as_str() {
        "off" => "no (the app's serial setting is off; the engine's --serial)".to_string(),
        "ask" => "when the app allows each (ask)".to_string(),
        other => other.to_string(),
    });
    // Each device is one {...} with "path" in it; split on the path key.
    let mut parts: Vec<&str> = json.split("{\"path\":").skip(1).collect();
    if parts.is_empty() {
        out.push_str("(no serial devices on the host)\n");
    }
    for part in parts.drain(..) {
        let obj = format!("{{\"path\":{part}");
        let path = field(&obj, "path").unwrap_or_default();
        let access = field(&obj, "access").unwrap_or_default();
        let bus = field(&obj, "bus").unwrap_or_default();
        let mut what = bus.clone();
        if let (Some(vid), Some(pid)) = (field(&obj, "vid"), field(&obj, "pid")) {
            what = format!("usb:{vid}:{pid}");
            if let Some(s) = field(&obj, "serial") {
                what.push(':');
                what.push_str(&s);
            }
            for k in ["manufacturer", "product"] {
                if let Some(v) = field(&obj, k) {
                    what.push(' ');
                    what.push_str(&v);
                }
            }
        }
        if let Some(d) = field(&obj, "driver") {
            what.push_str(&format!(" [{d}]"));
        }
        out.push_str(&format!("{path}\t{access}\t{what}\n"));
        if let Some(by) = field(&obj, "byId") {
            out.push_str(&format!("\t\t{by}\n"));
        }
    }
    out
}

/// The commands the daemon answers (anything else is a usage error found here).
const COMMANDS: &[&str] = &[
    "open", "list", "send", "enter", "key", "read", "screen", "wait", "run", "resize", "info", "close", "quit", "stop",
    "logs", "forget", "shutdown", "break", "line",
];

/// `--password-env VAR` / `--password-file FILE` (and `--passphrase-…`) become `--password VALUE`
/// here, so the secret goes to the daemon over the socket and never sits in a command line `ps`
/// shows.
fn resolve_secrets(args: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let (flag, how) = match a {
            "--password-env" => ("--password", "env"),
            "--password-file" => ("--password", "file"),
            "--passphrase-env" => ("--passphrase", "env"),
            "--passphrase-file" => ("--passphrase", "file"),
            _ => {
                out.push(args[i].clone());
                i += 1;
                continue;
            }
        };
        let source = args.get(i + 1).ok_or_else(|| format!("{a} needs a value"))?;
        let value = if how == "env" {
            std::env::var(source).map_err(|_| format!("{a}: ${source} is not set"))?
        } else {
            let text = std::fs::read_to_string(source).map_err(|e| format!("{a}: cannot read {source}: {e}"))?;
            text.strip_suffix('\n').unwrap_or(&text).strip_suffix('\r').unwrap_or(text.strip_suffix('\n').unwrap_or(&text)).to_string()
        };
        out.push(flag.to_string());
        out.push(value);
        i += 2;
    }
    Ok(out)
}

fn request(args: &[String]) -> i32 {
    if !COMMANDS.contains(&args[0].as_str()) {
        eprintln!("c-sh: unknown command \"{}\" (c-sh help)", args[0]);
        return 2;
    }
    let resolved = match resolve_secrets(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("c-sh: {e}");
            return 2;
        }
    };
    let mut resolved = resolved;
    // The daemon may have been started from another shell: what `open` should use from this one
    // (its ssh-agent, its folder) goes along with the request, before any `--` program words.
    if resolved[0] == "open" && resolved.len() >= 3 {
        let mut extra = Vec::new();
        let given = |f: &str| resolved.iter().take_while(|a| *a != "--").any(|a| a == f);
        if !given("--agent") && !given("--no-agent") {
            if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
                if !sock.is_empty() {
                    extra.extend(["--agent".to_string(), sock]);
                }
            }
        }
        if !given("--cwd") {
            if let Ok(dir) = std::env::current_dir() {
                extra.extend(["--cwd".to_string(), dir.to_string_lossy().into_owned()]);
            }
        }
        for (i, e) in extra.into_iter().enumerate() {
            resolved.insert(3 + i, e);
        }
    }
    let args = &resolved[..];
    // Commands that only read state do not start a daemon that is not there.
    let starts_daemon = matches!(args[0].as_str(), "open");
    let stream = match connect(starts_daemon) {
        Ok(s) => s,
        Err(e) => {
            if !starts_daemon && args[0] == "list" {
                println!("no sessions (c-sh open NAME TARGET)");
                return 0;
            }
            eprintln!("c-sh: {e}");
            return 5;
        }
    };
    let mut stream = stream;
    if let Err(e) = proto::write_request(&mut stream, args) {
        eprintln!("c-sh: {e}");
        return 5;
    }
    let (status, body) = match proto::read_reply(&mut stream) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("c-sh: {e}");
            return 5;
        }
    };
    // A first line starting "c-sh: " is a message for stderr; the rest is the output.
    let (msg, out) = if body.starts_with(b"c-sh: ") {
        match body.iter().position(|&b| b == b'\n') {
            Some(nl) => (&body[..=nl], &body[nl + 1..]),
            None => (&body[..], &body[body.len()..]),
        }
    } else {
        (&body[..0], &body[..])
    };
    let _ = std::io::stderr().write_all(msg);
    let _ = std::io::stdout().write_all(out);
    let _ = std::io::stdout().flush();
    // "no session" errors get their own status.
    if status == 1 && String::from_utf8_lossy(msg).contains("no session \"") {
        return 4;
    }
    status
}

/// Connects to the daemon, starting it first when `start` is set and none answers.
fn connect(start: bool) -> crate::Result<UnixStream> {
    let path = daemon::socket_dir().join("socket");
    if let Ok(s) = UnixStream::connect(&path) {
        return Ok(s);
    }
    if !start {
        return Err(crate::err!("no c-sh daemon is running (c-sh open starts one)"));
    }
    spawn_daemon()?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(s) = UnixStream::connect(&path) {
            return Ok(s);
        }
        if Instant::now() > deadline {
            return Err(crate::err!("the daemon did not start (see {})", daemon::socket_dir().join("daemon.log").display()));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Starts `c-sh daemon --detach` in the background, its output in daemon.log.
fn spawn_daemon() -> crate::Result<()> {
    let dir = daemon::socket_dir();
    std::fs::create_dir_all(&dir)?;
    let log_path = dir.join("daemon.log");
    let open_log = || std::fs::OpenOptions::new().create(true).append(true).open(&log_path);
    let exe = std::env::current_exe().unwrap_or_else(|_| "c-sh".into());
    std::process::Command::new(exe)
        .arg("daemon")
        .arg("--detach")
        .stdin(std::process::Stdio::null())
        .stdout(open_log()?)
        .stderr(open_log()?)
        .spawn()
        .map_err(|e| crate::err!("cannot start the daemon: {e}"))?;
    Ok(())
}
