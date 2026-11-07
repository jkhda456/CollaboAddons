#!/usr/bin/env python3
"""The rust add-on in the guest: guest.sh runs every step in one machine (tests/ at /work, no
network), this script checks what each printed. Run by tests/run.sh.

The guest is the packaged runtime (COLLABO_RUNTIME, default dist/runtime/collabo-core-<platform>)
with out/rust.cpio (RUST_CPIO); RUST_ENGINE picks another engine. NET=1 adds a step with crates.io
(serde, serde_json: the network allowed to crates.io's hosts); LLVM=1 one with the llvm add-on too
(LLVM_CPIO, default ../llvm/out/llvm.cpio): C in a build script, the zstd crate.
"""
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import time

T = os.path.dirname(os.path.abspath(__file__))
A = os.path.dirname(T)
ROOT = os.path.dirname(os.path.dirname(A))
PLATFORM = {("Linux", "x86_64"): "linux-x64", ("Linux", "aarch64"): "linux-arm64",
            ("Darwin", "arm64"): "darwin-arm64", ("Darwin", "x86_64"): "darwin-x64"}.get(
                (platform.system(), platform.machine()), "linux-x64")
RT = os.environ.get("COLLABO_RUNTIME") or os.path.join(ROOT, "dist/runtime/collabo-core-" + PLATFORM)
ENGINE = os.environ.get("RUST_ENGINE") or os.path.join(RT, "bin/collabo-core-engine")
CPIO = os.environ.get("RUST_CPIO") or os.path.join(A, "out/rust.cpio")
LLVM_CPIO = os.environ.get("LLVM_CPIO") or os.path.join(os.path.dirname(A), "llvm/out/llvm.cpio")
NET = bool(os.environ.get("NET"))
LLVM = bool(os.environ.get("LLVM"))

# what each step prints, line for line (a compiled regular expression: matched against the line)
EXPECTED = {
    "hello": ["hello, world"],
    "smoke": [
        "hello from rust 3",
        "drop deep 0", "drop deep n", "drop deep n",
        "caught: true",
        "payload: Some(7)",
        "thread: 6 main tl 1",
        "join of panicked thread: true",
        "spawned",
        "vfork bar /tmp",
        "missing program: Some(NotFound)",
        "env after: FOO=None",
        "file: 12345",
        "slept: true",
    ],
    "uncaught": ["exit 101"],
    "compile_error": ["error[E0308]: mismatched types", "error: aborting due to 1 previous error", "exit 1"],
    "cargo_build": [re.compile(r"\s*Finished `dev` profile .*"), "Point with 3 fields", "answer: 42",
                    "generated: 42 on wasm32-unknown-linux-musl, cfg true", "threads: 14"],
    "cargo_release": ["Point with 3 fields", "answer: 42", "generated: 42 on wasm32-unknown-linux-musl, cfg true",
                      "threads: 14"],
    "macro_panic": ["error: proc macro panicked", "  = help: message: this macro always fails", "exit 1"],
    "cargo_new": ["Hello, world!"],
    "clippy_fmt": ["clippy::len_zero", "formatted"],
    "net_serde": ['{"name":"guest","n":7,"tags":["a","b"]}', "round trip: true"],
    "c_build": ["from C: 402", re.compile(r"zstd: 47 -> \d+ -> 47, same: true")],
}

fails = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + ("" if ok or not detail else "\n" + detail))
    if not ok:
        fails.append(name)


def addon_dir():
    d = tempfile.mkdtemp(prefix="rust-addon-")
    os.symlink(os.path.abspath(CPIO), os.path.join(d, "rust.cpio"))
    shutil.copyfile(os.path.join(A, "addon.json"), os.path.join(d, "rust.json"))
    if LLVM:
        os.symlink(os.path.abspath(LLVM_CPIO), os.path.join(d, "llvm.cpio"))
        shutil.copyfile(os.path.join(os.path.dirname(LLVM_CPIO), "llvm.json"), os.path.join(d, "llvm.json"))
    return d


