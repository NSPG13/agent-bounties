#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if command -v python3 >/dev/null 2>&1; then
  exec python3 "$repo_root/scripts/review_external_pr.py" "$@"
elif command -v python >/dev/null 2>&1; then
  exec python "$repo_root/scripts/review_external_pr.py" "$@"
fi
echo "python3 or python is required for external PR review" >&2
exit 127
