#!/usr/bin/env python3
"""Rehearse Open Competition V2 Beta2 on live Base Sepolia or mainnet."""

from __future__ import annotations

import argparse
from copy import deepcopy
import json
import os
from pathlib import Path
import time
from typing import Any

from eth_account import Account

import build_open_competition_v2_beta2_release as release
import open_competition_v2_proof_rehearsal as rehearsal
from _shared.evm import keccak256, keccak_bytes
from _shared.rpc import rpc


ROOT = Path(__file__).resolve().parents[1]
PROGRAM_ROOT = ROOT / "programs" / "public-vector-metric-v1"
CHAIN_ID = 84532
NETWORK = "base-sepolia"
RUN_LABEL = "sepolia"
PREPARED_FUNDING_WINDOW = 7 * 24 * 60 * 60
SETTLED_TOPIC = rehearsal.SETTLED_TOPIC
CANCELLED_TOPIC = keccak256(
    b"CompetitionCancelledV2(bytes32,address,uint256,uint256,uint256,bytes32)"
)
REFUND_TOPIC = keccak256(
    b"CompetitionRefundWithdrawnV2(bytes32,address,address,uint256)"
)


class SepoliaRehearsalError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SepoliaRehearsalError(message)


def normalized_key(value: str) -> bytes:
    try:
        raw = bytes.fromhex(value.strip().removeprefix("0x"))
    except ValueError as error:
        raise SepoliaRehearsalError("signer key must be hexadecimal") from error
    require(len(raw) == 32 and int.from_bytes(raw, "big") != 0, "signer key must be 32 non-zero bytes")
    return raw


def actor_derivation_id(source_commit: str, derivation_salt: str) -> str:
    require(
        1 <= len(derivation_salt) <= 200,
        "actor derivation salt must contain 1 to 200 characters",
    )
    return keccak256(
        b"agent-bounties/open-competition-v2-beta2/actor-derivation\0"
        + NETWORK.encode()
        + b"\0"
        + bytes.fromhex(source_commit)
        + b"\0"
        + derivation_salt.encode()
    )


def derived_actor(
    root_key: bytes, source_commit: str, label: str, derivation_salt: str = "local"
) -> Any:
    actor_derivation_id(source_commit, derivation_salt)
    material = keccak_bytes(
        b"agent-bounties/open-competition-v2-beta2/actor\0"
        + NETWORK.encode()
        + b"\0"
        + root_key
        + bytes.fromhex(source_commit)
        + b"\0"
        + derivation_salt.encode()
        + b"\0"
        + label.encode()
    )
    require(int.from_bytes(material, "big") != 0, "derived an invalid actor key")
    return Account.from_key(material)


def receipt_hash(receipt: dict[str, Any]) -> str:
    return str(receipt["transactionHash"]).lower()


def has_topic(receipt: dict[str, Any], topic: str) -> bool:
    return any(log.get("topics", [None])[0] == topic for log in receipt.get("logs", []))


def proof_summary(value: dict[str, Any]) -> dict[str, Any]:
    proof = bytes.fromhex(value["proof_hex"].removeprefix("0x"))
    journal = bytes.fromhex(value["journal_hex"].removeprefix("0x"))
    return {
        "mode": value["mode"],
        "proof_hash": keccak256(proof),
        "proof_bytes": len(proof),
        "journal_hash": keccak256(journal),
        "journal_bytes": len(journal),
        "elapsed_seconds": value["elapsed_seconds"],
    }


def x402_canary_spec(
    fixture: dict[str, Any], competition: str, bounty_id: str, solver: str, solver_nonce: int
) -> dict[str, Any]:
    journal = rehearsal.expected_journal(fixture)
    return {
        "competition": competition,
        "bounty_id": bounty_id,
        "solver": solver.lower(),
        "solver_nonce": str(solver_nonce),
        "artifact_hash": "0x" + journal[6 * 32 : 7 * 32].hex(),
        "proof_system": "groth16",
        "metric": {
            "mode": fixture["mode"],
            "threshold": str(fixture["threshold"]),
            "vectors": fixture["vectors"],
        },
    }


