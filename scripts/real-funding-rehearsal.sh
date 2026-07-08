#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out_dir="${1:-target/real-funding-rehearsal}"

if command -v cygpath >/dev/null 2>&1 && [[ -n "${USERPROFILE:-}" ]]; then
  export PATH="$(cygpath -u "$USERPROFILE")/.cargo/bin:$PATH"
fi
if [[ -d "${HOME:-}/.cargo/bin" ]]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi
if [[ -d "/mnt/c/Users/${USER:-}/.cargo/bin" ]]; then
  export PATH="/mnt/c/Users/${USER}/.cargo/bin:$PATH"
fi
if ! command -v cargo >/dev/null 2>&1 && command -v cargo.exe >/dev/null 2>&1; then
  cargo() { cargo.exe "$@"; }
fi

if command -v python3 >/dev/null 2>&1; then
  python_cmd=(python3)
elif command -v python >/dev/null 2>&1; then
  python_cmd=(python)
elif command -v py >/dev/null 2>&1; then
  python_cmd=(py -3)
elif command -v py.exe >/dev/null 2>&1; then
  python_cmd=(py.exe -3)
else
  echo "python3, python, or py is required to validate real funding rehearsal artifacts" >&2
  exit 127
fi

cd "$repo_root"
mkdir -p "$out_dir"

cargo run -q -p cli -- funding-rehearsal-demo > "$out_dir/funding-rehearsal-demo.json"
cargo run -q -p cli -- real-funding-readiness \
  --network base-sepolia \
  --escrow-contract 0x1111111111111111111111111111111111111111 \
  --usdc-token 0x036CbD53842c5426634e7929541eC2318f3dCF7e \
  > "$out_dir/real-funding-readiness.json"

"${python_cmd[@]}" scripts/validate_real_funding_rehearsal.py "$out_dir"
