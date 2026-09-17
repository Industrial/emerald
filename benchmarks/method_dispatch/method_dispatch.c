#include <stdio.h>

typedef struct {
  long long base;
} Adder;

long long adder_add(Adder *a, long long n) { return a->base + n; }

int main(void) {
  Adder a = {1};
  long long total = 0;
  long long i = 0;
  while (i < 10000000LL) {
    total += adder_add(&a, i);
    i += 1;
  }
  printf("%lld\n", total);
  return 0;
}
