2026-09-21T21:37:00Z

---
name: Graph Data Structures & Algorithms — `petgraph`
overview: "A new `emerald-rt` module wrapping `petgraph` 0.8.x's `Graph<N, E, Ty>` (directed and undirected, both monomorphized at construction time via a `directed: Boolean` constructor flag) as a `Graph` resource type, exposing node/edge construction plus the real algorithms `petgraph::algo` ships — Dijkstra shortest path, topological sort, and connected/strongly-connected components — verified against the crate's own current module listing rather than assumed from general graph-library familiarity."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-scaffold-graph-module-and-resource
    content: "Create `crates/emerald-rt/src/graph.rs`. Register a compiler-provided, non-user-declarable `Graph` class (one hidden `u64` handle field, per plan 93, into a `HashMap<u64, GraphKind>` where `GraphKind` is a small enum over `petgraph::graph::DiGraph<i64, f64>` and `petgraph::graph::UnGraph<i64, f64>` — see Decision log for why node/edge weights are fixed to `Int64`/`Float64` rather than generic in v1). Every exported `emerald_rt_graph_*` function goes through plan 92's `emerald_rt_fn!` catch-unwind macro; an out-of-range node index (a real `petgraph` panic path — indexing a `Graph` with a stale or foreign `NodeIndex` panics rather than returning an `Option`, verified against the crate's own documented indexing behavior) is caught there and surfaces as `NativeError`, not a process abort."
    status: pending
  - id: leaf-construction-and-mutation
    content: "`Graph.new(directed: Boolean): Graph` (`DiGraph::new()` or `UnGraph::new()` depending on the flag — the graph's directedness is fixed at construction and cannot change later, matching `petgraph`'s own `Ty: EdgeType` type-parameter design, collapsed here to a runtime flag since Emerald has no compile-time generic-over-a-marker-type mechanism to mirror `petgraph`'s own `Directed`/`Undirected` phantom types). `.add_node(self, weight: Int64): Int64` (`.add_node(weight)`, returning the new `NodeIndex`'s raw `.index()` as a plain `Int64` — `petgraph`'s own `NodeIndex` is not exposed as a distinct Emerald type in v1; see Decision log). `.add_edge(self, from: Int64, to: Int64, weight: Float64): Void` (`.add_edge(NodeIndex::new(from as usize), NodeIndex::new(to as usize), weight)`). `.node_count(self): Int64`, `.edge_count(self): Int64`."
    status: pending
  - id: leaf-shortest-path-and-toposort
    content: "`Graph.shortest_path(self, from: Int64, to: Int64): Option[Float64]` (`petgraph::algo::dijkstra(&graph, NodeIndex::new(from), Some(NodeIndex::new(to)), |e| *e.weight())`, reading the resulting `HashMap<NodeIndex, f64>` for the `to` entry — `Some(total_weight)` if reachable, `None` if not; `petgraph::algo::dijkstra` requires non-negative edge weights, per its own documented contract — this plan does not validate that at the API boundary, see Decision log). `Graph.topological_sort(self): Result[Array[Int64], String]` (`petgraph::algo::toposort(&graph, None)` — `Ok` wraps the sorted node-index list as a plain `Array[Int64]`; `Err` wraps `petgraph`'s own real `Cycle` error, converted to a message string, since a cyclic graph is a routine, expected failure a topological sort can genuinely encounter, not a bug — the same expected-vs-`NativeError` split every domain plan in this batch already draws). `Graph.is_directed(self): Boolean`."
    status: pending
  - id: leaf-connected-components
    content: "`Graph.connected_components(self): Int64` (`petgraph::algo::connected_components(&graph)` — verified against `petgraph`'s own published module listing this session: this function computes weakly-connected components and works on both directed and undirected graphs, treating edge direction as irrelevant, per the crate's own documented behavior; a caller wanting strongly-connected components on a directed graph calls the separate function below instead). `Graph.strongly_connected_components(self): Array[Array[Int64]]` (`petgraph::algo::tarjan_scc(&graph)` — Tarjan's algorithm, one of `petgraph::algo`'s two real SCC implementations alongside `kosaraju_scc`; `tarjan_scc` is chosen here as the single exposed SCC entry point for v1 specifically to avoid presenting two algorithmically-equivalent-but-differently-ordered functions as if they were meaningfully different Emerald-facing capabilities — see Decision log)."
    status: pending
  - id: leaf-example-and-gate
    content: "Add `examples/graph_algorithms.em` (the Concrete Proof below) to `examples/`, wired into `emerald-cli/tests/examples.rs`'s CI-checked table per plan 95's checklist. Add `#[test]`s in `emerald-rt` covering: a directed graph with a known shortest path (hand-computed expected weight), a topological sort on a real DAG matching a valid ordering, a topological sort on a deliberately cyclic graph producing `Err`, and a disconnected undirected graph reporting the correct `connected_components` count (2 or more, not silently 1). Run the full `AGENTS.md` gate (`cargo nextest run --workspace`, `cargo clippy --workspace --all-targets`, `treefmt`) plus a clean-checkout end-to-end build."
    status: pending
