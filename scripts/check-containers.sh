#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
api_image="${API_IMAGE:-agent-bounties-api:local}"
mcp_image="${MCP_IMAGE:-agent-bounties-mcp:local}"

if ! docker version >/dev/null 2>&1 && command -v docker.exe >/dev/null 2>&1; then
  docker() { docker.exe "$@"; }
fi

cd "$repo_root"
docker build \
  --build-arg APP_PACKAGE=api \
  --build-arg APP_BINARY=api \
  -t "$api_image" \
  .
docker build \
  --build-arg APP_PACKAGE=mcp-server \
  --build-arg APP_BINARY=mcp-server \
  -t "$mcp_image" \
  .
