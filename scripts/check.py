#!/usr/bin/env python3
"""Run the deterministic repository gate with one cross-platform inventory."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
TMP = ROOT / "target" / "tmp"


def executable(name: str) -> str:
    for candidate in (name, f"{name}.exe", f"{name}.cmd"):
        if path := shutil.which(candidate):
            return path
    return name


def run(*args: object, cwd: pathlib.Path = ROOT) -> None:
    command = [str(value) for value in args]
    print(f"+ {subprocess.list2cmdline(command)}", flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def run_many(commands: list[list[str]]) -> None:
    for command in commands:
        run(*command)


def py(*args: str) -> None:
    run(sys.executable, *args)


def platform_script(name: str, platform: str, *args: str) -> None:
    script = ROOT / "scripts" / f"{name}.{'ps1' if platform == 'powershell' else 'sh'}"
    if platform == "powershell":
        run(executable("powershell"), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script, *args)
    else:
        run(executable("bash"), script, *args)


def compare_json_or_bytes(expected: pathlib.Path, actual: pathlib.Path, platform: str) -> None:
    matches = (
        json.loads(expected.read_text(encoding="utf-8"))
        == json.loads(actual.read_text(encoding="utf-8"))
        if platform == "powershell"
        else expected.read_bytes() == actual.read_bytes()
    )
    if not matches:
        raise RuntimeError(f"{expected.relative_to(ROOT)} is stale; regenerate it")


def compile_python(platform: str) -> None:
    sdk = [
        "crates/sdk-python/agent_bounties/client.py",
        "crates/sdk-python/agent_bounties/smoke.py",
        "crates/sdk-python/agent_bounties/__init__.py",
    ]
    scripts = [
        "scripts/_shared/github_actions.py", "scripts/_shared/evm.py", "scripts/_shared/rpc.py",
        "scripts/github_issue_plan_comment.py", "scripts/github_funding_comment.py",
        "scripts/github_claim_comment.py", "scripts/github_proof_comment.py",
        "scripts/sync_hosted_bounty_inventory.py", "scripts/test_sync_hosted_bounty_inventory.py",
        "scripts/reconcile_github_bounty_labels.py", "scripts/test_reconcile_github_bounty_labels.py",
        "scripts/diagnose_hosted_api.py", "scripts/test_diagnose_hosted_api.py",
        "scripts/github_audience_audit.py", "scripts/test_github_audience_audit.py",
        "scripts/ruleset_drift_check.py", "scripts/test_ruleset_drift_check.py",
        "scripts/code_size_report.py", "scripts/test_code_size_report.py",
        "scripts/test_mcp_tool_registry.py", "scripts/test_shared_evm.py", "scripts/test_shared_rpc.py",
        "scripts/relay_autonomous_action.py", "scripts/test_relay_autonomous_action.py",
        "scripts/relay_bounded_wallet_action.py", "scripts/test_relay_bounded_wallet_action.py",
        "scripts/bounded_agent_create.py", "scripts/plan_bounded_agent_budget.py",
        "scripts/test_bounded_agent_budget.py", "scripts/local_delegate_wallet.py",
        "scripts/test_local_delegate_wallet.py", "scripts/self_heal.py", "scripts/test_self_heal.py",
        "scripts/leaderboard_reward_pipeline.py", "scripts/test_leaderboard_reward_pipeline.py",
        "scripts/standing_meta_v3_deploy.py", "scripts/test_standing_meta_v3_deploy.py",
        "scripts/activate_standing_meta_v3_replacements.py",
        "scripts/test_activate_standing_meta_v3_replacements.py",
        "scripts/check-site.py", "scripts/check-migration-history.py", "scripts/check-render-blueprint.py",
        "scripts/review_external_pr.py", "scripts/test_review_external_pr.py",
        "scripts/stage_review_contract_root.py", "scripts/test_stage_review_contract_root.py",
        "scripts/validate_real_funding_rehearsal.py", "scripts/rehearse_autonomous_activation.py",
        "scripts/build_canonical_child_verifier_bundle.py",
        "scripts/rehearse_canonical_child_verifier.py", "scripts/build_base_sepolia_sponsor_bundle.py",
        "scripts/competitor_intelligence.py", "scripts/test_competitor_intelligence.py",
    ]
    if platform == "powershell":
        first = """
