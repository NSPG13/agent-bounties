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
Invoke-Checked { & (Join-Path $repoRoot "scripts\check-x402-relayer.ps1") }
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
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_reconcile_github_bounty_labels.py -v }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_diagnose_hosted_api.py -v }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_github_audience_audit.py -v }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_ruleset_drift_check.py -v }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_relay_autonomous_action.py -v }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m pip install -r scripts\requirements-wallet.txt }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_local_delegate_wallet.py -v }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_self_heal.py -v }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\self_heal.py bench --policy ops\self-healing-policy.json --fixtures ops\fixtures\recovery-cases.json --output target\tmp\recovery-bench.json }
Invoke-Checked { cargo run -p cli -- github-proof-comment-plan --bounty-id 00000000-0000-0000-0000-000000000001 --proof-url https://agentbounties.local/public/proofs/example --verifier-summary "GitHub CI passed" }
Invoke-Checked { cargo run -p cli -- discovery --public-base-url https://agentbounties.local --mcp-base-url https://agentbounties.local/mcp }
Invoke-Checked { cargo run -p cli -- discovery-report --input-fixture crates\cli\fixtures\discovery_answers.json --json-out target\tmp\discovery-report.json --markdown-out target\tmp\discovery-report.md }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\check-site.py }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\check-migration-history.py }
Invoke-Checked { node --check skills\agent-bounties\scripts\check-in.mjs }
Invoke-Checked { node --test scripts\test_agent_bounties_openclaw_skill.mjs }
Invoke-Checked { node scripts\test-autonomous-wallet-flow.js }
Invoke-Checked { node --check tools\autonomous-activation.js }
Invoke-Checked { node scripts\test-autonomous-activation-console.js }
Invoke-Checked { node --check tools\canonical-child-verifier-deployment.js }
Invoke-Checked { node scripts\test-canonical-child-verifier-deployment-console.js }
Invoke-Checked { node --check tools\base-sepolia-sponsor-activation.js }
Invoke-Checked { node scripts\test-base-sepolia-sponsor-activation-console.js }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m pip install -r scripts\requirements-attest.txt }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\check-render-blueprint.py }
Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\test_stage_review_contract_root.py -v }
Invoke-Checked { cargo run -p cli -- docs-contract-check }
Invoke-Checked { cargo run -p cli -- demo }
Invoke-Checked { cargo run -p cli -- pooled-funding-demo }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m py_compile crates\sdk-python\agent_bounties\client.py crates\sdk-python\agent_bounties\smoke.py crates\sdk-python\agent_bounties\__init__.py }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m pip install -e crates\sdk-python }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m unittest discover -s crates\sdk-python\tests -t crates\sdk-python -v }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m py_compile scripts\diagnose_hosted_api.py scripts\test_diagnose_hosted_api.py scripts\github_audience_audit.py scripts\test_github_audience_audit.py scripts\ruleset_drift_check.py scripts\test_ruleset_drift_check.py scripts\relay_autonomous_action.py scripts\test_relay_autonomous_action.py scripts\local_delegate_wallet.py scripts\test_local_delegate_wallet.py scripts\self_heal.py scripts\test_self_heal.py scripts\github_issue_plan_comment.py scripts\github_funding_comment.py scripts\github_claim_comment.py scripts\github_proof_comment.py scripts\sync_hosted_bounty_inventory.py scripts\test_sync_hosted_bounty_inventory.py scripts\reconcile_github_bounty_labels.py scripts\test_reconcile_github_bounty_labels.py scripts\validate_real_funding_rehearsal.py }
Invoke-Checked { & $pythonCommand.Source @pythonArgs -m py_compile scripts\check-site.py scripts\check-migration-history.py scripts\check-render-blueprint.py scripts\stage_review_contract_root.py scripts\test_stage_review_contract_root.py scripts\rehearse_autonomous_activation.py scripts\build_canonical_child_verifier_bundle.py scripts\rehearse_canonical_child_verifier.py scripts\build_base_sepolia_sponsor_bundle.py }
Pop-Location

Push-Location (Join-Path $repoRoot "crates\sdk-typescript")
Invoke-Checked { npm ci }
Invoke-Checked { npm test }
Invoke-Checked { npm run check:examples }
Pop-Location

Push-Location (Join-Path $repoRoot "contracts\base-escrow")
Invoke-Checked { forge test --fuzz-runs 1000 }
Pop-Location

