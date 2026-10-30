/* main with three parameters (make's): clang names it as the C library's start code does not
   call it, so the C library bridges (userspace/patches/musl-main-envp.patch). */
#include <stdio.h>
#include <string.h>

int main(int argc, char **argv, char **envp) {
  int n = 0, found = 0;
  for (; envp[n]; n++) found |= strncmp(envp[n], "ENVP_CHECK=", 11) == 0;
  printf("envp: argc %d, ENVP_CHECK %s\n", argc, found ? "there" : "missing");
  return 0;
}
