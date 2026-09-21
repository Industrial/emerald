2026-09-21T20:04:00Z

---
name: Crate-Vetting, Security Policy & Dependency Ledger
overview: "The gate every one of plans 96-191 must pass before it may add a third-party Rust crate to `emerald-rt`: a pure-Rust-first preference (decline `openssl-sys`-style C wrappers in favor of `rustls`-style pure-Rust alternatives whenever one is mature enough), a documented exception process for the real cases where no mature pure-Rust alternative exists (naming and WebSearch-verifying the alternatives considered and rejected, not assumed), a maintenance/adoption bar checked against real, current crates.io and RustSec advisory data at each plan's own authoring time rather than training-data assumptions, a `crates/emerald-rt/DEPENDENCIES.md` running ledger this plan creates with its own header and zero rows, `cargo audit`/`cargo deny` added to this project's CI gate alongside the `cargo clippy`/`treefmt`/`cargo nextest` commands `AGENTS.md` already documents, and the single, numbered, eight-item docs+example+test acceptance checklist every domain plan from 96 onward must satisfy, citing plans 91-94 by number."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-dependencies-ledger
    content: "Create `crates/emerald-rt/DEPENDENCIES.md` with a fixed Markdown table header — columns `Crate`, `Version`, `Why chosen`, `Pure-Rust or C-exception (+ justification)`, `Date added`, `Plan #` — and zero data rows beneath it, since no third-party crate has been added to `emerald-rt` as of this plan's own authoring (plan 91's own proof function and plan 92's checked/panic variants and plan 93's handle registry are all zero-dependency, verified against each plan's own Decision log). Each row-columns' exact contract, stated in the file's own preamble: `Why chosen` is a one-line summary, not a full justification (the full justification lives in that crate-adding plan's own Decision log, cited by `Plan #`); `Pure-Rust or C-exception (+ justification)` is either the literal word `pure-Rust` or `C-exception:` followed by a short reason (`links libsqlite3`, `wraps zlib`, etc.); `Date added` is the ISO date of the plan history file that added it, not the date the row was edited. State plainly in the preamble that this file is append-only in spirit — a later plan replacing crate X with crate Y adds a new row and marks the old one `superseded by plan N`, rather than deleting history."
    status: pending
  - id: leaf-pure-rust-preference-and-exception-process
    content: "Document the pure-Rust-first preference and its exception process in `DEPENDENCIES.md`'s own preamble (the ledger is the natural home for the policy that governs its own rows) and restated in this plan's Decision log below: prefer `rustls` over `native-tls`/`openssl` for TLS, prefer a pure-Rust parser/codec over one that shells out to or links a C library, for every domain this batch adds. Where no mature pure-Rust alternative exists for a domain that genuinely needs one (the plan's own worked example: SQLite — no mature pure-Rust SQLite *engine* exists as of this session, only pure-Rust *drivers* for other databases; `rusqlite` links `libsqlite3` as a real, disclosed C exception), the plan adding that crate must name the pure-Rust alternative(s) it actually considered (not a rhetorical strawman) and state why each was rejected (immaturity, a missing feature the domain genuinely needs, no maintained crate at all) — and must back that claim with a real `WebSearch`/`WebFetch` check performed at that plan's own authoring time, not an assumption carried over from training data, since crate maturity is exactly the kind of fact that goes stale between this session and whenever a given domain plan is actually authored or executed."
    status: pending
  - id: leaf-maintenance-bar-and-verification-mandate
    content: "Document the maintenance/adoption bar every crate added to `emerald-rt` must clear: real, current (checked at authoring time, not assumed) crates.io signals — non-trivial download counts, a release within a reasonable recency window, more than one maintainer or a maintainer with a real track record — and zero open, unpatched RustSec advisories against the exact version being pinned, checked against the real RustSec advisory database (`https://rustsec.org`/`cargo audit`'s own database) at authoring time. State the mandate explicitly and by name: every domain plan from 96 onward that adds or upgrades a crate must perform a real `WebSearch`/`WebFetch` against crates.io and the RustSec database as part of its own authoring, cite what it found (current version, last-release date, any advisory IDs checked and their status) directly in its own Decision log the same way plan 59 cites real file:line evidence — a plan asserting a crate's maintenance status or safety without that check is exactly the fabricated-citation failure mode `AGENTS.md`'s own 'Known tooling gotchas' section already warns this project has produced once before (plan 68's false compiler-bug report), applied here to dependency claims instead of test-output claims."
    status: pending
  - id: leaf-cargo-audit-ci-gate
    content: "Add `cargo audit` (RustSec-advisory-database scanning, the more narrowly-scoped and lower-friction of the two real options weighed in the Decision log below) to this project's quality gate, run alongside the three commands `AGENTS.md`'s own 'Build / test / verify' section already documents (`cargo nextest run --workspace`, `cargo clippy --workspace --all-targets`, `treefmt`), and update that section of `AGENTS.md` itself to list it as a fourth line (`audit: cargo audit`) — the one file-outside-`history/` edit this plan's own EXECUTE phase makes, small and additive, matching every prior plan's own light-touch `AGENTS.md` precedent. No `Cargo.lock`-pinning or `deny.toml` policy-file machinery is added in this plan beyond what `cargo audit` itself needs (it reads the workspace's existing `Cargo.lock` directly) — `cargo-deny`'s broader license/duplicate-version/banned-crate policy surface is named and declined for v1 in the Decision log, not silently assumed equivalent."
    status: pending
  - id: leaf-acceptance-checklist
    content: "Write the eight-item mandatory acceptance checklist, itemized exactly as follows, as this plan's own canonical, numbered list every domain plan (96-191) must satisfy and every reviewer checks a domain plan against: (1) real crate justification backed by `WebSearch`-verified current facts, not training-data assumptions, per `leaf-maintenance-bar-and-verification-mandate`; (2) the exact Emerald-facing API surface the plan adds (method/function signatures, types, module names); (3) FFI/ABI notes citing plan 92 (naming, panic-boundary macro usage, String vs. binary-safe convention, Result-vs-exception choice made explicit); (4) an ownership/error-handling story citing plan 93 for any long-lived native resource the domain introduces (or an explicit statement that the domain holds no such resource, if true); (5) a `## Concrete proof this plan targets` `.em` example wired into `examples/`'s CI-checked table, per plan 91's own precedent; (6) real Rust-side `#[test]`s inside `emerald-rt`, runnable via `cargo nextest run --workspace`, not merely asserted to exist; (7) doc comments on every new public/exported item, surfaced via the existing `emerald doc` subcommand (`crates/emerald-cli/src/doc_runner.rs`, verified present in the real source this session) — a domain plan adding undocumented surface fails this item even if every other item passes; (8) an explicit `Out of scope` section naming what the plan deliberately does not build. State plainly that item (3) is conditionally applicable (a domain with no async-backed dependency need not cite plan 94, but must say so rather than omit it silently) while items (1)-(2) and (5)-(8) are unconditional for every domain plan with any new stdlib surface at all."
    status: pending
