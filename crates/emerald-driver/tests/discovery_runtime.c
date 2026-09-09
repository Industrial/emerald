/* Plan 65 (automatic actor placement), `leaf-automatic-discovery`'s own
 * AC1/AC2/AC3: a standalone C harness — never linked into any shipped
 * Emerald program, mirroring `scheduler_runtime.c`'s own established
 * pattern — that exercises `runtime/emerald_runtime.c`'s real
 * `emerald_discover_peers` directly, with no codegen/LLVM involved at
 * all.
 *
 * argv[1] selects the mode:
 *   "basic"        - AC1: EMERALD_PEERS/EMERALD_SELF are read from this
 *                    process's own environment (set by the calling Rust
 *                    test). Prints `count`, `self_index`, then each
 *                    peer's `addr` on its own line.
 *   "malformed"     - AC2: expects `emerald_discover_peers` to fail;
 *                    prints "1" if it did (rc != 0), "0" otherwise.
 *   "missing_self"  - AC3: expects `emerald_discover_peers` to fail
 *                    because EMERALD_SELF is unset or doesn't match;
 *                    prints "1" if it did (rc != 0), "0" otherwise. */

#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define EMERALD_MAX_PEERS 32
#define EMERALD_HOST_PORT_MAX 64

typedef struct EmeraldPeer {
  uint32_t ip;
  uint16_t port;
  char addr[EMERALD_HOST_PORT_MAX];
} EmeraldPeer;

typedef struct EmeraldPeerSet {
  EmeraldPeer peers[EMERALD_MAX_PEERS];
  int count;
  int self_index;
} EmeraldPeerSet;

extern int emerald_discover_peers(EmeraldPeerSet *out);
extern const char *emerald_remote_last_error_message(void);

static void run_basic(void) {
  EmeraldPeerSet set;
  int rc = emerald_discover_peers(&set);
  if (rc != 0) {
    fprintf(stderr, "emerald_discover_peers failed: %s\n", emerald_remote_last_error_message());
    printf("-1\n");
    return;
  }
  printf("%d\n", set.count);
  printf("%d\n", set.self_index);
  for (int i = 0; i < set.count; i++) {
    printf("%s\n", set.peers[i].addr);
  }
}

static void run_expect_failure(void) {
  EmeraldPeerSet set;
  int rc = emerald_discover_peers(&set);
  printf("%d\n", rc != 0 ? 1 : 0);
  if (rc != 0) {
    fprintf(stderr, "expected failure, got: %s\n", emerald_remote_last_error_message());
  }
}

int main(int argc, char **argv) {
  if (argc < 2) {
    fprintf(stderr, "usage: discovery_runtime <basic|malformed|missing_self>\n");
    return 2;
  }
  if (strcmp(argv[1], "basic") == 0) {
    run_basic();
  } else if (strcmp(argv[1], "malformed") == 0) {
    run_expect_failure();
  } else if (strcmp(argv[1], "missing_self") == 0) {
    run_expect_failure();
  } else {
    fprintf(stderr, "unknown mode `%s`\n", argv[1]);
    return 2;
  }
  return 0;
}
