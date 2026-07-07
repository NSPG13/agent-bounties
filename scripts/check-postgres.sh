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

cd "$repo_root"
docker compose up -d postgres

ready=0
for _ in $(seq 1 30); do
  if docker compose exec -T postgres pg_isready -U agent_bounties >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 2
done

if [[ "$ready" != "1" ]]; then
  echo "Postgres did not become ready within 60 seconds" >&2
  exit 1
fi

database_url="${DATABASE_URL:-postgres://agent_bounties:agent_bounties@localhost:5432/agent_bounties}"

cargo build -p api -p mcp-server
cargo run -p cli -- service-smoke-spawn \
  --api-base-url http://127.0.0.1:18180 \
  --mcp-base-url http://127.0.0.1:18190 \
  --database-url "$database_url" \
  --verify-restart-persistence
