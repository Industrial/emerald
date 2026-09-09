# Emerald VSCode extension

Syntax coloring for `.em` files (`syntaxes/emerald.tmLanguage.json`) plus live
diagnostics via `emerald-lsp` (`extension.js`).

## What works today

- Syntax highlighting for the current `grammar.lalrpop` surface (keywords,
  operators, literals, instance variables) — verified against every file in
  `../../examples/*.em` by `check-grammar.mjs`.
- Live parse/type-check diagnostics, published on `didOpen`/`didChange`, via
  `emerald-lsp` over stdio.

## What's disclosed as not done

- No semantic tokens, go-to-definition, completion, rename, or formatting —
  `emerald-sema` doesn't expose a symbol table yet (see plan 17's Decision
  log).
- Sema diagnostics are anchored at the whole document, not a precise range —
  `emerald_sema::Diagnostic` carries no span (a pre-existing gap, plan 13).
- Full-document sync only, no incremental diffing — there is no incremental
  compiler to sync against yet.

## Setup (manual — this sandbox has no npm/yarn/pnpm installed)

1. Build `emerald-lsp`: `devenv shell -- cargo build -p emerald-lsp --release`
   from the repo root, then either put `target/release/emerald-lsp` on your
   `PATH`, or set the `emerald.serverPath` VSCode setting to its absolute
   path.
2. From this directory: `npm install` (pulls in `vscode-languageclient`,
   declared in `package.json`).
3. Open this directory in VSCode and press F5 to launch an Extension
   Development Host — `.em` files should get syntax coloring immediately,
   and diagnostics as soon as `emerald-lsp` starts.

## Automated checks (no npm required)

```bash
node check-grammar.mjs    # grammar tokenizes every examples/*.em file
node check-manifest.mjs   # package.json and the grammar file agree
node --check extension.js # syntax-only check (no vscode/vscode-languageclient install needed)
```
