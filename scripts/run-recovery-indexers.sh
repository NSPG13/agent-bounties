#!/usr/bin/env bash
# Dedicated recovery children only; no general worker or notification dispatcher.
set -euo pipefail
[[ $# == 0 ]] || exit 2
mode=${RECOVERY_READER_MODE:-standby}
case "$mode" in
  standby) exec env -i RENDER_SERVICE_ID="${RENDER_SERVICE_ID:-}" RENDER_GIT_COMMIT="${RENDER_GIT_COMMIT:-}" RECOVERY_READER_MODE=standby /usr/local/bin/recovery-reader ;;
  active) ;;
  *) echo 'Invalid recovery reader mode.' >&2; exit 2 ;;
esac
# Check the immutable artifact/runtime binding before reading dependent settings.
env -i RENDER_SERVICE_ID="${RENDER_SERVICE_ID:-}" RENDER_GIT_COMMIT="${RENDER_GIT_COMMIT:-}"   RECOVERY_READER_MODE=active /usr/local/bin/recovery-reader --check-binding
: "${DATABASE_URL:?Recovery database required}"
[[ "${BASE_INDEXER_NETWORK:-base-mainnet}" == base-mainnet ]] || exit 2
[[ -n "${BASE_INDEXER_RPC_URL:-${BASE_MAINNET_RPC_URL:-}}" ]] || exit 2
for flag in POSTING_CREATOR_INDEXER_ENABLED POSTING_V2_INDEXER_ENABLED; do
  value=${!flag:-false}
  [[ "$value" == true || "$value" == false ]] || exit 2
done
if [[ "${POSTING_CREATOR_INDEXER_ENABLED:-false}" == true ]]; then
  : "${CREATOR_OPEN_RELEASE_JSON:?Reviewed creator release required}"
  : "${BASE_MAINNET_RPC_URL:?Mainnet RPC required}"
fi
if [[ "${POSTING_V2_INDEXER_ENABLED:-false}" == true ]]; then
  : "${BASE_MAINNET_RPC_URL:?Mainnet RPC required}"
fi
children=()
stop_children() {
  trap - EXIT TERM INT
  if (( ${#children[@]} )); then
    kill -TERM "${children[@]}" 2>/dev/null || true
    wait "${children[@]}" 2>/dev/null || true
  fi
}
trap stop_children EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
common=("DATABASE_URL=$DATABASE_URL" "RENDER_SERVICE_ID=$RENDER_SERVICE_ID"   "RENDER_GIT_COMMIT=$RENDER_GIT_COMMIT" "RECOVERY_READER_MODE=active")
legacy=("${common[@]}" "BASE_INDEXER_PROTOCOL=autonomous-v1" "BASE_INDEXER_NETWORK=base-mainnet"   "BASE_INDEXER_FACTORY_CONTRACT=0x082c52131aaf0c56e76b075f895eab6fcab6d2f9")
for key in RUST_LOG PUBLIC_BASE_URL BASE_MAINNET_RPC_URL BASE_INDEXER_RPC_URL   BASE_INDEXER_START_BLOCK BASE_INDEXER_FROM_BLOCK BASE_INDEXER_BOOTSTRAP_ARCHIVE_RPC_URL   BASE_INDEXER_POLL_SECONDS BASE_INDEXER_CONFIRMATIONS BASE_INDEXER_MAX_BLOCKS_PER_QUERY   BASE_INDEXER_REQUEST_ID BASE_INDEXER_RETRY_INITIAL_SECONDS BASE_INDEXER_RETRY_MAX_SECONDS   BASE_INDEXER_EXIT_AFTER_FAILURES; do
  if [[ -v "$key" ]]; then legacy+=("$key=${!key}"); fi
done
env -i "${legacy[@]}" /usr/local/bin/recovery-reader &
children+=("$!")
if [[ "${POSTING_CREATOR_INDEXER_ENABLED:-false}" == true ]]; then
  env -i "${common[@]}" BASE_INDEXER_PROTOCOL=creator-open-v1     BASE_MAINNET_RPC_URL="$BASE_MAINNET_RPC_URL" CREATOR_OPEN_RELEASE_JSON="$CREATOR_OPEN_RELEASE_JSON"     RUST_LOG="${RUST_LOG:-info}" /usr/local/bin/recovery-reader &
  children+=("$!")
fi
if [[ "${POSTING_V2_INDEXER_ENABLED:-false}" == true ]]; then
  env -i "${common[@]}" BASE_INDEXER_PROTOCOL=autonomous-v2 BASE_INDEXER_NETWORK=base-mainnet     BASE_INDEXER_FACTORY_CONTRACT=0xc33e2ae33bb9580837ea59df18e57fa1039ae58a     BASE_INDEXER_START_BLOCK=52275492 BASE_INDEXER_RPC_URL="$BASE_MAINNET_RPC_URL"     PUBLIC_BASE_URL="${PUBLIC_BASE_URL:-https://api.agentizable.ai}"     RUST_LOG="${RUST_LOG:-info}" /usr/local/bin/recovery-reader &
  children+=("$!")
fi
set +e
wait -n "${children[@]}"
status=$?
(( status != 0 )) || status=1
exit "$status"
