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
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
/* Plan 55 (scheduler and message passing). */
#include <pthread.h>
#include <unistd.h>

void emerald_print_i64(long long n) {
  printf("%lld\n", n);
}

void emerald_print_f64(double n) {
  printf("%g\n", n);
}

/* Plan 51 (scope-based arena allocation) — a process-wide, single-
 * threaded (matches every other piece of shared state in this file —
 * no ownership/concurrency until plan 54's actor model) byte-
 * outstanding counter. Incremented by every `malloc`/`calloc` this
 * runtime performs (`emerald_alloc`/`emerald_alloc_zeroed` below, and
 * a region's own chunk growth further down); decremented only by
 * `emerald_region_destroy`'s frees — `emerald_alloc`'s own bytes never
 * come back down, since it has no corresponding free. Exists purely so
 * a test harness can measure "did memory actually stay bounded" as a
 * real number instead of trusting the design argument alone. */
static long long emerald_bytes_outstanding_counter = 0;

/* Plan 55's own mandatory, disclosed second fix to existing code
 * (alongside the exception handler stack becoming thread-local, below):
 * an actor method body can now genuinely run concurrently with another
 * on a separate OS thread and allocate (a `String` concat, an `Array.
 * new`, ...) at the same time — a plain non-atomic `+=`/`-=` on this
 * shared global would be a real, not hypothetical, data race (this
 * plan's own `Spinner` concurrency proof deliberately runs two actor
 * method bodies at once). Every update site below uses
 * `__atomic_fetch_add`/`__atomic_load_n` (a GCC/Clang builtin, no new
 * dependency) with relaxed ordering — sufficient since this counter is
 * only ever read back for diagnostic/test purposes, never used to gate
 * another memory access. */
long long emerald_bytes_outstanding(void) {
  return __atomic_load_n(&emerald_bytes_outstanding_counter, __ATOMIC_RELAXED);
}

/* Thin malloc wrapper backing `ClassName.new` (plan 08). No corresponding
 * free — no GC, no lifetime tracking yet; matches inception §12 (no
 * ownership system, no GC pressure, for now). */
void *emerald_alloc(long long size) {
  __atomic_fetch_add(&emerald_bytes_outstanding_counter, size, __ATOMIC_RELAXED);
  return malloc((size_t) size);
}

/* `Array.new(size)` (plan 25's Decision log) — `calloc`-backed so the
 * buffer is genuinely zero-filled, unlike `emerald_alloc`'s bare
 * `malloc`. No length tracking, matching `emerald_alloc`'s own
 * no-bounds-info contract. */
void *emerald_alloc_zeroed(long long size) {
  __atomic_fetch_add(&emerald_bytes_outstanding_counter, size, __ATOMIC_RELAXED);
  return calloc((size_t) size, 1);
}

/* Plan 51 (scope-based arena allocation) — a bump-allocated region
 * scoped to (in the future codegen consumer this leaf's own runtime
 * primitive is built for) a function's call frame, freed in one bulk
 * operation rather than per-object. `EmeraldRegion` owns a linked list
 * of growing chunks — never one `realloc`'d buffer, since `realloc`
 * may move memory and would invalidate every pointer this region has
 * already handed out. Single-threaded, matches every other helper in
 * this file (see the byte-counter comment above); no per-object free
 * inside a region (`emerald_region_free_one`-style) — that would
 * defeat the entire point of a bulk-free arena. */
#define EMERALD_REGION_DEFAULT_CHUNK_SIZE ((size_t) 4096)

typedef struct EmeraldRegionChunk {
  struct EmeraldRegionChunk *next;
  size_t capacity;
  size_t used;
  unsigned char data[];
} EmeraldRegionChunk;

typedef struct EmeraldRegion {
  /* The chunk allocations are currently bump-allocated from — the
   * most recently grown one. Older, now-full chunks stay reachable
   * via `next` purely so `emerald_region_destroy` can free them all;
   * `emerald_region_alloc` never looks at anything but `head`. */
  EmeraldRegionChunk *head;
} EmeraldRegion;

