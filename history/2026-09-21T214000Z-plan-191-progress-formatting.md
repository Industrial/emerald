2026-09-21T21:40:00Z

---
name: Progress Bars & Terminal Formatting
overview: "Wraps `indicatif` (v0.18.6, MIT, MSRV 1.85.0 — verified this session via crates.io, released 2026-07-01) and `console` (v0.16.6, MIT — verified this session via docs.rs, released 10 September 2026, from the same console-rs organization as indicatif and maintained by the same author, mitsuhiko, whose `tera` crate plan 171 also wraps) to give Emerald CLI programs the common, expected UX of progress bars/spinners during a long operation and basic colored/styled terminal output — the finishing polish that turns plan 100/106's HTTP downloads, plan 130/131's compression jobs, and plan 187's search indexing from silent, opaque waits into visible, legible progress."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-crate-vetting
    content: "Verify `indicatif` 0.18.6 (MIT, MSRV 1.85.0, released 2026-07-01 per crates.io) and `console` 0.16.6 (MIT, released 2026-09-10 per docs.rs) against plan 95's policy: both from the well-known `console-rs` GitHub organization, both pure Rust (`console`'s own dependency list, verified via docs.rs, is `libc`/`unicode-width`/`encode_unicode`/`windows-sys` — all either pure Rust or, for `libc`/`windows-sys`, thin raw-syscall-binding crates with no bundled C library, the same category plan 96's `std::net` already relies on transitively), no open RustSec advisory found for either. Add both rows to `crates/emerald-rt/DEPENDENCIES.md`."
    status: pending
  - id: leaf-progress-bar-api
    content: "Expose `ProgressBar.new(total: Int64): ProgressBar` as a plan-93 resource handle (a progress bar is genuinely stateful — position, message, render-refresh-timing — over its lifetime), with `.increment(n: Int64)`, `.set_message(text: String)`, and `.finish()` (which both stops the bar's redraw and marks the handle closed, folding plan 93's usual explicit `.close()` into the domain-natural `.finish()` call rather than requiring both). Cover the spinner variant (`ProgressBar.new_spinner(): ProgressBar`, for operations with no known total — e.g. plan 187's index-building, where a document count isn't known in advance) as a second constructor on the same handle type, not a separate one."
    status: pending
  - id: leaf-terminal-styling-api
    content: "Expose `Console.styled(text: String, color: String): String` (wrapping `console::Style`, verified via docs.rs to be the crate's real styling entry point) for basic named-color output (red/green/yellow/blue/etc.), and `Console.is_terminal(): Boolean` (wrapping the crate's own real TTY-detection, so an Emerald program can suppress color/progress-bar escape codes when its stdout is redirected to a file or piped to another program — a real, concrete correctness requirement: shipping raw ANSI escape codes into a log file or a downstream non-terminal consumer is a genuine, common bug this API should make easy to avoid by construction, not just possible to avoid)."
    status: pending
  - id: leaf-tests-and-example
    content: "Ship `#[test]`s in `emerald-rt` covering the styling function's output against known ANSI escape sequences for a fixed color/text input (a real, checkable string comparison, not a visual-only check) and the TTY-detection function against both a real terminal and a piped/redirected context in CI (most CI runners already run with stdout piped, making the 'non-terminal' branch trivially exercisable without special test scaffolding). Add `examples/progress_and_formatting_proof.em` (the Concrete Proof below) to `examples/`, wired into `emerald-cli/tests/examples.rs` — since CI itself runs non-interactively, the example's asserted output must be the plain, escape-code-free path (per `Console.is_terminal()` returning `false` under CI), with a documented note that the colored/progress-bar path is real but not the one CI's own byte-for-byte output assertion exercises."
    status: pending
isProject: false
---

# Plan 191 — Progress Bars & Terminal Formatting

