/* Emerald's native runtime shim (plan-of-plans rows 06, 08). Deliberately
 * minimal — only what generated code needs. printf's variadic calling
 * convention and malloc's ABI are handled here, inside real C compiled by
 * cc — Cranelift-generated code only ever makes ordinary fixed-signature
 * calls into this file (see spec/COMPILER.md and plan 06/08's Decision
 * logs). */

/* Plan 45's `emerald_gets` needs POSIX's `getline` — not declared by
 * plain ISO C. Defined before any system header is included, per
 * `feature_test_macros(7)`. */
#define _POSIX_C_SOURCE 200809L

#include <ctype.h>
#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>

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

/* Plan 36 (string interpolation) — the compiler-known stringification
 * set: `Int64`, `Float64`, `Boolean`. Each allocates via `emerald_alloc`
 * (same no-free contract as `emerald_string_concat`), sized generously
 * for the format used (`%lld` can print at most 20 digits + sign + NUL;
 * `%g` at most a handful more). `%g` matches `emerald_print_f64`'s own
 * format exactly, so an interpolated Float64 renders identically to a
 * directly-`puts`ed one. */
char *emerald_int64_to_string(long long n) {
  char *out = emerald_alloc(32);
  snprintf(out, 32, "%lld", n);
  return out;
}

char *emerald_float64_to_string(double n) {
  char *out = emerald_alloc(48);
  snprintf(out, 48, "%g", n);
  return out;
}

/* No allocation needed — `Boolean`/`ValKind::Bool` values need no heap
 * representation anywhere else in this compiler either; a pointer to a
 * static constant is safe to return and reuse indefinitely. */
