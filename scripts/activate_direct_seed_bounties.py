#!/usr/bin/env python3
"""Idempotently create, fund, and reconcile five direct coding bounties."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
from typing import Any, Mapping

from activate_routed_v3_replacements import (
    ActivationError,
    Cast,
    address,
    bytes32,
    http_json,
    lines,
    parse_bool,
    parse_uint,
    reconcile,
    run,
)
from bounded_agent_create import (
    SIGNED_QUORUM_VERIFIERS,
    SIGNED_QUORUM_VERIFIER_SET_HASH,
)


ROOT = Path(__file__).resolve().parents[1]
CHAIN_ID = 8453
RPC_DEFAULT = "https://mainnet.base.org"
API_DEFAULT = "https://api.agentbounties.app"
WALLET = "0x1eaa1c68772cf76bc5f4e4174766076e33ace662"
OWNER = "0x884834e884d6e93462655a2820140ad03e6747bc"
KEEPER = "0xc26a630e85134ed30968735c8e7de4576cfa5dbc"
FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
DETERMINISTIC_VERIFIER = "0x380c1af742593dd88b6f20387e9ee693a0536731"
ZERO_HASH = "0x" + "00" * 32
SOLVER_REWARD = 1_990_000
VERIFIER_REWARD = 10_000
TARGET = 2_000_000
FUNDING_DEADLINE = 1_800_000_000
CLAIM_WINDOW_SECONDS = 604_800
VERIFICATION_WINDOW_SECONDS = 259_200
ISSUES: dict[int, dict[str, Any]] = {
    634: {
        "title": "Restore the public ChatGPT bounty inventory tool",
        "goal": (
            "Make the production Agentbounties.app list_autonomous_bounties tool callable "
            "and prove claimable-only Base-mainnet inventory returns without INVALID_ARGUMENT."
        ),
        "criteria": [
            "A committed test invokes the public tool name used by the mounted ChatGPT app.",
            "The test fails on the unknown-or-unavailable-tool response and passes after the fix.",
            "The response remains fail-closed about funding, verifier readiness, and BountySettled evidence.",
        ],
    },
    635: {
        "title": "Add an end-to-end profitable inventory contract test",
        "goal": (
            "Prove one fully funded, terms-valid, verification-ready bounty appears in every "
            "claimable inventory surface with exact reward and bond economics."
        ),
        "criteria": [
            "One fixture covers API, MCP, discovery feed, and public website inventory.",
            "The test asserts reward, bond, funding, status, terms validity, and verifier readiness.",
            "A claimed bounty leaves claimable-only results without being treated as corrupt or unpaid.",
        ],
    },
    636: {
        "title": "Show solver cash margin before claim",
        "goal": (
            "Expose exact machine-readable and human-readable economics so agents can distinguish "
            "payout, refundable bond, required external spend, and gross cash margin before claiming."
        ),
        "criteria": [
            "API and MCP output expose reward, refundable bond, external spend, and gross cash margin.",
            "Public copy never describes gross cash margin as guaranteed net profit.",
            "Tests cover direct, standing-meta, and unprofitable inventory filtering.",
        ],
    },
    637: {
        "title": "Make activation reconciliation lifecycle-aware",
        "goal": (
            "Replace source-text lifecycle assertions with behavioral tests proving activation resumes "
            "across claimable, claimed, submitted, and verifying states without duplicate creation."
        ),
        "criteria": [
            "Tests model canonical factory and hosted feed state for all four active statuses.",
            "No planner or send path runs for an already-canonical contract.",
            "Invalid terms, unavailable verification, terminal failure, and ambiguity fail closed.",
        ],
    },
    638: {
        "title": "Expose direct-bounty verifier-readiness diagnostics",
        "goal": (
            "Expose fail-closed verifier-readiness diagnostics so agents know whether a direct "
            "sandboxed-regression bounty can be verified before bonding USDC."
        ),
        "criteria": [
            "API and MCP expose verifier set hash, threshold, runner identifier, and readiness.",
            "Unready inventory has one concise reason and is excluded from ready-to-earn results.",
            "Tests cover healthy, missing-signer, stale-runner, and verifier-set-mismatch states.",
        ],
    },
}


def creation_nonce(issue: int) -> str:
    value = f"agent-bounties/direct-seed/{issue}/v1".encode()
    return "0x" + hashlib.sha256(value).hexdigest()


def terms_document(issue: int, config: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "schema_version": "agent-bounties/terms-v1",
        "contract_terms": {
            "protocol_version": "agent-bounties/autonomous-v1",
            "creator_wallet": WALLET,
            "network": "base-mainnet",
            "settlement_token": USDC,
            "solver_reward": {"amount": SOLVER_REWARD, "currency": "usdc"},
            "verifier_reward": {"amount": VERIFIER_REWARD, "currency": "usdc"},
            "claim_bond": {"amount": VERIFIER_REWARD, "currency": "usdc"},
            "initial_funding": {"amount": TARGET, "currency": "usdc"},
            "funding_deadline": FUNDING_DEADLINE,
            "claim_window_seconds": CLAIM_WINDOW_SECONDS,
            "verification_window_seconds": VERIFICATION_WINDOW_SECONDS,
            "creation_nonce": creation_nonce(issue),
        },
        "title": config["title"],
        "goal": config["goal"],
        "acceptance_criteria": config["criteria"],
        "benchmark": {
            "engine": "sandboxed_regression_v1",
            "required_commands": ["python scripts/check.py"],
            "required_artifacts": [
                "repository",
                "commit_sha",
                "pull_request_url",
                "check_run_urls",
                "artifact_digest",
            ],
        },
        "evidence_schema": {
            "type": "object",
            "required": [
                "repository",
                "commit_sha",
                "pull_request_url",
                "check_run_urls",
                "artifact_digest",
            ],
            "additionalProperties": True,
        },
        "verification_policy": {
            "mechanism": "signed_quorum",
            "verifier_module": None,
            "verifier_reward_recipient": None,
            "verifiers": SIGNED_QUORUM_VERIFIERS,
            "threshold": 2,
            "rubric": (
                "Both pinned verifier agents independently run the committed sandboxed-regression "
                "policy and confirm every acceptance criterion from public evidence."
            ),
            "self_verification_forbidden": True,
        },
        "source_url": f"https://github.com/NSPG13/agent-bounties/issues/{issue}",
        "discovery_source": "maintainer-seeded direct profitable coding bounty",
    }


def creation_payload(document: Mapping[str, Any], published: Mapping[str, Any]) -> dict[str, Any]:
    terms = document["contract_terms"]
    policy = document["verification_policy"]
    return {
        "creator": terms["creator_wallet"],
        "solver_reward": terms["solver_reward"],
        "verifier_reward": terms["verifier_reward"],
        "terms_hash": published["terms_hash"],
        "policy_hash": published["policy_hash"],
        "acceptance_criteria_hash": published["acceptance_criteria_hash"],
        "benchmark_hash": published["benchmark_hash"],
        "evidence_schema_hash": published["evidence_schema_hash"],
        "funding_deadline": terms["funding_deadline"],
        "claim_window_seconds": terms["claim_window_seconds"],
        "verification_window_seconds": terms["verification_window_seconds"],
        "verification_mode": "signed_quorum",
        "verifier_module": None,
        "verifier_reward_recipient": None,
        "verifiers": policy["verifiers"],
        "threshold": policy["threshold"],
        "initial_funding": terms["initial_funding"],
        "creation_nonce": terms["creation_nonce"],
    }


def policy_state(cast: Cast) -> dict[str, Any]:
    if cast.chain_id() != CHAIN_ID:
        raise ActivationError("direct seed activation is pinned to Base mainnet")
    for label, target in {"factory": FACTORY, "wallet": WALLET, "USDC": USDC}.items():
        if cast.code(target) in {"0x", "0x0"}:
            raise ActivationError(f"{label} runtime code is missing")
    policy = lines(
        cast.call(
            WALLET,
            "policy()(address,uint64,uint64,uint64,uint256,uint256,uint256,uint256,uint8,uint8,address,bytes32,bytes32)",
        ),
        13,
        "bounded wallet policy",
    )
    now = parse_uint(cast.rpc("block", "latest", "--field", "timestamp"), "block timestamp")
    state = {
        "owner": address(cast.call(WALLET, "owner()(address)"), "wallet owner"),
        "delegate": address(policy[0], "policy delegate"),
        "valid_after": parse_uint(policy[1], "valid after"),
        "valid_until": parse_uint(policy[2], "valid until"),
        "period_seconds": parse_uint(policy[3], "period seconds"),
        "max_per_action": parse_uint(policy[4], "max per action"),
        "max_per_period": parse_uint(policy[5], "max per period"),
        "max_lifetime_spend": parse_uint(policy[6], "max lifetime spend"),
        "max_bounty_target": parse_uint(policy[7], "max bounty target"),
        "allowed_actions": parse_uint(policy[8], "allowed actions"),
        "allowed_verification_modes": parse_uint(policy[9], "allowed modes"),
        "deterministic_verifier": address(policy[10], "deterministic verifier"),
        "signed_quorum": bytes32(policy[11], "signed quorum"),
        "ai_quorum": bytes32(policy[12], "AI quorum"),
        "period_spent": parse_uint(cast.call(WALLET, "periodSpent()(uint256)"), "period spend"),
        "lifetime_spent": parse_uint(cast.call(WALLET, "lifetimeSpent()(uint256)"), "lifetime spend"),
        "wallet_balance": parse_uint(cast.call(USDC, "balanceOf(address)(uint256)", WALLET), "wallet balance"),
        "now": now,
    }
    expected = {
        "owner": OWNER,
        "delegate": KEEPER,
        "period_seconds": 86_400,
        "max_per_action": 5_000_000,
        "max_per_period": 10_000_000,
        "max_lifetime_spend": 89_000_000,
        "max_bounty_target": 5_000_000,
        "allowed_actions": 15,
        "allowed_verification_modes": 3,
        "deterministic_verifier": DETERMINISTIC_VERIFIER,
        "signed_quorum": SIGNED_QUORUM_VERIFIER_SET_HASH,
        "ai_quorum": ZERO_HASH,
    }
    for field, wanted in expected.items():
        if state[field] != wanted:
            raise ActivationError(f"bounded wallet {field} mismatch: expected {wanted}, got {state[field]}")
    if not state["valid_after"] <= now <= state["valid_until"]:
        raise ActivationError("bounded wallet policy is not active")
    bucket = parse_uint(cast.call(WALLET, "periodBucket()(uint256)"), "period bucket")
    effective_period_spent = state["period_spent"] if bucket == now // state["period_seconds"] else 0
    state["effective_period_spent"] = effective_period_spent
    state["remaining_period_budget"] = state["max_per_period"] - effective_period_spent
    state["remaining_lifetime_budget"] = state["max_lifetime_spend"] - state["lifetime_spent"]
    return state


def issue_body(issue: int, config: Mapping[str, Any], result: Mapping[str, Any]) -> str:
    criteria = "\n".join(f"- [ ] {item}" for item in config["criteria"])
    tx = result["transaction_hash"]
    transaction_line = (
        f"- Creation and funding transaction: https://basescan.org/tx/{tx}"
        if tx != "already-canonical"
        else "- Creation and funding transaction: previously confirmed canonical creation"
    )
    return f"""## Goal
{config['goal']}

