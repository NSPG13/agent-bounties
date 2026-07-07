#!/usr/bin/env bash
set -euo pipefail

mode="${1:-core}"
if [[ "$mode" != "core" && "$mode" != "full" ]]; then
  echo "usage: scripts/preflight.sh [core|full]" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if command -v cygpath >/dev/null 2>&1 && [[ -n "${USERPROFILE:-}" ]]; then
  export PATH="$(cygpath -u "$USERPROFILE")/.cargo/bin:$PATH"
fi
if [[ -d "/mnt/c/Users/${USER:-}/.cargo/bin" ]]; then
  export PATH="/mnt/c/Users/${USER}/.cargo/bin:$PATH"
fi
export PATH="$repo_root/.tools/foundry:$PATH"

failures=()
warnings=()

require_command() {
  local name="$1"
  local purpose="$2"
  if ! command -v "$name" >/dev/null 2>&1; then
    failures+=("$name is required for $purpose")
  fi
}

optional_command() {
  local name="$1"
  local purpose="$2"
  if ! command -v "$name" >/dev/null 2>&1; then
    warnings+=("$name is optional; install it for $purpose")
  fi
}

require_command cargo "Rust workspace commands"
require_command npm "TypeScript SDK checks"
if ! command -v python3 >/dev/null 2>&1 && ! command -v python >/dev/null 2>&1 && ! command -v py >/dev/null 2>&1 && ! command -v py.exe >/dev/null 2>&1; then
  failures+=("python3, python, or py is required for Python SDK checks")
fi

if [[ "$mode" == "full" ]]; then
  require_command forge "Base escrow contract tests"
  minimum_free_mb=4096
else
  minimum_free_mb=512
fi

optional_command docker "Postgres durability smoke tests"

free_mb="$(df -Pm "$repo_root" | awk 'NR==2 { print $4 }')"
if [[ -z "$free_mb" ]]; then
  failures+=("could not determine free disk for $repo_root")
elif (( free_mb < minimum_free_mb )); then
  failures+=("free disk is ${free_mb}MB; $mode mode expects at least ${minimum_free_mb}MB. If this is a development checkout, run cargo clean to remove generated target output.")
fi

for warning in "${warnings[@]}"; do
  echo "warning: $warning" >&2
done

if (( ${#failures[@]} > 0 )); then
  echo "preflight failed:" >&2
  for failure in "${failures[@]}"; do
    echo "- $failure" >&2
  done
  exit 1
fi

echo "preflight=$mode ok free_mb=$free_mb"
