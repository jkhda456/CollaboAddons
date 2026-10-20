"""Write the .dist-info of a package built from its source, as its wheel would have had it.

    dist-info.py SOURCE_DIR SITE [--project-dir DIR] [--version V] [--license [NAME=]PATH]...

Run with the host CPython 3.13 (python/host). The metadata is the source's PKG-INFO (an sdist's:
the same format, and text, as a wheel's METADATA), or else made from the [project] table of
pyproject.toml in SOURCE_DIR or --project-dir (LIEF's GitHub archive has no PKG-INFO; --version
when its version is dynamic). entry_points.txt comes from [project.scripts], [project.gui-scripts]
and [project.entry-points]; WHEEL gives the guest's tag, cp313-cp313-linux_wasm (what pip, on
the guest, takes a platform wheel of this machine to be); every License-File the metadata names,
and each --license (the libraries a package carries: NAME=PATH, as licenses/NAME), is copied to
licenses/. INSTALLER and RECORD are install-tree.py's to write.
"""
import argparse
import email.parser
import os
import re
import shutil
import sys
import tomllib

TAG = "cp313-cp313-linux_wasm"


def project_metadata(project, version):
    lines = ["Metadata-Version: 2.1", f"Name: {project['name']}", f"Version: {version or project['version']}"]
    if "description" in project:
        lines.append(f"Summary: {project['description']}")
    if "requires-python" in project:
        lines.append(f"Requires-Python: {project['requires-python']}")
    lic = project.get("license")
    if isinstance(lic, str):
        lines.append(f"License-Expression: {lic}")
    elif isinstance(lic, dict) and "text" in lic:
        lines.append(f"License: {lic['text']}")
    for url_name, url in project.get("urls", {}).items():
        lines.append(f"Project-URL: {url_name}, {url}")
    for c in project.get("classifiers", []):
        lines.append(f"Classifier: {c}")
    for d in project.get("dependencies", []):
        lines.append(f"Requires-Dist: {d}")
    return "\n".join(lines) + "\n"


def entry_points(project):
    groups = {}
    if "scripts" in project:
        groups["console_scripts"] = project["scripts"]
    if "gui-scripts" in project:
        groups["gui_scripts"] = project["gui-scripts"]
    groups.update(project.get("entry-points", {}))
    return "".join(f"[{g}]\n" + "".join(f"{k} = {v}\n" for k, v in items.items()) + "\n"
                   for g, items in groups.items() if items)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("source")
    ap.add_argument("site")
    ap.add_argument("--project-dir")
    ap.add_argument("--version")
    ap.add_argument("--license", action="append", default=[])
    a = ap.parse_args()

    pyproject = os.path.join(a.project_dir or a.source, "pyproject.toml")
    project = tomllib.load(open(pyproject, "rb")).get("project", {}) if os.path.exists(pyproject) else {}
    pkg_info = os.path.join(a.source, "PKG-INFO")
    if os.path.exists(pkg_info):
        metadata = open(pkg_info, encoding="utf-8").read()
    elif project:
        metadata = project_metadata(project, a.version)
    else:
        sys.exit(f"{a.source}: no PKG-INFO and no [project] in pyproject.toml")
    head = email.parser.Parser().parsestr(metadata, headersonly=True)
    name, version = head["Name"], head["Version"]

    info = os.path.join(a.site, f"{re.sub(r'[-_.]+', '_', name).lower()}-{version}.dist-info")
    shutil.rmtree(info, ignore_errors=True)
    os.makedirs(info)
    with open(os.path.join(info, "METADATA"), "w", encoding="utf-8") as f:
        f.write(metadata)
    with open(os.path.join(info, "WHEEL"), "w") as f:
        f.write(f"Wheel-Version: 1.0\nGenerator: collabo-core python-extension\nRoot-Is-Purelib: false\nTag: {TAG}\n")
    eps = entry_points(project)
    if eps:
        with open(os.path.join(info, "entry_points.txt"), "w") as f:
            f.write(eps)
    licenses = [(p, os.path.join(a.source, p)) for p in head.get_all("License-File") or []]
    licenses += [tuple(p.split("=", 1)) if "=" in p else (os.path.basename(p), p) for p in a.license]
    for rel, path in licenses:
        if not os.path.exists(path):
            sys.exit(f"license file {path} is missing")
        dest = os.path.join(info, "licenses", rel)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        shutil.copyfile(path, dest)
    print(f"  {os.path.basename(info)}: {len(licenses)} license files")


if __name__ == "__main__":
    main()
