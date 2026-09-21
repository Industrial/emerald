2026-09-21T20:38:00Z

---
name: Apache Arrow / Columnar Data
overview: "A heavy, stretch-tier, forward-looking plan, flagged as such deliberately: the official `arrow` crate (60.0.0, `apache/arrow-rs`, verified this session) wrapped at a deliberately minimal v1 slice — read/write a single-`RecordBatch` Arrow IPC *file* (not the streaming format, not multi-batch tables) restricted to Emerald's own four existing scalar types (Int64, Float64, String, Boolean) via a handle-based `ArrowTable`/`ArrowTableBuilder` API, everything else this large, many-sub-crate ecosystem offers (compute kernels, nested/list/struct/dictionary types, Parquet, Arrow Flight, and — honestly, the single most valuable real-world reason to include Arrow at all — true zero-copy interop with Python/pandas/pyarrow via the C Data Interface) explicitly deferred."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-arrowtable-read
    content: "`ArrowTable` resource handle (plan 93) wrapping one `arrow::array::RecordBatch`; `Arrow.read_ipc_file(path: String): Result[ArrowTable, String]` via `arrow_ipc::reader::FileReader`, reading exactly the first `RecordBatch` in the file (a real, disclosed v1 limitation — see Decision log) and erroring if the file contains none"
    status: pending
  - id: leaf-arrowtable-accessors
    content: "`.column_count(self): Int64`, `.column_name(self, i: Int64): String`, `.row_count(self): Int64`, `.get_int64(self, column: String, row: Int64): Option[Int64]`, `.get_float64(...): Option[Float64]`, `.get_string(...): Option[String]`, `.get_bool(...): Option[Boolean]` — `Option` because every Arrow array slot is independently nullable regardless of column type, reusing plan 73's `Option[T]` rather than a sentinel"
    status: pending
  - id: leaf-arrowtablebuilder-write
    content: "`ArrowTableBuilder.new(): ArrowTableBuilder`; `.add_int64_column(self, name: String, values: Array[Option[Int64]], count: Int64): Void` and the matching float64/string/bool variants (the `count` parameter reusing plan 45's Array[T]-has-no-length-metadata workaround, identically to plans 122/126); `Arrow.write_ipc_file(path: String, builder: ArrowTableBuilder): Result[Void, String]` via `arrow_ipc::writer::FileWriter`"
    status: pending
  - id: leaf-panic-boundary-and-tests
    content: "`std::panic::catch_unwind` at every export (plan 91); Rust `#[test]`s building a small in-memory `RecordBatch` with all four primitive types plus at least one null slot, round-tripping it through a temp-file `FileWriter`/`FileReader` pair, and asserting every value including the null"
    status: pending
  - id: leaf-example-and-gate
    content: "Add the Concrete Proof example to `examples/`, wire into `emerald-cli/tests/examples.rs`, run the full AGENTS.md gate"
    status: pending
isProject: false
---

# Plan 129 — Apache Arrow / Columnar Data

The official `arrow` crate (`apache/arrow-rs`, current release 60.0.0
published 2026-09-15, Apache-2.0, owned by the Apache Arrow project's own
maintainers — verified this session via docs.rs) is a genuinely different
weight class from every other crate in this batch. Its own crate-topology
documentation lists thirteen sub-crates it re-exports (`arrow-array`,
`arrow-buffer`, `arrow-cast`, `arrow-csv`, `arrow-data`, `arrow-ipc`,
`arrow-json`, `arrow-ord`, `arrow-row`, `arrow-schema`, `arrow-select`,
`arrow-string`, `arrow-arith`), plus separately published `arrow-flight`,
`parquet`, and `arrow-avro` crates in the same project family. Reading
its own top-level documentation this session (`compute` kernels,
`RecordBatch`/`Schema` tabular representation, a `ffi`/`ffi_stream`
module implementing the Arrow C Data Interface, ordering/selection/cast
kernels) makes plain that this is infrastructure for building a query
engine on top of, not a small format library — `arrow`'s own docs point
readers toward `DataFusion` for exactly that next step.

This plan's inclusion is a real, disclosed bet, not a foregone
conclusion: Arrow's actual value proposition — the reason a data-tooling-
minded language would want it at all — is **zero-copy interop with the
Python/pandas/PyArrow/Polars ecosystem**, where Arrow's in-memory
columnar layout is already the shared substrate letting those tools pass
data between processes and languages without a serialize/deserialize
step. That specific value is *not* what this plan's v1 slice delivers.
True zero-copy interop needs the C Data Interface (`arrow::ffi`) —
sharing raw buffer pointers and a schema descriptor across a process/
language boundary with no data movement at all — which is a materially
different, harder FFI design than every other plan in this batch's
already-established `(ptr, len)`/resource-handle conventions (plan
92/93) were built for. This plan instead proves the *file-format*
mechanism: reading and writing a real Arrow IPC file, the same format
`pyarrow.ipc.open_file`/`RecordBatchFileWriter` read and write, giving
*indirect*, file-mediated interop today (an Emerald program and a Python
script can hand data to each other via a `.arrow` file on disk) while
deferring the harder, more valuable zero-copy in-memory path explicitly,
honestly, as future work rather than claiming this plan delivers it.

