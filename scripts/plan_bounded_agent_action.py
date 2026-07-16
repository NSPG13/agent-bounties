#!/usr/bin/env python3
"""Validate and plan one fund, claim, or submit action for a bounded wallet."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from inspect_bounded_agent_wallet import call, inspect, word_address, word_uint, words
from plan_bounded_agent_budget import (
    ROOT,
    calldata,
    encode,
    keccak_hex,
    require_address,
    require_bytes32,
    usdc_units,
    validate_manifest,
)


DEFAULT_MANIFEST = ROOT / "deployments" / "bounded-agent-wallet-base-mainnet.json"
ZERO_ADDRESS = "0x" + "00" * 20
ACTIONS = {"fund": 1, "claim": 2, "submit": 3}


def one_word(rpc_url: str, target: str, signature: str, block: str, arguments: tuple[str, ...] = ()) -> str:
    result = words(call(rpc_url, target, signature, block, arguments))
    if len(result) != 1:
        raise SystemExit(f"{signature} returned an unexpected shape")
    return result[0]


def bounty_state(rpc_url: str, bounty: str, factory: str, block: str) -> dict:
    registered = bool(
        word_uint(one_word(rpc_url, factory, "isCanonicalBounty(address)(bool)", block, (bounty,)))
    )
    if not registered:
        raise SystemExit("target is not registered by the canonical bounty factory")
    return {
        "factory": word_address(one_word(rpc_url, bounty, "factory()(address)", block)),
        "settlement_token": word_address(one_word(rpc_url, bounty, "settlementToken()(address)", block)),
        "creator": word_address(one_word(rpc_url, bounty, "creator()(address)", block)),
        "solver": word_address(one_word(rpc_url, bounty, "solver()(address)", block)),
        "status": word_uint(one_word(rpc_url, bounty, "bountyStatus()(uint8)", block)),
        "verification_mode": word_uint(one_word(rpc_url, bounty, "verificationMode()(uint8)", block)),
        "verifier_module": word_address(one_word(rpc_url, bounty, "verifierModule()(address)", block)),
        "verifier_set_hash": f"0x{one_word(rpc_url, bounty, 'verifierSetHash()(bytes32)', block)}",
        "target_amount": word_uint(one_word(rpc_url, bounty, "targetAmount()(uint256)", block)),
        "funded_amount": word_uint(one_word(rpc_url, bounty, "fundedAmount()(uint256)", block)),
        "verifier_reward": word_uint(one_word(rpc_url, bounty, "verifierReward()(uint256)", block)),
        "funding_deadline": word_uint(one_word(rpc_url, bounty, "fundingDeadline()(uint64)", block)),
        "claim_expires_at": word_uint(one_word(rpc_url, bounty, "claimExpiresAt()(uint64)", block)),
    }


def validate_bounty_policy(state: dict, policy: dict, factory: str, settlement_token: str) -> None:
    if state["factory"] != factory:
        raise SystemExit("bounty factory binding does not match the wallet policy")
    if state["settlement_token"] != settlement_token:
        raise SystemExit("bounty token binding does not match the wallet policy")
    if state["target_amount"] > policy["max_bounty_target"]:
        raise SystemExit("bounty target exceeds the wallet policy")
    mode = state["verification_mode"]
    if not policy["allowed_verification_modes"] & (1 << mode):
        raise SystemExit("bounty verification mode is not allowed")
    if mode == 0 and state["verifier_module"] != policy["deterministic_verifier_module"]:
        raise SystemExit("deterministic verifier does not match the wallet policy")
    if mode == 1 and state["verifier_set_hash"] != policy["signed_quorum_verifier_set_hash"]:
        raise SystemExit("signed verifier set does not match the wallet policy")
    if mode == 2 and state["verifier_set_hash"] != policy["ai_judge_verifier_set_hash"]:
        raise SystemExit("AI verifier set does not match the wallet policy")


def validate_spend(report: dict, amount: int) -> None:
    if amount == 0:
        return
    state = report["state"]
    policy = state["policy"]
    if amount > policy["max_per_action"]:
        raise SystemExit("action exceeds the per-action cap")
    current_bucket = report["safe_block"]["timestamp"] // policy["period_seconds"]
    observed_period_spent = int(state["period_spent"])
    period_spent = observed_period_spent if current_bucket == int(state["period_bucket"]) else 0
    if period_spent + amount > policy["max_per_period"]:
        raise SystemExit("action exceeds the remaining period cap")
    if int(state["lifetime_spent"]) + amount > policy["max_lifetime_spend"]:
        raise SystemExit("action exceeds the remaining lifetime cap")
    if int(state["wallet_usdc_balance"]) < amount:
        raise SystemExit("wallet USDC balance is below the action spend")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=sorted(ACTIONS))
    parser.add_argument("--wallet", required=True)
    parser.add_argument("--bounty", required=True)
    parser.add_argument("--amount-usdc")
    parser.add_argument("--submission-hash")
    parser.add_argument("--evidence-hash")
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--rpc-url")
    parser.add_argument("--expect-owner")
    parser.add_argument("--expect-delegate")
    parser.add_argument("--expect-policy-hash")
    parser.add_argument("--deadline-seconds", type=int, default=900)
    parser.add_argument("--output", type=Path, default=ROOT / "target" / "bounded-agent-action-plan.json")
    args = parser.parse_args()

    manifest = validate_manifest(json.loads(args.manifest.read_text(encoding="utf-8")))
    wallet = require_address(args.wallet, "wallet")
    bounty = require_address(args.bounty, "bounty")
    rpc_url = args.rpc_url or manifest["rpc_url"]
    expected_owner = require_address(args.expect_owner, "expected owner") if args.expect_owner else None
    expected_delegate = require_address(args.expect_delegate, "expected delegate") if args.expect_delegate else None
    expected_policy_hash = (
        require_bytes32(args.expect_policy_hash, "expected policy hash") if args.expect_policy_hash else None
    )
    report = inspect(rpc_url, wallet, manifest, expected_owner, expected_delegate, expected_policy_hash)
    if not report["ready"]:
        raise SystemExit(f"bounded wallet inspection failed: {', '.join(report['failures'])}")

    block = hex(report["safe_block"]["number"])
    factory = require_address(manifest["canonical"]["bounty_factory"], "bounty factory")
    settlement_token = require_address(manifest["canonical"]["settlement_token"], "settlement token")
    observed = bounty_state(rpc_url, bounty, factory, block)
    policy = report["state"]["policy"]
    validate_bounty_policy(observed, policy, factory, settlement_token)
    action = ACTIONS[args.action]
    if not policy["allowed_actions"] & (1 << action):
        raise SystemExit("action is not enabled by the wallet policy")

    if args.action == "fund":
        if args.amount_usdc is None:
            raise SystemExit("fund requires --amount-usdc")
        requested = usdc_units(args.amount_usdc, "fund amount")
        remaining = observed["target_amount"] - observed["funded_amount"]
        if observed["status"] != 0 or remaining <= 0:
            raise SystemExit("bounty is not open for funding")
        if report["safe_block"]["timestamp"] > observed["funding_deadline"]:
            raise SystemExit("bounty funding deadline has passed")
        spend = min(requested, remaining)
        payload = encode("f(address,uint256)", bounty, str(requested))
        direct_data = calldata("fundBounty(address,uint256)", bounty, str(requested))
        action_summary = {"requested_amount": str(requested), "maximum_accepted_amount": str(spend)}
    elif args.action == "claim":
        if args.amount_usdc or args.submission_hash or args.evidence_hash:
            raise SystemExit("claim does not accept amount or submission hashes")
        if observed["status"] != 1 or observed["solver"] != ZERO_ADDRESS:
            raise SystemExit("bounty is not claimable")
        if observed["creator"] == wallet:
            raise SystemExit("creator wallet cannot claim its own bounty")
        spend = observed["verifier_reward"]
        payload = encode("f(address)", bounty)
        direct_data = calldata("claimBounty(address)", bounty)
        action_summary = {"claim_bond": str(spend)}
    else:
        if args.amount_usdc:
            raise SystemExit("submit does not accept an amount")
        submission_hash = require_bytes32(args.submission_hash or "", "submission hash")
        evidence_hash = require_bytes32(args.evidence_hash or "", "evidence hash")
        if observed["status"] != 2 or observed["solver"] != wallet:
            raise SystemExit("wallet does not own the active claim")
        if report["safe_block"]["timestamp"] > observed["claim_expires_at"]:
            raise SystemExit("claim has expired")
        spend = 0
        payload = encode("f(address,bytes32,bytes32)", bounty, submission_hash, evidence_hash)
        direct_data = calldata("submitBounty(address,bytes32,bytes32)", bounty, submission_hash, evidence_hash)
        action_summary = {"submission_hash": submission_hash, "evidence_hash": evidence_hash}

    validate_spend(report, spend)
    if args.deadline_seconds < 60 or args.deadline_seconds > 900:
        raise SystemExit("deadline-seconds must be between 60 and 900")
    deadline = min(
        report["safe_block"]["timestamp"] + args.deadline_seconds,
        policy["valid_until"],
    )
    if deadline <= report["safe_block"]["timestamp"]:
        raise SystemExit("wallet policy expires too soon")
    nonce = int(report["state"]["delegate_nonce"])
    policy_version = int(report["state"]["policy_version"])
    payload_hash = keccak_hex(payload)
    typed_data = {
        "types": {
            "EIP712Domain": [
                {"name": "name", "type": "string"},
                {"name": "version", "type": "string"},
                {"name": "chainId", "type": "uint256"},
                {"name": "verifyingContract", "type": "address"},
            ],
            "AgentAction": [
                {"name": "wallet", "type": "address"},
                {"name": "action", "type": "uint8"},
                {"name": "payloadHash", "type": "bytes32"},
                {"name": "nonce", "type": "uint256"},
                {"name": "deadline", "type": "uint256"},
                {"name": "policyVersion", "type": "uint64"},
            ],
        },
        "primaryType": "AgentAction",
        "domain": {
            "name": "Agent Bounties Bounded Wallet",
            "version": "1",
            "chainId": manifest["chain_id"],
            "verifyingContract": wallet,
        },
        "message": {
            "wallet": wallet,
            "action": action,
            "payloadHash": payload_hash,
            "nonce": str(nonce),
            "deadline": str(deadline),
            "policyVersion": policy_version,
        },
    }
    plan = {
        "schema": "agent-bounties/bounded-agent-action-plan-v1",
        "network": manifest["network"],
        "safe_block": report["safe_block"],
        "wallet": wallet,
        "delegate": policy["delegate"],
        "policy_hash": report["state"]["policy_hash"],
        "action": args.action,
        "action_code": action,
        "bounty": bounty,
        "bounty_state": observed,
        "action_summary": action_summary,
        "maximum_gross_spend": str(spend),
        "payload": payload,
        "payload_hash": payload_hash,
        "direct_transaction": {"from": policy["delegate"], "to": wallet, "data": direct_data, "value": "0x0"},
        "relay_authorization_typed_data": typed_data,
        "relay_call": {
            "to": wallet,
            "function": "executeWithSignature(uint8,bytes,uint256,uint256,bytes)",
            "arguments_before_signature": [action, payload, nonce, deadline],
            "signature_tail": ["delegate_signature"],
        },
        "evidence_boundary": (
            "This same-block plan is unsigned and moves no value. Re-inspect nonce, policy, counters, bounty state, "
            "and deadline immediately before signing. Only canonical events prove an applied action or payout."
        ),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(plan, indent=2) + "\n", encoding="utf-8")
    print(args.output)


if __name__ == "__main__":
    main()
