from __future__ import annotations

import csv
import fnmatch
import io
import json
import os
import re
import selectors
import shutil
import signal
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

VERSION = "0.2.4"

DEFAULT_SCOPES = ["docs", "src", "tests", "config", "analysis"]
DEFAULT_EXCLUDES = [
    "**/.git/**",
    "**/.hg/**",
    "**/.svn/**",
    "**/.venv/**",
    "**/venv/**",
    "**/node_modules/**",
    "**/__pycache__/**",
    "**/resources/code/github/**",
    "**/resources/cache/**",
    "**/resources/models/**",
    "**/experiments/runs/**",
    "**/runs/**",
    "**/checkpoints/**",
    "**/wandb/**",
    "**/*.pt",
    "**/*.pth",
    "**/*.safetensors",
    "**/*.onnx",
    "**/*.bin",
    "**/*.gguf",
    "**/*.parquet",
    "**/*.arrow",
    "**/*.npz",
    "**/*.npy",
    "**/*.pkl",
    "**/*.pickle",
    "**/*.zip",
    "**/*.tar",
    "**/*.tar.gz",
    "**/*.tgz",
    "**/*.7z",
    "**/uv.lock",
    "**/package-lock.json",
    "**/pnpm-lock.yaml",
    "**/yarn.lock",
]
SCOPE_INCLUDES = {
    "docs": [
        "**/*.md",
        "**/*.mdx",
        "**/*.txt",
        "**/*.rst",
        "**/*.adoc",
        "AGENTS.md",
        "README*",
        "CHANGELOG*",
        "TODO*",
        "NOTES*",
    ],
    "src": [
        "**/*.py",
        "**/*.pyi",
        "**/*.rs",
        "**/*.go",
        "**/*.js",
        "**/*.jsx",
        "**/*.ts",
        "**/*.tsx",
        "**/*.java",
        "**/*.kt",
        "**/*.c",
        "**/*.cc",
        "**/*.cpp",
        "**/*.h",
        "**/*.hpp",
        "**/*.cs",
        "**/*.rb",
        "**/*.php",
        "**/*.swift",
        "**/*.m",
        "**/*.scala",
        "**/*.sh",
        "**/*.bash",
        "**/*.zsh",
        "**/*.fish",
        "**/*.pl",
        "**/*.r",
        "**/*.R",
        "**/*.jl",
        "**/*.lua",
    ],
    "tests": ["**/test/**", "**/tests/**", "**/*test*.*", "**/*spec*.*"],
    "config": [
        "**/*.toml",
        "**/*.yaml",
        "**/*.yml",
        "**/*.json",
        "**/*.jsonl",
        "**/*.ini",
        "**/*.cfg",
        "**/*.conf",
        "**/*.env",
        "**/Dockerfile",
        "**/Makefile",
        "**/*.mk",
    ],
    "analysis": [
        "**/analysis/**",
        "**/reports/**",
        "**/summaries/**",
        "**/*.csv",
        "**/*.tsv",
    ],
    "runs": [
        "**/experiments/runs/**",
        "**/runs/**",
        "**/artifacts/**",
        "**/outputs/**",
    ],
    "vendor": ["**/resources/code/github/**", "**/vendor/**", "**/third_party/**"],
    "all": ["**/*"],
}
DATA_GLOBS = ["**/*.json", "**/*.jsonl", "**/*.ndjson", "**/*.csv", "**/*.tsv"]
LARGE_FIELD_PAT = re.compile(
    r"(?:^|_)(text|raw|content|prompt|completion|response|body|html|markdown|tokens?)(?:_|$)",
    re.I,
)


@dataclass
class ProcResult:
    lines: List[bytes]
    returncode: int
    stderr: str
    timed_out: bool
    byte_limit_hit: bool
    line_limit_hit: bool
    bytes_read: int


@dataclass
class Budget:
    # Public defaults. When callers omit explicit arguments, these values drive
    # cxs output size and subprocess runtime.
    max_total_hits: int = 300
    max_hits_per_file: int = 999
    max_line_chars: int = 240
    max_total_chars: int = 12000
    timeout_seconds: float = 5.0
    process_output_bytes: int = 2_000_000

    # Clamp bounds for user-provided budget arguments. Keep these here so the
    # allowed range changes together with the defaults above.
    min_total_hits: int = 1
    max_total_hits_limit: int = 5000
    min_hits_per_file: int = 1
    max_hits_per_file_limit: int = 5000
    min_line_chars: int = 40
    max_line_chars_limit: int = 5000
    min_total_chars: int = 1000
    max_total_chars_limit: int = 500000
    min_timeout_seconds: float = 0.2
    max_timeout_seconds: float = 60.0
    max_json_timeout_seconds: float = 120.0
    min_offset: int = 0
    max_offset: int = 1_000_000

    # rg/file-list scan controls. These are intentionally larger than returned
    # hits so cxs can detect truncation and offer a bounded next step.
    max_filesize: str = "5M"
    process_line_multiplier: int = 30
    file_scan_line_multiplier: int = 20
    files_only_min_line_limit: int = 20
    min_files_only_candidate_chars: int = 1000
    files_only_candidate_char_multiplier: int = 20
    next_files_batch_size: int = 10
    next_find_max_total_hits: int = 20
    limited_files_report_limit: int = 20

    # Internal subprocess IO/probe limits.
    read_chunk_bytes: int = 65536
    stderr_capture_bytes: int = 65536
    termination_wait_seconds: float = 0.5
    kill_wait_seconds: float = 1.0
    selector_min_sleep_seconds: float = 0.01
    selector_max_sleep_seconds: float = 0.1
    match_all_probe_output_bytes: int = 65536
    match_all_probe_lines: int = 2

    # Structured-data search controls.
    max_records_examined: int = 200000
    max_records_examined_limit: int = 5_000_000
    min_file_bytes: int = 1
    max_file_bytes: int = 64_000_000
    max_file_bytes_limit: int = 1_000_000_000
    json_large_list_items: int = 50
    json_large_scalar_skip_chars: int = 2000
    json_flatten_value_chars: int = 5000

    # Miscellaneous bounded diagnostic output.
    stderr_chars: int = 1000
    mcp_traceback_limit: int = 3
    self_check_rg_timeout_seconds: float = 3.0


