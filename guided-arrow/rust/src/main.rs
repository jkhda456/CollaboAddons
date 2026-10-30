//! guided-arrow: verified command-line cheat sheets — tldr pages and the sheets cheat.sh serves —
//! carried in an archive of its own compression, for people and agents (`guided-arrow help`).

mod archive;
mod cli;
mod codec;
mod json;
mod local;
mod pack;
mod page;
mod search;

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
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(cli::main(args));
}
