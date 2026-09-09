/* Plan 57 (supervision trees) — a standalone C harness, mirroring
 * `scheduler_runtime.c`'s own established pattern: exercises `runtime/
 * emerald_runtime.c`'s real crash-isolation and supervisor API
 * directly, with hand-built trampolines/respawn-thunks shaped exactly
 * the way codegen's own generated ones will be (`leaf-crash-
 * isolation`'s own synthetic `push_handler`/`setjmp` wrapper around
 * every actor method's real body) — proving the runtime contract is
 * sound before any compiler machinery consumes it.
 *
 * `setjmp` is redeclared here as a raw external function taking a
 * plain pointer, not included via `<setjmp.h>`'s own macro — this
 * mirrors EXACTLY what LLVM-generated code already does (plan 38's own
 * Decision log: `setjmp` is called directly by generated code, never
 * wrapped, and declared as an ordinary external function at the LLVM
 * IR level) — so this hand-written harness proves the identical ABI
 * contract codegen itself relies on, not a C-standard-library
 * convenience wrapper around it.
 *
 * argv[1] selects the mode:
 *   "crash_isolation" - AC1: an unsupervised actor's handler raises
 *                        uncaught; the process does not die, and a
 *                        second, unrelated actor keeps working.
 *   "concurrent_crash" - AC2: two actors, each forced onto its own
 *                        worker thread via EMERALD_WORKERS=2 and a
 *                        barrier, both raise at overlapping times;
 *                        each terminates independently with the
 *                        correct exception tag attributed to it.
 *   "supervisor"       - the full one_for_one worked-example proof:
 *                        crash, restart, state reset, sibling
 *                        isolation, dead-reference silent drop. */

#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

extern void *emerald_actor_init_header(void *arena_base);
extern void emerald_actor_set_region(void *self, void *region);
extern int emerald_actor_enqueue(void *self, void (*trampoline)(void *, long long *),
                                  long long *argv, long long argc);
extern void emerald_worker_pool_start(void);
extern void emerald_worker_pool_drain_and_join(void);

extern void *emerald_push_handler(void);
extern void *emerald_handler_jmpbuf(void *handler);
extern void emerald_pop_handler(void);
extern void emerald_free_handler(void *handler);
extern long long emerald_handler_tag(void *handler);
extern void emerald_raise(long long tag, void *exception_ptr);
extern int setjmp(void *env);

extern void *emerald_region_create(void);
extern void *emerald_region_alloc(void *region, long long size);
extern void emerald_actor_terminate(void *self);

extern void *emerald_supervisor_create(void);
extern long long emerald_supervisor_register_child(void *sup, const char *name,
                                                     const char *class_name,
                                                     void *(*respawn)(long long *),
                                                     void *child_self, long long *args,
                                                     long long argc);
extern void *emerald_supervisor_child(void *sup, const char *name);

/* Plan 60 (distributed, location-transparent actors) — Design decision
 * 1: an actor-typed VALUE is now a tagged `EmeraldActorRef*`, not a
 * bare arena pointer. `emerald_supervisor_register_child`/`emerald_
 * supervisor_notify_terminated` (the `respawn` callback's own return
 * value) both now unwrap `->local_arena` internally — this harness,
 * hand-mirroring what real codegen generates, must supply/consume that
 * same shape at the supervisor boundary. `emerald_actor_enqueue`
 * itself is UNCHANGED (still takes the raw arena pointer, always has)
 * — `crashy`/`survivor`/`a0`/`a1` above stay exactly as they were; only
 * a value that crosses the SUPERVISOR api needs this wrapping. Mirrors
 * only the leading fields actually read here — real, disclosed
 * duplication of the runtime's own struct layout (the same "this
 * harness hand-mirrors what codegen generates" precedent every other
 * function in this file already establishes), not the full struct
 * (its trailing `pthread_mutex_t` is never touched from here). */
struct EmeraldActorRefMirror {
  unsigned char is_remote;
  void *local_arena;
};
extern void *emerald_actor_ref_local(void *self);

/* --- shared helpers --- */

