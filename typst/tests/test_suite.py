#!/usr/bin/env python3
"""Typst's own test suite, compiled in the guest and by the official typst 0.15.1 binary.

Each snippet of tests/suite (at TYPST_REV, from build/src) — `--- name attrs ---` sections — is
written beside its file (so that its relative paths resolve) with a prelude that defines what
Typst's test runner adds to the library (test, test-repr, print, lines, bounds, two colors, the
120pt page). Both sides compile it with the same arguments: the dev assets (typst-dev-assets) at
/assets, their fonts only, the test packages as @test/…; `paged` tests to PNG and PDF, `html`
ones to HTML, `eval`-only ones to PDF. Compared: the exit status, the diagnostics, every output
file. PNGs that differ only by a level or two on under 1% of the pixels (rasterized PDF images:
hayro's SIMD code rounds differently natively and in the guest) are counted apart. `large` and `bundle` tests are
skipped. A panic counts as the same when it is the same panic: typst is built with panic=abort for
the guest (exit status 134, not 101).

    test_suite.py [--filter REGEX] [--shards N] [--keep] [--report FILE]
"""
import argparse
import atexit
import concurrent.futures
import json
import os
import re
import shutil
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import B, EPOCH, Timer, check, guest, png_difference, present, reference, summary, workdir  # noqa: E402

SUITE = os.path.join(B, "src", "tests", "suite")
TEST_PACKAGES = os.path.join(B, "src", "tests", "packages")
DEV_ASSETS = os.path.join(B, "vendor", "typst-dev-assets-0.15.1", "files")
HEADER = re.compile(r"^--- ([^\s]+)((?: [a-z]+)*) ---$")

# What tests/src/world.rs adds to the library, in Typst.
PRELUDE = """\
#let test(lhs, rhs) = assert(lhs == rhs, message: "Assertion failed: " + repr(lhs) + " != " + repr(rhs))
#let test-repr(lhs, rhs) = assert(repr(lhs) == repr(rhs), message: "Assertion failed: " + repr(lhs) + " != " + repr(rhs))
#let print(..args) = none
#let lines(count, numbering: "A") = range(1, count + 1).map(n => std.numbering(numbering, n)).join("\\n")
#let bounds(body) = body
#let conifer = rgb(0x9f, 0xeb, 0x52)
#let forest = rgb(0x43, 0xa1, 0x27)
#set page(width: 120pt, height: auto, margin: 10pt)
#set text(size: 10pt)
"""
# Tests whose results depend on the implementation the code picks for the CPU (SSE/AVX, NEON with
# or without RDM, scalar in the guest): reported apart, with the reason, not counted as failures.
KNOWN = {
    name: "CMYK to RGB goes through moxcms' ICC transform, whose LUT is built with the C library's "
          "math and interpolated with SIMD natively (NEON fixed-point here) and with scalar code in "
          "the guest: colors can differ by a level"
    for name in ("color-conversion", "color-spaces", "color-cmyk-ops", "gradient-linear-cmyk")
}

COMMON = ["--ignore-system-fonts", "--font-path", "assets/fonts", "--package-path", "pkgs",
          "--features", "html,a11y-extras", "--diagnostic-format", "short", "--root", "."]


def snippets():
    """(id, relative source path, attrs, text) of every test."""
    for directory, _, files in sorted(os.walk(SUITE)):
        for name in sorted(files):
            if not name.endswith(".typ"):
                continue
            path = os.path.join(directory, name)
            current, lines = None, []
            for line in open(path, encoding="utf-8").read().split("\n") + ["--- end end ---"]:
                m = HEADER.match(line)
                if m:
                    if current:
                        yield current[0], os.path.relpath(path, SUITE), current[1], "\n".join(lines)
                    current, lines = (m.group(1), m.group(2).split()), []
                elif current:
                    lines.append(line)


