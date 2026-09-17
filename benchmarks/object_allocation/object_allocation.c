#include <stdio.h>
#include <stdlib.h>

typedef struct {
  long long value;
} Box;

Box *box_new(long long v) {
  Box *b = (Box *) malloc(sizeof(Box));
  b->value = v;
  return b;
}

int main(void) {
  long long total = 0;
  long long i = 0;
  while (i < 1000000LL) {
    Box *b = box_new(i);
    total += b->value;
    free(b);
    i += 1;
  }
  printf("%lld\n", total);
  return 0;
}
