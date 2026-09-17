#include <cstdio>

struct Box {
  long long value;
};

int main() {
  long long total = 0;
  long long i = 0;
  while (i < 1000000LL) {
    Box *b = new Box{i};
    total += b->value;
    delete b;
    i += 1;
  }
  std::printf("%lld\n", total);
  return 0;
}
