#!/usr/bin/env bash
# Checks for the python-extension add-on, in the guest (test_guest.py): the interpreter's built-in
# modules; numpy, Pillow and LIEF printing, in the guest, exactly what they print with their own
# wheels from PyPI on this machine (checks/*.py against expected/*.out); pip, console scripts,
# threads, and the known differences from the wheels.
#
#   tests/run.sh               against expected/ (made by reference.py)
#   tests/run.sh --reference   remake expected/ first: the checks with the PyPI wheels of the
#                              versions built (needs the network), then the guest
#   tests/run.sh --upstream    and then numpy's and Pillow's own test suites in the guest
#                              (upstream.py: about 6 minutes; failures only the known ones,
#                              upstream-known.txt)
#   tests/make-samples.sh      remake samples/ (LIEF's test binaries; clang-19 and LLD)
#
# The guest is the packaged runtime (dist/runtime/collabo-core-<platform>, COLLABO_RUNTIME) with
# its python.cpio and out/python-extension.cpio (PYEXT_CPIO). The engine should have
# addons/mod/0016 (an uncaught C++ exception would otherwise stop the machine, not the program).
set -euo pipefail
T="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
upstream=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --reference) python3 "$T/reference.py" ;;
    --upstream) upstream=1 ;;
    *) break ;;
  esac
  shift
done
python3 "$T/test_guest.py" "$@"
if [[ $upstream == 1 ]]; then python3 "$T/upstream.py"; fi
