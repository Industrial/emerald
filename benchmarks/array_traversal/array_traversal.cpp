#include <cstdio>

int main() {
  long long arr[20] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
                        11, 12, 13, 14, 15, 16, 17, 18, 19, 20};
  long long total = 0;
  long long rep = 0;
  while (rep < 1000000LL) {
    int i = 0;
    while (i < 20) {
      total += arr[i];
      i += 1;
    }
    rep += 1;
  }
  std::printf("%lld\n", total);
  return 0;
}
