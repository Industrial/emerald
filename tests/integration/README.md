# tests/integration/

Cargo discovers integration tests per-crate (`crates/<name>/tests/`), not
from a shared root directory — a root-level test here would not run under
`cargo test` at all. This directory is a documentation/organization
pointer, per plan `03`'s workspace layout, not a Cargo test target.

The end-to-end milestone-1 test (parse → type-check → codegen → link →
run, asserting `examples/hello.em` prints `42`) lives at
[`crates/emerald-cli/tests/hello_em.rs`](../../crates/emerald-cli/tests/hello_em.rs)
and runs via `cargo test -p emerald-cli` (or `cargo test --workspace`).