DEFAULT_BUDGET = Budget()


def json_dumps(obj: Any, pretty: bool = False) -> str:
    if pretty:
        return json.dumps(obj, ensure_ascii=False, indent=2)
    return json.dumps(obj, ensure_ascii=False, separators=(",", ":"))


def as_list(x: Optional[Any]) -> List[str]:
    if x is None:
        return []
    if isinstance(x, str):
        return [x] if x else []
    return [str(v) for v in x if str(v)]


def root_path(root: Optional[str]) -> Path:
    p = Path(root or os.getcwd()).expanduser().resolve()
    if not p.exists():
        raise ValueError(f"root does not exist: {p}")
    return p


def safe_rel(root: Path, p: Path) -> Optional[str]:
    try:
        rp = p.expanduser().resolve()
        if rp == root:
            return "."
        rel = rp.relative_to(root)
        return str(rel)
    except Exception:
        return None


def normalize_paths(
    root: Path, paths: Optional[Any]
) -> Tuple[List[str], List[Dict[str, str]]]:
    raw = as_list(paths)
    if not raw:
        return ["."], []
    ok: List[str] = []
    bad: List[Dict[str, str]] = []
    for item in raw:
        p = Path(item)
        if not p.is_absolute():
            p = root / p
        rel = safe_rel(root, p)
        if rel is None:
            bad.append({"path": item, "reason": "outside_root"})
        elif not (root / rel).exists():
            bad.append({"path": item, "reason": "missing"})
        else:
            ok.append(rel)
    return ok, bad


def paths_all_files(root: Path, rels: Sequence[str]) -> bool:
    return bool(rels and rels != ["."] and all((root / r).is_file() for r in rels))


def find_rg() -> Optional[str]:
    shim = (
        Path(os.environ.get("CXS_SHIM_PATH", "")).resolve()
        if os.environ.get("CXS_SHIM_PATH")
        else None
    )
    for d in os.environ.get("PATH", "").split(os.pathsep):
        cand = Path(d) / "rg"
        if cand.exists() and os.access(cand, os.X_OK):
            try:
                if shim and cand.resolve() == shim:
                    continue
            except Exception:
                pass
            return str(cand)
    return shutil.which("rg")


def clamp_int(x: Any, lo: int, hi: int, default: int) -> int:
    try:
        n = int(x)
    except Exception:
        return default
    return max(lo, min(hi, n))


def clamp_float(x: Any, lo: float, hi: float, default: float) -> float:
    try:
        n = float(x)
    except Exception:
        return default
    return max(lo, min(hi, n))


def clean_rel(s: str) -> str:
    return s[2:] if s.startswith("./") else s


def truncate(s: str, n: int) -> str:
    if len(s) <= n:
        return s
    if n <= 16:
        return s[:n]
    return s[: max(0, n - 14)] + "…[truncated]"


def build_globs(
    scopes: Optional[Any],
    include_globs: Optional[Any] = None,
    exclude_globs: Optional[Any] = None,
    *,
    explicit_paths: bool = False,
    explicit_files: bool = False,
) -> Tuple[List[str], List[str]]:
    requested = as_list(scopes) or list(DEFAULT_SCOPES)
    scope_set = {s.strip().lower() for s in requested if s.strip()}
    includes: List[str] = []
    warnings: List[str] = []
    if not explicit_paths:
        for s in scope_set:
            if s in SCOPE_INCLUDES:
                includes.extend(SCOPE_INCLUDES[s])
            else:
                warnings.append(f"unknown_scope:{s}")
    includes.extend(as_list(include_globs))
    excludes = [] if explicit_files else list(DEFAULT_EXCLUDES)
    if "vendor" in scope_set:
        excludes = [
            g
            for g in excludes
            if "resources/code/github" not in g
            and "vendor" not in g
            and "third_party" not in g
        ]
    if "runs" in scope_set:
        excludes = [
            g for g in excludes if "experiments/runs" not in g and "**/runs/**" not in g
        ]
    excludes.extend(as_list(exclude_globs))
    globs: List[str] = []
    seen = set()
    for g in includes:
        if g and g not in seen:
            globs += ["-g", g]
            seen.add(g)
    for g in excludes:
        gg = g if g.startswith("!") else "!" + g
        if gg not in seen:
            globs += ["-g", gg]
            seen.add(gg)
    return globs, warnings


