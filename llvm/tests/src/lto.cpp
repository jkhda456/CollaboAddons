/* Exceptions and setjmp/longjmp through -flto: compiled to bitcode, code generated in wasm-ld,
   which cc gives the compile's exception options (-Wl,-mllvm,...). */
#include <csetjmp>
#include <cstdio>
#include <stdexcept>
#include <string>

static std::jmp_buf jb;
[[gnu::noinline]] static void jump(int x) { if (x) std::longjmp(jb, x); }
[[gnu::noinline]] static int deep(int x) { if (x > 2) throw std::runtime_error("deep " + std::to_string(x)); return x; }

int main(int argc, char **) {
  try { deep(argc + 2); } catch (const std::exception &e) { std::printf("lto caught: %s\n", e.what()); }
  if (int r = setjmp(jb)) std::printf("lto longjmp: %d\n", r); else jump(argc + 6);
  return 0;
}
