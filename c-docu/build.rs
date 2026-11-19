// The preset shape geometry docu draws with (sources.lock: preset-shapes), from DOCU_SHAPES or
// build/dl, copied where src/render/geom.rs includes it.
fn main() {
    println!("cargo:rerun-if-env-changed=DOCU_SHAPES");
    if std::env::var("CARGO_FEATURE_RENDER").is_err() {
        return;
    }
    let src = std::env::var("DOCU_SHAPES").unwrap_or_else(|_| format!("{}/build/dl/preset-shapes-presetShapeDefinitions.xml", std::env::var("CARGO_MANIFEST_DIR").unwrap()));
    println!("cargo:rerun-if-changed={src}");
    let out = format!("{}/presetShapeDefinitions.xml", std::env::var("OUT_DIR").unwrap());
    std::fs::copy(&src, &out).unwrap_or_else(|e| panic!("{src}: {e} (run build.sh, which fetches it, or set DOCU_SHAPES)"));
}