def run_lines(
    cmd: List[str], cwd: Path, timeout: float, max_bytes: int, max_lines: int
) -> ProcResult:
    proc = subprocess.Popen(
        cmd,
        cwd=str(cwd),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
    )
    assert proc.stdout is not None and proc.stderr is not None
    sel = selectors.DefaultSelector()
    sel.register(proc.stdout, selectors.EVENT_READ, "out")
    sel.register(proc.stderr, selectors.EVENT_READ, "err")
    lines: List[bytes] = []
    err = bytearray()
    buf = bytearray()
    bytes_read = 0
    byte_limit_hit = False
    line_limit_hit = False
    timed_out = False
    deadline = time.monotonic() + timeout
    try:
        while True:
            if time.monotonic() >= deadline:
                timed_out = True
                break
            if len(lines) >= max_lines:
                line_limit_hit = True
                break
            if bytes_read >= max_bytes:
                byte_limit_hit = True
                break
            if proc.poll() is not None:
                # drain a little
                pass
            sleep_for = max(
                DEFAULT_BUDGET.selector_min_sleep_seconds,
                min(
                    DEFAULT_BUDGET.selector_max_sleep_seconds,
                    deadline - time.monotonic(),
                ),
            )
            events = sel.select(sleep_for)
            if not events and proc.poll() is not None:
                break
            for key, _ in events:
                chunk = os.read(key.fileobj.fileno(), DEFAULT_BUDGET.read_chunk_bytes)
                if not chunk:
                    try:
                        sel.unregister(key.fileobj)
                    except Exception:
                        pass
                    continue
                if key.data == "out":
                    bytes_read += len(chunk)
                    buf.extend(chunk)
                    while True:
                        idx = buf.find(b"\n")
                        if idx < 0:
                            break
                        lines.append(bytes(buf[:idx]))
                        del buf[: idx + 1]
                        if len(lines) >= max_lines:
                            line_limit_hit = True
                            break
                else:
                    remaining_err = max(
                        0, DEFAULT_BUDGET.stderr_capture_bytes - len(err)
                    )
                    err.extend(chunk[:remaining_err])
            if proc.poll() is not None and not events:
                break
        if buf and len(lines) < max_lines:
            lines.append(bytes(buf))
    finally:
        if timed_out or byte_limit_hit or line_limit_hit:
            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except Exception:
                proc.terminate()
        try:
            rc = proc.wait(timeout=DEFAULT_BUDGET.termination_wait_seconds)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except Exception:
                proc.kill()
            rc = proc.wait(timeout=DEFAULT_BUDGET.kill_wait_seconds)
        try:
            sel.close()
        except Exception:
            pass
        for st in (proc.stdout, proc.stderr):
            try:
                st.close()
            except Exception:
                pass
    return ProcResult(
        lines,
        rc,
        err.decode("utf-8", "replace"),
        timed_out,
        byte_limit_hit,
        line_limit_hit,
        bytes_read,
    )


def make_base_response(status: str, **kw: Any) -> Dict[str, Any]:
    d = {"status": status}
    d.update(kw)
    return d


def build_rg_files_with_matches_cmd(
    rg: str,
    terms: Sequence[str],
    mode: str,
    search_paths: Sequence[str],
    globs: Sequence[str],
    case_sensitive: Optional[bool],
) -> List[str]:
    cmd = [rg, "-l", "--color=never", "--max-filesize", DEFAULT_BUDGET.max_filesize]
    if case_sensitive is not True:
        cmd += ["-i"]
    cmd += list(globs)
    if mode == "literal":
        cmd += ["-F"]
    for t in terms:
        cmd += ["-e", t]
    cmd += list(search_paths)
    return cmd


def collect_file_hits(
    lines: Iterable[bytes], max_files: int, max_total_chars: int, offset: int = 0
) -> Tuple[List[Dict[str, str]], int, bool, int, bool]:
    files: List[Dict[str, str]] = []
    seen = set()
    total_chars = 0
    truncated = False
    has_more = False
    for raw in lines:
        rel = clean_rel(raw.decode("utf-8", "replace").strip())
        if not rel or rel in seen:
            continue
        seen.add(rel)
        if len(seen) <= offset:
            continue
        add = len(rel) + 16
        if len(files) >= max_files:
            has_more = True
            truncated = True
            break
        if total_chars + add > max_total_chars:
            has_more = True
            truncated = True
            break
        total_chars += add
        files.append({"file": rel})
    return files, total_chars, truncated, len(seen), has_more


def make_find_files_next(
    query: Optional[str],
    term_list: Sequence[str],
    match: str,
    mode: str,
    files: Sequence[Dict[str, str]],
    max_total_hits: int,
    max_hits_per_file: int,
    max_line_chars: int,
    case_sensitive: Optional[bool],
) -> List[Dict[str, Any]]:
    if not files:
        return []
    batch_size = DEFAULT_BUDGET.next_files_batch_size
    args: Dict[str, Any] = {
        "paths": [f["file"] for f in files[:batch_size]],
        "max_total_hits": min(DEFAULT_BUDGET.next_find_max_total_hits, max_total_hits),
        "max_hits_per_file": max_hits_per_file,
        "max_line_chars": max_line_chars,
    }
    if query and len(term_list) == 1:
        args["query"] = query
    else:
        args["terms"] = list(term_list)
    if match != "any":
        args["match"] = match
    if mode != "literal":
        args["mode"] = mode
    if case_sensitive is True:
        args["case_sensitive"] = True
    return [{"op": "find", "args": args}]


def make_find_next_meta(
    files: Sequence[Dict[str, str]], batch_size: Optional[int] = None
) -> Dict[str, Any]:
    batch_size = (
        DEFAULT_BUDGET.next_files_batch_size if batch_size is None else batch_size
    )
    total = len(files)
    return {
        "batch_size": batch_size,
        "files_in_next": min(total, batch_size),
        "total_files_in_page": total,
        "remaining_files_in_page": max(0, total - batch_size),
        "next_is_first_batch": total > batch_size,
    }


