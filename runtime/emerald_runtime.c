/* Emerald's native runtime shim (plan-of-plans row 06). Deliberately
 * minimal: one function, matching exactly what generated code needs to
 * print inception §17's milestone-1 result. printf's variadic calling
 * convention is handled here, inside real C compiled by cc — Cranelift-
 * generated code only ever makes an ordinary fixed-signature call into
 * this function (see spec/COMPILER.md and plan 06's Decision log). */

#include <stdio.h>

void emerald_print_i64(long long n) {
  printf("%lld\n", n);
}
