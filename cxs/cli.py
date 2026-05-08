from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any, Dict, List

from . import core


def add_common(p: argparse.ArgumentParser) -> None:
    p.add_argument("--root")
    p.add_argument(
        "--scope",
        dest="scopes",
        action="append",
        choices=sorted(core.SCOPE_INCLUDES),
        help="Preset search scope. Use --path for concrete files/directories.",
    )
    p.add_argument("--path", dest="paths", action="append")
    p.add_argument("-g", "--glob", dest="include_globs", action="append")
    p.add_argument("--exclude", dest="exclude_globs", action="append")
    p.add_argument("--case-sensitive", action="store_true")


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="cxs")
    p.add_argument("--pretty", action="store_true")
    sub = p.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("find")
    f.add_argument("query")
    f.add_argument("--term", dest="terms", action="append")
    f.add_argument("--match", choices=["any", "all"], default="any")
    f.add_argument("--regex", action="store_true")
    f.add_argument("--max-hits", "--max-total-hits", dest="max_hits", type=int)
    f.add_argument("--per-file", "--max-hits-per-file", dest="per_file", type=int)
    f.add_argument("--line-chars", "--max-line-chars", dest="line_chars", type=int)
    f.add_argument("--max-total-chars", type=int)
    f.add_argument("--timeout", "--timeout-seconds", dest="timeout", type=float)
    f.add_argument("--files-only", action="store_true")
    f.add_argument("--offset", type=int, default=0)
    add_common(f)
    fs = sub.add_parser("files")
    fs.add_argument("query", nargs="?")
    fs.add_argument("--regex", action="store_true")
    fs.add_argument("--max-files", type=int)
    fs.add_argument("--timeout", "--timeout-seconds", dest="timeout", type=float)
    add_common(fs)
    sym = sub.add_parser("symbol")
    sym.add_argument("name")
    sym.add_argument("--max-hits", "--max-total-hits", dest="max_hits", type=int)
    sym.add_argument("--timeout", "--timeout-seconds", dest="timeout", type=float)
    add_common(sym)
    js = sub.add_parser("json")
    js.add_argument("query", nargs="?")
    js.add_argument("--term", dest="terms", action="append")
    js.add_argument("--filter", dest="filters", action="append", help="key=value")
    js.add_argument("--field", dest="fields", action="append")
    js.add_argument("--omit-field", dest="omit_fields", action="append")
    js.add_argument("--match", choices=["any", "all"], default="any")
    js.add_argument("--search-large-fields", action="store_true")
    js.add_argument("--allow-large-files", action="store_true")
    js.add_argument("--max-records", type=int)
    js.add_argument("--max-files", type=int)
    js.add_argument("--timeout", "--timeout-seconds", dest="timeout", type=float)
    add_common(js)
    sc = sub.add_parser("self-check")
    sc.add_argument("--root")
    return p


def parse_filters(items: Any) -> Dict[str, str]:
    out: Dict[str, str] = {}
    for item in items or []:
        if "=" not in item:
            raise SystemExit(f"bad --filter, expected key=value: {item}")
        k, v = item.split("=", 1)
        out[k] = v
    return out


def main(argv=None) -> int:
    raw = list(sys.argv[1:] if argv is None else argv)
    pretty = False
    if "--pretty" in raw:
        pretty = True
        raw = [a for a in raw if a != "--pretty"]
    ns = build_parser().parse_args(raw)
    if ns.cmd == "find":
        res = core.cxs_find(query=ns.query, terms=ns.terms, match=ns.match, mode="regex" if ns.regex else "literal", root=ns.root, paths=ns.paths, scopes=ns.scopes, include_globs=ns.include_globs, exclude_globs=ns.exclude_globs, case_sensitive=ns.case_sensitive, max_total_hits=ns.max_hits, max_hits_per_file=ns.per_file, max_line_chars=ns.line_chars, max_total_chars=ns.max_total_chars, timeout_seconds=ns.timeout, files_only=ns.files_only, offset=ns.offset)
    elif ns.cmd == "files":
        res = core.cxs_files(query=ns.query, mode="regex" if ns.regex else "literal", root=ns.root, paths=ns.paths, scopes=ns.scopes, include_globs=ns.include_globs, exclude_globs=ns.exclude_globs, case_sensitive=ns.case_sensitive, max_files=ns.max_files, timeout_seconds=ns.timeout)
    elif ns.cmd == "symbol":
        res = core.cxs_symbol(ns.name, root=ns.root, paths=ns.paths, scopes=ns.scopes, max_total_hits=ns.max_hits, timeout_seconds=ns.timeout)
    elif ns.cmd == "json":
        res = core.cxs_json(query=ns.query, terms=ns.terms, filters=parse_filters(ns.filters), fields=ns.fields, omit_fields=ns.omit_fields, match=ns.match, root=ns.root, paths=ns.paths, scopes=ns.scopes, include_globs=ns.include_globs, exclude_globs=ns.exclude_globs, case_sensitive=ns.case_sensitive, search_large_fields=ns.search_large_fields, allow_large_files=ns.allow_large_files, max_records=ns.max_records, max_files=ns.max_files, timeout_seconds=ns.timeout)
    else:
        res = core.cxs_self_check(ns.root)
    print(core.json_dumps(res, pretty=pretty))
    return 1 if res.get("status") == "error" else 0


def shim_rg_main(argv=None) -> int:
    args = list(sys.argv[1:] if argv is None else argv)
    if os.environ.get("CXS_RAW_RG") or os.environ.get("RG_RAW"):
        real = core.find_rg()
        if not real:
            print("rg not found", file=sys.stderr)
            return 127
        return subprocess.call([real] + args)
    # Intercept common Codex pattern: rg -n PATTERN [PATH...]
    if "--files" in args or "-l" in args or "--files-with-matches" in args:
        real = core.find_rg()
        return subprocess.call([real] + args) if real else 127
    numbered = "-n" in args or any(a.startswith("-n") for a in args) or "--line-number" in args
    if numbered:
        pats: List[str] = []
        paths: List[str] = []
        mode = "regex"
        i = 0
        while i < len(args):
            a = args[i]
            if a in ("-n", "--line-number", "--no-heading", "--color=never", "-S", "--smart-case"):
                i += 1; continue
            if a in ("-F", "--fixed-strings"):
                mode = "literal"; i += 1; continue
            if a in ("-e", "--regexp") and i + 1 < len(args):
                pats.append(args[i+1]); i += 2; continue
            if a.startswith("-"):
                # unsupported option: pass through guarded real rg
                break
            if not pats:
                pats.append(a)
            else:
                paths.append(a)
            i += 1
        else:
            res = core.cxs_find(
                query=pats[0],
                terms=pats[1:],
                mode=mode,
                paths=paths or None,
            )
            print(core.json_dumps(res))
            return 1 if res.get("status") == "error" else 0
    real = core.find_rg()
    if not real:
        print("rg not found", file=sys.stderr)
        return 127
    guard = ["--max-count", str(core.DEFAULT_BUDGET.max_hits_per_file), "--max-columns", str(core.DEFAULT_BUDGET.max_line_chars), "--max-columns-preview"]
    return subprocess.call([real] + guard + args)

if __name__ == "__main__":
    raise SystemExit(main())
