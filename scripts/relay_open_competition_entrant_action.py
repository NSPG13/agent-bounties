#!/usr/bin/env python3
"""Relay one exact signed Open Competition entrant-wallet action with bounded gas."""

from __future__ import annotations

import argparse
import json
import os
import re
import time
from pathlib import Path

from inspect_bounded_agent_wallet import word_address, word_uint, words
from plan_bounded_agent_budget import ROOT, require_address, require_bytes32
from plan_open_competition_entrant_action import (
    ACTIONS,
    SCHEMA as PLAN_SCHEMA,
    build_plan,
    exact_hex_bytes,
    inspect_state,
    rpc,
    run_cast,
    validate_commitment_envelope,
    validate_manifest,
)
from relay_autonomous_action import CastClient, RelayError, normalize_private_key, validate_receipt


SCHEMA = "agent-bounties/open-competition-entrant-wallet-relay-v1"
SIGNATURE = "executeWithSignature(uint8,bytes,uint256,uint256,bytes)"
SIGNATURE_RE = re.compile(r"^0x[0-9a-f]{130}$")
MAX_GAS_LIMIT = 1_000_000
MAX_GAS_PRICE_WEI = 2_000_000_000
MAX_GAS_COST_WEI = 2_000_000_000_000_000
SAFE_WAIT_SECONDS = 120
PLAN_KEYS = {
    "schema_version",
    "network",
    "chain_id",
    "safe_block",
    "wallet",
    "delegate",
    "policy_hash",
    "policy_version",
    "action",
    "action_code",
    "bounty",
    "action_summary",
    "nonce",
    "deadline",
    "payload",
    "payload_hash",
    "direct_transaction",
    "signing_payload",
    "relay_call",
    "evidence_boundary",
}
REVALIDATED_FIELDS = PLAN_KEYS - {"safe_block", "evidence_boundary"}


def load_json(path: Path, label: str) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise RelayError(f"{label} must contain one JSON object")
    return value


def validate_plan(value: dict) -> dict:
    if set(value) != PLAN_KEYS:
        raise RelayError("entrant action plan keys are incomplete or unexpected")
    if value.get("schema_version") != PLAN_SCHEMA:
        raise RelayError("entrant action plan schema is unsupported")
    if value.get("network") not in {"base-mainnet", "base-sepolia"}:
        raise RelayError("entrant action plan network is unsupported")
    expected_chain = 8453 if value["network"] == "base-mainnet" else 84532
    if value.get("chain_id") != expected_chain:
        raise RelayError("entrant action plan chain does not match its network")
    action = value.get("action")
    if action not in ACTIONS or value.get("action_code") != ACTIONS[action]:
        raise RelayError("entrant action name and code do not match")
    value["wallet"] = require_address(str(value.get("wallet", "")), "entrant wallet")
    value["delegate"] = require_address(str(value.get("delegate", "")), "delegate")
    value["bounty"] = require_address(str(value.get("bounty", "")), "competition bounty")
    value["policy_hash"] = require_bytes32(str(value.get("policy_hash", "")), "policy hash")
    value["payload_hash"] = require_bytes32(str(value.get("payload_hash", "")), "payload hash")
    value["payload"] = exact_hex_bytes(str(value.get("payload", "")), "payload")
    for field in ("policy_version", "nonce", "deadline"):
        item = value.get(field)
        if isinstance(item, bool) or not isinstance(item, int) or item < 0:
            raise RelayError(f"entrant action {field} must be a nonnegative integer")
    if value["policy_version"] == 0 or value["deadline"] == 0:
        raise RelayError("entrant action policy version and deadline must be positive")
    safe = value.get("safe_block")
    if not isinstance(safe, dict) or set(safe) != {"number", "hash", "timestamp"}:
        raise RelayError("entrant action safe block is malformed")
    if any(isinstance(safe.get(field), bool) or not isinstance(safe.get(field), int) for field in ("number", "timestamp")):
        raise RelayError("entrant action safe block number and timestamp must be integers")
    safe["hash"] = require_bytes32(str(safe.get("hash", "")), "safe block hash")
    if run_cast("keccak", value["payload"]) != value["payload_hash"]:
        raise RelayError("entrant action payload hash does not match the signed payload")
    return value


