//! The SSH backend: a [crate::ssh] session (transport, auth, one channel with a pty) presented as
//! a [Backend]. Host keys are checked against known_hosts with a policy; keys and the agent come
//! from the options.

use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{Backend, Target};
use crate::ssh::{self, agent::Agent, channel::Channel, channel::Command, hostkey, hostkey::PublicKey, privatekey::Identity};
use crate::sys::{self, FdStream};
use crate::{err, Result};

/// What to do about a host key that known_hosts does not confirm.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HostKeyMode {
    /// Refuse an unknown or changed key (the safe default once a key is known).
    Strict,
    /// Trust a key seen for the first time and remember it (TOFU); still refuse a changed one.
    AcceptNew,
    /// Accept any key, remembering it. For throwaway hosts; says so in `info`.
    Insecure,
}

/// The host-key decision, and what known_hosts recorded (for the agent to see).
pub struct HostKeyOutcome {
    pub fingerprint: String,
    pub status: String,
}

pub struct SshOptions<'a> {
    pub user: String,
    pub password: Option<String>,
    pub identities: Vec<Identity>,
    pub use_agent: bool,
    pub agent_path: Option<String>,
    pub known_hosts: std::path::PathBuf,
    pub mode: HostKeyMode,
    pub term: &'a str,
    pub env: Vec<(String, String)>,
    /// Run a single command instead of a shell (then the session ends when it does).
    pub command: Option<String>,
    pub want_pty: bool,
}

pub struct Ssh {
    channel: Channel<'static>,
    poll_fd: std::os::fd::OwnedFd,
    open: bool,
    pub outcome: Arc<Mutex<Option<HostKeyOutcome>>>,
    /// How authentication succeeded (method and key).
    pub auth: Option<String>,
}

impl Ssh {
    pub fn connect(target: &Target, cols: u16, rows: u16, opts: SshOptions, timeout: Duration) -> Result<Ssh> {
        let (host, port) = match target {
            Target::Ssh { host, port, .. } => (host.clone(), *port),
            _ => return Err(err!("not an SSH target")),
        };
        let stream = sys::tcp_connect(&host, port, timeout)?;
        stream.set_nodelay(true).ok();
        let fd = sys::tcp_into_fd(stream);
        let poll_fd = sys::dup(fd.as_raw_fd())?;
        let (read_half, write_half) = FdStream::split(fd)?;

        let entry = hostkey::host_entry(&host, port);
        let known = opts.known_hosts.clone();
        let mode = opts.mode;
        let outcome = Arc::new(Mutex::new(None));
        let outcome_for_verify = outcome.clone();
        let verify: ssh::transport::HostKeyVerifier = Box::new(move |key: &PublicKey| {
            let fingerprint = key.fingerprint();
            let decision = check_host_key(&known, &entry, key, mode)?;
            *outcome_for_verify.lock().unwrap() = Some(HostKeyOutcome { fingerprint, status: decision });
            Ok(())
        });

        let mut agent = None;
        if opts.use_agent {
            agent = Agent::connect(opts.agent_path.as_deref()).ok();
        }
        let mut creds = ssh::auth::Credentials::new(&opts.user);
        creds.password = opts.password;
        creds.identities = opts.identities;
        creds.agent = agent;

        let command = match &opts.command {
            Some(c) => Command::Exec(c.clone()),
            None => Command::Shell,
        };
        let pty = if opts.want_pty { Some((cols, rows, opts.term)) } else { None };
        let channel = ssh::connect(Box::new(read_half), Box::new(write_half), verify, &mut creds, command, pty, &opts.env)?;

        Ok(Ssh { channel, poll_fd, open: true, outcome, auth: creds.used.take() })
    }
}

/// Looks the key up and decides, recording a new or insecure key.
fn check_host_key(known: &std::path::Path, entry: &str, key: &PublicKey, mode: HostKeyMode) -> Result<String> {
    use hostkey::Known;
    match hostkey::check_known(known, &[entry.to_string()], key) {
        Known::Match => Ok("known".into()),
        Known::Revoked => Err(err!("the host key {} is marked revoked in {}", key.fingerprint(), known.display())),
        Known::Changed { line } => {
            if mode == HostKeyMode::Insecure {
                Ok(format!("CHANGED (accepted insecurely; was line {line})"))
            } else {
                Err(err!(
                    "the host key for {entry} has CHANGED — {} is not what {} line {line} records. If the server was \
                     rebuilt, remove the old key (c-sh forget {entry}); otherwise this may be an interception.",
                    key.fingerprint(),
                    known.display(),
                ))
            }
        }
        Known::Unknown { other_types } => match mode {
            HostKeyMode::Strict => Err(err!(
                "the host key for {entry} ({}) is not in {}. Add it (c-sh known-hosts add) or open with \
                 --accept-new to trust it on first sight.{}",
                key.fingerprint(),
                known.display(),
                if other_types { " (keys of other types are known for this host.)" } else { "" }
            )),
            HostKeyMode::AcceptNew | HostKeyMode::Insecure => {
                hostkey::add_known(known, entry, key)?;
                Ok("new (trusted on first sight and remembered)".into())
            }
        },
    }
}

impl Backend for Ssh {
    fn fd(&self) -> std::os::fd::RawFd {
        self.poll_fd.as_raw_fd()
    }

    fn read(&mut self) -> Result<Vec<u8>> {
        // The channel hands back what it buffered even after it closed.
        let data = self.channel.read()?;
        if !self.channel.is_open() {
            self.open = false;
        }
        Ok(data)
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        self.channel.write(data)
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.channel.resize(cols, rows)
    }

    fn is_open(&self) -> bool {
        self.open && self.channel.is_open()
    }

    fn control(&mut self, op: &super::Control) -> Result<String> {
        match op {
            super::Control::Break(ms) => {
                self.channel.send_break(*ms)?;
                Ok(format!("sent a break ({ms} ms) to the server"))
            }
            super::Control::Line { .. } => Err(err!("line settings are for serial sessions, not ssh")),
        }
    }

    fn pending(&self) -> bool {
        self.channel.has_pending()
    }

    fn exit_status(&self) -> Option<i32> {
        self.channel.exit_status()
    }

    fn close(&mut self) -> Result<()> {
        self.open = false;
        self.channel.close()
    }

    fn kind(&self) -> &'static str {
        "ssh"
    }
}
