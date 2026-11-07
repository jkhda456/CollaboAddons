// A build script: runs on the guest, writes code into OUT_DIR, sets a cfg and an env var.
fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    std::fs::write(format!("{out}/generated.rs"), "pub const GENERATED: u32 = 7 * 6;\n").unwrap();
    println!("cargo::rustc-check-cfg=cfg(from_build_script)");
    println!("cargo::rustc-cfg=from_build_script");
    println!("cargo::rustc-env=BUILD_TARGET={}", std::env::var("TARGET").unwrap());
    println!("cargo::rerun-if-changed=build.rs");
}
