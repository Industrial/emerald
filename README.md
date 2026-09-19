# Emerald

A statically-typed, ahead-of-time compiled language derived from Ruby's syntax and object model — native code (via LLVM) or WebAssembly, no interpreter, no bytecode VM, no dynamic runtime.

```ruby
class Point
  x: Float64
  y: Float64

  def initialize(x: Float64, y: Float64) -> Void
    @x = x
    @y = y
  end

  def distance_from_origin -> Float64
    Math.sqrt(@x * @x + @y * @y)
  end
end

p = Point.new(3.0, 4.0)
puts p.distance_from_origin  # 5.0
```

Emerald keeps Ruby's classes, blocks, symbols, string interpolation, exceptions, and modules — and cuts everything that would stop the compiler from knowing every type at compile time: no `eval`, no `method_missing`, no monkey-patching, no open classes, no duck typing. On top of that static core it adds algebraic data types, a `Result[T,E]` error channel, and an Erlang/Pony-inspired actor model — isolated per-actor heaps, supervision trees, and location-transparent distributed actors over real TCP with automatic consistent-hash cluster placement. See [`spec/SPEC.md`](spec/SPEC.md) for the full shape and what's deliberately *not* here.

## Install

```bash
nix run github:Industrial/emerald -- yourfile.em -o yourfile
nix build github:Industrial/emerald   # installs to ./result/bin/{emerald,emerald-lsp}
```

Without Nix, build from source (needs LLVM 21, `libffi`, `libxml2`):

```bash
git clone --recurse-submodules https://github.com/Industrial/emerald
cd emerald && devenv shell -- cargo build --release -p emerald-cli
# binary at target/release/emerald
```

## Try it

```bash
emerald examples/hello.em -o hello && ./hello   # 42
emerald test examples/test_framework.em          # PASS/FAIL runner
```

[`examples/`](examples/) is the actual, CI-checked source of truth for what works today — every file there is compiled, run, and asserted against its real output on every push; [`examples/README.md`](examples/README.md) also tracks the specific gaps and bugs found while writing them, rather than only listing what succeeded.

## Status, measured

- **43.7k lines of Rust**, 8 crates, **758 tests passing**, 0 clippy warnings — reproducible with `cargo nextest run --workspace`.
- **10-15% of standard Ruby's language-and-stdlib surface**, by design (see `history/`'s plan-of-plans) — a deliberate ceiling, not a target of 100%.
- **Benchmarks** (`benchmarks/REPORT.md`, 6 programs × 10 runs × 6 languages, methodology disclosed in full): beats Ruby 6×–43× on every benchmark measured; beats Crystal — the closest existing comparable language — on one benchmark and ties a second, still behind it on the rest; behind C/C++/Rust on all of them, by a gap that's real and only partly explained.
- **No garbage collector and no ownership system** — a disclosed, currently-permanent design choice (`spec/RUNTIME.md` §1), not a bug. Short-lived programs and arena/actor-scoped allocation are unaffected; unbounded allocation outside a region leaks.

None of this is rounded up. Where a claim couldn't be verified directly it isn't made.

## Documentation

| Doc | Covers |
|---|---|
| [`spec/SPEC.md`](spec/SPEC.md) | The whole shape, in one place — start here |
| [`spec/GRAMMAR.md`](spec/GRAMMAR.md) / [`spec/TYPE_SYSTEM.md`](spec/TYPE_SYSTEM.md) | What's legal to write |
| [`spec/SEMANTICS.md`](spec/SEMANTICS.md) | What it means once written |
| [`spec/RUNTIME.md`](spec/RUNTIME.md) | Memory model, actors/scheduler/supervision, distribution |
| [`spec/OWNERSHIP.md`](spec/OWNERSHIP.md) | Forward design for real ownership/borrowing (plan 82) — not yet implemented |
| [`spec/COMPILER.md`](spec/COMPILER.md) | How source becomes a binary |
| [`editors/README.md`](editors/README.md) | VSCode/Cursor, Neovim, Helix, Zed, MCP client setup |
| [`benchmarks/REPORT.md`](benchmarks/REPORT.md) | Full performance methodology and results |
| [`RELEASING.md`](RELEASING.md) | Release process |

## Contributing

`AGENTS.md` has repo conventions; `cargo nextest run --workspace`, `cargo clippy --workspace --all-targets`, and `treefmt` are the gates a change needs to clear (`devenv shell -- pre-push` runs all of them). Licensed MIT OR Apache-2.0, your choice.

---

## Dev environment

This repo's own tooling (unrelated to the language): Nix via [devenv](https://devenv.sh) for a reproducible toolchain, [moon](https://moonrepo.dev) as the task runner, [treefmt](https://github.com/numtide/treefmt) for formatting, [prek](https://github.com/j178/prek) for git hooks (pre-push runs the full moon pipeline; commit-msg enforces conventional commits), `cargo-deny`/`cargo-audit` for dependency hygiene.

```bash
devenv shell              # enters the dev shell; installs hooks, syncs moon, warms sccache
moon run :format :check :lint :build :test :audit   # the full local gate, same as pre-push
moon run :coverage        # cargo-llvm-cov under nextest
```

`devenv.nix` / `devenv.yaml` define the shell; `moon.yml` defines the tasks; `treefmt.toml` / `rustfmt.toml` define formatting; `deny.toml` / `cargo-audit.toml` define dependency policy. `flake.nix` is separate — it's the *package* (what `nix build`/`nix run` install), not the dev shell.
