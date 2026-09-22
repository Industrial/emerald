# `emerald-rt` dependency ledger

Plan 95's own artifact: the running record of every third-party Rust
crate `crates/emerald-rt` depends on, why it was chosen, and which
plan added it. As of this file's own creation (plan 95), `emerald-rt`
has **zero** third-party dependencies — plans 91-93 (the runtime crate
itself, its FFI/ABI conventions, and its resource-handle registry) are
all zero-dependency by their own Decision logs, verified directly
against each plan's own text before this ledger was written.

This file is append-only in spirit, mirroring `history/`'s own
immutability convention: a later plan replacing crate X with crate Y
adds a new row for Y and marks X's own row `superseded by plan N`
rather than deleting it. Never edit an existing row's `Why chosen` or
`Pure-Rust or C-exception` columns to reflect a later plan's own
reasoning — add a new row instead.

## Column contract

- **Crate** — the crate's name exactly as it appears in `Cargo.toml`.
- **Version** — the exact version pinned when the row was added (not
  "latest" — a specific version this project vetted).
- **Why chosen** — a one-line summary only. The full justification
  (alternatives considered, WebSearch evidence, maintenance/advisory
  check) lives in the adding plan's own Decision log, cited by `Plan #`
  below — this ledger is an index into that evidence, not a copy of it.
- **Pure-Rust or C-exception (+ justification)** — either the literal
  word `pure-Rust`, or `C-exception:` followed by a short reason (e.g.
  `links libsqlite3`, `wraps zlib`) — see plan 95's own Decision log
  for the pure-Rust-first preference this column enforces.
- **Date added** — the ISO date of the plan history file that added
  the row, not the date the row itself was last edited.
- **Plan #** — the plan number whose Decision log has the full
  justification, maintenance-bar check, and (for a C-exception) the
  pure-Rust alternatives actually considered and why each was rejected.

## Ledger

| Crate | Version | Why chosen | Pure-Rust or C-exception (+ justification) | Date added | Plan # |
|---|---|---|---|---|---|
| `serde_json` (+ `preserve_order` feature, pulling in `indexmap`) | 1.0.151 | #1 in crates.io's Encoding category, 116M downloads/month, no serious pure-Rust competing choice for JSON; `serde_json::Value` lowers directly to Emerald's own `JsonValue` enum layout | pure-Rust | 2026-09-22 | 118 |

## Pre-existing transitive dependencies (out of scope, triaged)

Not `emerald-rt` dependencies at all, and predating this batch — noted
here only because `cargo audit` (wired into the gate by this same
plan) surfaces them and a future reader should not mistake silence for
an unnoticed finding. `emerald-driver` depends on `id_effect`, which
pulls in `im` 15.1.0 (transitively `bitmaps` 2.1.0 and `sized-chunks`
0.6.5) — as of plan 95's own authoring, all three carry a real RustSec
`unmaintained` advisory, and `im`/`sized-chunks` additionally carry a
real `unsound` advisory (RUSTSEC-2023-0126, RUSTSEC-2026-0255). `cargo
audit` reports these as warnings, not hard failures (its default exit
code is 0 for advisories at `unmaintained`/`unsound` severity, only
nonzero for an actual `vulnerability`-class finding) — triaged here as
a pre-existing, out-of-scope condition; fixing `id_effect`'s own
dependency choice is not this plan's mandate and is left for whichever
future plan (or a dedicated maintenance pass) actually touches
`emerald-driver`'s own dependency tree.