## Concrete proof this plan targets

```ruby
builder: ArrowTableBuilder = ArrowTableBuilder.new
builder.add_int64_column("id", [Some(1), Some(2), Some(3)], 3)
builder.add_string_column("name", [Some("Ada"), Some("Grace"), None], 3)
builder.add_float64_column("score", [Some(97.5), None, Some(88.0)], 3)
builder.add_bool_column("active", [Some(true), Some(true), Some(false)], 3)

wrote: Result[Void, String] = Arrow.write_ipc_file("people.arrow", builder)

table: Result[ArrowTable, String] = Arrow.read_ipc_file("people.arrow")
case table do
  when Ok(t) do
    puts t.row_count
    i: Int64 = 0
    while i < t.row_count do
      name: Option[String] = t.get_string("name", i)
      match name do
        Some(n) do
          puts n
        end
        None do
          puts "(no name)"
        end
      end
      i += 1
    end
  end
  when Err(msg) do
    puts msg
  end
end
```

Expected output: `3` (row count), then `Ada`, `Grace`, `(no name)` — the
third row's `name` column round-trips through a real Arrow IPC file on
disk as a genuine null slot, read back as `None` rather than an empty
string, proving per-slot nullability survives the file round-trip
correctly.

## Decision log

- **v1 supports exactly one `RecordBatch` per file — a real, disclosed
  limitation, not an oversight.** An Arrow IPC file can legally contain
  multiple record batches sharing one schema; `arrow_ipc::reader::
  FileReader` iterates them. Supporting that would need `ArrowTable` to
  represent a *sequence* of batches (or this plan to concatenate them,
  itself a real `arrow::compute` operation) — either is real additional
  scope this deliberately minimal slice declines, matching the task's
  own framing of this plan as needing "a deliberately minimal v1 slice."
- **Primitive columns map 1:1 onto Emerald's existing four non-reference
  scalar types, inventing nothing new.** `Int64Array`/`Float64Array`/
  `StringArray`(`Utf8`)/`BooleanArray` are the only Arrow array types
  this plan supports, chosen specifically because they need zero new
  Emerald-side scalar types — Emerald's real `Type` enum already has
  exactly `Int64`, `Float64`, String, and `Boolean` and nothing
  narrower (plan 59's already-verified finding, cited again here). Every
  other Arrow primitive width (`Int8`/`Int16`/`Int32`, `Float32`,
  unsigned variants, `Date32`/`Date64`, `Timestamp*`, `Decimal128`) is
  therefore out of scope for the identical structural reason plan 126's
  protobuf field mapping declines them — there is nothing narrower or
  differently-shaped to receive them.
- **Every array slot is `Option[T]`, never a bare `T`, because Arrow's
  own nullability is per-slot and independent of column type — a real
  design constraint of the format, not an Emerald-side choice.** Every
  Arrow array carries an optional validity bitmap regardless of its
  logical type; this plan's accessor API reflects that directly with
  plan 73's `Option[T]` rather than picking a sentinel value (`0`,
  `""`) that would be genuinely ambiguous with a real, present zero or
  empty string.
- **Column construction takes an explicit `count` parameter alongside
  the `Array[Option[T]]` values, reusing plan 45's forced workaround a
  fifth time this batch (after plans 122, 123 implicitly, 126, and
  127's accessor pattern) rather than inventing a new one.** `Array[T]`
  still carries no runtime length metadata; this is the same disclosed,
  accepted shape every other plan in this batch with a runtime-length
  `Array[T]` input or output already carries.
- **Bulk, whole-column `(ptr, len)` transfer (plan 92's own convention,
  used by every other binary-payload plan in this batch) is explicitly
  deferred, not used for column construction.** Passing one
  `Array[Option[Int64]]` element at a time through the accessor API is
  real, measurable overhead for a column of any real size — the
  disclosed cost of keeping v1 simple. A later, more mature revision of
  this plan is the natural place to add a bulk raw-buffer column-
  construction path (a contiguous `i64` buffer plus a separate null-
  bitmap buffer, both `(ptr, len)`-shaped per plan 92), once this v1's
  simpler per-element path has proven the read/write/file-format
  mechanism works at all.
- **Out of scope, stated at the length this plan's own honesty
  requires.** True zero-copy interop via the Arrow C Data Interface
  (`arrow::ffi`/`arrow::ffi_stream`) — the single most valuable reason
  to want Arrow support at all, explicitly named and explicitly
  deferred, not silently dropped; the Arrow IPC *streaming* format
  (`StreamReader`/`StreamWriter`, distinct from the File format this
  plan uses); `arrow::compute` kernels (filter/sort/cast/arithmetic —
  none of this plan's read/write mechanism needs them, and exposing them
  is a distinct, large API-surface plan of its own); nested/list/struct/
  dictionary/map column types; Parquet (a separate `parquet` crate);
  Arrow Flight RPC; IPC compression codecs (LZ4/ZSTD); any date/
  timestamp/decimal logical type; multi-batch tables (above). This
  plan's honest self-assessment: it proves Emerald can read and write a
  real, standard columnar file format other tools already speak, and
  positions a future plan to build the harder zero-copy interop path on
  top of that — it does not, itself, deliver the differentiator that
  motivated including Arrow in this batch at all.
