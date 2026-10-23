#!/usr/bin/env python3
"""Typst Universe packages for the image, with everything they import: prints the sources.lock
lines (`universe:NAME:VERSION URL SHA256`) for the packages named and their @preview
dependencies, downloading each archive into build/dl to find them.

    tools/universe.py cmarker:0.1.10 cetz:0.5.2 ...

build.sh's `packages` stage unpacks every `universe:` entry of sources.lock into
/usr/share/typst/packages/preview/NAME/VERSION, where typst looks before downloading.
"""
import hashlib
import io
import os
import re
import sys
import tarfile
import urllib.request

A = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DL = os.path.join(A, "build", "dl")
IMPORT = re.compile(r'@preview/([a-z0-9-]+):(\d+\.\d+\.\d+)')


def fetch(name, version):
    url = f"https://packages.typst.org/preview/{name}-{version}.tar.gz"
    path = os.path.join(DL, os.path.basename(url))   # as build.sh fetch() names it
    if not os.path.exists(path):
        os.makedirs(DL, exist_ok=True)
        with urllib.request.urlopen(url, timeout=60) as r:
            data = r.read()
        with open(path + ".part", "wb") as f:
            f.write(data)
        os.rename(path + ".part", path)
    return url, path


def imports(path):
    """The @preview packages a package's .typ files import (not its template's)."""
    found = set()
    with tarfile.open(path) as tar:
        for member in tar.getmembers():
            if member.isfile() and member.name.endswith(".typ") and not member.name.startswith(("template/", "./template/")):
                text = tar.extractfile(member).read().decode("utf-8", "replace")
                found |= set(IMPORT.findall(text))
    return found


def main():
    todo = [tuple(spec.split(":")) for spec in sys.argv[1:]]
    done = {}
    while todo:
        name, version = todo.pop()
        if (name, version) in done:
            continue
        url, path = fetch(name, version)
        done[(name, version)] = (url, hashlib.sha256(open(path, "rb").read()).hexdigest(), os.path.getsize(path))
        todo += sorted(imports(path) - set(done))
    for (name, version), (url, sha, size) in sorted(done.items()):
        print(f"universe:{name}:{version} {url} {sha}  # {size // 1024} KB", file=sys.stderr)
        print(f"universe:{name}:{version} {url} {sha}")


if __name__ == "__main__":
    main()
