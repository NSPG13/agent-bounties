#!/usr/bin/env python3
"""Run V2 proof, settlement, pooled-funding, expiry, and refund paths on an Anvil fork."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shlex
import subprocess
import time
from typing import Any

from eth_abi import encode

import build_open_competition_v2_beta1_release as release
import prepare_open_competition_v2_metric_fixture as fixture_builder
from _shared.evm import keccak256, keccak_bytes
from _shared.rpc import rpc


ROOT = Path(__file__).resolve().parents[1]
PROGRAM_ROOT = ROOT / "programs/public-vector-metric-v1"
FUNDING_SOURCE = "0x1eaa1c68772cf76bc5f4e4174766076e33ace662"
CREATOR = "0x1000000000000000000000000000000000000001"
POOL_FUNDER = "0x2000000000000000000000000000000000000002"
SOLVER_A = "0x3000000000000000000000000000000000000003"
SOLVER_B = "0x4000000000000000000000000000000000000004"
KEEPER = "0x5000000000000000000000000000000000000005"
REFUND_HELPER = "0x6000000000000000000000000000000000000006"
PARAM_TYPE = "(uint256,uint256,uint64,uint64,uint8,uint8,int256,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32)"
PARAM_TYPES = (
    "uint256",
    "uint256",
    "uint64",
    "uint64",
    "uint8",
    "uint8",
    "int256",
    "bytes32",
    "bytes32",
    "bytes32",
    "bytes32",
    "bytes32",
    "bytes32",
    "bytes32",
    "bytes32",
    "bytes32",
    "bytes32",
)
SETTLED_TOPIC = keccak256(
    b"CompetitionSettledV2(bytes32,uint256,address,uint256,address,uint256,bytes32,bytes32,int256,bytes32)"
)


def b32(value: str) -> bytes:
    raw = bytes.fromhex(value.removeprefix("0x"))
    if len(raw) != 32:
        raise ValueError("bytes32 value required")
    return raw


def hash_label(value: str) -> str:
    return keccak256(value.encode())


def function_data(signature: str, types: list[str], values: list[Any]) -> str:
    return "0x" + (keccak_bytes(signature.encode())[:4] + encode(types, values)).hex()


def call(url: str, to: str, signature: str, types: list[str] | None = None, values: list[Any] | None = None) -> str:
    data = function_data(signature, types or [], values or [])
    return rpc(url, "eth_call", [{"to": to, "data": data}, "latest"])


def call_uint(url: str, to: str, signature: str, types: list[str] | None = None, values: list[Any] | None = None) -> int:
    return int(call(url, to, signature, types, values), 16)


def call_address(url: str, to: str, signature: str) -> str:
    return "0x" + bytes.fromhex(call(url, to, signature)[2:])[-20:].hex()


def send(url: str, sender: str, to: str, data: str) -> dict[str, Any]:
    rpc(url, "anvil_setBalance", [sender, hex(10**20)])
    rpc(url, "anvil_impersonateAccount", [sender])
    transaction = {"from": sender, "to": to, "data": data, "value": "0x0"}
    try:
        gas = int(rpc(url, "eth_estimateGas", [transaction]), 16)
        transaction["gas"] = hex(gas * 5 // 4 + 50_000)
        transaction_hash = rpc(url, "eth_sendTransaction", [transaction])
        for _ in range(120):
            receipt = rpc(url, "eth_getTransactionReceipt", [transaction_hash])
            if receipt:
                if int(receipt["status"], 16) != 1:
                    raise RuntimeError(f"transaction reverted: {transaction_hash}")
                return receipt
            time.sleep(0.1)
        raise RuntimeError(f"receipt timed out: {transaction_hash}")
    finally:
        rpc(url, "anvil_stopImpersonatingAccount", [sender])


def token_balance(url: str, token: str, account: str) -> int:
    return call_uint(url, token, "balanceOf(address)", ["address"], [account])


def transfer(url: str, token: str, sender: str, recipient: str, amount: int) -> dict[str, Any]:
    return send(url, sender, token, function_data("transfer(address,uint256)", ["address", "uint256"], [recipient, amount]))


def approve(url: str, token: str, owner: str, spender: str, amount: int) -> dict[str, Any]:
    return send(url, owner, token, function_data("approve(address,uint256)", ["address", "uint256"], [spender, amount]))


def latest_timestamp(url: str) -> int:
    return int(rpc(url, "eth_getBlockByNumber", ["latest", False])["timestamp"], 16)


def params(
    url: str, bundle: dict[str, Any], template: dict[str, Any], *, label: str, proof_system: str,
    winner_mode: int, solver_reward: int, keeper_reward: int, proof_window: int,
    funding_window: int = 3_600,
) -> tuple[Any, ...]:
    verification_policy = fixture_builder.verification_policy_hash(
        template["mode"], int(template["threshold"]), template["vectors"]
    )
    profile = bundle["metric_profile"]
    return (
        solver_reward,
        keeper_reward,
        latest_timestamp(url) + funding_window,
        proof_window,
        winner_mode,
        0,
        int(template["threshold"]),
        b32(fixture_builder.PROOF_SYSTEMS[proof_system]),
        b32(profile["program_vkey"]),
        b32(profile["source_hash"]),
        b32(profile["elf_hash"]),
        b32(profile["journal_schema_hash"]),
        b32(profile["metric_program_hash"]),
        b32(hash_label(f"agent-bounties/open-competition-v2/rehearsal/{label}/execution")),
        b32(verification_policy),
        b32(hash_label(f"agent-bounties/open-competition-v2/rehearsal/{label}/settlement")),
        b32(bundle["risk"]["hash"]),
    )


def predict(
    url: str, factory: str, creator: str, competition_params: tuple[Any, ...], creation_nonce: bytes
) -> tuple[str, str]:
    types = ["address", PARAM_TYPE, "bytes32"]
    values = [creator, competition_params, creation_nonce]
    bounty_id = call(url, factory, f"bountyIdFor(address,{PARAM_TYPE},bytes32)", types, values)
    competition = "0x" + bytes.fromhex(
        call(url, factory, f"predictCompetitionAddress(address,{PARAM_TYPE},bytes32)", types, values)[2:]
    )[-20:].hex()
    return competition, bounty_id


def create(
    url: str, factory: str, creator: str, competition_params: tuple[Any, ...], initial_funding: int,
    creation_nonce: bytes, risk_hash: bytes,
) -> dict[str, Any]:
    return send(
        url,
        creator,
        factory,
        function_data(
            f"createCompetition({PARAM_TYPE},uint256,bytes32,bytes32)",
            [PARAM_TYPE, "uint256", "bytes32", "bytes32"],
            [competition_params, initial_funding, creation_nonce, risk_hash],
        ),
    )


def scope(
    bundle: dict[str, Any], competition_params: tuple[Any, ...], competition: str, bounty_id: str,
    solver: str, solver_nonce: int, proof_system: str,
) -> dict[str, Any]:
    profile = bundle["metric_profile"]
    return {
        "chain_id": bundle["chain_id"],
        "competition": competition,
        "bounty_id": bounty_id,
        "solver": solver,
        "solver_nonce": solver_nonce,
        "proof_system": proof_system,
        "program_vkey": profile["program_vkey"],
        "source_hash": profile["source_hash"],
        "elf_hash": profile["elf_hash"],
        "execution_policy_hash": "0x" + competition_params[13].hex(),
        "settlement_policy_hash": "0x" + competition_params[15].hex(),
        "beta_risk_hash": bundle["risk"]["hash"],
    }


def prover_command(fixture_path: Path, mode: str) -> list[str]:
    fixture_path = fixture_path.resolve()
    if os.name != "nt":
        return [
            "cargo", "run", "--locked", "--release", "-p", "public-vector-metric-v1-script",
            "--", str(fixture_path), mode,
        ]
    def wsl_path(path: Path) -> str:
        resolved = path.resolve()
        drive = resolved.drive.removesuffix(":").lower()
        if len(drive) != 1 or not drive.isalpha():
            raise RuntimeError(f"cannot map Windows path into WSL: {resolved}")
        relative = resolved.relative_to(resolved.anchor).as_posix()
        return f"/mnt/{drive}/{relative}"

    fixture_wsl = shlex.quote(wsl_path(fixture_path))
    program_wsl = shlex.quote(wsl_path(PROGRAM_ROOT))
    command = (
        "set -euo pipefail; "
        'export PATH="$HOME/.sp1/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin"; '
        "export CARGO_TARGET_DIR=/mnt/d/agent-bounties-cache/sp1-public-vector-target; "
        f"cd {program_wsl}; "
        f"cargo run --locked --release -p public-vector-metric-v1-script -- {fixture_wsl} {shlex.quote(mode)}"
    )
    return ["wsl.exe", "bash", "-lc", command]


def prove(bound_fixture: dict[str, Any], mode: str, label: str, work: Path) -> dict[str, Any]:
    path = work / f"{label}.json"
    path.write_text(json.dumps(bound_fixture, indent=2) + "\n", encoding="utf-8")
    started = time.monotonic()
    completed = subprocess.run(
        prover_command(path, mode),
        cwd=PROGRAM_ROOT if os.name != "nt" else ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    elapsed = time.monotonic() - started
    log_path = work / f"{label}.prover.log"
    log_path.write_text(
        completed.stdout + "\n--- STDERR ---\n" + completed.stderr,
        encoding="utf-8",
    )
    if completed.returncode != 0:
        tail = (completed.stderr or completed.stdout).splitlines()[-20:]
        raise RuntimeError(
            f"{mode} prover exited {completed.returncode}; log={log_path}; tail={' | '.join(tail)}"
        )
    evidence = None
    for line in reversed(completed.stdout.splitlines()):
        try:
            candidate = json.loads(line)
        except json.JSONDecodeError:
            continue
        if candidate.get("mode") == mode:
            evidence = candidate
            break
    if not evidence:
        raise RuntimeError(f"{mode} prover did not return evidence JSON")
    evidence["elapsed_seconds"] = round(elapsed, 3)
    evidence["stderr_tail"] = completed.stderr.splitlines()[-8:]
    return evidence


def has_topic(receipt: dict[str, Any], topic: str) -> bool:
    return any(log.get("topics", [None])[0] == topic for log in receipt.get("logs", []))


def run(url: str, bundle: dict[str, Any], work: Path) -> dict[str, Any]:
    work.mkdir(parents=True, exist_ok=True)
    token = bundle["settlement_token"]
    factory = bundle["factory"]["address"]
    risk_hash = b32(bundle["risk"]["hash"])
    templates = {
        "first": json.loads((PROGRAM_ROOT / "fixtures/rehearsal-first-proven.json").read_text()),
        "best_a": json.loads((PROGRAM_ROOT / "fixtures/rehearsal-best-score-a.json").read_text()),
        "best_b": json.loads((PROGRAM_ROOT / "fixtures/rehearsal-best-score-b.json").read_text()),
    }
    if fixture_builder.verification_policy_hash(
        templates["best_a"]["mode"], templates["best_a"]["threshold"], templates["best_a"]["vectors"]
    ) != fixture_builder.verification_policy_hash(
        templates["best_b"]["mode"], templates["best_b"]["threshold"], templates["best_b"]["vectors"]
    ):
        raise RuntimeError("best-score entrants do not share an immutable verification policy")

    first_params = params(
        url, bundle, templates["first"], label="groth16-first", proof_system="groth16",
        winner_mode=0, solver_reward=250_000, keeper_reward=12_500, proof_window=3_600,
    )
    first_nonce = b32(hash_label("agent-bounties/open-competition-v2/rehearsal/groth16-first"))
    first_address, first_id = predict(url, factory, CREATOR, first_params, first_nonce)
    first_fixture = fixture_builder.bind(
        templates["first"], scope(bundle, first_params, first_address, first_id, SOLVER_A, 1, "groth16")
    )

    best_params = params(
        url, bundle, templates["best_a"], label="plonk-best", proof_system="plonk",
        winner_mode=1, solver_reward=250_000, keeper_reward=12_500, proof_window=3_600,
    )
    best_nonce = b32(hash_label("agent-bounties/open-competition-v2/rehearsal/plonk-best"))
    best_address, best_id = predict(url, factory, CREATOR, best_params, best_nonce)
    best_a_fixture = fixture_builder.bind(
        templates["best_a"], scope(bundle, best_params, best_address, best_id, SOLVER_A, 2, "plonk")
    )
    best_b_fixture = fixture_builder.bind(
        templates["best_b"], scope(bundle, best_params, best_address, best_id, SOLVER_B, 1, "plonk")
    )

    proofs = {
        "groth16_first": prove(first_fixture, "groth16", "groth16-first", work),
        "plonk_best_a": prove(best_a_fixture, "plonk", "plonk-best-a", work),
        "plonk_best_b": prove(best_b_fixture, "plonk", "plonk-best-b", work),
    }

    transfer(url, token, FUNDING_SOURCE, CREATOR, 467_500)
    transfer(url, token, FUNDING_SOURCE, POOL_FUNDER, 162_500)
    approve(url, token, CREATOR, first_address, 100_000)
    create(url, factory, CREATOR, first_params, 100_000, first_nonce, risk_hash)
    approve(url, token, POOL_FUNDER, first_address, 162_500)
    send(
        url, POOL_FUNDER, first_address,
        function_data("fund(uint256,bytes32)", ["uint256", "bytes32"], [162_500, risk_hash]),
    )
    first_before = token_balance(url, token, SOLVER_A)
    first_receipt = send(
        url, SOLVER_A, first_address,
        function_data(
            "submitProof(bytes,bytes)", ["bytes", "bytes"],
            [bytes.fromhex(proofs["groth16_first"]["journal_hex"][2:]), bytes.fromhex(proofs["groth16_first"]["proof_hex"][2:])],
        ),
    )
    if not has_topic(first_receipt, SETTLED_TOPIC):
        raise RuntimeError("Groth16 first-proven transaction emitted no CompetitionSettledV2")
    if token_balance(url, token, SOLVER_A) - first_before != 262_500:
        raise RuntimeError("Groth16 solver plus submitter payout mismatch")
    if token_balance(url, token, first_address) != 0:
        raise RuntimeError("Groth16 competition retained USDC")

    approve(url, token, CREATOR, best_address, 262_500)
    create(url, factory, CREATOR, best_params, 262_500, best_nonce, risk_hash)
    for solver, proof_name in ((SOLVER_A, "plonk_best_a"), (SOLVER_B, "plonk_best_b")):
        evidence = proofs[proof_name]
        send(
            url, solver, best_address,
            function_data(
                "submitProof(bytes,bytes)", ["bytes", "bytes"],
                [bytes.fromhex(evidence["journal_hex"][2:]), bytes.fromhex(evidence["proof_hex"][2:])],
            ),
        )
    if call_address(url, best_address, "leader()") != SOLVER_B:
        raise RuntimeError("strictly better PLONK entry did not become leader")
    best_deadline = call_uint(url, best_address, "proofDeadline()")
    rpc(url, "evm_setNextBlockTimestamp", [best_deadline + 1])
    rpc(url, "evm_mine", [])
    best_solver_before = token_balance(url, token, SOLVER_B)
    keeper_before = token_balance(url, token, KEEPER)
    best_receipt = send(url, KEEPER, best_address, function_data("finalizeBestScore()", [], []))
    if not has_topic(best_receipt, SETTLED_TOPIC):
        raise RuntimeError("PLONK best-score finalization emitted no CompetitionSettledV2")
    if token_balance(url, token, SOLVER_B) - best_solver_before != 250_000:
        raise RuntimeError("PLONK best-score solver payout mismatch")
    if token_balance(url, token, KEEPER) - keeper_before != 12_500:
        raise RuntimeError("PLONK finalizer reward mismatch")

    expiry_template = templates["first"]
    expiry_params = params(
        url, bundle, expiry_template, label="expiry", proof_system="groth16",
        winner_mode=0, solver_reward=100_000, keeper_reward=5_000, proof_window=1,
    )
    expiry_nonce = b32(hash_label("agent-bounties/open-competition-v2/rehearsal/expiry"))
    expiry_address, _ = predict(url, factory, CREATOR, expiry_params, expiry_nonce)
    approve(url, token, CREATOR, expiry_address, 105_000)
    create(url, factory, CREATOR, expiry_params, 105_000, expiry_nonce, risk_hash)
    expiry_deadline = call_uint(url, expiry_address, "proofDeadline()")
    rpc(url, "evm_setNextBlockTimestamp", [expiry_deadline + 1])
    rpc(url, "evm_mine", [])
    expiry_keeper_before = token_balance(url, token, KEEPER)
    send(url, KEEPER, expiry_address, function_data("expireCompetition()", [], []))
    if token_balance(url, token, KEEPER) - expiry_keeper_before != 5_000:
        raise RuntimeError("expiry caller reward mismatch")
    creator_before_refund = token_balance(url, token, CREATOR)
    send(
        url, REFUND_HELPER, expiry_address,
        function_data("withdrawRefundFor(address)", ["address"], [CREATOR]),
    )
    if token_balance(url, token, CREATOR) - creator_before_refund != 100_000:
        raise RuntimeError("permissionless contributor refund mismatch")
    if token_balance(url, token, expiry_address) != 0:
        raise RuntimeError("expired competition retained USDC")

    return {
        "passed": True,
        "proofs": proofs,
        "groth16_first_proven": {"competition": first_address, "bounty_id": first_id, "settled": True, "pooled_funding": True},
        "plonk_best_score": {"competition": best_address, "bounty_id": best_id, "entries": 2, "winner": SOLVER_B, "settled": True},
        "expiry_refund": {"competition": expiry_address, "keeper_paid": 5_000, "creator_refunded": 100_000, "third_party_withdrawal": True},
        "evidence_boundary": "Fork-only proof and accounting rehearsal. No live funds moved and no adoption metric may count these synthetic entries.",
    }