static EmeraldRegionChunk *emerald_region_new_chunk(size_t min_capacity) {
  size_t capacity = min_capacity > EMERALD_REGION_DEFAULT_CHUNK_SIZE
                       ? min_capacity
                       : EMERALD_REGION_DEFAULT_CHUNK_SIZE;
  EmeraldRegionChunk *chunk = malloc(sizeof(EmeraldRegionChunk) + capacity);
  chunk->next = NULL;
  chunk->capacity = capacity;
  chunk->used = 0;
  __atomic_fetch_add(&emerald_bytes_outstanding_counter,
                      (long long) (sizeof(EmeraldRegionChunk) + capacity),
                      __ATOMIC_RELAXED);
  return chunk;
}

void *emerald_region_create(void) {
  EmeraldRegion *region = malloc(sizeof(EmeraldRegion));
  __atomic_fetch_add(&emerald_bytes_outstanding_counter,
                      (long long) sizeof(EmeraldRegion), __ATOMIC_RELAXED);
  region->head = emerald_region_new_chunk(EMERALD_REGION_DEFAULT_CHUNK_SIZE);
  return region;
}

/* Bump-allocates `size` bytes from `region`'s current chunk, appending
 * a new chunk (doubling growth, or exactly `size` if that's larger —
 * mirroring Zig's `std.heap.ArenaAllocator`'s own growth policy) if
 * the current chunk lacks room. Every request is 8-byte-aligned within
 * its chunk, matching the alignment `malloc` itself already
 * guarantees on every platform this runtime targets — never returns
 * `NULL` for a satisfiable request, the same no-OOM-check contract
 * `emerald_alloc` already has. */
void *emerald_region_alloc(void *region_ptr, long long size) {
  EmeraldRegion *region = (EmeraldRegion *) region_ptr;
  size_t aligned = ((size_t) size + 7) & ~((size_t) 7);
  EmeraldRegionChunk *chunk = region->head;
  if (chunk->used + aligned > chunk->capacity) {
    size_t new_capacity = chunk->capacity * 2;
    if (new_capacity < aligned) {
      new_capacity = aligned;
    }
    EmeraldRegionChunk *new_chunk = emerald_region_new_chunk(new_capacity);
    new_chunk->next = chunk;
    region->head = new_chunk;
    chunk = new_chunk;
  }
  void *ptr = chunk->data + chunk->used;
  chunk->used += aligned;
  return ptr;
}

/* The region's one bulk-free operation: every chunk it ever grew into,
 * then the region's own control structure. */
