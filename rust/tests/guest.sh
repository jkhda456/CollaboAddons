#!/bin/sh
# The checks' guest half (test_guest.py runs it, with tests/ at /work): each step's output after
# "### NAME", then "### status N". Works in a copy of src/ in /tmp, offline.
rm -rf /tmp/rusttest && mkdir -p /tmp/rusttest && cd /tmp/rusttest && cp -r /work/src/* . || exit 1
export CARGO_HOME=/tmp/rusttest/cargo-home CARGO_NET_OFFLINE=true

tools() {
  rustc -vV
  cargo -V
  for t in rustc cargo rustdoc rustfmt clippy-driver cargo-clippy cargo-fmt; do
    command -v $t >/dev/null || echo "missing $t"
  done
  test -x "$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/rust-lld" || echo "missing rust-lld"
  rustdoc -V; rustfmt -V; clippy-driver -V
  rustc --print sysroot
  rustc --print target-list | grep -c .
}
hello() { rustc hello.rs -o hello && ./hello; }
# unwinding, threads, processes (spawn without fork), files: tests/src/smoke.rs
smoke() { rustc --edition 2024 -O smoke.rs -o smoke && ./smoke a b 2>/dev/null; }
uncaught() { ./smoke panic >/dev/null 2>&1; echo "exit $?"; }
# a compile error: diagnostics, and exit status 1 (rustc's fatal errors unwind; nothing aborts)
compile_error() {
  printf 'fn main() { let x: u32 = "no"; }\n' > bad.rs
  rustc bad.rs 2>&1 | grep -E '^error\[E0308\]|aborting'
  rustc bad.rs >/dev/null 2>&1; echo "exit $?"
}
# a cargo workspace: a proc macro (a side module rustc loads with dlopen), a build script, path
# dependencies, a binary
cargo_build() { cd proj && cargo build 2>&1 | tail -1 && ./target/debug/app; }
cargo_release() { cd proj && cargo build --release -q && ./target/release/app; }
# unit tests, an integration test, doc tests (rustdoc)
cargo_test() { cd proj && cargo test -q 2>&1 | grep -E '^test result|passed|failed' ; }
# a proc macro that panics: an error rustc reports, not a crash
macro_panic() {
  cd proj && lib="$(ls target/debug/deps/libderive-*.so | head -1)" && \
  printf 'derive::fails!();\nfn main() {}\n' > /tmp/rusttest/fails.rs && \
  rustc --edition 2024 --extern derive="$lib" /tmp/rusttest/fails.rs -o /tmp/rusttest/fails 2>&1 \
    | grep -E 'proc macro panicked|this macro always fails'
  rustc --edition 2024 --extern derive="$lib" /tmp/rusttest/fails.rs -o /tmp/rusttest/fails >/dev/null 2>&1
  echo "exit $?"
}
cargo_new() {
  cargo new -q --vcs none newbin && cd newbin && cargo run -q
}

# With the network (NET=1: crates.io allowed): serde's derive (syn, quote: proc macros from crates.io)
net_serde() {
  unset CARGO_NET_OFFLINE
  cargo new -q --vcs none netserde && cd netserde && cargo add -q serde --features derive && cargo add -q serde_json \
    && cp /work/src/serde_main.rs src/main.rs && cargo run -q
}
# With the llvm add-on too (LLVM=1): C in a build script (the cc crate), and a -sys crate (zstd)
c_build() {
  unset CARGO_NET_OFFLINE
  cargo new -q --vcs none cdemo && cd cdemo && cargo add -q --build cc && mkdir -p csrc \
    && printf 'int c_add(int a, int b) { return a * 100 + b; }\n' > csrc/add.c \
    && printf 'fn main() { cc::Build::new().file("csrc/add.c").compile("add"); }\n' > build.rs \
    && printf 'unsafe extern "C" { fn c_add(a: i32, b: i32) -> i32; }\nfn main() { println!("from C: {}", unsafe { c_add(4, 2) }); }\n' > src/main.rs \
    && cargo run -q && cd .. && cargo new -q --vcs none zs && cd zs && cargo add -q zstd \
    && cp /work/src/zstd_main.rs src/main.rs && cargo run -q
}
# clippy and rustfmt through cargo
clippy_fmt() {
  cd newbin && printf 'fn main() {\n    let v = vec![1, 2, 3];\n    if v.len() == 0 { println!("empty"); }\n    println!("{}",  v[0]);\n}\n' > src/main.rs
  cargo clippy -q 2>&1 | grep -o 'clippy::len_zero' | head -1
  cargo fmt && cargo fmt --check && echo "formatted"
}

steps="tools hello smoke uncaught compile_error cargo_build cargo_release cargo_test macro_panic cargo_new clippy_fmt"
[ -n "$NET" ] && steps="$steps net_serde"
[ -n "$LLVM" ] && steps="$steps c_build"
for s in $steps; do
  echo "### $s"
  ( $s ) 2>&1
  echo "### status $?"
done