def cxs_find_files_only(
    rg: str,
    rp: Path,
    query: Optional[str],
    term_list: Sequence[str],
    match: str,
    mode: str,
    search_paths: Sequence[str],
    globs: Sequence[str],
    rejected: Sequence[Dict[str, str]],
    warnings: Sequence[str],
    case_sensitive: Optional[bool],
    max_total_hits: int,
    max_hits_per_file: int,
    max_line_chars: int,
    max_total_chars: int,
    timeout: float,
    offset: int = 0,
) -> Dict[str, Any]:
    line_limit = max(
        DEFAULT_BUDGET.files_only_min_line_limit,
        max_total_hits * DEFAULT_BUDGET.file_scan_line_multiplier,
        offset + max_total_hits + 1,
    )
    stderr = ""
    bytes_read = 0
    truncated = False
    has_more = False
    seen_files = 0
    files: List[Dict[str, str]] = []
    proc: Optional[ProcResult] = None

    if match == "all" and len(term_list) > 1:
        first = max(term_list, key=len)
        rest = [t for t in term_list if t != first]
        proc = run_lines(
            build_rg_files_with_matches_cmd(
                rg, [first], mode, search_paths, globs, case_sensitive
            ),
            rp,
            timeout,
            DEFAULT_BUDGET.process_output_bytes,
            line_limit,
        )
        stderr = proc.stderr
        bytes_read += proc.bytes_read
        if proc.returncode in (0, 1):
            candidates, _, collect_truncated, candidate_count, candidate_has_more = (
                collect_file_hits(
                    proc.lines,
                    line_limit,
                    max(
                        DEFAULT_BUDGET.min_files_only_candidate_chars,
                        max_total_chars
                        * DEFAULT_BUDGET.files_only_candidate_char_multiplier,
                    ),
                )
            )
            truncated = (
                proc.timed_out
                or proc.byte_limit_hit
                or proc.line_limit_hit
                or collect_truncated
            )
            has_more = candidate_has_more
            deadline = time.monotonic() + max(0.0, timeout)
            total_chars = 0
            matched_files = 0
            seen = set()
            for cand in candidates:
                rel = cand["file"]
                if rel in seen:
                    continue
                ok = True
                for term in rest:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        truncated = True
                        ok = False
                        break
                    sub = run_lines(
                        build_rg_files_with_matches_cmd(
                            rg, [term], mode, [rel], [], case_sensitive
                        ),
                        rp,
                        min(remaining, timeout),
                        DEFAULT_BUDGET.match_all_probe_output_bytes,
                        DEFAULT_BUDGET.match_all_probe_lines,
                    )
                    bytes_read += sub.bytes_read
                    if sub.stderr and not stderr:
                        stderr = sub.stderr
                    if sub.returncode not in (0, 1):
                        if not files:
                            proc = sub
                        ok = False
                        break
                    if not sub.lines:
                        ok = False
                        break
                if not ok:
                    continue
                matched_files += 1
                if matched_files <= offset:
                    continue
                add = len(rel) + 16
                if len(files) >= max_total_hits:
                    has_more = True
                    truncated = True
                    break
                if total_chars + add > max_total_chars:
                    has_more = True
                    truncated = True
                    break
                seen.add(rel)
                total_chars += add
                files.append({"file": rel})
            seen_files = (
                matched_files if not has_more else max(matched_files, candidate_count)
            )
    else:
        proc = run_lines(
            build_rg_files_with_matches_cmd(
                rg, term_list, mode, search_paths, globs, case_sensitive
            ),
            rp,
            timeout,
            DEFAULT_BUDGET.process_output_bytes,
            line_limit,
        )
        stderr = proc.stderr
        bytes_read = proc.bytes_read
        files, _, collect_truncated, seen_files, has_more = collect_file_hits(
            proc.lines, max_total_hits, max_total_chars, offset=offset
        )
        truncated = (
            proc.timed_out
            or proc.byte_limit_hit
            or proc.line_limit_hit
            or collect_truncated
        )

    if len(files) >= max_total_hits:
        truncated = True
        has_more = True
    status = "truncated" if truncated else "ok"
    rc = proc.returncode if proc is not None else None
    if rc not in (0, 1, None) and not files:
        status = "error"
    limit_reasons: List[str] = []
    if proc is not None:
        if proc.timed_out:
            limit_reasons.append("timeout")
        if proc.byte_limit_hit:
            limit_reasons.append("process_output_bytes")
        if proc.line_limit_hit:
            limit_reasons.append("process_line_limit")
    if len(files) >= max_total_hits:
        limit_reasons.append("max_total_hits")
    if has_more:
        limit_reasons.append("files_page_has_more")
    if truncated and not limit_reasons:
        limit_reasons.append("files_only_limit")
    nexts = make_find_files_next(
        query,
        term_list,
        match,
        mode,
        files,
        max_total_hits,
        max_hits_per_file,
        max_line_chars,
        case_sensitive,
    )
    out: Dict[str, Any] = {
        "status": status,
        "query": query,
        "terms": list(term_list),
        "match": match,
        "mode": mode,
        "files_only": True,
        "stats": {
            "files": len(files),
            "offset": offset,
            "seen_files": seen_files,
            "has_more": has_more,
            "truncated": truncated,
            "limit_reasons": limit_reasons,
            "rc": rc,
            "bytes": bytes_read,
        },
        "files": files,
    }
    if nexts:
        out["next"] = nexts
        out["next_meta"] = make_find_next_meta(files)
    if has_more:
        out["next_page"] = {
            "offset": offset + len(files),
            "max_total_hits": max_total_hits,
        }
    if rejected:
        out["rejected_paths"] = list(rejected)
    if warnings:
        out["warnings"] = list(warnings)
    if stderr and (status == "error" or warnings):
        out["stderr"] = truncate(stderr, DEFAULT_BUDGET.stderr_chars)
    return out


