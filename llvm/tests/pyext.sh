#!/bin/sh
# A C extension module built in the guest with the interpreter's own build variables, as setuptools
# builds one (sysconfig: CC, CFLAGS, CCSHARED, LDSHARED, INCLUDEPY, EXT_SUFFIX), then imported.
# test_guest.py runs it with the Python image and the interpreter's headers at /usr/include/python3.13.
rm -rf /tmp/pyext && mkdir -p /tmp/pyext && cd /tmp/pyext && cp /work/src/spam.c . || exit 1
var() { python3 -c "import sysconfig; print(sysconfig.get_config_var('$1'))"; }
set -e
$(var CC) $(var CFLAGS) $(var CCSHARED) -I"$(var INCLUDEPY)" -c spam.c -o spam.o
$(var LDSHARED) spam.o -o "spam$(var EXT_SUFFIX)"
python3 - <<'PY'
import spam
print("add:", spam.add(2, 40))
try:
    spam.fail([1, "x"])
except spam.error as e:
    print("error:", e)
print("jump:", spam.jump())
print("loader:", type(spam.__loader__).__name__, spam.__file__.endswith(".cpython-313.so"))
PY