def jobs_for(test_id, attrs):
    """(output name, extra arguments) of the compilations a test gets."""
    if "large" in attrs or "bundle" in attrs:
        return []
    out = []
    if any(a in attrs for a in ("paged", "empty", "pdf", "pdftags")):
        out += [(f"{test_id}-{{p}}.png", ["--ppi", "72"]), (f"{test_id}.pdf", [])]
    if "html" in attrs:
        out.append((f"{test_id}.html", ["--format", "html"]))
    if not out and "eval" in attrs:
        out.append((f"{test_id}.pdf", []))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--filter", help="only the tests whose name matches this regular expression")
    ap.add_argument("--shards", type=int, default=3, help="guest compilations run in parallel (default 3)")
    ap.add_argument("--keep", action="store_true", help="keep the work folder")
    ap.add_argument("--report", default=os.path.join(B, "suite-report.json"), help="where to write the details")
    args = ap.parse_args()
    if not present():
        return 2
    if not os.path.isdir(SUITE) or not os.path.isdir(DEV_ASSETS):
        print("missing build/src/tests/suite or the vendored typst-dev-assets: run addons/typst/build.sh", file=sys.stderr)
        return 2
    binary = reference()

    root = workdir("typst-suite-")
    if not args.keep:
        atexit.register(shutil.rmtree, root, True)
    shutil.copytree(DEV_ASSETS, os.path.join(root, "assets"), ignore=shutil.ignore_patterns("screenshots"))
    shutil.copytree(SUITE, os.path.join(root, "tests", "suite"))
    for package in os.listdir(TEST_PACKAGES):
        name, version = package.rsplit("-", 1)
        shutil.copytree(os.path.join(TEST_PACKAGES, package), os.path.join(root, "pkgs", "test", name, version))
    for side in ("ref", "guest"):
        os.makedirs(os.path.join(root, f"out-{side}"))
        os.makedirs(os.path.join(root, f"err-{side}"))
    for d in ("out-guest", "err-guest"):
        os.chmod(os.path.join(root, d), 0o777)

    cases = []   # (key, source, output, extra args)
    seen = set()
    for test_id, rel, attrs, text in snippets():
        if args.filter and not re.search(args.filter, test_id):
            continue
        if test_id in seen:
            continue
        seen.add(test_id)
        source = os.path.join("tests", "suite", os.path.dirname(rel), f"__{test_id}.typ")
        with open(os.path.join(root, source), "w", encoding="utf-8") as f:
            f.write(PRELUDE + text)
        for output, extra in jobs_for(test_id, attrs):
            cases.append((output.replace("-{p}", ""), source, output, extra))
    print(f"{len(seen)} tests, {len(cases)} compilations", flush=True)

    env = dict(os.environ, SOURCE_DATE_EPOCH=EPOCH, NO_COLOR="1")
    env.pop("TYPST_FONT_PATHS", None)
    env.pop("TYPST_PACKAGE_PATH", None)

    def run_ref(case):
        key, source, output, extra = case
        import subprocess
        p = subprocess.run([binary, "compile", *COMMON, *extra, source, f"out-ref/{output}"], cwd=root, env=env,
                           capture_output=True, text=True, errors="replace", timeout=300)
        return key, p.returncode, p.stderr

    with Timer() as t_ref:
        with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count()) as pool:
            ref_results = {key: (rc, err) for key, rc, err in pool.map(run_ref, cases)}
    print(f"reference: {t_ref.seconds:.0f} s", flush=True)

    # The guest: one boot, the compilations in shards.
    shards = [cases[i::args.shards] for i in range(args.shards)]
    for i, shard in enumerate(shards):
        with open(os.path.join(root, f"shard{i}.sh"), "w") as f:
            for key, source, output, extra in shard:
                arguments = " ".join(COMMON + extra)
                f.write(f"typst compile {arguments} '{source}' 'out-guest/{output}' 2> 'err-guest/{key}.txt'; "
                        f"echo \"{key} $?\" >> rc-guest-{i}.txt\n")
    command = " ".join(f"sh shard{i}.sh &" for i in range(args.shards)) + " wait; echo suite-done"
    with Timer() as t_guest:
        rc, out = guest(root, command, timeout=7200, cpus=args.shards + 1)
    print(f"guest: {t_guest.seconds:.0f} s", flush=True)
    check("the guest ran every compilation", "suite-done" in out, out[-2000:])

    guest_rc = {}
    for i in range(args.shards):
        path = os.path.join(root, f"rc-guest-{i}.txt")
        if os.path.exists(path):
            for line in open(path):
                key, _, code = line.rstrip("\n").rpartition(" ")
                guest_rc[key] = int(code)

    def normalize(text):
        text = text.replace(root + "/", "").replace("/work/", "")
        # A panic: the thread's id differs, and std's hint about backtraces is the native one's.
        text = re.sub(r"thread '([^']*)' \(\d+\) panicked", r"thread '\1' panicked", text)
        text = text.replace("note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n", "")
        return text.strip()

    def same_status(guest_code, ref_code, guest_err):
        # panic=abort in the guest: a panic ends typst with SIGABRT (134), natively it unwinds (101).
        return guest_code == ref_code or (guest_code == 134 and ref_code == 101 and " panicked at " in guest_err)

    identical, tolerated, known, mismatched = [], [], [], []
    for key, source, output, extra in cases:
        ref_code, ref_err = ref_results[key]
        try:
            guest_err = open(os.path.join(root, "err-guest", f"{key}.txt"), encoding="utf-8", errors="replace").read()
        except OSError:
            guest_err = "<no stderr file>"
        problems = []
        if not same_status(guest_rc.get(key), ref_code, guest_err):
            problems.append(f"exit status {guest_rc.get(key)} != {ref_code}")
        if normalize(guest_err) != normalize(ref_err):
            problems.append(f"diagnostics differ:\n  guest: {normalize(guest_err)[:400]}\n  ref:   {normalize(ref_err)[:400]}")
        pattern = re.compile("^" + re.escape(output).replace(re.escape("{p}"), r"\d+") + "$")
        ref_files = sorted(f for f in os.listdir(os.path.join(root, "out-ref")) if pattern.match(f))
        guest_files = sorted(f for f in os.listdir(os.path.join(root, "out-guest")) if pattern.match(f))
        if ref_files != guest_files:
            problems.append(f"outputs {guest_files} != {ref_files}")
        near = []
        for name in set(ref_files) & set(guest_files):
            a, b = os.path.join(root, "out-ref", name), os.path.join(root, "out-guest", name)
            if open(a, "rb").read() == open(b, "rb").read():
                continue
            if name.endswith(".png"):
                d, total, m = png_difference(a, b)
                if m <= 2 and d * 100 < total:
                    near.append(f"{name}: {d}/{total} pixels by up to {m}")
                    continue
                problems.append(f"{name}: {d}/{total} pixels differ by up to {m}")
            else:
                problems.append(f"{name} differs")
        if problems and key.rsplit(".", 1)[0] in KNOWN:
            known.append({"case": key, "reason": KNOWN[key.rsplit(".", 1)[0]], "problems": problems})
        elif problems:
            mismatched.append({"case": key, "source": source, "problems": problems})
        elif near:
            tolerated.append({"case": key, "near": near})
        else:
            identical.append(key)

    with open(args.report, "w") as f:
        json.dump({"identical": len(identical), "tolerated": tolerated, "known": known, "mismatched": mismatched},
                  f, indent=1, ensure_ascii=False)
    print(f"{len(identical)} identical, {len(tolerated)} within the rasterizer's tolerance, "
          f"{len(known)} known to depend on the machine, {len(mismatched)} different (details: {args.report})")
    for k in known:
        print(f"  known: {k['case']}: " + "; ".join(k["problems"])[:300])
    for m in mismatched[:30]:
        print(f"  {m['case']} ({m['source']}): " + "; ".join(m["problems"])[:600])
    check(f"Typst's test suite: the guest's results are the official binary's ({len(cases)} compilations)",
          not mismatched and len(identical) + len(tolerated) + len(known) == len(cases), f"{len(mismatched)} differ")
    if args.keep:
        print(f"kept {root}")
    return summary()


if __name__ == "__main__":
    sys.exit(main())
