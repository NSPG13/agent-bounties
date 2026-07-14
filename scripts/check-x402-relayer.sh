#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
port="${X402_RELAYER_TEST_PORT:-18545}"
rpc_url="http://127.0.0.1:${port}"
log_file="$repo_root/target/tmp/x402-relayer-anvil.log"
mkdir -p "$(dirname "$log_file")"

anvil --silent --port "$port" --chain-id 31337 >"$log_file" 2>&1 &
anvil_pid=$!
trap 'kill "$anvil_pid" 2>/dev/null || true; wait "$anvil_pid" 2>/dev/null || true' EXIT

for _ in $(seq 1 30); do
  if cast block-number --rpc-url "$rpc_url" >/dev/null 2>&1; then
    export AGENT_BOUNTIES_TEST_RPC_URL="$rpc_url"
    cargo test -p chain-base \
      tests::hosted_relayer_rehearsal_broadcasts_bounded_zero_value_transaction \
      -- --ignored --exact --nocapture
    exit 0
  fi
  sleep 1
done

echo "Anvil did not become ready; see $log_file" >&2
exit 1