$sepoliaBundlePath = Join-Path $repoRoot "deployments\base-sepolia-sponsor-activation.json"
$sepoliaBundle = Get-Content $sepoliaBundlePath -Raw | ConvertFrom-Json
$sepoliaCheck = Join-Path $repoRoot "target\tmp\base-sepolia-sponsor-activation.json"
Invoke-Checked {
    & $pythonCommand.Source @pythonArgs scripts\build_base_sepolia_sponsor_bundle.py `
        --offline `
        --deployer $sepoliaBundle.deployer `
        --grant-signer $sepoliaBundle.grant_signer `
        --deployer-nonce $sepoliaBundle.preflight_block.deployer_nonce `
        --source-commit $sepoliaBundle.source_commit `
        --preflight-block-number $sepoliaBundle.preflight_block.number `
        --preflight-block-hash $sepoliaBundle.preflight_block.hash `
        --preflight-deployer-eth-wei $sepoliaBundle.preflight_block.deployer_eth_wei `
        --preflight-deployer-usdc-base-units $sepoliaBundle.preflight_block.deployer_usdc_base_units `
        --output $sepoliaCheck
}
$generatedSepoliaBundle = Get-Content $sepoliaCheck -Raw | ConvertFrom-Json | ConvertTo-Json -Depth 100 -Compress
$committedSepoliaBundle = $sepoliaBundle | ConvertTo-Json -Depth 100 -Compress
if ($generatedSepoliaBundle -ne $committedSepoliaBundle) {
    throw "deployments/base-sepolia-sponsor-activation.json is stale; regenerate it with build_base_sepolia_sponsor_bundle.py"
}

$verifierBundlePath = Join-Path $repoRoot "deployments\canonical-child-verifier-base-mainnet-deployment.json"
$verifierBundle = Get-Content $verifierBundlePath -Raw | ConvertFrom-Json
$verifierCheck = Join-Path $repoRoot "target\tmp\canonical-child-verifier-base-mainnet-deployment.json"
Invoke-Checked {
    & $pythonCommand.Source @pythonArgs scripts\build_canonical_child_verifier_bundle.py `
        --deployer $verifierBundle.deployment.from `
        --deployer-nonce $verifierBundle.deployment.deployer_nonce `
        --source-commit $verifierBundle.source_commit `
        --preflight-block-number $verifierBundle.preflight_block.number `
        --preflight-block-hash $verifierBundle.preflight_block.hash `
        --output $verifierCheck
}
$generatedVerifierBundle = Get-Content $verifierCheck -Raw | ConvertFrom-Json | ConvertTo-Json -Depth 100 -Compress
$committedVerifierBundle = $verifierBundle | ConvertTo-Json -Depth 100 -Compress
if ($generatedVerifierBundle -ne $committedVerifierBundle) {
    throw "deployments/canonical-child-verifier-base-mainnet-deployment.json is stale; regenerate it with build_canonical_child_verifier_bundle.py"
}

$activationCheck = Join-Path $repoRoot "target\tmp\base-mainnet-activation.json"
Invoke-Checked {
    cargo run -p cli -- autonomous-activation-bundle `
        --deployer 0x884834E884d6e93462655A2820140aD03E6747bC `
        --deployer-nonce 4 `
        --output $activationCheck
}
$generatedActivation = Get-Content $activationCheck -Raw | ConvertFrom-Json | ConvertTo-Json -Depth 100 -Compress
$committedActivation = Get-Content (Join-Path $repoRoot "deployments\base-mainnet-activation.json") -Raw | ConvertFrom-Json | ConvertTo-Json -Depth 100 -Compress
if ($generatedActivation -ne $committedActivation) {
    throw "deployments/base-mainnet-activation.json is stale; regenerate it with the autonomous-activation-bundle command"
}

$seedActivationCheck = Join-Path $repoRoot "target\tmp\canonical-child-seeds-base-mainnet.json"
Invoke-Checked {
    cargo run -p cli -- autonomous-activation-bundle `
        --manifest bounties/autonomous-v1/canonical-child-seeds-manifest.json `
        --deployer 0x884834E884d6e93462655A2820140aD03E6747bC `
        --deployer-nonce 4 `
        --output $seedActivationCheck
}
$generatedSeedActivation = Get-Content $seedActivationCheck -Raw | ConvertFrom-Json | ConvertTo-Json -Depth 100 -Compress
$committedSeedActivation = Get-Content (Join-Path $repoRoot "deployments\canonical-child-seeds-base-mainnet.json") -Raw | ConvertFrom-Json | ConvertTo-Json -Depth 100 -Compress
if ($generatedSeedActivation -ne $committedSeedActivation) {
    throw "deployments/canonical-child-seeds-base-mainnet.json is stale; regenerate it from the canonical child seed manifest"
}
