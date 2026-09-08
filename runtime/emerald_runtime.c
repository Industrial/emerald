/* Emerald's native runtime shim (plan-of-plans rows 06, 08). Deliberately
 * minimal — only what generated code needs. printf's variadic calling
 * convention and malloc's ABI are handled here, inside real C compiled by
 * cc — Cranelift-generated code only ever makes ordinary fixed-signature
 * calls into this file (see spec/COMPILER.md and plan 06/08's Decision
 * logs). */

#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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

/* `Array.new(size)` (plan 25's Decision log) — `calloc`-backed so the
 * buffer is genuinely zero-filled, unlike `emerald_alloc`'s bare
 * `malloc`. No length tracking, matching `emerald_alloc`'s own
 * no-bounds-info contract. */
void *emerald_alloc_zeroed(long long size) {
  return calloc((size_t) size, 1);
}

/* `Hash[K, V]` indexed read/write with no matching key (plan 25's
 * Decision log): a real, disclosed runtime abort — not silently
 * undefined behavior the way out-of-bounds `Array` access is (plan 09)
 * — since the linear-scan lookup that finds this case is code this
 * runtime fully controls, unlike raw pointer arithmetic. */
void emerald_hash_key_not_found(void) {
  fprintf(stderr, "uncaught error: Hash key not found\n");
  exit(1);
}

/* Plan 19 (string literals). `String` is a bare pointer to a
 * null-terminated UTF-8 buffer (spec/TYPE_SYSTEM.md §9) — no length
 * header — specifically so these three helpers can reuse ordinary libc
 * string functions with zero new conventions to hand-roll. */
void emerald_print_str(const char *s) {
  printf("%s\n", s);
}

/* `a + b` on two `String`s. Allocates via the existing `emerald_alloc`
 * (no corresponding free — matches `emerald_alloc`'s own no-GC
 * contract above) rather than calling `malloc` directly, so every
 * heap allocation this runtime makes goes through the one path. */
char *emerald_string_concat(const char *a, const char *b) {
  size_t len_a = strlen(a);
  size_t len_b = strlen(b);
  char *out = emerald_alloc((long long) (len_a + len_b + 1));
  memcpy(out, a, len_a);
  memcpy(out + len_a, b, len_b);
  out[len_a + len_b] = '\0';
  return out;
}

/* `a == b` / `a != b` on two `String`s — byte-for-byte comparison, not
 * pointer identity. Returns `0`/`1` rather than a real C `bool` so
 * generated code can compare it against the `Int64` constant `0` via
 * the same integer-comparison machinery every other codegen path
 * already uses. */
long long emerald_string_eq(const char *a, const char *b) {
  return strcmp(a, b) == 0 ? 1 : 0;
}

/* `raise`/`begin...rescue...end` (plan 11) — a setjmp/longjmp handler
 * stack, NOT true native (DWARF-unwind + personality-function) exceptions
 * (see plan 11's Decision log for why: real unwind-table-based exceptions
 * are a much larger undertaking than this project's current scope).
 * `setjmp` itself is called *directly* by Cranelift-generated code, never
 * wrapped in a function here — the C standard requires the call site and
 * the frame it's called from to still be live when `longjmp` targets it,
 * so a wrapper would jump back into an already-returned stack frame.
 * These helpers only manage the handler *stack itself*, which has no such
 * restriction. Single-threaded only (no ownership/concurrency in v1 —
 * inception §12/§20), so a plain global linked list is enough. */
typedef struct EmeraldHandler {
  jmp_buf buf;
  long long exception_tag;
  void *exception_ptr;
  struct EmeraldHandler *prev;
} EmeraldHandler;

static EmeraldHandler *emerald_handler_stack = NULL;

void *emerald_push_handler(void) {
  EmeraldHandler *h = malloc(sizeof(EmeraldHandler));
  h->exception_tag = -1;
  h->exception_ptr = NULL;
  h->prev = emerald_handler_stack;
  emerald_handler_stack = h;
  return h;
}

/* The pointer generated code must pass directly to `setjmp`. */
void *emerald_handler_jmpbuf(void *handler) {
  return (void *) ((EmeraldHandler *) handler)->buf;
}

/* Pops AND frees the still-linked top-of-stack handler — the "no
 * exception occurred" path, after the `begin` body completes normally. */
void emerald_pop_handler(void) {
  EmeraldHandler *h = emerald_handler_stack;
  emerald_handler_stack = h->prev;
  free(h);
}

/* Frees a handler `emerald_raise` already unlinked from the stack (its
 * tag/pointer have been read by the `rescue` landing pad, matched or
 * re-raised) — unlike `emerald_pop_handler`, this does NOT touch
 * `emerald_handler_stack`, since the handler is no longer on it. */
void emerald_free_handler(void *handler) {
  free(handler);
}

long long emerald_handler_tag(void *handler) {
  return ((EmeraldHandler *) handler)->exception_tag;
}

void *emerald_handler_exception_ptr(void *handler) {
  return ((EmeraldHandler *) handler)->exception_ptr;
}

/* Pops the innermost handler (it's about to be consumed — a `longjmp`
 * only fires once per `setjmp`), records the raised exception's class
 * tag and pointer on it, and jumps back to its `begin` site. With no
 * handler left (an uncaught exception, or a caught-but-tag-mismatched
 * one re-raised past the outermost `begin`), prints a diagnostic and
 * exits rather than continuing in an undefined state. */
void emerald_raise(long long tag, void *exception_ptr) {
  EmeraldHandler *h = emerald_handler_stack;
  if (h == NULL) {
    fprintf(stderr, "uncaught Emerald exception (class tag %lld)\n", tag);
    exit(1);
  }
  emerald_handler_stack = h->prev;
  h->exception_tag = tag;
  h->exception_ptr = exception_ptr;
  longjmp(h->buf, 1);
}
