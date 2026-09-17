#include <cstdio>

long long square(long long n) { return n * n; }

int main() {
  long long total = 0;
  long long i = 0;
  while (i < 1000000LL) {
    total += square(i);
    i += 1;
  }
  std::printf("%lld\n", total);
  return 0;
}
