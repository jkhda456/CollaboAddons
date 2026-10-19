//! Links OpenSSL's libcrypto: the guest's static library (python/deps/lib/libcrypto.a, the one
//! python, curl and git link) when C_SH_OPENSSL_LIB names its folder, else this machine's own
//! libcrypto 3 (native builds, for the tests).
fn main() {
    println!("cargo:rerun-if-env-changed=C_SH_OPENSSL_LIB");
    let target = std::env::var("TARGET").unwrap_or_default();
    match std::env::var("C_SH_OPENSSL_LIB") {
        Ok(dir) if !dir.is_empty() => {
            println!("cargo:rustc-link-search=native={dir}");
            println!("cargo:rustc-link-lib=static=crypto");
        }
        _ if target.starts_with("wasm32") => {
            panic!("C_SH_OPENSSL_LIB: the folder of the guest's libcrypto.a (python/deps/lib)")
        }
        // Distributions ship libcrypto.so.3 without the libcrypto.so of the -dev package.
        _ if target.contains("linux") => println!("cargo:rustc-link-arg=-l:libcrypto.so.3"),
        _ => println!("cargo:rustc-link-lib=dylib=crypto"),
    }
}
