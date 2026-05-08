---
name: rg-budget-search
description: Use before raw rg/grep/cat when searching files; keeps Codex context small.
---

Use MCP `cxs(op,args)` or CLI `cxs` before raw search.

Ops: `find`, `files`, `symbol`, `json`, `self_check`.

Pattern: broad content search with `find(files_only:true)`, then `find(paths=[...])`, then use Codex-native file reading only for the exact span you need.

Avoid `rg -n PATTERN .`, broad OR regexes, raw JSONL/log dumps, and `runs/vendor` unless required. If `truncated`, refine query/scope/path or continue `files_only` with `next_page.offset`.
