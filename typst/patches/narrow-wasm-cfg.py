#!/usr/bin/env python3
"""Narrow a crate's `wasm` cfgs to wasm that is not Linux, writing patches/crates/<crate>.patch.

Most crates read `target_arch = "wasm32"` or `target_family = "wasm"` as "a browser" (or WASI):
no threads, no files, no processes, JavaScript for the clock and the network. The guest is
wasm32 *Linux* (musl), where the Unix paths are the right ones. For each crate named, this
rewrites, in its Rust sources and Cargo.toml (cfg attributes, cfg!(), cfg_if!, [target.'cfg()']):

    target_arch = "wasm32"   ->  all(target_arch = "wasm32", not(target_os = "linux"))
    target_family = "wasm"   ->  all(target_family = "wasm", not(target_os = "linux"))

and writes the difference against the pristine crate as a patch, which build.sh applies to the
vendored copy. Crates whose wasm cfgs are about the instruction set (SIMD, linker sections:
memchr, hashbrown, ctor, inventory, …) are not named, and keep theirs.

    patches/narrow-wasm-cfg.py VENDOR_DIR CRATE-VERSION...

VENDOR_DIR is a pristine `cargo vendor --versioned-dirs` tree (build.sh's build/vendor before
patching, or a fresh one); an existing patch for the crate is replaced.
"""
import difflib
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "crates")

PATTERNS = [
    (re.compile(r'target_arch\s*=\s*"wasm32"'), 'all(target_arch = "wasm32", not(target_os = "linux"))'),
    (re.compile(r'target_family\s*=\s*"wasm"'), 'all(target_family = "wasm", not(target_os = "linux"))'),
]


def narrow(text):
    lines = text.splitlines(keepends=True)
    for i, line in enumerate(lines):
        if line.lstrip().startswith("//"):  # documentation examples
            continue
        for pattern, replacement in PATTERNS:
            line = pattern.sub(replacement, line)
        lines[i] = line
    return "".join(lines)


def crate_patch(vendor, crate):
    root = os.path.join(vendor, crate)
    if not os.path.isdir(root):
        sys.exit(f"no crate {crate} in {vendor}")
    chunks = []
    for directory, subdirs, files in os.walk(root):
        subdirs[:] = sorted(d for d in subdirs if d not in ("tests", "benches", "examples", "fuzz"))
        for name in sorted(files):
            if not (name.endswith(".rs") or name == "Cargo.toml"):
                continue
            path = os.path.join(directory, name)
            rel = os.path.relpath(path, root)
            with open(path, encoding="utf-8") as f:
                before = f.read()
            after = narrow(before)
            if after == before:
                continue
            chunks.extend(difflib.unified_diff(
                before.splitlines(keepends=True), after.splitlines(keepends=True),
                fromfile=f"a/{rel}", tofile=f"b/{rel}"))
    return "".join(chunks)


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    vendor = sys.argv[1]
    os.makedirs(OUT, exist_ok=True)
    for crate in sys.argv[2:]:
        patch = crate_patch(vendor, crate)
        target = os.path.join(OUT, f"{crate}.patch")
        if not patch:
            print(f"{crate}: no wasm cfgs")
            continue
        with open(target, "w", encoding="utf-8") as f:
            f.write(patch)
        print(f"{crate}: {patch.count(chr(10) + '+') } lines -> {os.path.relpath(target, HERE)}")


if __name__ == "__main__":
    main()