void emerald_region_destroy(void *region_ptr) {
  EmeraldRegion *region = (EmeraldRegion *) region_ptr;
  EmeraldRegionChunk *chunk = region->head;
  while (chunk != NULL) {
    EmeraldRegionChunk *next = chunk->next;
    __atomic_fetch_sub(&emerald_bytes_outstanding_counter,
                        (long long) (sizeof(EmeraldRegionChunk) + chunk->capacity),
                        __ATOMIC_RELAXED);
    free(chunk);
    chunk = next;
  }
  __atomic_fetch_sub(&emerald_bytes_outstanding_counter,
                      (long long) sizeof(EmeraldRegion), __ATOMIC_RELAXED);
  free(region);
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
 * restriction. Plan 55's own mandatory, disclosed fix to this existing
 * code: with a real worker pool now running actor method bodies
 * concurrently on separate OS threads, two threads can each `raise`/
 * `rescue` at once — a single shared global handler stack would let one
 * thread's `push_handler`/`raise` corrupt another's. `_Thread_local`
 * gives each OS thread (including every worker) its own independent
 * stack with the exact same single-threaded semantics as before from
 * any one thread's own point of view — no other change to this
 * mechanism. */
typedef struct EmeraldHandler {
  jmp_buf buf;
  long long exception_tag;
  void *exception_ptr;
  struct EmeraldHandler *prev;
} EmeraldHandler;

static _Thread_local EmeraldHandler *emerald_handler_stack = NULL;

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

/* Plan 53 (Result type and error propagation) — `is_valid_int`/
 * `parse_digits`'s runtime backing, both `Int64`-returning per the
 * "plain i64, not i1/C bool" ABI convention `emerald_bool_to_string`
 * already established for this file. A plain ASCII-digit scan with an
 * optional leading `-` (a real, disclosed narrower check than a full
 * numeric-literal grammar — no leading `+`, no whitespace, no
 * exponent/decimal forms, matching this plan's own `Int64`-only
 * worked example); `strtoll` for the actual value once validity is
 * already confirmed, no dependency on plan 45's `emerald_string_to_i`
 * (which silently returns `0` for invalid input instead of signaling
 * failure — exactly the gap `Result[T, E]` exists to close). */
long long emerald_is_valid_int(const char *s) {
  if (s == NULL || s[0] == '\0') {
    return 0;
  }
  size_t i = 0;
  if (s[0] == '-') {
    i = 1;
  }
  if (s[i] == '\0') {
    return 0;
  }
  for (; s[i] != '\0'; i++) {
    if (!isdigit((unsigned char) s[i])) {
      return 0;
    }
  }
  return 1;
}

long long emerald_parse_digits(const char *s) {
  return strtoll(s, NULL, 10);
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

/* Plan 55 (scheduler and message passing) — "N actors over M OS
 * threads" (see the plan's own Decision log for why this is not a
 * green-thread M:N scheduler): a fixed pool of pthread workers, a
 * mutex/condvar-guarded mailbox per actor, and one shared mutex/
 * condvar-guarded runnable-actor queue all M workers consume from.
 *
 * Actor arena layout (codegen's own contract with this file): every
 * `.spawn`-allocated arena reserves its own leading 8 bytes (one
 * pointer width — the same uniform width every other field in this
 * backend already uses) as a header-pointer slot, holding a pointer to
 * this file's own, separately `malloc`'d `EmeraldActorHeader`. `self`,
 * everywhere else in this compiler (field access, `initialize`, an
 * ordinary same-actor method call), is the address *past* that slot —
 * `emerald_actor_enqueue`'s very first job is walking back one pointer
 * width from `self` to recover the header. This sidesteps ever needing
 * `crates/emerald-codegen` to know this struct's real C `sizeof` (which
 * varies by platform/libc, e.g. `pthread_mutex_t`'s own size) — codegen
 * only ever needs to know "one pointer width," a constant it already
 * relies on everywhere.
 *
 * Safety argument (restated from the Decision log, load-bearing enough
 * to repeat here next to the code it governs): an actor is in exactly
 * one of three mutually exclusive states — idle (`scheduled == 0`,
 * mailbox empty or about to be appended to), runnable-but-unclaimed
 * (`scheduled == 1`, linked into the global queue, no worker executing
 * it yet), or currently-executing (`scheduled == 1`, unlinked from the
 * queue, exactly one worker inside its mailbox-drain loop). The
 * idle -> runnable transition and the "did a message arrive while I was
 * about to go idle" recheck both happen while holding that ONE actor's
 * own `mailbox_mutex`, so no two workers can ever observe the same
 * actor as claimable at once. This is why no lock guards an actor's own
 * fields anywhere in generated code: at most one thread is ever inside
 * a given actor's code, full stop. */

#define EMERALD_MESSAGE_ARGV_MAX 16

typedef struct EmeraldMessage {
  void (*trampoline)(void *self, long long *argv);
  long long argv[EMERALD_MESSAGE_ARGV_MAX];
  struct EmeraldMessage *next;
} EmeraldMessage;

typedef struct EmeraldActorHeader {
  /* The instance's own field region — `arena_base + sizeof(void*)` —
   * cached here so a worker dequeuing this header already has the
   * exact pointer every trampoline expects as its `self` argument. */
  void *self;
  pthread_mutex_t mailbox_mutex;
  EmeraldMessage *mailbox_head;
  EmeraldMessage *mailbox_tail;
  /* 0 = idle (not linked into the runnable queue, no worker owns it);
   * 1 = runnable-or-executing (see this section's own doc comment). */
  int scheduled;
  /* Plan 57 (supervision trees). Set once, true forever, under
   * `mailbox_mutex` — `emerald_actor_enqueue` drops any further send
   * once set (Decision log: a dead reference's send silently drops,
   * matching Erlang's own dead-Pid-send behavior), and `emerald_
   * worker_main`'s own drain loop stops calling this actor's
   * trampolines and discards whatever else was already queued. */
  int terminated;
  /* This actor's own region handle (plan 51), set by `.spawn`'s own
   * codegen via `emerald_actor_set_region` right after `emerald_
   * region_alloc` — `NULL` until then, and for any actor nobody ever
   * calls the setter for (this leaf's own pre-existing test harnesses,
   * which never terminate their actors). Bulk-freed by `emerald_
   * actor_terminate` — plan 51's own disclosed "actor termination is a
   * second, later-arriving trigger for the identical bulk-free
   * operation" materializing for real. */
  void *region;
  /* `NULL` unless this actor was registered as a tracked child of a
   * `supervise do ... end` block (`emerald_supervisor_register_child`)
   * — consulted only by `emerald_actor_terminate`. */
  void *supervisor;
  long long child_slot;
  struct EmeraldActorHeader *next_runnable;
} EmeraldActorHeader;

/* Called once per `.spawn`, on the raw allocation, before `initialize`
 * runs — mallocs the real header, stores this instance's field-region
 * pointer on it, and writes the header pointer into the arena's own
 * leading 8-byte slot (see this section's own doc comment). */
void *emerald_actor_init_header(void *arena_base) {
  EmeraldActorHeader *header = malloc(sizeof(EmeraldActorHeader));
  __atomic_fetch_add(&emerald_bytes_outstanding_counter,
                      (long long) sizeof(EmeraldActorHeader), __ATOMIC_RELAXED);
  header->self = (char *) arena_base + sizeof(void *);
  pthread_mutex_init(&header->mailbox_mutex, NULL);
  header->mailbox_head = NULL;
  header->mailbox_tail = NULL;
  header->scheduled = 0;
  header->terminated = 0;
  header->region = NULL;
  header->supervisor = NULL;
  header->child_slot = -1;
  header->next_runnable = NULL;
  *(void **) arena_base = header;
  return header;
}

/* Plan 57's own addition — a separate setter rather than widening
 * `emerald_actor_init_header`'s own signature, so every existing
 * caller (codegen's own `.spawn` call site from before this plan, and
 * every pre-existing hand-written C test harness that constructs a
 * bare actor arena directly) keeps compiling completely unchanged;
 * only `.spawn`'s own codegen calls this, immediately after `emerald_
 * region_alloc`. */
void emerald_actor_set_region(void *self, void *region) {
  EmeraldActorHeader *header = *(EmeraldActorHeader **) ((char *) self - sizeof(void *));
  header->region = region;
}

static pthread_mutex_t emerald_runnable_mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t emerald_runnable_cond = PTHREAD_COND_INITIALIZER;
static EmeraldActorHeader *emerald_runnable_head = NULL;
static EmeraldActorHeader *emerald_runnable_tail = NULL;
/* Every message from the moment it's enqueued until its trampoline call
 * returns — the sole condition `emerald_worker_pool_drain_and_join`
 * waits on, so a caller never guesses via a fixed sleep whether every
 * send has actually finished processing. */
static long long emerald_outstanding_messages = 0;
static int emerald_pool_shutdown = 0;
static pthread_t *emerald_workers = NULL;
static int emerald_worker_count = 0;
static int emerald_pool_started = 0;

/* Builds a message node, appends it to `self`'s own actor's mailbox,
 * and — only on that actor's idle -> runnable transition — links it
 * onto the shared runnable queue and wakes one worker. `argc` beyond
 * `EMERALD_MESSAGE_ARGV_MAX` truncates (sema/codegen's own compile-time
 * arity cap on a cross-actor call keeps this from ever firing in
 * practice — see the plan's own Decision log). */
void emerald_actor_enqueue(void *self, void (*trampoline)(void *, long long *),
                            long long *argv, long long argc) {
  EmeraldActorHeader *header = *(EmeraldActorHeader **) ((char *) self - sizeof(void *));

  EmeraldMessage *msg = malloc(sizeof(EmeraldMessage));
  msg->trampoline = trampoline;
  long long n = argc < EMERALD_MESSAGE_ARGV_MAX ? argc : EMERALD_MESSAGE_ARGV_MAX;
  for (long long i = 0; i < n; i++) {
    msg->argv[i] = argv[i];
  }
  msg->next = NULL;

  int need_schedule = 0;
  pthread_mutex_lock(&header->mailbox_mutex);
  /* Plan 57 (supervision trees): a terminated actor's mailbox never
   * accepts another message — silently dropped, matching Erlang's own
   * dead-Pid-send behavior (Decision log). Checked and appended inside
   * the SAME `mailbox_mutex` critical section as the outstanding-
   * message increment below, so there is no window where a message
   * could be counted as in-flight (blocking `emerald_worker_pool_
   * drain_and_join`/`emerald_supervisor_child` forever) without ever
   * actually being enqueued anywhere. */
  if (header->terminated) {
    pthread_mutex_unlock(&header->mailbox_mutex);
    free(msg);
    return;
  }
  pthread_mutex_lock(&emerald_runnable_mutex);
  emerald_outstanding_messages++;
  pthread_mutex_unlock(&emerald_runnable_mutex);
  if (header->mailbox_tail == NULL) {
    header->mailbox_head = msg;
    header->mailbox_tail = msg;
  } else {
    header->mailbox_tail->next = msg;
    header->mailbox_tail = msg;
  }
  if (!header->scheduled) {
    header->scheduled = 1;
    need_schedule = 1;
  }
  pthread_mutex_unlock(&header->mailbox_mutex);

  if (need_schedule) {
    pthread_mutex_lock(&emerald_runnable_mutex);
    header->next_runnable = NULL;
    if (emerald_runnable_tail == NULL) {
      emerald_runnable_head = header;
      emerald_runnable_tail = header;
    } else {
      emerald_runnable_tail->next_runnable = header;
      emerald_runnable_tail = header;
    }
    pthread_cond_signal(&emerald_runnable_cond);
    pthread_mutex_unlock(&emerald_runnable_mutex);
  }
}

static void *emerald_worker_main(void *arg) {
  (void) arg;
  for (;;) {
    pthread_mutex_lock(&emerald_runnable_mutex);
    while (emerald_runnable_head == NULL && !emerald_pool_shutdown) {
      pthread_cond_wait(&emerald_runnable_cond, &emerald_runnable_mutex);
    }
    if (emerald_runnable_head == NULL && emerald_pool_shutdown) {
      pthread_mutex_unlock(&emerald_runnable_mutex);
      break;
    }
    EmeraldActorHeader *header = emerald_runnable_head;
    emerald_runnable_head = header->next_runnable;
    if (emerald_runnable_head == NULL) {
      emerald_runnable_tail = NULL;
    }
    header->next_runnable = NULL;
    pthread_mutex_unlock(&emerald_runnable_mutex);

    /* Drain every message this actor has right now, one at a time, to
     * completion — real FIFO order, since both this dequeue and any
     * concurrent `emerald_actor_enqueue`'s append serialize on the same
     * `mailbox_mutex`. Only clear `scheduled` (going back to idle) once
     * the mailbox is observed empty under that same lock — the
     * invariant this section's own doc comment states. */
    for (;;) {
      pthread_mutex_lock(&header->mailbox_mutex);
      /* Plan 57 (supervision trees): a message's own trampoline call
       * (below) can terminate this actor via `emerald_actor_terminate`
       * — checked back here, at the top of every iteration, so the
       * NEXT message never gets dispatched against a now-freed arena.
       * Every message still queued at that point is drained and
       * dropped here instead (each one still counted in `emerald_
       * outstanding_messages`, so each must still be individually
       * decremented, or `drain_and_join`/`emerald_supervisor_child`
       * would wait forever). */
      if (header->terminated) {
        EmeraldMessage *dead = header->mailbox_head;
        header->mailbox_head = NULL;
        header->mailbox_tail = NULL;
        header->scheduled = 0;
        pthread_mutex_unlock(&header->mailbox_mutex);
        while (dead != NULL) {
          EmeraldMessage *next = dead->next;
          free(dead);
          pthread_mutex_lock(&emerald_runnable_mutex);
          emerald_outstanding_messages--;
          if (emerald_outstanding_messages == 0) {
            pthread_cond_broadcast(&emerald_runnable_cond);
          }
          pthread_mutex_unlock(&emerald_runnable_mutex);
          dead = next;
        }
        break;
      }
      EmeraldMessage *msg = header->mailbox_head;
      if (msg == NULL) {
        header->scheduled = 0;
        pthread_mutex_unlock(&header->mailbox_mutex);
        break;
      }
      header->mailbox_head = msg->next;
      if (header->mailbox_head == NULL) {
        header->mailbox_tail = NULL;
      }
      pthread_mutex_unlock(&header->mailbox_mutex);

      msg->trampoline(header->self, msg->argv);
      free(msg);

      pthread_mutex_lock(&emerald_runnable_mutex);
      emerald_outstanding_messages--;
      if (emerald_outstanding_messages == 0) {
        pthread_cond_broadcast(&emerald_runnable_cond);
      }
      pthread_mutex_unlock(&emerald_runnable_mutex);
    }
  }
  return NULL;
}

/* Spawns `EMERALD_WORKERS` (when set and `> 0`) or
 * `sysconf(_SC_NPROCESSORS_ONLN)` worker threads. A no-op if already
 * started — generated `main` calls this exactly once, at its very
 * start, but this stays idempotent rather than relying on that being
 * the only caller forever. */
void emerald_worker_pool_start(void) {
  if (emerald_pool_started) {
    return;
  }
  int count = 0;
  const char *env = getenv("EMERALD_WORKERS");
  if (env != NULL) {
    long env_count = atol(env);
    if (env_count > 0) {
      count = (int) env_count;
    }
  }
  if (count <= 0) {
    long nproc = sysconf(_SC_NPROCESSORS_ONLN);
    count = nproc > 0 ? (int) nproc : 1;
  }
  emerald_worker_count = count;
  emerald_workers = malloc(sizeof(pthread_t) * (size_t) count);
  emerald_pool_shutdown = 0;
  for (int i = 0; i < count; i++) {
    pthread_create(&emerald_workers[i], NULL, emerald_worker_main, NULL);
  }
  emerald_pool_started = 1;
}

/* Generated `main`'s implicit barrier, emitted as the last thing before
 * its own `ret` (see the plan's own Decision log for why this is
 * compiler-inserted rather than a language-visible `await`): blocks
 * until every message ever enqueued has finished running, then signals
 * shutdown and joins every worker. Only `main`'s own generated code
 * ever calls this, after every top-level statement (and therefore every
 * send `main` will ever issue) has already run — see this file's own
 * `EmeraldActorHeader` doc comment for why a send racing this call is
 * not a scenario generated code can produce. */
void emerald_worker_pool_drain_and_join(void) {
  pthread_mutex_lock(&emerald_runnable_mutex);
  while (emerald_outstanding_messages > 0) {
    pthread_cond_wait(&emerald_runnable_cond, &emerald_runnable_mutex);
  }
  emerald_pool_shutdown = 1;
  pthread_cond_broadcast(&emerald_runnable_cond);
  pthread_mutex_unlock(&emerald_runnable_mutex);

  for (int i = 0; i < emerald_worker_count; i++) {
    pthread_join(emerald_workers[i], NULL);
  }
  free(emerald_workers);
  emerald_workers = NULL;
  emerald_worker_count = 0;
  emerald_pool_started = 0;
  emerald_pool_shutdown = 0;
}

/* Observability only (plan 55's `leaf-worked-concurrency-proof`'s own
 * best-effort, disclosed-probabilistic evidence) — never consulted by
 * any dispatch/safety logic above. */
long long emerald_current_thread_id(void) {
  return (long long) (intptr_t) pthread_self();
}

/* Plan 57 (supervision trees).
 *
 * Crash isolation reuses plan 38's own `setjmp`/`longjmp` handler
 * stack verbatim (see this file's own `EmeraldHandler` section above)
 * — codegen wraps every actor method's real call, inside its own
 * trampoline, in a `push_handler`/`setjmp` frame exactly shaped like
 * `build_begin`'s existing bare-`rescue`-equivalent codegen. On the
 * `setjmp`-returned-nonzero path (a `raise` reached this frame
 * uncaught), the trampoline calls `emerald_actor_terminate` — below —
 * instead of resuming normal dispatch.
 *
 * `Supervisor` is a real, disclosed architectural simplification of
 * this plan's own literal "compiled as an ordinary actor" framing: an
 * ordinary actor's own query (`child(name)`) would have to be
 * synchronous-with-a-return-value, a genuine "ask" pattern plan 55's
 * own Decision log explicitly declined to build. A plain, `pthread_
 * mutex_t`-guarded record achieves the identical *observable*
 * contract this plan's own acceptance criteria actually test (one_for_
 * one isolation, state-reset on restart, dead-reference sends silently
 * dropped) via directly-synchronizable reads/writes instead of
 * inventing a new blocking-round-trip messaging primitive this batch's
 * scheduler design doesn't otherwise need. */

#define EMERALD_SUPERVISOR_MAX_CHILDREN 16

typedef struct EmeraldSupervisedChild {
  /* `NULL` for a bare, unnamed `.spawn` inside the block — a real,
   * disclosed gap (Decision log): unreachable via `child(name)`. */
  char *name;
  /* For the `"restarting %s\n"` line only. */
  char *class_name;
  /* A per-supervised-class, codegen-generated "respawn thunk" —
   * `void *(*)(long long *argv)` — doing exactly what `Expr::Spawn`'s
   * own codegen already does (region_create, region_alloc, actor_
   * init_header, unpack argv, call initialize), just callable from
   * plain C with a raw argv array instead of an AST `Args` list. */
  void *(*respawn)(long long *argv);
  long long args[EMERALD_MESSAGE_ARGV_MAX];
  long long argc;
  /* The currently-live reference — read by `emerald_supervisor_child`,
   * replaced (never mutated through an outstanding pointer — a whole
   * new pointer value is written here) by `emerald_supervisor_notify_
   * terminated` on restart. */
  void *current_self;
} EmeraldSupervisedChild;

typedef struct EmeraldSupervisor {
  pthread_mutex_t mutex;
  EmeraldSupervisedChild children[EMERALD_SUPERVISOR_MAX_CHILDREN];
  long long child_count;
} EmeraldSupervisor;

/* Called once per `supervise do ... end` expression, before any of its
 * tracked `.spawn`s run. */
void *emerald_supervisor_create(void) {
  EmeraldSupervisor *sup = malloc(sizeof(EmeraldSupervisor));
  pthread_mutex_init(&sup->mutex, NULL);
  sup->child_count = 0;
  return sup;
}

/* Registers the next free slot for a just-spawned tracked child,
 * capturing its already-evaluated original spawn arguments (Decision
 * log: captured values, never re-evaluated expressions) and wiring
 * the child's own header back-pointer so a future crash can find this
 * supervisor again. Called once per tracked `.spawn`, in source order,
 * by `supervise`'s own generated body — same call site shape as an
 * ordinary `Expr::Spawn`, just followed by this one extra call.
 * Returns the assigned slot index (also stored in the child's own
 * header). */
long long emerald_supervisor_register_child(void *sup_ptr, const char *name,
                                             const char *class_name,
                                             void *(*respawn)(long long *),
                                             void *child_self, long long *args,
                                             long long argc) {
  EmeraldSupervisor *sup = (EmeraldSupervisor *) sup_ptr;
  long long slot = sup->child_count++;
  EmeraldSupervisedChild *child = &sup->children[slot];
  child->name = name != NULL ? strdup(name) : NULL;
  child->class_name = strdup(class_name);
  child->respawn = respawn;
  long long n = argc < EMERALD_MESSAGE_ARGV_MAX ? argc : EMERALD_MESSAGE_ARGV_MAX;
  for (long long i = 0; i < n; i++) {
    child->args[i] = args[i];
  }
  child->argc = argc;
  child->current_self = child_self;

  EmeraldActorHeader *header =
      *(EmeraldActorHeader **) ((char *) child_self - sizeof(void *));
  pthread_mutex_lock(&header->mailbox_mutex);
  header->supervisor = sup_ptr;
  header->child_slot = slot;
  pthread_mutex_unlock(&header->mailbox_mutex);

  return slot;
}

/* Called from `emerald_actor_terminate`, synchronously, on the crashed
 * actor's own worker thread (before that thread does anything else) —
 * re-`.spawn`s the failed child ALONE (`one_for_one`: no sibling is
 * touched) from its captured original arguments, replaces its record
 * entry with the fresh reference, wires the new instance's own
 * supervisor back-pointer (so IT can be supervised/restarted again in
 * turn), and logs the restart. Mutex-guarded so a concurrent `emerald_
 * supervisor_child` query never observes a half-updated record. */
void emerald_supervisor_notify_terminated(void *sup_ptr, long long slot) {
  EmeraldSupervisor *sup = (EmeraldSupervisor *) sup_ptr;
  pthread_mutex_lock(&sup->mutex);
  EmeraldSupervisedChild *child = &sup->children[slot];
  void *new_self = child->respawn(child->args);
  child->current_self = new_self;
  printf("restarting %s\n", child->class_name);
  pthread_mutex_unlock(&sup->mutex);

  EmeraldActorHeader *header =
      *(EmeraldActorHeader **) ((char *) new_self - sizeof(void *));
  pthread_mutex_lock(&header->mailbox_mutex);
  header->supervisor = sup_ptr;
  header->child_slot = slot;
  pthread_mutex_unlock(&header->mailbox_mutex);
}

/* A blocking query for `name`'s current live reference. Waits for the
 * WHOLE mailbox system to quiesce first (the same condition `emerald_
 * worker_pool_drain_and_join` waits on) — a real, disclosed
 * conservative simplification: this guarantees any restart already
 * triggered by an earlier send has genuinely finished before this
 * reads the record (Decision log: "blocks until any restart in flight
 * ... has completed"), at the real cost of also waiting on totally
 * unrelated in-flight traffic elsewhere in the program. Scoped to
 * being called from OUTSIDE any actor's own worker thread — this
 * plan's own real scope, matching its worked example's own usage
 * (every call is top-level, never from inside another actor's own
 * message handler, which could deadlock against its own pending
 * message under this design). */
void *emerald_supervisor_child(void *sup_ptr, const char *name) {
  EmeraldSupervisor *sup = (EmeraldSupervisor *) sup_ptr;

  pthread_mutex_lock(&emerald_runnable_mutex);
  while (emerald_outstanding_messages > 0) {
    pthread_cond_wait(&emerald_runnable_cond, &emerald_runnable_mutex);
  }
  pthread_mutex_unlock(&emerald_runnable_mutex);

  pthread_mutex_lock(&sup->mutex);
  void *result = NULL;
  for (long long i = 0; i < sup->child_count; i++) {
    if (sup->children[i].name != NULL && strcmp(sup->children[i].name, name) == 0) {
      result = sup->children[i].current_self;
      break;
    }
  }
  pthread_mutex_unlock(&sup->mutex);
  if (result == NULL) {
    fprintf(stderr, "uncaught error: supervisor has no child named '%s'\n", name);
    exit(1);
  }
  return result;
}

/* Called from a message trampoline's own synthetic catch handler
 * (codegen, `leaf-crash-isolation`) when an uncaught exception escapes
 * the real method call. Marks the actor terminated — under `mailbox_
 * mutex`, the same lock `emerald_actor_enqueue`/`emerald_worker_main`
 * already synchronize on for this exact flag — and, if supervised,
 * notifies the supervisor synchronously before returning.
 *
 * Real, disclosed non-goal found and accepted this session, NOT the
 * plan's own original design: this does NOT call `emerald_region_
 * destroy` on `header->region`, despite `region` being tracked for
 * exactly that purpose. The actor's own header-pointer slot (`self -
 * sizeof(void*)`, `EmeraldActorHeader`'s own doc comment) lives INSIDE
 * that same region — freeing the region would free the very memory
 * every future `self`-based header lookup (starting with `emerald_
 * actor_enqueue`'s own terminated-check on a *later, post-crash* send
 * to this same dead reference) depends on, corrupting exactly the
 * mechanism this leaf exists to make safe. Verified as a real, not
 * hypothetical, bug this session (a genuine `SIGSEGV` in `emerald_
 * actor_enqueue`, reproduced and root-caused via `gdb`, not merely
 * theorized). Actually fixing it needs the header-pointer slot moved
 * to its own allocation independent of the region — a real, disclosed,
 * larger redesign of plan 54/55's own established arena-layout
 * convention, deferred as future work; this leaf keeps the actor's
 * arena allocated-but-unused after termination, the same disclosed
 * "leaks by design" precedent `emerald_alloc`'s own no-corresponding-
 * free contract and plan 54's own untracked region handle already
 * established — a real cost, not a silent one. */
void emerald_actor_terminate(void *self) {
  EmeraldActorHeader *header = *(EmeraldActorHeader **) ((char *) self - sizeof(void *));

  pthread_mutex_lock(&header->mailbox_mutex);
  header->terminated = 1;
  void *supervisor = header->supervisor;
  long long slot = header->child_slot;
  pthread_mutex_unlock(&header->mailbox_mutex);

  if (supervisor != NULL) {
    emerald_supervisor_notify_terminated(supervisor, slot);
  }
}
