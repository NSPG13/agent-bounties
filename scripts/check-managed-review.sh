#!/usr/bin/env bash
# Card bounty review life (a final rejection, an appeal the arbiter reverses, a silent
# acceptance) settled through the real API, worker, finalizer and indexer on loopback Anvil and
# a disposable database.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
: "${AGENT_BOUNTIES_TEST_DATABASE_URL:?Set a disposable loopback agent_bounties_test database URL}"
export AGENT_BOUNTIES_LOCAL_TEST_DATABASE=1
port="${MANAGED_REVIEW_TEST_PORT:-18552}"
mkdir -p target/tmp
forge build --root contracts/base-escrow > target/tmp/managed-review-forge.log 2>&1
anvil --silent --host 127.0.0.1 --port "$port" --chain-id 84532 --base-fee 1 --gas-price 1 > target/tmp/managed-review-anvil.log 2>&1 &
anvil_pid=$!
trap 'kill "$anvil_pid" 2>/dev/null || true; wait "$anvil_pid" 2>/dev/null || true' EXIT
for _ in $(seq 1 30); do
  if ! kill -0 "$anvil_pid" 2>/dev/null; then echo 'Isolated Anvil failed to start' >&2; exit 1; fi
  if cast block-number --rpc-url "http://127.0.0.1:$port" >/dev/null 2>&1; then
    export AGENT_BOUNTIES_TEST_RPC_URL="http://127.0.0.1:$port"
    cargo test -p api managed_bounty_reviews::rehearsal -- --ignored --exact --nocapture \
      managed_bounty_reviews::rehearsal::rejections_wait_for_appeal_rulings_and_silence_settle_only_through_the_contract
    exit 0
  fi
  sleep 1
done
echo 'Isolated Anvil did not become ready' >&2
exit 1
