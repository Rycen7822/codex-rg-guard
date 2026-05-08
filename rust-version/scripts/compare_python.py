#!/usr/bin/env python3
"""Semantic compatibility check between Python cxs and the Rust MCP binary.

The Rust self_check output intentionally reports a Rust runtime instead of the
Python interpreter. For search operations this script compares the returned
JSON after dropping runtime-only counters such as subprocess byte counts.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
RUST_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from cxs import core  # noqa: E402


def make_fixture(root: Path) -> None:
    (root / "src").mkdir()
    (root / "docs").mkdir()
    (root / "data").mkdir()
    (root / "resources" / "code" / "github").mkdir(parents=True)
    (root / "experiments" / "runs" / "r1").mkdir(parents=True)
    (root / "src" / "app.py").write_text(
        "def train_loop(x):\n    return x\n\nclass Trainer:\n    pass\n",
        encoding="utf-8",
    )
    (root / "src" / "multi.py").write_text("alpha\nbeta\n", encoding="utf-8")
    (root / "src" / "one.py").write_text("alpha\n", encoding="utf-8")
    (root / "docs" / "README.md").write_text(
        "hello\nneedle " + "x" * 1000 + "\n",
        encoding="utf-8",
    )
    (root / "resources" / "code" / "github" / "vendor.txt").write_text(
        "needle vendor\n",
        encoding="utf-8",
    )
    with (root / "data" / "records.jsonl").open("w", encoding="utf-8") as f:
        f.write(json.dumps({"doc_id": "doc-1", "token_count": 12, "text": "A" * 10000, "kind": "summary"}) + "\n")
        f.write(json.dumps({"doc_id": "doc-2", "token_count": 99, "text": "needle hidden in large text", "kind": "raw"}) + "\n")
    (root / "experiments" / "runs" / "r1" / "metrics.jsonl").write_text(
        json.dumps({"run_id": "r1", "acc": 0.9}) + "\n",
        encoding="utf-8",
    )


def rust_call(server: Path, op: str, args: dict[str, Any]) -> Any:
    proc = subprocess.Popen(
        [str(server)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    assert proc.stdin and proc.stdout
    proc.stdin.write(
        json.dumps(
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {"name": "cxs", "arguments": {"op": op, "args": args}},
            }
        )
        + "\n"
    )
    proc.stdin.flush()
    line = proc.stdout.readline()
    proc.stdin.close()
    proc.terminate()
    try:
        proc.wait(timeout=2)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=2)
    resp = json.loads(line)
    return json.loads(resp["result"]["content"][0]["text"])


def normalize(x: Any) -> Any:
    if isinstance(x, dict):
        out = {}
        for k, v in x.items():
            if k == "bytes":
                continue
            if k in {"runtime", "python", "rust"}:
                continue
            if k == "rg":
                continue
            if k == "rg_version":
                continue
            out[k] = normalize(v)
        return out
    if isinstance(x, list):
        return [normalize(v) for v in x]
    return x


def python_call(op: str, args: dict[str, Any]) -> Any:
    if op == "find":
        return core.cxs_find(**args)
    if op == "files":
        return core.cxs_files(**args)
    if op == "symbol":
        return core.cxs_symbol(**args)
    if op == "json":
        return core.cxs_json(**args)
    if op == "self_check":
        return core.cxs_self_check(**args)
    raise AssertionError(op)


def main() -> int:
    subprocess.run(["cargo", "build", "--quiet", "--bins"], cwd=RUST_ROOT, check=True)
    server = RUST_ROOT / "target" / "debug" / "cxs-mcp-server-rs"
    cases: list[tuple[str, dict[str, Any]]] = []
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        make_fixture(root)
        cases.extend(
            [
                ("find", {"query": "needle", "root": str(root), "scopes": ["docs"], "max_line_chars": 60}),
                ("find", {"query": "needle", "root": str(root), "scopes": ["docs"], "files_only": True}),
                ("find", {"terms": ["alpha", "beta"], "match": "all", "root": str(root), "scopes": ["src"], "files_only": True}),
                ("files", {"query": "app", "root": str(root), "scopes": ["src"]}),
                ("symbol", {"name": "train_loop", "root": str(root)}),
                ("json", {"query": "needle hidden", "root": str(root), "paths": ["data/records.jsonl"]}),
                ("json", {"query": "needle hidden", "root": str(root), "paths": ["data/records.jsonl"], "search_large_fields": True}),
                ("json", {"filters": {"doc_id": "doc-1"}, "fields": ["doc_id", "token_count", "text"], "root": str(root), "paths": ["data/records.jsonl"]}),
                ("json", {"query": "r1", "root": str(root), "scopes": ["runs"]}),
                ("self_check", {"root": str(root)}),
            ]
        )
        for op, args in cases:
            py = normalize(python_call(op, args))
            rs = normalize(rust_call(server, op, args))
            if py != rs:
                print(f"COMPAT_FAIL op={op} args={args}", file=sys.stderr)
                print("python:", json.dumps(py, ensure_ascii=False, indent=2), file=sys.stderr)
                print("rust:", json.dumps(rs, ensure_ascii=False, indent=2), file=sys.stderr)
                return 1
    print("COMPAT_OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
