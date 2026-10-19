//! A local program in a pseudo-terminal: the login shell by default, or any command. Useful on
//! its own (a persistent shell the agent drives) and as the model the other backends follow.

use std::io::Read;
use std::os::fd::AsRawFd;

use super::Backend;
use crate::sys::{self, FdStream, Pty};
use crate::{err, Result};

pub struct Local {
    master: FdStream,
    pid: libc::pid_t,
    exit: Option<i32>,
    open: bool,
}

impl Local {
    pub fn spawn(argv: &[String], cols: u16, rows: u16, env: &[(String, String)], cwd: Option<&str>) -> Result<Local> {
        let mut argv = if argv.is_empty() {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            vec![shell]
        } else {
            argv.to_vec()
        };
        // posix_spawn has no portable "change directory" action: a shell does the cd, then
        // replaces itself with the program (argv unchanged, quoting untouched).
        if let Some(dir) = cwd {
            if std::path::Path::new(dir).is_dir() {
                let mut wrapped = vec!["/bin/sh".to_string(), "-c".into(), "cd \"$0\" && exec \"$@\"".into(), dir.to_string()];
                wrapped.append(&mut argv);
                argv = wrapped;
            }
        }
        let pty = Pty::open()?;
        pty.set_size(cols, rows);
        let mut full_env = env.to_vec();
        if !full_env.iter().any(|(k, _)| k == "TERM") {
            full_env.push(("TERM".into(), "xterm-256color".into()));
        }
        let pid = sys::spawn_on_pty(&pty, &argv, &full_env)?;
        let master = FdStream::new(pty.master);
        master.set_nonblocking(true)?;
        Ok(Local { master, pid, exit: None, open: true })
    }
}

impl Backend for Local {
    fn fd(&self) -> std::os::fd::RawFd {
        self.master.as_raw_fd()
    }

    fn read(&mut self) -> Result<Vec<u8>> {
        let mut buf = [0u8; 16384];
        match self.master.read(&mut buf) {
            Ok(0) => {
                self.ended();
                Ok(Vec::new())
            }
            Ok(n) => Ok(buf[..n].to_vec()),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(Vec::new()),
            // The master returns EIO once the child has gone and the slave is closed.
            Err(e) if e.raw_os_error() == Some(libc::EIO) => {
                self.ended();
                Ok(Vec::new())
            }
            Err(e) => Err(err!("reading the program's output: {e}")),
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        use std::io::Write;
        let mut at = 0;
        while at < data.len() {
            match self.master.write(&data[at..]) {
                Ok(0) => return Err(err!("the program's input is closed")),
                Ok(n) => at += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    sys::wait_readable(self.master.as_raw_fd(), 50).ok();
                }
                Err(e) => return Err(err!("writing to the program: {e}")),
            }
        }
        Ok(())
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        sys::set_winsize(self.master.as_raw_fd(), cols, rows);
        Ok(())
    }

    fn tick(&mut self) {
        self.poll_exit();
    }

    fn is_open(&self) -> bool {
        self.open
    }

    fn exit_status(&self) -> Option<i32> {
        self.exit
    }

    fn close(&mut self) -> Result<()> {
        // SAFETY: signalling our own child; harmless if it is already gone.
        unsafe {
            libc::kill(self.pid, libc::SIGHUP);
        }
        self.open = false;
        Ok(())
    }

    fn kind(&self) -> &'static str {
        "local"
    }
}

impl Local {
    /// The pty hung up: the program is exiting, so reap it (briefly waiting) to learn its status.
    fn ended(&mut self) {
        self.open = false;
        for _ in 0..100 {
            self.poll_exit();
            if self.exit.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Reaps the child if it has exited, recording its status.
    pub fn poll_exit(&mut self) {
        if self.exit.is_none() {
            if let Some(status) = sys::try_wait(self.pid) {
                self.exit = Some(status);
                self.open = false;
            }
        }
    }
}

impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: best-effort cleanup of our child.
        unsafe {
            libc::kill(self.pid, libc::SIGHUP);
            let mut status = 0;
            libc::waitpid(self.pid, &mut status, libc::WNOHANG);
        }
    }
}