def guest(command, timeout=7200, mounts=()):
    more = []
    for host, path, ro in mounts:
        more += ["--mount", f"{host}:{path}" + (":ro" if ro else "")]
    # the base and Python images (CA certificates), the network only for NET=1
    net = ["--allow", "index.crates.io", "--allow", "static.crates.io", "--allow", "crates.io"] if NET else ["--no-network"]
    addons = ["--addon", "llvm"] if LLVM else []
    env = " ".join(f"{k}=1" for k, on in (("NET", NET), ("LLVM", LLVM)) if on)
    argv = [ENGINE, "exec", "--kernel", f"{RT}/app/images/vmlinux.wasm", "--initramfs", f"{RT}/app/images/initramfs.cpio",
            "--python-image", f"{RT}/app/images/python.cpio", *more, "--addon-dir", ADDONS, *addons, "--addon", "rust",
            *net, "--mount", f"{T}:/work", "--cwd", "/work", "--", "/bin/sh", "-c", (env + " " if env else "") + command]
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as e:
        return -1, f"timed out after {timeout} s: {e.stdout or ''}", ""
    err = p.stderr
    mark = err.find("init: starting /bin/sh")
    if mark >= 0:
        err = err[err.find("\n", mark) + 1:]
    return p.returncode, p.stdout, err


def lines_match(want, got):
    if len(want) != len(got):
        return False
    for w, g in zip(want, got):
        if isinstance(w, str) and w != g:
            return False
        if not isinstance(w, str) and not w.fullmatch(g):
            return False
    return True


def main():
    for f in (ENGINE, CPIO):
        if not os.path.exists(f):
            print(f"missing {f}", file=sys.stderr)
            return 2
    global ADDONS
    ADDONS = addon_dir()

    start = time.time()
    rc, out, err = guest("/work/guest.sh")
    print(f"  (guest: {time.time() - start:.1f} s)")
    steps, last = {}, None
    for chunk in out.replace("\r", "").split("### ")[1:]:
        head, _, body = chunk.partition("\n")
        if head.startswith("status "):
            steps[last] = (int(head.split()[1]), steps[last][1])
        else:
            last = head.strip()
            steps[last] = (None, body)
    if not steps:
        check("guest ran", False, f"exit {rc}\n{out[-3000:]}\n{err[-3000:]}")
        return 1

    for name, (status, body) in steps.items():
        got = [l for l in body.splitlines() if l.strip()]
        detail = f"status {status}\n" + "\n".join(got[-40:])
        if name == "tools":
            ok = status == 0 and any(l.startswith("rustc 1.99.1 (58cf8f9c3 2026-10-13)") for l in got) \
                and "host: wasm32-unknown-linux-musl" in got and any(l.startswith("cargo 1.99.1") for l in got) \
                and not any(l.startswith("missing") for l in got) and "/usr" in got \
                and "rustdoc 1.99.1 (58cf8f9c3 2026-10-13)" in got and "clippy 0.1.99 (58cf8f9c35 2026-10-13)" in got \
                and "rustfmt 1.10.0-stable (58cf8f9c35 2026-10-13)" in got
            check(name, ok, detail)
        elif name == "cargo_test":
            results = [l for l in got if l.startswith("test result: ok.")]
            ok = status == 0 and len(results) >= 3 and not any("FAILED" in l or "failed;" in l and " 0 failed" not in l for l in got)
            check(name, ok, detail)
        elif name in EXPECTED:
            check(name, status == 0 and lines_match(EXPECTED[name], got), detail)
        else:
            check(name, status == 0, detail)
    wanted = [n for n in EXPECTED if (n != "net_serde" or NET) and (n != "c_build" or LLVM)]
    missing = [n for n in wanted + ["tools", "cargo_test"] if n not in steps]
    if missing:
        check("all steps ran", False, "missing: " + ", ".join(missing) + "\n" + err[-2000:])
    print(f"{len(fails)} failed" if fails else "all passed")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
