#!/usr/bin/env bash
set -euo pipefail
SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAME="codex-rg-guard"
DEST="${HOME}/.codex/plugins/${NAME}"
MARKET="${HOME}/.agents/plugins/marketplace.json"
mkdir -p "$(dirname "$DEST")" "$(dirname "$MARKET")"
rm -rf "$DEST"
cp -R "$SRC" "$DEST"
chmod +x "$DEST/bin/cxs" "$DEST/bin/cxs-mcp-server" "$DEST/bin/rg" "$DEST/mcp/cxs_mcp_server.py"
python3 - "$MARKET" <<'PY'
import json, sys
from pathlib import Path
p = Path(sys.argv[1]).expanduser()
p.parent.mkdir(parents=True, exist_ok=True)
try:
    data = json.loads(p.read_text()) if p.exists() else {}
except Exception:
    p.with_suffix(p.suffix + '.bak').write_text(p.read_text(errors='replace'))
    data = {}
if not isinstance(data, dict): data = {}
data.setdefault('name', 'local-personal-plugins')
data.setdefault('interface', {'displayName': 'Local Personal Plugins'})
plugins = [x for x in data.get('plugins', []) if x.get('name') != 'codex-rg-guard']
plugins.append({'name':'codex-rg-guard','source':{'source':'local','path':'./.codex/plugins/codex-rg-guard'},'policy':{'installation':'AVAILABLE','authentication':'ON_INSTALL'},'category':'Productivity'})
data['plugins'] = plugins
p.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')
print(p)
PY
cat <<EOF
Installed: $DEST
Updated:   $MARKET
Restart Codex and enable "Codex rg Guard".
Fallback:  codex mcp add cxs-rg-guard -- python3 $DEST/mcp/cxs_mcp_server.py
Optional:  export PATH="$DEST/bin:\$PATH"
EOF
