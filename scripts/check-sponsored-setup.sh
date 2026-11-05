#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
: "${AGENT_BOUNTIES_TEST_DATABASE_URL:?Set a disposable loopback posting_ database URL}"
port="${SPONSORED_SETUP_TEST_PORT:-18551}"
mkdir -p target/tmp
forge build --root contracts/base-escrow > target/tmp/sponsored-setup-forge.log 2>&1
anvil --silent --host 127.0.0.1 --port "$port" --chain-id 84532 --base-fee 1 --gas-price 1 > target/tmp/sponsored-setup-anvil.log 2>&1 &
anvil_pid=$!
trap 'kill "$anvil_pid" 2>/dev/null || true; wait "$anvil_pid" 2>/dev/null || true' EXIT
for _ in $(seq 1 30); do
  if ! kill -0 "$anvil_pid" 2>/dev/null; then echo 'Isolated Anvil failed to start' >&2; exit 1; fi
  if cast block-number --rpc-url "http://127.0.0.1:$port" >/dev/null 2>&1; then
    export AGENT_BOUNTIES_TEST_RPC_URL="http://127.0.0.1:$port"
    export ENABLE_SPONSORED_SETUP=true
    cargo test -p api sponsored_setup::rehearsal::zero_eth_setup_checks_authority_recovers_and_shares_budget -- --ignored --exact --nocapture
    exit 0
  fi
  sleep 1
done
echo 'Isolated Anvil did not become ready' >&2
exit 1
