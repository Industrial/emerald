/* Plan 51 (scope-based arena allocation), `leaf-bounded-memory-proof`
 * (adapted, see the commit message / test-runner comments for why):
 * a standalone C harness — never linked into any shipped Emerald
 * program — that exercises `runtime/emerald_runtime.c`'s
 * `emerald_region_create`/`_alloc`/`_destroy` and
 * `emerald_bytes_outstanding` directly, simulating the call-frame
 * pattern a future codegen consumer would generate: a "function"
 * called many times, each call allocating a couple of small,
 * non-escaping objects (mirroring this plan's own worked
 * `compute_local_sum` example: two 16-byte `Point`s per call) and
 * discarding them.
 *
 * argv[1] selects the mode:
 *   "region"      - each simulated call creates its own region,
 *                    allocates through it, destroys it before the
 *                    next call — leaf-runtime-region-arena's actual
 *                    bulk-free path.
 *   "region_leak" - one region, forced to grow past its first chunk,
 *                    then destroyed — leaf-runtime-region-arena's
 *                    own AC2 (valgrind leak-check target) and AC3
 *                    (counter round-trips to its pre-allocation value)
 *                    in one pass.
 *   "alloc"       - the explicit contrast: the same per-call
 *                    allocation pattern routed through plain
 *                    `emerald_alloc` with no region and no free at
 *                    all — what today's world does, and what this
 *                    plan's headline claim is contrasted against.
 *
 * Every mode prints one `emerald_bytes_outstanding()` sample per line,
 * taken at fixed intervals, to stdout — the calling Rust test parses
 * these lines and asserts on the shape (bounded vs. growing). */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern long long emerald_bytes_outstanding(void);
extern void *emerald_alloc(long long size);
extern void *emerald_region_create(void);
extern void *emerald_region_alloc(void *region, long long size);
extern void emerald_region_destroy(void *region);

#define ITERATIONS 1000
#define SAMPLE_EVERY 100
#define POINT_BYTES 16

static void run_region_mode(void) {
  for (long long i = 0; i < ITERATIONS; i++) {
    void *region = emerald_region_create();
    void *p1 = emerald_region_alloc(region, POINT_BYTES);
    void *p2 = emerald_region_alloc(region, POINT_BYTES);
    /* Touch the memory, the same way generated code would write
     * `@x`/`@y` during `initialize` — proves these aren't just
     * reserved-but-unused bytes. */
    memset(p1, 0, POINT_BYTES);
    memset(p2, 0, POINT_BYTES);
    emerald_region_destroy(region);
    if ((i + 1) % SAMPLE_EVERY == 0) {
      printf("%lld\n", emerald_bytes_outstanding());
    }
  }
}

static void run_alloc_mode(void) {
  for (long long i = 0; i < ITERATIONS; i++) {
    void *p1 = emerald_alloc(POINT_BYTES);
    void *p2 = emerald_alloc(POINT_BYTES);
    memset(p1, 0, POINT_BYTES);
    memset(p2, 0, POINT_BYTES);
    if ((i + 1) % SAMPLE_EVERY == 0) {
      printf("%lld\n", emerald_bytes_outstanding());
    }
  }
}

static void run_region_leak_mode(void) {
  long long before = emerald_bytes_outstanding();
  printf("%lld\n", before);

  void *region = emerald_region_create();
  /* Force at least one chunk growth: the default chunk is 4096 bytes,
   * so allocate well past it. */
  long long total_requested = 0;
  for (int i = 0; i < 2000; i++) {
    void *p = emerald_region_alloc(region, POINT_BYTES);
    memset(p, 0, POINT_BYTES);
    total_requested += POINT_BYTES;
  }
  long long after_alloc = emerald_bytes_outstanding();
  printf("%lld\n", after_alloc);
  printf("%lld\n", total_requested);

  emerald_region_destroy(region);
  long long after_destroy = emerald_bytes_outstanding();
  printf("%lld\n", after_destroy);
}

int main(int argc, char **argv) {
  if (argc < 2) {
    fprintf(stderr, "usage: %s [region|alloc|region_leak]\n", argv[0]);
    return 2;
  }
  if (strcmp(argv[1], "region") == 0) {
    run_region_mode();
  } else if (strcmp(argv[1], "alloc") == 0) {
    run_alloc_mode();
  } else if (strcmp(argv[1], "region_leak") == 0) {
    run_region_leak_mode();
  } else {
    fprintf(stderr, "unknown mode: %s\n", argv[1]);
    return 2;
  }
  return 0;
}
