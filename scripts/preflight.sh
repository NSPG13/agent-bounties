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
if command -v powershell.exe >/dev/null 2>&1; then
  windows_userprofile="$(powershell.exe -NoProfile -Command '$env:USERPROFILE' 2>/dev/null | tr -d '\r' || true)"
  if [[ -n "$windows_userprofile" ]]; then
    if command -v wslpath >/dev/null 2>&1; then
      export PATH="$(wslpath -u "$windows_userprofile")/.cargo/bin:$PATH"
    elif command -v cygpath >/dev/null 2>&1; then
      export PATH="$(cygpath -u "$windows_userprofile")/.cargo/bin:$PATH"
    fi
  fi
fi
if [[ -d "/mnt/c/Users/${USER:-}/.cargo/bin" ]]; then
  export PATH="/mnt/c/Users/${USER}/.cargo/bin:$PATH"
fi
export PATH="$repo_root/.tools/foundry:$PATH"
if ! command -v cargo >/dev/null 2>&1 && command -v cargo.exe >/dev/null 2>&1; then
  cargo() { cargo.exe "$@"; }
fi
if ! command -v rustc >/dev/null 2>&1 && command -v rustc.exe >/dev/null 2>&1; then
  rustc() { rustc.exe "$@"; }
fi

failures=()
warnings=()
minimum_rust_major=1
minimum_rust_minor=88

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

require_minimum_version() {
  local name="$1"
  local purpose="$2"
  if ! command -v "$name" >/dev/null 2>&1; then
    return
  fi

  local version_output version major minor
  version_output="$("$name" --version 2>/dev/null || true)"
  version="$(printf '%s' "$version_output" | awk '{ print $2 }')"
  major="${version%%.*}"
  minor="${version#*.}"
  minor="${minor%%.*}"
  if [[ ! "$major" =~ ^[0-9]+$ || ! "$minor" =~ ^[0-9]+$ ]]; then
    failures+=("could not parse $name version for $purpose")
    return
  fi

  if (( major < minimum_rust_major || (major == minimum_rust_major && minor < minimum_rust_minor) )); then
    failures+=("$name ${minimum_rust_major}.${minimum_rust_minor} or newer is required for $purpose; found $version_output")
  fi
}

require_command rustc "Rust compiler commands"
require_command cargo "Rust workspace commands"
require_command npm "TypeScript SDK checks"
require_minimum_version rustc "the locked dependency graph"
require_minimum_version cargo "the locked dependency graph"
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
