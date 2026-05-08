# Completion Audit

Objective: migrate the current cxs implementation to Rust under
`/home/xu/project/codex-rg-guard/rust-version`, keep enough notes for a long
task, and report whether functionality, output compatibility, and cleanup are
complete.

## Prompt-to-artifact checklist

| Requirement | Evidence |
| --- | --- |
| Work path and save path is `rust-version` | `Cargo.toml`, `src/`, `tests/`, `scripts/`, and docs live under `rust-version/`. |
| Migrate all active code to Rust | `src/lib.rs` implements `find`, `files`, `symbol`, `json`, `self_check`, subprocess budget control, and the guarded rg shim logic. |
| Provide CLI replacement | `src/bin/cxs.rs` builds `cxs-rs`. |
| Provide MCP replacement | `src/bin/cxs_mcp_server.rs` builds `cxs-mcp-server-rs` with the same single advertised `cxs` tool and compatibility aliases. |
| Provide rg shim replacement | `src/bin/rg.rs` builds an `rg` shim. |
| Keep removed read op removed | Tests assert MCP tool output does not advertise `read`; docs state it is not reintroduced. |
| Output matches Python version | `python3 rust-version/scripts/compare_python.py` returned `COMPAT_OK`. |
| Functional test coverage | `cargo test` returned 7 integration tests passing plus unit/doc test harnesses passing. |
| Formatting | `cargo fmt --check` passed. |
| Lint cleanup | `cargo clippy --all-targets --all-features -- -D warnings` passed. |
| Python package not broken | `rtk pytest -q` returned `Pytest: 15 passed`; `python3 scripts/self_check.py` returned `SELF_CHECK_OK`. |
| Avoid committing build artifacts | `.gitignore` ignores `rust-version/target/`; `rtk find rust-version -maxdepth 3 -type f` lists only source/docs/scripts/lockfile files. |

## Status

All active cxs functionality has been migrated to Rust. The only intentional
output difference is `self_check`, where Rust reports `runtime: "rust"` instead
of Python interpreter metadata. The compatibility script ignores only these
runtime-specific fields and subprocess byte counters.

The Rust implementation removes the obsolete `read` operation, keeps MCP tool
injection terse, and keeps binary entry points thin. No temporary comparison
fixtures or Cargo build artifacts are part of the tracked source tree.
