2026-09-21T19:50:00Z

# Inception 3 — Standard Library Supremacy

Companion to [`2026-09-08T173600Z-inception.md`](./2026-09-08T173600Z-inception.md)
and [`2026-09-19T101000Z-inception-2-sable-alignment.md`](./2026-09-19T101000Z-inception-2-sable-alignment.md),
not a replacement for either. This document governs a single, large body
of existing work: **plans 91 through 191** (101 files, `history/2026-09-21T20*`
through `2026-09-21T214000Z`), authored in one session as a sequential,
one-crate-at-a-time expansion of `emerald-rt` — a real Rust native-runtime
crate, an FFI/ABI convention, a resource-handle lifetime model, and then
networking, cryptography, data formats, compression, databases, OS
integration, text processing, observability, media codecs, and CLI
polish, roughly one Rust crate wrapped per plan.

**Every one of those 101 plans is kept. None is cancelled, shrunk, or
declared out of scope by this document.** The research behind them is
real and unusually careful — version numbers verified against live
crates.io/docs.rs pages, security advisories checked and named rather
than assumed away, pure-Rust-vs-C tradeoffs made explicitly and
consistently against a stated policy (plan 95), honest stretch-tier
flags on the plans that deserve them. This document does not second-guess
that research.

What it does instead, because the batch needs it and nothing in the
batch itself provides it:

1. **Grounds the whole batch against how other languages actually
   structure a standard library** — Node.js, Python, Bun, Go, Rust, C++,
   and Ruby (Emerald's own syntactic parent) — not as a checklist to
   match feature-for-feature, but to borrow each one's real, hard-won
   lesson about what a "standard library" is *for*.
2. **Reorganizes the 101 plans into domains and priority tiers**, since
   right now they carry only a flat authoring-order sequence (91, 92,
   93, ...) with no signal about what's foundational versus what's
   genuinely a stretch. The batch's own stretch-tier flags (plans 128,
   129, 165, 166, 167, 175, 186, and others) are real and correct where
   they exist — but most plans carry no priority signal at all, and nothing
   currently distinguishes "clap-based CLI flag parsing" (plan 182,
   something close to every real program needs) from "7z archive
   support" (plan 136, explicitly this batch's own lowest-priority
   compression format) beyond their position 91 slots apart in one long
   list.
3. **Names the real gaps** the comparative survey surfaces — capabilities
   every other language's stdlib treats as load-bearing that no plan in
   91-191 currently owns. The most consequential one (§4) is worth
   reading even if nothing else here is: automatic serialization for a
   user's *own* class, not just a dynamic parsed-JSON tree.

