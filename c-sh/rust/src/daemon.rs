//! The c-sh daemon: it holds the open sessions and answers the `c-sh` command over a Unix socket.
//! One daemon per socket directory; the first command starts it (see [crate::cli]).

use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::backend::{local::Local, serial::Serial, ssh, tcp::Tcp, Backend, Control, LineOpts, Target};
use crate::session::{Condition, Session};
use crate::term;
use crate::ssh::privatekey;
use crate::util::{home, now_ms};
use crate::{err, proto, Result};

pub fn socket_dir() -> PathBuf {
    std::env::var_os("COLLABO_CSH_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp/.collabo-csh"))
}

fn socket_path() -> PathBuf {
    socket_dir().join("socket")
}

/// The registry of live sessions.
#[derive(Default)]
struct Registry {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

impl Registry {
    fn get(&self, name: &str) -> Result<Arc<Session>> {
        self.sessions.lock().unwrap().get(name).cloned().ok_or_else(|| err!("no session \"{name}\" (c-sh list)"))
    }
}

/// Runs the daemon until it is told to stop (or all is idle for a long time is not enforced; the
/// daemon lives as long as the socket does). `detached` only affects logging of the banner.
pub fn run(detached: bool) -> Result<()> {
    let dir = socket_dir();
    std::fs::create_dir_all(&dir)?;
    let path = socket_path();
    // Only one daemon: an abstract-free flock on a lock file.
    let lock = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(dir.join("daemon.lock"))?;
    // SAFETY: flock on an open descriptor.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(err!("another c-sh daemon is already running"));
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| err!("cannot bind {}: {e}", path.display()))?;
    if detached {
        // SAFETY: a new session so the terminal that started us can go away.
        unsafe { libc::setsid() };
    }
    eprintln!("c-sh daemon listening at {}", path.display());

    let registry = Arc::new(Registry::default());
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let registry = registry.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle(stream, registry) {
                        eprintln!("c-sh: connection error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("c-sh: accept: {e}"),
        }
    }
    Ok(())
}

fn handle(mut stream: UnixStream, registry: Arc<Registry>) -> Result<()> {
    let words = proto::read_request(&mut stream)?;
    let (status, body) = dispatch(&registry, &words);
    proto::write_reply(&mut stream, status, &body)
}

/// Runs one command, returning (exit status, output bytes). Status mirrors the CLI's: 0 ok,
/// 1 failed, 2 usage, 3 timed out, 4 no such session, 5 could not connect.
fn dispatch(registry: &Registry, words: &[String]) -> (i32, Vec<u8>) {
    match run_command(registry, words) {
        Ok((status, body)) => (status, body),
        Err(e) => {
            let status = if e.0.starts_with("no session") { 4 } else { 1 };
            (status, format!("c-sh: {e}\n").into_bytes())
        }
    }
}

fn run_command(registry: &Registry, words: &[String]) -> Result<(i32, Vec<u8>)> {
    let command = words.first().map(String::as_str).unwrap_or("");
    let args = &words[1.min(words.len())..];
    match command {
        "open" => cmd_open(registry, args),
        "list" => cmd_list(registry),
        "send" => cmd_send(registry, args, false),
        "enter" => cmd_send(registry, args, true),
        "key" => cmd_key(registry, args),
        "read" => cmd_read(registry, args),
        "screen" => cmd_screen(registry, args),
        "wait" => cmd_wait(registry, args),
        "run" => cmd_run(registry, args),
        "resize" => cmd_resize(registry, args),
        "info" => cmd_info(registry, args),
        "close" | "quit" | "stop" => cmd_close(registry, args),
        "logs" => cmd_logs(registry, args),
        "break" => cmd_break(registry, args),
        "line" => cmd_line(registry, args),
        "forget" => cmd_forget(args),
        "shutdown" => {
            // Close every session (a serial port waits until the host has closed it), answer,
            // then end.
            let sessions: Vec<Arc<Session>> = registry.sessions.lock().unwrap().drain().map(|(_, s)| s).collect();
            for s in &sessions {
                let _ = s.close();
            }
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                std::process::exit(0);
            });
            Ok((0, format!("c-sh daemon stopped ({} session{} closed)\n", sessions.len(), if sessions.len() == 1 { "" } else { "s" }).into_bytes()))
        }
        other => Ok((2, format!("c-sh: unknown command \"{other}\" (c-sh help)\n").into_bytes())),
    }
}

