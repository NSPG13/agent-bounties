#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if command -v python3 >/dev/null 2>&1; then
  python_cmd=(python3)
elif command -v python >/dev/null 2>&1; then
  python_cmd=(python)
elif command -v py >/dev/null 2>&1; then
  python_cmd=(py -3)
elif command -v py.exe >/dev/null 2>&1; then
  python_cmd=(py.exe -3)
else
  echo "python3, python, or py is required" >&2
  exit 127
fi

exec "${python_cmd[@]}" "$script_dir/github_proof_comment.py" "$@"
