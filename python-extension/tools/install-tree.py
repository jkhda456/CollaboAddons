"""Install the packages' built trees into one site-packages, as pip would.

    install-tree.py INTERPRETER SITE BIN PACKAGE_SITE...

Run with the host CPython 3.13 (python/host), so the .pyc files it writes are the guest's version.
Each PACKAGE_SITE is a packages/<name>/build.sh result: a site-packages tree with one .dist-info
(tools/dist-info.py), or several pure-Python wheels unpacked, each .dist-info with its wheel's
RECORD (packages/mcp); its extension modules shared libraries for the guest's dlopen
(WebAssembly side modules: tools/wasm-pyext-cc's -shared link, numpy/_core/_multiarray_umath.cpython-313.so).
Each is checked to be that, and to link: everything it imports is the interpreter's
(INTERPRETER, the guest's python3.13, exports the Python API and the C library) or its own, or
weak — what the guest's dlopen would otherwise report as an undefined symbol. Per distribution:

  * every file goes to SITE as it is, .py files byte-compiled beside them;
  * console scripts (entry_points.txt) become launchers in BIN, as pip writes them;
  * INSTALLER and RECORD complete the .dist-info, so pip sees an ordinary installed distribution.
"""
import base64
import configparser
import csv
import hashlib
import io
import os
import py_compile
import shutil
import sys

PYTHON = "/usr/bin/python3.13"
GUEST_SITE = "/usr/lib/python3.13/site-packages"


def record_hash(data):
    return "sha256=" + base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()


def uleb(data, pos):
    result = shift = 0
    while True:
        byte = data[pos]
        pos += 1
        result |= (byte & 0x7F) << shift
        shift += 7
        if not byte & 0x80:
            return result, pos


def wasm_name(data, pos):
    n, pos = uleb(data, pos)
    return data[pos:pos + n].decode(), pos + n


def wasm_links(path):
    """A WebAssembly module's imports [(module, name)], exports {name} and weak imports {(module, name)}."""
    data = open(path, "rb").read()
    if data[:4] != b"\0asm":
        sys.exit(f"{path}: not a WebAssembly module")
    imports, exports, weak = [], set(), set()
    pos = 8
    while pos < len(data):
        kind = data[pos]
        size, pos = uleb(data, pos + 1)
        end = pos + size
        if kind == 0:
            name, p = wasm_name(data, pos)
            while name == "dylink.0" and p < end:
                sub = data[p]
                length, p = uleb(data, p + 1)
                q, p = p, p + length
                if sub == 4:  # WASM_DYLINK_IMPORT_INFO
                    n, q = uleb(data, q)
                    for _ in range(n):
                        module, q = wasm_name(data, q)
                        field, q = wasm_name(data, q)
                        flags, q = uleb(data, q)
                        if flags & 1:
                            weak.add((module, field))
        elif kind == 2:
            n, p = uleb(data, pos)
            for _ in range(n):
                module, p = wasm_name(data, p)
                field, p = wasm_name(data, p)
                imports.append((module, field))
                ty = data[p]
                p += 1
                if ty == 0:                       # function: a type index
                    _, p = uleb(data, p)
                elif ty == 1:                     # table: element type, limits
                    p += 1
                    flags, p = uleb(data, p)
                    _, p = uleb(data, p)
                    if flags & 1:
                        _, p = uleb(data, p)
                elif ty == 2:                     # memory: limits
                    flags, p = uleb(data, p)
                    _, p = uleb(data, p)
                    if flags & 1:
                        _, p = uleb(data, p)
                elif ty == 3:                     # global: value type, mutability
                    p += 2
                elif ty == 4:                     # tag: attribute, type index
                    _, p = uleb(data, p + 1)
                else:
                    sys.exit(f"{path}: an import of kind {ty}")
        elif kind == 7:
            n, p = uleb(data, pos)
            for _ in range(n):
                name, p = wasm_name(data, p)
                _, p = uleb(data, p + 1)
                exports.add(name)
        pos = end
    return imports, exports, weak


# what the engine's dynamic linker gives every library (engine/src/dylink.rs)
LINKER = {("env", "memory"), ("env", "__indirect_function_table"), ("env", "__stack_pointer"),
          ("env", "__memory_base"), ("env", "__table_base"), ("GOT.mem", "__dso_handle")}


def check_links(path, interpreter):
    imports, own, weak = wasm_links(path)
    missing = sorted(f"{module}.{name}" for module, name in imports
                     if (module, name) not in LINKER and name not in interpreter and name not in own
                     and (module, name) not in weak and ("env", name) not in weak)
    if missing:
        sys.exit(f"{path}: undefined symbols (neither the interpreter's nor its own):\n  " + "\n  ".join(missing))


