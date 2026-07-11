#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

api_base_url="${PRODUCTION_API_BASE_URL:-}"
mcp_base_url="${PRODUCTION_MCP_BASE_URL:-}"
require_eval_history="false"
expected_revision="${PRODUCTION_EXPECTED_REVISION:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --api-base-url)
      api_base_url="${2:-}"
      shift 2
      ;;
    --mcp-base-url)
      mcp_base_url="${2:-}"
      shift 2
      ;;
    --require-eval-history)
      require_eval_history="true"
      shift
      ;;
    --expected-revision)
      expected_revision="${2:-}"
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$api_base_url" ]]; then
  echo "Set PRODUCTION_API_BASE_URL or pass --api-base-url." >&2
  exit 2
fi
if [[ -z "$mcp_base_url" ]]; then
  echo "Set PRODUCTION_MCP_BASE_URL or pass --mcp-base-url." >&2
  exit 2
fi

if command -v cygpath >/dev/null 2>&1 && [[ -n "${USERPROFILE:-}" ]]; then
  export PATH="$(cygpath -u "$USERPROFILE")/.cargo/bin:$PATH"
fi
if [[ -d "/mnt/c/Users/${USER:-}/.cargo/bin" ]]; then
  export PATH="/mnt/c/Users/${USER}/.cargo/bin:$PATH"
fi
export PATH="$repo_root/.tools/foundry:$PATH"
if ! command -v cargo >/dev/null 2>&1 && command -v cargo.exe >/dev/null 2>&1; then
  cargo() { cargo.exe "$@"; }
fi

cd "$repo_root"
args=(
  run -p cli --
  production-smoke
  --api-base-url "$api_base_url"
  --mcp-base-url "$mcp_base_url"
)
if [[ "$require_eval_history" == "true" ]]; then
  args+=(--require-eval-history)
fi
if [[ -n "$expected_revision" ]]; then
  args+=(--expected-revision "$expected_revision")
fi
cargo "${args[@]}"
