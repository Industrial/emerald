#include <stdio.h>

long long square(long long n) { return n * n; }

int main(void) {
  long long total = 0;
  long long i = 0;
  while (i < 1000000LL) {
    total += square(i);
    i += 1;
  }
  printf("%lld\n", total);
  return 0;
}
