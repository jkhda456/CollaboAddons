// For the tools' mains as modules of this program (patches/rust/0009): the cfg they take, and what
// their own build scripts give them — clippy-driver its name and version (clippy's build script asks
// git, which a source tarball has not), rustfmt its version and OUT_DIR/commit-info.txt.
use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(collabo_multicall)");
    println!("cargo::rustc-cfg=collabo_multicall");
    let hash = env::var("CFG_VER_HASH").unwrap_or_default();
    let hash = &hash[..hash.len().min(10)];
    let date = env::var("CFG_VER_DATE").unwrap_or_default();
    let channel = env::var("CFG_RELEASE_CHANNEL").unwrap_or("stable".into());
    // clippy-driver --version: rustc_tools_util's VersionInfo of this crate (0.1.99, as clippy's)
    println!("cargo::rustc-env=CARGO_PKG_NAME=clippy");
    println!("cargo::rustc-env=GIT_HASH={hash}");
    println!("cargo::rustc-env=COMMIT_DATE={date}");
    println!("cargo::rustc-env=RUSTC_RELEASE_CHANNEL={channel}");
    // rustfmt --version: rustfmt's own package version, and what its build script writes
    let manifest = std::fs::read_to_string("../rustfmt/Cargo.toml").unwrap();
    let version = manifest
        .lines()
        .find_map(|l| l.strip_prefix("version = \"").and_then(|v| v.strip_suffix('"')))
        .unwrap();
    println!("cargo::rustc-env=COLLABO_RUSTFMT_VERSION={version}");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let info = if hash.is_empty() { String::new() } else { format!("{channel} ({hash} {date})") };
    std::fs::write(out.join("commit-info.txt"), info).unwrap();
    println!("cargo::rerun-if-env-changed=CFG_VER_HASH");
    println!("cargo::rerun-if-changed=../rustfmt/Cargo.toml");
}
