---
name: rg-budget-search
description: Prefer this plugin over raw rg/grep for broad searches in large or many-file projects; keeps Codex context small.
---

Prefer MCP `cxs(op,args)` or CLI `cxs` instead of raw `rg`/`grep` when the project has many files, the search scope is broad, or raw search is likely to produce noisy/truncated output.

For small projects, known files, or narrow local checks, use normal file reads or direct shell tools instead.

Ops: `find`, `files`, `symbol`, `json`, `self_check`.

Pattern: broad content search with `find(files_only:true)`, then `find(paths=[...])`, then use Codex-native file reading only for the exact span you need.

Avoid `rg -n PATTERN .`, broad OR regexes, raw JSONL/log dumps, and `runs/vendor` unless required. If `truncated`, refine query/scope/path or continue `files_only` with `next_page.offset`.
