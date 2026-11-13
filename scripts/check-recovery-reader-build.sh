#!/usr/bin/env bash
# Nonsecret immutable build inputs only. Never evaluates an input as shell code.
set -euo pipefail
[[ $# == 3 ]] || exit 2
service=$1
revision=$2
capability=$3
[[ "$service" =~ ^srv-[a-z0-9]{20}$ ]] || exit 2
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || exit 2
[[ "$capability" == standby || "$capability" == reader ]] || exit 2
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
[[ -r "$root/ops/recovery-reader-denied-services.txt" && -s "$root/ops/recovery-reader-denied-services.txt" ]] || exit 2
if grep -Fxq -- "$service" "$root/ops/recovery-reader-denied-services.txt"; then
  echo 'Existing service cannot host the recovery reader.' >&2
  exit 1
fi
