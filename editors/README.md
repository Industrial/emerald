# Emerald editor and agent integration

Plan 17's IDE-integration tooling: a TextMate grammar, an LSP server
(`emerald-lsp`), an MCP server (`emerald-mcp`), and a VSCode extension
wiring the first two together. Per-editor support differs — read the
table below before wiring anything up.

| Editor | Syntax coloring | Live diagnostics |
|---|---|---|
| VSCode | yes (TextMate grammar) | yes (`emerald-lsp`) |
| Neovim | no | yes (`emerald-lsp`) |
| Helix | no | yes (`emerald-lsp`) |
| Zed | no | yes (`emerald-lsp`) |

Neovim/Helix/Zed native highlighting all need a tree-sitter grammar,
which this plan deliberately doesn't build (a second, parallel-
maintained grammar with real drift risk against `grammar.lalrpop` —
see plan 17's own Decision log). All three still get live diagnostics
from `emerald-lsp`, an ordinary LSP server with no tree-sitter
dependency.

## Build the binaries first

Every setup below assumes `emerald-lsp` and `emerald-mcp` are already
built and on `PATH` (or referenced by absolute path):

```bash
devenv shell -- cargo build --release -p emerald-lsp -p emerald-mcp
# binaries land at target/release/emerald-lsp and target/release/emerald-mcp
```

## VSCode

`editors/vscode/` — a TextMate grammar (`syntaxes/emerald.tmLanguage.json`)
plus a client (`extension.js`) that spawns `emerald-lsp` via
`vscode-languageclient`.

This sandbox has no `npm`/`yarn`/`pnpm` on `PATH`, so installing
`vscode-languageclient` and launching a real Extension Development Host
is a disclosed manual step, not something this repo automates:

```bash
cd editors/vscode
npm install                 # pulls in vscode-languageclient (package.json)
# then open this directory in VSCode and press F5
```

The extension's `emerald.serverPath` setting (defaults to `emerald-lsp`
on `PATH`) points VSCode at the server binary if it isn't on `PATH`.

Automated, no-npm-required checks for this directory:

```bash
node editors/vscode/check-grammar.mjs    # grammar tokenizes every examples/**/*.em file
node editors/vscode/check-manifest.mjs   # package.json and the grammar file agree
node --check editors/vscode/extension.js # syntax-only check
```

## Neovim (`nvim-lspconfig`)

Neovim has no per-language extension model — one generic LSP server
registration plus filetype detection is everything it needs:

```lua
vim.filetype.add({ extension = { em = "emerald" } })

vim.api.nvim_create_autocmd("FileType", {
  pattern = "emerald",
  callback = function(args)
    vim.lsp.start({
      name = "emerald-lsp",
      cmd = { "emerald-lsp" }, -- or an absolute path to the built binary
      root_dir = vim.fs.dirname(vim.fs.find({ "emerald.toml", ".git" }, { upward = true })[1]),
    })
  end,
})
```

## Helix

Add to `languages.toml` (project-local or `~/.config/helix/languages.toml`):

```toml
[[language]]
name = "emerald"
scope = "source.emerald"
file-types = ["em"]
roots = ["emerald.toml"]
language-servers = ["emerald-lsp"]

[language-server.emerald-lsp]
command = "emerald-lsp"
```

## Zed

Zed only consumes tree-sitter grammars for native highlighting, which
this plan doesn't provide (see the table above) — Zed can still be
pointed at `emerald-lsp` as a generic LSP server for diagnostics
through Zed's own custom-language-server configuration; `.em` files
render as plain text otherwise.

## MCP clients (Claude Code, Claude Desktop, etc.)

`emerald-mcp` is a stdio MCP server exposing three tools:
`check_source`, `compile_and_run`, and `list_examples` (see
`crates/emerald-mcp/src/main.rs`). Register it as a local stdio server,
e.g. in a `.mcp.json`:

```json
{
  "mcpServers": {
    "emerald": {
      "command": "emerald-mcp"
    }
  }
}
```

or the equivalent Claude Desktop `claude_desktop_config.json` entry
under `mcpServers`. No arguments or environment variables are required
— `emerald-mcp` embeds the `examples/*.em` corpus at compile time and
needs no repo checkout present at runtime.
