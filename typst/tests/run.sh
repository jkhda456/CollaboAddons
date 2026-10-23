#!/usr/bin/env bash
# Checks for the typst add-on, in the guest:
#   test_guest.py   what it installs; its PDF/PNG/SVG/HTML against the official typst 0.15.1
#                   binary's, byte for byte; the CLI; typst watch; packages through the host's
#                   request API (a fake one, and the real one under a deny rule; a real download
#                   when this machine is online); the report template; speed
#   test_suite.py   Typst's own test suite (tests/suite at v0.15.1, ~3,800 snippets): every
#                   snippet compiled in the guest and by the official binary, outputs and
#                   diagnostics compared (about 10 minutes; --quick skips it)
#
# The guest is the packaged runtime (dist/runtime/collabo-core-<platform>, COLLABO_RUNTIME) with
# out/typst.cpio (TYPST_CPIO); TYPST_ENGINE and TYPST_KERNEL pick another engine binary or kernel.
# The official binary is downloaded once into build/ (sources.lock); TYPST_REF names another.
#
#   addons/typst/tests/run.sh [--quick] [SUITE-ARGS...]   (test_suite.py --help)
set -euo pipefail
T="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
quick=
[[ "${1:-}" == --quick ]] && { quick=1; shift; }
status=0
python3 "$T/test_guest.py" || status=1
[[ -n "$quick" ]] || python3 "$T/test_suite.py" "$@" || status=1
exit $status