isProject: false
---

# Plan 95 — Crate-Vetting, Security Policy & Dependency Ledger

Plans 91-94 built the mechanism (a second, Rust-compiled archive), the
calling convention (naming, panic boundary, `String`/binary-safe ABI,
`Result`-vs-exception), the resource-lifetime model, and the async-
bridging rule every future `emerald-rt` export follows. None of them
answer the one question that actually motivated this entire ten-plan
foundation in the first place, per this session's own directive: *which*
third-party crates are safe to depend on, and how does a domain plan
prove that rather than assert it. A wrapped crate is code this project
did not write and, for most of the 96+ domains this batch will add, did
not even review line-by-line — the same trust-boundary problem plan 59
named for arbitrary C FFI ("calling code the Emerald compiler has never
seen and cannot check") now applies to arbitrary *Rust* code as well,
with the specific difference that Rust's own memory-safety guarantees
remove one entire category of risk (no UB from a signature mismatch
inside safe Rust) while leaving every other category open: a crate can
still be unmaintained, vulnerable, malicious, or simply the wrong
architectural choice (a C-backed crate where a pure-Rust one would do,
an async-only crate where a domain never actually needs concurrency).
This plan is the policy that governs every one of the ~90+ crate
additions the rest of this batch will make, written once so no domain
plan re-derives it, and the ledger that makes "which crate did plan N
add and why" answerable by reading one file instead of grepping 90
history files.

This plan, like plan 92, adds no third-party dependency of its own —
its only artifact touching `crates/emerald-rt/` at all is the empty
`DEPENDENCIES.md` ledger file. Its only edit outside `history/` and
`crates/emerald-rt/` is a small, additive line in `AGENTS.md` naming
`cargo audit` as a fourth gate command.

## Concrete proof this plan targets

This plan, like plan 94, adds no new `emerald_rt_*` export and
therefore has no `.em` program to run — its deliverable is policy, a
ledger file, and a CI gate command, the same "a document, not a
compiling program" shape plan 82 (ownership model design) already
established as a legitimate plan shape in this project when the thing
being decided is process rather than stdlib surface. What "done" looks
like, concretely, stated the way plan 82 itself stated it: `cargo audit`
runs clean (or with every finding explicitly triaged) against the
workspace's real `Cargo.lock` as part of the same command sequence
`AGENTS.md` already documents; `crates/emerald-rt/DEPENDENCIES.md`
exists, has the six-column header specified above, and has exactly zero
data rows (verifiable with a trivial `grep -c '^|' DEPENDENCIES.md`
returning `1`, the header row alone); and this plan's own file, read by
any future domain-plan author, answers "what must my plan's own Decision
log contain before it's acceptable" without that author needing to
reconstruct the checklist from scattered precedent across plans 91-94
individually.

## Decision log

- **Pure-Rust-first is a real preference with teeth, not a soft
  suggestion — `rustls` over `native-tls`/`openssl`, a pure-Rust
  parser/codec over one that links or shells out to a C library,
  whenever a mature pure-Rust option exists for the domain in
  question.** The concrete reason this matters more here than it would
  in an ordinary Rust application: this project's entire *raison
  d'être* for this ten-plan foundation is escaping hand-written,
  unaudited C (`runtime/emerald_runtime.c`) in favor of "vetted, widely-
  used Rust crates," per this session's own directive, cited verbatim
  in plan 91's own framing. A domain plan that reaches for a crate
  which itself links an arbitrary C library (`openssl-sys`, a bundled
  `libz`, a vendored `libsqlite3`) has only moved the C-trust-boundary
  problem one layer down, not solved it — the actual C code still
  compiles into the final binary, still needs its own security patching
  cadence tracked, and now does so less visibly than
  `emerald_runtime.c` did, since it's buried inside a crate's own build
  script rather than sitting in this repository's own `runtime/`
  directory where every prior plan in this project's history has
  already been auditing it directly. Preferring pure Rust is preferring
  to actually keep the promise this whole batch exists to make, not a
  generic "Rust is safer" platitude.
- **The exception process is real and specific, not a rubber stamp: name
  the pure-Rust alternatives actually considered, verified current via
  `WebSearch` at that plan's own authoring time, and state precisely
  why each was rejected.** `rusqlite` (a real, anticipated future
  domain plan — SQLite is exactly the kind of mature, widely-used,
  genuinely-hard-to-reimplement-in-pure-Rust dependency this exception
  process exists for) is this plan's own worked example of a legitimate
  exception: as of this session, no mature pure-Rust SQLite *engine*
  exists as a drop-in `libsqlite3` replacement (pure-Rust *client
  drivers* exist for other databases — Postgres, MySQL — precisely
  because those are wire-protocol clients talking to a server process,
  a fundamentally different, easier problem than reimplementing an
  embedded file-format-and-query-engine from scratch). A plan adding
  `rusqlite` must state this comparison explicitly, with a real
  `WebSearch` performed at *that plan's own* authoring time (crate
  maturity is a moving target; a pure-Rust SQLite engine reaching
  production quality between this session and whenever that plan is
  actually authored is a real, plausible outcome this plan does not
  get to assume away), not merely repeat this plan's own 2026-09-21
  snapshot of the ecosystem as though it were permanent fact. Citing
  this plan's own text in place of doing that plan's own fresh check
  is exactly the failure mode `leaf-maintenance-bar-and-verification-
  mandate` names and forbids.
