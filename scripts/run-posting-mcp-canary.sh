#!/usr/bin/env bash
# A separate process indexes the exact Sepolia release against the existing
# private staging database. Its environment contains no signer or provider key.
set -euo pipefail
[[ "${RENDER_SERVICE_ID:-}" == 'srv-dau3p1favr4c73fij9jg' ]] || {
  echo 'This entrypoint is restricted to the existing posting MCP canary.' >&2; exit 1;
}
: "${DATABASE_URL:?Staging database required}"
: "${CREATOR_OPEN_RELEASE_JSON:?Reviewed Sepolia manifest required}"
[[ "${BASE_SEPOLIA_RPC_URL:-}" == 'https://sepolia.base.org' ]] || exit 1
children=()
stop_children() {
  trap - EXIT TERM INT
  if (( ${#children[@]} )); then
    kill -TERM "${children[@]}" 2>/dev/null || true
    wait "${children[@]}" 2>/dev/null || true
  fi
}
trap stop_children EXIT TERM INT
env -i DATABASE_URL="$DATABASE_URL" BASE_INDEXER_PROTOCOL=creator-open-v1 \
  BASE_SEPOLIA_RPC_URL="$BASE_SEPOLIA_RPC_URL" \
  CREATOR_OPEN_RELEASE_JSON="$CREATOR_OPEN_RELEASE_JSON" \
  /usr/local/bin/posting-indexer &
children+=("$!")
# Pooled-funding fixtures originate at the already reviewed legacy factory. Its
# independent canonical indexer gets only the isolated DB and public test RPC.
env -i DATABASE_URL="$DATABASE_URL" BASE_INDEXER_PROTOCOL=autonomous-v1 \
  BASE_INDEXER_NETWORK=base-sepolia BASE_INDEXER_RPC_URL="$BASE_SEPOLIA_RPC_URL" \
  BASE_INDEXER_FACTORY_CONTRACT=0xf699c4261709e67ad748c731ac0f7d014677e6e7 \
  BASE_INDEXER_START_BLOCK=47510671 BASE_INDEXER_MAX_BLOCKS_PER_QUERY=1000 \
  BASE_INDEXER_CONFIRMATIONS=3 BASE_INDEXER_POLL_SECONDS=15 \
  BASE_INDEXER_RETRY_INITIAL_SECONDS=5 BASE_INDEXER_RETRY_MAX_SECONDS=120 \
  BASE_INDEXER_EXIT_AFTER_FAILURES=8 /usr/local/bin/posting-indexer &
children+=("$!")
/usr/local/bin/posting-mcp &
children+=("$!")
# If either process exits, the container exits and Render restarts both.
set +e
wait -n "${children[@]}"
status=$?
(( status != 0 )) || status=1
exit "$status"