def proof_from_file(path: Path | None) -> str | None:
    if path is None:
        return None
    raw = path.read_text(encoding="utf-8").strip()
    if raw.startswith("{"):
        value = json.loads(raw)
        if not isinstance(value, dict) or set(value) != {"proof"}:
            raise RelayError("proof JSON must contain only the proof field")
        raw = str(value["proof"])
    return exact_hex_bytes(raw, "proof")


def assert_original_safe_block_is_canonical(rpc_url: str, plan: dict, current: dict) -> None:
    original = plan["safe_block"]
    if current["safe_block"]["number"] < original["number"]:
        raise RelayError("current safe head is behind the action plan")
    observed = rpc(rpc_url, "eth_getBlockByNumber", [hex(original["number"]), False], 70)
    if not isinstance(observed, dict) or str(observed.get("hash", "")).lower() != original["hash"]:
        raise RelayError("the action plan's original safe block is no longer canonical")


def revalidate_plan(
    rpc_url: str,
    manifest: dict,
    plan: dict,
    envelope: dict | None,
    proof: str | None,
) -> tuple[dict, dict]:
    action = plan["action"]
    if action == "commit":
        if envelope is not None or proof is not None:
            raise RelayError("commit relay accepts no plaintext commitment envelope or proof")
        summary = plan.get("action_summary")
        if not isinstance(summary, dict) or set(summary) != {"commitment", "maximum_gross_spend"}:
            raise RelayError("commit action summary is malformed")
        commitment = require_bytes32(str(summary["commitment"]), "commitment")
        planning_envelope = {"commitment": commitment}
    elif action == "reveal":
        if envelope is None or proof is None:
            raise RelayError("reveal relay requires the local commitment envelope and proof file")
        planning_envelope = envelope
    else:
        if envelope is not None or proof is not None:
            raise RelayError("withdraw_bond relay accepts no commitment envelope or proof")
        planning_envelope = None
    current = inspect_state(rpc_url, manifest, plan["wallet"], plan["bounty"])
    assert_original_safe_block_is_canonical(rpc_url, plan, current)
    if action == "reveal":
        assert envelope is not None
        planning_envelope = validate_commitment_envelope(envelope, current)
    regenerated = build_plan(
        current,
        action,
        planning_envelope,
        proof,
        300,
        exact_deadline=plan["deadline"],
    )
    for field in sorted(REVALIDATED_FIELDS):
        if regenerated[field] != plan[field]:
            raise RelayError(f"live safe-state revalidation changed signed field {field}")
    return current, regenerated


def topic_address(value: str) -> str:
    return "0x" + "00" * 12 + require_address(value, "topic address")[2:]


def topic_uint(value: int) -> str:
    return "0x" + value.to_bytes(32, "big").hex()


def canonical_receipt(rpc_url: str, tx_hash: str, receipt_block: int, receipt_hash: str) -> tuple[dict, dict]:
    deadline = time.monotonic() + SAFE_WAIT_SECONDS
    while time.monotonic() < deadline:
        safe = rpc(rpc_url, "eth_getBlockByNumber", ["safe", False], 80)
        receipt = rpc(rpc_url, "eth_getTransactionReceipt", [tx_hash], 81)
        block = rpc(rpc_url, "eth_getBlockByNumber", [hex(receipt_block), False], 82)
        if (
            isinstance(safe, dict)
            and int(str(safe.get("number", "0x0")), 16) >= receipt_block
            and isinstance(receipt, dict)
            and str(receipt.get("status", "")).lower() == "0x1"
            and str(receipt.get("blockHash", "")).lower() == receipt_hash
            and isinstance(block, dict)
            and str(block.get("hash", "")).lower() == receipt_hash
        ):
            return receipt, safe
        time.sleep(2)
    raise RelayError("relay receipt did not become canonical at a Base safe block")


def matching_logs(receipt: dict, address: str, event_signature: str) -> list[dict]:
    topic0 = run_cast("keccak", input_text=event_signature)
    result = []
    for log in receipt.get("logs", []):
        if not isinstance(log, dict) or str(log.get("address", "")).lower() != address:
            continue
        topics = log.get("topics")
        if isinstance(topics, list) and topics and str(topics[0]).lower() == topic0:
            result.append(log)
    return result