- **The maintenance bar is checked against real, current data fetched at
  each plan's own authoring time — never asserted from training-data
  familiarity with a crate's reputation.** This is the single most
  load-bearing rule in this plan, stated here as plainly as `AGENTS.md`'s
  own "Known tooling gotchas" section states its own precedent: that
  section records a real, named prior failure in this exact project —
  "plan 68 — a 'generic method repeated-print bug' that was never
  real" — caused by trusting a tool's compressed output instead of
  verifying directly. This plan applies the identical discipline to a
  different failure surface: a domain plan asserting "crate X is
  well-maintained and has no known vulnerabilities" from a language
  model's own training-data impression of that crate's reputation,
  without a real `WebSearch`/`WebFetch` against crates.io and the
  RustSec advisory database performed *at that plan's own authoring
  time*, is the dependency-vetting equivalent of that same false-report
  failure mode — plausible-sounding, citation-shaped, and wrong in a
  way nothing catches until a real vulnerability or an abandoned crate
  surfaces in production. Every domain plan's own Decision log must
  show its work: the version checked, the date of that check, the
  specific RustSec advisory IDs (if any) considered and their
  resolution status — the same "real file:line evidence, not a stale
  summary" bar plan 32 already established for source-code claims,
  applied here to external-ecosystem claims.
