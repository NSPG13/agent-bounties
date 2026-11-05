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
python3 scripts/run-required-postgres-test.py -p db tests::x402_relay_attempt_is_idempotent_and_lease_bounded --exact
python3 scripts/run-required-postgres-test.py -p db tests::claim_funnel_counts_direct_and_atomic_sponsored_confirmations --exact
python3 scripts/run-required-postgres-test.py -p db tests::opportunity_lifecycle_query_executes_against_migrated_postgres --exact
python3 scripts/run-required-postgres-test.py -p db tests::site_analytics_round_trip_executes_against_migrated_postgres --exact
python3 scripts/run-required-postgres-test.py -p db tests::platform_metrics_query_separates_history_from_identity_and_cohort_boundaries --exact
python3 scripts/run-required-postgres-test.py -p db tests::controlled_card_test_exclusion_preserves_money_and_shared_wallet_customers --exact
python3 scripts/run-required-postgres-test.py -p db managed_service_orders::metrics::postgres_tests:: --test-threads=1
python3 scripts/run-required-postgres-test.py -p api platform_payouts::tests::platform_payout_http_replays_real_database_selection --exact
python3 scripts/run-required-postgres-test.py -p db tests::opportunity_comment_round_trip_is_durable_and_idempotent --exact
python3 scripts/run-required-postgres-test.py -p db tests::chatgpt_action_intent_replays_and_confirms_only_observed_transaction --exact
python3 scripts/run-required-postgres-test.py -p db site_posting_drafts
python3 scripts/run-required-postgres-test.py -p db tests::social_mention_ingestion_round_trip_executes_against_migrated_postgres --exact
python3 scripts/run-required-postgres-test.py -p db tests::discovery_webhook_round_trip_executes_against_migrated_postgres --exact
python3 scripts/run-required-postgres-test.py -p api open_competition_v2_api::tests::postgres_readiness_hold_blocks_new_payments_but_preserves_reconciliation --exact --test-threads=1
python3 scripts/run-required-postgres-test.py -p api tests::audience_audit_persists_idempotently_across_processes --exact
python3 scripts/run-required-postgres-test.py -p api tests::github_issue_api_sync_postgres_rejects_stale_cross_process_activity --exact
python3 scripts/run-required-postgres-test.py -p db scoped_submission_history_reads_only_selected_bounty_postgres
python3 scripts/run-required-postgres-test.py -p db tests::autonomous_v2_events_round_trip_through_postgres --exact
python3 scripts/run-required-postgres-test.py -p db managed_service_orders::tests:: --test-threads=1
python3 scripts/run-required-postgres-test.py -p api managed_payments::tests:: --test-threads=1
python3 scripts/run-required-postgres-test.py -p api tests::opportunity_exact_unfunded_lookup_crosses_list_window_postgres --exact
python3 scripts/run-required-postgres-test.py -p api tests::github_issue_api_sync_postgres_serializes_concurrent_initial_sync --exact
python3 scripts/run-required-postgres-test.py -p api tests::neynar_webhook_persists_one_short_draft_and_one_reply_across_retries --exact
python3 scripts/run-required-postgres-test.py -p api tests::bounty_status_reads_scoped_postgres_after_cross_process_funding --exact
python3 scripts/run-required-postgres-test.py -p mcp-server tests::mcp_bounty_status_reads_scoped_postgres_after_cross_process_funding --exact
cargo run -p cli -- service-smoke-spawn \
  --api-base-url http://127.0.0.1:18180 \
  --mcp-base-url http://127.0.0.1:18190 \
  --database-url "$database_url" \
  --verify-restart-persistence
