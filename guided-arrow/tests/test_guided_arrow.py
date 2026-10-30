#!/usr/bin/env python3
"""guided-arrow's scripted checks, the same steps natively and in the guest.

    test_guided_arrow.py native               build/host/release/guided-arrow with build/guided-arrow.gar
    test_guided_arrow.py guest RUNTIME        out/guided-arrow.cpio booted in RUNTIME's engine

Each step is a shell command whose stdout, stderr and exit status are kept in a shared folder;
the checks run here afterwards. What the upstream repositories say (a page's exact text, its git
blob id) is read from build/src, so the archive is compared with the sources, not with itself.
The translation steps run when the archive has Korean (GUIDED_ARROW_LANGS=en,ko or all; the
default archive is English only). The guest runs two more, for the add-on's "sheets" setting.
"""
import json, os, shutil, subprocess, sys, tempfile

T = os.path.dirname(os.path.abspath(__file__))
A = os.path.dirname(T)
SRC = os.path.join(A, "build", "src")


def git(repo, *args):
    return subprocess.run(["git", "-C", os.path.join(SRC, repo)] + list(args), capture_output=True, text=True, check=True).stdout.strip()


def has(*parts):
    return lambda out, err, rc: all(p in out for p in parts) or "missing one of %r" % (parts,)


def lacks(*parts):
    return lambda out, err, rc: not any(p in out for p in parts) or "has one of %r" % (parts,)


def rc(n):
    return lambda out, err, code: code == n or "exit status %d, not %d" % (code, n)


def err_has(*parts):
    return lambda out, err, rc: all(p in err for p in parts) or "stderr lacks one of %r" % (parts,)


def json_ok(check):
    def f(out, err, rc):
        try:
            v = json.loads(out)
        except ValueError as e:
            return "not JSON: %s" % e
        return bool(check(v)) or "JSON check failed"
    return f


def starts(text):
    return lambda out, err, rc: out.startswith(text) or "does not start with %r" % text


def ends(text):
    return lambda out, err, rc: out.endswith(text) or "does not end with %r" % text


def equals(text):
    return lambda out, err, rc: out == text or "is not %r" % text


def same_as(path):
    want = open(path, encoding="utf-8").read()
    return lambda out, err, rc: out.split("===\n", 1)[-1].rstrip("\n") == want.rstrip("\n") or "differs from %s" % path


TAR_BLOB = git("tldr", "rev-parse", "HEAD:pages/common/tar.md")
TLDR_REV = git("tldr", "rev-parse", "HEAD")
GAR = os.path.join(A, "build", "guided-arrow.gar")
HOST = os.path.join(A, "build", "host", "release", "guided-arrow")
# The guest's archive is this one (build.sh packs it into the cpio).
LANGS = [l["lang"] for l in json.loads(subprocess.run([HOST, "--data", GAR, "--json", "--info"], capture_output=True,
                                                      text=True, check=True).stdout)["languages"]]