- **`cargo audit`, not `cargo-deny`, for v1 — a real, disclosed, narrower
  choice, not an oversight of the broader tool.** `cargo-deny` covers a
  materially larger policy surface than advisory scanning alone —
  license compatibility auditing, duplicate-version detection, an
  explicit crate allow/deny list backed by its own TOML policy file.
  All of that is real, useful, and plausibly worth adding once this
  batch has enough actual crates in `DEPENDENCIES.md` to make a license-
  compatibility or duplicate-version policy meaningful to enforce — with
  zero rows in the ledger as of this plan's own authoring, a `deny.toml`
  written now would either allow-list nothing yet (providing no value)
  or would have to speculatively pre-approve licenses/crates no plan has
  chosen yet (asserting policy ahead of any real decision it governs).
  `cargo audit` needs no policy file at all — it reads the workspace's
  existing `Cargo.lock` directly against the RustSec database and reports
  findings, a strictly smaller, immediately-useful addition to the CI
  gate today. Revisiting `cargo-deny` once `DEPENDENCIES.md` has enough
  real rows for a license/duplicate-version policy to mean something is
  named here as a legitimate future upgrade, not silently declined
  forever.
- **The `DEPENDENCIES.md` ledger lives inside `crates/emerald-rt/`, not
  inside `history/` alongside the plans that populate it.** A plan file
  is a point-in-time record of a decision, immutable once written (this
  project's own convention, verified by every prior plan in `history/`
  never being edited by a later one) — the ledger, by contrast, needs
  to answer "what does `emerald-rt` depend on *right now*" as a living
  summary, which a scan of 90 separate immutable history files cannot
  efficiently provide (deliberately, since immutability is the point of
  keeping them in `history/` at all — see the next bullet). Placing it
  next to the crate's own `Cargo.toml`, the file whose real, authoritative
  dependency list it summarizes in human-readable, decision-annotated
  form, means a reviewer or a future domain-plan author checking "has
  anyone already vetted a crate for this domain" never has to open a
  single `history/*.md` file to find out.
