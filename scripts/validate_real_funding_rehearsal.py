import json
import sys
from pathlib import Path
from typing import Any


EXPECTED_BASE_SEPOLIA_USDC = "0x036CbD53842c5426634e7929541eC2318f3dCF7e"
EXPECTED_BASE_MAINNET_USDC = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"


def expect(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def is_hex(value: Any, byte_length: int) -> bool:
    if not isinstance(value, str) or not value.startswith("0x"):
        return False
    raw = value[2:]
    return len(raw) == byte_length * 2 and all(
        character in "0123456789abcdefABCDEF" for character in raw
    )


def check_discovery(discovery: dict[str, Any]) -> None:
    expect(
        discovery["schema"]
        == "https://agentbounties.org/schemas/discovery-manifest.v2.json",
        "rehearsal discovery must use v2",
    )
    protocol = discovery["protocol"]
    expect(
        protocol["version"] == "agent-bounties/autonomous-v1",
        "rehearsal discovery must identify autonomous-v1",
    )
    expect(
        protocol["operator_settlement_signer"] is False,
        "autonomous-v1 must not have an operator settlement signer",
    )
    expect(
        "BountySettled" in protocol["payout_authority"],
        "discovery must bind payout evidence to BountySettled",
    )
    endpoints = discovery["endpoints"]
    for name in (
        "autonomous_terms_publish",
        "autonomous_bounty_feed",
        "autonomous_verification_jobs",
        "autonomous_events",
        "autonomous_creation_plan",
        "autonomous_contribution_plan",
        "autonomous_claim_plan",
        "autonomous_submission_plan",
        "autonomous_module_settlement_plan",
        "autonomous_attestation_settlement_plan",
    ):
        expect(isinstance(endpoints.get(name), str), f"missing autonomous endpoint: {name}")
    for retired in (
        "base_escrow_events",
        "base_release_queue",
        "base_funding_plan",
        "base_refund_plan",
        "base_dispute_plan",
    ):
        expect(retired not in endpoints, f"retired V1 endpoint leaked: {retired}")
    tools = discovery["agent_tools"]
    expect(
        "plan_autonomous_bounty_creation" in tools
        and "plan_autonomous_attestation_settlement" in tools,
        "autonomous planning tools are incomplete",
    )
    expect(
        not any(tool.startswith(("plan_base_", "reconcile_base_")) for tool in tools),
        "retired V1 tools leaked into discovery",
    )
    modes = {mode["name"]: mode for mode in discovery["verification_modes"]}
    expect("deterministic_module" in modes, "deterministic verifier mode missing")
    expect("signed_quorum" in modes, "signed verifier quorum missing")
    expect(
        modes["ai_judge_quorum"]["minimum_threshold"] >= 2,
        "AI judge quorum must require at least two committed judges",
    )
    actions = [action["kind"] for action in discovery["post_value_loop"]["actions"]]
    for action in (
        "share_verified_value",
        "tell_your_human",
        "star_upvote_repo",
        "post_own_bounty",
    ):
        expect(action in actions, f"post-value distribution action missing: {action}")


def check_readiness(readiness: dict[str, Any]) -> None:
    expect(readiness["local_rehearsal_ready"] is True, "local rehearsal must be ready")
    expect(readiness["network"] == "Base Sepolia", "readiness must describe Base Sepolia")
    expect(readiness["network_chain_id"] == 84532, "Base Sepolia chain id is wrong")
    expect(
        readiness["network_native_usdc_token_address"] == EXPECTED_BASE_SEPOLIA_USDC,
        "Base Sepolia native USDC address is wrong",
    )
    expect(
        readiness["supplied_usdc_token_matches_native"] is True,
        "readiness must reject a non-native settlement token",
    )
    checks = {check["name"]: check for check in readiness["checks"]}
    expect(
        checks["local deterministic rehearsal"]["configured"] is True,
        "deterministic rehearsal must be configured",
    )
    expect(
        checks["Autonomous bounty factory"]["configured"] is True,
        "rehearsal factory configuration must be accepted",
    )
    expect(
        "Autonomous Base event indexing" in checks,
        "readiness must describe canonical autonomous indexing",
    )
    boundaries = "\n".join(readiness["evidence_boundaries"])
    for phrase in (
        "signature or transaction hash is not funding evidence",
        "confirmed canonical FundingAdded events",
        "confirmed BountySettled event",
        "Stripe and PayPal are optional convenience on-ramps",
    ):
        expect(phrase in boundaries, f"missing evidence boundary: {phrase}")
    for retired in ("EscrowReleased", "createEscrow", "release(uint256"):
        expect(retired not in boundaries, f"retired V1 evidence leaked: {retired}")


def check_deployment(deployment: dict[str, Any]) -> None:
    expect(deployment["schema_version"] == 2, "deployment manifest must use schema v2")
    expect(
        deployment["protocol_version"] == "agent-bounties/autonomous-v1",
        "deployment manifest must identify autonomous-v1",
    )
    status = deployment["status"]
    factory = deployment["factory"]
    if status == "pending_external_review_and_deployment":
        expect(factory["contract"] is None, "pending factory must be null")
        expect(factory["implementation"] is None, "pending implementation must be null")
    elif status == "active":
        expect(deployment["network"] == "base-mainnet", "active deployment must be Base mainnet")
        expect(deployment["chain_id"] == 8453, "active deployment chain id is wrong")
        expect(
            deployment["native_usdc"].lower() == EXPECTED_BASE_MAINNET_USDC.lower(),
            "active deployment must use native Base USDC",
        )
        expect(is_hex(factory["contract"], 20), "active factory address is invalid")
        expect(
            is_hex(factory["implementation"], 20),
            "active implementation address is invalid",
        )
        expect(
            factory["contract"].lower() != factory["implementation"].lower(),
            "factory and implementation must be distinct",
        )
        expect(
            is_hex(factory["deployment_transaction"], 32),
            "active deployment transaction is invalid",
        )
        expect(
            isinstance(factory["deployment_block"], int) and factory["deployment_block"] > 0,
            "active deployment block is invalid",
        )
        expect(
            is_hex(factory["runtime_code_hash"], 32),
            "active factory runtime hash is invalid",
        )
        expect(
            is_hex(factory["implementation_runtime_code_hash"], 32),
            "active implementation runtime hash is invalid",
        )
    else:
        raise AssertionError(f"unsupported deployment status: {status}")
    expect(
        deployment["policy"]["operator_settlement_signer"] is False,
        "deployment policy must not have an operator settlement signer",
    )
    expect(
        deployment["policy"]["hosted_paid_state_requires_confirmed_bounty_settled_event"]
        is True,
        "deployment policy must require canonical payout evidence",
    )


def main() -> None:
    out_dir = Path(sys.argv[1] if len(sys.argv) > 1 else "target/real-funding-rehearsal")
    discovery = load_json(out_dir / "autonomous-discovery.json")
    readiness = load_json(out_dir / "autonomous-readiness.json")
    deployment = load_json(out_dir / "base-mainnet-deployment.json")
    check_discovery(discovery)
    check_readiness(readiness)
    check_deployment(deployment)
    print(
        "validated autonomous funding rehearsal: v2 discovery, canonical Base USDC "
        "configuration, deployment-state evidence, immutable verifier modes, and "
        "BountySettled evidence boundaries"
    )


if __name__ == "__main__":
    main()
