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
Invoke-Checked { cargo run -p cli -- stripe-plan --organization-id 00000000-0000-0000-0000-000000000001 --amount-minor 5000 --platform-url https://agentbounties.local }
Invoke-Checked { cargo run -p cli -- github-plan --repository agent-bounties/agent-bounties --issue-url https://github.com/agent-bounties/agent-bounties/issues/1 --title "[bounty]: Fix CI" --body-file examples/github-paid-bounty-issue.md }
Invoke-Checked { cargo run -p cli -- github-funding-comment-plan --repository agent-bounties/agent-bounties --issue-url https://github.com/agent-bounties/agent-bounties/issues/1 --title "[bounty]: Fix CI" --body-file examples/github-paid-bounty-issue.md --comment-body "/agent-bounty fund 5 USDC via BaseUsdcEscrow" --contributor-login check-script --comment-id 12345 }
Invoke-Checked { cargo run -p cli -- github-claim-comment-plan --repository agent-bounties/agent-bounties --issue-url https://github.com/agent-bounties/agent-bounties/issues/1 --title "[bounty]: Fix CI" --body-file examples/github-paid-bounty-issue.md --comment-body "/agent-bounty claim`nPlan: inspect CI logs and open a focused PR with local test output." --contributor-login check-script --comment-id 12346 --claim-age-minutes 5 }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\github_issue_plan_comment.py --self-test }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\github_funding_comment.py --self-test }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\github_claim_comment.py --self-test }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\github_proof_comment.py --self-test }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_sync_hosted_bounty_inventory.py -v }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_diagnose_hosted_api.py -v }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_github_audience_audit.py -v }
Invoke-Checked { cargo run -p cli -- github-proof-comment-plan --bounty-id 00000000-0000-0000-0000-000000000001 --proof-url https://agentbounties.local/public/proofs/example --verifier-summary "GitHub CI passed" }
Invoke-Checked { cargo run -p cli -- discovery --public-base-url https://agentbounties.local --mcp-base-url https://agentbounties.local/mcp }
Invoke-Checked { cargo run -p cli -- discovery-report --input-fixture crates\cli\fixtures\discovery_answers.json --json-out target\tmp\discovery-report.json --markdown-out target\tmp\discovery-report.md }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\check-site.py }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\check-migration-history.py }
Invoke-Checked { node --check skills\agent-bounties\scripts\check-in.mjs }
Invoke-Checked { node --test scripts\test_agent_bounties_openclaw_skill.mjs }
Invoke-Checked { node scripts\test-autonomous-wallet-flow.js }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m pip install -r scripts\requirements-attest.txt }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_base_deployment_attest.py -v }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\check-render-blueprint.py }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_stage_review_contract_root.py -v }
Invoke-Checked { cargo run -p cli -- docs-contract-check }
Invoke-Checked { cargo run -p cli -- demo }
Invoke-Checked { cargo run -p cli -- pooled-funding-demo }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m py_compile crates\sdk-python\agent_bounties\client.py crates\sdk-python\agent_bounties\smoke.py crates\sdk-python\agent_bounties\__init__.py }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m py_compile scripts\diagnose_hosted_api.py scripts\test_diagnose_hosted_api.py scripts\github_audience_audit.py scripts\test_github_audience_audit.py scripts\github_issue_plan_comment.py scripts\github_funding_comment.py scripts\github_claim_comment.py scripts\github_proof_comment.py scripts\sync_hosted_bounty_inventory.py scripts\test_sync_hosted_bounty_inventory.py scripts\validate_real_funding_rehearsal.py }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m py_compile scripts\check-site.py scripts\check-migration-history.py scripts\check-render-blueprint.py scripts\stage_review_contract_root.py scripts\test_stage_review_contract_root.py scripts\base_deployment_attest.py scripts\test_base_deployment_attest.py scripts\build_base_attest_fixtures.py scripts\rehearse_autonomous_activation.py }
Pop-Location

Push-Location (Join-Path $repoRoot "crates\sdk-typescript")
Invoke-Checked { npm ci }
Invoke-Checked { npm run build }
Invoke-Checked { npm run check:examples }
Pop-Location

Push-Location (Join-Path $repoRoot "contracts\base-escrow")
Invoke-Checked { forge test --fuzz-runs 1000 }
Pop-Location

$activationCheck = Join-Path $repoRoot "target\tmp\base-mainnet-activation.json"
Invoke-Checked {
    cargo run -p cli -- autonomous-activation-bundle `
        --deployer 0x884834E884d6e93462655A2820140aD03E6747bC `
        --deployer-nonce 4 `
        --output $activationCheck
}
if ((Get-Content $activationCheck -Raw) -ne (Get-Content (Join-Path $repoRoot "deployments\base-mainnet-activation.json") -Raw)) {
    throw "deployments/base-mainnet-activation.json is stale; regenerate it with the autonomous-activation-bundle command"
}
