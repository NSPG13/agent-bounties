#!/usr/bin/env python3
"""Plan one exact signed action for an Open Competition entrant wallet."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from pathlib import Path

from inspect_bounded_agent_wallet import call, code_hash, rpc, word_address, word_uint, words
from plan_bounded_agent_budget import ROOT, require_address, require_bytes32


SCHEMA = "agent-bounties/open-competition-entrant-wallet-action-v1"
COMMITMENT_SCHEMA = "agent-bounties/open-competition-v1-commitment-v1"
MANIFEST_SCHEMA = "agent-bounties/open-competition-entrant-wallet-deployment-v1"
POLICY_SIGNATURE = (
    "policy()(address,uint64,uint64,uint64,uint256,uint256,uint256,uint256,uint8,address,"
    "bytes32,bytes32,bytes32,bytes32,bytes32)"
)
POLICY_FIELDS = (
    "delegate",
    "valid_after",
    "valid_until",
    "period_seconds",
    "max_per_action",
    "max_per_period",
    "max_lifetime_spend",
    "max_bounty_target",
    "allowed_actions",
    "verifier_module",
    "verifier_runtime_code_hash",
    "verifier_policy_hash",
    "acceptance_criteria_hash",
    "benchmark_hash",
    "evidence_schema_hash",
)
ACTIONS = {"commit": 0, "reveal": 1, "withdraw_bond": 2}
ZERO_HASH = "0x" + "00" * 32
HEX_BYTES = re.compile(r"^0x(?:[0-9a-f]{2})*$")


def run_cast(*args: str, input_text: str | None = None) -> str:
    cast_bin = ROOT / ".tools" / "foundry" / "cast.exe"
    command = [str(cast_bin) if cast_bin.exists() else "cast", *args]
    completed = subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        input=input_text,
    )
    return completed.stdout.strip().lower()


def one_word(rpc_url: str, target: str, signature: str, block: str, arguments: tuple[str, ...] = ()) -> str:
    result = words(call(rpc_url, target, signature, block, arguments))
    if len(result) != 1:
        raise SystemExit(f"{signature} returned an unexpected shape")
    return result[0]


def exact_hex_bytes(value: str, label: str, maximum_bytes: int = 16_384) -> str:
    normalized = value.strip().lower()
    if not HEX_BYTES.fullmatch(normalized):
        raise SystemExit(f"{label} must be complete 0x-prefixed bytes")
    if (len(normalized) - 2) // 2 > maximum_bytes:
        raise SystemExit(f"{label} exceeds {maximum_bytes} bytes")
    return normalized


def validate_manifest(value: dict) -> dict:
    if value.get("schema_version") != MANIFEST_SCHEMA:
        raise SystemExit("entrant-wallet manifest schema is unsupported")
    if value.get("network") not in {"base-mainnet", "base-sepolia"}:
        raise SystemExit("entrant-wallet manifest network is unsupported")
    expected_chain = 8453 if value["network"] == "base-mainnet" else 84532
    if value.get("chain_id") != expected_chain:
        raise SystemExit("entrant-wallet manifest chain id does not match its network")
    if value.get("contract_source_dirty") is not False:
        raise SystemExit("entrant-wallet manifest was generated from dirty contract sources")
    if value.get("contract_source_revision_kind") != "git-tree":
        raise SystemExit("entrant-wallet manifest must pin a git tree")
    source_revision = str(value.get("contract_source_revision", ""))
    if not re.fullmatch(r"[0-9a-f]{40}", source_revision):
        raise SystemExit("entrant-wallet manifest source revision is invalid")
    canonical = value.get("canonical") or {}
    require_address(str(canonical.get("competition_factory", "")), "competition factory")
    require_address(str(canonical.get("settlement_token", "")), "settlement token")
    deployer = value.get("deterministic_deployer") or {}
    require_address(str(deployer.get("address", "")), "deterministic deployer")
    require_bytes32(str(deployer.get("runtime_code_hash", "")), "deterministic deployer runtime")
    wallet_factory = value.get("entrant_wallet_factory") or {}
    for name in ("address", "implementation"):
        require_address(str(wallet_factory.get(name, "")), name.replace("_", " "))
    for name in (
        "runtime_code_hash",
        "implementation_runtime_code_hash",
        "clone_runtime_code_hash",
    ):
        require_bytes32(str(wallet_factory.get(name, "")), name.replace("_", " "))
    return value


def policy_from_result(result: str) -> tuple[dict, str]:
    raw_words = words(result)
    if len(raw_words) != len(POLICY_FIELDS):
        raise SystemExit("entrant-wallet policy returned an unexpected shape")
    policy = dict(zip(POLICY_FIELDS, raw_words, strict=True))
    for name in ("delegate", "verifier_module"):
        policy[name] = word_address(str(policy[name]))
    for name in POLICY_FIELDS[1:9]:
        policy[name] = word_uint(str(policy[name]))
    for name in POLICY_FIELDS[10:]:
        policy[name] = f"0x{policy[name]}"
    return policy, run_cast("keccak", result)


def inspect_state(rpc_url: str, manifest: dict, wallet_value: str, bounty_value: str) -> dict:
    wallet = require_address(wallet_value, "entrant wallet")
    bounty = require_address(bounty_value, "competition bounty")
    safe = rpc(rpc_url, "eth_getBlockByNumber", ["safe", False], 1)
    if not isinstance(safe, dict) or not safe.get("number") or not safe.get("hash"):
        raise SystemExit("Base safe block is unavailable")
    block = str(safe["number"])
    safe_number = int(block, 16)
    timestamp = int(str(safe["timestamp"]), 16)
    observed_chain = int(str(rpc(rpc_url, "eth_chainId", [], 2)), 16)
    if observed_chain != int(manifest["chain_id"]):
        raise SystemExit("Base RPC chain does not match the entrant-wallet manifest")

    canonical = manifest["canonical"]
    wallet_manifest = manifest["entrant_wallet_factory"]
    wallet_factory = require_address(wallet_manifest["address"], "wallet factory")
    implementation = require_address(wallet_manifest["implementation"], "wallet implementation")
    competition_factory = require_address(canonical["competition_factory"], "competition factory")
    settlement_token = require_address(canonical["settlement_token"], "settlement token")
    addresses = (wallet_factory, implementation, wallet)
    hashes = {}
    for index, address in enumerate(addresses):
        code = str(rpc(rpc_url, "eth_getCode", [address, block], 10 + index)).lower()
        hashes[address] = code_hash(code)
    expected_hashes = (
        wallet_manifest["runtime_code_hash"],
        wallet_manifest["implementation_runtime_code_hash"],
        wallet_manifest["clone_runtime_code_hash"],
    )
    if tuple(hashes[address] for address in addresses) != expected_hashes:
        raise SystemExit("entrant-wallet factory, implementation, or clone runtime does not match the manifest")
    if word_address(one_word(rpc_url, wallet_factory, "implementation()(address)", block)) != implementation:
        raise SystemExit("entrant-wallet factory implementation binding changed")
    if (
        word_address(one_word(rpc_url, wallet_factory, "competitionFactory()(address)", block))
        != competition_factory
    ):
        raise SystemExit("entrant-wallet factory competition binding changed")
    if word_address(one_word(rpc_url, wallet_factory, "settlementToken()(address)", block)) != settlement_token:
        raise SystemExit("entrant-wallet factory token binding changed")
    if not bool(
        word_uint(one_word(rpc_url, wallet_factory, "isFactoryWallet(address)(bool)", block, (wallet,)))
    ):
        raise SystemExit("entrant wallet is not registered by the frozen factory")

    owner = word_address(one_word(rpc_url, wallet, "owner()(address)", block))
    policy_result = call(rpc_url, wallet, POLICY_SIGNATURE, block)
    policy, computed_policy_hash = policy_from_result(policy_result)
    onchain_policy_hash = f"0x{one_word(rpc_url, wallet, 'policyHash()(bytes32)', block)}"
    if computed_policy_hash != onchain_policy_hash:
        raise SystemExit("entrant-wallet policy hash does not match its canonical ABI encoding")
    wallet_state = {
        "owner": owner,
        "policy": policy,
        "policy_hash": onchain_policy_hash,
        "policy_version": word_uint(one_word(rpc_url, wallet, "policyVersion()(uint64)", block)),
        "delegate_nonce": word_uint(one_word(rpc_url, wallet, "delegateNonce()(uint256)", block)),
        "period_bucket": word_uint(one_word(rpc_url, wallet, "periodBucket()(uint256)", block)),
        "period_spent": word_uint(one_word(rpc_url, wallet, "periodSpent()(uint256)", block)),
        "lifetime_spent": word_uint(one_word(rpc_url, wallet, "lifetimeSpent()(uint256)", block)),
        "revoked": bool(word_uint(one_word(rpc_url, wallet, "revoked()(bool)", block))),
        "token_balance": word_uint(
            one_word(rpc_url, settlement_token, "balanceOf(address)(uint256)", block, (wallet,))
        ),
    }
    if wallet_state["revoked"] or not policy["valid_after"] <= timestamp <= policy["valid_until"]:
        raise SystemExit("entrant-wallet policy is revoked, pending, or expired")
    verifier_code = str(rpc(rpc_url, "eth_getCode", [policy["verifier_module"], block], 20)).lower()
    if code_hash(verifier_code) != policy["verifier_runtime_code_hash"]:
        raise SystemExit("entrant-wallet verifier runtime changed")

    if not bool(
        word_uint(
            one_word(
                rpc_url,
                competition_factory,
                "isCanonicalCompetition(address)(bool)",
                block,
                (bounty,),
            )
        )
    ):
        raise SystemExit("target is not a canonical Open Competition V1 bounty")
    entry_words = words(
        call(
            rpc_url,
            bounty,
            "entries(address)(bytes32,uint64,uint64,uint256,uint8)",
            block,
            (wallet,),
        )
    )
    if len(entry_words) != 5:
        raise SystemExit("competition entry returned an unexpected shape")
    entry = {
        "commitment": f"0x{entry_words[0]}",
        "committed_block": word_uint(entry_words[1]),
        "reveal_deadline": word_uint(entry_words[2]),
        "bond": word_uint(entry_words[3]),
        "state": word_uint(entry_words[4]),
    }
    bounty_state = {
        "factory": word_address(one_word(rpc_url, bounty, "factory()(address)", block)),
        "settlement_token": word_address(one_word(rpc_url, bounty, "settlementToken()(address)", block)),
        "creator": word_address(one_word(rpc_url, bounty, "creator()(address)", block)),
        "target_amount": word_uint(one_word(rpc_url, bounty, "targetAmount()(uint256)", block)),
        "verifier_reward": word_uint(one_word(rpc_url, bounty, "verifierReward()(uint256)", block)),
        "status": word_uint(one_word(rpc_url, bounty, "status()(uint8)", block)),
        "competition_ends_at": word_uint(one_word(rpc_url, bounty, "competitionEndsAt()(uint64)", block)),
        "entry_count": word_uint(one_word(rpc_url, bounty, "entryCount()(uint8)", block)),
        "max_entries": word_uint(one_word(rpc_url, bounty, "maxEntries()(uint8)", block)),
        "verifier_module": word_address(one_word(rpc_url, bounty, "verifierModule()(address)", block)),
        "policy_hash": f"0x{one_word(rpc_url, bounty, 'policyHash()(bytes32)', block)}",
        "acceptance_criteria_hash": f"0x{one_word(rpc_url, bounty, 'acceptanceCriteriaHash()(bytes32)', block)}",
        "benchmark_hash": f"0x{one_word(rpc_url, bounty, 'benchmarkHash()(bytes32)', block)}",
        "evidence_schema_hash": f"0x{one_word(rpc_url, bounty, 'evidenceSchemaHash()(bytes32)', block)}",
        "has_entered": bool(
            word_uint(one_word(rpc_url, bounty, "hasEntered(address)(bool)", block, (wallet,)))
        ),
        "entry": entry,
    }
    if bounty_state["factory"] != competition_factory or bounty_state["settlement_token"] != settlement_token:
        raise SystemExit("competition factory or settlement-token binding changed")
    expected_profile = {
        "verifier_module": policy["verifier_module"],
        "policy_hash": policy["verifier_policy_hash"],
        "acceptance_criteria_hash": policy["acceptance_criteria_hash"],
        "benchmark_hash": policy["benchmark_hash"],
        "evidence_schema_hash": policy["evidence_schema_hash"],
    }
    if any(bounty_state[name] != expected for name, expected in expected_profile.items()):
        raise SystemExit("competition does not match the wallet's pinned verifier profile")
    if bounty_state["target_amount"] > policy["max_bounty_target"]:
        raise SystemExit("competition target exceeds the entrant-wallet policy")
    return {
        "network": manifest["network"],
        "chain_id": observed_chain,
        "safe_block": {"number": safe_number, "hash": safe["hash"], "timestamp": timestamp},
        "wallet": wallet,
        "bounty": bounty,
        "wallet_state": wallet_state,
        "bounty_state": bounty_state,
    }


def validate_commitment_envelope(value: dict, report: dict) -> dict:
    required = {
        "schema_version",
        "network",
        "chain_id",
        "bounty",
        "solver",
        "submission_hash",
        "evidence_hash",
        "salt",
        "commitment",
        "committed_block",
        "reveal_deadline",
        "evidence_boundary",
    }
    if set(value) != required:
        raise SystemExit("commitment envelope keys are incomplete or unexpected")
    if value["schema_version"] != COMMITMENT_SCHEMA:
        raise SystemExit("commitment envelope schema is unsupported")
    if value["network"] != report["network"] or value["chain_id"] != report["chain_id"]:
        raise SystemExit("commitment envelope network or chain does not match")
    if require_address(str(value["bounty"]), "envelope bounty") != report["bounty"]:
        raise SystemExit("commitment envelope bounty does not match")
    if require_address(str(value["solver"]), "envelope solver") != report["wallet"]:
        raise SystemExit("commitment envelope solver must be the entrant wallet")
    for field in ("submission_hash", "evidence_hash", "salt", "commitment"):
        value[field] = require_bytes32(str(value[field]), field)
        if value[field] == ZERO_HASH:
            raise SystemExit(f"commitment envelope {field} cannot be zero")
    domain = run_cast("keccak", input_text="agent-bounties/open-competition-v1-solution")
    encoded = run_cast(
        "abi-encode",
        "f(bytes32,uint256,address,address,bytes32,bytes32,bytes32)",
        domain,
        str(report["chain_id"]),
        report["bounty"],
        report["wallet"],
        value["submission_hash"],
        value["evidence_hash"],
        value["salt"],
    )
    if run_cast("keccak", encoded) != value["commitment"]:
        raise SystemExit("commitment envelope does not reconstruct its commitment")
    return value


def build_plan(
    report: dict,
    action: str,
    envelope: dict | None,
    proof: str | None,
    deadline_seconds: int,
    *,
    exact_deadline: int | None = None,
) -> dict:
    if action not in ACTIONS:
        raise SystemExit("unsupported entrant-wallet action")
    if deadline_seconds < 60 or deadline_seconds > 900:
        raise SystemExit("deadline-seconds must be between 60 and 900")
    action_code = ACTIONS[action]
    policy = report["wallet_state"]["policy"]
    bounty = report["bounty_state"]
    entry = bounty["entry"]
    timestamp = report["safe_block"]["timestamp"]
    if not policy["allowed_actions"] & (1 << action_code):
        raise SystemExit("action is disabled by the entrant-wallet policy")
    if action == "commit":
        if envelope is None:
            raise SystemExit("commit requires a local commitment envelope")
        if bounty["status"] != 1 or timestamp >= bounty["competition_ends_at"]:
            raise SystemExit("competition is not open for a new commitment")
        if bounty["has_entered"] or bounty["entry_count"] >= bounty["max_entries"]:
            raise SystemExit("wallet already entered or competition capacity is full")
        if bounty["creator"] in {report["wallet_state"]["owner"], policy["delegate"]}:
            raise SystemExit("creator-controlled entrant wallet is ineligible")
        bond = bounty["verifier_reward"]
        bucket = timestamp // policy["period_seconds"]
        period_spent = (
            report["wallet_state"]["period_spent"]
            if bucket == report["wallet_state"]["period_bucket"]
            else 0
        )
        if (
            bond == 0
            or bond > policy["max_per_action"]
            or period_spent + bond > policy["max_per_period"]
            or report["wallet_state"]["lifetime_spent"] + bond > policy["max_lifetime_spend"]
            or report["wallet_state"]["token_balance"] < bond
        ):
            raise SystemExit("entry bond exceeds the wallet's remaining bounded budget")
        payload = run_cast("abi-encode", "f(address,bytes32)", report["bounty"], envelope["commitment"])
        direct_data = run_cast(
            "calldata", "commitSolution(address,bytes32)", report["bounty"], envelope["commitment"]
        )
        summary = {"commitment": envelope["commitment"], "maximum_gross_spend": str(bond)}
    elif action == "reveal":
        if envelope is None or proof is None:
            raise SystemExit("reveal requires a local commitment envelope and proof")
        if bounty["creator"] in {report["wallet_state"]["owner"], policy["delegate"]}:
            raise SystemExit("creator-controlled entrant wallet is ineligible")
        if bounty["status"] != 1 or entry["state"] != 1:
            raise SystemExit("wallet has no live committed entry")
        if entry["commitment"] != envelope["commitment"]:
            raise SystemExit("onchain commitment differs from the recovery envelope")
        if report["safe_block"]["number"] <= entry["committed_block"]:
            raise SystemExit("reveal requires a later safe block")
        if timestamp > entry["reveal_deadline"] or timestamp > bounty["competition_ends_at"]:
            raise SystemExit("reveal deadline has elapsed")
        payload = run_cast(
            "abi-encode",
            "f(address,bytes32,bytes32,bytes32,bytes)",
            report["bounty"],
            envelope["submission_hash"],
            envelope["evidence_hash"],
            envelope["salt"],
            proof,
        )
        direct_data = run_cast(
            "calldata",
            "revealSolution(address,bytes32,bytes32,bytes32,bytes)",
            report["bounty"],
            envelope["submission_hash"],
            envelope["evidence_hash"],
            envelope["salt"],
            proof,
        )
        summary = {
            "submission_hash": envelope["submission_hash"],
            "evidence_hash": envelope["evidence_hash"],
            "salt": envelope["salt"],
            "proof_hash": run_cast("keccak", proof),
            "maximum_gross_spend": "0",
        }
    else:
        if envelope is not None or proof is not None:
            raise SystemExit("withdraw_bond does not accept an envelope or proof")
        if bounty["status"] != 2 or entry["state"] != 1 or entry["bond"] == 0:
            raise SystemExit("wallet has no losing bond available for withdrawal")
        payload = run_cast("abi-encode", "f(address)", report["bounty"])
        direct_data = run_cast("calldata", "withdrawEntryBond(address)", report["bounty"])
        summary = {"recoverable_bond": str(entry["bond"]), "maximum_gross_spend": "0"}
    payload_hash = run_cast("keccak", payload)
    nonce = report["wallet_state"]["delegate_nonce"]
    deadline = (
        min(timestamp + deadline_seconds, policy["valid_until"])
        if exact_deadline is None
        else exact_deadline
    )
    if isinstance(deadline, bool) or not isinstance(deadline, int):
        raise SystemExit("entrant-wallet action deadline must be an integer")
    if deadline > timestamp + 900:
        raise SystemExit("entrant-wallet action deadline exceeds the 15-minute relay window")
    if deadline > policy["valid_until"]:
        raise SystemExit("entrant-wallet action deadline exceeds the policy validity window")
    if deadline <= timestamp:
        raise SystemExit("entrant-wallet policy expires too soon")
    wallet = report["wallet"]
    typed = {
        "types": {
            "EIP712Domain": [
                {"name": "name", "type": "string"},
                {"name": "version", "type": "string"},
                {"name": "chainId", "type": "uint256"},
                {"name": "verifyingContract", "type": "address"},
            ],
            "OpenCompetitionEntrantAction": [
                {"name": "wallet", "type": "address"},
                {"name": "action", "type": "uint8"},
                {"name": "payloadHash", "type": "bytes32"},
                {"name": "nonce", "type": "uint256"},
                {"name": "deadline", "type": "uint256"},
                {"name": "policyVersion", "type": "uint64"},
            ],
        },
        "primaryType": "OpenCompetitionEntrantAction",
        "domain": {
            "name": "Agent Bounties Open Competition Entrant Wallet",
            "version": "1",
            "chainId": report["chain_id"],
            "verifyingContract": wallet,
        },
        "message": {
            "wallet": wallet,
            "action": action_code,
            "payloadHash": payload_hash,
            "nonce": str(nonce),
            "deadline": str(deadline),
            "policyVersion": report["wallet_state"]["policy_version"],
        },
    }
    return {
        "schema_version": SCHEMA,
        "network": report["network"],
        "chain_id": report["chain_id"],
        "safe_block": report["safe_block"],
        "wallet": wallet,
        "delegate": policy["delegate"],
        "policy_hash": report["wallet_state"]["policy_hash"],
        "policy_version": report["wallet_state"]["policy_version"],
        "action": action,
        "action_code": action_code,
        "bounty": report["bounty"],
        "action_summary": summary,
        "nonce": nonce,
        "deadline": deadline,
        "payload": payload,
        "payload_hash": payload_hash,
        "direct_transaction": {
            "from": policy["delegate"],
            "to": wallet,
            "data": direct_data,
            "value": "0x0",
        },
        "signing_payload": typed,
        "relay_call": {
            "to": wallet,
            "function": "executeWithSignature(uint8,bytes,uint256,uint256,bytes)",
            "arguments_before_signature": [action_code, payload, nonce, deadline],
            "signature_tail": ["delegate_signature"],
        },
        "evidence_boundary": (
            "This safe-block plan is unsigned. A keeper may pay gas only for the exact signed action. "
            "Canonical competition events prove entry, reveal, or bond recovery; only BountySettled proves payout."
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=sorted(ACTIONS))
    parser.add_argument("--wallet", required=True)
    parser.add_argument("--bounty", required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--rpc-url")
    parser.add_argument("--commitment-envelope", type=Path)
    parser.add_argument("--proof")
    parser.add_argument("--deadline-seconds", type=int, default=300)
    parser.add_argument(
        "--output", type=Path, default=ROOT / "target" / "open-competition-entrant-action-plan.json"
    )
    args = parser.parse_args()
    manifest = validate_manifest(json.loads(args.manifest.read_text(encoding="utf-8")))
    rpc_url = args.rpc_url or str(manifest["rpc_url"])
    report = inspect_state(rpc_url, manifest, args.wallet, args.bounty)
    envelope = None
    if args.commitment_envelope:
        value = json.loads(args.commitment_envelope.read_text(encoding="utf-8"))
        if not isinstance(value, dict):
            raise SystemExit("commitment envelope must contain one JSON object")
        envelope = validate_commitment_envelope(value, report)
    proof = exact_hex_bytes(args.proof, "proof") if args.proof is not None else None
    plan = build_plan(report, args.action, envelope, proof, args.deadline_seconds)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(plan, indent=2) + "\n", encoding="utf-8")
    print(args.output)


if __name__ == "__main__":
    main()
