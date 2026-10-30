#!/usr/bin/env bash
# Checks for the build-tools add-on, in the guest (test_guest.py): make, ninja and pkgconf at work,
# with the python-extension and llvm add-ons when they are built; --upstream also their own test
# suites (build them first: TEST=1 addons/build-tools/build.sh).
#
# The guest is the packaged runtime (dist/runtime/collabo-core-<platform>, COLLABO_RUNTIME) with
# out/build-tools.cpio (BT_DIR); PYEXT_DIR and LLVM_DIR name the other add-ons' out/ folders.
set -euo pipefail
T="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$T/test_guest.py" "$@"