static void *new_actor(long long extra_bytes) {
  void *region = emerald_region_create();
  void *arena = emerald_region_alloc(region, (long long) sizeof(void *) + extra_bytes);
  emerald_actor_init_header(arena);
  void *self = (char *) arena + sizeof(void *);
  emerald_actor_set_region(self, region);
  return self;
}

/* --- "crash_isolation" mode (AC1) --- */

static void crashy_body(void *self, long long *argv) {
  (void) self;
  (void) argv;
  emerald_raise(1, NULL);
}

static void crashy_trampoline(void *self, long long *argv) {
  void *h = emerald_push_handler();
  void *jb = emerald_handler_jmpbuf(h);
  if (setjmp(jb) == 0) {
    crashy_body(self, argv);
    emerald_pop_handler();
  } else {
    emerald_free_handler(h);
    emerald_actor_terminate(self);
  }
}

static void survivor_body(void *self, long long *argv) {
  (void) self;
  (void) argv;
  printf("survivor ok\n");
}

static void survivor_trampoline(void *self, long long *argv) {
  void *h = emerald_push_handler();
  void *jb = emerald_handler_jmpbuf(h);
  if (setjmp(jb) == 0) {
    survivor_body(self, argv);
    emerald_pop_handler();
  } else {
    emerald_free_handler(h);
    emerald_actor_terminate(self);
  }
}

static void run_crash_isolation_mode(void) {
  void *crashy = new_actor(0);
  void *survivor = new_actor(0);

  emerald_worker_pool_start();

  long long argv[1] = {0};
  emerald_actor_enqueue(crashy, crashy_trampoline, argv, 0);
  emerald_actor_enqueue(survivor, survivor_trampoline, argv, 0);

  emerald_worker_pool_drain_and_join();
  /* Reaching here at all — instead of the process having called
   * `exit(1)` from `emerald_raise`'s own `h == NULL` path — is itself
   * half the proof; `survivor_trampoline`'s own real "survivor ok\n"
   * line is the other half. */
  printf("process survived\n");
}

/* --- "concurrent_crash" mode (AC2) --- */

static pthread_barrier_t concurrent_crash_barrier;
static long long concurrent_crash_tags[2];

static void tagged_crash_body(void *self, long long *argv) {
  (void) self;
  long long index = argv[0];
  long long tag = argv[1];
  pthread_barrier_wait(&concurrent_crash_barrier);
  emerald_raise(tag, NULL);
  (void) index;
}

static void tagged_crash_trampoline(void *self, long long *argv) {
  void *h = emerald_push_handler();
  void *jb = emerald_handler_jmpbuf(h);
  if (setjmp(jb) == 0) {
    tagged_crash_body(self, argv);
    emerald_pop_handler();
  } else {
    long long index = argv[0];
    concurrent_crash_tags[index] = emerald_handler_tag(h);
    emerald_free_handler(h);
    emerald_actor_terminate(self);
  }
}

static void run_concurrent_crash_mode(void) {
  pthread_barrier_init(&concurrent_crash_barrier, NULL, 2);
  void *a0 = new_actor(0);
  void *a1 = new_actor(0);

  emerald_worker_pool_start();

  long long argv0[2] = {0, 101};
  long long argv1[2] = {1, 202};
  emerald_actor_enqueue(a0, tagged_crash_trampoline, argv0, 2);
  emerald_actor_enqueue(a1, tagged_crash_trampoline, argv1, 2);

  emerald_worker_pool_drain_and_join();
  pthread_barrier_destroy(&concurrent_crash_barrier);
  printf("%lld\n", concurrent_crash_tags[0]);
  printf("%lld\n", concurrent_crash_tags[1]);
}

/* --- "supervisor" mode --- */

/* `Worker`: one Int64 field (`count`). Crashes when `count` reaches 3. */
static void *worker_respawn(long long *args) {
  void *self = new_actor(8);
  *(long long *) self = args[0];
  return emerald_actor_ref_local(self);
}

static void worker_handle_body(void *self, long long *argv) {
  long long *count = (long long *) self;
  *count += argv[0];
  if (*count == 3) {
    emerald_raise(7, NULL);
  }
  printf("%lld\n", *count);
}