KO = "ko" in LANGS
STEPS = [
    ("lookup", "guided-arrow tar", [rc(0), has(
        "=== tar [tldr: common] ===", "# Archiving utility.", "tar xvf {{path/to/source.tar.ext}}",
        "=== tar [cheat.sheets] ===", "=== tar [cheatsheets] ===", "tar -xzvf /path/to/foo.tgz",
        "More information: https://www.gnu.org/software/tar/manual/tar.html")]),
    ("filter", "guided-arrow tar extract gz", [rc(0), has("tar -xzvf /path/to/foo.tgz"), lacks("tar cf ")]),
    ("tilde", "guided-arrow 'tar~list'", [rc(0), has("Lis[t] the contents", "tar tvf")]),
    ("dot-page", "guided-arrow .", [rc(0), has("=== . [tldr: common] ===")]),
    ("multiword", "guided-arrow git commit", [rc(0), has("=== git-commit [tldr: common] ===", "git commit")]),
    ("platform", "guided-arrow cal", [rc(0), has("=== cal [tldr: linux] ===", "(also:", "osx"), has("Display a calendar")]),
    ("platform-osx", "guided-arrow -p osx cal", [rc(0), has("=== cal [tldr: osx] ===")]),
    ("other-platform", "guided-arrow pbcopy", [rc(0), has("=== pbcopy [tldr: osx] ===", "(no linux or common page)")]),
] + ([
    ("korean", "guided-arrow -L ko tar", [rc(0), has("=== tar [tldr: common, ko] ===", "아카이브 유틸리티")]),
    ("lang-env", "LANG=ko_KR.UTF-8 guided-arrow tar", [rc(0), has("아카이브 유틸리티")]),
    ("no-translation", "guided-arrow -L ko avifdec", [rc(0), has("=== avifdec [tldr: common] ===", "(no ko translation)")]),
    ("search-korean", "guided-arrow -L ko -s 압축 해제 -n 5", [rc(0), has(", ko] ===")]),
] if KO else [
    # English only: a language asked for is English, said so; one from the environment quietly.
    ("lang-english-only", "guided-arrow -L ko tar", [rc(0), has("=== tar [tldr: common] ===", "# Archiving utility."),
                                                      err_has('no pages in language "ko"', "English is shown")]),
    ("lang-env", "LANG=ko_KR.UTF-8 guided-arrow tar", [rc(0), has("# Archiving utility."), lacks("아카이브")]),
    ("info-english-only", "guided-arrow --info", [rc(0), has("languages (pages, compressed bytes):\n  en ")]),
]) + [
    ("lang-c", "LANG=C.UTF-8 LANGUAGE= guided-arrow tar", [rc(0), has("# Archiving utility.")]),
    ("bad-lang", "guided-arrow -L xx tar", [rc(0), has("# Archiving utility."), err_has('no pages in language "xx"')]),
    ("short-options", "guided-arrow -S tar", [rc(0), has("tar czf {{path/to/target.tar.gz}} -C {{path/to/directory}} .")]),
    ("both-options", "guided-arrow -S -E tar", [rc(0), has("[-C|--directory]")]),
    ("no-braces", "guided-arrow --no-braces --from tldr tar", [rc(0), has("tar xvf path/to/source.tar.ext"), lacks("{{")]),
    ("color", "guided-arrow --color always tar", [rc(0), has("\x1b[1;36m=== tar", "\x1b[4mpath/to/source.tar.ext\x1b[24m")]),
    ("raw", "guided-arrow --raw --from tldr tar", [rc(0), same_as(os.path.join(SRC, "tldr", "pages", "common", "tar.md"))]),
    ("language-sheet", "guided-arrow python/lambda", [rc(0), has("=== python/lambda [cheat.sheets] ===", "lambda")]),
    ("language-topics", "guided-arrow python/", [rc(0), has("python/lambda\n")]),
    ("language-search", "guided-arrow python/reverse+dict", [rc(0), has("[cheat.sheets]")]),
    ("topic", "guided-arrow json-and-yaml-in-command-line", [rc(0), has("tools for a task", "\njq\n")]),
    ("search", "guided-arrow -s extract tar gz archive -n 3", [rc(0), has("tar -xzvf /path/to/foo.tgz")]),
    ("search-task", "guided-arrow -s find files modified in the last day -n 5", [rc(0), has("=== find [", "-mtime")]),
    ("search-json", "guided-arrow --json -s extract tar gz -n 3", [rc(0), json_ok(lambda v: v["results"] and v["results"][0]["upstream"]["license"])]),
    ("search-none", "guided-arrow -s qqqqzzzz", [rc(1), err_has("nothing matches")]),
    ("not-found", "guided-arrow tarr", [rc(1), err_has('no page named "tarr"', "close: tar")]),
    ("list", "guided-arrow -l git-c", [rc(0), has("git-commit\n"), lacks("tar\n")]),
    ("list-json", "guided-arrow --json -l zst", [rc(0), json_ok(lambda v: any(p["name"] == "zstd" for p in v["pages"]))]),
    ("info", "guided-arrow --info", [rc(0), has("tldr-pages/tldr at " + TLDR_REV, "CC-BY-4.0", "CC0-1.0", "MIT")]),
    ("info-page", "guided-arrow --info tar", [rc(0), has("path:     pages/common/tar.md", "git blob: " + TAR_BLOB,
                                                           "https://github.com/tldr-pages/tldr/blob/%s/pages/common/tar.md" % TLDR_REV)]),
    ("json", "guided-arrow --json tar", [rc(0), json_ok(lambda v: v["pages"][0]["upstream"]["path"] == "pages/common/tar.md"
                                                         and v["pages"][0]["examples"][0]["command"].startswith("tar cf {{"))]),
    ("verify", "guided-arrow --verify", [rc(0), has("every checksum matches")]),
    ("version", "guided-arrow --version", [rc(0), has("guided-arrow ", "tldr " + TLDR_REV[:12])]),
    ("usage", "guided-arrow", [rc(2), has("Usage:")]),
    ("bad-option", "guided-arrow --frobnicate", [rc(2), err_has("unknown option")]),
    # A damaged archive is refused, not misread.
    ("corrupt", "cp \"$GAR\" $W/bad.gar && printf 'XXXXXXXX' | dd of=$W/bad.gar bs=1 seek=40000 conv=notrunc 2>/dev/null"
                " && guided-arrow --data $W/bad.gar --verify", [rc(3), err_has("guided-arrow: ")]),
    ("not-an-archive", "echo hello > $W/x.gar && guided-arrow --data $W/x.gar tar", [rc(3), err_has("not a guided-arrow archive")]),
    # Packing sheets of one's own (natively the packer is the build's; in the guest it is the same program).
    ("pack-local", "mkdir -p $W/mine && printf '# deploy\\n# Ship it\\n\\n# Roll back the last release:\\nkubectl rollout undo deploy/web\\n' > $W/mine/deploy"
                   " && guided-arrow --pack -o $W/mine.gar --quiet --source mine=$W/mine && guided-arrow --data $W/mine.gar deploy roll",
     [rc(0), has("=== deploy [mine] ===", "kubectl rollout undo deploy/web")]),
]

