/* The C library and compiler-rt as a program of the guest's sees them: setjmp/longjmp (WebAssembly
   exceptions), threads with thread-local storage, the math library, long double and __int128
   (compiler-rt), a large allocation, files. */
#include <math.h>
#include <pthread.h>
#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static jmp_buf jb;
static _Thread_local int tls = 1;

static void jump(int v) { longjmp(jb, v); }

static void *worker(void *arg) {
  tls = 2;
  return (void *)(long)(tls * 100 + *(int *)arg);
}

static int cmp(const void *a, const void *b) { return *(const int *)a - *(const int *)b; }

int main(void) {
  volatile int r = setjmp(jb);
  if (r == 0) jump(42);
  printf("longjmp: %d\n", r);

  int arg = 7;
  pthread_t t;
  void *ret;
  pthread_create(&t, NULL, worker, &arg);
  pthread_join(t, &ret);
  printf("thread: %ld, main's tls still %d\n", (long)ret, tls);

  volatile double x = 2.0;
  printf("math: %.6f %.6f\n", sqrt(x) * cos(0.5), pow(x, 10.5));
  volatile long double third = 1.0L / 3;
  printf("long double: %.20Lf\n", third);
  volatile unsigned __int128 big = (unsigned __int128)0xffffffffffffffffULL * 0x10001;
  printf("__int128: %llx%016llx\n", (unsigned long long)(big >> 64), (unsigned long long)big);

  size_t n = 64 << 20;
  char *p = malloc(n);
  memset(p, 0x5a, n);
  printf("malloc 64 MiB: %d\n", p[n - 1] == 0x5a);
  free(p);

  int v[] = {5, 3, 9, 1};
  qsort(v, 4, sizeof v[0], cmp);
  printf("qsort: %d %d %d %d\n", v[0], v[1], v[2], v[3]);

  FILE *f = fopen("cfeatures.txt", "w");
  fprintf(f, "written %d\n", 123);
  fclose(f);
  char line[64] = "";
  f = fopen("cfeatures.txt", "r");
  fgets(line, sizeof line, f);
  fclose(f);
  printf("file: %s", line);
  return 0;
}
