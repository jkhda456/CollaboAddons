//! c-sh (collabo sh): terminal sessions that stay connected — ssh, telnet, rlogin, raw TCP, a
//! serial line, or a program in a terminal here — held by a small daemon, and driven from the
//! command line one command at a time (`c-sh help`).

mod crypto;
mod util;
mod sys;
mod ssh;
mod backend;
mod term;
mod session;
mod proto;
mod daemon;
mod cli;

/// An error, as text for the person or agent reading it.
#[derive(Debug, Clone, PartialEq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[macro_export]
macro_rules! err {
    ($($t:tt)*) => { $crate::Error(format!($($t)*)) };
}

fn main() {
    // A reader that went away is an error to report, not a signal that kills us.
    // SAFETY: setting a signal disposition.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(cli::main(args));
}
