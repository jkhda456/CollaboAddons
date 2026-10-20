#!/usr/bin/env bash
# pycparser 3.11, the C parser cffi reads its declarations with (ffi.cdef): pure Python, installed
# from its wheel as it is (its RECORD and INSTALLER are tools/install-tree.py's to write).
# Run by ../../build.sh (packages/README.md); result: build/pycparser/site.
set -euo pipefail
source "$PYEXT/tools/lib.sh"
SITE="$BP/site"
mkdir -p "$BP"

if stage pycparser "$SITE/.done" "$(lock pycparser)" "$PYEXT/packages/pycparser/build.sh"; then
  rm -rf "$SITE"; mkdir -p "$SITE"
  "$HOSTPY" -I -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' "$(fetch pycparser)" "$SITE"
  rm -f "$SITE"/pycparser-*.dist-info/RECORD
  touch "$SITE/.done"
fi