scripts/code_size_report.py scripts/test_code_size_report.py scripts/test_mcp_tool_registry.py
scripts/_shared/github_actions.py scripts/_shared/evm.py scripts/_shared/rpc.py
scripts/test_shared_evm.py scripts/test_shared_rpc.py
scripts/diagnose_hosted_api.py scripts/test_diagnose_hosted_api.py
scripts/github_audience_audit.py scripts/test_github_audience_audit.py
scripts/ruleset_drift_check.py scripts/test_ruleset_drift_check.py
scripts/relay_autonomous_action.py scripts/test_relay_autonomous_action.py
scripts/relay_bounded_wallet_action.py scripts/test_relay_bounded_wallet_action.py
scripts/bounded_agent_create.py scripts/plan_bounded_agent_budget.py scripts/test_bounded_agent_budget.py
scripts/local_delegate_wallet.py scripts/test_local_delegate_wallet.py scripts/self_heal.py scripts/test_self_heal.py
scripts/leaderboard_reward_pipeline.py scripts/test_leaderboard_reward_pipeline.py
scripts/standing_meta_v3_deploy.py scripts/test_standing_meta_v3_deploy.py
scripts/activate_standing_meta_v3_replacements.py scripts/test_activate_standing_meta_v3_replacements.py
scripts/github_issue_plan_comment.py scripts/github_funding_comment.py scripts/github_claim_comment.py
scripts/github_proof_comment.py scripts/sync_hosted_bounty_inventory.py
scripts/test_sync_hosted_bounty_inventory.py scripts/reconcile_github_bounty_labels.py
scripts/test_reconcile_github_bounty_labels.py scripts/validate_real_funding_rehearsal.py
scripts/competitor_intelligence.py scripts/test_competitor_intelligence.py
""".split()
        second = scripts[scripts.index("scripts/check-site.py") :]
        py("-m", "py_compile", *sdk)
        py("-m", "pip", "install", "-e", "crates/sdk-python")
        py("-m", "unittest", "discover", "-s", "crates/sdk-python/tests", "-t", "crates/sdk-python", "-v")
        py("-m", "py_compile", *first)
        py("-m", "py_compile", *second)
    else:
        py("-m", "py_compile", *sdk, *scripts)
        py("-m", "pip", "install", "-e", "crates/sdk-python")
        py("-m", "unittest", "discover", "-s", "crates/sdk-python/tests", "-t", "crates/sdk-python", "-v")


def check_deployment_bundles(cargo: str, platform: str) -> None:
    TMP.mkdir(parents=True, exist_ok=True)
    sepolia = ROOT / "deployments/base-sepolia-sponsor-activation.json"
    data = json.loads(sepolia.read_text(encoding="utf-8"))
    sepolia_check = TMP / sepolia.name
    py(
        "scripts/build_base_sepolia_sponsor_bundle.py", "--offline",
        "--deployer", data["deployer"], "--grant-signer", data["grant_signer"],
        "--deployer-nonce", str(data["preflight_block"]["deployer_nonce"]),
        "--source-commit", data["source_commit"],
        "--preflight-block-number", str(data["preflight_block"]["number"]),
        "--preflight-block-hash", data["preflight_block"]["hash"],
        "--preflight-deployer-eth-wei", str(data["preflight_block"]["deployer_eth_wei"]),
        "--preflight-deployer-usdc-base-units", str(data["preflight_block"]["deployer_usdc_base_units"]),
        "--output", str(sepolia_check),
    )
    compare_json_or_bytes(sepolia, sepolia_check, platform)

    verifier = ROOT / "deployments/canonical-child-verifier-base-mainnet-deployment.json"
    data = json.loads(verifier.read_text(encoding="utf-8"))
    verifier_check = TMP / verifier.name
    py(
        "scripts/build_canonical_child_verifier_bundle.py",
        "--deployer", data["deployment"]["from"],
        "--deployer-nonce", str(data["deployment"]["deployer_nonce"]),
        "--source-commit", data["source_commit"],
        "--preflight-block-number", str(data["preflight_block"]["number"]),
        "--preflight-block-hash", data["preflight_block"]["hash"], "--output", str(verifier_check),
    )
    compare_json_or_bytes(verifier, verifier_check, platform)

    activation = TMP / "base-mainnet-activation.json"
    run(cargo, "run", "-p", "cli", "--", "autonomous-activation-bundle", "--deployer",
        "0x884834E884d6e93462655A2820140aD03E6747bC", "--deployer-nonce", "4", "--output", activation)
    compare_json_or_bytes(ROOT / "deployments/base-mainnet-activation.json", activation, platform)
    seeds = TMP / "canonical-child-seeds-base-mainnet.json"
    run(cargo, "run", "-p", "cli", "--", "autonomous-activation-bundle", "--manifest",
        "bounties/autonomous-v1/canonical-child-seeds-manifest.json", "--deployer",
        "0x884834E884d6e93462655A2820140aD03E6747bC", "--deployer-nonce", "4", "--output", seeds)
    compare_json_or_bytes(ROOT / "deployments/canonical-child-seeds-base-mainnet.json", seeds, platform)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=("powershell", "posix"), required=True)
    platform = parser.parse_args().platform
    cargo, node, npm, forge = map(executable, ("cargo", "node", "npm", "forge"))
    os.environ["PATH"] = os.pathsep.join((str(ROOT / ".tools/foundry"), os.environ.get("PATH", "")))

    platform_script("preflight", platform, "-Mode", "full") if platform == "powershell" else platform_script("preflight", platform, "full")
    run_many([
        [cargo, "fmt", "--all", "--", "--check"], [cargo, "clippy", "--workspace", "--", "-D", "warnings"],
        [cargo, "test", "--workspace"],
    ])
    platform_script("check-x402-relayer", platform)
    funding = "/agent-bounty fund 5 USDC via BaseUsdcEscrow" if platform == "powershell" else "/agent-bounty fund 5 USD via StripeFiatLedger"
    commands = [
        [cargo, "build", "-p", "api", "-p", "mcp-server"],
        [cargo, "run", "-p", "cli", "--", "service-smoke-spawn", "--api-base-url", "http://127.0.0.1:18080", "--mcp-base-url", "http://127.0.0.1:18090"],
        *[[cargo, "run", "-p", "cli", "--", name] for name in ("bountybench", "abusebench", "judgebench", "eval-loops", "risk-policy")],
        [cargo, "run", "-p", "cli", "--", "stripe-plan", "--organization-id", "00000000-0000-0000-0000-000000000001", "--amount-minor", "5000", "--platform-url", "https://agentbounties.local"],
        [cargo, "run", "-p", "cli", "--", "github-plan", "--repository", "agent-bounties/agent-bounties", "--issue-url", "https://github.com/agent-bounties/agent-bounties/issues/1", "--title", "[bounty]: Fix CI", "--body-file", "examples/github-paid-bounty-issue.md"],
        [cargo, "run", "-p", "cli", "--", "github-create-comment-plan", "--repository", "agent-bounties/agent-bounties", "--issue-url", "https://github.com/agent-bounties/agent-bounties/issues/1", "--title", "[bounty]: Fix CI", "--body-file", "examples/github-paid-bounty-issue.md", "--comment-body", "/agent-bounty create 25 USDC", "--contributor-login", "check-script", "--comment-id", "12344"],
        [cargo, "run", "-p", "cli", "--", "github-funding-comment-plan", "--repository", "agent-bounties/agent-bounties", "--issue-url", "https://github.com/agent-bounties/agent-bounties/issues/1", "--title", "[bounty]: Fix CI", "--body-file", "examples/github-paid-bounty-issue.md", "--comment-body", funding, "--contributor-login", "check-script", "--comment-id", "12345"],
        [cargo, "run", "-p", "cli", "--", "github-claim-comment-plan", "--repository", "agent-bounties/agent-bounties", "--issue-url", "https://github.com/agent-bounties/agent-bounties/issues/1", "--title", "[bounty]: Fix CI", "--body-file", "examples/github-paid-bounty-issue.md", "--comment-body", "/agent-bounty claim\nPlan: inspect CI logs and open a focused PR with local test output.", "--contributor-login", "check-script", "--comment-id", "12346", "--claim-age-minutes", "5"],
    ]
    run_many(commands)
    for name in ("github_issue_plan_comment", "github_create_comment", "github_funding_comment", "github_claim_comment", "github_proof_comment"):
        py(f"scripts/{name}.py", "--self-test")
    for name in ("sync_hosted_bounty_inventory", "reconcile_github_bounty_labels", "diagnose_hosted_api", "github_audience_audit", "ruleset_drift_check", "code_size_report", "mcp_tool_registry", "shared_rpc", "relay_autonomous_action", "relay_bounded_wallet_action", "bounded_agent_budget", "competitor_intelligence"):
        py(f"scripts/test_{name}.py", "-v")
    for name in ("standing_meta_v3_deploy", "activate_standing_meta_v3_replacements"):
        py(f"scripts/test_{name}.py", "-v")
    py("-m", "pip", "install", "-r", "scripts/requirements-wallet.txt")
    for name in ("local_delegate_wallet", "self_heal", "leaderboard_reward_pipeline"):
        py(f"scripts/test_{name}.py", "-v")
    py("scripts/self_heal.py", "bench", "--policy", "ops/self-healing-policy.json", "--fixtures", "ops/fixtures/recovery-cases.json", "--output", "target/tmp/recovery-bench.json")
    run_many([
        [cargo, "run", "-p", "cli", "--", "github-proof-comment-plan", "--bounty-id", "00000000-0000-0000-0000-000000000001", "--proof-url", "https://agentbounties.local/public/proofs/example", "--verifier-summary", "GitHub CI passed"],
        [cargo, "run", "-p", "cli", "--", "discovery", "--public-base-url", "https://agentbounties.local", "--mcp-base-url", "https://agentbounties.local/mcp"],
        [cargo, "run", "-p", "cli", "--", "discovery-report", "--input-fixture", "crates/cli/fixtures/discovery_answers.json", "--json-out", "target/tmp/discovery-report.json", "--markdown-out", "target/tmp/discovery-report.md"],
    ])
    py("scripts/check-site.py")
    py("scripts/check-migration-history.py")
    run_many([[node, *args] for args in (
        ["--check", "skills/agent-bounties/scripts/check-in.mjs"], ["--test", "scripts/test_agent_bounties_openclaw_skill.mjs"],
        ["benchmarks/standing-meta-v2/mcp-discovery/self-test.mjs"], ["benchmarks/direct-v1/agent-loop/self-test.mjs"],
        ["scripts/test-autonomous-wallet-flow.js"], ["--check", "tools/autonomous-activation.js"],
        ["scripts/test-autonomous-activation-console.js"], ["--check", "tools/canonical-child-verifier-deployment.js"],
        ["scripts/test-canonical-child-verifier-deployment-console.js"], ["--check", "tools/base-sepolia-sponsor-activation.js"],
        ["scripts/test-base-sepolia-sponsor-activation-console.js"],
        ["--check", "site/standing-meta-v3-migration.js"],
    )])
    py("-m", "pip", "install", "-r", "scripts/requirements-attest.txt")
    py("scripts/test_shared_evm.py", "-v")
    py("scripts/check-render-blueprint.py")
    py("scripts/test_mcp_tool_registry.py", "-v")
    py("scripts/test_review_external_pr.py", "-v")
    py("scripts/test_stage_review_contract_root.py", "-v")
    for name in ("docs-contract-check", "demo", "pooled-funding-demo"):
        run(cargo, "run", "-p", "cli", "--", name)
    compile_python(platform)
    run(npm, "ci", cwd=ROOT / "crates/sdk-typescript")
    run(npm, "test", cwd=ROOT / "crates/sdk-typescript")
    run(npm, "run", "check:examples", cwd=ROOT / "crates/sdk-typescript")
    run(forge, "test", "--fuzz-runs", "1000", cwd=ROOT / "contracts/base-escrow")
    run(forge, "build", "--force", "--ast", cwd=ROOT / "contracts/base-escrow")
    check_deployment_bundles(cargo, platform)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
