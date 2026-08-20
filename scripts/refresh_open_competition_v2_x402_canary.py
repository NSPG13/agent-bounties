#!/usr/bin/env python3
"""Replace an expired synthetic Beta3 x402 canary without changing release code."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import time
from typing import Any

from eth_account import Account

import open_competition_v2_proof_rehearsal as rehearsal
import run_open_competition_v2_sepolia_rehearsal as sepolia
from _shared.rpc import rpc


TARGET_FUNDING = 262_500
PROOF_WINDOW = 7_200
MINIMUM_SLA = 1_800
DEADLINE_MARGIN = 300
REPLACEMENT_FUNDING_STEP = 7 * 24 * 60 * 60
ZERO_ADDRESS = "0x" + "00" * 20


class CanaryRefreshError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise CanaryRefreshError(message)


def runtime_bundle(runtime: dict[str, Any], chain_id: int | None = None) -> dict[str, Any]:
    profiles = runtime.get("metric_programs")
    require(isinstance(profiles, list), "runtime metric programs are missing")
    profile = next(
        (value for value in profiles if value.get("profile_id") == "public-vector-metric-v1"),
        None,
    )
    require(isinstance(profile, dict), "reviewed public-vector metric profile is missing")
    require(profile.get("classification") == "reviewed", "public-vector metric profile is not reviewed")
    return {
        "chain_id": chain_id if chain_id is not None else int(runtime.get("chain_id", 84532)),
        "source_commit": runtime["source_commit"],
        "settlement_token": runtime["settlement_token"],
        "factory": {"address": runtime["factory_contract"]},
        "risk": {"hash": runtime["beta_risk_hash"]},
        "metric_profile": profile,
    }


def replacement_funding_deadline(old_proof_deadline: int, replacement_id: int) -> int:
    require(replacement_id > 0, "replacement ID must be positive")
    return old_proof_deadline + replacement_id * REPLACEMENT_FUNDING_STEP


def has_runtime_code(value: str) -> bool:
    require(isinstance(value, str) and value.startswith("0x"), "invalid bytecode response")
    payload = value[2:]
    return bool(payload) and int(payload, 16) != 0


def replace_rehearsal_canary(
    document: dict[str, Any], new_canary: dict[str, Any], evidence: dict[str, Any]
) -> dict[str, Any]:
    old = document.get("x402_canary")
    require(isinstance(old, dict), "preserved rehearsal has no x402 canary")
    superseded = list(document.get("superseded_x402_canaries", []))
    if old.get("competition", "").lower() != new_canary["competition"].lower():
        old_record = {
            **old,
            "active": False,
            "superseded_reason": "immutable_proof_deadline_below_broker_sla",
            "replacement_competition": new_canary["competition"],
            "recovery": evidence["superseded_recovery"],
        }
        superseded = [
            value
            for value in superseded
            if value.get("competition", "").lower() != old_record["competition"].lower()
        ]
        superseded.append(old_record)
    document["superseded_x402_canaries"] = superseded
    document["x402_canary"] = new_canary
    document["groth16_first_proven"] = {
        "competition": new_canary["competition"],
        "bounty_id": new_canary["bounty_id"],
        "pooled_funding": False,
        "settled": False,
        "settlement_deferred_to_x402": True,
    }
    document["x402_canary_replacement"] = {
        "replacement_id": evidence["replacement_id"],
        "competition": new_canary["competition"],
        "evidence_file": "x402-canary-replacement.json",
    }
    document["solver_test_usdc_retained_for_x402"] = True
    document["test_usdc_reclaimed"] = False
    return document


def atomic_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def transaction_record(receipt: dict[str, Any]) -> dict[str, Any]:
    return {
        "transaction_hash": sepolia.receipt_hash(receipt),
        "block_number": int(receipt["blockNumber"], 16),
        "block_hash": receipt["blockHash"].lower(),
    }


def word(url: str, address: str, signature: str) -> str:
    return rehearsal.call(url, address, signature).lower()


def recover_superseded(
    client: sepolia.SignedRpc,
    signer: Any,
    token: str,
    canary: dict[str, Any],
    receipts: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    competition = canary["competition"].lower()
    require(
        rehearsal.call_address(client.url, competition, "creator()") == signer.address.lower(),
        "superseded canary creator mismatch",
    )
    status = rehearsal.call_uint(client.url, competition, "status()")
    deadline = rehearsal.call_uint(client.url, competition, "proofDeadline()")
    latest = rehearsal.latest_timestamp(client.url)
    if status == 1 and latest > deadline:
        require(
            rehearsal.call_address(client.url, competition, "leader()") == ZERO_ADDRESS,
            "superseded first-proven canary unexpectedly has a leader",
        )
        receipts["expire_superseded"] = client.send(
            signer,
            to=competition,
            data=rehearsal.function_data("expireCompetition()", [], []),
        )
        status = rehearsal.call_uint(client.url, competition, "status()")
    require(status in (1, 3), f"superseded canary has unexpected status {status}")
    contribution = rehearsal.call_uint(
        client.url,
        competition,
        "contributions(address)",
        ["address"],
        [signer.address],
    )
    if status == 3 and contribution:
        receipts["refund_superseded"] = client.send(
            signer,
            to=competition,
            data=rehearsal.function_data(
                "withdrawRefundFor(address)", ["address"], [signer.address]
            ),
        )
        contribution = rehearsal.call_uint(
            client.url,
            competition,
            "contributions(address)",
            ["address"],
            [signer.address],
        )
    recovered = status == 3 and contribution == 0
    if recovered:
        require(
            rehearsal.token_balance(client.url, token, competition) == 0,
            "superseded canary retained USDC after recovery",
        )
    return {
        "competition": competition,
        "proof_deadline": deadline,
        "status": "recovered" if recovered else "awaiting_expiry",
        "recovered": recovered,
        "remaining_creator_contribution": str(contribution),
    }


def verify_new_canary(
    url: str,
    token: str,
    competition: str,
    bounty_id: str,
    signer: Any,
    params: tuple[Any, ...],
) -> int:
    expected_words = {
        "bountyId()": bounty_id,
        "proofSystem()": "0x" + params[7].hex(),
        "programVKey()": "0x" + params[8].hex(),
        "sourceHash()": "0x" + params[9].hex(),
        "elfHash()": "0x" + params[10].hex(),
        "journalSchemaHash()": "0x" + params[11].hex(),
        "metricProgramHash()": "0x" + params[12].hex(),
        "executionPolicyHash()": "0x" + params[13].hex(),
        "verificationPolicyHash()": "0x" + params[14].hex(),
        "settlementPolicyHash()": "0x" + params[15].hex(),
        "betaRiskHash()": "0x" + params[16].hex(),
    }
    for signature, expected in expected_words.items():
        require(word(url, competition, signature) == expected.lower(), f"{signature} mismatch")
    require(rehearsal.call_address(url, competition, "creator()") == signer.address.lower(), "replacement creator mismatch")
    require(rehearsal.call_uint(url, competition, "solverReward()") == 250_000, "replacement solver reward mismatch")
    require(rehearsal.call_uint(url, competition, "keeperReward()") == 12_500, "replacement keeper reward mismatch")
    require(rehearsal.call_uint(url, competition, "fundedAmount()") == TARGET_FUNDING, "replacement funding mismatch")
    require(rehearsal.call_uint(url, competition, "status()") == 1, "replacement canary is not active")
    require(rehearsal.token_balance(url, token, competition) == TARGET_FUNDING, "replacement custody mismatch")
    deadline = rehearsal.call_uint(url, competition, "proofDeadline()")
    require(
        deadline > rehearsal.latest_timestamp(url) + MINIMUM_SLA + DEADLINE_MARGIN,
        "replacement proof deadline cannot satisfy the broker SLA",
    )
    return deadline


def run(args: argparse.Namespace) -> dict[str, Any]:
    require(args.rpc_url.startswith("https://"), "replacement RPC must use HTTPS")
    chain_id = 8453 if args.network == "base-mainnet" else 84532
    sepolia.configure_network(argparse.Namespace(network=args.network, rpc_url=args.rpc_url))
    runtime = json.loads(args.runtime.read_text(encoding="utf-8"))
    document = json.loads(args.rehearsal.read_text(encoding="utf-8"))
    bundle = runtime_bundle(runtime, chain_id)
    require(runtime.get("network") == args.network, "runtime network differs from the requested network")
    require(document.get("passed") is True, "preserved rehearsal did not pass")
    require(document.get("source_commit") == runtime.get("source_commit"), "release source mismatch")

    raw_key = sepolia.normalized_key(os.environ.get(args.private_key_env, ""))
    signer, solver, _ = sepolia.actors_for(raw_key, bundle, args.actor_derivation_salt)
    require(signer.address.lower() == document["actors"]["deployer"], "deployer identity mismatch")
    require(solver.address.lower() == document["actors"]["solver_a"], "solver identity mismatch")
    require(
        sepolia.actor_derivation_id(bundle["source_commit"], args.actor_derivation_salt)
        == document.get("actor_derivation_id"),
        "actor derivation identity mismatch",
    )
    client = sepolia.SignedRpc(args.rpc_url)
    token = bundle["settlement_token"]
    old = document["x402_canary"]
    receipts: dict[str, dict[str, Any]] = {}
    recovery = recover_superseded(client, signer, token, old, receipts)

    template = json.loads(args.fixture.read_text(encoding="utf-8"))
    network_label = "mainnet" if args.network == "base-mainnet" else "sepolia"
    label = f"{network_label}-x402-first-replacement-{args.replacement_id}"
    params = list(
        rehearsal.params(
            client.url,
            bundle,
            template,
            label=label,
            proof_system="groth16",
            winner_mode=0,
            solver_reward=250_000,
            keeper_reward=12_500,
            proof_window=PROOF_WINDOW,
            funding_window=REPLACEMENT_FUNDING_STEP,
        )
    )
    params[2] = replacement_funding_deadline(int(old["proof_deadline"]), args.replacement_id)
    require(params[2] > rehearsal.latest_timestamp(client.url), "replacement funding deadline has passed")
    params_tuple = tuple(params)
    creation_nonce = rehearsal.b32(
        rehearsal.hash_label(f"{bundle['source_commit']}:{label}")
    )
    factory = bundle["factory"]["address"]
    competition, bounty_id = rehearsal.predict(
        client.url, factory, signer.address, params_tuple, creation_nonce
    )
    fixture = rehearsal.fixture_builder.bind(
        template,
        rehearsal.scope(
            bundle, params_tuple, competition, bounty_id, solver.address, 3, "groth16"
        ),
    )

    code = rpc(client.url, "eth_getCode", [competition, "latest"])
    if not has_runtime_code(code):
        require(
            rehearsal.token_balance(client.url, token, signer.address) >= TARGET_FUNDING,
            "deployer lacks USDC for the replacement canary",
        )
        receipts["approve_replacement"] = client.send(
            signer,
            to=token,
            data=rehearsal.function_data(
                "approve(address,uint256)", ["address", "uint256"], [competition, TARGET_FUNDING]
            ),
        )
        receipts["create_replacement"] = client.send(
            signer,
            to=factory,
            data=rehearsal.function_data(
                f"createCompetition({rehearsal.PARAM_TYPE},uint256,bytes32,bytes32)",
                [rehearsal.PARAM_TYPE, "uint256", "bytes32", "bytes32"],
                [params_tuple, TARGET_FUNDING, creation_nonce, rehearsal.b32(bundle["risk"]["hash"])],
            ),
        )
    proof_deadline = verify_new_canary(
        client.url, token, competition, bounty_id, signer, params_tuple
    )
    safe = (
        sepolia.wait_safe(client.url, list(receipts.values()), args.safe_timeout)
        if receipts
        else {
            "number": int(rpc(client.url, "eth_getBlockByNumber", ["safe", False])["number"], 16),
            "hash": rpc(client.url, "eth_getBlockByNumber", ["safe", False])["hash"].lower(),
        }
    )
    new_canary = sepolia.x402_canary_spec(
        fixture, competition, bounty_id, solver.address, 3
    )
    new_canary.update(
        {
            "proof_deadline": proof_deadline,
            "active": True,
            "replacement_id": args.replacement_id,
            "replaces_competition": old["competition"].lower(),
        }
    )
    evidence = {
        "schema_version": "agent-bounties/open-competition-v2-beta3-x402-canary-replacement-v1",
        "passed": True,
        "network": args.network,
        "chain_id": chain_id,
        "source_commit": bundle["source_commit"],
        "replacement_id": args.replacement_id,
        "competition": competition,
        "bounty_id": bounty_id,
        "solver": solver.address.lower(),
        "proof_deadline": proof_deadline,
        "minimum_broker_sla_seconds": MINIMUM_SLA,
        "superseded_recovery": recovery,
        "transactions": {name: transaction_record(value) for name, value in receipts.items()},
        "safe_block": safe,
        "evidence_boundary": (
            "Synthetic Base mainnet canary replacement; canonical Base USDC moved only through the isolated release contracts."
            if args.network == "base-mainnet"
            else "Synthetic Base Sepolia canary replacement only; no mainnet value moved."
        ),
    }
    replace_rehearsal_canary(document, new_canary, evidence)
    atomic_json(args.rehearsal, document)
    atomic_json(args.output, evidence)
    return evidence


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--network", choices=("base-sepolia", "base-mainnet"), default="base-sepolia"
    )
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--rehearsal", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--private-key-env", default="BASE_SEPOLIA_DEPLOYER_PRIVATE_KEY")
    parser.add_argument("--actor-derivation-salt", required=True)
    parser.add_argument("--replacement-id", type=int, default=1)
    parser.add_argument("--safe-timeout", type=int, default=1_800)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    result = run(args)
    print(json.dumps({"passed": True, "competition": result["competition"], "output": str(args.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
