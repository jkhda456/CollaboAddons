#!/usr/bin/env bash
# Checks for the doom add-on (tests/test_guest.py): the frontend's unit tests; frames and terminal
# renderings in the guest the same as the native build's; the game in a pseudo-terminal in the
# guest's shell (keys of legacy, kitty and Windows terminals, resizing, the screen as sent); a slow
# terminal; errors.
#
#   tests/run.sh [unit] [frames] [tty] [slow] [errors]     (default: all)
#
# The guest is the packaged runtime (dist/runtime/collabo-core-<platform>, COLLABO_RUNTIME) with
# out/doom.cpio (DOOM_CPIO); DOOM_ENGINE picks another engine.
set -euo pipefail
exec python3 "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/test_guest.py" "$@"
