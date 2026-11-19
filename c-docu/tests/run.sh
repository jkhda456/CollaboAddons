#!/usr/bin/env bash
# Checks for the c-docu add-on:
#   test_host.py    docu built natively (build/host): edits are partial (other parts of the package
#                   byte for byte, the edited part changed only where the edit is), the results
#                   open in LibreOffice with the change (when soffice is there), the old binary
#                   formats, new documents, conversions, rendering
#   test_guest.py   in the guest: what the add-on installs, and tests/edits.sh run in the guest and
#                   natively with the same outputs and files, byte for byte
#
# The guest is the packaged runtime (dist/runtime/collabo-core-<platform>, COLLABO_RUNTIME) with
# out/c-docu.cpio (DOCU_CPIO); DOCU_ENGINE and DOCU_KERNEL pick another engine binary or kernel,
# DOCU_HOST another native docu, DOCU_SOFFICE LibreOffice.
#
#   addons/c-docu/tests/run.sh [--host]     (--host: only the native checks)
set -euo pipefail
T="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
status=0
python3 "$T/test_host.py" || status=1
[[ "${1:-}" == --host ]] || python3 "$T/test_guest.py" || status=1
exit $status
