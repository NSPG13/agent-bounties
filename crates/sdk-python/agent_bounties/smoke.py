import argparse
import json
import os
import time
import uuid

import httpx

from .client import AgentBountiesClient, hash_artifact


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def exercise_surface(client: AgentBountiesClient) -> dict:
    suffix = f"{int(time.time())}-{uuid.uuid4().hex[:8]}"

    discovery = client.get_discovery_manifest()
    _require("agent_entrypoints" in discovery, "discovery manifest missing agent entrypoints")
    _require(
        discovery.get("schema") == "https://agentbounties.org/schemas/discovery-manifest.v1.json",
        "discovery manifest missing expected schema id",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("llms_txt"), str),
        "discovery manifest missing llms.txt endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("discovery_schema"), str),
        "discovery manifest missing schema endpoint",
    )
    discovery_schema = client.get_discovery_manifest_schema()
    _require(
        discovery_schema.get("$id") == discovery.get("schema"),
        "discovery schema id did not match manifest schema id",
    )
    _require(
        "agent_entrypoints" in discovery_schema.get("required", []),
        "discovery schema must require agent entrypoints",
    )
    _require(
        "payment_rails" in discovery_schema.get("required", []),
        "discovery schema must require payment rails",
    )
    endpoint_required = (
        discovery_schema.get("properties", {})
        .get("endpoints", {})
        .get("required", [])
    )
    _require(
        "discovery_schema" in endpoint_required,
        "discovery schema must require its own endpoint",
    )
    _require(
        "github_issue_template" in endpoint_required,
        "discovery schema must require the GitHub bounty issue template endpoint",
    )
    _require(
        "base_escrow_events" in endpoint_required,
        "discovery schema must require the Base escrow event endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("base_fetch_rpc_logs"), str),
        "discovery manifest missing Base RPC fetch endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("base_escrow_events"), str),
        "discovery manifest missing Base escrow event reconciliation endpoint",
    )
    _require(
        isinstance(
            discovery.get("endpoints", {}).get("base_broadcast_signed_transaction"),
            str,
        ),
        "discovery manifest missing Base signed transaction broadcast endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("base_transaction_receipt"), str),
        "discovery manifest missing Base transaction receipt endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("base_funding_plan"), str),
        "discovery manifest missing Base funding planning endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("base_refund_plan"), str),
        "discovery manifest missing Base refund planning endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("base_dispute_plan"), str),
        "discovery manifest missing Base dispute planning endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("stripe_live_checkout_top_ups"), str),
        "discovery manifest missing live Stripe Checkout execution endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("stripe_live_connect_accounts"), str),
        "discovery manifest missing live Stripe Connect execution endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("github_issue_bounty_plan"), str),
        "discovery manifest missing GitHub issue bounty planner endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("github_proof_comment_plan"), str),
        "discovery manifest missing GitHub proof comment planner endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("eval_runs"), str),
        "discovery manifest missing eval run history endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("risk_policy"), str),
        "discovery manifest missing risk policy endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("risk_events"), str),
        "discovery manifest missing risk review events endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("risk_reviews"), str),
        "discovery manifest missing risk review records endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("risk_bounty_approvals"), str),
        "discovery manifest missing risk bounty approval endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("risk_payout_approvals"), str),
        "discovery manifest missing risk payout approval endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("agent_paid_status"), str),
        "discovery manifest missing agent payout status endpoint",
    )
    risk_policy = client.get_risk_policy()
    _require(
        risk_policy["low_value_usdc_cap_minor"] == 10_000_000,
        "risk policy did not expose the low-value Base USDC cap",
    )
    _require(
        risk_policy["ai_judges_can_authorize_payment"] is False,
        "risk policy must state that AI judges cannot authorize payment",
    )
    try:
        client.post_bounty(
            f"Python SDK review-required bounty {suffix}",
            "fix-ci-failure",
            25_000_000,
            "usdc",
            "BaseUsdcEscrow",
            "Public",
        )
    except httpx.HTTPStatusError as error:
        _require(error.response.status_code == 400, "over-cap bounty should return 400")
    else:
        raise AssertionError("over-cap Base USDC bounty should require review")
    risk_events = client.get_risk_events(action="NeedsReview", surface="Bounty", limit=10)
    review_event = next(
        (
            event
            for event in risk_events
            if event["action"] == "NeedsReview"
            and any("low-value cap" in reason for reason in event["reasons"])
        ),
        None,
    )
    _require(
        review_event is not None,
        "risk events did not include the review-required bounty",
    )
    reviewed_approval = client.approve_risk_bounty(
        review_event["id"],
        f"Python SDK review-required bounty {suffix}",
        "fix-ci-failure",
        25_000_000,
        "usdc",
        "BaseUsdcEscrow",
        "Public",
        "python-sdk-smoke",
        "Approved review-required bounty during Python SDK smoke.",
    )
    _require(
        reviewed_approval["bounty"]["status"] == "Unfunded",
        "risk approval did not create a funding-ready bounty",
    )
    _require(
        reviewed_approval["review"]["outcome"] == "Approved",
        "risk approval did not record review",
    )
    risk_reviews = client.list_risk_reviews()
    _require(
        any(
            review["outcome"] == "Approved"
            and review["bounty_id"] == reviewed_approval["bounty"]["id"]
            for review in risk_reviews
        ),
        "risk review list did not include approval",
    )

    review_solver = client.register_agent(
        f"python-sdk-review-solver-{suffix}",
        "0x2222222222222222222222222222222222222222",
    )
    reviewed_bounty_id = reviewed_approval["bounty"]["id"]
    reviewed_created_event = {
        "id": str(uuid.uuid4()),
        "log_key": f"python-sdk-review:{reviewed_bounty_id}:created",
        "tx_hash": f"0x{uuid.uuid4().hex}",
        "block_number": 2,
        "onchain_escrow_id": 2,
        "bounty_id": reviewed_bounty_id,
        "kind": "Created",
        "status": "Funded",
        "token": "0x3333333333333333333333333333333333333333",
        "amount": {"amount": 25_000_000, "currency": "usdc"},
        "terms_hash": reviewed_approval["bounty"]["terms_hash"],
        "proof_hash": None,
        "reason_hash": None,
        "dispute_hash": None,
        "occurred_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    reviewed_funding = client.reconcile_base_escrow_event(reviewed_created_event)
    _require(
        reviewed_funding["bounty"]["status"] == "Claimable",
        "reviewed Base escrow create event did not make bounty claimable",
    )
    reviewed_claim = client.claim_bounty(reviewed_bounty_id, review_solver["id"])
    _require(
        reviewed_claim["status"] == "Claimed",
        "reviewed bounty claim did not move to Claimed",
    )
    reviewed_submission = client.submit_result(
        reviewed_bounty_id,
        review_solver["id"],
        "https://github.com/example/repo/actions/runs/1",
        json.dumps({"check": "green"}, separators=(",", ":")),
    )
    reviewed_evidence = {"check_conclusion": "success", "check_name": "test"}
    try:
        client.request_verification(
            reviewed_bounty_id,
            reviewed_submission["id"],
            "not-used-by-github-ci",
            evidence=reviewed_evidence,
        )
    except httpx.HTTPStatusError as error:
        _require(
            error.response.status_code == 400,
            "high-value payout review should return 400",
        )
    else:
        raise AssertionError("high-value payout should require review before verification")
    payout_events = client.get_risk_events(
        action="NeedsReview",
        surface="Payout",
        bounty_id=reviewed_bounty_id,
        limit=10,
    )
    payout_event = next(
        (
            event
            for event in payout_events
            if event["action"] == "NeedsReview"
            and any("automatic release cap" in reason for reason in event["reasons"])
        ),
        None,
    )
    _require(payout_event is not None, "payout risk event was not recorded")
    payout_review = client.approve_risk_payout(
        payout_event["id"],
        "python-sdk-smoke",
        "Approved payout review during Python SDK smoke.",
    )
    _require(payout_review["surface"] == "Payout", "payout approval used wrong surface")
    reviewed_proof = client.request_verification(
        reviewed_bounty_id,
        reviewed_submission["id"],
        "not-used-by-github-ci",
        evidence=reviewed_evidence,
        approved_risk_event_id=payout_event["id"],
    )
    _require("proof_hash" in reviewed_proof, "reviewed payout verification missing proof")
    reviewed_status = client.get_bounty_status(reviewed_bounty_id)
    _require(
        reviewed_status["bounty"]["status"] == "Payable",
        "reviewed payout bounty is not Payable",
    )

    route = client.route_blocked_goal(
        "Patch the SDK live smoke bounty flow",
        "The agent needs a small coding task with deterministic verification.",
        1_000_000,
    )
    _require("capability_class" in route, "route response missing capability_class")
    capability_class = route["capability_class"]
    template_slug = route.get("template_slug") or "small-code-change"

    requester = client.register_agent(f"python-sdk-requester-{suffix}")
    solver = client.register_agent(
        f"python-sdk-solver-{suffix}",
        "0x2222222222222222222222222222222222222222",
    )
    stripe_checkout = client.plan_stripe_checkout_top_up(requester["id"], 5_000)
    _require(
        stripe_checkout["endpoint"] == "/v1/checkout/sessions",
        "Stripe Checkout top-up planner used the wrong endpoint",
    )
    stripe_connect = client.plan_stripe_connect_account(solver["id"])
    _require(
        stripe_connect["request"]["endpoint"] == "/v2/core/accounts",
        "Stripe Connect account planner used the wrong endpoint",
    )
    github_issue_plan = client.plan_github_issue_bounty(
        "agent-bounties/agent-bounties",
        "https://github.com/agent-bounties/agent-bounties/issues/1",
        "[bounty]: Fix CI",
        "### Goal\nFix the failing CI check.\n\n### Acceptance criteria\nThe test job is green and the patch explains the failure.\n\n### Template\nfix-ci-failure\n\n### Suggested amount\n10 USDC\n",
    )
    _require(github_issue_plan["ready"] is True, "GitHub issue planner rejected valid issue")
    _require(
        github_issue_plan["check"]["conclusion"] == "Success",
        "GitHub issue planner did not produce a success check",
    )
    github_proof_plan = client.plan_github_proof_comment(
        solver["id"],
        "https://agentbounties.local/public/proofs/sdk-smoke",
        "GitHub CI passed",
    )
    _require(
        len(github_proof_plan["fingerprint"]) == 64,
        "GitHub proof comment planner did not produce a stable fingerprint",
    )
    base_log_query = client.plan_base_log_query(
        "0x1111111111111111111111111111111111111111",
        123,
        request_id=11,
    )
    _require(base_log_query["method"] == "eth_getLogs", "Base log query used the wrong method")
    _require(
        base_log_query["params"][0]["fromBlock"] == "0x7b",
        "Base log query did not encode fromBlock",
    )
    base_rpc_log_report = client.reconcile_base_rpc_logs(
        {
            "jsonrpc": "2.0",
            "id": 11,
            "result": [],
        }
    )
    _require(
        base_rpc_log_report["decoded_events"] == 0,
        "Base RPC log reconciliation did not accept an empty provider response",
    )

    client.register_capability(
        solver["id"],
        capability_class,
        [template_slug],
        500_000,
        1_000_000,
        supported_verifiers=["JsonSchema"],
    )
    capability_feed = client.list_capability_feed()
    _require(
        any(item["agent_id"] == solver["id"] for item in capability_feed),
        "registered solver missing from public capability feed",
    )
    capability_search = client.search_capabilities(
        capability_class=capability_class,
        template_slug=template_slug,
        currency="usdc",
        max_price_minor=1_000_000,
    )
    _require(
        any(item["agent_id"] == solver["id"] for item in capability_search),
        "registered solver missing from filtered capability search",
    )

    help_request = client.create_help_request(
        requester["id"],
        "Patch the SDK live smoke bounty flow",
        "Return a JSON artifact that proves the client can complete work.",
        1_000_000,
    )
    quotes = client.request_quotes(help_request["id"])
    _require(len(quotes["quotes"]) >= 1, "quote flow did not return a solver quote")

    bounty = client.fund_quote_as_bounty(
        quotes["quotes"][0]["id"],
        "Python SDK live smoke bounty",
        "BaseUsdcEscrow",
    )
    bounty_id = bounty["id"]

    funding_plan = client.plan_base_funding(
        bounty_id,
        "0x1111111111111111111111111111111111111111",
        "0x2222222222222222222222222222222222222222",
        "0x3333333333333333333333333333333333333333",
        network="base-mainnet",
    )
    _require(
        funding_plan["network"]["chain_id"] == 8_453,
        "Base funding plan did not honor explicit Base mainnet network",
    )
    _require(
        funding_plan["create"]["terms_hash"] == bounty["terms_hash"],
        "Base funding plan did not use bounty terms hash",
    )
    _require(
        funding_plan["funding"]["create_escrow"]["function"]
        == "createEscrow(bytes32,address,uint256,bytes32)",
        "Base funding plan used the wrong createEscrow function",
    )

    created_event = {
        "id": str(uuid.uuid4()),
        "log_key": f"python-sdk-smoke:{bounty_id}:created",
        "tx_hash": f"0x{uuid.uuid4().hex}",
        "block_number": 1,
        "onchain_escrow_id": 1,
        "bounty_id": bounty_id,
        "kind": "Created",
        "status": "Funded",
        "token": "0x3333333333333333333333333333333333333333",
        "amount": {"amount": 1_000_000, "currency": "usdc"},
        "terms_hash": bounty["terms_hash"],
        "proof_hash": None,
        "reason_hash": None,
        "dispute_hash": None,
        "occurred_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    escrow_reconciliation = client.reconcile_base_escrow_event(created_event)
    _require(
        escrow_reconciliation["bounty"]["status"] == "Claimable",
        "Base escrow create event did not make bounty claimable",
    )
    _require(
        escrow_reconciliation["escrow"]["status"] == "Funded",
        "Base escrow create event did not produce funded escrow state",
    )

    feed = client.list_public_bounty_feed()
    _require(
        any(item["bounty_id"] == bounty_id for item in feed),
        "funded SDK bounty missing from public feed",
    )

    claimed = client.claim_bounty(bounty_id, solver["id"])
    _require(claimed["status"] == "Claimed", "claim did not move bounty to Claimed")

    artifact_body = json.dumps({"sdk": "python", "ok": True}, separators=(",", ":"))
    submission = client.submit_result(
        bounty_id,
        solver["id"],
        "s3://agent-bounties/python-sdk-smoke/artifact.json",
        artifact_body,
    )
    proof = client.request_verification(
        bounty_id,
        submission["id"],
        hash_artifact(artifact_body),
        "JsonSchema",
    )
    _require("proof_hash" in proof, "verification did not return proof_hash")

    status = client.get_bounty_status(bounty_id)
    _require(status["bounty"]["status"] == "Payable", "verified bounty is not Payable")
    paid = client.get_paid_status(bounty_id)
    _require(len(paid["settlements"]) >= 1, "paid status missing settlement records")
    agent_paid = client.get_agent_paid_status(solver["id"])
    _require(len(agent_paid["payouts"]) >= 1, "agent paid status missing payout lines")
    _require(
        any(
            total["currency"] == "usdc" and total["pending_minor"] == 900_000
            for total in agent_paid["totals"]
        ),
        "agent paid status missing pending USDC total",
    )
    release_queue = client.list_base_release_queue(
        "0x1111111111111111111111111111111111111111",
        "0x4444444444444444444444444444444444444444",
    )
    release_queue_item = next(
        (item for item in release_queue if item["bounty"]["id"] == bounty_id),
        None,
    )
    _require(
        release_queue_item is not None,
        "Base release queue did not return the SDK smoke bounty",
    )
    _require(release_queue_item["ready"] is True, "Base release queue did not become ready")
    _require(
        release_queue_item["release_plan"]["network"]["chain_id"] == 84_532,
        "Base release queue did not default to Base Sepolia",
    )
    release_plan = client.plan_base_release(
        bounty_id,
        "0x1111111111111111111111111111111111111111",
        "0x4444444444444444444444444444444444444444",
        network="base-mainnet",
    )
    _require(
        release_plan["network"]["chain_id"] == 8_453,
        "Base release plan did not honor explicit Base mainnet network",
    )
    _require(
        release_plan["transaction"]["function"] == "release(uint256,address[],uint256[],bytes32)",
        "Base release plan used the wrong transaction function",
    )
    eval_loops = client.run_eval_loops()
    _require(eval_loops["passed"] is True, "eval loop suite did not pass")
    _require(len(eval_loops["loops"]) == 5, "eval loop count changed")
    eval_runs = client.get_eval_runs()
    _require(
        any(run["suite"] == "EvalLoops/all-v0" for run in eval_runs),
        "eval run history did not record EvalLoops/all-v0",
    )

    return {
        "sdk_smoke": "ok",
        "language": "python",
        "bounty_id": bounty_id,
        "status": status["bounty"]["status"],
        "settlements": len(paid["settlements"]),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Run a live Agent Bounties Python SDK smoke.")
    parser.add_argument("--base-url", default="http://127.0.0.1:8080")
    parser.add_argument("--operator-api-token", default=os.getenv("OPERATOR_API_TOKEN"))
    args = parser.parse_args()

    result = exercise_surface(
        AgentBountiesClient(args.base_url, operator_api_token=args.operator_api_token)
    )
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
