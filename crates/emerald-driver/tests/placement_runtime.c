/* Plan 65 (automatic actor placement), `leaf-virtual-actor-placement`'s
 * own consistent-hash proof, isolated from any codegen/LLVM machinery —
 * mirrors `discovery_runtime.c`'s own established pattern.
 *
 * argv[1] selects the mode:
 *   "owner"       - prints the owning peer index for argv[2] (a key)
 *                    over the 3-peer EMERALD_PEERS set below, all live.
 *   "stability"   - AC3: prints the owning peer index for "shard-7"
 *                    over the SAME 3 real peers, once with only those
 *                    3 in the set and once with a 4th, always-dead peer
 *                    appended — both must print the same index.
 *   "deterministic" - prints the owning index for a fixed key, called
 *                    twice in the same process — both must agree (and
 *                    a separate process invocation, compared by the
 *                    Rust harness, must agree too — the "every process
 *                    computes the identical ring independently" claim). */

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

extern int emerald_parse_host_port(const char *addr, uint32_t *ip_out, uint16_t *port_out);
extern int emerald_consistent_hash_owner(const char *key, const EmeraldPeerSet *peers,
                                          const int *live);

static void add_peer(EmeraldPeerSet *set, const char *addr) {
  uint32_t ip = 0;
  uint16_t port = 0;
  emerald_parse_host_port(addr, &ip, &port);
  set->peers[set->count].ip = ip;
  set->peers[set->count].port = port;
  strncpy(set->peers[set->count].addr, addr, sizeof(set->peers[set->count].addr) - 1);
  set->peers[set->count].addr[sizeof(set->peers[set->count].addr) - 1] = '\0';
  set->count++;
}

int main(int argc, char **argv) {
  if (argc < 2) {
    fprintf(stderr, "usage: placement_runtime <owner|stability|deterministic> [key]\n");
    return 2;
  }

  EmeraldPeerSet three;
  memset(&three, 0, sizeof(three));
  three.self_index = -1;
  add_peer(&three, "127.0.0.1:9001");
  add_peer(&three, "127.0.0.1:9002");
  add_peer(&three, "127.0.0.1:9003");

  if (strcmp(argv[1], "owner") == 0) {
    const char *key = argc > 2 ? argv[2] : "shard-7";
    printf("%d\n", emerald_consistent_hash_owner(key, &three, NULL));
    return 0;
  }

  if (strcmp(argv[1], "stability") == 0) {
    int owner3 = emerald_consistent_hash_owner("shard-7", &three, NULL);

    EmeraldPeerSet four = three;
    add_peer(&four, "127.0.0.1:9999"); /* never contacted, always dead */
    int live[4] = {1, 1, 1, 0};
    int owner4 = emerald_consistent_hash_owner("shard-7", &four, live);

    printf("%d\n", owner3);
    printf("%d\n", owner4);
    return 0;
  }

  if (strcmp(argv[1], "deterministic") == 0) {
    const char *key = argc > 2 ? argv[2] : "shard-7";
    printf("%d\n", emerald_consistent_hash_owner(key, &three, NULL));
    printf("%d\n", emerald_consistent_hash_owner(key, &three, NULL));
    return 0;
  }

  fprintf(stderr, "unknown mode `%s`\n", argv[1]);
  return 2;
}