class SignedRpc:
    def __init__(self, url: str) -> None:
        self.url = url
        require(int(rpc(url, "eth_chainId", []), 16) == CHAIN_ID, f"RPC is not {NETWORK}")

    def fees(self) -> tuple[int, int]:
        block = rpc(self.url, "eth_getBlockByNumber", ["latest", False])
        base_fee = int(block.get("baseFeePerGas", "0x0"), 16)
        try:
            priority = int(rpc(self.url, "eth_maxPriorityFeePerGas", []), 16)
        except RuntimeError:
            priority = 1_000_000
        priority = max(priority, 1_000_000)
        return base_fee * 2 + priority, priority

    def send(self, actor: Any, *, to: str | None = None, data: str = "0x", value: int = 0) -> dict[str, Any]:
        nonce = int(rpc(self.url, "eth_getTransactionCount", [actor.address, "pending"]), 16)
        maximum, priority = self.fees()
        transaction: dict[str, Any] = {
            "chainId": CHAIN_ID,
            "from": actor.address,
            "nonce": nonce,
            "value": value,
            "data": data,
            "maxFeePerGas": maximum,
            "maxPriorityFeePerGas": priority,
            "type": 2,
        }
        estimate_request = {
            "from": actor.address,
            "value": hex(value),
            "data": data,
            "maxFeePerGas": hex(maximum),
            "maxPriorityFeePerGas": hex(priority),
        }
        if to is not None:
            transaction["to"] = to
            estimate_request["to"] = to
        gas = int(rpc(self.url, "eth_estimateGas", [estimate_request]), 16)
        transaction["gas"] = gas * 5 // 4 + 25_000
        signed = actor.sign_transaction(transaction)
        transaction_hash = rpc(
            self.url, "eth_sendRawTransaction", ["0x" + bytes(signed.raw_transaction).hex()]
        )
        return self.wait_receipt(transaction_hash)

    def wait_receipt(self, transaction_hash: str, timeout: int = 300) -> dict[str, Any]:
        deadline = time.time() + timeout
        while time.time() < deadline:
            receipt = rpc(self.url, "eth_getTransactionReceipt", [transaction_hash])
            if receipt:
                require(int(receipt["status"], 16) == 1, f"transaction reverted: {transaction_hash}")
                target = int(receipt["blockNumber"], 16) + 2
                while int(rpc(self.url, "eth_blockNumber", []), 16) < target:
                    require(time.time() < deadline, "confirmation wait timed out")
                    time.sleep(1)
                canonical = rpc(self.url, "eth_getBlockByNumber", [receipt["blockNumber"], False])
                if canonical and canonical["hash"].lower() == receipt["blockHash"].lower():
                    return receipt
            time.sleep(1)
        raise SepoliaRehearsalError(f"receipt timed out: {transaction_hash}")


def runtime_hash(url: str, address: str, block: str = "latest") -> tuple[str, int]:
    code = bytes.fromhex(rpc(url, "eth_getCode", [address, block]).removeprefix("0x"))
    if not code:
        return "0x" + "00" * 32, 0
    return keccak256(code), len(code)


def bundle_for_nonce(bundle: dict[str, Any], nonce: int) -> dict[str, Any]:
    preflight = deepcopy(bundle["preflight_safe_block"])
    preflight["deployer_nonce"] = nonce
    return release.build_bundle(
        network_name=NETWORK,
        deployer=bundle["deployer"],
        source_commit=bundle["source_commit"],
        repository_subject=bundle["repository_subject"]["hash"],
        preflight=preflight,
        gates=deepcopy(bundle["release_gates"]),
        verifier_assets=release.load_verifier_assets(),
    )


