#!/bin/sh
# The checks' guest half (test_guest.py runs it, with tests/ at /work): each step's output after
# "### NAME", then "### status N". Works in a copy of src/ in /tmp.
rm -rf /tmp/llvmtest && mkdir -p /tmp/llvmtest && cd /tmp/llvmtest && cp /work/src/* . || exit 1

tools() {
  for t in clang clang++ opt llc llvm-mc lld ld.lld wasm-ld ld llvm-ar llvm-ranlib ar ranlib llvm-nm nm \
           llvm-objcopy llvm-strip strip objcopy llvm-objdump objdump llvm-readobj llvm-readelf readelf \
           llvm-dwarfdump llvm-symbolizer llvm-addr2line addr2line cc c++ cpp; do
    command -v $t >/dev/null || echo "missing $t"
  done
  clang --version | head -1
  llvm-ar --version | grep -o 'LLVM version [0-9.]*'
  ld --version
}
hello() { cc hello.c && ./a.out; }
cfeatures() { cc -O2 cfeatures.c -o cfeatures -lm -lpthread && ./cfeatures; }
cxx_hello() { c++ hello.cpp -o hellocxx && ./hellocxx; }
cxx_features() { c++ -std=c++20 -O2 cxxfeatures.cpp -o cxxfeatures && ./cxxfeatures; }
shared() {
  cc -shared -O2 lib.c -o lib.so && c++ -shared -O2 libxx.cpp -o libxx.so && cc -rdynamic -O2 dlmain.c -o dlmain -ldl \
    && ./dlmain
}
# a shared library linked the way build systems for Linux link one: GNU ld's options wasm-ld lacks
ld_options() {
  cc -shared -fPIC -O2 -Wl,--as-needed -Wl,-z,relro,-z,now -Wl,--version-script=lib.map -Wl,-soname,libtest.so.1 \
    -Wl,--no-undefined -Wl,-O1 -Wl,--start-group -Xlinker --exclude-libs -Xlinker ALL -Wl,--end-group -pthread \
    lib.c -o lib2.so && ./dlmain ./lib2.so ./libxx.so | head -2
}
archive() {
  cc -c a.c b.c && ar rcs libab.a a.o b.o && ranlib libab.a && nm libab.a && cc ab.c -L. -lab -o ab && ./ab \
    && strip ab -o ab.stripped && ./ab.stripped && objdump -h ab.stripped | grep -c CODE \
    && llvm-readobj --file-headers ab | grep Format && cc -r a.o b.o -o ab.o && nm ab.o
}
main_envp() { cc envp.c -o envp && ENVP_CHECK=1 ./envp a b; }
lto() { c++ -O2 -flto lto.cpp -o lto && ./lto && c++ -O2 -flto=thin lto.cpp -o ltothin && ./ltothin; }
# strip keeps what dlopen reads of a library (dylink.0: patches/0005), as CMake's install/strip
# and pybind11's modules strip them
strip_shared() {
  cp lib.so lib.stripped.so && cp libxx.so libxx.stripped.so && strip lib.stripped.so && llvm-strip libxx.stripped.so \
    && objdump -h lib.stripped.so | grep -c dylink.0 && ./dlmain ./lib.stripped.so ./libxx.stripped.so | head -1
}
probes() {
  cc -dumpmachine
  test -f "$(cc -print-libgcc-file-name)" && echo "compiler-rt: found"
  cc -E -dM - </dev/null | grep -E '^#define (__linux__|__wasm32__|__wasm_exception_handling__|__PIC__) ' | sort
  printf '#define X 6*7\nX\n' | cpp - | tail -1
  printf 'int main(void) { return 0; }\n' | cc -x c - -o stdin && ./stdin && echo "stdin: built and ran"
  cc -Wl,--version | grep -o '^LLD 24'
  c++ -v 2>&1 | head -1
}

for s in tools hello cfeatures cxx_hello cxx_features shared ld_options archive main_envp lto strip_shared probes; do
  echo "### $s"
  ( $s ) 2>&1
  echo "### status $?"
done
