2026-09-21T21:08:00Z

---
name: Syntax Highlighting — `syntect` for Real Multi-Language Code Highlighting
overview: "`SyntaxHighlight.highlight_html(code, language): String` and `.highlight_terminal(code, language): String`, wrapping `syntect` 5.3.0 (the Sublime-Text-grammar-based highlighting engine, verified this session at 3.76 million downloads/month, 1,901 dependent crates, its own author's words: \"I consider this project mostly complete... not under heavy development\") built with its pure-Rust `fancy-regex` backend rather than its C-linking `onig`-backed default, so a genuinely fun \"build your own code viewer / doc generator with real multi-language highlighting\" capability doesn't smuggle a second C dependency into a Rust-native runtime crate whose entire point (plan 91) is avoiding exactly that."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-syntect-rt-crate-pure-rust-backend
    content: "Add `syntect = { version = \"5.3.0\", default-features = false, features = [\"default-fancy\"] }` to `crates/emerald-rt/Cargo.toml` — `default-fancy` pulls in `regex-fancy` (the `fancy-regex` pure-Rust backend) plus `parsing`/`dump-load`/`default-syntaxes`/`default-themes`/`html`/`yaml-load`, explicitly *not* the crate's real default `default-onig` feature (which pulls in `onig`, a binding to the C Oniguruma library via `onig-sys`, needing a C compiler and system linking) — per plan 95's pure-Rust-first crate-vetting policy, verified against `syntect`'s own real, confirmed Cargo feature graph this session."
    status: pending
  - id: leaf-load-defaults-once
    content: "A `std::sync::OnceLock<(SyntaxSet, ThemeSet)>` in `crates/emerald-rt/src/highlight.rs`, populated on first use via `SyntaxSet::load_defaults_newlines()` / `ThemeSet::load_defaults()` — both load from a compressed binary dump `syntect` embeds in its own compiled artifact (confirmed from `syntect`'s own Features/Goals list: \"Include a compressed dump of all the default syntax definitions in the library binary\"; its own README states this loads in \"around 23ms\"), so this cost is paid once per process, not once per `highlight` call."
    status: pending
  - id: leaf-highlight-html-and-terminal
    content: "`emerald_rt_syntax_highlight_html(code, language) -> *mut c_char` — `ps.find_syntax_by_token(lang).unwrap_or_else(|| ps.find_syntax_plain_text())` (graceful fallback on an unrecognized language tag, never an abort — see Decision log), `syntect::html::highlighted_html_for_string(&decoded, &ps, syntax, &ts.themes[\"base16-ocean.dark\"])`. `emerald_rt_syntax_highlight_terminal(code, language) -> *mut c_char` — the exact pattern from `syntect`'s own README example: `HighlightLines::new(syntax, theme)`, `LinesWithEndings::from(&decoded)`, `h.highlight_line(line, &ps)`, `as_24_bit_terminal_escaped(&ranges[..], true)`, concatenated. Both copy into an `emerald_alloc` buffer per the plan-153 pattern. Exposed as `SyntaxHighlight.highlight_html(code: String, language: String): String` / `.highlight_terminal(code: String, language: String): String`."
    status: pending
  - id: leaf-example-and-rust-tests
    content: "Add `examples/syntax_highlighting_proof.em` (the Concrete Proof below) to `emerald-cli/tests/examples.rs`'s checked table — its expected-output check is structural (valid non-empty HTML / a non-empty ANSI-escaped string), not a hand-transcribed literal, per the Decision log below. Add `#[test]`s in `emerald-rt` asserting `emerald_rt_syntax_highlight_html`/`_terminal`'s output against `syntect`'s own freshly computed highlighting for the same fixed input and language, and one asserting an unrecognized language tag falls back to plain-text highlighting rather than panicking."
    status: pending
isProject: false
---

# Plan 159 — Syntax Highlighting

This is the genuinely fun one in this batch: an Emerald program could use
this plan to build its own code viewer, its own documentation generator with
real highlighted code samples (pairing naturally with plan 158's fenced code
blocks), or its own `less`-with-colors. `syntect` is the standard way to get
real, Sublime-Text-quality syntax highlighting in a Rust program — verified
this session against its own `lib.rs`/`docs.rs` listing: version 5.3.0
(released 2025-09-27), MIT, by Tristan Hume (`trishume`), 3.76 million
downloads a month, 1,901 dependent crates, #3 in crates.io's own Parser-
implementations category. Its own README is refreshingly candid about its
maturity, worth quoting directly rather than paraphrasing: "I consider this
project mostly complete, I still maintain it and review PRs, but it's not
under heavy development" — a real, disclosed maintenance-posture signal
this plan takes at face value rather than glossing over, exactly the kind
of fact a crate-vetting pass (plan 95) should surface plainly.

The task framing calls this "a heavier dependency (bundles or loads syntax-
definition files)," and that is concretely true, not a vague caveat: the
crate itself is already 1MB/8K SLoC, and its own Features/Goals list states
it "Include[s] a compressed dump of all the default syntax definitions in
the library binary so users don't have to manage a folder of syntaxes" —
i.e. every language grammar and bundled theme `syntect` ships with compiles
directly into `emerald-rt`'s own static archive, a real, disclosed binary-size
cost paid by every program linking this stdlib module, whether or not it
calls `SyntaxHighlight.highlight_*` at runtime (mitigated, per plan 91's own
Decision log, by the final `cc` link step's `--gc-sections`, which can still
only discard the *function*, not the syntax-definition dump `include_bytes!`s
into the same object). In exchange, load time is genuinely fast — the
crate's own README states "around 23ms" — and highlighting itself is,
per the same README, "one of the faster syntax highlighting engines."

## Concrete proof this plan targets