def resolve_or_deploy_factory(client: SignedRpc, signer: Any, bundle: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any] | None]:
    require(bundle.get("network") == NETWORK and bundle.get("chain_id") == CHAIN_ID, f"release bundle is not {NETWORK}")
    require(signer.address.lower() == bundle["deployer"], "signer does not match release bundle deployer")
    pending_nonce = int(rpc(client.url, "eth_getTransactionCount", [signer.address, "pending"]), 16)

    candidates = [bundle]
    for nonce in range(max(0, pending_nonce - 32), pending_nonce):
        if nonce != bundle["factory"]["from_nonce"]:
            candidates.append(bundle_for_nonce(bundle, nonce))
    for candidate in candidates:
        observed, size = runtime_hash(client.url, candidate["factory"]["address"])
        if size and observed == candidate["factory"]["runtime_code_hash"]:
            return candidate, None

    require(NETWORK == "base-sepolia", "mainnet rehearsal requires the exact factory to be deployed first")
    require(pending_nonce == bundle["factory"]["from_nonce"], "deployer nonce moved and no exact prior Beta2 factory was found; rebuild the release bundle")
    observed, size = runtime_hash(client.url, bundle["factory"]["address"])
    require(size == 0, f"predicted factory is occupied by {observed}")
    receipt = client.send(signer, data=bundle["factory"]["deployment_calldata"])
    require(
        str(receipt.get("contractAddress", "")).lower() == bundle["factory"]["address"],
        "deployed factory address differs from the release bundle",
    )
    return bundle, receipt