def require_wallet_action_event(receipt: dict, plan: dict, keeper: str) -> None:
    logs = matching_logs(
        receipt,
        plan["wallet"],
        "EntrantActionExecuted(uint8,address,address,uint256,bytes32)",
    )
    expected_topics = [topic_uint(plan["action_code"]), topic_address(plan["delegate"]), topic_address(keeper)]
    for log in logs:
        topics = [str(value).lower() for value in log.get("topics", [])[1:]]
        data = words(str(log.get("data", "")))
        if (
            topics == expected_topics
            and len(data) == 2
            and word_uint(data[0]) == plan["nonce"]
            and f"0x{data[1]}" == plan["payload_hash"]
        ):
            return
    raise RelayError("canonical receipt lacks the exact entrant-wallet action event")


def validate_action_events_and_balances(receipt: dict, plan: dict, before: dict, after: dict) -> dict:
    action = plan["action"]
    bounty = plan["bounty"]
    wallet = plan["wallet"]
    before_wallet = before["wallet_state"]
    after_wallet = after["wallet_state"]
    before_bounty = before["bounty_state"]
    after_bounty = after["bounty_state"]
    if after_wallet["delegate_nonce"] != plan["nonce"] + 1:
        raise RelayError("entrant wallet nonce did not advance exactly once")
    if (
        after_wallet["policy_hash"] != before_wallet["policy_hash"]
        or after_wallet["policy_version"] != before_wallet["policy_version"]
    ):
        raise RelayError("entrant wallet policy changed during the relayed action")
    result: dict = {"payment_proven": False}
    if action == "commit":
        logs = matching_logs(
            receipt,
            bounty,
            "SolutionCommitted(bytes32,address,uint8,bytes32,uint64,uint64,uint256)",
        )
        if len(logs) != 1 or topic_address(wallet) not in [str(t).lower() for t in logs[0].get("topics", [])]:
            raise RelayError("canonical receipt lacks the entrant's SolutionCommitted event")
        committed_data = words(str(logs[0].get("data", "")))
        if len(committed_data) != 4 or f"0x{committed_data[0]}" != plan["action_summary"]["commitment"]:
            raise RelayError("SolutionCommitted event does not contain the signed commitment")
        bond = before_bounty["verifier_reward"]
        entry = after_bounty["entry"]
        if entry["state"] != 1 or entry["commitment"] != plan["action_summary"]["commitment"] or entry["bond"] != bond:
            raise RelayError("committed entry state does not match the signed commitment")
        if after_wallet["token_balance"] != before_wallet["token_balance"] - bond:
            raise RelayError("entrant wallet token balance does not reconcile with the entry bond")
        if after_wallet["lifetime_spent"] != before_wallet["lifetime_spent"] + bond:
            raise RelayError("entrant wallet lifetime spend does not reconcile with the entry bond")
        result["entry_bond_minor"] = bond
    elif action == "reveal":
        entry = after_bounty["entry"]
        settled = matching_logs(
            receipt,
            bounty,
            "BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)",
        )
        rejected = matching_logs(
            receipt,
            bounty,
            "CompetitionSubmissionRejected(bytes32,uint64,address,uint256,bytes32)",
        )
        if len(settled) + len(rejected) != 1:
            raise RelayError("canonical reveal receipt must contain exactly one settlement or rejection event")
        if settled:
            if topic_address(wallet) not in [str(t).lower() for t in settled[0].get("topics", [])]:
                raise RelayError("BountySettled does not identify the entrant wallet as solver")
            if after_bounty["status"] != 2 or entry["state"] != 4:
                raise RelayError("settled reveal state does not identify the entrant wallet as winner")
            data = words(str(settled[0].get("data", "")))
            if len(data) != 8:
                raise RelayError("BountySettled event data is malformed")
            if (
                f"0x{data[4]}" != plan["action_summary"]["submission_hash"]
                or f"0x{data[5]}" != plan["action_summary"]["evidence_hash"]
            ):
                raise RelayError("BountySettled hashes do not match the signed reveal")
            solver_reward, returned_bond, timeout_bonus, verifier_reward = map(word_uint, data[:4])
            expected_delta = solver_reward + returned_bond + timeout_bonus
            if after_wallet["token_balance"] != before_wallet["token_balance"] + expected_delta:
                raise RelayError("winner token balance does not reconcile with BountySettled")
            result.update(
                {
                    "payment_proven": True,
                    "solver_reward_minor": solver_reward,
                    "entry_bond_returned_minor": returned_bond,
                    "timeout_bonus_minor": timeout_bonus,
                    "verifier_reward_minor": verifier_reward,
                }
            )
        else:
            if topic_address(wallet) not in [str(t).lower() for t in rejected[0].get("topics", [])]:
                raise RelayError("CompetitionSubmissionRejected does not identify the entrant wallet")
            if entry["state"] != 2 or entry["bond"] != 0:
                raise RelayError("rejected reveal state does not match CompetitionSubmissionRejected")
            if after_wallet["token_balance"] != before_wallet["token_balance"]:
                raise RelayError("rejected reveal unexpectedly changed the entrant wallet token balance")
            result["reveal_rejected"] = True
    else:
        logs = matching_logs(receipt, bounty, "EntryBondWithdrawn(bytes32,address,uint256)")
        if len(logs) != 1 or topic_address(wallet) not in [str(t).lower() for t in logs[0].get("topics", [])]:
            raise RelayError("canonical receipt lacks the entrant's EntryBondWithdrawn event")
        amount_words = words(str(logs[0].get("data", "")))
        if len(amount_words) != 1:
            raise RelayError("EntryBondWithdrawn event data is malformed")
        amount = word_uint(amount_words[0])
        if after_bounty["entry"]["state"] != 5 or after_bounty["entry"]["bond"] != 0:
            raise RelayError("withdrawn entry-bond state is not final")
        if after_wallet["token_balance"] != before_wallet["token_balance"] + amount:
            raise RelayError("entrant wallet token balance does not reconcile with bond withdrawal")
        result["entry_bond_withdrawn_minor"] = amount
    return result


