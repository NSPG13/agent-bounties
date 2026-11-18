#!/usr/bin/env bash
# Fixed public build identity; no dependency or credential access.
set -euo pipefail
[[ $# == 2 ]] || exit 2
service=$1
revision=$2
[[ "$service" =~ ^srv-[a-z0-9]{20}$ ]] || exit 2
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || exit 2
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
[[ -s "$root/ops/recovery-reader-denied-services.txt" ]] || exit 2
if grep -Fxq -- "$service" "$root/ops/recovery-reader-denied-services.txt"; then
  echo 'Existing service cannot host the recovery funding launcher.' >&2
  exit 1
fi
