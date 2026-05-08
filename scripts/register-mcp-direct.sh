#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
codex mcp add cxs-rg-guard -- python3 "$ROOT/mcp/cxs_mcp_server.py"