Per direction: this document is authored to number-and-date *before*
plan 91 (`2026-09-21T20*`) precisely so it reads, in the plan-of-plans
index and in `history/`'s own chronological order, as the document the
91-191 batch should be checked against — the same relationship
`inception-2` already has to plans 71-85. It is **not** plan 91 itself,
and the 101 existing files are **not renumbered** — every one of them
carries dozens of internal cross-references to other plans by number
("plan 93's resource-handle registry," "plan 100's HTTP client," "plan
118's `JsonValue`"); renumbering 101 mutually-referencing documents to
close a one-slot gap would trade a real, low-risk organizational
problem for a real, high-risk one (silently stale cross-references)
for zero durable benefit over the domain/tier tables in §5 below.

---

## 1. What a standard library is *for* — six real answers, not one

Six ecosystems, six different bets about what belongs in the box versus
one import/crate/package away. Each bet is a real, working answer to
the same underlying tension: safety and consistency (the stdlib's own
code is reviewed once, by the language's own maintainers, and trusted
by everyone) versus velocity and specialization (a fast-moving
ecosystem package can iterate, compete, and die on its own schedule
without the stdlib carrying its mistakes forever).

| Language | The bet | What it optimizes for | The real cost, paid honestly |
|---|---|---|---|
| **Python** | "Batteries included" — a famously broad stdlib (`itertools`, `functools`, `collections`, `asyncio`, `dataclasses`, `unittest`, `argparse`, `json`, `sqlite3`, `logging`, `pathlib`, ...) covering most everyday needs with zero external dependency | Discoverability and zero-decision-fatigue for the common case; a script that only needs the stdlib never touches a package manager at all | Stdlib modules age slower than the ecosystem — `urllib`/`http.client` are real but nobody reaches for them over `requests`/`httpx` in practice; the stdlib becomes a second-tier, "it's there but the good version is a `pip install` away" experience for entire domains |
| **Go** | A disciplined, curated core (`net/http`, `encoding/json`, `context`, `sync`, `io`) with an explicit, public [compatibility promise](https://go.dev/doc/go1compat) and a famously high bar for what gets added — the language's own maintainers have publicly declined entire categories (a general-purpose GUI toolkit, a full ORM) as permanently out of scope | Long-term stability or a whole ecosystem depends on `net/http` never breaking; genuine "no dependency needed" confidence for the things that *are* in the box | Real gaps stay real gaps for a long time — no first-party `Set` collection type in the language `or` its stdlib to this day, generics arriving only in 1.18 (2022) after a decade of the language shipping without them |
| **Rust** | The opposite bet from Python: a deliberately *minimal* `std` (collections, `io`, threads, no JSON, no HTTP, no regex, no async runtime) — everything else lives in crates.io, and the ecosystem converged on a small number of de facto standards (`serde`, `tokio`, `clap`, `regex`) that function as an unofficial second stdlib tier | `std` can be genuinely stable and dependency-free even as the ecosystem iterates fast around it; no domain is ever "stuck" with a stdlib module that can't keep up | Real fragmentation risk (multiple competing crates for the same job) that only Rust's own unusually strong convergence culture keeps in check — and even then, this exact 101-plan batch's own research had to make a real, cited call between competing crates for JSON codecs, YAML parsers, HTTP clients, and embedded databases, every single time |
| **Node.js** | A small, low-level core (`fs`, `http`, `net`, `crypto`, `stream`, `buffer`, `events`) deliberately leaving almost everything higher-level (routing, ORMs, validation, most utility functions) to npm | Extreme flexibility, the fastest-moving ecosystem of any language here | The single most infamous "dependency hell"/supply-chain-risk case study in mainstream programming — `is-odd`/`left-pad`-class incidents are a direct, structural consequence of this exact bet |
| **Bun** | Node's own bet, but with the load-bearing *tooling* pulled back into the runtime itself (`Bun.serve`, `bun:sqlite`, `bun:test`, a built-in bundler/transpiler) rather than left to the ecosystem, specifically because those particular tools turned out, in practice, to be load-bearing enough that every real Node project ends up choosing one anyway | Genuinely faster time-to-first-line-of-code than Node for the common "web server + SQLite + tests" shape, without npm-installing three separate competing packages for it | A second, narrower "what's actually load-bearing enough to deserve first-party status" bet than Node's own — correct often enough to be Bun's actual selling point, but still a bet, and still incomplete outside its own chosen niches |
| **C++ (STL)** | Algorithms and containers as a genuinely *generic*, composable toolkit (`<algorithm>`, `<ranges>`, iterators, `std::vector`/`std::map`/`std::unordered_map`) rather than a domain checklist — no JSON, no HTTP, no regex until C++11, no filesystem until C++17 | A small number of extremely well-designed, zero-cost-abstraction primitives that compose into almost anything; genuinely the model for how algorithmic/collection-shaped stdlib code should look | Everything domain-specific (networking, JSON, crypto) is either absent or arrived a decade-plus late (`<filesystem>` in 2017, no first-party JSON/HTTP/crypto ever) — the ecosystem (Boost, then a fragmented mess of competing libraries) fills the gap unevenly |
| **Ruby** (Emerald's own syntactic parent, cited for continuity with `inception.md`'s own framing) | A genuinely broad stdlib (`net/http`, `json`, `csv`, `digest`, `openssl`, `date`) *plus* an unusually powerful ecosystem convention (RubyGems, Bundler) that makes "one gem away" feel almost as immediate as "in the box" | The productivity Rails-era Ruby was famous for — batteries included, and the ecosystem batteries are just as easy to reach | Real stdlib/gem overlap and inconsistency (multiple JSON-adjacent conventions, `Net::HTTP`'s own famously awkward API next to `Faraday`/`HTTParty` as the ecosystem's real answer) |

**The synthesis this document draws from these six, not a repeat of
any one of them:** Emerald is a statically-typed, no-reflection,
no-macro, ahead-of-time-compiled language with an actor model already
built in and a real, disclosed "no GC yet" ceiling — closer in spirit
to Rust and Go than to Python or Node, but without either's ecosystem
scale to lean on (Rust's crates.io convergence culture, Go's decade of
public API stability, both took *years* neither this batch nor a v1
stdlib gets). That means:

- **Go's discipline, not Python's breadth, is the right default posture**
  — a curated core with a real, stated compatibility bar, not "include
  everything eventually." Plan 95's own crate-vetting policy already
  gestures at this; this document makes it explicit as the whole
  batch's governing posture, not just its dependency-hygiene rule.
- **But Rust's "std stays minimal, one obvious crate fills each gap"
  model is the wrong fit here specifically** — Emerald has no package
  manager ecosystem remotely close to crates.io's scale (plan 46's
  `emerald.toml` exists and works, but there is no second-tier
  "everyone converges on this one crate" culture behind it yet, because
  there's no crate ecosystem behind it at all). A `Result[T, String]`
  from `Json.parse` with no ecosystem alternative a caller could reach
  for instead is a very different tradeoff than Rust's own `serde_json`
  sitting next to four competing alternatives.
- **C++'s algorithmic-generality lesson matters more than its own
  history suggests**, precisely because Emerald's enumerable stdlib
  (plan 70/74) and this batch's own many ad hoc streaming-handle types
  (`TarReader`, `ZipReader`, `XmlReader`, `GzipReader`, image pixel
  buffers) are exactly the kind of thing a small number of well-designed
  generic primitives (a real iterator protocol, a real reader/writer
  interface) would let compose, instead of each domain plan re-deriving
  its own handle shape independently — see §4.2.
- **Bun's "pull load-bearing tooling into the runtime" instinct is
  already this batch's own instinct**, correctly: SQLite (plan 137),
  a real test runner (already shipped, plan 47/80), structured logging
  (plan 168) as this batch's own named "foundational... every later
  plan only becomes debuggable once this exists" plan. This document
  endorses that instinct explicitly rather than leaving it implicit.

---

## 2. What "supersedes any language's stdlib" actually has to mean

Read literally, "better than any stdlib ever" is not a coherent
engineering target — six languages above made six different, each
individually defensible bets, and "better on every one of those axes
simultaneously" is not a real option. Read as this project's own
"no rounding up" standard applies to itself: the honest, achievable
version of that ambition is —

> **Ship a stdlib whose *core* (§5.A-§5.C below) is as disciplined and
> stable as Go's, whose data/serialization story is as ergonomic as
> Rust's `serde` ecosystem (not just a dynamic JSON tree — see §4.1),
> whose collection/iteration primitives are as composable as C++'s STL
> and Rust's `Iterator` trait (see §4.2), and whose crate-selection
> rigor (already real, in plan 95 and every domain plan's own Decision
> log) stays exactly this careful as the batch grows — while declining,
> explicitly and by name, the parts of other languages' stdlibs that
> exist only because of constraints Emerald doesn't share** (Python's
> `asyncio` — Emerald has actors instead, already more developed per
> `inception-2`'s own comparison table; Node's `EventEmitter` — same
> reason; C++'s manual memory management primitives — Emerald's own
> memory model, disclosed-incomplete as it is, is a deliberately
> different bet already).

That is a real, falsifiable target, not a marketing claim — and it is
the standard the rest of this document, and every plan it reorders, is
measured against.

---

## 3. A concrete illustration of the reorganization problem

Before the domain tables: one real example, not a hypothetical, of why
flat authoring order isn't priority order.

**Plan 182** (`cli-flag-parsing`, wrapping `clap` 4.x) — real, structured
command-line argument parsing with subcommands, flags, and help text —
is something close to every non-trivial Emerald program will eventually
want; every language surveyed in §1 either ships this in its stdlib
(Python's `argparse`, Go's `flag`) or converged on one de facto standard
crate for it (Rust's `clap` — the exact crate plan 182 itself wraps).

**Plan 136** (`7z-archives`) is explicitly, self-describedly this
batch's own "lowest-priority, stretch-tier" compression format plan —
a real capability, honestly scoped, but one its own author states
plainly is "far less common in real-world Emerald programs than
`.tar`/`.zip`."

In the 101-plan batch's own flat sequence, plan 136 comes **46 plans
before** plan 182. Nothing about that ordering is wrong in the sense of
containing an error — each plan is independently correct and honestly
scoped — but a reader (or an implementing agent) working straight
through the batch in file order would build 7z support fully six weeks
of plan-authoring "distance" before building the argument parser most
Emerald *programs* — including the ones needed to exercise every other
plan in this batch from a real command line — will actually reach for
first. §5 fixes this specific problem, batch-wide, not just for these
two plans.

---

## 4. Real gaps — capabilities no plan in 91-191 currently owns

Checked against all 101 plans' own overview text, not assumed from a
gap in memory. Each of these is something at least one language in §1
treats as load-bearing enough to be a stdlib (or de facto ecosystem
standard) primitive, and none of the 101 plans claims it.

### 4.1 Automatic serialization for a user's own class — the single highest-leverage gap

Plan 118 gives Emerald a real, well-designed answer to "how does a
dynamically-shaped, schema-less value (parsed JSON, TOML, YAML, XML,
CBOR) live inside a statically-typed language with no reflection" — a
compiler-synthesized `JsonValue` tagged union, consumed via exhaustive
`case`/`when` pattern matching. That is a genuinely good answer to that
question, and plans 119, 120, 124, 189 correctly reuse it rather than
each re-deriving a competing dynamic-value model.

**It does not answer a different, more common question**: a caller with
their own `class Point { x: Float64; y: Float64 }` who wants
`Json.to_string(my_point)` and `Json.parse_as[Point](s)` — the thing
every language surveyed in §1 treats as the *actual* everyday JSON
story (Rust's `#[derive(Serialize, Deserialize)]`, Go's struct tags +
`encoding/json`, Python's `dataclasses`+`json`/`pydantic`, C#'s
`System.Text.Json`, even Ruby's own `to_json`/`JSON.generate` convention
on plain objects). Today, per every one of plans 118-129's own stated
scope, a caller must manually walk their own class's fields into/out of
a `JsonValue` tree by hand, every time, for every format.

Emerald has no macro system and no runtime reflection (both correctly,
permanently declined — `inception.md` §19/§20) — so this cannot be
solved Rust's way (a derive macro) or Python's way (runtime
introspection). It has to be solved the way the compiler itself already
solves the closest analogous problem: `derive Comparable`
(`ast::expand_derives`, cited directly in this session's own
plan-of-plans corrections) already exists as a real, working,
compile-time, non-reflective mechanism for synthesizing method bodies
from a class's declared field list. **The recommended shape**: a
`derive Serializable` (or similarly-named) compiler-synthesized trait,
implemented once in `emerald-parser`'s own `expand_derives` alongside
`Comparable`, generating `to_json_value(self): JsonValue` and
`from_json_value(v: JsonValue): Result[Self, String]` method bodies
directly from a class's own already-known, already-typed field list —
zero reflection, zero macros, the exact same "the compiler already has
this information at the point it needs it" argument `derive Comparable`
already proved out. Plans 119/120/124/189 (TOML/YAML/XML/CBOR) would
each get this for free the moment plan 118's `JsonValue` gets it, since
all four already reuse `JsonValue` as their own dynamic-value
representation.

**Priority: foundational.** This belongs in §5.A, authored and shipped
before or alongside plan 118, not after it — every later data-format
plan's own real-world usefulness compounds once user classes, not just
dynamic trees, can round-trip.

### 4.2 A real iteration protocol, and the streaming-handle types it would unify

Two related, compounding gaps:

- **`for...in` is grammar-restricted to a literal array (plan 30's own
  disclosed scope), and enumerable chaining (plan 74) materializes a
  real, fully-allocated `Array` at every stage** — eager, not lazy, a
  real difference from Python's generators, Rust's `Iterator` trait, or
  C++'s `<ranges>`, none of which allocate an intermediate collection
  for `.filter(...).map(...).take(5)`-shaped code. Plan 74's own record
  already discloses `Iterable[T]` doesn't exist as a real,
  compiler-recognized interface (blocked on `Proc[Args...,Ret]`
  bracketed-type parsing at the time) — a gap 91-191 does not revisit.
- **Independently, this exact batch invents at least five different ad
  hoc "read one chunk/entry at a time" handle shapes** — `TarReader`
  (plan 132), `ZipReader` (plan 133), `XmlReader` streaming mode (plan
  124), `GzipReader`/`ZstdReader`/`Lz4Reader`/`BrotliReader` (plans
  130/131/134/135), `Field.read_chunk` (plan 105) — each independently
  designed, each unable to compose with the others (no single "copy
  from any readable to any writable, in fixed-size chunks" function can
  be written once and reused across all five).

**Recommended shape**: a real `Iterable[T]`/`Iterator[T]` compiler-
recognized interface (closing plan 74's own disclosed gap, not
reopening a settled question) is the more foundational of the two and
should land first — it is a language-surface change, not a stdlib
wrapper, and every later stdlib plan that produces or consumes a
sequence benefits from it existing. A general `Reader`/`Writer`
interface pair (mirroring Go's `io.Reader`/`io.Writer` and Rust's
`std::io::Read`/`Write` — both chosen for direct precedent, not
invented from scratch) is the natural second step once `Iterable[T]`
exists, and should be retrofitted onto the batch's existing streaming
handles (mechanical, low-risk per-plan follow-ups) rather than
redesigned into each of them today.

**Priority: foundational for `Iterable[T]`, important-but-deferrable for
the `Reader`/`Writer` retrofit** — the retrofit's value compounds as
more streaming plans ship, so it is more valuable done once, later,
against a fuller set of handle types than done piecemeal now.

### 4.3 Missing collection/algorithm primitives

No `Set[T]` anywhere in the shipped stdlib or the 91-191 batch (every
language in §1 except pre-C++11 has one); no priority queue/binary heap
(Python `heapq`, Rust `BinaryHeap`, Go `container/heap`, C++
`std::priority_queue`); no double-ended queue (Python `collections.deque`,
Rust `VecDeque`, C++ `std::deque`). These are small, well-understood,
low-risk additions — real gaps, but not architecturally deep the way
§4.1/§4.2 are.

**Priority: important, not foundational** — genuinely useful, low
implementation risk, no design dependency on anything else in this
document; can be slotted into §5.B without disturbing the rest of the
sequencing.

### 4.4 Structured, typed errors instead of bare `String` messages

Nearly every plan in the batch that can fail returns `Result[T, String]`
— `Json.parse`, `Toml.parse`, `Csv.parse`, `Regex.compile`, and dozens
more. A bare string is a real, working, low-effort answer (and this
project's own existing `Result[T,E]`/`?`-propagation model, plan 53,
already supports a richer `E` just as easily) — but it means a caller
can never *branch* on error kind (`is this a not-found error, a
permission error, a parse error, a timeout?`) the way Rust's
`std::error::Error`/downcast ecosystem, Go's `errors.Is`/`errors.As`, or
Python's exception-class hierarchy all deliberately allow.

**Recommended shape**: not a new mechanism — the language already has
enums/ADTs (plan 52) and `Result[T,E]` (plan 53) — just a real,
consistent *convention*: each domain plan that currently returns
`Result[T, String]` should return `Result[T, <Domain>Error]` where
`<Domain>Error` is a small, real enum (`JsonError = Syntax(String) |
UnexpectedEnd | ...`), not a further wave of new infrastructure. This is
cheap to retrofit per-plan and cheap to get right from the start in
every plan not yet implemented.

**Priority: important, cross-cutting, cheapest to fix now** — a
one-line convention change to every not-yet-implemented plan's own
Concrete Proof section costs far less than a retrofit after 101 plans'
worth of call sites exist.

---

## 5. The batch, reorganized — domains and priority tiers

Every plan number below is one of the 101 already-authored files
(91-191); nothing here is a new plan number except where §4 recommends
one (marked **NEW**). Tier meanings: **Foundation** — nothing else in
the batch is buildable without it; **Core** — what most real Emerald
programs will reach for; **Extended** — real, valuable, not universally
needed; **Specialized/Stretch** — narrow-audience, several already
self-flagged as such by their own authors, unchanged here.

### 5.A Foundation (build first, strictly in this order)

| Plans | Domain | Why first |
|---|---|---|
| 91, 92, 93, 94, 95 | `emerald-rt` crate, FFI/ABI, resource handles, async/sync bridging, crate-vetting policy | Every other plan in the batch is a tenant of this mechanism |
| **NEW** (§4.1) `derive Serializable` | Compiler-level serialization derive | Every data-format plan (118-129, 189) is more valuable once user classes round-trip, not just dynamic trees |
| **NEW** (§4.2) `Iterable[T]`/`Iterator[T]` | Language-level iteration protocol | Closes plan 74's own disclosed gap; every streaming/collection plan downstream benefits |
| 118 | JSON (`JsonValue`) | The dynamic-value answer every later format plan (119, 120, 124, 125, 126, 128, 189) reuses verbatim |
| 168 | Structured logging | This batch's own stated foundational plan — everything native becomes debuggable once this exists |

### 5.B Core (the everyday stdlib — build second, order within is low-stakes)

| Domain | Plans |
|---|---|
| Text/bytes fundamentals | 122 (regex), 123 (base64/hex), 153 (charset encoding), 154 (Unicode normalization/segmentation) |
| Collections | **NEW** (§4.3) `Set[T]`, priority queue, deque |
| Numbers/time | 163 (bignum/decimal), 160 (datetime/timezones), 162 (humantime), 164 (libm) |
| Filesystem/OS | 144 (extended fs), 145 (process spawn), 146 (env vars), 147 (temp files), 150 (globbing), 152 (system info) |
| Networking core | 96 (TCP/UDP), 97 (DNS), 98 (URL), 99 (TLS), 100 (HTTP client), 101 (HTTP server) |
| Data formats | 119 (TOML), 121 (CSV), 125 (binary/msgpack) |
| Compression | 130 (gzip/deflate), 132 (tar), 133 (zip) |
| Crypto core | 109 (hashing), 113 (CSPRNG), 117 (constant-time compare), 110 (AEAD), 112 (password hashing), 115 (KDF) |
| Embedded storage | 137 (SQLite), 142 (redb) |
| CLI ergonomics | 182 (clap flag parsing), 183 (layered config), 191 (progress/formatting) |
| Testing | 180 (real property-based testing, closing plan 80's own disclosed simplification) |

### 5.C Extended (real, valuable, build as demand appears)

| Domain | Plans |
|---|---|
| Networking extended | 102 (WebSocket), 104 (SSE), 105 (multipart), 106 (hyper/advanced HTTP) |
| Crypto extended | 111 (asymmetric), 114 (JWT), 116 (X.509) |
| Auth protocols | 184 (TOTP/2FA), 185 (OAuth2 client) |
| Data formats extended | 120 (YAML), 124 (XML), 127 (INI), 189 (CBOR) |
| Compression extended | 131 (zstd), 134 (LZ4), 135 (brotli) |
| Databases extended | 138 (PostgreSQL), 139 (MySQL), 140 (Redis), 143 (S3) |
| OS extended | 148 (file locking), 149 (fs watching), 151 (signal handling) |
| Text extended | 155 (fuzzy matching), 156 (diffing), 157 (slugify), 158 (markdown) |
| Templating | 171 (template rendering) |
| Observability extended | 169 (metrics), 170 (OpenTelemetry) |
| Email | 176 (SMTP) |
| Algorithms | 188 (graph algorithms) |

### 5.D Specialized / stretch (real, honestly narrow — unchanged from the batch's own self-assessment)

| Domain | Plans |
|---|---|
| Exotic networking | 103 (gRPC — disclosed-incomplete closure marshaling gap), 107 (mDNS), 108 (QUIC/HTTP3) |
| Exotic archives/formats | 126 (protobuf — a real second build pipeline, heaviest-lift in the batch), 128 (HCL), 129 (Arrow), 136 (7z) |
| Security, narrow | 186 (WebAuthn — self-flagged stretch) |
| Math/science | 165 (linear algebra), 166 (statistics), 167 (geospatial) — all three self-flagged stretch |
| Media | 172 (image codec), 173 (PDF generation), 174 (QR code), 175 (audio decode) |
| Search | 187 (full-text search/tantivy — a genuine standout capability, but narrow-audience) |
| Concurrency internals | 177 (rayon, Emerald-visible surface only), 178 (lock-free, internal-only), 179 (channels, internal-only) |
| Terminal | 190 (terminal UI/ratatui) |
| Compiler quality, not stdlib | 161 (cron — scheduling primitive, low-stakes either tier), **181 (fuzzing harness) — flagged here as a real miscategorization**: it fuzzes the compiler's own front end, not a stdlib surface at all, and should be tracked and prioritized against compiler-quality work, not this batch |

---

## 6. What this document asks of the 91-191 batch, concretely

1. **No plan is cancelled, shrunk, or reordered in its own file.** Every
   plan's own content stands as authored; this document is a routing
   layer, not an edit to any of the 101 files.
2. **Two new, small design plans should be authored and slotted into
   §5.A before implementation of the batch begins in earnest**: the
   `derive Serializable` mechanism (§4.1) and the `Iterable[T]`/
   `Iterator[T]` protocol (§4.2) — both closer in kind to `inception.md`'s
   own §12/§17 milestone-shaped plans than to this batch's crate-wrapping
   plans, and both change the language surface, not just add an
   `emerald-rt` module.
3. **§4.3's collection primitives and §4.4's structured-error convention**
   should be applied to plans 118 and onward as they are actually
   implemented — a cheap correction now, an expensive one after the
   fact.
4. **Plan 181 should be tracked separately from this batch** — real,
   valuable work, wrong shelf.
5. **The tier tables in §5 are the actual build order this document
   recommends**, superseding plan-authoring order (91, 92, 93, ...) as
   the sequencing signal for anyone picking up implementation.

This document does not itself decompose into Maestro tasks or carry an
execution overlay — per `inception.md`/`inception-2`'s own precedent,
that is each individual numbered plan's job (91-191 already have that
shape; the two `NEW` items in §5.A need it authored before EXECUTE).

## 7. Update (2026-09-22): every real gap named in §4 now has an authored plan

Planning only — nothing in this update reflects executed code beyond
what plan-of-plans.md's own Foundation-tier row already discloses
(`derive Serializable` implemented, 4/5 leaves).

- **§4.1 (automatic serialization)** — `to_json_value` shipped
  (`history/2026-09-22T033000Z-derive-serializable.md`). Its deferred
  `from_json_value` half now has a real unblocking plan:
  `history/2026-09-22T224200Z-plan-196-class-level-static-methods.md`
  (an explicit `static` modifier on `MethodDef`, no grammar change on
  the call side — verified against `grammar.lalrpop`'s current
  `MethodDef` production, which has no self-less form today).
- **§4.2 (iteration protocol)** — plan 192's own two open follow-ups are
  no longer open: `history/2026-09-22T224000Z-plan-194-iterable-iterator-protocol.md`
  scopes a concrete ChainCallExpr diagnosis (re-verified directly:
  the recursive DoBlock alternative is embedded at three separate
  grammar tiers, not the one plan 192's own text focused on) and a
  Reader/Writer interface pair, while explicitly declining the full
  `Array`/`Hash`-onto-`Iterable[T]` dispatch retrofit for the same
  reason plan 192 itself left it open — `check_enumerable_call`'s
  existing hardcoded dispatch is not blocking anything today.
- **§4.3 (Set/priority-queue/deque)** — `history/2026-09-22T223900Z-plan-193-set-deque-priority-queue.md`.
  Routed through plan 93's `emerald-rt` resource-handle registry
  (verified: `Array[T]`/`Hash[K,V]` are hand-rolled LLVM memory in
  `emerald-codegen`, not `emerald-rt`-wrapped, but plan 09's own
  Decision log already discloses `Hash` doesn't carry `Array`'s
  unboxed-representation mandate — so the new collection types follow
  the proven resource-handle mechanism instead of extending
  codegen's built-in representation). Backed by Rust's own
  `std::collections` (`HashSet`/`VecDeque`/`BinaryHeap`) — no external
  crate; `Set`/`PriorityQueue` scoped to `Int64`/`String` (`Float64`
  excluded on the same grounds Rust's own `std` excludes it from
  `Eq`/`Hash`/`Ord`).
- **§4.4 (typed errors)** — `history/2026-09-22T224100Z-plan-195-typed-domain-errors-convention.md`,
  shaped as a cross-cutting policy document (plan 95's own
  crate-vetting-policy precedent, not a crate-wrap plan): per-domain
  `<Domain>Error` enums, never one shared cross-domain enum, with a
  mandatory `Other(String)` escape hatch; an additive
  `emerald_rt_result_err_tagged` FFI export alongside plan 92's
  existing untagged one, so real Rust error classification (e.g.
  `serde_json::Error::classify()`) survives the native boundary rather
  than being flattened to a string first. Plans 118 (JSON) and 122
  (regex) — both already shipped — are named as the real worked-example
  retrofits; every other not-yet-implemented plan applies the
  convention from EXECUTE time forward via a checklist.

**What this does not close**: plan 181's miscategorization (§5.D) is
unchanged — still real, valuable work, still the wrong shelf, still
not a stdlib plan; no new plan was authored for it, consistent with
this document's own original recommendation to track it separately
rather than fold it into this batch.

One disclosed verification gap, not silently smoothed over: plan 196's
own citation of `emerald-sema/src/lib.rs`'s exact `Expr::New` line
number was NOT independently re-verified when this update was authored
(the session's shared code-search tool was rate-limited by concurrent
authoring work at the time) — it instead cites `derive-serializable.md`'s
own already-verified finding, which is a legitimate reuse of another
plan's Decision-log finding per this project's own convention, but
whichever agent picks up plan 196 for EXECUTE should re-confirm that
line directly first, per this project's own "verified, not assumed"
discipline that plan 196 otherwise follows throughout.
