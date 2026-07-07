#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if command -v cygpath >/dev/null 2>&1 && [[ -n "${USERPROFILE:-}" ]]; then
  export PATH="$(cygpath -u "$USERPROFILE")/.cargo/bin:$PATH"
fi
if [[ -d "/mnt/c/Users/${USER:-}/.cargo/bin" ]]; then
  export PATH="/mnt/c/Users/${USER}/.cargo/bin:$PATH"
fi
if ! command -v cargo >/dev/null 2>&1 && command -v cargo.exe >/dev/null 2>&1; then
  cargo() { cargo.exe "$@"; }
fi
if command -v python3 >/dev/null 2>&1; then
  python_cmd=(python3)
elif command -v python >/dev/null 2>&1; then
  python_cmd=(python)
elif command -v py >/dev/null 2>&1; then
  python_cmd=(py -3)
elif command -v py.exe >/dev/null 2>&1; then
  python_cmd=(py.exe -3)
else
  echo "python3, python, or py is required to run the Python SDK smoke" >&2
  exit 127
fi

cd "$repo_root"
cargo build -p api

api_bin="$repo_root/target/debug/api"
if [[ -f "$api_bin.exe" ]]; then
  api_bin="$api_bin.exe"
fi
api_bind_addr="127.0.0.1:18280"
api_base_url="http://127.0.0.1:18280"
sdk_python_path="$repo_root/crates/sdk-python"
if [[ "$api_bin" == *.exe ]] && [[ -f /proc/version ]] && grep -qi microsoft /proc/version; then
  echo "scripts/check-sdk-live.sh needs a native Unix API binary under WSL; use scripts/check-sdk-live.ps1 with the Windows Rust toolchain, or install Rust inside WSL." >&2
  exit 2
fi

env -u DATABASE_URL \
  API_BIND_ADDR="$api_bind_addr" \
  PUBLIC_BASE_URL="$api_base_url" \
  MCP_BASE_URL="http://127.0.0.1:18290" \
  "$api_bin" >"$repo_root/.api-sdk-smoke.out.log" 2>"$repo_root/.api-sdk-smoke.err.log" &
api_pid=$!

cleanup() {
  kill "$api_pid" >/dev/null 2>&1 || true
  wait "$api_pid" >/dev/null 2>&1 || true
}
trap cleanup EXIT

ready=0
for _ in $(seq 1 80); do
  if "${python_cmd[@]}" - "$api_base_url" <<'PY' >/dev/null 2>&1
import sys
from urllib.request import urlopen

with urlopen(f"{sys.argv[1]}/health", timeout=2) as response:
    if response.read().decode() != "ok":
        raise SystemExit(1)
PY
  then
    ready=1
    break
  fi
  sleep 0.25
done

if [[ "$ready" != "1" ]]; then
  echo "API did not become ready at $api_base_url" >&2
  exit 1
fi

PYTHONPATH="$sdk_python_path" "${python_cmd[@]}" -m agent_bounties.smoke --base-url "$api_base_url"

cd "$repo_root/crates/sdk-typescript"
npm ci
npm run build
node dist/smoke.js --base-url "$api_base_url"