def cxs_find(
    query: Optional[str] = None,
    terms: Optional[Any] = None,
    match: str = "any",
    mode: str = "literal",
    root: Optional[str] = None,
    paths: Optional[Any] = None,
    scopes: Optional[Any] = None,
    include_globs: Optional[Any] = None,
    exclude_globs: Optional[Any] = None,
    case_sensitive: Optional[bool] = None,
    max_total_hits: Optional[int] = None,
    max_hits_per_file: Optional[int] = None,
    max_line_chars: Optional[int] = None,
    max_total_chars: Optional[int] = None,
    timeout_seconds: Optional[float] = None,
    files_only: bool = False,
    offset: int = 0,
) -> Dict[str, Any]:
    rp = root_path(root)
    rg = find_rg()
    if not rg:
        return make_base_response("error", error="rg_not_found")
    term_list = [query] if query else []
    term_list += as_list(terms)
    term_list = [t for t in term_list if t]
    if not term_list:
        return make_base_response("error", error="empty_query")
    max_hits = clamp_int(
        max_total_hits,
        DEFAULT_BUDGET.min_total_hits,
        DEFAULT_BUDGET.max_total_hits_limit,
        DEFAULT_BUDGET.max_total_hits,
    )
    max_per = clamp_int(
        max_hits_per_file,
        DEFAULT_BUDGET.min_hits_per_file,
        DEFAULT_BUDGET.max_hits_per_file_limit,
        DEFAULT_BUDGET.max_hits_per_file,
    )
    max_line = clamp_int(
        max_line_chars,
        DEFAULT_BUDGET.min_line_chars,
        DEFAULT_BUDGET.max_line_chars_limit,
        DEFAULT_BUDGET.max_line_chars,
    )
    max_chars = clamp_int(
        max_total_chars,
        DEFAULT_BUDGET.min_total_chars,
        DEFAULT_BUDGET.max_total_chars_limit,
        DEFAULT_BUDGET.max_total_chars,
    )
    timeout = clamp_float(
        timeout_seconds,
        DEFAULT_BUDGET.min_timeout_seconds,
        DEFAULT_BUDGET.max_timeout_seconds,
        DEFAULT_BUDGET.timeout_seconds,
    )
    file_offset = clamp_int(
        offset,
        DEFAULT_BUDGET.min_offset,
        DEFAULT_BUDGET.max_offset,
        DEFAULT_BUDGET.min_offset,
    )
    search_paths, rejected = normalize_paths(rp, paths)
    if not search_paths:
        return make_base_response(
            "error", error="no_valid_paths", rejected_paths=rejected
        )
    explicit = bool(as_list(paths))
    explicit_files = paths_all_files(rp, search_paths)
    globs, warnings = build_globs(
        scopes,
        include_globs,
        exclude_globs,
        explicit_paths=explicit,
        explicit_files=explicit_files,
    )
    if files_only:
        return cxs_find_files_only(
            rg,
            rp,
            query,
            term_list,
            match,
            mode,
            search_paths,
            globs,
            rejected,
            warnings,
            case_sensitive,
            max_hits,
            max_per,
            max_line,
            max_chars,
            timeout,
            file_offset,
        )
    cmd = [
        rg,
        "-n",
        "-H",
        "--no-heading",
        "--color=never",
        "--field-match-separator",
        "\t",
        "--max-count",
        str(max_per),
        "--max-columns",
        str(max_line),
        "--max-columns-preview",
        "--max-filesize",
        DEFAULT_BUDGET.max_filesize,
    ]
    if case_sensitive is not True:
        cmd += ["-i"]
    cmd += globs
    if match == "all":
        # Search most selective-looking term first, filter all terms after parsing.
        first = max(term_list, key=len)
        search_terms = [first]
    else:
        search_terms = term_list
    if mode == "literal":
        cmd += ["-F"]
    for t in search_terms:
        cmd += ["-e", t]
    cmd += search_paths
    line_limit = max_hits * DEFAULT_BUDGET.process_line_multiplier
    proc = run_lines(cmd, rp, timeout, DEFAULT_BUDGET.process_output_bytes, line_limit)
    hits: List[Dict[str, Any]] = []
    file_counts: Dict[str, int] = {}
    total_chars = 0
    hit_limit_hit = False
    output_limit_hit = False
    for raw in proc.lines:
        s = raw.decode("utf-8", "replace")
        parts = s.split("\t", 2)
        if len(parts) != 3:
            continue
        file_s, line_s, text = parts
        file_s = clean_rel(file_s)
        if match == "all":
            hay = text if case_sensitive else text.lower()
            needles = term_list if case_sensitive else [t.lower() for t in term_list]
            if mode == "literal":
                if not all(n in hay for n in needles):
                    continue
            else:
                try:
                    flags = 0 if case_sensitive else re.I
                    if not all(re.search(t, text, flags) for t in term_list):
                        continue
                except re.error:
                    return make_base_response(
                        "error", error="bad_regex", query=query, terms=term_list
                    )
        try:
            line = int(line_s)
        except Exception:
            line = None
        if file_counts.get(file_s, 0) >= max_per:
            continue
        snippet = truncate(text.rstrip("\n"), max_line)
        add = len(file_s) + len(snippet) + 32
        if len(hits) >= max_hits:
            hit_limit_hit = True
            break
        if total_chars + add > max_chars:
            output_limit_hit = True
            break
        file_counts[file_s] = file_counts.get(file_s, 0) + 1
        total_chars += add
        hits.append({"file": file_s, "line": line, "snippet": snippet})
    files_at_hit_limit = sorted(
        f for f, count in file_counts.items() if count >= max_per
    )
    limit_reasons: List[str] = []
    if proc.timed_out:
        limit_reasons.append("timeout")
    if proc.byte_limit_hit:
        limit_reasons.append("process_output_bytes")
    if proc.line_limit_hit or len(proc.lines) >= line_limit:
        limit_reasons.append("process_line_limit")
    if hit_limit_hit or len(hits) >= max_hits:
        limit_reasons.append("max_total_hits")
    if output_limit_hit or total_chars >= max_chars:
        limit_reasons.append("max_total_chars")
    if files_at_hit_limit:
        limit_reasons.append("max_hits_per_file")
    truncated = (
        proc.timed_out
        or proc.byte_limit_hit
        or proc.line_limit_hit
        or len(proc.lines) >= line_limit
        or hit_limit_hit
        or len(hits) >= max_hits
        or output_limit_hit
        or total_chars >= max_chars
        or bool(files_at_hit_limit)
    )
    status = "truncated" if truncated else "ok"
    if proc.returncode not in (0, 1) and not hits:
        status = "error"
    out: Dict[str, Any] = {
        "status": status,
        "query": query,
        "terms": term_list,
        "match": match,
        "mode": mode,
        "stats": {
            "hits": len(hits),
            "files": len(file_counts),
            "truncated": truncated,
            "limit_reasons": limit_reasons,
            "files_at_hit_limit": len(files_at_hit_limit),
            "rc": proc.returncode,
            "bytes": proc.bytes_read,
        },
        "hits": hits,
    }
    if files_at_hit_limit:
        out["limited_files"] = [
            {"file": f, "hits": file_counts[f], "reason": "max_hits_per_file"}
            for f in files_at_hit_limit[: DEFAULT_BUDGET.limited_files_report_limit]
        ]
    if rejected:
        out["rejected_paths"] = rejected
    if warnings:
        out["warnings"] = warnings
    if proc.stderr and (status == "error" or warnings):
        out["stderr"] = truncate(proc.stderr, DEFAULT_BUDGET.stderr_chars)
    return out


