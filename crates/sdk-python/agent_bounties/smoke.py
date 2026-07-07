import argparse
import json
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
        isinstance(discovery.get("endpoints", {}).get("llms_txt"), str),
        "discovery manifest missing llms.txt endpoint",
    )
    _require(
        isinstance(discovery.get("endpoints", {}).get("base_fetch_rpc_logs"), str),
        "discovery manifest missing Base RPC fetch endpoint",
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
        reviewed_approval["bounty"]["status"] == "Claimable",
        "risk approval did not create a claimable bounty",
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
    args = parser.parse_args()

    result = exercise_surface(AgentBountiesClient(args.base_url))
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