def owners(package):
    """{.dist-info: [its files, relative to package]}: everything in the tree is the one .dist-info's;
    a tree of several wheels (pure-Python ones installed as they are, packages/mcp) is split by each
    wheel's own RECORD."""
    infos = sorted(d for d in os.listdir(package) if d.endswith(".dist-info"))
    if not infos:
        sys.exit(f"{package}: no .dist-info")
    files = []
    for directory, dirs, names in os.walk(package):
        dirs[:] = [d for d in dirs if d != "__pycache__"]
        files += [os.path.relpath(os.path.join(directory, name), package) for name in sorted(names)]
    files = [f for f in files if f != ".done" and f not in (f"{info}/RECORD" for info in infos)]
    if len(infos) == 1:
        return {infos[0]: files}
    owner = {}
    for info in infos:
        record = os.path.join(package, info, "RECORD")
        if not os.path.exists(record):
            sys.exit(f"{package}: several .dist-info, and {info} has no RECORD to say which files are its")
        with open(record, newline="") as f:
            for row in csv.reader(f):
                if row and row[0] in owner:
                    sys.exit(f"{package}: {row[0]} is both {owner[row[0]]}'s and {info}'s")
                if row:
                    owner[row[0]] = info
    stray = [f for f in files if f not in owner]
    if stray:
        sys.exit(f"{package}: files no wheel's RECORD names: {stray[:5]}")
    return {info: [f for f in files if owner[f] == info] for info in infos}


def install(package, site, bindir, interpreter):
    for info, files in owners(package).items():
        install_dist(package, info, files, site, bindir, interpreter)


def install_dist(package, info, files, site, bindir, interpreter):
    written = {}  # path relative to SITE -> bytes, for RECORD
    count = 0
    for rel in files:
        src = os.path.join(package, rel)
        if rel.endswith(".so"):
            with open(src, "rb") as f:
                head = f.read(64)
            if head[:4] != b"\0asm" or b"dylink.0" not in head:
                sys.exit(f"{src}: an extension module that is no WebAssembly shared library (not built by wasm-pyext-cc?)")
            check_links(src, interpreter)
            count += 1
        dest = os.path.join(site, rel)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        shutil.copyfile(src, dest)
        written[rel] = open(src, "rb").read()

    written[f"{info}/INSTALLER"] = b"collabo-core\n"
    with open(os.path.join(site, info, "INSTALLER"), "wb") as f:
        f.write(written[f"{info}/INSTALLER"])

    # Console scripts, as pip writes them (the kernel runs #! scripts).
    eps = os.path.join(package, info, "entry_points.txt")
    if os.path.exists(eps):
        parser = configparser.ConfigParser(delimiters=("=",))
        parser.optionxform = str
        parser.read(eps)
        for script, target in (parser["console_scripts"].items() if "console_scripts" in parser else []):
            # (an entry point's extras, "mcp.cli:app [cli]", are only metadata: pip writes the script anyway)
            module, _, func = target.split("[")[0].strip().partition(":")
            body = (f"#!{PYTHON}\n# -*- coding: utf-8 -*-\nimport re\nimport sys\n"
                    f"from {module} import {func.split('.')[0]}\n"
                    f"if __name__ == \"__main__\":\n"
                    f"    sys.argv[0] = re.sub(r\"(-script\\.pyw|\\.exe)?$\", \"\", sys.argv[0])\n"
                    f"    sys.exit({func}())\n").encode()
            path = os.path.join(bindir, script)
            with open(path, "wb") as f:
                f.write(body)
            os.chmod(path, 0o755)
            written[os.path.relpath(path, site)] = body

    # Byte-compile like pip; unchecked-hash because the image's timestamps are not the sources'.
    for rel in list(written):
        if rel.endswith(".py") and not rel.startswith(".."):
            try:
                pyc = py_compile.compile(os.path.join(site, rel), dfile=os.path.join(GUEST_SITE, rel), doraise=True,
                                         invalidation_mode=py_compile.PycInvalidationMode.UNCHECKED_HASH)
            except py_compile.PyCompileError as e:
                # pip leaves files it cannot compile (templates named .py) as they are; so do we
                print(f"  (not compiled: {rel}: {e.exc_type_name})")
                continue
            written[os.path.relpath(pyc, site)] = open(pyc, "rb").read()

    rows = [(rel, record_hash(data), len(data)) for rel, data in sorted(written.items())]
    rows.append((f"{info}/RECORD", "", ""))
    out = io.StringIO()
    csv.writer(out, lineterminator="\n").writerows(rows)
    with open(os.path.join(site, info, "RECORD"), "w") as f:
        f.write(out.getvalue())
    print(f"  {info:32} {len(written)} files, {count} extension modules")


def main():
    if sys.version_info[:2] != (3, 13):
        sys.exit("run this with the host CPython 3.13 (its .pyc files must match the guest's)")
    interpreter, site, bindir, *packages = sys.argv[1:]
    interpreter = wasm_links(interpreter)[1]
    for d in (site, bindir):
        shutil.rmtree(d, ignore_errors=True)
        os.makedirs(d)
    for package in packages:
        install(package, site, bindir, interpreter)


if __name__ == "__main__":
    main()
