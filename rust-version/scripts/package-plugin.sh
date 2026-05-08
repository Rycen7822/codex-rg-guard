#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUST_ROOT="${ROOT}/rust-version"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "${RUST_ROOT}/Cargo.toml" | head -n 1)"
HOST_TRIPLE="$(rustc -vV | awk '/^host:/ {print $2}')"
TARGET_TRIPLE="${TARGET:-${HOST_TRIPLE}}"
PACKAGE_NAME="codex-rg-guard-rust-${VERSION}-${TARGET_TRIPLE}"
DIST="${RUST_ROOT}/dist"
STAGE="${DIST}/${PACKAGE_NAME}"
ARCHIVE="${DIST}/${PACKAGE_NAME}.tar.gz"
BIN_DIR="${RUST_ROOT}/target/release"

if [[ -z "${VERSION}" || -z "${TARGET_TRIPLE}" ]]; then
  echo "failed to determine package version or target triple" >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required to build the Rust distribution package" >&2
  exit 1
fi

if ! command -v rg >/dev/null 2>&1; then
  echo "ripgrep is required for runtime validation" >&2
  exit 1
fi

if [[ -n "${TARGET:-}" ]]; then
  cargo build --release --bins --target "${TARGET}" --manifest-path "${RUST_ROOT}/Cargo.toml"
  BIN_DIR="${RUST_ROOT}/target/${TARGET}/release"
else
  cargo build --release --bins --manifest-path "${RUST_ROOT}/Cargo.toml"
fi

rm -rf "${STAGE}" "${ARCHIVE}"
mkdir -p "${STAGE}/.codex-plugin" "${STAGE}/bin" "${STAGE}/docs"

cp "${ROOT}/.codex-plugin/plugin.json" "${STAGE}/.codex-plugin/plugin.json"
cp "${ROOT}/LICENSE" "${STAGE}/LICENSE"
cp "${ROOT}/README.md" "${STAGE}/README-repo.md"
cp "${RUST_ROOT}/README.md" "${STAGE}/README.md"
cp -R "${ROOT}/skills" "${STAGE}/skills"
cp -R "${ROOT}/docs" "${STAGE}/docs/python"
cp -R "${RUST_ROOT}/docs" "${STAGE}/docs/rust"
cp "${RUST_ROOT}/packaging/install-local.sh" "${STAGE}/install-local.sh"

cp "${BIN_DIR}/cxs-rs" "${STAGE}/bin/cxs"
cp "${BIN_DIR}/cxs-mcp-server-rs" "${STAGE}/bin/cxs-mcp-server"
cp "${BIN_DIR}/rg" "${STAGE}/bin/rg"
chmod +x "${STAGE}/bin/cxs" "${STAGE}/bin/cxs-mcp-server" "${STAGE}/bin/rg" "${STAGE}/install-local.sh"

cat > "${STAGE}/.mcp.json" <<'JSON'
{
  "mcp_servers": {
    "cxs-rg-guard": {
      "command": "./bin/cxs-mcp-server",
      "args": [],
      "cwd": ".",
      "env": {
        "CXS_DEFAULT_SCOPES": "docs,src,tests,config,analysis",
        "CXS_SHIM_PATH": "./bin/rg"
      }
    }
  }
}
JSON

"${STAGE}/bin/cxs" self-check --root "${ROOT}" >/dev/null
printf '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}\n' \
  | "${STAGE}/bin/cxs-mcp-server" >/dev/null

tar -C "${DIST}" -czf "${ARCHIVE}" "${PACKAGE_NAME}"

echo "Package: ${ARCHIVE}"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "${ARCHIVE}"
fi
echo "Install after extraction: ./${PACKAGE_NAME}/install-local.sh"