```ruby
code: String = "fn main() {\n    println!(\"hi\");\n}\n"
html: String = SyntaxHighlight.highlight_html(code, "rs")
puts html

term: String = SyntaxHighlight.highlight_terminal(code, "rs")
puts term
```

Expected output: the first `puts` prints a `<pre style="background-color:
#...;">` block containing one or more `<span style="color:#...">` runs
around the tokens `syntect`'s bundled Rust grammar recognizes (`fn`, `main`,
`println!`, the string literal, braces); the second prints the same code
with 24-bit ANSI color escape sequences inserted around the same token
boundaries, ready to `print!` straight to a real terminal. This plan does
not hand-transcribe the exact hex colors or byte-for-byte span boundaries
here — doing so would itself be an untrustworthy transcription of a third
party's theme/grammar data, not a real proof, exactly the reasoning plan
91's own Concrete Proof gives for not hand-stating its FNV-1a hash values
either ("The three values themselves are not the point... computed
identically by Emerald's own compiled output and by the `emerald-rt`
crate's own `#[test]`"). The real acceptance check is structural (valid,
non-empty HTML containing at least one `<span style=`; a non-empty string
containing at least one ANSI CSI escape `\x1b[`) and cross-checked directly
against `syntect`'s own freshly computed output in this plan's own
Rust-side `#[test]`s, not against a literal string baked into this document.

## Decision log

- **Built with `default-fancy` (the `fancy-regex` pure-Rust backend), not
  `syntect`'s own real default `default-onig` (the `onig` C-library
  backend) — a direct, deliberate application of plan 95's pure-Rust-
  first crate-vetting policy, not an oversight of what `syntect` ships
  with out of the box.** Verified this session against `syntect`'s own
  real, confirmed Cargo feature graph: `regex-onig` (pulled in by the
  crate's actual `default` feature set, `default-onig`) depends on
  `onig`, which links against the C Oniguruma regex library via
  `onig-sys` — a real C dependency, and a second one at that, landing
  inside `emerald-rt`, the very crate plan 91 exists specifically to let
  this project stop adding more C to. `regex-fancy` (`fancy-regex`) is
  `syntect`'s own real, first-class pure-Rust alternative backend,
  selectable via `default-features = false, features = ["default-fancy"]`
  exactly as its own `Cargo.toml` feature table documents. The honest
  cost, disclosed rather than hidden: `fancy-regex` is a newer, less
  battle-tested regex engine than Oniguruma for some of the more exotic
  regex features a handful of Sublime syntax definitions lean on
  (lookaround, backreferences), and is generally the slower of the two
  backends for pathological patterns — a real trade this plan accepts
  in exchange for staying entirely C-free, consistent with plan 91's own
  stated reason for existing at all.
- **Unknown/unrecognized language tags fall back to `find_syntax_plain_text()`
  rather than aborting — a deliberate contrast with plan 45's File-I/O
  abort precedent, reasoned explicitly rather than copied reflexively.**
  A missing file or a malformed encoding label (plans 45/153) genuinely
  cannot proceed; an unrecognized `language` string for a highlighter
  has an obviously reasonable degraded behavior — render the code
  unhighlighted, exactly as `syntect`'s own `find_syntax_plain_text()`
  is designed for — so crashing a caller's entire doc-generation run over
  one code fence tagged with a typo'd or simply-unsupported language
  string would be strictly worse UX for no real safety benefit. This
  plan picks the least-surprising failure mode per call site rather than
  applying one fixed policy project-wide.
- **Two output modes, HTML and 24-bit ANSI terminal, both self-contained
  — no CSS-class-based (`ClassedHTMLGenerator`) output is offered.**
  `highlighted_html_for_string`'s inline-`style`-attribute output needs
  no external stylesheet shipped alongside it, matching this stdlib's
  existing "one `String` in, one `String` out, nothing else to manage"
  shape for every other intrinsic in this batch; `ClassedHTMLGenerator`
  (CSS classes referencing a separately-authored theme stylesheet) is a
  real, coherent alternative this plan declines for now specifically
  because it would leave a caller with an incomplete result (highlighted
  markup with no way to get matching CSS out of this same API).
- **The bundled `base16-ocean.dark` theme is hard-coded, not
  caller-selectable — the exact theme name `syntect`'s own README worked
  example uses, not a guess.** `ThemeSet::load_defaults()` bundles
  several real themes; exposing theme choice as a third parameter is a
  small, low-risk, real follow-on this plan leaves for a later revision
  rather than deciding a whole theme-naming/selection API design
  unilaterally as a side effect of the first, focused version of this
  plan.
- **The Concrete Proof above states a structural, not literal, expected
  output — the same honesty precedent plan 91 itself set for its own
  FNV-1a proof, extended here because the underlying data (bundled
  syntax/theme dumps) is third-party, not first-party, content this
  document could responsibly transcribe by hand.** Stated directly so a
  future reader treats this plan's Rust-side `#[test]`s, not this
  document's prose, as the real source of truth for exact output.
- **Out of scope.** `syntect`'s "code intelligence" / incremental
  re-highlighting APIs (piece-table integration, cached parse-state
  reuse across edits — real capabilities the crate advertises for text
  editors specifically; this plan's two functions are whole-buffer,
  one-shot calls, not an incremental highlighting session a hypothetical
  future Emerald-based editor plan would need), loading user-supplied
  `.sublime-syntax`/`.tmTheme` files from disk at runtime (this plan only
  exposes the bundled default syntax/theme sets baked in at compile
  time), `ClassedHTMLGenerator`/external-stylesheet output (see above),
  and any integration with plan 158's Markdown fenced-code-block
  rendering (named there and here as a real, natural follow-on, built by
  neither plan on its own).
