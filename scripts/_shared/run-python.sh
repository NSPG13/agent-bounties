#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if command -v cygpath >/dev/null 2>&1 && [[ -n "${USERPROFILE:-}" ]]; then
  export PATH="$(cygpath -u "$USERPROFILE")/.cargo/bin:$PATH"
fi
if [[ -d "/mnt/c/Users/${USER:-}/.cargo/bin" ]]; then
  export PATH="/mnt/c/Users/${USER}/.cargo/bin:$PATH"
fi

for candidate in python3 python "py -3" "py.exe -3"; do
  read -r -a python_cmd <<< "$candidate"
  if command -v "${python_cmd[0]}" >/dev/null 2>&1; then
    exec "${python_cmd[@]}" "$script_dir/$1" "${@:2}"
  fi
done
echo "python3, python, or py is required" >&2
exit 127