def cxs_files(
    query: Optional[str] = None,
    mode: str = "literal",
    root: Optional[str] = None,
    paths: Optional[Any] = None,
    scopes: Optional[Any] = None,
    include_globs: Optional[Any] = None,
    exclude_globs: Optional[Any] = None,
    case_sensitive: Optional[bool] = None,
    max_files: Optional[int] = None,
    max_total_chars: Optional[int] = None,
    timeout_seconds: Optional[float] = None,
) -> Dict[str, Any]:
    rp = root_path(root)
    rg = find_rg()
    if not rg:
        return make_base_response("error", error="rg_not_found")
    maxf = clamp_int(
        max_files,
        DEFAULT_BUDGET.min_total_hits,
        DEFAULT_BUDGET.max_total_hits_limit,
        DEFAULT_BUDGET.max_total_hits,
    )
    maxc = clamp_int(
        max_total_chars,
        DEFAULT_BUDGET.min_total_chars,
        DEFAULT_BUDGET.max_total_chars_limit,
        DEFAULT_BUDGET.max_total_chars,
    )
    search_paths, rejected = normalize_paths(rp, paths)
    if not search_paths:
        return make_base_response(
            "error", error="no_valid_paths", rejected_paths=rejected
        )
    explicit = bool(as_list(paths))
    explicit_files = paths_all_files(rp, search_paths)
    globs, warnings = build_globs(
        scopes,
        include_globs,
        exclude_globs,
        explicit_paths=explicit,
        explicit_files=explicit_files,
    )
    cmd = [rg, "--files", "--color=never"] + globs + search_paths
    proc = run_lines(
        cmd,
        rp,
        clamp_float(
            timeout_seconds,
            DEFAULT_BUDGET.min_timeout_seconds,
            DEFAULT_BUDGET.max_timeout_seconds,
            DEFAULT_BUDGET.timeout_seconds,
        ),
        DEFAULT_BUDGET.process_output_bytes,
        maxf * DEFAULT_BUDGET.file_scan_line_multiplier,
    )
    files = []
    total = 0
    pat = query or ""
    flags = 0 if case_sensitive else re.I
    for raw in proc.lines:
        p = clean_rel(raw.decode("utf-8", "replace").strip())
        if not p:
            continue
        ok = True
        if pat:
            if mode == "literal":
                ok = (pat if case_sensitive else pat.lower()) in (
                    p if case_sensitive else p.lower()
                )
            else:
                try:
                    ok = bool(re.search(pat, p, flags))
                except re.error:
                    return make_base_response("error", error="bad_regex", query=query)
        if not ok:
            continue
        if len(files) >= maxf or total + len(p) > maxc:
            break
        total += len(p)
        files.append({"file": p})
    truncated = (
        proc.timed_out
        or proc.byte_limit_hit
        or proc.line_limit_hit
        or len(files) >= maxf
        or total >= maxc
    )
    out: Dict[str, Any] = {
        "status": "truncated" if truncated else "ok",
        "query": query,
        "stats": {"files": len(files), "truncated": truncated, "rc": proc.returncode},
        "files": files,
    }
    if rejected:
        out["rejected_paths"] = rejected
    if warnings:
        out["warnings"] = warnings
    return out


def symbol_pattern(name: str) -> str:
    n = re.escape(name)
    return "|".join(
        [
            rf"^\s*(?:async\s+def|def|class)\s+{n}\b",
            rf"^\s*(?:export\s+)?(?:async\s+)?function\s+{n}\b",
            rf"^\s*(?:export\s+)?(?:const|let|var)\s+{n}\s*[=:]",
            rf"^\s*(?:export\s+)?class\s+{n}\b",
            rf"^\s*(?:pub\s+)?(?:async\s+)?fn\s+{n}\b",
            rf"^\s*(?:pub\s+)?(?:struct|enum|trait|type)\s+{n}\b",
            rf"^\s*func\s+(?:\([^)]*\)\s*)?{n}\b",
            rf"^\s*type\s+{n}\b",
            rf"^\s*(?:public|private|protected|static|final|abstract|internal|open|override|inline|extern|constexpr)\b.*\b{n}\s*(?:\(|[:={{])",
            rf"^\s*{n}\s*\([^)]*\)\s*(?:=>|\{{)",
        ]
    )


