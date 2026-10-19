//! A session: one backend and its terminal state, held open by a reader thread that pumps output
//! into the terminal while the daemon's command threads send input and read the state. Everything
//! the two threads share is behind one mutex, so backend calls never overlap.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::backend::{Backend, Target};
use crate::term::{self, Terminal};
use crate::util::now_ms;
use crate::Result;

/// The mutable core, guarded so the reader and command threads never touch the backend at once.
struct Inner {
    backend: Box<dyn Backend>,
    term: Terminal,
}

pub struct Session {
    pub name: String,
    pub target: Target,
    pub opened_ms: u64,
    inner: Mutex<Inner>,
    running: AtomicBool,
    /// Where the default `read` (new output since the last read) continues from.
    read_cursor: std::sync::atomic::AtomicUsize,
    /// Absolute time (ms) of the last output, for idle waits and info.
    last_output_ms: AtomicU64,
    /// Set once the backend has closed; the exit status if known.
    closed: AtomicBool,
    exit: Mutex<Option<i32>>,
    /// A note for `info` (the host-key decision, errors).
    note: Mutex<String>,
}

/// What `wait` waits for.
pub enum Condition {
    /// No new output for this long.
    Idle(Duration),
    /// The transcript (recent output) contains this text.
    Text(String),
    /// A regular expression matches the recent output.
    Regex(regex_lite::Regex),
    /// The backend has closed.
    Closed,
}

impl Session {
    pub fn start(name: String, target: Target, backend: Box<dyn Backend>, cols: u16, rows: u16, note: String) -> Arc<Session> {
        let session = Arc::new(Session {
            name,
            target,
            opened_ms: now_ms(),
            inner: Mutex::new(Inner { backend, term: Terminal::new(cols, rows) }),
            running: AtomicBool::new(true),
            read_cursor: std::sync::atomic::AtomicUsize::new(0),
            last_output_ms: AtomicU64::new(now_ms()),
            closed: AtomicBool::new(false),
            exit: Mutex::new(None),
            note: Mutex::new(note),
        });
        let reader = session.clone();
        std::thread::Builder::new()
            .name(format!("csh-{}", session.name))
            .spawn(move || reader.pump())
            .expect("spawn session reader");
        session
    }

    /// The reader loop: wait for output (no lock), then take it under the lock.
    fn pump(self: Arc<Session>) {
        let fd = {
            let inner = self.inner.lock().unwrap();
            inner.backend.fd()
        };
        let mut busy = false;
        while self.running.load(Ordering::Relaxed) {
            // Buffered output first; else wait (without the lock) for the descriptor.
            let readable = busy || crate::sys::wait_readable(fd, 200).unwrap_or(false);
            let mut inner = self.inner.lock().unwrap();
            inner.backend.tick();
            if readable {
                match inner.backend.read() {
                    Ok(data) if !data.is_empty() => {
                        inner.term.feed(&data);
                        let replies = inner.term.take_replies();
                        if !replies.is_empty() {
                            let _ = inner.backend.write(&replies);
                        }
                        self.last_output_ms.store(now_ms(), Ordering::Relaxed);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        if inner.backend.is_open() {
                            *self.note.lock().unwrap() = format!("connection lost: {e}");
                        }
                        self.mark_closed(&mut inner);
                        break;
                    }
                }
            }
            busy = inner.backend.pending();
            if !inner.backend.is_open() && !busy {
                self.mark_closed(&mut inner);
                break;
            }
        }
    }

    fn mark_closed(&self, inner: &mut Inner) {
        if !self.closed.swap(true, Ordering::Relaxed) {
            *self.exit.lock().unwrap() = inner.backend.exit_status();
            self.running.store(false, Ordering::Relaxed);
        }
    }

    pub fn is_open(&self) -> bool {
        !self.closed.load(Ordering::Relaxed)
    }

    pub fn exit_status(&self) -> Option<i32> {
        *self.exit.lock().unwrap()
    }

    pub fn note(&self) -> String {
        self.note.lock().unwrap().clone()
    }

    pub fn set_note(&self, note: String) {
        *self.note.lock().unwrap() = note;
    }

    /// Sends raw input.
    pub fn write(&self, data: &[u8]) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        inner.backend.write(data)
    }

    /// Sends a list of key names (see [term::key_to_bytes]).
    pub fn send_keys(&self, keys: &[String]) -> Result<()> {
        let mut bytes = Vec::new();
        for k in keys {
            bytes.extend_from_slice(&term::key_to_bytes(k)?);
        }
        self.write(&bytes)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        inner.term.resize(cols, rows);
        inner.backend.resize(cols, rows)
    }

    /// Output from absolute offset `since`: (bytes, new end offset, whether earlier output was lost).
    pub fn read_since(&self, since: usize) -> (Vec<u8>, usize, bool) {
        let inner = self.inner.lock().unwrap();
        inner.term.since(since)
    }

    /// New output since the last call (the session's own read cursor), advancing it.
    pub fn read_new(&self) -> (Vec<u8>, bool) {
        let since = self.read_cursor.load(Ordering::Relaxed);
        let (data, end, lost) = self.read_since(since);
        self.read_cursor.store(end, Ordering::Relaxed);
        (data, lost)
    }

    pub fn read_cursor(&self) -> usize {
        self.read_cursor.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> usize {
        self.inner.lock().unwrap().term.total()
    }

    pub fn screen(&self) -> String {
        self.inner.lock().unwrap().term.screen()
    }

    pub fn transcript(&self) -> Vec<u8> {
        self.inner.lock().unwrap().term.transcript().to_vec()
    }

    /// (cols, rows, cursor_row, cursor_col, title)
    pub fn screen_info(&self) -> (u16, u16, u16, u16, String) {
        let inner = self.inner.lock().unwrap();
        let (cols, rows) = inner.term.size();
        let (cr, cc) = inner.term.cursor();
        (cols, rows, cr, cc, inner.term.title())
    }

    /// A line control (break, serial settings).
    pub fn control(&self, op: &crate::backend::Control) -> Result<String> {
        self.inner.lock().unwrap().backend.control(op)
    }

    pub fn detail(&self) -> String {
        self.inner.lock().unwrap().backend.detail()
    }

    /// What carries the session (ssh, telnet, rlogin, raw, serial, local).
    pub fn kind(&self) -> &'static str {
        self.inner.lock().unwrap().backend.kind()
    }

    pub fn last_output_ms(&self) -> u64 {
        self.last_output_ms.load(Ordering::Relaxed)
    }

    /// Waits until the condition holds or the deadline passes. Returns true if it held.
    pub fn wait(&self, cond: &Condition, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let held = match cond {
                Condition::Closed => !self.is_open(),
                Condition::Idle(gap) => {
                    let idle = now_ms().saturating_sub(self.last_output_ms());
                    idle >= gap.as_millis() as u64
                }
                Condition::Text(needle) => {
                    let inner = self.inner.lock().unwrap();
                    contains(inner.term.transcript(), needle.as_bytes())
                }
                Condition::Regex(re) => {
                    let inner = self.inner.lock().unwrap();
                    re.is_match(&String::from_utf8_lossy(inner.term.transcript()))
                }
            };
            if held {
                return true;
            }
            if Instant::now() >= deadline || (!self.is_open() && !matches!(cond, Condition::Closed)) {
                // A closed session will produce no more output; one last check already happened.
                return match cond {
                    Condition::Closed => !self.is_open(),
                    _ => false,
                };
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    pub fn close(&self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        let mut inner = self.inner.lock().unwrap();
        let r = inner.backend.close();
        self.closed.store(true, Ordering::Relaxed);
        r
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}
