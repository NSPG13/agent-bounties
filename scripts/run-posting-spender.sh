#!/usr/bin/env bash
set -euo pipefail
case "${RENDER_SERVICE_ID:-}" in
  srv-da32s6ek1f9s73duedlg) posting_expected_protocol=open-competition-v2-keeper ;;
  srv-da32s70ae00c73a01ffg) posting_expected_protocol=open-competition-v2-broker ;;
  *) echo 'Posting spender image is restricted to the reviewed existing workers.' >&2; exit 1 ;;
esac
if [[ "${POSTING_SENDS_PAUSED:-true}" != false ]]; then
  echo 'posting_spender=paused; no worker or signer has started'
  exec sleep infinity
fi
if [[ "${BASE_INDEXER_PROTOCOL:-}" != "$posting_expected_protocol" ]]; then
  echo 'Posting spender protocol does not match its reviewed service.' >&2
  exit 1
fi
exec /usr/local/bin/agent-bounties-service
