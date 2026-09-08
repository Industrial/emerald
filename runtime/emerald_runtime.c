/* Emerald's native runtime shim (plan-of-plans rows 06, 08). Deliberately
 * minimal — only what generated code needs. printf's variadic calling
 * convention and malloc's ABI are handled here, inside real C compiled by
 * cc — Cranelift-generated code only ever makes ordinary fixed-signature
 * calls into this file (see spec/COMPILER.md and plan 06/08's Decision
 * logs). */

#include <stdio.h>
#include <stdlib.h>

void emerald_print_i64(long long n) {
  printf("%lld\n", n);
}

void emerald_print_f64(double n) {
  printf("%g\n", n);
}

/* Thin malloc wrapper backing `ClassName.new` (plan 08). No corresponding
 * free — no GC, no lifetime tracking yet; matches inception §12 (no
 * ownership system, no GC pressure, for now). */
void *emerald_alloc(long long size) {
  return malloc((size_t) size);
}
