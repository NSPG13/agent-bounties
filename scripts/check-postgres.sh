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
export AGENT_BOUNTIES_TEST_DATABASE_URL="$database_url"
cargo test -p db tests::x402_relay_attempt_is_idempotent_and_lease_bounded -- --ignored --exact --nocapture
cargo test -p db tests::claim_funnel_counts_direct_and_atomic_sponsored_confirmations -- --ignored --exact --nocapture
cargo test -p db tests::opportunity_lifecycle_query_executes_against_migrated_postgres -- --ignored --exact --nocapture
cargo test -p db tests::site_analytics_round_trip_executes_against_migrated_postgres -- --ignored --exact --nocapture
cargo test -p db tests::social_mention_ingestion_round_trip_executes_against_migrated_postgres -- --ignored --exact --nocapture
cargo test -p db tests::discovery_webhook_round_trip_executes_against_migrated_postgres -- --ignored --exact --nocapture
cargo test -p api tests::audience_audit_persists_idempotently_across_processes -- --ignored --exact --nocapture
cargo test -p api tests::github_issue_api_sync_postgres_rejects_stale_cross_process_activity -- --ignored --exact --nocapture
cargo test -p api tests::github_issue_api_sync_postgres_serializes_concurrent_initial_sync -- --ignored --exact --nocapture
cargo test -p api tests::neynar_webhook_persists_one_short_draft_and_one_reply_across_retries -- --ignored --exact --nocapture
cargo run -p cli -- service-smoke-spawn \
  --api-base-url http://127.0.0.1:18180 \
  --mcp-base-url http://127.0.0.1:18190 \
  --database-url "$database_url" \
  --verify-restart-persistence
