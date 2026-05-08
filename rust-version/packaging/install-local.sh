#!/usr/bin/env bash
set -euo pipefail

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NAME="codex-rg-guard"
DEST="${CODEX_HOME:-${HOME}/.codex}/plugins/${NAME}"
MARKET="${HOME}/.agents/plugins/marketplace.json"

if [[ ! -x "${SRC}/bin/cxs-mcp-server" ]]; then
  echo "missing executable: ${SRC}/bin/cxs-mcp-server" >&2
  exit 1
fi

if ! command -v rg >/dev/null 2>&1; then
  echo "ripgrep is required: install rg and rerun this script" >&2
  exit 1
fi

mkdir -p "$(dirname "${DEST}")"
rm -rf "${DEST}"
cp -R "${SRC}" "${DEST}"
chmod +x "${DEST}/bin/cxs" "${DEST}/bin/cxs-mcp-server" "${DEST}/bin/rg"

if command -v python3 >/dev/null 2>&1; then
  mkdir -p "$(dirname "${MARKET}")"
  python3 - "${MARKET}" <<'PY'
import json
import sys
from pathlib import Path

p = Path(sys.argv[1]).expanduser()
p.parent.mkdir(parents=True, exist_ok=True)
try:
    data = json.loads(p.read_text()) if p.exists() else {}
except Exception:
    try:
        p.with_suffix(p.suffix + ".bak").write_text(p.read_text(errors="replace"))
    except Exception:
        pass
    data = {}
if not isinstance(data, dict):
    data = {}
data.setdefault("name", "local-personal-plugins")
data.setdefault("interface", {"displayName": "Local Personal Plugins"})
plugins = [x for x in data.get("plugins", []) if x.get("name") != "codex-rg-guard"]
plugins.append({
    "name": "codex-rg-guard",
    "source": {"source": "local", "path": "./.codex/plugins/codex-rg-guard"},
    "policy": {"installation": "AVAILABLE", "authentication": "ON_INSTALL"},
    "category": "Productivity",
})
data["plugins"] = plugins
p.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n")
print(p)
PY
else
  echo "python3 not found; skipped marketplace.json update"
fi

cat <<EOF
Installed Rust plugin: ${DEST}

Restart Codex and enable "Codex rg Guard".

Fallback direct MCP registration:
  codex mcp add cxs-rg-guard -- ${DEST}/bin/cxs-mcp-server

Optional shell tools:
  export PATH="${DEST}/bin:\$PATH"
EOF
