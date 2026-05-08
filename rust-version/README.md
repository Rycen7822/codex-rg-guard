# codex-rg-guard Rust version

This directory contains the Rust migration of the Python `cxs` implementation.
It is intentionally self-contained so the Python package can remain the golden
reference while the Rust binary is validated.

## Binaries

- `cxs-rs`: CLI equivalent of `bin/cxs`.
- `cxs-mcp-server-rs`: MCP JSON-RPC server equivalent of `bin/cxs-mcp-server`.
- `rg`: guarded `rg` shim equivalent of `bin/rg`.

## Implemented operations

- `find`
- `files`
- `symbol`
- `json`
- `self_check`

The removed Python `read` operation is not reintroduced.

## Verification

Run from this directory:

```bash
cargo test
```

Run from the repository root:

```bash
python3 rust-version/scripts/compare_python.py
```

The compatibility script compares Rust MCP output with Python output for the
core search paths and ignores only runtime-specific self-check fields and
subprocess byte counters.

## Build a distributable plugin package

Run from the repository root:

```bash
bash rust-version/scripts/package-plugin.sh
```

The script builds release binaries and writes:

```text
rust-version/dist/codex-rg-guard-rust-<version>-<target>.tar.gz
```

Users can install the package without Rust, Cargo, or Python at runtime:

```bash
tar -xzf codex-rg-guard-rust-<version>-<target>.tar.gz
./codex-rg-guard-rust-<version>-<target>/install-local.sh
```

Runtime requirement: `rg` must be installed and available in `PATH`. The
installer uses Python only when available to update the local plugin marketplace
entry; the MCP server itself does not need Python.

To build for a specific installed Rust target:

```bash
TARGET=x86_64-unknown-linux-musl bash rust-version/scripts/package-plugin.sh
```
