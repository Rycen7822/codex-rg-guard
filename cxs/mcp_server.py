from __future__ import annotations

import json
import sys
import traceback
from typing import Any, Dict, List

from . import core

PROTOCOL_VERSION = "2025-06-18"
SERVER_INFO = {"name": "cxs-rg-guard", "version": core.VERSION}
OPS = {
    "find": core.cxs_find,
    "files": core.cxs_files,
    "symbol": core.cxs_symbol,
    "json": core.cxs_json,
    "self_check": core.cxs_self_check,
}
ALIASES = {
    "cxs_find": ("find", core.cxs_find),
    "cxs_files": ("files", core.cxs_files),
    "cxs_symbol": ("symbol", core.cxs_symbol),
    "cxs_json": ("json", core.cxs_json),
    "cxs_self_check": ("self_check", core.cxs_self_check),
}


def tool_defs() -> List[Dict[str, Any]]:
    # One tool keeps the MCP tool-list context small.
    return [{
        "name": "cxs",
        "description": "Low-context rg search.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "op": {"type": "string", "enum": ["find", "files", "symbol", "json", "self_check"]},
                "args": {"type": "object"}
            },
            "required": ["op"],
            "additionalProperties": False
        }
    }]


def send(obj: Dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(obj, ensure_ascii=False, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def result(req_id: Any, data: Any) -> Dict[str, Any]:
    return {"jsonrpc": "2.0", "id": req_id, "result": data}


def error(req_id: Any, code: int, msg: str, data: Any = None) -> Dict[str, Any]:
    e = {"code": code, "message": msg}
    if data is not None:
        e["data"] = data
    return {"jsonrpc": "2.0", "id": req_id, "error": e}


def call_tool(name: str, arguments: Dict[str, Any]) -> Dict[str, Any]:
    if name == "cxs":
        op = arguments.get("op")
        kwargs = arguments.get("args") or {}
        if op not in OPS:
            return {"status": "error", "error": "bad_op", "valid_ops": list(OPS)}
        if not isinstance(kwargs, dict):
            return {"status": "error", "error": "args_must_be_object"}
        return OPS[op](**kwargs)
    # Compatibility only; not advertised in tools/list.
    if name in ALIASES:
        _, fn = ALIASES[name]
        return fn(**(arguments or {}))
    return {"status": "error", "error": "unknown_tool", "tool": name}


def handle(msg: Dict[str, Any]) -> Dict[str, Any] | None:
    method = msg.get("method")
    rid = msg.get("id")
    params = msg.get("params") or {}
    if method == "initialize":
        requested = params.get("protocolVersion") or PROTOCOL_VERSION
        proto = requested if requested in {"2025-06-18", "2025-03-26", "2024-11-05"} else PROTOCOL_VERSION
        return result(rid, {"protocolVersion": proto, "capabilities": {"tools": {"listChanged": False}}, "serverInfo": SERVER_INFO, "instructions": "Broad content search: find(files_only:true), then find(paths) for bounded line hits; read exact files or spans outside cxs only after a concrete file/line is known."})
    if method == "notifications/initialized":
        return None
    if method == "ping":
        return result(rid, {})
    if method == "tools/list":
        return result(rid, {"tools": tool_defs()})
    if method == "tools/call":
        name = params.get("name")
        args = params.get("arguments") or {}
        try:
            data = call_tool(name, args)
            return result(rid, {"content": [{"type": "text", "text": core.json_dumps(data)}], "isError": data.get("status") == "error"})
        except Exception as exc:
            data = {"status": "error", "error": exc.__class__.__name__, "message": str(exc), "trace": traceback.format_exc(limit=core.DEFAULT_BUDGET.mcp_traceback_limit)}
            return result(rid, {"content": [{"type": "text", "text": core.json_dumps(data)}], "isError": True})
    if rid is None:
        return None
    return error(rid, -32601, f"method not found: {method}")


def main() -> int:
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except Exception as exc:
            send(error(None, -32700, "parse error", str(exc)))
            continue
        resp = handle(msg)
        if resp is not None:
            send(resp)
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
