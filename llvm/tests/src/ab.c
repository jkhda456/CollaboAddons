#include <stdio.h>
int a_value(void);
int b_value(void);
int main(void) {
  printf("archive: %d\n", a_value() + b_value());
  return 0;
}
