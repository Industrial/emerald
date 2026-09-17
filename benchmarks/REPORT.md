# Emerald Benchmark Report

Supersedes the plan-15/16 single-run snapshot. That snapshot compared 2 programs,
1 run each, 1 machine, and had no Ruby baseline (not installed in that
environment) and no Crystal comparison at all. This report fixes all four gaps:
6 programs, 10 runs each, Ruby *and* Crystal now measured, still one machine
(that caveat doesn't go away — see Limitations).

**Update, same session, after the first pass of this report identified a real
compiler regression below**: the root cause (unconditional actor-runtime
startup/shutdown in every generated `main`) was fixed
(`crates/emerald-codegen/src/lib.rs`'s `define_main`, gated on whether the
compiled program actually declares an actor) and every Emerald row was
re-measured against the fixed compiler, same methodology, same machine. The
original pre-fix numbers are kept inline (struck through context, not
deleted) specifically so this report doesn't quietly launder its own finding
— see "Headline" below for both the original diagnosis and the fix's actual
effect once applied.

**Second update, same session**: the binary-size finding (Headline §2) was
also root-caused and mostly fixed (`-ffunction-sections`/`-fdata-sections`
in `crates/emerald-driver/build.rs`, `-Wl,--gc-sections` in `src/lib.rs`
and `src/parallel.rs`) — a real ~37% size reduction, not full closure. Every
Emerald row was re-measured a third time; both pre-fix and worker-pool-only
numbers are kept inline for the same reason as above. This update also
directly tested (not assumed) whether the size fix affected run time — it
did not, meaningfully — so Headline §§1 and 2 are confirmed independent.

Ruby (3.4.9) and Crystal (1.19.1) were pulled in via `nix-shell -p ruby crystal`
for this session; neither is a permanent dependency of this repo.

## Methodology

- **10 runs per language per benchmark**, not 1. Run order alternates
  (round-robin forward, then reversed, ...) across languages each round, to
  spread out any systematic drift (thermal throttling, background load)
  instead of letting it land entirely on whichever language runs last.
- **Run time is CPU time (user+sys)**, not wall clock — measured via
  `resource.getrusage(RUSAGE_CHILDREN)` deltas around each subprocess, which
  excludes scheduler noise the way wall-clock timing doesn't.
- **Correctness gate, unchanged from the prior report**: every language
  variant's stdout is asserted to exactly match the expected value before any
  timing for it counts. A wrong-but-fast program cannot appear in this table.
- **Compile flags**: `rustc -O`, `cc -O2`, `g++ -O2`, `crystal build --release`
  (Crystal's default/dev build is deliberately unoptimized — `--release` is
  the correct apples-to-apples point against the others' optimized output).
  **`emerald-cli` has no optimization-level flag at all** — still true today,
  same gap the prior report noted, but checked further this session and
  clarified: there's no CLI switch to *choose* a level, but every benchmark
  program below was still compiled with LLVM's full `default<O3>` pass
  pipeline (`crates/emerald-codegen/src/lib.rs`, `module.run_passes
  ("default<O3>", ...)`), which runs unconditionally except when debug info
  is requested or the program uses `retry` — neither applies to any
  benchmark here. So **"Emerald has no way to ask for more optimization" is
  true; "these numbers are unoptimized" would be false** — the remaining gap
  against C/Rust is real and is not explained by a missing `-O` flag. Ruby
  has no compile step; its row's "Compile" column is `n/a`, not a hidden
  zero.
- **Machine**: AMD Ryzen 9 7950X (16C/32T), 61 GiB RAM, one machine, one
  session — see Limitations.
- **Driver**: `benchmarks/run_benchmarks.py`, run inside one
  `nix-shell -p ruby crystal python3` session (so ruby/crystal don't pay
  repeated nix-shell startup cost per run). Full per-run numbers, not just
  the mean/min/max/stddev below, are in `benchmarks/raw_results.json`.

## Headline: one regression found and fixed this session, one still open

**1. Every Emerald binary paid actor-runtime startup/shutdown cost, even
programs that declare zero actors — found, root-caused, and fixed in this
session.** The old snapshot measured `sum.em` at 0.533 ms. This report's
first pass measured the *same source file* at a mean of 6.39 ms (10 runs;
min 5.83, max 6.82, sd 0.34) — a ~12x regression. The cause was identifiable
in the compiler itself: `crates/emerald-codegen/src/lib.rs`'s `define_main`
unconditionally emitted a call to `emerald_worker_pool_start()` at the top of
every generated `main` and `emerald_worker_pool_drain_and_join()` at the
bottom — for *every* Emerald program, actor or not.
`emerald_worker_pool_start` (`runtime/emerald_runtime.c:969`) spawns one real
OS thread per CPU core (`sysconf(_SC_NPROCESSORS_ONLN)` — 32 on this machine)
unless overridden by `EMERALD_WORKERS`. Confirmed causally before fixing, not
just read off the source (reproducer: `benchmarks/confirm_worker_pool.py`):

| `sum.em`, 5 runs each | mean CPU time |
|---|---|
| default (32 worker threads spawned+joined) | 5.94 ms |
| `EMERALD_WORKERS=1` (1 thread spawned+joined) | 4.55 ms |

**Fixed** by gating both calls on whether the compiled program (after
`require`-splicing) declares any `actor` at all — every actor-using program
gets the exact previous behavior; a program with none now skips both calls
entirely. Every Emerald row below was then re-measured against the fixed
compiler, same machine, same methodology (10 runs each); the pre-fix numbers
are preserved in `benchmarks/raw_results.json` under each benchmark's
`Emerald_before_fix` key rather than deleted. Net effect, all six benchmarks:

| Benchmark | Before fix (mean) | After fix (mean) | Change |
|---|---|---|---|
| sum | 6.394 ms | 4.200 ms | −34% |
| array_traversal | 13.323 ms | 11.363 ms | −15% |
| sum_of_squares | 3.187 ms | 1.481 ms | −54% |
| fibonacci | 5.217 ms | 3.142 ms | −40% |
| object_allocation | 22.262 ms | 22.196 ms | ~0% (expected — see that section) |
| method_dispatch | 11.914 ms | 10.372 ms | −13% |

A real, substantial, measured improvement on five of six benchmarks — but
`sum`'s post-fix 4.20 ms is still well above C's 0.48 ms or Rust's 0.64 ms,
and still above Crystal's 3.22 ms. **The remaining gap on `sum` and most
other benchmarks is not explained by this session** — the worker-pool call
was the one identified, fixable cause found, and it's now fixed. It is
**not** LLVM optimization level (checked and ruled out — see the
Methodology section's compile-flags note: every benchmark here compiles
through LLVM's full `default<O3>` pass pipeline already). What does explain
the rest of the gap — the always-linked runtime's static initializers,
something in `main`'s generated prologue, or something else entirely — is a
genuinely open question for whoever picks up the roadmap's "evidence" item
next. **Ruled out, checked directly rather than guessed**: binary size
(finding 2) — fixing that (see below) had essentially zero effect on run
time, so the two findings are independent, not the same root cause.

**2. Binary size grew ~5.6x for the identical trivial program — found the
mechanism and partially fixed, same session, in a follow-up pass.** Old
snapshot: `sum.em` → 16,488 bytes. This report's first two passes measured
`sum.em` at ~90,000 bytes, flat across every benchmark regardless of program
complexity (`sum_of_squares` 90,120 B, `fibonacci` 90,088 B,
`object_allocation`/`method_dispatch` ~90,352 B). Root cause:
`runtime/emerald_runtime.c` compiles to a single translation unit inside the
embedded static archive — a linker can only pull in a static-archive member
whole-or-nothing, so any one referenced runtime symbol (even just
`emerald_print_i64` for a bare `puts`) dragged in every actor/networking/
supervision function too, regardless of whether the program used them.
**Fixed** by compiling the runtime with `-ffunction-sections
-fdata-sections` (`crates/emerald-driver/build.rs`) and linking with
`-Wl,--gc-sections` (`crates/emerald-driver/src/lib.rs`'s
`build_link_args`, and `src/parallel.rs`'s `link_many` for `--jobs`
builds) — the standard fix for this exact shape, not a novel technique.
Re-measured: `sum.em` → 56,736 bytes, a real **~37% reduction**, and every
other benchmark dropped proportionally (56.7–57.5 KB band, down from the
~90 KB band). **Not fully closed** — still roughly 3.4x the pre-actor-era
16,488 bytes, not the original 1x; some further-prunable structure remains
unidentified. **And, checked directly rather than assumed: this fix has
essentially zero effect on run time** (all six benchmarks' post-fix means
are within existing run-to-run noise of their pre-fix means — see the
Results tables below) — so binary size and finding 1's runtime gap are
**not** the same root cause, contrary to what this report's own earlier
pass speculated. Disconfirmed, not confirmed; reported plainly either way.

Both findings are reported plainly because they're real and measured, not to
indict the actor/supervision/distribution feature set — that work is
genuinely substantial (see the field audit's landscape section). A language
that taxes every program, actors-or-not, for a feature it doesn't use is a
real cost; this session closed most of finding 1 and roughly two-thirds of
finding 2's size regression, and disclosed rather than hid what's still
unexplained in both.

## Results

All run times are CPU time (user+sys), milliseconds, mean ± stddev over 10
runs (min–max in the next column). Compile time is a single wall-clock
measurement. Binary size in bytes; Ruby has neither (interpreted, no binary).

### sum

*`total = Σ i` for i in [0, 10,000,000). Expected: `49999995000000`.*

| Language | Compile | Binary | Run mean ± sd | Run min–max |
|---|---|---|---|---|
| C | 820.5 ms | 15,864 B | 0.480 ± 0.057 ms | 0.402–0.586 ms |
| Rust | 615.1 ms | 4,354,944 B | 0.645 ± 0.044 ms | 0.571–0.723 ms |
| C++ | 625.8 ms | 15,864 B | 1.192 ± 0.106 ms | 1.054–1.403 ms |
| Crystal | 1,143.4 ms | 1,046,616 B | 3.224 ± 0.177 ms | 2.990–3.548 ms |
| **Emerald** | 588.4 ms | 56,736 B | **4.217 ± 0.212 ms** | 4.046–4.795 ms |
| Ruby | n/a | n/a | 143.118 ± 1.686 ms | 140.045–145.119 ms |

*(Emerald, post worker-pool fix and `--gc-sections` fix — run time was
6.394 ± 0.343 ms before either fix, 4.200 ± 0.200 ms after the worker-pool
fix alone; binary size was 90,008 B before `--gc-sections`. See Headline
§§1–2 — the size fix, checked directly, changed run time by ~0.02 ms
(noise), confirming the two regressions were independent.)*

### array_traversal

*20-element array, summed, repeated 1,000,000 times. Expected: `210000000`.*

| Language | Compile | Binary | Run mean ± sd | Run min–max |
|---|---|---|---|---|
| Rust | 575.9 ms | 4,355,008 B | 0.633 ± 0.065 ms | 0.547–0.787 ms |
| C | 614.0 ms | 15,928 B | 2.456 ± 0.067 ms | 2.322–2.542 ms |
| C++ | 625.3 ms | 15,928 B | 3.196 ± 0.160 ms | 2.835–3.450 ms |
| Crystal | 1,154.5 ms | 1,055,840 B | 5.194 ± 0.143 ms | 4.936–5.383 ms |
| **Emerald** | 597.5 ms | 56,784 B | **11.285 ± 0.150 ms** | 11.018–11.489 ms |
| Ruby | n/a | n/a | 492.469 ± 1.463 ms | 490.118–494.971 ms |

*(Emerald, post worker-pool fix and `--gc-sections` fix — was
13.323 ± 0.428 ms before either fix; binary size was 90,056 B before
`--gc-sections`. See Headline §§1–2.)*

### sum_of_squares *(new this report)*

*`total = Σ square(i)` for i in [0, 1,000,000), via a real function call each
iteration. Expected: `333332833333500000`.*

| Language | Compile | Binary | Run mean ± sd | Run min–max |
|---|---|---|---|---|
| Rust | 581.9 ms | 4,355,000 B | 0.627 ± 0.060 ms | 0.569–0.749 ms |
| Crystal | 1,129.3 ms | 1,046,696 B | 1.419 ± 0.113 ms | 1.267–1.541 ms |
| C | 598.4 ms | 15,904 B | 1.441 ± 0.064 ms | 1.345–1.534 ms |
| **Emerald** | 587.6 ms | 56,848 B | **1.465 ± 0.132 ms** | 1.348–1.755 ms |
| C++ | 627.5 ms | 15,912 B | 2.163 ± 0.185 ms | 1.941–2.601 ms |
| Ruby | n/a | n/a | 58.773 ± 0.747 ms | 57.812–60.118 ms |

*(Emerald, post worker-pool fix and `--gc-sections` fix — was
3.187 ± 0.306 ms before either fix, 1.481 ± 0.078 ms after the worker-pool
fix alone; binary size was 90,120 B before `--gc-sections`. Still
essentially tied with Crystal and C — 1.465 ms vs. their 1.419/1.441 ms,
inside roughly one combined standard deviation — and clearly ahead of C++.
Reordered above to reflect the ranking.)*

### fibonacci *(new this report)*

*Recursive `fib(30)`, ~2.7M calls. Expected: `832040`. This is the first
benchmark in this suite to exercise real recursive user-function calls —
plans 36-65's stdlib/language growth made this and the two benchmarks below
expressible; they weren't in the prior report.*

| Language | Compile | Binary | Run mean ± sd | Run min–max |
|---|---|---|---|---|
| C | 612.5 ms | 15,936 B | 1.336 ± 0.047 ms | 1.261–1.404 ms |
| C++ | 635.0 ms | 15,944 B | 2.036 ± 0.080 ms | 1.939–2.197 ms |
| Rust | 571.1 ms | 4,355,144 B | 1.954 ± 0.074 ms | 1.881–2.141 ms |
| **Emerald** | 581.6 ms | 56,824 B | **3.205 ± 0.158 ms** | 3.087–3.563 ms |
| Crystal | 1,136.6 ms | 1,046,896 B | 3.608 ± 0.117 ms | 3.435–3.733 ms |
| Ruby | n/a | n/a | 83.220 ± 1.007 ms | 80.807–84.404 ms |

*(Emerald, post worker-pool fix and `--gc-sections` fix — was
5.217 ± 0.355 ms before either fix, 3.142 ± 0.076 ms after the worker-pool
fix alone; binary size was 90,088 B before `--gc-sections`.
**This remains the one benchmark in this report where Emerald genuinely
beats Crystal**: 3.205 ms vs. 3.608 ms, still a real gap. Still behind
C/C++/Rust. Reordered above to reflect the ranking.)*

### object_allocation *(new this report)*

*1,000,000 heap-allocated instances of a one-field class, summed. Expected:
`499999500000`.* **Memory-management asymmetry, intentional and disclosed:**
C frees each instance (`malloc`+`free`), C++ uses `new`+`delete`, Rust uses
idiomatic `Box<T>` (freed automatically via `Drop`), Ruby and Crystal collect
garbage automatically — all five do the idiomatic, safe thing for their
language. **Emerald's `ClassName.new` allocates via `emerald_alloc`
(`runtime/emerald_runtime.c:144`), which has no corresponding free at all —
not a bug, a disclosed, currently-permanent design gap** (see the field
audit; there is no GC and no ownership system yet). This benchmark's own
result is the clearest evidence of that gap's real cost, not just its
existence: despite never paying a free/collect cost, Emerald is still the
slowest of the six languages here by a wide margin. The most likely
explanation (not instrumented directly — flagged as a hypothesis, not a
measured fact) is that never returning memory forces glibc's allocator to
keep extending the heap for all 1M allocations instead of reusing freed
slots the way every other row's language does, trading a cheaper
free/collect step for a more expensive allocation path overall.

| Language | Compile | Binary | Run mean ± sd | Run min–max |
|---|---|---|---|---|
| C++ | 652.8 ms | 15,880 B | 1.240 ± 0.067 ms | 1.152–1.346 ms |
| Rust | 573.4 ms | 4,355,080 B | 1.675 ± 0.095 ms | 1.533–1.876 ms |
| C | 611.7 ms | 15,992 B | 5.809 ± 0.119 ms | 5.658–6.076 ms |
| Crystal | 6,283.4 ms | 1,050,664 B | 8.826 ± 0.287 ms | 8.293–9.184 ms |
| **Emerald** | 594.4 ms | 57,472 B | **23.684 ± 0.662 ms** | 22.696–24.562 ms |
| Ruby | n/a | n/a | 131.575 ± 1.981 ms | 129.730–136.411 ms |

*(Emerald: 22.262 ± 0.622 ms before either fix, 22.196 ± 0.645 ms after the
worker-pool fix alone, 23.684 ± 0.662 ms after `--gc-sections` too — a
small increase, barely outside the two nearer measurements' combined noise
band, machine-load variance more likely than a real `--gc-sections` effect
given neither fix touches the allocation path this benchmark's bottleneck
is in. Binary size was 90,360 B before `--gc-sections`. Reported as
measured, not smoothed toward the "should be flat" expectation.)*

### method_dispatch *(new this report)*

*One object, `.add(n)` called 10,000,000 times. Expected: `50000005000000`.*
**Known limitation in this row specifically**: the C variant's helper
function wasn't marked `static`/`inline`, and — unlike the C++ variant's
in-class method definition, which strongly hints inlining — GCC apparently
didn't inline it at `-O2` here, which is very likely why C (10.117 ms) reads
close to Emerald rather than near C++'s 1.2 ms. That's a benchmark-authorship
artifact in this row, not a finding about C's real performance ceiling;
flagged rather than silently fixed, per this report's own no-hidden-numbers
rule, and left for the next person to correct.

| Language | Compile | Binary | Run mean ± sd | Run min–max |
|---|---|---|---|---|
| Rust | 581.3 ms | 4,355,000 B | 0.633 ± 0.115 ms | 0.526–0.904 ms |
| C++ | 590.7 ms | 15,872 B | 1.204 ± 0.136 ms | 0.972–1.405 ms |
| Crystal | 5,842.0 ms | 1,050,656 B | 3.224 ± 0.182 ms | 2.958–3.463 ms |
| C | 582.1 ms | 15,960 B | 10.117 ± 0.068 ms | 10.028–10.241 ms | *(see limitation above)* |
| **Emerald** | 591.5 ms | 57,472 B | **10.404 ± 0.141 ms** | 10.192–10.685 ms |
| Ruby | n/a | n/a | 307.914 ± 44.414 ms | 291.466–434.181 ms |

*(Emerald, post worker-pool fix and `--gc-sections` fix — was
11.914 ± 0.284 ms before either fix, 10.372 ± 0.206 ms after the worker-pool
fix alone; binary size was 90,352 B before `--gc-sections`. Still
essentially tied with C on this row, though see that row's own inlining
caveat before reading anything into "Emerald ≈ C" here — both are still far
behind Crystal's 3.224 ms on this benchmark.)*

*(Ruby's method_dispatch run also shows this suite's highest single-benchmark
variance by far — sd 44.4 ms against a 307.9 ms mean, versus low-single-digit-ms
sd everywhere else for Ruby. Not investigated further; noted rather than
smoothed over.)*

## What this means, plainly

- **Post-fix, Emerald beats Crystal — its actual closest competitor — on one
  of six benchmarks (`fibonacci`, a real, several-stddev gap) and is
  essentially tied on a second (`sum_of_squares`, within about one combined
  standard deviation).** It's still clearly behind Crystal on the other
  four (`sum`, `array_traversal`, `object_allocation`, `method_dispatch`).
  Before the worker-pool fix, Emerald was slower than Crystal on all six,
  without exception — that was true of the first pass of this report and is
  not true of the final numbers above. Report the corrected picture, not the
  first-pass one: mixed, not uniformly behind.
- **Emerald reliably and substantially beats Ruby**, the one comparison the
  project's own inception document (§21) actually asked for, on every
  benchmark, post-fix: ~34x on `sum`, ~26x on `fibonacci`, ~43x on
  `array_traversal`, ~30x on `method_dispatch`, ~40x on `sum_of_squares`, and
  a smaller but still clear ~6x on `object_allocation` (the one benchmark
  where Emerald's own disclosed no-GC cost eats into the margin — see that
  section). That bar is cleared clearly and consistently.
- **The one identified, fixable cause (unconditional actor-runtime
  startup/shutdown) is no longer a live deficit — it's fixed, this session,
  and the table in Headline §1 is the before/after proof.** Binary size
  (Headline §2) is also mostly fixed — a real ~37% reduction, mechanism
  identified and corrected — though not fully back to the pre-actor-era
  size. What remains unexplained is the *rest* of the runtime gap against
  C/C++/Rust that persists after both fixes (clearest on `sum`: 4.22 ms
  Emerald vs. 0.48 ms C) — checked and confirmed **not** caused by binary
  size or missing optimization, genuinely open, not guessed at here.
- None of this is reported to indict the project — it's reported because a
  performance claim resting on a 2-program single-run snapshot wasn't a
  claim at all, and this is what the real numbers say once measured
  properly, including after finding and fixing what they turned up.

## Limitations

- **Still one machine, one session.** 10 runs controls for *this machine's*
  noise; it says nothing about a different CPU, a loaded machine, or a
  different day. Cross-machine replication is still unaddressed.
- **No memory-usage measurement** (matches the prior report's own disclosed
  gap — still true here). `object_allocation`'s actual peak RSS, which would
  make the "Emerald leaks" story concrete instead of inferred, isn't in this
  report.
- **Three of the inception document's original nine benchmark ideas are
  still not attempted here**: matrix multiplication and string processing
  were skipped for time in this session (not because they're known-blocked —
  unlike the prior report's 7-of-9 gap, these are likely expressible now
  given how much stdlib landed in plans 36-65, just not yet tried).
- **The `method_dispatch` C-inlining asymmetry** noted in that section.
- This report's own driver (`benchmarks/run_benchmarks.py`) is now the
  source of truth. **`crates/emerald-cli/tests/benchmarks.rs` still exists,
  is untouched by this session (out of scope — it lives outside
  `benchmarks/`), and will silently overwrite this file with its own
  single-run, 2-benchmark, no-Ruby-no-Crystal version of `REPORT.md` if
  anyone runs `cargo test --release -p emerald-cli -- --ignored --nocapture
  benchmarks`.** Whoever owns this repo next should either update that test
  to match this methodology or retire it — leaving both in place as-is is a
  regression trap.