isProject: false
---

# Plan 188 — Graph Data Structures & Algorithms

Graphs and the handful of algorithms that operate on them — shortest
path, cycle detection via topological sort, connected components — are
common enough building blocks (dependency resolution, routing,
clustering, reachability analysis) that hand-rolling Dijkstra's
algorithm correctly inside application code every time it's needed is
real, avoidable, error-prone repetition. `petgraph` is the standard
Rust graph library for exactly this: real node/edge storage plus a
substantial, well-tested `algo` module. This plan wraps a fixed,
practical subset of both, verified against `petgraph`'s own current
published API rather than assumed from general familiarity with what a
graph library "probably" ships.

Depends on: plan 91 (`emerald-rt` archive), plan 92 (`catch_unwind`
convention, `NativeError`, `Result[T,E]` helpers), plan 93 (opaque-
handle resource model for `Graph`), and plan 95 (crate-vetting policy
and checklist). This plan needs no HTTP, crypto, or randomness
dependency from elsewhere in the batch — like plan 96's raw sockets, it
is a self-contained wrapper over one crate's own in-memory data
structure and algorithms, with no cross-plan runtime dependency beyond
the shared `emerald-rt` scaffolding itself.

## Concrete proof this plan targets

```ruby
g: Graph = Graph.new(true)

a: Int64 = g.add_node(1)
b: Int64 = g.add_node(2)
c: Int64 = g.add_node(3)
d: Int64 = g.add_node(4)

g.add_edge(a, b, 1.0)
g.add_edge(b, c, 2.0)
g.add_edge(a, c, 5.0)
g.add_edge(c, d, 1.0)

case g.shortest_path(a, d)
when Some(weight)
  puts weight
when None
  puts "unreachable"
end

case g.topological_sort()
when Ok(order)
  puts order.length
when Err(msg)
  puts msg
end

g.add_edge(d, a, 1.0)
case g.topological_sort()
when Ok(order)
  puts order.length
when Err(msg)
  puts msg
end
```

Expected output: `4.0` (the real shortest `a`→`d` path is `a`→`b`→`c`→`d`
at weight `1.0 + 2.0 + 1.0 = 4.0`, strictly less than the direct `a`→`c`
edge's own `5.0` plus `c`→`d`'s `1.0` = `6.0`, a genuine Dijkstra
result, not a first-path-found greedy answer); `4` (the graph is a DAG
before the last edge is added, so topological sort succeeds over all
four nodes); then `petgraph`'s own real cycle-detection error message
(once the `d`→`a` edge closes a cycle, the identical graph's
topological sort now genuinely fails) — a real, working proof that
adding one edge changes the graph's actual cyclicity, not a
pre-scripted pair of outputs.

## Decision log

