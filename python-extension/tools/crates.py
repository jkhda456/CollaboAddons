"""The crates a Rust extension module carries, each with its license: what `cargo metadata` (on
stdin) resolves for the guest's target from ROOT_CRATE, without the proc macros and build-time
dependencies (they only run on the host), and without the package's own crates (no source: its
own license covers them) — but libc, tombl's fork in a path of its own.

    cargo metadata --format-version 1 --filter-platform TARGET ... | crates.py ROOT_CRATE MODULE

Prints the list, a file for the package's .dist-info licenses (as the codex add-on lists them).
"""
import json
import sys


def main():
    root_name, module = sys.argv[1:]
    meta = json.load(sys.stdin)
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    packages = {p["id"]: p for p in meta["packages"]}
    root = next(i for i, p in packages.items() if p["name"] == root_name)
    seen, stack = set(), [root]
    while stack:
        i = stack.pop()
        if i in seen or any("proc-macro" in t["kind"] for t in packages[i]["targets"]):
            continue
        seen.add(i)
        stack += [d["pkg"] for d in nodes[i]["deps"] if any(k["kind"] in (None, "normal") for k in d["dep_kinds"])]
    print(f"The crates in {module} but its own, with their licenses")
    print("(libc: the fork of github.com/tombl/libc for the guest):")
    for p in sorted((packages[i] for i in seen), key=lambda p: (p["name"], p["version"])):
        if p["source"] or p["name"] == "libc":
            print(" ", p["name"], p["version"] + ":", p.get("license") or "see the crate", p.get("repository") or "")


if __name__ == "__main__":
    main()
