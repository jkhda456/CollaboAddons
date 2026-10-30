/* The program that loads lib.so and libxx.so (cc -rdynamic: the C library in it and exported). */
#include <dlfcn.h>
#include <stdio.h>

int main(int argc, char **argv) {
  void *h = dlopen(argc > 1 ? argv[1] : "./lib.so", RTLD_NOW);
  if (!h) { printf("dlopen: %s\n", dlerror()); return 1; }
  int (*add)(int, int) = (int (*)(int, int))dlsym(h, "lib_add");
  int (*sj)(void) = (int (*)(void))dlsym(h, "lib_setjmp");
  int (*calls)(void) = (int (*)(void))dlsym(h, "lib_calls");
  int (*(*fn)(void))(int) = (int (*(*)(void))(int))dlsym(h, "lib_fn");
  void (*hello)(const char *) = (void (*)(const char *))dlsym(h, "lib_hello");
  printf("lib_add: %d\n", add(1, 2));
  printf("lib_setjmp: %d\n", sj());
  printf("lib_calls: %d\n", calls());
  printf("lib_fn: %d\n", fn()(21));
  hello("main");
  printf("missing: %s\n", dlsym(h, "no_such_symbol") ? "found" : "not found");

  void *x = dlopen(argc > 2 ? argv[2] : "./libxx.so", RTLD_NOW);
  if (!x) { printf("dlopen: %s\n", dlerror()); return 1; }
  int (*entry)(int, char *, int) = (int (*)(int, char *, int))dlsym(x, "libxx_entry");
  char buf[128];
  entry(2, buf, sizeof buf);
  printf("libxx: %s\n", buf);
  entry(5, buf, sizeof buf);
  printf("libxx: %s\n", buf);
  return 0;
}