- **`petgraph`'s real algorithm surface, verified against its own
  current source and docs this session — not assumed.** `petgraph`
  0.8.3 (published 2 August 2026, MIT/Apache-2.0 dual-licensed,
  maintained by `bluss` and the `petgraph` release team)'s `algo` module
  is confirmed, this session, to ship (among others) `dijkstra`,
  `bellman_ford`, `astar`, `toposort`, `is_cyclic_directed`,
  `is_cyclic_undirected`, `connected_components`, `kosaraju_scc`,
  `tarjan_scc`, `min_spanning_tree`, and `all_simple_paths` — this
  plan's own `leaf-shortest-path-and-toposort`/`leaf-connected-
  components` leaves wrap exactly the subset the task's own worked
  proof needs (shortest path, topological sort, connected components),
  not the full list; the remainder (`astar`, `bellman_ford`,
  `min_spanning_tree`, `all_simple_paths`) is real, available,
  unexposed surface named explicitly in Out of scope rather than
  silently omitted.
- **Node and edge weights are fixed to `Int64`/`Float64`, not generic —
  a real, deliberate simplification of `petgraph::Graph<N, E, Ty>`'s
  own two free type parameters.** `petgraph`'s real `Graph` is generic
  over arbitrary node-weight and edge-weight types; this plan's v1
  `Graph` is not — every node carries a fixed `Int64` payload and every
  edge a fixed `Float64` weight (chosen specifically because Dijkstra
  and the other numeric algorithms this plan exposes need a real,
  orderable, summable edge-weight type, and `Float64` is the one
  Emerald already has that fits). A caller wanting a richer node
  payload (a string label, a whole record) stores it in a side
  `Array`/`Hash` keyed by the `Int64` node index this plan returns from
  `.add_node`, rather than this plan attempting to generalize over
  Emerald's own type system the way `petgraph`'s Rust generics do —
  Emerald's monomorphized-generics story (plan 41/58) could in
  principle support a truly generic `Graph[N, E]` in a future
  iteration, but that is real, additional design work this plan's v1
  does not attempt.
- **`NodeIndex` is a bare `Int64` at the Emerald boundary, not a
  distinct opaque type — a real, disclosed loss of `petgraph`'s own
  type-level safety.** `petgraph`'s real `NodeIndex<Ix>` is a distinct
  newtype specifically so a node index from one graph can't be silently
  used against a different graph by accident; collapsing it to a plain
  `Int64` at the FFI boundary (the simplest, lowest-friction choice
  given Emerald's existing `Int64`-only numeric type system, per plan
  59's own verified finding that no narrower/distinct integer type
  exists) reintroduces exactly that class of possible misuse — an
  Emerald caller could pass a `NodeIndex` `Int64` from one `Graph`
  handle into a method call on a different `Graph` handle, and this
  plan's Rust-side wrapper would need to bounds-check it defensively
  (returning `NativeError` on an out-of-range index, per `leaf-scaffold-
  graph-module-and-resource`) rather than relying on Rust's own type
  system to make the mistake unrepresentable. Stated explicitly as a
  real tradeoff, not hidden behind the wrapper's clean method names.
- **`connected_components` and `strongly_connected_components` are two
  separate functions, not one function with a mode flag — because they
  compute genuinely different things, verified against `petgraph`'s own
  documented behavior.** `petgraph::algo::connected_components` computes
  weakly-connected components (ignoring edge direction entirely, valid
  on both directed and undirected graphs); `tarjan_scc` computes
  strongly-connected components (respecting edge direction — two nodes
  are in the same SCC only if each is reachable from the other via
  directed edges). These are not the same computation with a toggle;
  presenting them as one function with a `Boolean strict` parameter
  would obscure a real graph-theory distinction this plan's own worked
  proof and documentation should instead name plainly.
- **`tarjan_scc`, not `kosaraju_scc`, is v1's one exposed SCC
  algorithm — both exist in `petgraph::algo`, this plan picks one
  rather than exposing an arbitrary choice-of-algorithm parameter.**
  Both are real, correct, differently-ordered SCC implementations
  `petgraph` ships; from an Emerald caller's perspective the two differ
  only in the internal order components are returned in, not in
  correctness or in the asymptotic complexity class a caller would
  reasonably choose between. Exposing both as if the choice mattered to
  an Emerald program would be surface without a real distinction behind
  it; `tarjan_scc` is picked as the single-pass, no-second-graph-
  traversal-needed option.
- **Negative edge weights are not validated or rejected at the API
  boundary — `petgraph::algo::dijkstra`'s own documented contract is
  inherited as-is, not re-enforced.** Dijkstra's algorithm is only
  correct for non-negative edge weights; `petgraph`'s own `dijkstra`
  function does not itself check this and will silently produce a
  wrong (not panicking, not erroring) result on a graph with a negative
  edge, exactly as the textbook algorithm's own known limitation
  describes. This plan does not add a validation pass over edge weights
  before calling `dijkstra` — doing so correctly and efficiently (an
  extra full edge-list scan on every `.shortest_path` call) is real
  additional cost for a misuse case `petgraph`'s own real Rust API
  already accepts without complaint; a caller needing negative-weight
  shortest paths should use `petgraph::algo::bellman_ford` instead,
  which this plan does not currently expose (see Out of scope) but
  which is available for later exposure without redesigning this
  plan's `Graph` resource shape.
- **Out of scope.** A* search (`petgraph::algo::astar`, real, available,
  requires an admissible-heuristic closure this plan's fixed method
  signatures have no natural way to accept from Emerald source without
  a first-class closure/lambda-as-argument mechanism this batch does
  not assume exists for `emerald-rt` FFI boundaries). Bellman-Ford
  (negative-weight-tolerant shortest path) and minimum spanning tree —
  both real, both named above, deferred to a follow-up once a concrete
  caller need is identified. Graph serialization/deserialization
  (`petgraph`'s own optional `serde` feature, or DOT-format export via
  its `dot` module) — a real, separate, bounded feature this plan does
  not attempt. Mutating a graph after algorithms have run against it in
  ways that would invalidate previously-returned `NodeIndex` values
  (`petgraph`'s own `remove_node` reuses freed indices) — this plan
  exposes no node/edge removal at all in v1, sidestepping that whole
  class of stale-index hazard entirely rather than half-solving it.
