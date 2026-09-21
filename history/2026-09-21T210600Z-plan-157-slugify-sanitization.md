2026-09-21T21:06:00Z

---
name: Slugify & Text Sanitization — `slug` for URL-/Filename-Safe Strings
overview: "One method, `String.slugify(self): String`, wrapping the `slug` crate (0.1.6, by Steven Allen, 941 dependent crates) to turn arbitrary Unicode text into a lowercase, hyphen-separated, URL-safe and filename-safe token — \"Hello World!\" -> \"hello-world\" — via `slug`'s own tiny, single-function public API (`slugify(s)`, its own crate is 7KB / 50 lines) plus its `deunicode` dependency for transliterating accented/non-Latin characters to plain ASCII first. A small, narrow utility wrapped with a correspondingly small plan — no invented configurability the underlying crate doesn't actually offer."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-slug-rt-crate-and-intrinsic
    content: "Add `slug = \"0.1.6\"` to `crates/emerald-rt/Cargo.toml`. `emerald_rt_string_slugify(s: *const c_char) -> *mut c_char` — decode via the plan-154-established `from_utf8_lossy` policy, `slug::slugify(&decoded)`, copy the result (already guaranteed plain ASCII — `slug`'s own output alphabet is `[a-z0-9-]` only) into an `emerald_alloc` buffer per the plan-153 callback pattern. Exposed as `String.slugify(self): String`, dispatched inside plan 45's existing `recv_ty == Type::String` `MethodCall` arm."
    status: pending
  - id: leaf-example-and-rust-tests
    content: "Add `examples/slugify_proof.em` (the Concrete Proof below) to `emerald-cli/tests/examples.rs`'s checked table. Add `#[test]`s in `emerald-rt` asserting `emerald_rt_string_slugify` against `slug::slugify`'s own output directly for the same fixed inputs used in the proof."
    status: pending
isProject: false
---

# Plan 157 — Slugify & Text Sanitization

Turning free-form text into a URL path segment, a filename, or an HTML `id`
attribute — lowercase, no spaces, no punctuation, ASCII-only, hyphen-joined
— is a small, common, and easy-to-get-subtly-wrong task (naive
`.downcase.gsub(" ", "-")`-style approaches mishandle consecutive separators,
leading/trailing punctuation, and any character outside ASCII entirely). The
`slug` crate does exactly this one job: its entire public surface is a
single function, `slugify(s: impl AsRef<str>) -> String`, built on top of
`deunicode` (a real dependency, verified this session against `slug`'s own
`lib.rs` listing) for transliterating accented Latin and non-Latin
characters to their nearest ASCII approximation before applying the
lowercase/separator/collapse rules. Verified this session: version 0.1.6
(2024-08-15), MIT/Apache, 941 dependent crates, a genuinely tiny dependency
(7KB, 50 lines of Rust) — there is no larger API surface hiding behind this
one function to expose, which is why this plan is correspondingly short.

## Concrete proof this plan targets

```ruby
puts "Hello World!".slugify
puts "  Rust & Emerald  ".slugify
puts "Café Münchën".slugify
```

Expected output:
```
hello-world
rust-emerald
cafe-munchen
```

The first two exercise `slug`'s core rules directly: lowercase, punctuation
(`!`, `&`) and whitespace runs collapse to a single `-`, leading/trailing
whitespace and separators are trimmed entirely. The third exercises the
`deunicode` transliteration step: "é" -> "e", "ü" -> "u", "ë" -> "e", all
before the same lowercase/hyphenate rules apply — the canonical example used
by both `slug`'s and `deunicode`'s own documentation.

## Decision log

- **`slug::slugify` is used with zero configuration, because it has none to
  offer — no custom separator character, no case-preserving mode, no
  maximum-length truncation.** `slug`'s entire public API is the one
  function; there is nothing to expose beyond calling it, and no config
  knob this plan could add without reimplementing pieces of the crate
  itself. This mirrors plan 155's identical posture toward `strsim`'s own
  fixed function set — a real external crate's real API surface bounds
  what a *wrapping* plan can honestly offer, and this plan doesn't
  pretend otherwise.
- **No HTML-entity decoding happens first — `"AT&amp;T".slugify` produces
  `\"at-amp-t\"`, not `\"at-t\"` or `\"atandt\"`, and that is `slug`'s own
  real, disclosed behavior, not a bug this plan introduces.** A caller
  slugifying HTML-sourced text that may contain entities needs to decode
  them first with a separate step (out of scope for this plan and this
  crate both); `slug` operates on the literal Unicode text it is handed.
- **Output is always plain ASCII (`[a-z0-9-]`), which is exactly why this
  plan's FFI shape needs no lossy-output-encoding caveat the way plan
  153's `Encoding.encode` does — the return value is always genuinely
  valid UTF-8 (a strict subset of ASCII), with no disclosed contract
  violation to warn about.**
- **Out of scope.** Custom separator characters, case-preserving slugs,
  maximum-length/truncation limits, reserved-word blocklisting, and
  locale-specific transliteration rules beyond whatever `deunicode`
  itself bakes in — none of these exist in `slug`'s own real API, and
  adding them here would mean forking or reimplementing the crate rather
  than wrapping it, a materially different and larger undertaking than
  this plan's scope.