def cxs_symbol(
    name: str,
    root: Optional[str] = None,
    paths: Optional[Any] = None,
    scopes: Optional[Any] = None,
    max_total_hits: Optional[int] = None,
    max_hits_per_file: Optional[int] = None,
    timeout_seconds: Optional[float] = None,
) -> Dict[str, Any]:
    if not name or not re.match(r"^[\w.$:-]+$", name):
        return make_base_response("error", error="bad_symbol_name")
    return cxs_find(
        query=symbol_pattern(name),
        match="any",
        mode="regex",
        root=root,
        paths=paths,
        scopes=scopes or ["src", "tests"],
        max_total_hits=max_total_hits,
        max_hits_per_file=max_hits_per_file,
        timeout_seconds=timeout_seconds,
    )


def flatten_text(rec: Any, search_large_fields: bool = False) -> str:
    parts: List[str] = []

    def walk(x: Any, key: str = ""):
        if isinstance(x, dict):
            for k, v in x.items():
                walk(v, str(k))
        elif isinstance(x, list):
            for v in x[: DEFAULT_BUDGET.json_large_list_items]:
                walk(v, key)
        else:
            if not search_large_fields and LARGE_FIELD_PAT.search(key):
                return
            s = str(x)
            if (
                len(s) > DEFAULT_BUDGET.json_large_scalar_skip_chars
                and not search_large_fields
            ):
                return
            parts.append(s[: DEFAULT_BUDGET.json_flatten_value_chars])

    walk(rec)
    return "\n".join(parts)


def project_record(
    rec: Dict[str, Any],
    fields: Optional[Any],
    omit_fields: Optional[Any],
    max_value_chars: int,
) -> Dict[str, Any]:
    field_list = as_list(fields)
    omit = set(as_list(omit_fields))
    if field_list:
        items = [(k, rec.get(k)) for k in field_list if k in rec]
    else:
        items = [(k, v) for k, v in rec.items() if k not in omit]
    out: Dict[str, Any] = {}
    for k, v in items:
        if k in omit:
            continue
        if not field_list and LARGE_FIELD_PAT.search(k):
            out[k] = "[omitted]"
            continue
        if isinstance(v, (dict, list)):
            s = json_dumps(v)
            out[k] = truncate(s, max_value_chars)
        elif isinstance(v, str):
            out[k] = truncate(v, max_value_chars)
        else:
            out[k] = v
    return out


def iter_records(path: Path):
    suffix = path.suffix.lower()
    if suffix in (".jsonl", ".ndjson"):
        with path.open("r", encoding="utf-8", errors="replace") as f:
            for i, line in enumerate(f, 1):
                if not line.strip():
                    continue
                try:
                    rec = json.loads(line)
                    if isinstance(rec, dict):
                        yield i, rec
                except Exception:
                    continue
    elif suffix == ".json":
        try:
            data = json.loads(path.read_text(encoding="utf-8", errors="replace"))
        except Exception:
            return
        if isinstance(data, dict):
            yield 1, data
        elif isinstance(data, list):
            for i, rec in enumerate(data, 1):
                if isinstance(rec, dict):
                    yield i, rec
    elif suffix in (".csv", ".tsv"):
        delim = "\t" if suffix == ".tsv" else ","
        with path.open("r", encoding="utf-8", errors="replace", newline="") as f:
            rdr = csv.DictReader(f, delimiter=delim)
            for i, rec in enumerate(rdr, 2):
                yield i, dict(rec)


def data_file_candidates(
    root: Path,
    paths: Optional[Any],
    scopes: Optional[Any],
    include_globs: Optional[Any],
    exclude_globs: Optional[Any],
    max_files: int,
    timeout_seconds: Optional[float],
) -> Tuple[List[str], Dict[str, Any]]:
    rg = find_rg()
    if not rg:
        return [], {"error": "rg_not_found"}
    search_paths, rejected = normalize_paths(root, paths)
    if not search_paths:
        return [], {"error": "no_valid_paths", "rejected_paths": rejected}
    explicit = bool(as_list(paths))
    explicit_files = paths_all_files(root, search_paths)
    includes = as_list(include_globs) + DATA_GLOBS
    globs, warnings = build_globs(
        scopes or ["config", "analysis"],
        includes,
        exclude_globs,
        explicit_paths=explicit,
        explicit_files=explicit_files,
    )
    cmd = [rg, "--files", "--color=never"] + globs + search_paths
    proc = run_lines(
        cmd,
        root,
        clamp_float(
            timeout_seconds,
            DEFAULT_BUDGET.min_timeout_seconds,
            DEFAULT_BUDGET.max_timeout_seconds,
            DEFAULT_BUDGET.timeout_seconds,
        ),
        DEFAULT_BUDGET.process_output_bytes,
        max_files * DEFAULT_BUDGET.file_scan_line_multiplier,
    )
    out: List[str] = []
    for raw in proc.lines:
        p = clean_rel(raw.decode("utf-8", "replace").strip())
        if not p or Path(p).suffix.lower() not in (
            ".json",
            ".jsonl",
            ".ndjson",
            ".csv",
            ".tsv",
        ):
            continue
        if len(out) >= max_files:
            break
        out.append(p)
    meta = {
        "rejected_paths": rejected,
        "warnings": warnings,
        "truncated": proc.timed_out
        or proc.byte_limit_hit
        or proc.line_limit_hit
        or len(out) >= max_files,
    }
    return out, meta


