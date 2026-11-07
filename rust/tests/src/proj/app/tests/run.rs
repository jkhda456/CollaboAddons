#[test]
fn binary_runs() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_app")).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("answer: 42"));
}
