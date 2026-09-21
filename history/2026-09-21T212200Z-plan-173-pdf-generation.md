2026-09-21T21:22:00Z

---
name: PDF Generation
overview: "Wraps `printpdf` (v0.12.8, MIT, pure Rust — verified this session via docs.rs, released 5 September 2026) to give Emerald programs a native PDF-generation capability: placing text, shapes, and images on a page and writing a real `.pdf` file, with zero external tool (no shelling out to LaTeX/wkhtmltopdf/a headless browser) and no C dependency. Scoped deliberately narrow for v1 — simple document generation (text blocks, lines/rectangles, embedded images, multi-page documents), not full PDF-1.7-spec coverage (no forms, no encryption, no embedded video/3D)."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-crate-vetting-and-dependency-ledger
    content: "Verify `printpdf` 0.12.8 (MIT, released 2026-09-05 per docs.rs) against plan 95's crate-vetting policy: check for open RustSec advisories, confirm its own dependency tree is pure Rust (real, verified this session via docs.rs: `lopdf` for the PDF object model, `flate2` for stream compression — cite plan 130's own `miniz_oxide`-backend decision and use the same feature selection here to avoid silently reintroducing a C zlib dependency through printpdf's transitive `flate2` — `base64`, `serde`/`serde_derive`/`serde_json`, `smallvec`, `allsorts-azul` for font shaping/subsetting, `svg2pdf` for optional SVG embedding, and an optional `image` dependency for raster embedding shared with plan 172). Add printpdf's row (and its notable transitive `flate2` pin) to `crates/emerald-rt/DEPENDENCIES.md`."
    status: pending
  - id: leaf-document-and-page-model
    content: "Expose `PdfDocument.new(title: String): PdfDocument` and `.add_page(width_mm: Float64, height_mm: Float64): PdfPage` as a plan-93 resource-handle pair (the in-progress document is mutable native state accumulated across many calls before one final `.save(path: String)` — not a one-shot pure function like plan 172's image codec), with `.save` consuming the handle (a second `.save` call after the first, or any further mutation after `.save`, raises the same use-after-close `NativeError` plan 93 defines for every other handle type)."
    status: pending
  - id: leaf-text-shape-image-placement
    content: "Expose `PdfPage.draw_text(text: String, x_mm: Float64, y_mm: Float64, font_size: Float64)`, `.draw_line(x1,y1,x2,y2: Float64)`, `.draw_rect(x,y,width,height: Float64, filled: Boolean)`, and `.draw_image(bytes: Bytes, x,y,width_mm,height_mm: Float64)` (the last taking plan 92's binary-safe buffer convention, and composing directly with plan 172's `Image.decode`-then-`PdfPage.draw_image` pipeline — decode with one module, place with this one). Font handling for v1: ship with one bundled, license-clear default font (printpdf requires an explicit font for any text op; verify via docs.rs which embedding API the 0.12.x series exposes and pick the simplest one that avoids requiring the *caller* to source a font file) rather than exposing arbitrary user-supplied font loading — name custom font embedding as a real, deferred v2 feature."
    status: pending
  - id: leaf-tests-and-example
    content: "Ship `#[test]`s in `emerald-rt` asserting the generated PDF's byte output starts with a real `%PDF-1.x` header and is structurally parseable by re-opening it with `lopdf` directly (printpdf's own dependency — a legitimate, already-present round-trip check, not a new dependency) rather than only asserting 'it didn't panic'. Add `examples/pdf_generation_proof.em` (the Concrete Proof below) to `examples/`, wired into `emerald-cli/tests/examples.rs`'s CI table — its assertion checks the output file's magic bytes and non-trivial size (a PDF's exact byte layout can vary across printpdf patch versions even for identical input, so byte-for-byte output comparison is the wrong CI check here, structural validity is the right one)."
    status: pending
isProject: false
---

# Plan 173 — PDF Generation

Most languages need either a heavy external dependency (a bundled LaTeX
distribution, a headless Chromium instance driven via `wkhtmltopdf`-style
tooling) or a C library (`libharu`, `poppler`) to generate a PDF from a
program. `printpdf` is a real, actively maintained, pure-Rust PDF writer
(MIT, current release 0.12.8 dated 2026-09-05, verified this session via
docs.rs — its own dependency graph is a font-shaping crate
(`allsorts-azul`), a PDF-object-model crate (`lopdf`), and ordinary
serialization/compression crates, no C anywhere). Wrapping it lets an
Emerald program generate a real PDF document natively, no subprocess, no
C toolchain requirement at build time beyond what plan 91 already needs.
This is a genuine "kitchen sink" differentiator per the batch's own
mandate — most stdlibs at this maturity level do not ship this.

