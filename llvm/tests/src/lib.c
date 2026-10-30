/* A shared library (cc -shared): a WebAssembly side module the program loads with dlopen. Its own
   data and constructor, the program's C library (printf, its setjmp/longjmp), thread-local
   storage of its own, a function pointer handed back. */
#include <setjmp.h>
#include <stdio.h>

static int counter = 40;
static _Thread_local int calls;
static jmp_buf jb;

__attribute__((constructor)) static void init(void) { counter += 2; }

int lib_add(int a, int b) {
  calls++;
  return a + b + counter;
}

static void jump(void) { longjmp(jb, 9); }

int lib_setjmp(void) {
  volatile int r = setjmp(jb);
  if (r == 0) jump();
  return r;
}

int lib_calls(void) { return calls; }

static int twice(int x) { return 2 * x; }
int (*lib_fn(void))(int) { return twice; }

void lib_hello(const char *who) { printf("hello from lib.so, %s\n", who); }
