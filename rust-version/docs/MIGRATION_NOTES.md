# Rust Migration Notes

## Scope

The Rust version ports the active Python surface:

- budget-controlled `find`
- `files_only` first-stage search with `next` and `next_page`
- path listing through `files`
- regex-based `symbol`
- bounded JSON/JSONL/CSV/TSV search through `json`
- single-tool MCP server with compatibility aliases
- guarded `rg` shim

The old `read` operation remains removed.

## Budget Ownership

All user-visible defaults and clamp limits are centralized in
`src/lib.rs::Budget`. The intent matches `cxs/core.py`: omitted arguments use
the default budget, while caller-provided values are clamped before they affect
subprocess runtime or response size.

The most important fields to edit later are:

- `max_total_hits`
- `max_hits_per_file`
- `max_line_chars`
- `max_total_chars`
- `timeout_seconds`
- `process_output_bytes`

Internal scan multipliers and structured-data limits are also kept in the same
struct so future tuning does not require hunting through the implementation.

## Compatibility

`scripts/compare_python.py` treats the Python implementation as the golden
reference. It exercises the Rust MCP server and compares semantic JSON output.
The verifier intentionally ignores:

- subprocess `stats.bytes`
- `self_check` runtime identity fields
- real `rg` executable path/version fields

Those fields are expected to differ between Python and Rust. The search results,
status values, pagination hints, bounded snippets, path rejection behavior, and
MCP tool contract are compared.

## Cleanup Choices

The Rust implementation does not preserve Python-only layering such as separate
CLI wrapper modules. The core behavior lives in `src/lib.rs`, with three thin
binary entry points under `src/bin`.

The design keeps `rg` as the search engine instead of rewriting ripgrep. Rust
mainly removes Python startup overhead, packages a single native implementation,
and makes subprocess/runtime handling explicit.
