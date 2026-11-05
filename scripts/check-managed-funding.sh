#!/usr/bin/env bash
# Card-paid order to funded autonomous-v2 bounty on loopback Anvil and a disposable database.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
: "${AGENT_BOUNTIES_TEST_DATABASE_URL:?Set a disposable loopback agent_bounties_test database URL}"
export AGENT_BOUNTIES_LOCAL_TEST_DATABASE=1
port="${MANAGED_FUNDING_TEST_PORT:-18551}"
mkdir -p target/tmp
forge build --root contracts/base-escrow > target/tmp/managed-funding-forge.log 2>&1
anvil --silent --host 127.0.0.1 --port "$port" --chain-id 84532 --base-fee 1 --gas-price 1 > target/tmp/managed-funding-anvil.log 2>&1 &
anvil_pid=$!
trap 'kill "$anvil_pid" 2>/dev/null || true; wait "$anvil_pid" 2>/dev/null || true' EXIT
for _ in $(seq 1 30); do
  if ! kill -0 "$anvil_pid" 2>/dev/null; then echo 'Isolated Anvil failed to start' >&2; exit 1; fi
  if cast block-number --rpc-url "http://127.0.0.1:$port" >/dev/null 2>&1; then
    export AGENT_BOUNTIES_TEST_RPC_URL="http://127.0.0.1:$port"
    cargo test -p worker managed_funding::rehearsal -- --ignored --exact --nocapture \
      managed_funding::rehearsal::paid_card_order_funds_its_exact_bounty_once_through_real_evm_and_indexer
    exit 0
  fi
  sleep 1
done
echo 'Isolated Anvil did not become ready' >&2
exit 1
