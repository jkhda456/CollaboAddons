#!/usr/bin/env python3
"""ImageMagick's own checks (its `make check`: tests/*.tap — the validate program's suites of
conversions, formats in memory, on disk and mapped, colorspaces, composites, compares, montages,
identify, stream; drawtest and wandtest of MagickWand's API; the command line's colorspace,
mogrify, pipe, statistic, SVG, EXIF and PCX checks) in the guest, the wasm build of them in
build/wasm/im, beside the same checks of the native reference (build/native/im).

Every check the reference passes must pass in the guest, but for KNOWN: probes of cli-pipe.tap
that the guest's shell (BusyBox ash) answers differently, whose subject test_guest.py checks
otherwise. The HEIC/AVIF and Ultra HDR checks skip themselves in both builds (no libheif, no
libultrahdr). Run by tests/run.sh; see common.py for the environment variables.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import B, Timer, check, guest, native, present, skip, summary  # noqa: E402

SRC = os.path.join(B, "src")
TAPS = sorted(f for f in os.listdir(os.path.join(SRC, "tests")) if f.endswith(".tap"))
MARK = "=== tap "
# (tap, number): why it fails in the guest though ImageMagick does what it checks there
KNOWN = {
    ("cli-pipe.tap", 17): "`... | magick -script fd:5 5<&0`: the guest's shell (BusyBox ash, which has no "
                          "fork) cannot duplicate a pipeline's stdin (\"fd#0 is not open\"); test_guest.py "
                          "reads a -script from a descriptor",
    ("cli-pipe.tap", 20): "its probe `printf x >/dev/full` succeeds: BusyBox's printf returns 0 after a "
                          "failed write; test_guest.py checks magick's own write errors on /dev/full",
    ("cli-pipe.tap", 21): "as 20",
}


def run_all(runner, build):
    """Runs every .tap from build/<build>/im, as `make check` does; {tap: output}."""
    script = "; ".join(f"echo '{MARK}{tap}'; sh {SRC}/tests/{tap} 2>&1" for tap in TAPS)
    status, out = runner(f"export srcdir={SRC}; cd {B}/{build}/im && {{ {script}; }}")
    parts, current = {}, None
    for line in out.splitlines():
        if line.startswith(MARK):
            current = line[len(MARK):].strip()
            parts[current] = []
        elif current:
            parts[current].append(line)
    return {k: "\n".join(v) for k, v in parts.items()}, out


def results(text):
    """TAP: (planned, {number: (ok, description)})."""
    plan = re.search(r"^1\.\.(\d+)", text, re.M)
    tests = {}
    n = 0
    for m in re.finditer(r"^(not ok|ok)\b(?:\s+(\d+))?(.*)$", text, re.M):
        n = int(m.group(2)) if m.group(2) else n + 1
        tests[n] = (m.group(1) == "ok", m.group(3).strip(" -"))
    return (int(plan.group(1)) if plan else None), tests


def main():
    if not present():
        return 1
    for build in ("wasm", "native"):
        for prog in ("tests/validate", "tests/drawtest", "tests/wandtest", "utilities/magick"):
            path = os.path.join(B, build, "im", prog)
            if os.path.exists(path):
                os.chmod(path, 0o755)
    with Timer() as tn:
        ref, ref_out = run_all(lambda c: native(c, cwd=B, timeout=3600), "native")
    with Timer() as tg:
        got, got_out = run_all(lambda c: guest(c, mounts=[(B, B)], cwd=B, timeout=7200), "wasm")
    print(f"(reference {tn.seconds:.0f} s, guest {tg.seconds:.0f} s)")
    total = passed = 0
    for tap in TAPS:
        if tap not in got:
            check(tap, False, "no output from the guest:\n" + got_out[-2000:])
            continue
        plan_g, tests_g = results(got[tap])
        plan_n, tests_n = results(ref.get(tap, ""))
        skipped = re.search(r"^1\.\.0\s*#\s*SKIP\s*(.*)$", got[tap], re.M)
        if plan_g == 0 and plan_n == 0 and skipped:
            skip(tap, skipped.group(1) + " (in both builds)")
            continue
        for (name, n), why in KNOWN.items():
            if name == tap and n in tests_g and not tests_g[n][0]:
                print(f"  known: {tap} {n}: {why}")
                tests_g[n] = (True, tests_g[n][1])
        failed = [f"{n} {d}" for n, (ok, d) in sorted(tests_g.items()) if not ok and tests_n.get(n, (False,))[0]]
        both = [f"{n} {d}" for n, (ok, d) in sorted(tests_g.items()) if not ok and not tests_n.get(n, (True,))[0]]
        complete = plan_g is not None and len(tests_g) == plan_g
        total += len(tests_g)
        passed += sum(1 for ok, _ in tests_g.values() if ok)
        detail = (f"{len(tests_g)} of {plan_g} run; guest only: {failed}; both: {both}\n"
                  + "\n".join(got[tap].splitlines()[-25:]))
        check(f"{tap}: {sum(1 for ok, _ in tests_g.values() if ok)}/{len(tests_g)}"
              + (f" (the reference fails {len(both)} too)" if both else ""),
              complete and not failed and len(tests_g) >= len(tests_n), detail)
    print(f"\n{passed} of {total} TAP checks pass in the guest")
    return summary()


if __name__ == "__main__":
    sys.exit(main())
