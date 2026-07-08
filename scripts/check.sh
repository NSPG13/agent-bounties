#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if command -v cygpath >/dev/null 2>&1 && [[ -n "${USERPROFILE:-}" ]]; then
  export PATH="$(cygpath -u "$USERPROFILE")/.cargo/bin:$PATH"
fi
if [[ -d "/mnt/c/Users/${USER:-}/.cargo/bin" ]]; then
  export PATH="/mnt/c/Users/${USER}/.cargo/bin:$PATH"
fi
export PATH="$repo_root/.tools/foundry:$PATH"
if ! command -v cargo >/dev/null 2>&1 && command -v cargo.exe >/dev/null 2>&1; then
  cargo() { cargo.exe "$@"; }
fi
if ! command -v forge >/dev/null 2>&1 && command -v forge.exe >/dev/null 2>&1; then
  forge() { forge.exe "$@"; }
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
  echo "python3, python, or py is required to compile the Python SDK" >&2
  exit 127
fi

cd "$repo_root"
bash "$repo_root/scripts/preflight.sh" full
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
cargo build -p api -p mcp-server
cargo run -p cli -- service-smoke-spawn \
  --api-base-url http://127.0.0.1:18080 \
  --mcp-base-url http://127.0.0.1:18090
cargo run -p cli -- bountybench
cargo run -p cli -- abusebench
cargo run -p cli -- judgebench
cargo run -p cli -- eval-loops
cargo run -p cli -- risk-policy
cargo run -p cli -- base-plan \
  --network base-mainnet \
  --escrow-contract 0x1111111111111111111111111111111111111111 \
  --token 0x3333333333333333333333333333333333333333 \
  --amount-minor 1000000
cargo run -p cli -- base-decode-demo
cargo run -p cli -- base-log-query \
  --escrow-contract 0x1111111111111111111111111111111111111111 \
  --from-block 0
cargo run -p cli -- base-release-queue-demo \
  --escrow-contract 0x1111111111111111111111111111111111111111 \
  --platform-fee-wallet 0x4444444444444444444444444444444444444444
cargo run -p cli -- base-refund-plan \
  --escrow-contract 0x1111111111111111111111111111111111111111 \
  --onchain-escrow-id 1 \
  --reason-hash 0x5555555555555555555555555555555555555555555555555555555555555555
cargo run -p cli -- base-dispute-plan \
  --escrow-contract 0x1111111111111111111111111111111111111111 \
  --onchain-escrow-id 1 \
  --dispute-hash 0x6666666666666666666666666666666666666666666666666666666666666666
cargo run -p cli -- base-sepolia-runbook \
  --settlement-signer 0x5555555555555555555555555555555555555555 \
  --escrow-contract 0x1111111111111111111111111111111111111111 \
  --usdc-token 0x3333333333333333333333333333333333333333
cargo run -p cli -- stripe-plan \
  --organization-id 00000000-0000-0000-0000-000000000001 \
  --amount-minor 5000 \
  --platform-url https://agentbounties.local
cargo run -p cli -- github-plan \
  --repository agent-bounties/agent-bounties \
  --issue-url https://github.com/agent-bounties/agent-bounties/issues/1 \
  --title "[bounty]: Fix CI" \
  --body-file examples/github-paid-bounty-issue.md
cargo run -p cli -- github-funding-comment-plan \
  --repository agent-bounties/agent-bounties \
  --issue-url https://github.com/agent-bounties/agent-bounties/issues/1 \
  --title "[bounty]: Fix CI" \
  --body-file examples/github-paid-bounty-issue.md \
  --comment-body "/agent-bounty fund 5 USDC via BaseUsdcEscrow" \
  --contributor-login check-script \
  --comment-id 12345
"${python_cmd[@]}" scripts/github_issue_plan_comment.py --self-test
"${python_cmd[@]}" scripts/github_funding_comment.py --self-test
"${python_cmd[@]}" scripts/github_proof_comment.py --self-test
cargo run -p cli -- github-proof-comment-plan \
  --bounty-id 00000000-0000-0000-0000-000000000001 \
  --proof-url https://agentbounties.local/public/proofs/example \
  --verifier-summary "GitHub CI passed"
cargo run -p cli -- discovery \
  --public-base-url https://agentbounties.local \
  --mcp-base-url https://agentbounties.local/mcp
cargo run -p cli -- docs-contract-check
cargo run -p cli -- demo
cargo run -p cli -- pooled-funding-demo
cargo run -p cli -- funding-rehearsal-demo
"${python_cmd[@]}" -m py_compile \
  crates/sdk-python/agent_bounties/client.py \
  crates/sdk-python/agent_bounties/smoke.py \
  crates/sdk-python/agent_bounties/__init__.py \
  crates/sdk-python/examples/cofund_claim.py \
  scripts/github_issue_plan_comment.py \
  scripts/github_funding_comment.py \
  scripts/github_proof_comment.py

cd "$repo_root/crates/sdk-typescript"
npm ci
npm run build
npm run check:examples

cd "$repo_root/contracts/base-escrow"
forge test