This is the batch's finishing-polish plan: `indicatif` and `console`
(both from the `console-rs` GitHub organization, both actively
maintained — `indicatif` 0.18.6 released 2026-07-01, `console` 0.16.6
released 2026-09-10, both verified this session) are the de facto
standard Rust crates for exactly this job, used by a large fraction of
real-world Rust CLI tools. Wrapping them turns every long-running
operation this whole 91-191 batch adds — an HTTP download (plan 100),
compressing a large archive (plans 130/131), building a search index
(plan 187) — from a silent, opaque wait into a program with the same
visible-progress UX users expect from any modern CLI tool.

## Concrete proof this plan targets

```ruby
bar: ProgressBar = ProgressBar.new(100)
i: Int64 = 0
while i < 100 do
  bar.increment(1)
  i = i + 1
end
bar.finish()
puts Console.styled("done", "green")
puts Console.is_terminal
```

Expected output under CI (piped stdout): `done` printed as plain,
escape-code-free text (since `Console.is_terminal` is `false` in a
piped context, `Console.styled` returns the text unmodified — verify
this is `console::Style`'s actual real behavior when color support is
disabled, matching the crate's own documented auto-detection, rather
than assuming it), then `false`.

## Decision log

- **`indicatif`/`console` are chosen as a pair, not independently,
  because they are designed and maintained together for exactly this
  combination — verified, not assumed.** Both crates come from the
  same `console-rs` GitHub organization; `indicatif`'s own progress-bar
  rendering itself depends on `console` internally for terminal
  width/cursor control (a real, checkable fact from `indicatif`'s own
  dependency list). Wrapping both under one plan, rather than splitting
  progress bars and styling into two separate plans, reflects this
  real upstream coupling rather than an arbitrary batching choice.
- **`console`'s own TTY-detection is exposed directly (`Console.is_
  terminal`) as a first-class, separately-callable function, not
  hidden as an internal implementation detail — because getting this
  wrong is a real, common, concrete bug class.** A program that
  unconditionally emits ANSI color codes regardless of whether its
  output is a real terminal corrupts log files and breaks any
  downstream tool piping its stdout (a `grep`, a CI log aggregator).
  Making `Console.is_terminal` a plain, checkable Emerald-level boolean
  — not just an invisible internal default inside `Console.styled` —
  lets a caller make this decision explicitly wherever it matters (e.g.
  choosing whether to even construct a `ProgressBar` at all when output
  isn't a terminal, since a progress bar redrawing itself into a piped
  file produces useless escape-code noise, not a progress bar).
- **`ProgressBar.finish()` folds plan 93's usual explicit-`.close()`
  discipline into one domain-natural call rather than requiring both
  `.finish()` and a separate `.close()` — a deliberate, narrow,
  disclosed exception, in the same spirit as plan 147's tempfile
  auto-cleanup exception.** A progress bar's natural end-of-life event
  (the operation it tracks completing) and its resource-cleanup event
  (stopping the background redraw timer) are the same real moment in
  every actual use case; requiring two separate calls for what is
  always one logical event would be needless API friction without a
  corresponding safety benefit, unlike (for example) a database
  connection, where "the query finished" and "the connection should
  close" are genuinely different, independently-triggered moments.
- **CI's own byte-for-byte example assertion necessarily exercises only
  the non-terminal path — stated honestly rather than glossed over.**
  Per `examples/README.md`'s own CI contract (every example compiled,
  run, and asserted against real output on every push), and since CI
  runners run with stdout piped/captured, `Console.is_terminal` is
  `false` under that exact CI execution — meaning the colored/styled-
  escape-code path is real, shipped, working code, but is not the path
  CI's automated assertion actually exercises byte-for-byte (an
  interactive terminal session is required to see it visually). This
  gap is named explicitly in this plan's own leaf rather than silently
  leaving CI's coverage narrower than the shipped feature without
  comment.
- **Out of scope.** Full-screen/alternate-buffer terminal rendering
  (multiple simultaneous progress bars in a fixed multi-line layout,
  cursor-position-based redrawing beyond indicatif's own default
  single-line behavior) and rich interactive terminal UI are explicitly
  not this plan's job — that is plan 190's `ratatui` wrapper's domain;
  this plan is deliberately the lighter-weight, single-purpose
  progress-indicator-and-basic-styling tool, not a TUI framework.