def execute_or_validate(
    *,
    rpc_url: str,
    cast_bin: str,
    manifest: dict,
    plan: dict,
    envelope: dict | None,
    proof: str | None,
    signature: str,
    execute: bool,
    keeper_value: str | None,
    private_key: str | None,
) -> dict:
    before, _ = revalidate_plan(rpc_url, manifest, plan, envelope, proof)
    client = CastClient(cast_bin, rpc_url, "latest")
    if private_key:
        normalized_key = normalize_private_key(private_key)
        keeper = client.keeper_address(normalized_key)
    else:
        normalized_key = None
        if execute:
            raise RelayError("BASE_KEEPER_PRIVATE_KEY is required with --execute")
        if keeper_value is None:
            raise RelayError("--keeper is required for dry-run validation")
        keeper = require_address(keeper_value, "keeper")
    if client.chain_id() != plan["chain_id"]:
        raise RelayError("keeper RPC chain does not match the signed entrant action")
    digest = client.call(
        plan["wallet"],
        "actionDigest(uint8,bytes32,uint256,uint256)(bytes32)",
        str(plan["action_code"]),
        plan["payload_hash"],
        str(plan["nonce"]),
        str(plan["deadline"]),
        block="latest",
    ).strip().lower()
    require_bytes32(digest, "entrant action digest")
    args = (
        str(plan["action_code"]),
        plan["payload"],
        str(plan["nonce"]),
        str(plan["deadline"]),
        signature,
    )
    gas_estimate = client.estimate(keeper, plan["wallet"], SIGNATURE, *args)
    gas_limit = gas_estimate * 125 // 100 + 10_000
    gas_price = client.gas_price()
    max_gas_cost = gas_limit * gas_price
    if gas_limit > MAX_GAS_LIMIT:
        raise RelayError("entrant action gas exceeds the keeper limit")
    if gas_price > MAX_GAS_PRICE_WEI or max_gas_cost > MAX_GAS_COST_WEI:
        raise RelayError("entrant action gas price or maximum total cost exceeds the keeper ceiling")
    keeper_balance = client.balance(keeper)
    if keeper_balance < max_gas_cost:
        raise RelayError("keeper balance is below the bounded relay cost", retryable=True)
    report = {
        "schema_version": SCHEMA,
        "outcome": "validated",
        "network": plan["network"],
        "chain_id": plan["chain_id"],
        "wallet": plan["wallet"],
        "bounty": plan["bounty"],
        "action": plan["action"],
        "nonce": plan["nonce"],
        "deadline": plan["deadline"],
        "payload_hash": plan["payload_hash"],
        "signature_hash": run_cast("keccak", signature),
        "action_digest": digest,
        "keeper": keeper,
        "keeper_balance_before_wei": keeper_balance,
        "gas_estimate": gas_estimate,
        "gas_limit": gas_limit,
        "gas_price_wei": gas_price,
        "maximum_gas_cost_wei": max_gas_cost,
        "entrant_wallet_eth_required_wei": 0,
        "safe_block_before": before["safe_block"],
        "payment_proven": False,
        "evidence_boundary": (
            "Validation and a signature are not entry or payment evidence. A canonical safe-block action event "
            "proves the relay; only canonical BountySettled proves solver payment."
        ),
    }
    if not execute:
        return report
    assert normalized_key is not None
    sent = client.send(normalized_key, gas_limit, plan["wallet"], SIGNATURE, *args)
    tx_hash, block_tag = validate_receipt(sent, plan["wallet"])
    receipt_block = int(block_tag, 0)
    receipt_hash = require_bytes32(
        str(sent.get("blockHash") or sent.get("block_hash") or ""), "receipt block hash"
    )
    receipt, safe = canonical_receipt(rpc_url, tx_hash, receipt_block, receipt_hash)
    require_wallet_action_event(receipt, plan, keeper)
    after = inspect_state(rpc_url, manifest, plan["wallet"], plan["bounty"])
    if after["safe_block"]["number"] < receipt_block:
        raise RelayError("post-action inspection did not reach the canonical receipt block")
    action_result = validate_action_events_and_balances(receipt, plan, before, after)
    report.update(
        {
            "outcome": "relayed",
            "transaction_hash": tx_hash,
            "receipt_block": receipt_block,
            "receipt_block_hash": receipt_hash,
            "canonical_safe_block": {
                "number": int(str(safe["number"]), 16),
                "hash": str(safe["hash"]).lower(),
                "timestamp": int(str(safe["timestamp"]), 16),
            },
            "safe_block_after": after["safe_block"],
            "keeper_balance_after_wei": client.balance(keeper),
            "gas_payer": keeper,
            "entrant_wallet_eth_spent_wei": 0,
            **action_result,
        }
    )
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--signature-file", type=Path, required=True)
    parser.add_argument("--commitment-envelope", type=Path)
    parser.add_argument("--proof-file", type=Path)
    parser.add_argument("--rpc-url")
    parser.add_argument("--cast-bin", default=str(ROOT / ".tools" / "foundry" / "cast.exe"))
    parser.add_argument("--keeper")
    parser.add_argument("--execute", action="store_true")
    parser.add_argument(
        "--report", type=Path, default=ROOT / "target" / "open-competition-entrant-relay.json"
    )
    args = parser.parse_args()
    manifest = validate_manifest(load_json(args.manifest, "entrant-wallet manifest"))
    plan = validate_plan(load_json(args.plan, "entrant action plan"))
    if plan["network"] != manifest["network"] or plan["chain_id"] != manifest["chain_id"]:
        raise SystemExit("entrant action plan and deployment manifest disagree")
    signature = args.signature_file.read_text(encoding="utf-8").strip().lower()
    if not SIGNATURE_RE.fullmatch(signature):
        raise SystemExit("signature file must contain exactly one 65-byte signature")
    envelope = load_json(args.commitment_envelope, "commitment envelope") if args.commitment_envelope else None
    proof = proof_from_file(args.proof_file)
    rpc_url = args.rpc_url or str(manifest["rpc_url"])
    report = execute_or_validate(
        rpc_url=rpc_url,
        cast_bin=args.cast_bin,
        manifest=manifest,
        plan=plan,
        envelope=envelope,
        proof=proof,
        signature=signature,
        execute=args.execute,
        keeper_value=args.keeper,
        private_key=os.environ.get("BASE_KEEPER_PRIVATE_KEY"),
    )
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(args.report)


if __name__ == "__main__":
    main()