static void worker_handle_trampoline(void *self, long long *argv) {
  void *h = emerald_push_handler();
  void *jb = emerald_handler_jmpbuf(h);
  if (setjmp(jb) == 0) {
    worker_handle_body(self, argv);
    emerald_pop_handler();
  } else {
    emerald_free_handler(h);
    emerald_actor_terminate(self);
  }
}

/* `Logger`: no fields, always prints "log". Never crashes. */
static void *logger_respawn(long long *args) {
  (void) args;
  return emerald_actor_ref_local(new_actor(0));
}

static void logger_handle_body(void *self, long long *argv) {
  (void) self;
  (void) argv;
  printf("log\n");
}

static void logger_handle_trampoline(void *self, long long *argv) {
  void *h = emerald_push_handler();
  void *jb = emerald_handler_jmpbuf(h);
  if (setjmp(jb) == 0) {
    logger_handle_body(self, argv);
    emerald_pop_handler();
  } else {
    emerald_free_handler(h);
    emerald_actor_terminate(self);
  }
}

static void run_supervisor_mode(void) {
  emerald_worker_pool_start();

  void *sup = emerald_supervisor_create();

  long long worker_args[1] = {0};
  void *worker = worker_respawn(worker_args);
  emerald_supervisor_register_child(sup, "worker", "Worker", worker_respawn, worker,
                                     worker_args, 1);

  long long logger_args[1] = {0};
  void *logger = logger_respawn(logger_args);
  emerald_supervisor_register_child(sup, "logger", "Logger", logger_respawn, logger,
                                     logger_args, 1);

  /* `emerald_supervisor_child` returns exactly what was registered —
   * now a ref (plan 60) — but `emerald_actor_enqueue` itself still
   * takes the raw arena pointer (unchanged): unwrap `->local_arena` at
   * each call site below, the identical unwrap real codegen's own
   * `emerald_actor_dispatch` now does internally on the local path. */
  void *w = emerald_supervisor_child(sup, "worker");
  void *l = emerald_supervisor_child(sup, "logger");
  void *w_arena = ((struct EmeraldActorRefMirror *) w)->local_arena;
  void *l_arena = ((struct EmeraldActorRefMirror *) l)->local_arena;

  long long one[1] = {1};
  emerald_actor_enqueue(w_arena, worker_handle_trampoline, one, 1); /* 1 */
  emerald_actor_enqueue(w_arena, worker_handle_trampoline, one, 1); /* 2 */
  emerald_actor_enqueue(l_arena, logger_handle_trampoline, one, 1); /* log */
  emerald_actor_enqueue(w_arena, worker_handle_trampoline, one, 1); /* crashes; restarts */

  void *w2 = emerald_supervisor_child(sup, "worker"); /* blocks until restart done */
  void *w2_arena = ((struct EmeraldActorRefMirror *) w2)->local_arena;
  emerald_actor_enqueue(l_arena, logger_handle_trampoline, one, 1); /* log */
  emerald_actor_enqueue(w2_arena, worker_handle_trampoline, one, 1); /* 1, not 4 */

  emerald_worker_pool_drain_and_join();

  /* AC4: `w` (the pre-crash, now-dead reference) used again — must be
   * silently dropped, not crash the process and not reach `w2`. */
  emerald_worker_pool_start();
  emerald_actor_enqueue(w_arena, worker_handle_trampoline, one, 1);
  emerald_worker_pool_drain_and_join();
  printf("dead send did not crash\n");
}

int main(int argc, char **argv) {
  if (argc < 2) {
    fprintf(stderr, "usage: %s [crash_isolation|concurrent_crash|supervisor]\n", argv[0]);
    return 2;
  }
  if (strcmp(argv[1], "crash_isolation") == 0) {
    run_crash_isolation_mode();
  } else if (strcmp(argv[1], "concurrent_crash") == 0) {
    run_concurrent_crash_mode();
  } else if (strcmp(argv[1], "supervisor") == 0) {
    run_supervisor_mode();
  } else {
    fprintf(stderr, "unknown mode: %s\n", argv[1]);
    return 2;
  }
  return 0;
}