/// A flag/value parser over the remaining words.
struct Opts<'a> {
    args: &'a [String],
    at: usize,
}

impl<'a> Opts<'a> {
    fn new(args: &'a [String]) -> Opts<'a> {
        Opts { args, at: 0 }
    }
    fn next(&mut self) -> Option<&'a str> {
        let v = self.args.get(self.at).map(String::as_str);
        if v.is_some() {
            self.at += 1;
        }
        v
    }
    fn value(&mut self, flag: &str) -> Result<&'a str> {
        self.next().ok_or_else(|| err!("{flag} needs a value"))
    }
}

fn cmd_open(registry: &Registry, args: &[String]) -> Result<(i32, Vec<u8>)> {
    if args.len() < 2 {
        return Ok((2, b"usage: c-sh open NAME TARGET [options]\n".to_vec()));
    }
    let name = args[0].clone();
    if name.contains('/') || name.is_empty() {
        return Err(err!("a session name has no '/'"));
    }
    if registry.sessions.lock().unwrap().contains_key(&name) {
        return Err(err!("a session \"{name}\" is already open (close it first)"));
    }
    let spec = args[1].clone();

    // Options.
    let mut cols = 80u16;
    let mut rows = 24u16;
    let mut term = "xterm-256color".to_string();
    let mut password: Option<String> = None;
    let mut identity_files: Vec<String> = Vec::new();
    let mut passphrase: Option<String> = None;
    let mut use_agent = true;
    let mut agent_path: Option<String> = None;
    let mut known_hosts = home().join(".ssh/known_hosts");
    let mut mode = ssh::HostKeyMode::AcceptNew;
    let mut env: Vec<(String, String)> = Vec::new();
    let mut user_override: Option<String> = None;
    let mut command: Option<String> = None;
    let mut want_pty = true;
    let mut connect_timeout = Duration::from_secs(30);
    let mut run_argv: Vec<String> = Vec::new();
    let mut cwd: Option<String> = None;
    let mut line = LineOpts::default();

    let mut o = Opts::new(&args[2..]);
    while let Some(flag) = o.next() {
        match flag {
            "--size" => {
                let (c, r) = parse_size(o.value("--size")?)?;
                cols = c;
                rows = r;
            }
            "--term" => term = o.value("--term")?.to_string(),
            "--password" => password = Some(o.value("--password")?.to_string()),
            "--identity" | "-i" => identity_files.push(o.value(flag)?.to_string()),
            "--passphrase" => passphrase = Some(o.value("--passphrase")?.to_string()),
            "--no-agent" => use_agent = false,
            "--agent" => agent_path = Some(o.value("--agent")?.to_string()),
            "--known-hosts" => known_hosts = PathBuf::from(o.value("--known-hosts")?),
            "--accept-new" => mode = ssh::HostKeyMode::AcceptNew,
            "--strict" => mode = ssh::HostKeyMode::Strict,
            "--insecure" => mode = ssh::HostKeyMode::Insecure,
            "--user" | "-l" => user_override = Some(o.value(flag)?.to_string()),
            "--command" | "-c" => command = Some(o.value(flag)?.to_string()),
            "--no-pty" => want_pty = false,
            "--cwd" => cwd = Some(o.value("--cwd")?.to_string()),
            "--baud" => line.set("baud", o.value(flag)?)?,
            "--data-bits" => line.set("data", o.value(flag)?)?,
            "--parity" => line.set("parity", o.value(flag)?)?,
            "--stop-bits" => line.set("stop", o.value(flag)?)?,
            "--flow" => line.set("flow", o.value(flag)?)?,
            "--line" => line.set_compact(o.value(flag)?)?,
            "--connect-timeout" => connect_timeout = Duration::from_secs(o.value("--connect-timeout")?.parse().map_err(|_| err!("--connect-timeout wants seconds"))?),
            "--env" | "-e" => {
                let kv = o.value(flag)?;
                let (k, v) = kv.split_once('=').ok_or_else(|| err!("--env wants KEY=VALUE"))?;
                env.push((k.to_string(), v.to_string()));
            }
            "--" => {
                while let Some(a) = o.next() {
                    run_argv.push(a.to_string());
                }
            }
            other => return Err(err!("unknown option {other} for open")),
        }
    }
    if !env.iter().any(|(k, _)| k == "TERM") {
        env.push(("TERM".to_string(), term.clone()));
    }

    let target = Target::parse(&spec, &run_argv)?;
    let login_password = password.clone();
    let mut note = String::new();
    let mut resolved_line: Option<LineOpts> = None;
    let backend: Box<dyn Backend> = match &target {
        Target::Ssh { user, .. } => {
            let login = user_override.clone().or_else(|| user.clone()).unwrap_or_else(default_user);
            let mut identities = Vec::new();
            for f in &identity_files {
                let pem = std::fs::read(f).map_err(|e| err!("cannot read {f}: {e}"))?;
                if privatekey::is_encrypted(&pem) && passphrase.is_none() {
                    return Err(err!("{f} is encrypted; give --passphrase"));
                }
                identities.push(privatekey::load(&pem, passphrase.as_deref()).map_err(|e| err!("{f}: {e}"))?);
            }
            if identity_files.is_empty() {
                identities.extend(default_identities());
            }
            let opts = ssh::SshOptions {
                user: login,
                password,
                identities,
                use_agent,
                agent_path,
                known_hosts,
                mode,
                term: &term,
                env: env.clone(),
                command,
                want_pty,
            };
            let s = ssh::Ssh::connect(&target, cols, rows, opts, connect_timeout).map_err(|e| err!("{e}"))?;
            if let Some(outcome) = s.outcome.lock().unwrap().as_ref() {
                note = format!("host key {} — {}", outcome.fingerprint, outcome.status);
            }
            if let Some(how) = &s.auth {
                note = format!("{note}; logged in with {how}");
            }
            Box::new(s)
        }
        Target::Telnet { .. } | Target::Rlogin { .. } | Target::Raw { .. } => {
            Box::new(Tcp::connect(&target, cols, rows, &term, user_override.as_deref(), connect_timeout)?)
        }
        Target::Serial { path, line: in_target } => {
            // Options on the command line win over the target's query.
            let mut settings = in_target.clone();
            for (k, v) in [
                ("baud", line.baud.map(|b| b.to_string())),
                ("data", line.data_bits.map(|d| d.to_string())),
                ("parity", line.parity.clone()),
                ("stop", line.stop_bits.map(|s| s.to_string())),
                ("flow", line.flow.clone()),
            ] {
                if let Some(v) = v {
                    settings.set(k, &v)?;
                }
            }
            let port = Serial::open(path, &settings)?;
            note = port.detail.clone();
            resolved_line = Some(settings);
            Box::new(port)
        }
        Target::Run { argv } => Box::new(Local::spawn(argv, cols, rows, &env, cwd.as_deref())?),
    };

    // The target as it was opened (a serial line's settings with the options applied).
    let target = match (target, resolved_line) {
        (Target::Serial { path, .. }, Some(line)) => Target::Serial { path, line },
        (t, _) => t,
    };
    let session = Session::start(name.clone(), target.clone(), backend, cols, rows, note.clone());
    registry.sessions.lock().unwrap().insert(name.clone(), session.clone());
    // A text login (telnet, rlogin, serial, raw): answer the prompts when asked to.
    let login_user = match &target {
        Target::Telnet { user, .. } => user_override.clone().or_else(|| user.clone()),
        Target::Serial { .. } | Target::Raw { .. } => user_override.clone(),
        _ => None,
    };
    if !matches!(target, Target::Ssh { .. } | Target::Run { .. }) && (login_user.is_some() || login_password.is_some()) {
        let login = text_login(&session, login_user.as_deref(), login_password.as_deref(), connect_timeout);
        let what = match &login {
            Ok(()) => "logged in".to_string(),
            Err(e) => format!("login failed: {e}"),
        };
        note = if note.is_empty() { what } else { format!("{note}; {what}") };
        session.set_note(note.clone());
        if let Err(e) = login {
            // The session stays, so `c-sh read` / `screen` show what the server said.
            let screen = term::plain(&session.read_new().0);
            return Ok((1, format!("c-sh: opened {name} but the login failed: {e}\n{screen}").into_bytes()));
        }
    }
    let mut out = format!("opened {name}: {}\n", target.describe());
    if !note.is_empty() {
        out.push_str(&note);
        out.push('\n');
    }
    Ok((0, out.into_bytes()))
}