char *emerald_bool_to_string(long long b) {
  static char true_str[] = "true";
  static char false_str[] = "false";
  return b != 0 ? true_str : false_str;
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

/* Plan 45 (stdlib strings and I/O) — `leaf-string-basic-intrinsics`.
 * `.length` is a bare `strlen`; `.upcase`/`.downcase` allocate a
 * same-length copy via `emerald_alloc` and convert byte-wise
 * (`toupper`/`tolower` per byte — ASCII only, matching this plan's own
 * Decision log: a byte-wise conversion can corrupt a multi-byte UTF-8
 * sequence, and full Unicode case folding is out of scope). `.strip`
 * trims ASCII space/tab/newline/carriage-return from both ends into a
 * fresh allocation. `.to_i`/`.to_f` are bare `atoll`/`atof`. */
long long emerald_string_length(const char *s) {
  return (long long) strlen(s);
}

char *emerald_string_upcase(const char *s) {
  size_t len = strlen(s);
  char *out = emerald_alloc((long long) (len + 1));
  for (size_t i = 0; i < len; i++) {
    out[i] = (char) toupper((unsigned char) s[i]);
  }
  out[len] = '\0';
  return out;
}

char *emerald_string_downcase(const char *s) {
  size_t len = strlen(s);
  char *out = emerald_alloc((long long) (len + 1));
  for (size_t i = 0; i < len; i++) {
    out[i] = (char) tolower((unsigned char) s[i]);
  }
  out[len] = '\0';
  return out;
}

static int emerald_is_strip_space(char c) {
  return c == ' ' || c == '\t' || c == '\n' || c == '\r';
}

char *emerald_string_strip(const char *s) {
  size_t len = strlen(s);
  size_t start = 0;
  while (start < len && emerald_is_strip_space(s[start])) {
    start++;
  }
  size_t end = len;
  while (end > start && emerald_is_strip_space(s[end - 1])) {
    end--;
  }
  size_t out_len = end - start;
  char *out = emerald_alloc((long long) (out_len + 1));
  memcpy(out, s + start, out_len);
  out[out_len] = '\0';
  return out;
}

long long emerald_string_to_i(const char *s) {
  return atoll(s);
}

double emerald_string_to_f(const char *s) {
  return atof(s);
}

/* `leaf-string-indexing-slicing`. Both unchecked (no bounds check),
 * matching `Array`'s own existing no-bounds-check precedent exactly —
 * an out-of-range access is real, disclosed undefined behavior, not a
 * new UB surface this plan introduces. */
char *emerald_string_char_at(const char *s, long long i) {
  char *out = emerald_alloc(2);
  out[0] = s[i];
  out[1] = '\0';
  return out;
}

char *emerald_string_slice(const char *s, long long start, long long len) {
  char *out = emerald_alloc(len + 1);
  memcpy(out, s + start, (size_t) len);
  out[len] = '\0';
  return out;
}

/* `leaf-string-split`. Two independent `strstr`-based scans (a
 * disclosed, accepted inefficiency over sharing one scan between
 * `.split`/`.split_count` — see the plan's own Decision log) — always
 * literal-substring splitting, every segment including trailing
 * empties, no whitespace-run collapsing. An empty `sep` is a real,
 * disclosed runtime abort, matching `emerald_hash_key_not_found`'s own
 * precedent, rather than looping forever. */
long long emerald_string_split_count(const char *s, const char *sep) {
  size_t sep_len = strlen(sep);
  if (sep_len == 0) {
    fprintf(stderr, "uncaught error: String#split separator must not be empty\n");
    exit(1);
  }
  long long count = 1;
  const char *cursor = s;
  const char *found;
  while ((found = strstr(cursor, sep)) != NULL) {
    count++;
    cursor = found + sep_len;
  }
  return count;
}

void **emerald_string_split(const char *s, const char *sep) {
  size_t sep_len = strlen(sep);
  if (sep_len == 0) {
    fprintf(stderr, "uncaught error: String#split separator must not be empty\n");
    exit(1);
  }
  long long count = emerald_string_split_count(s, sep);
  void **out = emerald_alloc(count * (long long) sizeof(void *));
  const char *cursor = s;
  const char *found;
  long long i = 0;
  while ((found = strstr(cursor, sep)) != NULL) {
    size_t seg_len = (size_t) (found - cursor);
    char *seg = emerald_alloc((long long) (seg_len + 1));
    memcpy(seg, cursor, seg_len);
    seg[seg_len] = '\0';
    out[i] = seg;
    i++;
    cursor = found + sep_len;
  }
  size_t seg_len = strlen(cursor);
  char *seg = emerald_alloc((long long) (seg_len + 1));
  memcpy(seg, cursor, seg_len);
  seg[seg_len] = '\0';
  out[i] = seg;
  return out;
}

/* `leaf-file-io`. Errors are a disclosed runtime abort
 * (`fprintf(stderr, ...); exit(1)`), not wired into the class-based
 * `raise`/`rescue` exception system — mirrors `emerald_hash_key_not_
 * found`'s existing precedent exactly. */
char *emerald_file_read(const char *path) {
  FILE *f = fopen(path, "rb");
  if (f == NULL) {
    fprintf(stderr, "uncaught error: File.read could not open '%s'\n", path);
    exit(1);
  }
  fseek(f, 0, SEEK_END);
  long size = ftell(f);
  fseek(f, 0, SEEK_SET);
  char *out = emerald_alloc(size + 1);
  size_t read = fread(out, 1, (size_t) size, f);
  out[read] = '\0';
  fclose(f);
  return out;
}

void emerald_file_write(const char *path, const char *content) {
  FILE *f = fopen(path, "wb");
  if (f == NULL) {
    fprintf(stderr, "uncaught error: File.write could not open '%s'\n", path);
    exit(1);
  }
  fwrite(content, 1, strlen(content), f);
  fclose(f);
}

/* `leaf-argv-and-gets`. `emerald_build_argv` skips `argv[0]` (the
 * program name, matching Ruby's own `ARGV`) and reuses the OS-owned
 * `argv` string pointers directly with no copy, since they already
 * live for the whole process — the same no-ownership assumption every
 * allocation in this runtime already makes. `emerald_gets` copies into
 * an `emerald_alloc`'d buffer so the returned `String` came from this
 * runtime's one allocator; returns an empty `""` at EOF rather than
 * `nil` (`Type::Nil`'s already-narrow, non-optional scope). */
void **emerald_build_argv(int argc, char **argv) {
  long long count = argc > 0 ? argc - 1 : 0;
  void **out = emerald_alloc(count > 0 ? count * (long long) sizeof(void *) : (long long) sizeof(void *));
  for (long long i = 0; i < count; i++) {
    out[i] = argv[i + 1];
  }
  return out;
}

char *emerald_gets(void) {
  char *line = NULL;
  size_t cap = 0;
  ssize_t n = getline(&line, &cap, stdin);
  if (n < 0) {
    free(line);
    char *empty = emerald_alloc(1);
    empty[0] = '\0';
    return empty;
  }
  char *out = emerald_alloc((long long) (n + 1));
  memcpy(out, line, (size_t) n);
  out[n] = '\0';
  free(line);
  return out;
}
