import argparse
import json
import time
import uuid

from .client import AgentBountiesClient, hash_artifact


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def exercise_surface(client: AgentBountiesClient) -> dict:
    suffix = f"{int(time.time())}-{uuid.uuid4().hex[:8]}"

    discovery = client.get_discovery_manifest()
    _require(
        discovery.get("schema")
        == "https://agentbounties.org/schemas/discovery-manifest.v2.json",
        "discovery manifest missing v2 schema",
    )
    _require(
        discovery.get("protocol", {}).get("version")
        == "agent-bounties/autonomous-v1",
        "discovery manifest missing autonomous-v1",
    )
    _require(
        discovery.get("protocol", {}).get("operator_settlement_signer") is False,
        "autonomous protocol must not expose an operator settlement signer",
    )
    tools = discovery.get("agent_tools", [])
    for tool in (
        "list_opportunities",
        "create_discovery_subscription",
        "get_discovery_subscription",
        "delete_discovery_subscription",
        "get_opportunity_conversion_funnel",
        "analyze_bounty_fit",
        "list_autonomous_bounties",
        "get_solver_leaderboard",
        "plan_autonomous_canonical_child_terms",
        "plan_autonomous_bounty_creation",
        "plan_autonomous_bounty_contribution",
        "agent_native_claim",
        "plan_autonomous_bounty_claim",
        "plan_autonomous_bounty_submission",
        "prepare_autonomous_bounty_submission",
        "plan_autonomous_bounty_submission_authorization",
        "plan_autonomous_module_settlement",
        "plan_autonomous_attestation_settlement",
    ):
        _require(tool in tools, f"discovery manifest missing {tool}")
    _require(
        not any(tool.startswith(("plan_base_", "reconcile_base_")) for tool in tools),
        "discovery manifest leaked retired V1 tools",
    )
    endpoints = discovery.get("endpoints", {})
    for endpoint in (
        "opportunities",
        "discovery_subscriptions",
        "discovery_subscription",
        "opportunity_conversion_funnel",
        "autonomous_bounty_analysis",
        "autonomous_creation_plan",
        "autonomous_agent_native_claim",
        "autonomous_bounty_feed",
        "autonomous_verification_jobs",
        "autonomous_events",
        "autonomous_terms_publish",
    ):
        _require(isinstance(endpoints.get(endpoint), str), f"missing endpoint {endpoint}")

    schema = client.get_discovery_manifest_schema()
    _require(schema.get("$id") == discovery.get("schema"), "schema id mismatch")
    required = schema.get("properties", {}).get("endpoints", {}).get("required", [])
    _require(
        "autonomous_creation_plan" in required
        and "autonomous_attestation_settlement_plan" in required,
        "v2 schema does not require autonomous endpoints",
    )

    route = client.route_blocked_goal(
        "Fix the Python autonomous SDK smoke",
        "The result has deterministic acceptance criteria.",
        1_000_000,
    )
    _require("capability_class" in route, "router response missing capability class")
    _require(
        client.decode_autonomous_bounty_events([]) == [],
        "autonomous event decoder rejected an empty batch",
    )
    _require(
        isinstance(client.list_autonomous_bounty_events("base-mainnet"), list),
        "autonomous event feed must be an array",
    )
    _require(
        isinstance(
            client.list_opportunities(network="base-mainnet", view="recent").get("items"),
            list,
        ),
        "opportunity projection items must be an array",
    )

    for method in (
        "list_opportunities",
        "create_discovery_subscription",
        "get_discovery_subscription",
        "delete_discovery_subscription",
        "get_opportunity_conversion_funnel",
        "analyze_bounty_fit",
        "publish_autonomous_bounty_terms",
        "get_solver_leaderboard",
        "publish_autonomous_submission_evidence",
        "plan_autonomous_canonical_child_terms",
        "plan_autonomous_bounty_creation",
        "plan_autonomous_bounty_authorized_creation",
        "plan_autonomous_bounty_contribution",
        "plan_autonomous_bounty_authorized_contribution",
        "agent_native_claim",
        "plan_autonomous_bounty_claim",
        "plan_autonomous_bounty_authorized_claim",
        "plan_autonomous_bounty_submission",
        "prepare_autonomous_bounty_submission",
        "plan_autonomous_bounty_submission_authorization",
        "plan_autonomous_verification_attestation",
        "plan_autonomous_module_settlement",
        "plan_autonomous_attestation_settlement",
        "plan_autonomous_cancel",
        "plan_autonomous_refund_withdrawal",
    ):
        _require(callable(getattr(client, method, None)), f"Python SDK missing {method}")

    solver = client.register_agent(
        f"python-autonomous-smoke-solver-{suffix}",
        "0x2222222222222222222222222222222222222222",
    )
    bounty = client.open_pooled_bounty(
        title=f"Python SDK deterministic local loop {suffix}",
        template_slug="extract-data-to-schema",
        target_amount_minor=1_000,
        currency="usdc",
        funding_mode="Simulated",
        privacy="Public",
    )
    bounty_id = bounty["id"]
    funded = client.add_funding_contribution(
        bounty_id,
        amount_minor=1_000,
        currency="usdc",
        rail="Simulated",
        external_reference=f"python-autonomous-smoke-{suffix}",
    )
    _require(funded["bounty"]["status"] == "Claimable", "funding did not reach target")
    claimed = client.claim_bounty(bounty_id, solver["id"])
    _require(claimed["status"] == "Claimed", "claim did not become active")
    artifact = json.dumps({"sdk": "python", "autonomous": True}, separators=(",", ":"))
    submission = client.submit_result(
        bounty_id,
        solver["id"],
        "memory://python-autonomous-smoke.json",
        artifact,
    )
    proof = client.request_verification(
        bounty_id,
        submission["id"],
        hash_artifact(artifact),
        verifier_kind="JsonSchema",
    )
    _require("proof_hash" in proof, "verification did not produce proof_hash")
    status = client.get_bounty_status(bounty_id)
    _require(status["bounty"]["status"] == "Paid", "local loop did not reach Paid")

    return {
        "sdk": "python",
        "schema": discovery["schema"],
        "protocol": discovery["protocol"]["version"],
        "bounty_id": bounty_id,
        "status": status["bounty"]["status"],
        "autonomous_tools": len(tools),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Run the Agent Bounties Python SDK smoke.")
    parser.add_argument("--base-url", default="http://127.0.0.1:8080")
    args = parser.parse_args()
    print(
        json.dumps(
            exercise_surface(AgentBountiesClient(args.base_url)),
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
