import argparse
import json
import time
import uuid

from agent_bounties import AgentBountiesClient, hash_artifact


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def run_example(client: AgentBountiesClient) -> dict:
    suffix = f"{int(time.time())}-{uuid.uuid4().hex[:8]}"

    discovery = client.get_discovery_manifest()
    endpoints = discovery.get("endpoints", {})
    require(isinstance(endpoints.get("llms_txt"), str), "discovery missing llms.txt")
    require(
        isinstance(endpoints.get("pooled_bounties"), str),
        "discovery missing pooled bounty endpoint",
    )
    require(
        isinstance(endpoints.get("base_funding_plan"), str),
        "discovery missing Base funding planner endpoint",
    )

    solver = client.register_agent(
        f"python-example-solver-{suffix}",
        "0x2222222222222222222222222222222222222222",
    )
    first_funder = client.register_agent(f"python-example-funder-a-{suffix}")
    second_funder = client.register_agent(f"python-example-funder-b-{suffix}")

    bounty = client.open_pooled_bounty(
        title=f"Python SDK co-funded local bounty {suffix}",
        template_slug="extract-data-to-schema",
        target_amount_minor=1_000_000,
        currency="usdc",
        funding_mode="Simulated",
        privacy="Public",
    )
    bounty_id = bounty["id"]

    partial = client.add_funding_contribution(
        bounty_id,
        amount_minor=400_000,
        currency="usdc",
        rail="Simulated",
        contributor_agent_id=first_funder["id"],
        external_reference=f"python-example-{suffix}-funding-a",
    )
    require(partial["bounty"]["status"] == "Unfunded", "partial funding became claimable")
    require(
        partial["funding_summary"]["remaining"]["amount"] == 600_000,
        "partial funding remaining amount drifted",
    )

    funded = client.add_funding_contribution(
        bounty_id,
        amount_minor=600_000,
        currency="usdc",
        rail="Simulated",
        contributor_agent_id=second_funder["id"],
        external_reference=f"python-example-{suffix}-funding-b",
    )
    require(funded["bounty"]["status"] == "Claimable", "fully funded bounty is not claimable")
    require(funded["funding_summary"]["claimable"] is True, "funding summary is not claimable")

    claimable = client.list_claimable_bounties()
    require(any(item["id"] == bounty_id for item in claimable), "bounty missing from claimable feed")

    claimed = client.claim_bounty(bounty_id, solver["id"])
    require(claimed["status"] == "Claimed", "claim did not move bounty to Claimed")

    artifact_body = json.dumps({"sdk": "python", "cofunded": True}, separators=(",", ":"))
    submission = client.submit_result(
        bounty_id,
        solver["id"],
        "memory://python-sdk-cofund-claim.json",
        artifact_body,
    )
    proof = client.request_verification(
        bounty_id,
        submission["id"],
        hash_artifact(artifact_body),
        verifier_kind="JsonSchema",
    )
    require("proof_hash" in proof, "verification did not return proof_hash")

    status = client.get_bounty_status(bounty_id)
    require(status["bounty"]["status"] == "Paid", "simulated bounty did not settle as paid")
    paid = client.get_paid_status(bounty_id)
    require(len(paid["settlements"]) == 1, "paid status missing simulated settlement")

    base_bounty = client.open_pooled_bounty(
        title=f"Python SDK Base Sepolia funding plan {suffix}",
        template_slug="fix-ci-failure",
        target_amount_minor=1_000_000,
        currency="usdc",
        funding_mode="BaseUsdcEscrow",
        privacy="Public",
    )
    base_plan = client.plan_base_funding(
        base_bounty["id"],
        "0x1111111111111111111111111111111111111111",
        "0x2222222222222222222222222222222222222222",
        "0x3333333333333333333333333333333333333333",
        network="base-sepolia",
    )
    require(base_plan["network"]["chain_id"] == 84_532, "Base plan did not use Base Sepolia")
    require(
        base_plan["funding"]["create_escrow"]["function"]
        == "createEscrow(bytes32,address,uint256,bytes32)",
        "Base plan createEscrow function drifted",
    )

    return {
        "example": "python-cofund-claim",
        "bounty_id": bounty_id,
        "status": status["bounty"]["status"],
        "settlements": len(paid["settlements"]),
        "base_plan_network": base_plan["network"]["name"],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Run the Agent Bounties Python SDK co-funding example.")
    parser.add_argument("--base-url", default="http://127.0.0.1:8080")
    args = parser.parse_args()

    result = run_example(AgentBountiesClient(args.base_url))
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
