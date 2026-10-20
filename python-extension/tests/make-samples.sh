#!/usr/bin/env bash
# The binaries lief_check.py parses (tests/samples), made with this machine's clang-19 and LLD:
# small, of every format LIEF reads that LLVM writes, and kept here, so that the checks and their
# expected output (tests/expected) do not depend on the compiler of the day.
#
#   elf-x86_64-static   an x86-64 Linux executable (no libc: _start makes the exit system call)
#   elf-aarch64.so      an AArch64 shared library with a soname and exported functions
#   pe-x86_64.exe       a Windows x86-64 program importing kernel32's ExitProcess, exporting add
#   coff-x86_64.obj     its COFF object
#   macho-arm64.o       a macOS arm64 object, and macho-arm64.dylib, a library linked from it
set -euo pipefail
T="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/samples"
W="$(mktemp -d)"; trap 'rm -rf "$W"' EXIT
cd "$W"
cat > hello.c <<'C'
int counter = 42;
const char message[] = "hello from collabo";
__attribute__((visibility("default"))) int add(int a, int b) { return a + b + counter; }
int square(int x) { return x * x; }
C
cat > start.c <<'C'
extern int add(int, int);
static void sys_exit(long code) {
  __asm__ volatile("syscall" :: "a"(60), "D"(code));
}
void _start(void) { sys_exit(add(1, 2)); }
C
cat > win.c <<'C'
__declspec(dllimport) void __stdcall ExitProcess(unsigned);
__declspec(dllexport) int add(int a, int b) { return a + b; }
void mainCRTStartup(void) { ExitProcess(add(1, 2)); }
C
mkdir -p "$T"
clang-19 --target=x86_64-linux-gnu -O1 -nostdlib -static -fuse-ld=lld -Wl,--build-id=none -o "$T/elf-x86_64-static" hello.c start.c
clang-19 --target=aarch64-linux-gnu -O1 -nostdlib -shared -fPIC -fuse-ld=lld -Wl,--build-id=none \
  -Wl,-soname,libcollabo.so.1 -o "$T/elf-aarch64.so" hello.c
clang-19 --target=x86_64-pc-windows-msvc -O1 -c win.c -o "$T/coff-x86_64.obj"
printf 'EXPORTS\nExitProcess\n' > kernel32.def
llvm-dlltool-19 -m i386:x86-64 -d kernel32.def -D kernel32.dll -l kernel32.lib
lld-link-19 /nologo /entry:mainCRTStartup /subsystem:console /Brepro /out:"$T/pe-x86_64.exe" "$T/coff-x86_64.obj" kernel32.lib
rm -f "$T/pe-x86_64.lib"
clang-19 --target=arm64-apple-macos11 -O1 -c hello.c -o "$T/macho-arm64.o"
clang-19 --target=arm64-apple-macos11 -O1 -nostdlib -shared -fuse-ld=lld -o "$T/macho-arm64.dylib" hello.c
ls -l "$T"
