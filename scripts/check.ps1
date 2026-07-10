$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$repoRoot\.tools\foundry;$env:Path"

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)]
        [scriptblock] $Command
    )

    $global:LASTEXITCODE = 0
    & $Command
    $commandSucceeded = $?
    $exitCode = $global:LASTEXITCODE
    if (-not $commandSucceeded -or $exitCode -ne 0) {
        throw "Command failed with exit code $exitCode`: $Command"
    }
    $global:LASTEXITCODE = 0
}

$pythonCommand = Get-Command python -ErrorAction SilentlyContinue
$pythonArgs = @()
if (-not $pythonCommand) {
    $pythonCommand = Get-Command py -ErrorAction SilentlyContinue
    $pythonArgs = @("-3")
}
if (-not $pythonCommand) {
    throw "python or py is required to compile the Python SDK"
}

Push-Location $repoRoot
Invoke-Checked { & (Join-Path $repoRoot "scripts\preflight.ps1") -Mode full }
Invoke-Checked { cargo fmt --all -- --check }
Invoke-Checked { cargo clippy --workspace -- -D warnings }
Invoke-Checked { cargo test --workspace }
Invoke-Checked { cargo build -p api -p mcp-server }
Invoke-Checked { cargo run -p cli -- service-smoke-spawn --api-base-url http://127.0.0.1:18080 --mcp-base-url http://127.0.0.1:18090 }
Invoke-Checked { cargo run -p cli -- bountybench }
Invoke-Checked { cargo run -p cli -- abusebench }
Invoke-Checked { cargo run -p cli -- judgebench }
Invoke-Checked { cargo run -p cli -- eval-loops }
Invoke-Checked { cargo run -p cli -- risk-policy }
Invoke-Checked { cargo run -p cli -- base-plan --network base-mainnet --escrow-contract 0x1111111111111111111111111111111111111111 --token 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913 --amount-minor 1000000 }
Invoke-Checked { cargo run -p cli -- base-decode-demo }
Invoke-Checked { cargo run -p cli -- base-log-query --escrow-contract 0x1111111111111111111111111111111111111111 --from-block 0 }
Invoke-Checked { cargo run -p cli -- base-release-queue-demo --escrow-contract 0x1111111111111111111111111111111111111111 --platform-fee-wallet 0x4444444444444444444444444444444444444444 }
Invoke-Checked { cargo run -p cli -- base-refund-plan --escrow-contract 0x1111111111111111111111111111111111111111 --onchain-escrow-id 1 --reason-hash 0x5555555555555555555555555555555555555555555555555555555555555555 }
Invoke-Checked { cargo run -p cli -- base-dispute-plan --escrow-contract 0x1111111111111111111111111111111111111111 --onchain-escrow-id 1 --dispute-hash 0x6666666666666666666666666666666666666666666666666666666666666666 }
Invoke-Checked { cargo run -p cli -- base-sepolia-runbook --settlement-signer 0x5555555555555555555555555555555555555555 --escrow-contract 0x1111111111111111111111111111111111111111 --usdc-token 0x036CbD53842c5426634e7929541eC2318f3dCF7e }
Invoke-Checked { cargo run -p cli -- stripe-plan --organization-id 00000000-0000-0000-0000-000000000001 --amount-minor 5000 --platform-url https://agentbounties.local }
Invoke-Checked { cargo run -p cli -- github-plan --repository agent-bounties/agent-bounties --issue-url https://github.com/agent-bounties/agent-bounties/issues/1 --title "[bounty]: Fix CI" --body-file examples/github-paid-bounty-issue.md }
Invoke-Checked { cargo run -p cli -- github-funding-comment-plan --repository agent-bounties/agent-bounties --issue-url https://github.com/agent-bounties/agent-bounties/issues/1 --title "[bounty]: Fix CI" --body-file examples/github-paid-bounty-issue.md --comment-body "/agent-bounty fund 5 USDC via BaseUsdcEscrow" --contributor-login check-script --comment-id 12345 }
Invoke-Checked { cargo run -p cli -- github-claim-comment-plan --repository agent-bounties/agent-bounties --issue-url https://github.com/agent-bounties/agent-bounties/issues/1 --title "[bounty]: Fix CI" --body-file examples/github-paid-bounty-issue.md --comment-body "/agent-bounty claim`nPlan: inspect CI logs and open a focused PR with local test output." --contributor-login check-script --comment-id 12346 --claim-age-minutes 5 }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\github_issue_plan_comment.py --self-test }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\github_funding_comment.py --self-test }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\github_claim_comment.py --self-test }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\github_proof_comment.py --self-test }
Invoke-Checked { cargo run -p cli -- github-proof-comment-plan --bounty-id 00000000-0000-0000-0000-000000000001 --proof-url https://agentbounties.local/public/proofs/example --verifier-summary "GitHub CI passed" }
Invoke-Checked { cargo run -p cli -- discovery --public-base-url https://agentbounties.local --mcp-base-url https://agentbounties.local/mcp }
Invoke-Checked { cargo run -p cli -- discovery-report --input-fixture crates\cli\fixtures\discovery_answers.json --json-out target\tmp\discovery-report.json --markdown-out target\tmp\discovery-report.md }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\check-site.py }
Invoke-Checked { node scripts\test-base-wallet-flow.js }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m pip install -r scripts\requirements-attest.txt }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_base_deployment_attest.py -v }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\check-render-blueprint.py }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_stage_review_contract_root.py -v }
Invoke-Checked { cargo run -p cli -- docs-contract-check }
Invoke-Checked { cargo run -p cli -- demo }
Invoke-Checked { cargo run -p cli -- pooled-funding-demo }
Invoke-Checked { cargo run -p cli -- funding-rehearsal-demo }
Invoke-Checked { cargo run -p cli -- real-funding-readiness --network base-sepolia --escrow-contract 0x1111111111111111111111111111111111111111 --usdc-token 0x036CbD53842c5426634e7929541eC2318f3dCF7e }
Invoke-Checked { & (Join-Path $repoRoot "scripts\real-funding-rehearsal.ps1") }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m py_compile crates\sdk-python\agent_bounties\client.py crates\sdk-python\agent_bounties\smoke.py crates\sdk-python\agent_bounties\__init__.py crates\sdk-python\examples\cofund_claim.py }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m py_compile scripts\github_issue_plan_comment.py scripts\github_funding_comment.py scripts\github_claim_comment.py scripts\github_proof_comment.py scripts\validate_real_funding_rehearsal.py }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m py_compile scripts\check-site.py scripts\check-render-blueprint.py scripts\stage_review_contract_root.py scripts\test_stage_review_contract_root.py scripts\base_deployment_attest.py scripts\test_base_deployment_attest.py scripts\build_base_attest_fixtures.py }
Pop-Location

Push-Location (Join-Path $repoRoot "crates\sdk-typescript")
Invoke-Checked { npm ci }
Invoke-Checked { npm run build }
Invoke-Checked { npm run check:examples }
Pop-Location

Push-Location (Join-Path $repoRoot "contracts\base-escrow")
Invoke-Checked { forge test }
Pop-Location
