#!/usr/bin/env bash
# Postgres MCP: connection URL from .env (not committed in mcp.json).
#
# Env (first match wins):
#   MCP_POSTGRES_URL — optional override for MCP only
#   DATABASE_URL — default for this repo
#
# Usage from repo root: ./scripts/postgres-mcp-wrapper.sh
# Cursor: "command": "./scripts/postgres-mcp-wrapper.sh"

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [[ -f "$REPO_ROOT/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "$REPO_ROOT/.env"
    set +a
fi

URL="${MCP_POSTGRES_URL:-${DATABASE_URL:-}}"
if [[ -z "$URL" ]]; then
    echo "postgres-mcp-wrapper: set MCP_POSTGRES_URL or DATABASE_URL in .env" >&2
    exit 1
fi

cd "$REPO_ROOT"
exec npx -y @modelcontextprotocol/server-postgres@latest "$URL"
