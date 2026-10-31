#!/usr/bin/env python3
"""Writes browser-js.h: the browser's scripts (snapshot.js, target.js, ...) as C string constants;
also a text as it is (agent-help.txt: agent_help_txt, for web).

    gen-js.py OUT.h SCRIPT.js|TEXT.txt...
"""
import json
import os
import sys


def c_string(line):
    """A C string literal of any text: bytes past ASCII as octal escapes (the UTF-8 kept)"""
    out = []
    for b in line.encode("utf-8"):
        c = chr(b)
        if c in '"\\?':  # ? too: no trigraphs
            out.append("\\" + c)
        elif c == "\n":
            out.append("\\n")
        elif c == "\t":
            out.append("\\t")
        elif 32 <= b < 127:
            out.append(c)
        else:
            out.append("\\%03o" % b)
    return '"' + "".join(out) + '"'


with open(sys.argv[1], "w") as out:
    out.write("/* written by gen-js.py from the browser's .js files */\n")
    for path in sys.argv[2:]:
        stem, ext = os.path.splitext(os.path.basename(path))
        source = open(path, encoding="utf-8").read()
        if ext == ".txt":
            out.write("static const char %s_txt[] =\n" % stem.replace("-", "_"))
            for line in source.splitlines():
                out.write("    %s\n" % c_string(line + "\n"))
            out.write(";\n")
            continue
        name = stem + "_js"
        # the function expression, without the leading comment lines
        lines = [l for l in source.splitlines() if not l.startswith("//")]
        text = "\n".join(lines).strip()
        out.write("static const char %s[] =\n" % name)
        for line in text.splitlines():
            out.write("    %s\n" % json.dumps(line + "\n", ensure_ascii=True))
        out.write(";\n")