def verify_components(url: str, bundle: dict[str, Any]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key in ("factory", "groth16_adapter", "plonk_adapter", "implementation"):
        expected = bundle[key]
        observed_hash, observed_bytes = runtime_hash(url, expected["address"])
        require(observed_hash == expected["runtime_code_hash"], f"{key} runtime hash mismatch")
        require(observed_bytes == expected["runtime_code_bytes"], f"{key} runtime length mismatch")
        result[key] = {
            "address": expected["address"],
            "runtime_code_hash": observed_hash,
            "runtime_code_bytes": observed_bytes,
        }
    return result


def wait_until_chain_time(url: str, timestamp: int, timeout: int = 300) -> None:
    deadline = time.time() + timeout
    while rehearsal.latest_timestamp(url) <= timestamp:
        require(time.time() < deadline, "chain timestamp wait timed out")
        time.sleep(1)


def wait_safe(url: str, receipts: list[dict[str, Any]], timeout: int) -> dict[str, Any]:
    target = max(int(receipt["blockNumber"], 16) for receipt in receipts)
    deadline = time.time() + timeout
    while time.time() < deadline:
        safe = rpc(url, "eth_getBlockByNumber", ["safe", False])
        if safe and int(safe["number"], 16) >= target:
            for receipt in receipts:
                canonical = rpc(url, "eth_getBlockByNumber", [receipt["blockNumber"], False])
                require(canonical and canonical["hash"].lower() == receipt["blockHash"].lower(), "receipt was reorged before safe reconciliation")
            return {
                "number": int(safe["number"], 16),
                "hash": safe["hash"].lower(),
                "timestamp": int(safe["timestamp"], 16),
            }
        time.sleep(2)
    raise SepoliaRehearsalError(f"{NETWORK} safe-block reconciliation timed out")


def actors_for(
    raw_key: bytes, bundle: dict[str, Any], derivation_salt: str
) -> tuple[Any, Any, Any]:
    return (
        Account.from_key(raw_key),
        derived_actor(raw_key, bundle["source_commit"], "solver-a", derivation_salt),
        derived_actor(raw_key, bundle["source_commit"], "solver-b", derivation_salt),
    )


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    raw_key = normalized_key(os.environ.get(args.private_key_env, ""))
    signer = Account.from_key(raw_key)
    raw_bundle = json.loads(args.bundle.read_text(encoding="utf-8"))
    client = SignedRpc(args.rpc_url)
    bundle, deployment_receipt = resolve_or_deploy_factory(client, signer, raw_bundle)
    components = verify_components(client.url, bundle)
    signer, solver_a, solver_b = actors_for(raw_key, bundle, args.actor_derivation_salt)
    context = rehearsal.prepare_context(
        client.url,
        bundle,
        args.prepare_proof_fixtures,
        creator=signer.address,
        solver_a=solver_a.address,
        solver_b=solver_b.address,
        first_label=f"{RUN_LABEL}-groth16-first",
        best_label=f"{RUN_LABEL}-plonk-best",
        first_nonce_label=f"{bundle['source_commit']}:{RUN_LABEL}-groth16-first",
        best_nonce_label=f"{bundle['source_commit']}:{RUN_LABEL}-plonk-best",
        proof_window=90,
        funding_window=PREPARED_FUNDING_WINDOW,
    )
    args.resolved_bundle_output.parent.mkdir(parents=True, exist_ok=True)
    args.resolved_bundle_output.write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
    if args.resolved_runtime_output:
        runtime = release.runtime_manifest(
            bundle, int(bundle["preflight_safe_block"]["number"])
        )
        args.resolved_runtime_output.parent.mkdir(parents=True, exist_ok=True)
        args.resolved_runtime_output.write_text(
            json.dumps(runtime, indent=2) + "\n", encoding="utf-8"
        )
    result = {
        "schema_version": f"agent-bounties/open-competition-v2-beta2-{RUN_LABEL}-preparation-v1",
        "passed": True,
        "broadcast": deployment_receipt is not None,
        "factory_deployment_transaction": receipt_hash(deployment_receipt) if deployment_receipt else None,
        "source_commit": bundle["source_commit"],
        "components": components,
        "actors": {
            "deployer": signer.address.lower(),
            "solver_a": solver_a.address.lower(),
            "solver_b": solver_b.address.lower(),
        },
        "actor_derivation_id": actor_derivation_id(
            bundle["source_commit"], args.actor_derivation_salt
        ),
        "proof_context_hash": context["context_hash"],
        "funds_moved": False,
        "evidence_boundary": "Factory preparation and proof-input binding only. No competition was funded or settled.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return result


def run(args: argparse.Namespace) -> dict[str, Any]:
    raw_key = normalized_key(os.environ.get(args.private_key_env, ""))
    signer = Account.from_key(raw_key)
    raw_bundle = json.loads(args.bundle.read_text(encoding="utf-8"))
    client = SignedRpc(args.rpc_url)
    bundle, deployment_receipt = resolve_or_deploy_factory(client, signer, raw_bundle)
    components = verify_components(client.url, bundle)

    signer, solver_a, solver_b = actors_for(raw_key, bundle, args.actor_derivation_salt)
    actors = {"deployer": signer.address.lower(), "solver_a": solver_a.address.lower(), "solver_b": solver_b.address.lower()}
    token = bundle["settlement_token"]
    factory = bundle["factory"]["address"]
    risk_hash = rehearsal.b32(bundle["risk"]["hash"])
    work = args.output.parent / f"open-competition-v2-{RUN_LABEL}-proof-work"
    work.mkdir(parents=True, exist_ok=True)
    prepared = args.prepared_proof_dir or (work / "prepared")
    if args.prepared_proof_dir is None:
        rehearsal.prepare_context(
            client.url,
            bundle,
            prepared,
            creator=signer.address,
            solver_a=solver_a.address,
            solver_b=solver_b.address,
            first_label=f"{RUN_LABEL}-groth16-first",
            best_label=f"{RUN_LABEL}-plonk-best",
            first_nonce_label=f"{bundle['source_commit']}:{RUN_LABEL}-groth16-first",
            best_nonce_label=f"{bundle['source_commit']}:{RUN_LABEL}-plonk-best",
            proof_window=90,
            funding_window=PREPARED_FUNDING_WINDOW,
        )
    context, fixtures = rehearsal.load_context(bundle, prepared)
    require(context["actors"] == actors, "prepared proof actor set changed")
    required_proofs = (
        {"plonk_best_a", "plonk_best_b"}
        if args.x402_replaces_first_proven
        else set(rehearsal.PROOF_SPECS)
    )
    if args.proof_evidence_dir is None:
        proofs = {}
        for name, (mode, label) in rehearsal.PROOF_SPECS.items():
            if name not in required_proofs:
                continue
            evidence = rehearsal.prove(fixtures[name], mode, f"{RUN_LABEL}-{label}", work)
            proofs[name] = rehearsal.validate_proof_evidence(
                bundle, context, name, fixtures[name], evidence
            )
    else:
        proofs = {}
        for name in required_proofs:
            evidence = json.loads(
                (args.proof_evidence_dir / f"{name}.json").read_text(encoding="utf-8")
            )
            proofs[name] = rehearsal.validate_proof_evidence(
                bundle, context, name, fixtures[name], evidence
            )

    first_params = rehearsal.params_tuple(context["first"]["params"])
    first_nonce = rehearsal.b32(context["first"]["nonce"])
    first_address, first_id = rehearsal.predict(client.url, factory, signer.address, first_params, first_nonce)
    best_params = rehearsal.params_tuple(context["best"]["params"])
    best_nonce = rehearsal.b32(context["best"]["nonce"])
    best_address, best_id = rehearsal.predict(client.url, factory, signer.address, best_params, best_nonce)
    require(
        (first_address, first_id) == (context["first"]["address"], context["first"]["bounty_id"]),
        "prepared first-proven competition identity changed",
    )
    require(
        (best_address, best_id) == (context["best"]["address"], context["best"]["bounty_id"]),
        "prepared best-score competition identity changed",
    )

    receipts: dict[str, dict[str, Any]] = {}
    if deployment_receipt:
        receipts["factory_deployment"] = deployment_receipt
    for name, actor in (("solver_a", solver_a), ("solver_b", solver_b)):
        if int(rpc(client.url, "eth_getBalance", [actor.address, "latest"]), 16) < args.actor_eth_wei:
            receipts[f"fund_{name}_gas"] = client.send(signer, to=actor.address, value=args.actor_eth_wei)

    solver_a_funding = 110_000 if args.x402_replaces_first_proven else 162_500
    receipts["fund_solver_a_usdc"] = client.send(signer, to=token, data=rehearsal.function_data("transfer(address,uint256)", ["address", "uint256"], [solver_a.address, solver_a_funding]))
    if not args.x402_replaces_first_proven:
        receipts["approve_first_creator"] = client.send(signer, to=token, data=rehearsal.function_data("approve(address,uint256)", ["address", "uint256"], [first_address, 100_000]))
        receipts["create_first"] = client.send(signer, to=factory, data=rehearsal.function_data(f"createCompetition({rehearsal.PARAM_TYPE},uint256,bytes32,bytes32)", [rehearsal.PARAM_TYPE, "uint256", "bytes32", "bytes32"], [first_params, 100_000, first_nonce, risk_hash]))
        receipts["approve_first_pool"] = client.send(solver_a, to=token, data=rehearsal.function_data("approve(address,uint256)", ["address", "uint256"], [first_address, 162_500]))
        receipts["fund_first_pool"] = client.send(solver_a, to=first_address, data=rehearsal.function_data("fund(uint256,bytes32)", ["uint256", "bytes32"], [162_500, risk_hash]))
        first_before = rehearsal.token_balance(client.url, token, solver_a.address)
        receipts["settle_first"] = client.send(solver_a, to=first_address, data=rehearsal.function_data("submitProof(bytes,bytes)", ["bytes", "bytes"], [bytes.fromhex(proofs["groth16_first"]["journal_hex"][2:]), bytes.fromhex(proofs["groth16_first"]["proof_hex"][2:])]))
        require(has_topic(receipts["settle_first"], SETTLED_TOPIC), "Groth16 settlement event missing")
        require(rehearsal.token_balance(client.url, token, solver_a.address) - first_before == 262_500, "Groth16 payout mismatch")

    receipts["approve_best"] = client.send(signer, to=token, data=rehearsal.function_data("approve(address,uint256)", ["address", "uint256"], [best_address, 262_500]))
    receipts["create_best"] = client.send(signer, to=factory, data=rehearsal.function_data(f"createCompetition({rehearsal.PARAM_TYPE},uint256,bytes32,bytes32)", [rehearsal.PARAM_TYPE, "uint256", "bytes32", "bytes32"], [best_params, 262_500, best_nonce, risk_hash]))
    for actor, proof_name in ((solver_a, "plonk_best_a"), (solver_b, "plonk_best_b")):
        evidence = proofs[proof_name]
        receipts[f"submit_{proof_name}"] = client.send(actor, to=best_address, data=rehearsal.function_data("submitProof(bytes,bytes)", ["bytes", "bytes"], [bytes.fromhex(evidence["journal_hex"][2:]), bytes.fromhex(evidence["proof_hex"][2:])]))
    require(rehearsal.call_address(client.url, best_address, "leader()") == solver_b.address.lower(), "best-score leader mismatch")
    wait_until_chain_time(client.url, rehearsal.call_uint(client.url, best_address, "proofDeadline()"))
    best_before = rehearsal.token_balance(client.url, token, solver_b.address)
    keeper_before = rehearsal.token_balance(client.url, token, signer.address)
    receipts["finalize_best"] = client.send(signer, to=best_address, data=rehearsal.function_data("finalizeBestScore()", [], []))
    require(has_topic(receipts["finalize_best"], SETTLED_TOPIC), "PLONK settlement event missing")
    require(rehearsal.token_balance(client.url, token, solver_b.address) - best_before == 250_000, "PLONK solver payout mismatch")
    require(rehearsal.token_balance(client.url, token, signer.address) - keeper_before == 12_500, "PLONK keeper payout mismatch")

    expiry_result = None
    if not args.skip_expiry_refund:
        expiry_template = json.loads(
            (PROGRAM_ROOT / "fixtures/rehearsal-first-proven.json").read_text(encoding="utf-8")
        )
        expiry_params = rehearsal.params(client.url, bundle, expiry_template, label=f"{RUN_LABEL}-expiry", proof_system="groth16", winner_mode=0, solver_reward=100_000, keeper_reward=5_000, proof_window=1)
        expiry_nonce = rehearsal.b32(rehearsal.hash_label(f"{bundle['source_commit']}:{RUN_LABEL}-expiry"))
        expiry_address, expiry_id = rehearsal.predict(client.url, factory, signer.address, expiry_params, expiry_nonce)
        receipts["approve_expiry"] = client.send(signer, to=token, data=rehearsal.function_data("approve(address,uint256)", ["address", "uint256"], [expiry_address, 105_000]))
        receipts["create_expiry"] = client.send(signer, to=factory, data=rehearsal.function_data(f"createCompetition({rehearsal.PARAM_TYPE},uint256,bytes32,bytes32)", [rehearsal.PARAM_TYPE, "uint256", "bytes32", "bytes32"], [expiry_params, 105_000, expiry_nonce, risk_hash]))
        invalid_proof = bytearray(bytes.fromhex(proofs["groth16_first"]["proof_hex"][2:]))
        invalid_proof[-1] ^= 1
        invalid_call = rehearsal.function_data(
            "submitProof(bytes,bytes)",
            ["bytes", "bytes"],
            [bytes.fromhex(proofs["groth16_first"]["journal_hex"][2:]), bytes(invalid_proof)],
        )
        try:
            rpc(
                client.url,
                "eth_call",
                [{"from": solver_a.address, "to": expiry_address, "data": invalid_call}, "latest"],
            )
        except RuntimeError:
            invalid_proof_rejected = True
        else:
            raise SepoliaRehearsalError("the pinned verifier accepted a malformed proof")
        wait_until_chain_time(client.url, rehearsal.call_uint(client.url, expiry_address, "proofDeadline()"))
        receipts["expire"] = client.send(signer, to=expiry_address, data=rehearsal.function_data("expireCompetition()", [], []))
        require(has_topic(receipts["expire"], CANCELLED_TOPIC), "expiry cancellation event missing")
        receipts["refund"] = client.send(solver_b, to=expiry_address, data=rehearsal.function_data("withdrawRefundFor(address)", ["address"], [signer.address]))
        require(has_topic(receipts["refund"], REFUND_TOPIC), "permissionless refund event missing")
        require(rehearsal.token_balance(client.url, token, expiry_address) == 0, "expiry escrow retained USDC")
        expiry_result = {
            "competition": expiry_address,
            "bounty_id": expiry_id,
            "invalid_proof_rejected": invalid_proof_rejected,
            "permissionless_refund": True,
            "escrow_balance": 0,
        }

    x402_result = None
    if args.prepare_x402_canary:
        template = json.loads(
            (PROGRAM_ROOT / "fixtures/rehearsal-best-score-a.json").read_text(encoding="utf-8")
        )
        solver_nonce = 3
        x402_params = rehearsal.params(
            client.url,
            bundle,
            template,
            label=f"{RUN_LABEL}-x402-first",
            proof_system="groth16",
            winner_mode=0,
            solver_reward=250_000,
            keeper_reward=12_500,
            proof_window=7_200,
            funding_window=PREPARED_FUNDING_WINDOW,
        )
        x402_nonce = rehearsal.b32(
            rehearsal.hash_label(f"{bundle['source_commit']}:{RUN_LABEL}-x402-first")
        )
        x402_address, x402_id = rehearsal.predict(
            client.url, factory, signer.address, x402_params, x402_nonce
        )
        bound_fixture = rehearsal.fixture_builder.bind(
            template,
            rehearsal.scope(
                bundle,
                x402_params,
                x402_address,
                x402_id,
                solver_a.address,
                solver_nonce,
                "groth16",
            ),
        )
        receipts["approve_x402_canary"] = client.send(
            signer,
            to=token,
            data=rehearsal.function_data(
                "approve(address,uint256)",
                ["address", "uint256"],
                [x402_address, 262_500],
            ),
        )
        receipts["create_x402_canary"] = client.send(
            signer,
            to=factory,
            data=rehearsal.function_data(
                f"createCompetition({rehearsal.PARAM_TYPE},uint256,bytes32,bytes32)",
                [rehearsal.PARAM_TYPE, "uint256", "bytes32", "bytes32"],
                [x402_params, 262_500, x402_nonce, risk_hash],
            ),
        )
        x402_result = x402_canary_spec(
            bound_fixture, x402_address, x402_id, solver_a.address, solver_nonce
        )
        x402_result["proof_deadline"] = rehearsal.call_uint(
            client.url, x402_address, "proofDeadline()"
        )
        x402_result["active"] = True

    for name, actor in (("solver_a", solver_a), ("solver_b", solver_b)):
        if args.prepare_x402_canary and name == "solver_a":
            continue
        balance = rehearsal.token_balance(client.url, token, actor.address)
        if balance:
            receipts[f"reclaim_{name}_usdc"] = client.send(actor, to=token, data=rehearsal.function_data("transfer(address,uint256)", ["address", "uint256"], [signer.address, balance]))

    safe = wait_safe(client.url, list(receipts.values()), args.safe_timeout)
    if not args.x402_replaces_first_proven:
        require(rehearsal.token_balance(client.url, token, first_address) == 0, "first-proven escrow retained USDC")
    require(rehearsal.token_balance(client.url, token, best_address) == 0, "best-score escrow retained USDC")

    result = {
        "schema_version": f"agent-bounties/open-competition-v2-beta2-{RUN_LABEL}-rehearsal-v1",
        "passed": True,
        "synthetic": True,
        "source_commit": bundle["source_commit"],
        "source_tree_hash": bundle["source_tree_hash"],
        "release_bundle_hash": keccak256(json.dumps(bundle, sort_keys=True, separators=(",", ":")).encode()),
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "safe_block": safe,
        "components": components,
        "actors": actors,
        "actor_derivation_id": actor_derivation_id(
            bundle["source_commit"], args.actor_derivation_salt
        ),
        "transactions": {name: {"transaction_hash": receipt_hash(value), "block_number": int(value["blockNumber"], 16), "block_hash": value["blockHash"].lower()} for name, value in receipts.items()},
        "proofs": {name: proof_summary(value) for name, value in proofs.items()},
        "groth16_first_proven": (
            {
                "competition": x402_result["competition"],
                "bounty_id": x402_result["bounty_id"],
                "pooled_funding": False,
                "settled": False,
                "settlement_deferred_to_x402": True,
            }
            if args.x402_replaces_first_proven
            else {"competition": first_address, "bounty_id": first_id, "pooled_funding": True, "settled": True}
        ),
        "plonk_best_score": {"competition": best_address, "bounty_id": best_id, "entries": 2, "winner": solver_b.address.lower(), "settled": True},
        "byo_proof_submission": {
            "proof_system": "plonk",
            "transaction_hash": receipt_hash(receipts["submit_plonk_best_a"]),
            "canonical_safe_block": safe["number"],
        },
        "x402_canary": x402_result,
        "expiry_refund": expiry_result,
        "test_usdc_reclaimed": not args.prepare_x402_canary,
        "solver_test_usdc_retained_for_x402": args.prepare_x402_canary,
        "evidence_boundary": f"Live {NETWORK} technical rehearsal. Synthetic entries are excluded from adoption and external earning metrics.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return result


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--network", choices=("base-sepolia", "base-mainnet"), default="base-sepolia")
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--rpc-url")
    parser.add_argument("--private-key-env", default="BASE_KEEPER_PRIVATE_KEY")
    parser.add_argument("--actor-derivation-salt", default="local")
    parser.add_argument("--actor-eth-wei", type=int, default=100_000_000_000_000)
    parser.add_argument("--safe-timeout", type=int, default=1_800)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--prepare-proof-fixtures", type=Path)
    parser.add_argument("--resolved-bundle-output", type=Path)
    parser.add_argument("--resolved-runtime-output", type=Path)
    parser.add_argument("--prepared-proof-dir", type=Path)
    parser.add_argument("--proof-evidence-dir", type=Path)
    parser.add_argument("--skip-expiry-refund", action="store_true")
    parser.add_argument("--prepare-x402-canary", action="store_true")
    parser.add_argument("--x402-replaces-first-proven", action="store_true")
    return parser.parse_args()


def configure_network(args: argparse.Namespace) -> None:
    global CHAIN_ID, NETWORK, RUN_LABEL
    NETWORK = args.network
    if NETWORK == "base-mainnet":
        CHAIN_ID = 8453
        RUN_LABEL = "mainnet"
        args.rpc_url = args.rpc_url or os.environ.get("BASE_MAINNET_RPC_URL", "https://mainnet.base.org")
    else:
        CHAIN_ID = 84532
        RUN_LABEL = "sepolia"
        args.rpc_url = args.rpc_url or os.environ.get("BASE_SEPOLIA_RPC_URL", "https://sepolia.base.org")


def main() -> int:
    args = parse_args()
    configure_network(args)
    if args.x402_replaces_first_proven and not args.prepare_x402_canary:
        raise SystemExit("--x402-replaces-first-proven requires --prepare-x402-canary")
    if bool(args.prepared_proof_dir) != bool(args.proof_evidence_dir):
        raise SystemExit("--prepared-proof-dir and --proof-evidence-dir must be supplied together")
    if args.prepare_proof_fixtures:
        if not args.resolved_bundle_output:
            raise SystemExit("--prepare-proof-fixtures requires --resolved-bundle-output")
        if args.prepared_proof_dir:
            raise SystemExit("preparation and proof execution are mutually exclusive")
        result = prepare(args)
        summary = {
            "output": str(args.output),
            "passed": result["passed"],
            "factory": result["components"]["factory"]["address"],
            "proof_context_hash": result["proof_context_hash"],
        }
    else:
        result = run(args)
        summary = {
            "output": str(args.output),
            "passed": result["passed"],
            "factory": result["components"]["factory"]["address"],
            "safe_block": result["safe_block"]["number"],
        }
    print(json.dumps(summary))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