# Sheets of one's own (--add, --remove), in their own folder; every other step has an empty one.
OWN = "export GUIDED_ARROW_SHEETS=$W/own; "
SECRET = "kubectl create secret generic {{name}} --from-literal={{key}}={{value}}"
STEPS += [
    ("add", OWN + "guided-arrow --add kubectl-secret 'Create a secret from literal values:' '%s'" % SECRET,
     [rc(0), has("added 1 example to kubectl-secret (a new sheet): ", "/own/kubectl-secret")]),
    ("add-stdin", OWN + "printf '# Show one value of a secret, decoded:\\nkubectl get secret {{name}} -o jsonpath={{.data.key}} | base64 -d\\n'"
                        " | guided-arrow --add kubectl-secret", [rc(0), has("added 1 example to kubectl-secret (2 in all)")]),
    ("add-again", OWN + "guided-arrow --add kubectl-secret 'The same command' '%s'" % SECRET, [rc(0), has("already in kubectl-secret")]),
    ("add-file", OWN + "cat $W/own/kubectl-secret",
     [rc(0), equals("# Create a secret from literal values:\n%s\n\n# Show one value of a secret, decoded:\n"
                    "kubectl get secret {{name}} -o jsonpath={{.data.key}} | base64 -d\n" % SECRET)]),
    # One's own sheet comes before the archive's pages of the same name.
    ("add-first", OWN + "guided-arrow --add tar 'Back up the home folder (ours):' 'tar czf /backup/home.tgz ~' >/dev/null && guided-arrow tar",
     [rc(0), starts("=== tar [local] ===\n# Back up the home folder (ours):\ntar czf /backup/home.tgz ~\n"), has("=== tar [tldr: common] ===")]),
    ("local-filter", OWN + "guided-arrow kubectl-secret decoded", [rc(0), has("base64 -d"), lacks("from-literal")]),
    ("local-search", OWN + "guided-arrow -s decode secret value -n 3", [rc(0), starts("=== kubectl-secret [local] ===\n"), has("base64 -d")]),
    ("local-list", OWN + "guided-arrow -l --from local", [rc(0), equals("kubectl-secret\ntar\n")]),
    ("no-local", OWN + "guided-arrow --no-local tar", [rc(0), starts("=== tar [tldr: common] ==="), lacks("[local]")]),
    ("local-info", OWN + "guided-arrow --info kubectl-secret", [rc(0), has("kubectl-secret [local]\n", "/own/kubectl-secret\n", "git blob: "), lacks("license:")]),
    ("local-info-all", OWN + "guided-arrow --info", [rc(0), has("sheets of one's own (source local):\n", "/own (2 sheets; --add writes here)")]),
    ("local-json", OWN + "guided-arrow --json kubectl-secret",
     [rc(0), json_ok(lambda v: v["pages"][0]["source"] == "local" and len(v["pages"][0]["examples"]) == 2)]),
    ("local-verify", OWN + "guided-arrow --verify", [rc(0), has("every checksum matches", "and 2 sheets of one's own (local)\n")]),
    ("remove-example", OWN + "guided-arrow --remove kubectl-secret decoded && cat $W/own/kubectl-secret",
     [rc(0), has("removed 1 example from kubectl-secret (1 left)", "base64 -d"), ends("\n# Create a secret from literal values:\n%s\n" % SECRET)]),
    ("remove-none", OWN + "guided-arrow --remove kubectl-secret nothing-like-this", [rc(1), err_has("no example of kubectl-secret mentions")]),
    ("remove-sheet", OWN + "guided-arrow --remove tar && guided-arrow tar", [rc(0), has("removed the sheet tar", "=== tar [tldr: common] ==="), lacks("[local]")]),
    ("remove-missing", OWN + "guided-arrow --remove nosuch", [rc(1), err_has("no sheet nosuch of one's own")]),
    ("add-bad-name", OWN + "guided-arrow --add ../evil 'a' 'b'", [rc(2), err_has("not a sheet name")]),
    ("add-no-command", OWN + "guided-arrow --add x 'only a description'", [rc(2), err_has("no command")]),
    ("add-not-one", OWN + "guided-arrow --add x 'A comment' '# not a command'", [rc(2), err_has("does not read back as one example")]),
    ("add-dash", OWN + "guided-arrow --add x 'Its version' -- --version && guided-arrow --from local x", [rc(0), has("# Its version\n--version\n")]),
    # sheets/ beside the archive is read too (what the image or another add-on puts there).
    ("sheets-beside", "mkdir -p $W/sys/sheets && cp \"$GAR\" $W/sys/a.gar && printf '# Say hello:\\necho hello\\n' > $W/sys/sheets/site-tool"
                      " && guided-arrow --data $W/sys/a.gar site-tool", [rc(0), has("=== site-tool [local] ===", "echo hello")]),
]