## Live payment evidence
**Funded and claimable on Base mainnet.**

- Contract: `{result['contract']}`
{transaction_line}
- Confirmed funding: **2.00 / 2.00 USDC**
- Solver payout: **1.99 USDC**
- Verifier reward / refundable claim bond: **0.01 USDC**
- Required external spend: **0 USDC**
- Verification: `sandboxed_regression_v1`, pinned threshold-two quorum
- Status: `claimable`

## First action
Post `/claim #{issue} wallet: 0xYOUR_PUBLIC_BASE_ADDRESS` on this issue, or call MCP `agent_native_claim` with contract `{result['contract']}`. Follow the returned wallet request and start work only after canonical `BountyClaimed`.

## Acceptance criteria
{criteria}

Claim with a Base wallet containing 0.01 USDC for the refundable bond. Eligible claim gas is sponsored. Submit a PR and public evidence matching the criteria. A PR merge or verifier signature is not payment. Only canonical `BountySettled` proves payment.

**Post your own bounty:** https://agentbounties.app/post.html
"""


def activate(args: argparse.Namespace) -> dict[str, Any]:
    private_key = os.environ.get("BASE_KEEPER_PRIVATE_KEY", "").strip()
    if not private_key:
        raise ActivationError("BASE_KEEPER_PRIVATE_KEY is required")
    cast = Cast(args.cast, args.rpc_url)
    keeper = address(run([args.cast, "wallet", "address", "--private-key", private_key]), "keeper")
    if keeper != KEEPER:
        raise ActivationError(f"keeper key resolves to {keeper}, expected {KEEPER}")
    before = policy_state(cast)

    results: list[dict[str, Any]] = []
    for issue, config in ISSUES.items():
        document = terms_document(issue, config)
        published = http_json(
            "POST",
            f"{args.api.rstrip('/')}/v1/base/autonomous-bounties/terms",
            {"creator_wallet": WALLET, "document": document},
        )
        if not isinstance(published, dict):
            raise ActivationError(f"terms publication for #{issue} returned a non-object")
        create = creation_payload(document, published)
        plan = http_json(
            "POST",
            f"{args.api.rstrip('/')}/v1/base/autonomous-bounties/creation-plan",
            {"network": "base-mainnet", "create": create},
        )
        if not isinstance(plan, dict):
            raise ActivationError(f"creation plan for #{issue} returned a non-object")
        plan_path = args.output_dir / f"direct-{issue}-creation-plan.json"
        plan_path.write_text(json.dumps(plan, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        predicted = address(plan.get("predicted_bounty_contract"), f"#{issue} predicted bounty")
        bounty_id = bytes32(plan.get("bounty_id"), f"#{issue} bounty id")
        canonical = parse_bool(
            cast.call(FACTORY, "isCanonicalBounty(address)(bool)", predicted),
            f"#{issue} canonical state",
        )
        if canonical:
            tx_hash = "already-canonical"
        else:
            current = policy_state(cast)
            if min(
                current["remaining_period_budget"],
                current["remaining_lifetime_budget"],
                current["wallet_balance"],
            ) < TARGET:
                raise ActivationError(f"bounded wallet cannot fund remaining issue #{issue}")
            action_path = args.output_dir / f"direct-{issue}-bounded-action.json"
            run(
                [
                    args.python,
                    "scripts/plan_bounded_agent_action.py",
                    "create",
                    "--wallet",
                    WALLET,
                    "--creation-plan",
                    str(plan_path),
                    "--rpc-url",
                    args.rpc_url,
                    "--expect-owner",
                    OWNER,
                    "--expect-delegate",
                    KEEPER,
                    "--output",
                    str(action_path),
                ]
            )
            action = json.loads(action_path.read_text(encoding="utf-8"))
            direct = action.get("direct_transaction")
            if not isinstance(direct, dict):
                raise ActivationError(f"#{issue} action plan lacks a direct transaction")
            sent = cast.send_data(
                address(direct.get("to"), "direct transaction target"),
                str(direct.get("data")),
                private_key,
            )
            tx_hash = str(sent.get("transactionHash") or sent.get("transaction_hash"))
        reconciled = reconcile(args.api, predicted, bounty_id)
        result = {
            "issue": issue,
            "title": config["title"],
            "contract": predicted,
            "bounty_id": bounty_id,
            "transaction_hash": tx_hash,
            "terms_hash": bytes32(published.get("terms_hash"), f"#{issue} terms hash"),
            "reconciliation": reconciled,
        }
        (args.output_dir / f"direct-{issue}-issue.md").write_text(
            issue_body(issue, config, result),
            encoding="utf-8",
        )
        results.append(result)

    after = policy_state(cast)
    created = sum(item["transaction_hash"] != "already-canonical" for item in results)
    expected_spend = created * TARGET
    if after["lifetime_spent"] != before["lifetime_spent"] + expected_spend:
        raise ActivationError("lifetime spend did not increase by the exact creation amount")
    if after["wallet_balance"] != before["wallet_balance"] - expected_spend:
        raise ActivationError("wallet balance did not decrease by the exact creation amount")
    return {
        "schema": "agent-bounties/direct-seed-activation-v1",
        "network": "base-mainnet",
        "wallet": WALLET,
        "wallet_balance_before": before["wallet_balance"],
        "wallet_balance_after": after["wallet_balance"],
        "new_spend": expected_spend,
        "complete": len(results) == len(ISSUES),
        "results": results,
        "evidence_boundary": (
            "Canonical creation, funding, claimability, valid terms, and verifier readiness prove "
            "earning inventory is live. Only BountySettled proves solver payment."
        ),
    }


def markdown(report: Mapping[str, Any]) -> str:
    lines = [
        "## Five direct profitable coding bounties activated",
        "",
        f"- Bounded wallet: `{report['wallet']}`",
        f"- New canonical funding: **{int(report['new_spend']) / 1_000_000:.2f} USDC**",
        f"- Wallet after: **{int(report['wallet_balance_after']) / 1_000_000:.2f} USDC**",
        "",
    ]
    for item in report["results"]:
        lines.append(
            f"- #{item['issue']} `{item['contract']}`: 1.99 USDC solver / 0.01 USDC refundable bond"
        )
    lines.extend(["", "Only canonical `BountySettled` proves future solver payment.", ""])
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", RPC_DEFAULT))
    parser.add_argument("--api", default=os.environ.get("AGENT_BOUNTIES_API_URL", API_DEFAULT))
    parser.add_argument("--cast", default=os.environ.get("CAST_BIN", "cast"))
    parser.add_argument("--python", default=os.environ.get("PYTHON_BIN", "python"))
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target" / "direct-seed-activation")
    parser.add_argument("--output", type=Path, default=ROOT / "target" / "direct-seed-activation.json")
    parser.add_argument("--markdown-output", type=Path, default=ROOT / "target" / "direct-seed-activation.md")
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    report = activate(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    args.markdown_output.write_text(markdown(report), encoding="utf-8")
    print(json.dumps({"complete": report["complete"], "new_spend": report["new_spend"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
