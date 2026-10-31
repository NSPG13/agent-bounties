#!/usr/bin/env bash
# Reuse the existing paid legacy worker. No signing or new service authority.
set -euo pipefail
[[ "${RENDER_SERVICE_ID:-}" == 'srv-d982ftu7r5hc73ar4st0' ]] || {
  echo 'This entrypoint is restricted to the existing production indexer.' >&2; exit 1;
}
: "${DATABASE_URL:?Existing shared database required}"
[[ "${BASE_INDEXER_NETWORK:-base-mainnet}" == 'base-mainnet' ]] || exit 1
case "${POSTING_CREATOR_INDEXER_ENABLED:-false}" in
  true)
    : "${CREATOR_OPEN_RELEASE_JSON:?Reviewed mainnet release required}"
    : "${BASE_MAINNET_RPC_URL:?Mainnet RPC required}"
    ;;
  false) ;;
  *) echo 'Invalid creator indexer enable flag.' >&2; exit 1 ;;
esac
case "${POSTING_V2_INDEXER_ENABLED:-false}" in
  true) : "${BASE_MAINNET_RPC_URL:?Mainnet RPC required}" ;;
  false) ;;
  *) echo 'Invalid v2 indexer enable flag.' >&2; exit 1 ;;
esac

children=()
stop_children() {
  trap - EXIT TERM INT
  if (( ${#children[@]} )); then
    kill -TERM "${children[@]}" 2>/dev/null || true
    wait "${children[@]}" 2>/dev/null || true
  fi
}
trap stop_children EXIT TERM INT

# Preserve the legacy indexer's documented configuration, including its existing
# discovery delivery boundary. No relayer, app, budget or mail credential enters
# either worker. The protocol cannot be changed through an inherited variable.
legacy=("DATABASE_URL=$DATABASE_URL" "BASE_INDEXER_PROTOCOL=autonomous-v1" "BASE_INDEXER_NETWORK=base-mainnet")
for key in RUST_LOG PUBLIC_BASE_URL BASE_MAINNET_RPC_URL BASE_MAINNET_BOUNTY_FACTORY \
  BASE_INDEXER_FACTORY_CONTRACT BASE_INDEXER_RPC_URL BASE_INDEXER_START_BLOCK \
  BASE_INDEXER_FROM_BLOCK BASE_INDEXER_POLL_SECONDS BASE_INDEXER_CONFIRMATIONS \
  BASE_INDEXER_MAX_BLOCKS_PER_QUERY BASE_INDEXER_REQUEST_ID \
  BASE_INDEXER_RETRY_INITIAL_SECONDS BASE_INDEXER_RETRY_MAX_SECONDS \
  BASE_INDEXER_EXIT_AFTER_FAILURES DISCOVERY_WEBHOOK_SIGNING_KEY; do
  if [[ -v "$key" ]]; then legacy+=("$key=${!key}"); fi
done
env -i "${legacy[@]}" /usr/local/bin/posting-indexer &
children+=("$!")
if [[ "${POSTING_CREATOR_INDEXER_ENABLED:-false}" == true ]]; then
  env -i DATABASE_URL="$DATABASE_URL" BASE_INDEXER_PROTOCOL=creator-open-v1 \
    BASE_MAINNET_RPC_URL="$BASE_MAINNET_RPC_URL" \
    CREATOR_OPEN_RELEASE_JSON="$CREATOR_OPEN_RELEASE_JSON" \
    RUST_LOG="${RUST_LOG:-info}" /usr/local/bin/posting-indexer &
  children+=("$!")
fi
# Read-only autonomous-v2 indexing of the recorded mainnet factory from its deployment
# block. Card-bounty funding and customer review read only events it has confirmed.
if [[ "${POSTING_V2_INDEXER_ENABLED:-false}" == true ]]; then
  env -i DATABASE_URL="$DATABASE_URL" BASE_INDEXER_PROTOCOL=autonomous-v2 \
    BASE_INDEXER_NETWORK=base-mainnet \
    BASE_INDEXER_FACTORY_CONTRACT=0xc33e2ae33bb9580837ea59df18e57fa1039ae58a \
    BASE_INDEXER_START_BLOCK=52275492 BASE_INDEXER_RPC_URL="$BASE_MAINNET_RPC_URL" \
    PUBLIC_BASE_URL="${PUBLIC_BASE_URL:-https://api.agentizable.ai}" \
    RUST_LOG="${RUST_LOG:-info}" /usr/local/bin/posting-indexer &
  children+=("$!")
fi
# A crashed child stops the container so Render restarts both from their durable
# cursors. This read-only image deliberately does not activate provider notices.
set +e
wait -n "${children[@]}"
status=$?
(( status != 0 )) || status=1
exit "$status"