# The guest is started with the add-on's setting sheets=/w/kept (an app's mounted folder).
GUEST_STEPS = [
    ("setting", "unset GUIDED_ARROW_SHEETS; guided-arrow --add from-setting 'Say hi:' 'echo hi' && ls /w/kept",
     [rc(0), has("/w/kept/from-setting\n", "\nfrom-setting\n")]),
    # Without it: the sandbox's own home, and --add says it will not last.
    ("no-setting", "cp /etc/collabo/addons/guided-arrow.json $W/settings.json && echo '{}' > /etc/collabo/addons/guided-arrow.json"
                   " && unset GUIDED_ARROW_SHEETS; guided-arrow --add short-lived 'Say hi:' 'echo hi'; r=$?;"
                   " cp $W/settings.json /etc/collabo/addons/guided-arrow.json; exit $r",
     [rc(0), has("/root/.local/share/guided-arrow/sheets/short-lived"), err_has("is gone when the sandbox stops")]),
]


def script(env, steps):
    lines = ["#!/bin/sh"] + ["export %s='%s'" % kv for kv in env.items()]
    for name, cmd, _ in steps:
        lines.append("( %s ) > $W/%s.out 2> $W/%s.err; echo $? > $W/%s.rc" % (cmd, name, name, name))
    return "\n".join(lines) + "\n"


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "native"
    os.makedirs(os.path.join(A, "build"), exist_ok=True)
    work = tempfile.mkdtemp(prefix="test-", dir=os.path.join(A, "build"))
    steps = STEPS if mode == "native" else STEPS + GUEST_STEPS
    try:
        if mode == "native":
            env = {"W": work, "GAR": GAR, "GUIDED_ARROW_DATA": GAR, "GUIDED_ARROW_SHEETS": work + "/none",
                   "PATH": os.path.dirname(HOST) + ":" + os.environ["PATH"]}
            with open(os.path.join(work, "steps.sh"), "w") as f:
                f.write(script(env, steps))
            subprocess.run(["sh", os.path.join(work, "steps.sh")], timeout=600, env=dict(os.environ, LANG="", LANGUAGE="", NO_COLOR=""))
        else:
            rt = sys.argv[2]
            img = os.path.join(rt, "app", "images")
            env = {"W": "/w", "GAR": "/usr/share/guided-arrow/guided-arrow.gar", "GUIDED_ARROW_SHEETS": "/w/none", "LANG": "", "LANGUAGE": ""}
            with open(os.path.join(work, "steps.sh"), "w") as f:
                f.write(script(env, steps))
            cmd = [os.path.join(rt, "bin", "collabo-core-engine"), "exec", "--kernel", img + "/vmlinux.wasm",
                   "--initramfs", img + "/initramfs.cpio", "--tools-image", img + "/tools.cpio",
                   "--addon-dir", os.path.join(A, "out"), "--addon", "guided-arrow", "--addon-config", "guided-arrow:sheets=/w/kept",
                   "--mount", work + ":/w", "--", "/bin/sh", "/w/steps.sh"]
            r = subprocess.run(cmd, capture_output=True, timeout=1200)
            open(os.path.join(work, "engine.log"), "wb").write(r.stdout + r.stderr)
        fails = []
        for name, _, checks in steps:
            try:
                code = int(open(os.path.join(work, name + ".rc")).read().strip())
                out = open(os.path.join(work, name + ".out"), encoding="utf-8", errors="replace").read()
                err = open(os.path.join(work, name + ".err"), encoding="utf-8", errors="replace").read()
            except (OSError, ValueError):
                fails.append("%s: did not run" % name)
                continue
            for c in checks:
                r = c(out, err, code)
                if r is not True:
                    fails.append("%s: %s\n    stdout: %r\n    stderr: %r" % (name, r, out[:400], err[:300]))
                    break
        print("%s: %d steps, %d failed" % (mode, len(steps), len(fails)))
        for f in fails:
            print("FAIL " + f)
        if fails:
            print("(outputs kept in %s)" % work)
            return 1
        shutil.rmtree(work, ignore_errors=True)
        return 0
    except BaseException:
        print("(outputs kept in %s)" % work)
        raise


if __name__ == "__main__":
    sys.exit(main())
