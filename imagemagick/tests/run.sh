#!/usr/bin/env bash
# Checks for the imagemagick add-on: test_guest.py (what it installs; agents' commands — resizing,
# cropping, color, filters, compositing, text in Korean and Latin, drawing, SVG, the formats,
# identify, compare, montage, mogrify — in the guest and by the native reference, their files and
# what they print compared; PerlMagick's sample files of many formats read alike; the fallback font,
# a delegate started without fork, write errors, pipes, the pixel cache on disk, OpenMP; speed),
# then test_suite.py (ImageMagick's own `make check`, in the guest beside the reference).
#
#   addons/imagemagick/tests/run.sh [--quick]   (--quick: test_guest.py only)
#
# The reference is build/native (REF=1 build.sh: built when missing). The guest is the packaged
# runtime (dist/runtime/collabo-core-<platform>, COLLABO_RUNTIME) with out/imagemagick.cpio
# (IM_CPIO); IM_ENGINE and IM_KERNEL pick another engine binary or kernel, IM_CPUS the guest's CPUs.
set -euo pipefail
T="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A="$(dirname "$T")"
if [[ ! -x "$A/build/native/root/usr/bin/magick" || ! -f "$A/build/native/config/.done" ]]; then
  echo "== building the native reference (REF=1 build.sh)"
  REF=1 bash "$A/build.sh"
fi
status=0
python3 "$T/test_guest.py" || status=1
if [[ "${1:-}" != --quick ]]; then
  python3 "$T/test_suite.py" || status=1
fi
exit $status
