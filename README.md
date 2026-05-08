# Codex rg Guard

Low-context Codex plugin that should be preferred over raw `rg`/`grep` for broad searches in large or many-file projects.

## What it ships

- `skills/rg-budget-search/SKILL.md`: short routing rule.
- Python version:
  - `.mcp.json` + `mcp/cxs_mcp_server.py`: one MCP tool, `cxs(op,args)`.
  - `bin/cxs`: CLI.
  - `bin/rg`: optional raw `rg` shim.
- Rust version:
  - `rust-version/src/`: Rust implementation.
  - `rust-version/scripts/package-plugin.sh`: builds a distributable plugin package.
  - packaged binaries: `bin/cxs`, `bin/cxs-mcp-server`, `bin/rg`.

The Skill and MCP tool descriptions are intentionally terse to avoid injecting large instruction blocks into Codex.

Prefer this plugin over raw `rg`/`grep` for broad searches across many files,
especially when raw search would flood the model context or hit output limits.
For small projects, known files, or narrow local checks, normal file reads and
direct shell tools are usually simpler.

## Requirements

Python version:

- Python 3.10+
- ripgrep (`rg`)

Rust prebuilt package:

- ripgrep (`rg`)
- no Rust/Cargo/Python required to run the MCP server

Rust build/package workflow:

- Rust toolchain with `cargo`
- ripgrep (`rg`)
- optional: `rustfmt` and `clippy` for validation

Optional:

- ast-grep (`sg`) only if you later extend symbol search to AST search.

Not used: vector search, SQLite, embeddings, `jq`, `fd`.

## Install Python Version

Use this when you want the original Python implementation or want to edit the
Python code directly.

```bash
unzip codex-rg-guard.zip
cd codex-rg-guard
./scripts/install-local.sh
```

Restart Codex, then enable **Codex rg Guard**.

Fallback if bundled MCP is not active:

```bash
codex mcp add cxs-rg-guard -- python3 ~/.codex/plugins/codex-rg-guard/mcp/cxs_mcp_server.py
```

## Install Rust Prebuilt Package

Use this for normal distribution. The target machine does not need Rust, Cargo,
or Python to run the MCP server. It only needs `rg` in `PATH`.

Build the package on the target platform or CI:

```bash
bash rust-version/scripts/package-plugin.sh
```

This writes:

```text
rust-version/dist/codex-rg-guard-rust-<version>-<target>.tar.gz
```

Distribute that archive. On the target machine:

```bash
tar -xzf codex-rg-guard-rust-<version>-<target>.tar.gz
./codex-rg-guard-rust-<version>-<target>/install-local.sh
```

Restart Codex, then enable **Codex rg Guard**.

Fallback direct MCP registration:

```bash
codex mcp add cxs-rg-guard -- ~/.codex/plugins/codex-rg-guard/bin/cxs-mcp-server
```

Optional shell tools:

```bash
export PATH="$HOME/.codex/plugins/codex-rg-guard/bin:$PATH"
```

## Build Rust From Source

Use this for development:

```bash
cd rust-version
cargo build --release --bins
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

From the repository root, build a package for a specific installed Rust target:

```bash
TARGET=x86_64-unknown-linux-musl bash rust-version/scripts/package-plugin.sh
```

## MCP

Codex sees only one tool:

```json
{"op":"find","args":{"query":"ExactIdentifier","scopes":["docs","analysis"]}}
```

For broad content search in a large or many-file project, first ask for files only:

```json
{"op":"find","args":{"query":"ExactIdentifier","scopes":["docs","src"],"files_only":true}}
```

Then run `find` with the returned `paths` to get bounded line hits. After that,
read exact files or spans with Codex-native file tools only when a concrete file
and line are known.
If a files-only result is truncated, repeat the same call with `offset` from `next_page`.

## CLI

```bash
bin/cxs self-check
bin/cxs find "ExactIdentifier" --scope docs --scope analysis
bin/cxs find "ExactIdentifier" --files-only --scope docs --scope src
bin/cxs find "ExactIdentifier" --files-only --scope docs --scope src --offset 30
bin/cxs symbol train_loop
bin/cxs json --filter doc_id=doc-123 --field doc_id --field token_count --scope analysis
```

All outputs are compact JSON by default. Add `--pretty` for manual inspection.
When a limit flag is omitted, core uses `cxs.core.DEFAULT_BUDGET`.

## Optional rg shim

```bash
export PATH="$HOME/.codex/plugins/codex-rg-guard/bin:$PATH"
```

Common `rg -n` calls return compact, budgeted JSON. Escape hatch:

```bash
CXS_RAW_RG=1 rg -n "pattern" .
```

## Tests

```bash
python3 -m unittest discover -s tests -v
python3 scripts/self_check.py
```