def cxs_json(
    query: Optional[str] = None,
    terms: Optional[Any] = None,
    filters: Optional[Dict[str, Any]] = None,
    fields: Optional[Any] = None,
    omit_fields: Optional[Any] = None,
    match: str = "any",
    root: Optional[str] = None,
    paths: Optional[Any] = None,
    scopes: Optional[Any] = None,
    include_globs: Optional[Any] = None,
    exclude_globs: Optional[Any] = None,
    case_sensitive: Optional[bool] = None,
    search_large_fields: bool = False,
    allow_large_files: bool = False,
    max_files: Optional[int] = None,
    max_records: Optional[int] = None,
    max_records_examined: Optional[int] = None,
    max_file_bytes: Optional[int] = None,
    max_value_chars: Optional[int] = None,
    max_total_chars: Optional[int] = None,
    timeout_seconds: Optional[float] = None,
) -> Dict[str, Any]:
    rp = root_path(root)
    maxf = clamp_int(
        max_files,
        DEFAULT_BUDGET.min_total_hits,
        DEFAULT_BUDGET.max_total_hits_limit,
        DEFAULT_BUDGET.max_total_hits,
    )
    maxr = clamp_int(
        max_records,
        DEFAULT_BUDGET.min_total_hits,
        DEFAULT_BUDGET.max_total_hits_limit,
        DEFAULT_BUDGET.max_total_hits,
    )
    maxexam = clamp_int(
        max_records_examined,
        DEFAULT_BUDGET.min_total_hits,
        DEFAULT_BUDGET.max_records_examined_limit,
        DEFAULT_BUDGET.max_records_examined,
    )
    maxfile = clamp_int(
        max_file_bytes,
        DEFAULT_BUDGET.min_file_bytes,
        DEFAULT_BUDGET.max_file_bytes_limit,
        DEFAULT_BUDGET.max_file_bytes,
    )
    maxval = clamp_int(
        max_value_chars,
        DEFAULT_BUDGET.min_line_chars,
        DEFAULT_BUDGET.max_line_chars_limit,
        DEFAULT_BUDGET.max_line_chars,
    )
    maxtotal = clamp_int(
        max_total_chars,
        DEFAULT_BUDGET.min_total_chars,
        DEFAULT_BUDGET.max_total_chars_limit,
        DEFAULT_BUDGET.max_total_chars,
    )
    timeout = clamp_float(
        timeout_seconds,
        DEFAULT_BUDGET.min_timeout_seconds,
        DEFAULT_BUDGET.max_json_timeout_seconds,
        DEFAULT_BUDGET.timeout_seconds,
    )
    files, meta = data_file_candidates(
        rp, paths, scopes, include_globs, exclude_globs, maxf, timeout
    )
    if "error" in meta:
        return make_base_response("error", **meta)
    qs = [query] if query else []
    qs += as_list(terms)
    qs = [q for q in qs if q]
    filters = filters or {}
    hits: List[Dict[str, Any]] = []
    examined = 0
    total_chars = 0
    start_time = time.monotonic()
    flags_lower = case_sensitive is not True
    for rel in files:
        if time.monotonic() - start_time > timeout:
            meta["truncated"] = True
            break
        p = rp / rel
        try:
            if not allow_large_files and p.stat().st_size > maxfile:
                continue
        except Exception:
            continue
        for line_no, rec in iter_records(p):
            examined += 1
            if examined > maxexam:
                meta["truncated"] = True
                break
            ok = True
            for k, v in filters.items():
                if str(rec.get(k, "")) != str(v):
                    ok = False
                    break
            if not ok:
                continue
            if qs:
                hay = flatten_text(rec, search_large_fields=search_large_fields)
                if flags_lower:
                    hay_cmp = hay.lower()
                    qs_cmp = [q.lower() for q in qs]
                else:
                    hay_cmp = hay
                    qs_cmp = qs
                if match == "all":
                    ok = all(q in hay_cmp for q in qs_cmp)
                else:
                    ok = any(q in hay_cmp for q in qs_cmp)
                if not ok:
                    continue
            proj = project_record(rec, fields, omit_fields, maxval)
            entry = {"file": rel, "line": line_no, "record": proj}
            entry_chars = len(json_dumps(entry))
            if len(hits) >= maxr or total_chars + entry_chars > maxtotal:
                meta["truncated"] = True
                break
            total_chars += entry_chars
            hits.append(entry)
        if len(hits) >= maxr or examined > maxexam:
            break
    truncated_flag = bool(meta.get("truncated"))
    out: Dict[str, Any] = {
        "status": "truncated" if truncated_flag else "ok",
        "query": query,
        "stats": {
            "files": len(files),
            "records_examined": examined,
            "hits": len(hits),
            "truncated": truncated_flag,
        },
        "hits": hits,
    }
    if meta.get("rejected_paths"):
        out["rejected_paths"] = meta["rejected_paths"]
    if meta.get("warnings"):
        out["warnings"] = meta["warnings"]
    return out


def cxs_self_check(root: Optional[str] = None) -> Dict[str, Any]:
    rp = root_path(root)
    rg = find_rg()
    out: Dict[str, Any] = {
        "status": "ok" if rg else "error",
        "version": VERSION,
        "root": str(rp),
        "python": sys.version.split()[0],
        "rg": rg,
        "default_scopes": DEFAULT_SCOPES,
    }
    if rg:
        try:
            p = subprocess.run(
                [rg, "--version"],
                text=True,
                capture_output=True,
                timeout=DEFAULT_BUDGET.self_check_rg_timeout_seconds,
            )
            out["rg_version"] = p.stdout.splitlines()[0] if p.stdout else "unknown"
        except Exception as exc:
            out["rg_version_error"] = str(exc)
    else:
        out["error"] = "rg_not_found"
    return out
