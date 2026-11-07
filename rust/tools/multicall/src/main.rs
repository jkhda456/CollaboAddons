//! rustc, rustdoc, clippy-driver or rustfmt, by the name this program is run as (a link to it).
#![feature(rustc_private)]
#![allow(unused_crate_dependencies)]

extern crate rustc_driver;

rustc_driver::override_c_allocator_in_binary!();

#[path = "../../clippy/src/driver.rs"]
mod clippy_driver;
#[path = "../../rustfmt/src/bin/main.rs"]
mod rustfmt_bin;

use std::process::ExitCode;

fn main() -> ExitCode {
    let argv0 = std::env::args_os().next().unwrap_or_default();
    let name = std::path::Path::new(&argv0).file_name().and_then(|n| n.to_str()).unwrap_or("rustc");
    match name {
        "rustdoc" => rustdoc::main(),
        "clippy-driver" => clippy_driver::main(),
        "rustfmt" => {
            rustfmt_bin::main();
            ExitCode::SUCCESS
        }
        _ => rustc_driver::main(),
    }
}