- **The ledger is additive/append-only in spirit, mirroring `history/`'s
  own convention, even though it is a living file and `history/` entries
  are not.** A plan replacing crate X with crate Y (a real, foreseeable
  future event — a pure-Rust alternative to some C-exception crate
  reaching maturity, say) adds a new row for Y and marks X's own row
  `superseded by plan N` rather than deleting X's row outright — the
  same "state the correction, don't erase the record" discipline plan
  54's own Decision log already modeled directly in its own text
  ("Post-authoring correction: plan 51's real, landed API is three
  functions, not the single placeholder assumed above" — corrected in
  place, with the wrong assumption left visible, not silently removed).
  A ledger that silently deletes a superseded row loses exactly the
  "why did we ever have this dependency" history a security review six
  months from now would want back.
- **The eight-item acceptance checklist is this plan's actual mechanism
  for making plans 91-94 enforceable rather than merely aspirational.**
  Plans 92, 93, and 94 each state, in their own text, what a domain plan
  citing them must show (plan 92: "a domain plan's own leaf descriptions
  must show the macro in use"; plan 94: "a domain plan that reaches for
  [async] without first stating why [sync] was unavailable... fails
  plan 95's own acceptance checklist item 3") — this plan is where that
  distributed set of forward-references actually resolves into one
  concrete, numbered list, so "does this domain plan comply" is a
  checklist a reviewer runs down once rather than a scavenger hunt
  across four other plans' prose. Item numbering is fixed and stable
  (plan 92 and plan 94 both already cite "item 3" for the FFI/ABI-notes
  requirement specifically) — a future revision to this plan that
  renumbers the list would silently break those two plans' own citations,
  so any future edit to this checklist's ordering must grep `history/`
  for `checklist item` first and update every citing plan in the same
  change.
- **Out of scope.** No retroactive audit of plan 91's own zero-dependency
  crate (there is nothing to audit — `emerald-rt` as plan 91 leaves it
  has no third-party dependency at all, verified against that plan's
  own Decision log). No specific crate is vetted, chosen, or added by
  this plan for any domain (HTTP, TLS, compression, databases, ...) —
  every one of those is a distinct future domain plan's own job, using
  this plan's process. No change to `runtime/emerald_runtime.c`'s own
  existing C code and no retroactive C-dependency audit of it either —
  this plan's policy governs new Rust-side crate additions to
  `emerald-rt` going forward, not the pre-existing hand-written C
  runtime plan 91 explicitly declined to touch or deprecate. No
  `cargo-deny` license/duplicate-version policy (named above as a real,
  deliberately deferred future upgrade). No automated CI enforcement
  mechanism beyond adding the `cargo audit` command itself to the
  documented gate — wiring an actual CI pipeline/GitHub Actions
  workflow (if one does not already exist for this project) is
  infrastructure this plan does not build, only the command this
  project's existing gate-running convention (per `AGENTS.md`) must
  include from now on.

## Not yet decided (blocking EXECUTE)

1. Whether `cargo audit` findings against a transitive (not directly
   chosen) dependency of a vetted crate block a domain plan's own merge,
   or are logged and triaged case-by-case — real projects diverge on
   this (some treat any unresolved advisory as a hard gate failure,
   others allow an explicit, documented `cargo audit --ignore
   RUSTSEC-...` suppression with a stated reason and a revisit date).
   Left for the first domain plan that actually encounters a real
   transitive advisory to decide against a concrete case, rather than a
   speculative blanket rule authored against zero real findings.
2. Whether this plan's own `AGENTS.md` edit (adding the `audit: cargo
   audit` line) needs a matching CI-workflow-file change if one exists
   outside this repository's own visible tree at authoring time — not
   verified this session; the executing leaf should check for a
   `.github/workflows/` (or equivalent) directory before assuming
   `AGENTS.md`'s own documented command list is the only place this
   needs to be wired in.
