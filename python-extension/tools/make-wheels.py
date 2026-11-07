"""Pack packages' built trees as wheels, for pip in the guest (/usr/share/python-wheels).

    make-wheels.py INTERPRETER OUTDIR TREE...

Run with the host CPython 3.13 (python/host). Each TREE is a site-packages tree with one .dist-info,
as a package's build.sh leaves it (build/<name>/site, build/<name>/wheels/<name>-<version>):
tools/dist-info.py's, or a pure-Python wheel's unpacked. Its extension modules, and the libraries a
binding loads by name (unicorn/lib/libunicorn.so.2, through ctypes), are checked as
tools/install-tree.py checks those it installs (WebAssembly side modules, everything they import the
interpreter's, INTERPRETER, or their own, or weak). The wheel is <name>-<version>-<tags>.whl, the
tags its WHEEL file's, with the files of the tree (not .pyc files: pip compiles them as it installs)
and a RECORD of them; in a fixed order and time, so the same tree makes the same wheel. OUTDIR is
emptied first.
"""
import base64
import hashlib
import importlib.util
import os
import re
import shutil
import sys
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("install_tree", os.path.join(HERE, "install-tree.py"))
install_tree = importlib.util.module_from_spec(spec)
spec.loader.exec_module(install_tree)

TIME = (2020, 1, 1, 0, 0, 0)


def tags(wheel_file):
    """The file name's tags from WHEEL's Tag lines (several make dotted sets: py2.py3-none-any)."""
    found = [line.split(":", 1)[1].strip() for line in open(wheel_file) if line.startswith("Tag:")]
    if not found:
        sys.exit(f"{wheel_file}: no Tag")
    parts = [sorted({t.split("-")[i] for t in found}) for i in range(3)]
    return "-".join(".".join(p) for p in parts)


def pack(tree, outdir, interpreter):
    infos = [d for d in os.listdir(tree) if d.endswith(".dist-info")]
    if len(infos) != 1:
        sys.exit(f"{tree}: {len(infos)} .dist-info, one wanted")
    info = infos[0]
    files, modules = [], 0
    for directory, dirs, names in os.walk(tree):
        dirs[:] = sorted(d for d in dirs if d != "__pycache__")
        for name in sorted(names):
            rel = os.path.relpath(os.path.join(directory, name), tree)
            if rel == ".done" or rel in (f"{info}/RECORD", f"{info}/INSTALLER") or rel.endswith(".pyc"):
                continue
            # (and a library loaded by its file name, libunicorn.so.2: a module's, through ctypes)
            if rel.endswith(".so") or re.search(r"\.so\.\d+$", rel):
                with open(os.path.join(tree, rel), "rb") as f:
                    head = f.read(64)
                if head[:4] != b"\0asm" or b"dylink.0" not in head:
                    sys.exit(f"{tree}/{rel}: an extension module that is no WebAssembly shared library")
                install_tree.check_links(os.path.join(tree, rel), interpreter)
                modules += 1
            files.append(rel)
    files.sort(key=lambda rel: (rel.startswith(info + "/"), rel))   # the .dist-info last
    whl = os.path.join(outdir, f"{info[:-len('.dist-info')]}-{tags(os.path.join(tree, info, 'WHEEL'))}.whl")
    record = []
    with zipfile.ZipFile(whl, "w", zipfile.ZIP_DEFLATED) as z:
        for rel in files:
            data = open(os.path.join(tree, rel), "rb").read()
            entry = zipfile.ZipInfo(rel, TIME)
            entry.external_attr = 0o644 << 16
            z.writestr(entry, data, zipfile.ZIP_DEFLATED)
            record.append(f"{rel},{install_tree.record_hash(data)},{len(data)}")
        record.append(f"{info}/RECORD,,")
        entry = zipfile.ZipInfo(f"{info}/RECORD", TIME)
        entry.external_attr = 0o644 << 16
        z.writestr(entry, "\n".join(record) + "\n", zipfile.ZIP_DEFLATED)
    print(f"  {os.path.basename(whl):48} {len(files)} files, {modules} shared libraries, {os.path.getsize(whl) // 1024} KiB")


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    interpreter, outdir, *trees = sys.argv[1:]
    interpreter = install_tree.wasm_links(interpreter)[1]
    shutil.rmtree(outdir, ignore_errors=True)
    os.makedirs(outdir)
    for tree in trees:
        pack(tree, outdir, interpreter)


if __name__ == "__main__":
    main()
