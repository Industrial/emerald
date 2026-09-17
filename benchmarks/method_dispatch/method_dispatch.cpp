#include <cstdio>

class Adder {
public:
  long long base;
  Adder(long long b) : base(b) {}
  long long add(long long n) { return base + n; }
};

int main() {
  Adder a(1);
  long long total = 0;
  long long i = 0;
  while (i < 10000000LL) {
    total += a.add(i);
    i += 1;
  }
  std::printf("%lld\n", total);
  return 0;
}
