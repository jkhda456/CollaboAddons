#!/usr/bin/env bash
# Checks for the llvm add-on, in the guest (test_guest.py, guest.sh): the tools and their other
# names; cc and c++ building programs of the guest's own (the C library, threads, setjmp/longjmp,
# libc++ with exceptions, std::format, std::filesystem) and shared libraries a program loads with
# dlopen (C, and C++ with its exceptions); GNU ld's options left out; ar/ranlib/nm/strip; the
# compiler's answers to build systems' questions.
#
# The guest is the packaged runtime (dist/runtime/collabo-core-<platform>, COLLABO_RUNTIME) with
# out/llvm.cpio (LLVM_CPIO); LLVM_ENGINE picks another engine.
set -euo pipefail
exec python3 "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/test_guest.py" "$@"