## Concrete proof this plan targets

```ruby
doc: PdfDocument = PdfDocument.new("Emerald Report")
page: PdfPage = doc.add_page(210.0, 297.0)
page.draw_text("Hello from Emerald", 20.0, 270.0, 24.0)
page.draw_rect(20.0, 240.0, 100.0, 20.0, true)
doc.save("report.pdf")
puts File.exists("report.pdf")
```

Expected output: `true` — a real, valid `report.pdf` file exists on disk
afterward, openable in any standard PDF viewer, containing the page text
and filled rectangle.

## Decision log

- **`printpdf` is chosen over the alternative `pdf` crate (a reader, not
  a writer) and over shelling out to an external tool, on real,
  disclosed grounds.** Verified this session via a real search: the
  `pdf` crate on docs.rs is a PDF *parsing* library, not a generation
  one — a different problem this plan does not solve (reading an
  existing PDF's contents is out of scope here entirely, a possible
  future plan). Shelling out to `wkhtmltopdf` or a LaTeX toolchain was
  rejected as a first-class dependency for the same reason plan 95's
  policy generally favors an in-process pure-Rust crate over an
  external-process dependency: it would require that tool installed on
  every machine running a compiled Emerald binary, breaking the
  self-contained-binary property plan 91's own dual-archive design
  exists to preserve.
- **The document/page object is a plan-93 resource handle, not a
  one-shot pure function, and this is a deliberate difference from
  plan 172's image codec.** A PDF document accumulates state across
  many draw calls before one final serialization — `PdfDocument.new`
  and `.add_page` return handles specifically because the underlying
  `printpdf::PdfDocument`/page objects are genuinely stateful Rust
  values with no sensible one-shot API shape, unlike plan 172's
  decode-a-buffer-return-pixels image functions.
- **Font handling ships one bundled default font for v1, not arbitrary
  user font loading — a real, disclosed scope cut, not an oversight.**
  `printpdf` requires an explicit font object for any text-drawing call
  (verified via its own docs.rs API surface); exposing full custom font
  loading to Emerald source would require a font-file-handling design
  (path? bytes? which formats — TTF/OTF only, or also WOFF?) this plan
  does not need to solve to deliver real value. One bundled,
  license-clear default (e.g. a permissively-licensed open font shipped
  inside the `emerald-rt` archive itself, embedded via
  `include_bytes!`) covers the common case; arbitrary font embedding is
  named explicitly as deferred v2 work in Out of scope, not silently
  dropped.
- **The `flate2` transitive dependency reuses plan 130's own pure-Rust
  backend decision rather than introducing a second, inconsistent
  choice.** `printpdf` depends on `flate2` (verified via docs.rs) for
  its internal stream compression; this plan pins the same
  `miniz_oxide`-backend feature selection plan 130 already chose for
  Emerald's own Gzip module, so the workspace never links two different
  `flate2` backend configurations (one pure-Rust, one C-linked) for what
  is, underneath, the same crate.
- **Byte-for-byte PDF output is not a stable test target — a real,
  disclosed testing constraint, not a weaker-than-usual bar.** PDF
  generation libraries commonly embed a generation timestamp and
  internal object-numbering that can shift across patch versions of the
  same crate for byte-identical input. This plan's own `#[test]`s
  therefore assert structural validity (a real `%PDF-1.x` header,
  successful re-parse via `lopdf`) rather than exact-byte-match,
  matching how a PDF-generating library's own test suite would
  reasonably be written — this is a deliberate, justified deviation
  from the "assert exact output" pattern most of this batch's simpler
  plans (base64, hashing) can use.
- **Out of scope.** Interactive form fields, digital signatures/
  encryption, embedded video/3D content, and arbitrary custom font
  loading are explicitly deferred — this plan targets simple generated
  documents (reports, receipts, labels), not a general-purpose PDF
  authoring toolkit. Reading/parsing an existing PDF's content is a
  different, unaddressed problem (the `pdf` crate, not `printpdf`,
  would be the relevant dependency for that, a possible separate future
  plan).
