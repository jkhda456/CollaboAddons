#!/usr/bin/env python3
"""The c-docu add-on in the guest: what it installs, and tests/edits.sh (every command, on every
format: reading, editing, new documents, converting, rendering) run in the guest and natively —
the outputs and every file written must be the same, byte for byte. Run by tests/run.sh; see
common.py for the environment variables.
"""
import os
import shutil
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import HOST, FONTS, T, check, fixtures, guest, present, summary, workdir  # noqa: E402


def main():
    if not present():
        return 1
    if not os.path.exists(HOST):
        print(f"missing {HOST} (cargo build --target-dir build/host)", file=sys.stderr)
        return 1
    w = workdir("docu-guest-")
    st, out = guest(w, "docu version; ls /usr/bin/collabo-docu /usr/share/doc/docu/AGENTS.md "
                       "/etc/codex/skills/office-docs/SKILL.md /root/.agent/skills/office-docs/SKILL.md "
                       "/root/.claude/skills/office-docs/SKILL.md; ls /usr/share/docu/fonts | wc -l")
    check("installed", st == 0 and out.startswith("docu ") and "No such file" not in out, out)

    g, h = os.path.join(w, "g"), os.path.join(w, "h")
    # One set of fixtures for both (they carry the time they were made).
    os.mkdir(g)
    os.chmod(g, 0o777)
    fixtures(g)
    shutil.copyfile(os.path.join(T, "edits.sh"), os.path.join(g, "edits.sh"))
    shutil.copytree(g, h)
    start = time.monotonic()
    st, out = guest(w, "cd g && sh edits.sh")
    took = time.monotonic() - start
    check(f"edits.sh in the guest ({took:.0f} s)", st == 0, out)
    env = dict(os.environ, DOCU=HOST, DOCU_FONT_PATH=FONTS, DOCU_SYSTEM_FONTS="0")
    import subprocess
    subprocess.run(["sh", "edits.sh"], cwd=h, env=env, check=True)
    log_g = open(os.path.join(g, "log.txt"), encoding="utf-8", errors="replace").read()
    log_h = open(os.path.join(h, "log.txt"), encoding="utf-8", errors="replace").read()
    if not check("the same output as natively", log_g == log_h, first_difference(log_g, log_h)):
        pass
    failed = [l for l in log_h.splitlines() if l.startswith("exit ") and l != "exit 0"]
    check("every command but the one meant to fail succeeds", len(failed) == 1, "\n".join(failed))
    names = sorted(n for n in os.listdir(h) if n not in ("log.txt", "edits.sh"))
    keep = os.environ.get("DOCU_KEEP")
    if keep:
        shutil.copytree(w, keep, dirs_exist_ok=True)
    differ = [n for n in names if not same(os.path.join(g, n), os.path.join(h, n))]
    check(f"the same files as natively ({len(names)})", not differ, differ)
    return summary()


def same(a, b):
    if not os.path.exists(a):
        return False
    with open(a, "rb") as fa, open(b, "rb") as fb:
        return fa.read() == fb.read()


def first_difference(a, b):
    la, lb = a.splitlines(), b.splitlines()
    for i, (x, y) in enumerate(zip(la, lb)):
        if x != y:
            return f"line {i + 1}:\n  guest:  {x}\n  native: {y}"
    return f"lengths {len(la)} / {len(lb)}"


if __name__ == "__main__":
    sys.exit(main())
