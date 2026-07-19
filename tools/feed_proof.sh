#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

API_BASE_URL="${API_BASE_URL:-http://127.0.0.1:8080}"
PROOF_DIR="feeds/proof"

cleanup() { rm -rf "$PROOF_DIR"; }
trap cleanup EXIT

python3 tools/feed_generator.py \
  --api-base-url "$API_BASE_URL" \
  --output-dir "$PROOF_DIR"

echo "Live RSS, Atom, and JSON Feed validation passed."
