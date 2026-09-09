/* Plan 55 (scheduler and message passing), `leaf-thread-safe-runtime`'s
 * own AC2/AC3/AC4: a standalone C harness — never linked into any
 * shipped Emerald program, mirroring `region_arena.c`'s own established
 * pattern — that exercises `runtime/emerald_runtime.c`'s real mailbox/
 * worker-pool API directly (`emerald_actor_init_header`,
 * `emerald_actor_enqueue`, `emerald_worker_pool_start`/`_drain_and_join`,
 * `emerald_current_thread_id`), with no codegen/LLVM involved at all —
 * this leaf's own scope is the runtime primitive, proven before any
 * compiler machinery consumes it.
 *
 * Every "actor" here is a bare `malloc`d arena: 8 bytes reserved for
 * the header-pointer slot (this runtime's own real layout contract,
 * see `emerald_runtime.c`'s own `EmeraldActorHeader` doc comment),
 * followed by whatever plain fields this harness's own trampolines
 * want to read/write — never a real Emerald object, no codegen-emitted
 * layout involved.
 *
 * argv[1] selects the mode:
 *   "pool"     - AC2: 50 no-op messages spread round-robin across 5
 *                actor arenas, `EMERALD_WORKERS=4` (set by the calling
 *                Rust test), drained; prints the summed per-actor
 *                counters (must be exactly 50 — every message's
 *                trampoline ran exactly once).
 *   "fifo"     - AC3: two messages enqueued back-to-back onto the SAME
 *                actor's mailbox from this harness's own main thread;
 *                the second message's trampoline checks the first
 *                already ran before it did; prints 1 (ok) or 0.
 *   "threadid" - AC4: two actors, one message each, both trampolines
 *                block on a 2-party barrier after recording their own
 *                `emerald_current_thread_id()` — requires
 *                `EMERALD_WORKERS=2` (set by the calling Rust test) so
 *                both can genuinely be running at once; prints the two
 *                recorded thread ids, one per line. */

#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern void *emerald_actor_init_header(void *arena_base);
extern int emerald_actor_enqueue(void *self, void (*trampoline)(void *, long long *),
                                  long long *argv, long long argc);
extern void emerald_worker_pool_start(void);
extern void emerald_worker_pool_drain_and_join(void);
extern long long emerald_current_thread_id(void);
extern void emerald_actor_terminate(void *self);

static void *actor_self(void *arena) {
  return (char *) arena + sizeof(void *);
}

static void *new_actor_arena(size_t extra_bytes) {
  void *arena = malloc(sizeof(void *) + extra_bytes);
  emerald_actor_init_header(arena);
  return arena;
}

/* --- "pool" mode (AC2) --- */

static void pool_trampoline(void *self, long long *argv) {
  (void) argv;
  long long *counter = (long long *) self;
  *counter += 1;
}

#define NUM_ACTORS 5
#define NUM_MESSAGES 50

static void run_pool_mode(void) {
  void *arenas[NUM_ACTORS];
  for (int i = 0; i < NUM_ACTORS; i++) {
    arenas[i] = new_actor_arena(sizeof(long long));
    *(long long *) actor_self(arenas[i]) = 0;
  }

  emerald_worker_pool_start();

  long long argv[1] = {0};
  for (int i = 0; i < NUM_MESSAGES; i++) {
    void *self = actor_self(arenas[i % NUM_ACTORS]);
    emerald_actor_enqueue(self, pool_trampoline, argv, 0);
  }

  emerald_worker_pool_drain_and_join();

  long long total = 0;
  for (int i = 0; i < NUM_ACTORS; i++) {
    total += *(long long *) actor_self(arenas[i]);
  }
  printf("%lld\n", total);
}

/* --- "fifo" mode (AC3) --- */

static int fifo_ok = 0;

static void fifo_first_trampoline(void *self, long long *argv) {
  (void) argv;
  *(long long *) self = 1;
}

static void fifo_second_trampoline(void *self, long long *argv) {
  (void) argv;
  fifo_ok = (*(long long *) self == 1) ? 1 : 0;
  *(long long *) self = 2;
}

static void run_fifo_mode(void) {
  void *arena = new_actor_arena(sizeof(long long));
  *(long long *) actor_self(arena) = 0;

  emerald_worker_pool_start();

  long long argv[1] = {0};
  void *self = actor_self(arena);
  emerald_actor_enqueue(self, fifo_first_trampoline, argv, 0);
  emerald_actor_enqueue(self, fifo_second_trampoline, argv, 0);

  emerald_worker_pool_drain_and_join();
  printf("%d\n", fifo_ok);
}

/* --- "threadid" mode (AC4) --- */

static long long thread_ids[2];
static pthread_barrier_t threadid_barrier;

static void threadid_trampoline(void *self, long long *argv) {
  (void) self;
  long long index = argv[0];
  thread_ids[index] = emerald_current_thread_id();
  pthread_barrier_wait(&threadid_barrier);
}

static void run_threadid_mode(void) {
  pthread_barrier_init(&threadid_barrier, NULL, 2);
  void *arena0 = new_actor_arena(0);
  void *arena1 = new_actor_arena(0);

  emerald_worker_pool_start();

  long long argv0[1] = {0};
  long long argv1[1] = {1};
  emerald_actor_enqueue(actor_self(arena0), threadid_trampoline, argv0, 1);
  emerald_actor_enqueue(actor_self(arena1), threadid_trampoline, argv1, 1);

  emerald_worker_pool_drain_and_join();
  pthread_barrier_destroy(&threadid_barrier);
  printf("%lld\n", thread_ids[0]);
  printf("%lld\n", thread_ids[1]);
}

/* Plan 65 (automatic actor placement), `leaf-unified-fallible-send`'s
 * own AC3, proven directly at the runtime layer (mirroring this
 * file's own established "pool"/"fifo"/"threadid" modes): a message
 * enqueued against an actor already marked terminated (via a real
 * `emerald_actor_terminate` call, the identical mechanism a crashed
 * message trampoline itself uses) must return -1, not silently
 * succeed — the "hardcoded `return 0`" gap `emerald_actor_dispatch`'s
 * own local branch used to have, closed by `emerald_actor_enqueue`'s
 * own widened `int` return (`leaf-unified-fallible-send`'s Decision
 * log). Prints 1 if the post-termination enqueue correctly reported
 * failure, 0 otherwise. */
static void noop_trampoline(void *self, long long *argv) {
  (void) self;
  (void) argv;
}

static void run_terminated_mode(void) {
  void *arena = new_actor_arena(0);
  void *self = actor_self(arena);
  long long argv[1] = {0};
  emerald_actor_terminate(self);
  int rc = emerald_actor_enqueue(self, noop_trampoline, argv, 0);
  printf("%d\n", rc != 0 ? 1 : 0);
}

int main(int argc, char **argv) {
  if (argc < 2) {
    fprintf(stderr, "usage: %s [pool|fifo|threadid|terminated]\n", argv[0]);
    return 2;
  }
  if (strcmp(argv[1], "pool") == 0) {
    run_pool_mode();
  } else if (strcmp(argv[1], "fifo") == 0) {
    run_fifo_mode();
  } else if (strcmp(argv[1], "threadid") == 0) {
    run_threadid_mode();
  } else if (strcmp(argv[1], "terminated") == 0) {
    run_terminated_mode();
  } else {
    fprintf(stderr, "unknown mode: %s\n", argv[1]);
    return 2;
  }
  return 0;
}
