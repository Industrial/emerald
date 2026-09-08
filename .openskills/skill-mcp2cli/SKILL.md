---
description: "Use when interacting with an MCP server, OpenAPI/REST API, or GraphQL API from the terminal—list tools, call operations, bake configs, or scaffold a new skill from an API. Triggers: mcp2cli, call MCP from CLI, list tools from server, OpenAPI/GraphQL without hand-written clients."
alwaysApply: false
---

# mcp2cli

Turn any MCP server, OpenAPI spec, or GraphQL endpoint into a **CLI at runtime** (no codegen). Project: [knowsuchagency/mcp2cli](https://github.com/knowsuchagency/mcp2cli).

## This monorepo

- **`uv` is on PATH in devenv.** Prefer **`devenv shell -- uvx mcp2cli ...`** for every invocation (see `.cursor/rules/shell.mdc`). Do not assume a bare shell has `uvx`.
- When generating a skill for an API, put wrappers under **`.openskills/skill-<name>/scripts/`** (or `history/` for design notes), not `.claude/skills/`.

## Install / verify

```bash
devenv shell -- uvx mcp2cli --help
```

Optional global install (user machine): `uv tool install mcp2cli` — still use `devenv shell --` here when the task runs through project automation.

## Core workflow

1. **Connect** — pick exactly one source: `--mcp`, `--mcp-stdio`, `--spec`, or `--graphql`.
2. **Discover** — `--list` or `--search PATTERN` (search implies list).
3. **Inspect** — `<subcommand> --help`.
4. **Execute** — subcommand + flags; add `--pretty`, `--jq`, `--head`, `--toon`, or `--raw` as needed.

### MCP (HTTP/SSE)

```bash
devenv shell -- uvx mcp2cli --mcp https://mcp.example.com/sse --list
devenv shell -- uvx mcp2cli --mcp https://mcp.example.com/sse --transport sse --list
devenv shell -- uvx mcp2cli --mcp https://mcp.example.com/sse some-tool --help
```

### MCP (stdio)

```bash
devenv shell -- uvx mcp2cli --mcp-stdio "npx @modelcontextprotocol/server-filesystem /tmp" --list
```

### OpenAPI

```bash
devenv shell -- uvx mcp2cli --spec https://petstore3.swagger.io/api/v3/openapi.json --list
devenv shell -- uvx mcp2cli --spec ./openapi.json --base-url https://api.example.com list-pets --status available
```

### GraphQL

```bash
devenv shell -- uvx mcp2cli --graphql https://api.example.com/graphql --list
devenv shell -- uvx mcp2cli --graphql https://api.example.com/graphql users --limit 10 --fields "id name email"
```

## Authentication

```bash
# Header (prefer env:/file: for secrets — avoids argv exposure)
devenv shell -- uvx mcp2cli --mcp https://mcp.example.com/sse \
  --auth-header "Authorization:env:MY_API_TOKEN" --list

# OAuth (browser PKCE or client credentials — tokens cached under ~/.cache/mcp2cli/oauth/)
devenv shell -- uvx mcp2cli --spec https://api.example.com/openapi.json --oauth --list
```

## High-signal flags

| Flag | Use |
|------|-----|
| `--refresh` | Bypass cache |
| `--cache-ttl SECONDS` | TTL for cached specs / tool lists |
| `--jq EXPR` | Filter JSON (prefer over ad-hoc Python) |
| `--head N` | Truncate large arrays |
| `--toon` | Token-efficient encoding for LLM-sized payloads |
| `--stdin` | POST JSON body from stdin (OpenAPI) |

## Bake mode — reuse connections

Saves connection + auth profile as a named config (`~/.config/mcp2cli/baked.json`, override with `MCP2CLI_CONFIG_DIR`):

```bash
devenv shell -- uvx mcp2cli bake create petstore --spec https://api.example.com/spec.json \
  --exclude "delete-*,update-*" --methods GET,POST

devenv shell -- uvx mcp2cli @petstore --list
devenv shell -- uvx mcp2cli bake install petstore --dir ./scripts/
```

## Generating a skill from an API (for this repo)

1. Discover: `devenv shell -- uvx mcp2cli --mcp … --list` (or `--spec` / `--graphql`).
2. Inspect: `devenv shell -- uvx mcp2cli … <cmd> --help`.
3. Probe edge cases: pagination, huge fields (use `--head`), date formats, errors.
4. Optional bake: `devenv shell -- uvx mcp2cli bake create myapi …`
5. Install wrapper into **`.openskills/skill-myapi/scripts/`** if you want a stable path for docs.
6. Add **`skill-myapi`** to `AGENTS.md` (PRPM block) and a short `SKILL.md` that documents **gotchas and workflows**, not a copy of `--help`.

**Knowledge delta:** document defaults, failure modes, and parameter combos that matter in practice.

## Full CLI reference

```
mcp2cli [global options] <subcommand> [command options]

Source (one required):
  --spec URL|FILE       OpenAPI (JSON/YAML)
  --mcp URL             MCP HTTP
  --mcp-stdio CMD       MCP stdio
  --graphql URL         GraphQL

Notable options:
  --auth-header K:V     repeatable; value: env:VAR / file:PATH
  --base-url URL
  --transport auto|sse|streamable
  --env KEY=VALUE       for stdio server
  --oauth, --oauth-client-id, --oauth-client-secret, --oauth-scope
  --list, --search PATTERN
  --pretty, --raw, --toon, --jq, --head
  bake create|list|show|update|remove|install
  @NAME                 run baked config
```

Subcommands are **dynamic** from the source—always use `--list` and `<cmd> --help` when unsure.
