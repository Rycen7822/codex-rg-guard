# Design

Principle: in large or many-file projects, prefer cxs over raw `rg`/`grep` for broad searches while keeping `rg` as the mature search engine under the hood.

Controls:

- `Budget` is the single default source for hit, line, character, timeout, and process-output limits;
- argv-based subprocess calls; no shell interpolation;
- literal search by default;
- scopes and default excludes;
- `--max-count`, `--max-columns`, byte, line, hit, and time budgets;
- compact JSON outputs;
- intended as the preferred replacement for raw `rg`/`grep` in broad project searches; known-file or small-scope work should use normal file reads or direct shell tools;
- broad content search can use `find(files_only:true)` to return matching files without snippets;
- files-only results expose `next_meta` / `next_page` when only a page or first follow-up batch is represented;
- truncated output reports limit reasons such as total-hit, total-character, per-file-hit, process-byte, process-line, or timeout limits;
- no MCP file-reader op: after second-stage `find(paths=[...])` identifies a concrete file and line, Codex reads the needed file span with its native file tools;
- JSONL/CSV projection with large fields omitted by default.

No vector index, no SQLite, no embedding layer.