/// Answers a text login: waits for a "login:"/"username:" prompt and sends the user, then for
/// "password:" and sends the password. A prompt that does not come within the timeout is skipped
/// (a server may ask only for the password, or nothing).
fn text_login(session: &Session, user: Option<&str>, password: Option<&str>, timeout: Duration) -> Result<()> {
    let user_prompt = regex_lite::Regex::new(r"(?i)(login|username|user name)\s*:\s*$").unwrap();
    let pass_prompt = regex_lite::Regex::new(r"(?i)password\s*:\s*$").unwrap();
    let tail_matches = |re: &regex_lite::Regex, from: usize| {
        let (data, _, _) = session.read_since(from);
        let text = term::plain(&data);
        re.is_match(text.trim_end_matches('\n'))
    };
    let deadline = std::time::Instant::now() + timeout.min(Duration::from_secs(20));
    let mut from = 0;
    if let Some(user) = user {
        loop {
            if tail_matches(&user_prompt, from) {
                break;
            }
            if tail_matches(&pass_prompt, from) || std::time::Instant::now() > deadline || !session.is_open() {
                return Err(err!("no login prompt came"));
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        from = session.total();
        session.write(format!("{user}\r").as_bytes())?;
    }
    if let Some(password) = password {
        loop {
            if tail_matches(&pass_prompt, from) {
                break;
            }
            if std::time::Instant::now() > deadline || !session.is_open() {
                return Err(err!("no password prompt came"));
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        from = session.total();
        session.write(format!("{password}\r").as_bytes())?;
    }
    // Let the welcome and the first prompt arrive.
    session.wait(&Condition::Idle(Duration::from_millis(300)), Duration::from_secs(5));
    if session.read_since(from).0.windows(9).any(|w| w.eq_ignore_ascii_case(b"incorrect")) {
        return Err(err!("the server said the login was incorrect"));
    }
    Ok(())
}

fn default_user() -> String {
    std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_else(|_| "root".into())
}

/// The default key files (~/.ssh/id_*), those that load without a passphrase.
fn default_identities() -> Vec<privatekey::Identity> {
    let mut ids = Vec::new();
    for name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
        let path = home().join(".ssh").join(name);
        if let Ok(pem) = std::fs::read(&path) {
            if !privatekey::is_encrypted(&pem) {
                if let Ok(id) = privatekey::load(&pem, None) {
                    ids.push(id);
                }
            }
        }
    }
    ids
}

fn cmd_list(registry: &Registry) -> Result<(i32, Vec<u8>)> {
    let sessions = registry.sessions.lock().unwrap();
    if sessions.is_empty() {
        return Ok((0, b"no sessions (c-sh open NAME TARGET)\n".to_vec()));
    }
    let mut names: Vec<&String> = sessions.keys().collect();
    names.sort();
    let mut out = String::new();
    for name in names {
        let s = &sessions[name];
        let state = if s.is_open() {
            "open".to_string()
        } else {
            match s.exit_status() {
                Some(code) => format!("closed (exit {code})"),
                None => "closed".to_string(),
            }
        };
        let age = (now_ms().saturating_sub(s.opened_ms)) / 1000;
        out.push_str(&format!("{name}\t{}\t{state}\t{age}s\n", s.target.describe()));
    }
    Ok((0, out.into_bytes()))
}

fn cmd_send(registry: &Registry, args: &[String], enter: bool) -> Result<(i32, Vec<u8>)> {
    if args.is_empty() {
        return Ok((2, b"usage: c-sh send NAME TEXT...\n".to_vec()));
    }
    let session = registry.get(&args[0])?;
    // \n, \t, \r, \e escapes in the text, so an agent can send control characters portably.
    let mut bytes = unescape(&args[1..].join(" ")).into_bytes();
    if enter {
        bytes.push(b'\r');
    }
    session.write(&bytes)?;
    Ok((0, Vec::new()))
}

fn cmd_key(registry: &Registry, args: &[String]) -> Result<(i32, Vec<u8>)> {
    if args.len() < 2 {
        return Ok((2, b"usage: c-sh key NAME KEY...\n".to_vec()));
    }
    let session = registry.get(&args[0])?;
    session.send_keys(&args[1..])?;
    Ok((0, Vec::new()))
}

fn cmd_read(registry: &Registry, args: &[String]) -> Result<(i32, Vec<u8>)> {
    if args.is_empty() {
        return Ok((2, b"usage: c-sh read NAME [--all | --since OFFSET] [--timeout MS] [--raw]\n".to_vec()));
    }
    let session = registry.get(&args[0])?;
    let mut since: Option<usize> = None;
    let mut timeout_ms = 0u64;
    let mut raw = false;
    let mut o = Opts::new(&args[1..]);
    while let Some(f) = o.next() {
        match f {
            "--since" => since = Some(o.value("--since")?.parse().map_err(|_| err!("--since wants a byte offset"))?),
            "--all" => since = Some(0),
            "--timeout" => timeout_ms = o.value("--timeout")?.parse().map_err(|_| err!("--timeout wants milliseconds"))?,
            "--raw" => raw = true,
            other => return Err(err!("unknown option {other} for read")),
        }
    }
    // Optionally wait for something new to arrive.
    let from = since.unwrap_or_else(|| session.read_cursor());
    if timeout_ms > 0 {
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
        while session.total() <= from && session.is_open() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let (data, lost) = match since {
        Some(n) => {
            let (d, _, lost) = session.read_since(n);
            (d, lost)
        }
        None => session.read_new(),
    };
    let mut body = Vec::new();
    if lost {
        body.extend_from_slice(b"c-sh: earlier output was dropped (the last MiB is kept)\n");
    }
    if raw {
        body.extend_from_slice(&data);
    } else {
        let text = term::plain(&data);
        body.extend_from_slice(text.as_bytes());
        // A prompt has no newline after it; end the output with one so it reads as a line.
        if !text.is_empty() && !text.ends_with('\n') {
            body.push(b'\n');
        }
    }
    Ok((0, body))
}

fn cmd_screen(registry: &Registry, args: &[String]) -> Result<(i32, Vec<u8>)> {
    if args.is_empty() {
        return Ok((2, b"usage: c-sh screen NAME\n".to_vec()));
    }
    let session = registry.get(&args[0])?;
    let mut screen = session.screen();
    if !screen.ends_with('\n') {
        screen.push('\n');
    }
    Ok((0, screen.into_bytes()))
}

fn cmd_wait(registry: &Registry, args: &[String]) -> Result<(i32, Vec<u8>)> {
    if args.is_empty() {
        return Ok((2, b"usage: c-sh wait NAME [text STR | regex RE | idle MS | closed] [--timeout S]\n".to_vec()));
    }
    let session = registry.get(&args[0])?;
    let mut timeout = Duration::from_secs(30);
    let mut cond = Condition::Idle(Duration::from_millis(500));
    let mut o = Opts::new(&args[1..]);
    while let Some(w) = o.next() {
        match w {
            "text" => cond = Condition::Text(o.value("text")?.to_string()),
            "regex" => {
                let re = regex_lite::Regex::new(o.value("regex")?).map_err(|e| err!("bad regex: {e}"))?;
                cond = Condition::Regex(re);
            }
            "idle" => cond = Condition::Idle(Duration::from_millis(o.value("idle")?.parse().map_err(|_| err!("idle wants milliseconds"))?)),
            "closed" => cond = Condition::Closed,
            "--timeout" => timeout = Duration::from_secs_f64(o.value("--timeout")?.parse().map_err(|_| err!("--timeout wants seconds"))?),
            other => return Err(err!("unknown wait argument {other}")),
        }
    }
    if session.wait(&cond, timeout) {
        Ok((0, Vec::new()))
    } else {
        Ok((3, b"c-sh: timed out waiting\n".to_vec()))
    }
}

fn cmd_run(registry: &Registry, args: &[String]) -> Result<(i32, Vec<u8>)> {
    if args.len() < 2 {
        return Ok((2, b"usage: c-sh run NAME COMMAND [--timeout S]\n(sends the command and waits for the shell prompt; for shell sessions)\n".to_vec()));
    }
    let session = registry.get(&args[0])?;
    let mut timeout = Duration::from_secs(60);
    let mut cmd = String::new();
    let mut prompt: Option<regex_lite::Regex> = None;
    let mut idle: Option<Duration> = None;
    let mut o = Opts::new(&args[1..]);
    while let Some(w) = o.next() {
        match w {
            "--timeout" => timeout = Duration::from_secs_f64(o.value("--timeout")?.parse().map_err(|_| err!("--timeout wants seconds"))?),
            "--prompt" => prompt = Some(regex_lite::Regex::new(o.value("--prompt")?).map_err(|e| err!("bad --prompt regex: {e}"))?),
            "--idle" => idle = Some(Duration::from_millis(o.value("--idle")?.parse().map_err(|_| err!("--idle wants milliseconds"))?)),
            other => {
                if !cmd.is_empty() {
                    cmd.push(' ');
                }
                cmd.push_str(other);
            }
        }
    }
    if prompt.is_some() || idle.is_some() {
        return run_until(&session, &cmd, prompt.as_ref(), idle, timeout);
    }
    run_with_marker(&session, &cmd, timeout)
}

/// `run` for something that is not a POSIX shell (a router's CLI, a REPL, a BBS): sends the
/// line, then waits until the output ends in the prompt (a regex on the last line) or goes quiet
/// for `idle`. Prints what came between the echoed line and the prompt; no exit status.
fn run_until(session: &Session, cmd: &str, prompt: Option<&regex_lite::Regex>, idle: Option<Duration>, timeout: Duration) -> Result<(i32, Vec<u8>)> {
    let start = session.total();
    session.write(format!("{cmd}\r").as_bytes())?;
    let deadline = std::time::Instant::now() + timeout;
    let quiet = idle.unwrap_or(Duration::from_millis(150));
    let finished = loop {
        let (data, _, _) = session.read_since(start);
        let text = term::plain(&data);
        let last = text.trim_end_matches('\n').rsplit('\n').next().unwrap_or("");
        // Past the echo of our own line (when the far end echoes) before looking for the prompt.
        let past_echo = text.contains('\n');
        let idle_now = now_ms().saturating_sub(session.last_output_ms()) >= quiet.as_millis() as u64;
        let done = match prompt {
            Some(re) => past_echo && re.is_match(last) && idle_now,
            None => idle_now && !data.is_empty(),
        };
        if done {
            break true;
        }
        if std::time::Instant::now() > deadline || !session.is_open() {
            break false;
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let (data, _, _) = session.read_since(start);
    session.read_new();
    let text = term::plain(&data);
    // Drop the echo of the command (a first line that ends with what was typed: a prompt may
    // precede it) — a device that does not echo (a modem) answers on the first line — and, with
    // a prompt, the prompt line at the end.
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.len() > 1 && !cmd.trim().is_empty() && lines[0].trim_end().ends_with(cmd.trim()) {
        lines.remove(0);
    }
    if prompt.is_some() && finished {
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        lines.pop();
    }
    let mut body = lines.join("\n").trim_end_matches('\n').to_string().into_bytes();
    if !body.is_empty() {
        body.push(b'\n');
    }
    if finished {
        Ok((0, body))
    } else if !session.is_open() {
        let mut b = b"c-sh: the session ended\n".to_vec();
        b.extend_from_slice(&body);
        Ok((1, b))
    } else {
        let mut b = format!("c-sh: timed out after {}s waiting for the prompt; output so far:\n", timeout.as_secs()).into_bytes();
        b.extend_from_slice(&body);
        Ok((3, b))
    }
}

/// Runs a command in a shell session and returns its output and exit code. The shell prints a
/// begin marker before the command and an end marker with `$?` after it; the output is what lies
/// between. The markers are typed split by quotes, so the echo of the typed line (however the
/// terminal wraps it) never contains them; each is erased from the screen once printed (CR and
/// erase-line), so `screen` stays clean while the transcript keeps them.
fn run_with_marker(session: &Session, cmd: &str, timeout: Duration) -> Result<(i32, Vec<u8>)> {
    let tag: u64 = {
        let mut b = [0u8; 8];
        crate::crypto::random(&mut b);
        u64::from_le_bytes(b)
    };
    let begin = format!("__CSHBEGIN_{tag:016x}__");
    let end = format!("__CSHDONE_{tag:016x}__");
    let split = |m: &str| {
        let (a, b) = m.split_at(8);
        format!("\"{a}\"\"{b}\"")
    };
    let cmd = cmd.trim_end();
    // `cmd &` cannot be followed by `;`.
    let sep = if cmd.ends_with('&') && !cmd.ends_with("&&") { " " } else { "; " };
    let start = session.total();
    // A leading space keeps it out of bash's history (HISTCONTROL=ignorespace, the default on
    // many systems).
    let line = format!(
        " printf '%s\\r\\033[2K' {}; {cmd}{sep}printf '\\n%s%d\\r\\033[2K' {} \"$?\"\r",
        split(&begin),
        split(&end)
    );
    session.write(line.as_bytes())?;

    let done = session.wait(&Condition::Text(end.clone()), timeout);
    if done {
        // The exit code's digits follow the marker in the same write; give them a moment.
        session.wait(&Condition::Idle(Duration::from_millis(60)), Duration::from_secs(2));
    }
    let (data, _, _) = session.read_since(start);
    let raw = String::from_utf8_lossy(&data).into_owned();
    // The output starts after the begin marker (and its erase sequence).
    let after_begin = match raw.find(&begin) {
        Some(p) => {
            let rest = &raw[p + begin.len()..];
            rest.strip_prefix("\r\x1b[2K").unwrap_or(rest).to_string()
        }
        None => String::new(),
    };
    session.read_new();
    if !done {
        let output = term::plain(after_begin.as_bytes());
        let output = output.trim_end();
        let mut body = if !session.is_open() {
            // The command ended the shell (exit, logout): its status is the session's.
            let code = session.exit_status();
            let head = format!("c-sh: the session ended (exit {})\n", code.map(|c| c.to_string()).unwrap_or_else(|| "?".into()));
            let mut b = head.into_bytes();
            b.extend_from_slice(output.as_bytes());
            if !output.is_empty() {
                b.push(b'\n');
            }
            return Ok((code.unwrap_or(1), b));
        } else {
            format!(
                "c-sh: timed out after {}s; the command is still running (c-sh read / screen, c-sh key NAME ctrl+c). Output so far:\n",
                timeout.as_secs()
            )
            .into_bytes()
        };
        body.extend_from_slice(output.as_bytes());
        if !output.is_empty() {
            body.push(b'\n');
        }
        return Ok((3, body));
    }
    let (output, code) = extract(&after_begin, &end);
    let mut body = output.into_bytes();
    if !body.is_empty() {
        body.push(b'\n');
    }
    Ok((code.unwrap_or(0), body))
}

/// Splits the region after the begin marker at the end marker: (the output as plain text, the
/// exit code after the marker).
fn extract(text: &str, marker: &str) -> (String, Option<i32>) {
    let Some(pos) = text.find(marker) else { return (term::plain(text.as_bytes()), None) };
    let code = text[pos + marker.len()..].split(|c: char| !c.is_ascii_digit()).next().and_then(|d| d.parse().ok());
    let output = term::plain(text[..pos].as_bytes());
    (output.trim_end_matches(['\r', '\n']).to_string(), code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_output_and_status() {
        let (out, code) = extract("hello\r\nworld\r\n\n__M__7\r\x1b[2K", "__M__");
        assert_eq!(out, "hello\nworld");
        assert_eq!(code, Some(7));
        let (out, code) = extract("\n__M__0\r", "__M__");
        assert_eq!(out, "");
        assert_eq!(code, Some(0));
    }
}

fn cmd_resize(registry: &Registry, args: &[String]) -> Result<(i32, Vec<u8>)> {
    if args.len() < 2 {
        return Ok((2, b"usage: c-sh resize NAME WxH\n".to_vec()));
    }
    let session = registry.get(&args[0])?;
    let (cols, rows) = parse_size(&args[1])?;
    session.resize(cols, rows)?;
    Ok((0, Vec::new()))
}

fn cmd_info(registry: &Registry, args: &[String]) -> Result<(i32, Vec<u8>)> {
    if args.is_empty() {
        return Ok((2, b"usage: c-sh info NAME\n".to_vec()));
    }
    let session = registry.get(&args[0])?;
    let (cols, rows, cr, cc, title) = session.screen_info();
    let mut out = String::new();
    out.push_str(&format!("name:    {}\n", session.name));
    out.push_str(&format!("target:  {}\n", session.target.describe()));
    out.push_str(&format!("via:     {}\n", session.kind()));
    let detail = session.detail();
    if !detail.is_empty() {
        out.push_str(&format!("reaches: {detail}\n"));
    }
    out.push_str(&format!("state:   {}\n", if session.is_open() { "open".into() } else { format!("closed (exit {})", session.exit_status().map(|c| c.to_string()).unwrap_or_else(|| "?".into())) }));
    out.push_str(&format!("size:    {cols}x{rows}\n"));
    out.push_str(&format!("cursor:  row {cr}, col {cc}\n"));
    if !title.is_empty() {
        out.push_str(&format!("title:   {title}\n"));
    }
    out.push_str(&format!("output:  {} bytes\n", session.total()));
    let idle = now_ms().saturating_sub(session.last_output_ms()) / 1000;
    out.push_str(&format!("idle:    {idle}s\n"));
    let note = session.note();
    if !note.is_empty() {
        out.push_str(&format!("note:    {note}\n"));
    }
    Ok((0, out.into_bytes()))
}

fn cmd_close(registry: &Registry, args: &[String]) -> Result<(i32, Vec<u8>)> {
    if args.is_empty() {
        return Ok((2, b"usage: c-sh close NAME\n".to_vec()));
    }
    let name = &args[0];
    let session = { registry.sessions.lock().unwrap().remove(name) };
    match session {
        Some(s) => {
            s.close().ok();
            Ok((0, format!("closed {name}\n").into_bytes()))
        }
        None => Ok((0, format!("no session \"{name}\"\n").into_bytes())),
    }
}

fn cmd_logs(registry: &Registry, args: &[String]) -> Result<(i32, Vec<u8>)> {
    if args.is_empty() {
        return Ok((2, b"usage: c-sh logs NAME [--raw]\n".to_vec()));
    }
    let session = registry.get(&args[0])?;
    let data = session.transcript();
    if args.iter().any(|a| a == "--raw") {
        Ok((0, data))
    } else {
        Ok((0, term::plain(&data).into_bytes()))
    }
}

fn cmd_break(registry: &Registry, args: &[String]) -> Result<(i32, Vec<u8>)> {
    if args.is_empty() {
        return Ok((2, b"usage: c-sh break NAME [MS]\n".to_vec()));
    }
    let session = registry.get(&args[0])?;
    let ms = match args.get(1) {
        Some(m) => m.parse().map_err(|_| err!("break: milliseconds, e.g. 250"))?,
        None => 250,
    };
    let said = session.control(&Control::Break(ms))?;
    Ok((0, format!("{said}\n").into_bytes()))
}

fn cmd_line(registry: &Registry, args: &[String]) -> Result<(i32, Vec<u8>)> {
    if args.is_empty() {
        return Ok((2, b"usage: c-sh line NAME [baud=N] [data=5-8] [parity=none|even|odd|mark|space] [stop=1|2] [flow=none|rtscts|xonxoff] [dtr=on|off] [rts=on|off]\n".to_vec()));
    }
    let session = registry.get(&args[0])?;
    let mut line = LineOpts::default();
    let (mut dtr, mut rts) = (None, None);
    let on_off = |v: &str| match v.to_ascii_lowercase().as_str() {
        "on" | "1" | "true" | "high" => Ok(true),
        "off" | "0" | "false" | "low" => Ok(false),
        _ => Err(err!("dtr and rts are on or off")),
    };
    for a in &args[1..] {
        let (k, v) = a.split_once('=').ok_or_else(|| err!("line settings are KEY=VALUE, e.g. baud=9600"))?;
        match k {
            "dtr" => dtr = Some(on_off(v)?),
            "rts" => rts = Some(on_off(v)?),
            _ => line.set(k, v)?,
        }
    }
    let said = session.control(&Control::Line { line, dtr, rts })?;
    Ok((0, format!("{said}\n").into_bytes()))
}

fn cmd_forget(args: &[String]) -> Result<(i32, Vec<u8>)> {
    if args.is_empty() {
        return Ok((2, b"usage: c-sh forget HOST[:PORT] [--known-hosts FILE]\n".to_vec()));
    }
    let entry = &args[0];
    let mut path = home().join(".ssh/known_hosts");
    let mut o = Opts::new(&args[1..]);
    while let Some(f) = o.next() {
        match f {
            "--known-hosts" => path = PathBuf::from(o.value("--known-hosts")?),
            other => return Err(err!("unknown option {other} for forget")),
        }
    }
    let removed = crate::ssh::hostkey::forget_known(&path, entry)?;
    Ok((0, format!("removed {removed} entr{} for {entry}\n", if removed == 1 { "y" } else { "ies" }).into_bytes()))
}

fn parse_size(s: &str) -> Result<(u16, u16)> {
    let (c, r) = s.split_once(['x', 'X']).ok_or_else(|| err!("a size is WxH, e.g. 120x40"))?;
    Ok((c.parse().map_err(|_| err!("bad width in {s}"))?, r.parse().map_err(|_| err!("bad height in {s}"))?))
}

/// Turns \n \r \t \\ \xNN escapes in `send` text into their bytes.
fn unescape(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some('e') => out.push('\x1b'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}
