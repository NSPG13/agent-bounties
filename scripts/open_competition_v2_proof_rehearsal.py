#!/usr/bin/env python3
"""Run V2 proof, settlement, pooled-funding, expiry, and refund paths on an Anvil fork."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
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
PREPARED_SCHEMA = "agent-bounties/open-competition-v2-beta1-proof-context-v1"
JOURNAL_DOMAIN = keccak_bytes(b"agent-bounties/open-competition-v2-beta1/journal")
SUBMISSION_DOMAIN = bytes.fromhex("402204460b00978c26cee42ae0089d94fe8b0b17bd90c45a6cd78d466463a507")
EVIDENCE_DOMAIN = bytes.fromhex("16f60f26d350a38e6993a5454967d1efb0461d93785b7cdb38ba463284c5ab15")
PROOF_SPECS = {
    "groth16_first": ("groth16", "groth16-first"),
    "plonk_best_a": ("plonk", "plonk-best-a"),
    "plonk_best_b": ("plonk", "plonk-best-b"),
}


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
    work.mkdir(parents=True, exist_ok=True)
    path = work / f"{label}.json"
    path.write_text(json.dumps(bound_fixture, indent=2) + "\n", encoding="utf-8")
    started = time.monotonic()
    process = subprocess.Popen(
        prover_command(path, mode),
        cwd=PROGRAM_ROOT if os.name != "nt" else ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        bufsize=1,
    )
    log_path = work / f"{label}.prover.log"
    evidence = None
    tail: list[str] = []
    with log_path.open("w", encoding="utf-8") as log:
        assert process.stdout is not None
        for line in process.stdout:
            stripped = line.rstrip("\r\n")
            candidate = None
            try:
                candidate = json.loads(stripped)
            except json.JSONDecodeError:
                pass
            if isinstance(candidate, dict) and candidate.get("mode") == mode:
                evidence = candidate
                summary = json.dumps({
                    "mode": mode,
                    "program_vkey": candidate.get("program_vkey"),
                    "elf_keccak256": candidate.get("elf_keccak256"),
                    "proof_bytes": len(bytes.fromhex(str(candidate.get("proof_hex", "0x")).removeprefix("0x"))),
                    "journal_bytes": len(bytes.fromhex(str(candidate.get("journal_hex", "0x")).removeprefix("0x"))),
                })
                log.write(summary + "\n")
                print(summary, flush=True)
            else:
                log.write(line)
                print(stripped, flush=True)
            tail.append(stripped)
            tail = tail[-20:]
    returncode = process.wait()
    elapsed = time.monotonic() - started
    if returncode != 0:
        raise RuntimeError(
            f"{mode} prover exited {returncode}; log={log_path}; tail={' | '.join(tail)}"
        )
    if not evidence:
        raise RuntimeError(f"{mode} prover did not return evidence JSON")
    evidence["elapsed_seconds"] = round(elapsed, 3)
    evidence["log_tail"] = tail[-8:]
    return evidence


def canonical_hash(value: Any) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return "0x" + hashlib.sha256(encoded).hexdigest()


def params_json(value: tuple[Any, ...]) -> list[int | str]:
    return ["0x" + item.hex() if isinstance(item, bytes) else int(item) for item in value]


def params_tuple(value: list[int | str]) -> tuple[Any, ...]:
    if len(value) != len(PARAM_TYPES):
        raise ValueError("prepared competition parameter count changed")
    result: list[Any] = []
    for kind, item in zip(PARAM_TYPES, value, strict=True):
        if kind == "bytes32":
            if not isinstance(item, str) or not re.fullmatch(r"0x[0-9a-f]{64}", item):
                raise ValueError("prepared bytes32 parameter is not canonical hex")
            result.append(b32(str(item)))
        else:
            if type(item) is not int:
                raise ValueError("prepared numeric parameter is not an integer")
            result.append(int(item))
    return tuple(result)


def word(value: bytes) -> bytes:
    if len(value) > 32:
        raise ValueError("journal word exceeds 32 bytes")
    return value.rjust(32, b"\0")


def signed_word(value: int) -> bytes:
    return value.to_bytes(32, "big", signed=True)


def evaluate_fixture(fixture: dict[str, Any]) -> tuple[bool, int]:
    mode = fixture["mode"]
    threshold = int(fixture["threshold"])
    vectors = fixture["vectors"]
    if mode in ("all_equal", "maximize_exact_matches"):
        score = sum(int(item["weight"]) for item in vectors if int(item["observed"]) == int(item["expected"]))
        total = sum(int(item["weight"]) for item in vectors)
        passed = score == total if mode == "all_equal" else score >= threshold
    elif mode == "minimize_absolute_error":
        score = sum(
            abs(int(item["expected"]) - int(item["observed"])) * int(item["weight"])
            for item in vectors
        )
        passed = score <= threshold
    else:
        raise ValueError("prepared fixture uses an unsupported metric mode")
    return passed, score


def expected_journal(fixture: dict[str, Any]) -> bytes:
    scope_value = fixture["scope"]
    vectors = fixture["vectors"]
    policy = b32(
        fixture_builder.verification_policy_hash(fixture["mode"], int(fixture["threshold"]), vectors)
    )
    submission_payload = bytearray(SUBMISSION_DOMAIN)
    submission_payload.extend(len(vectors).to_bytes(4, "big"))
    for item in vectors:
        submission_payload.extend(int(item["observed"]).to_bytes(8, "big", signed=True))
    submission = keccak_bytes(bytes(submission_payload))
    evidence = keccak_bytes(EVIDENCE_DOMAIN + policy + submission)
    passed, score = evaluate_fixture(fixture)
    values = (
        JOURNAL_DOMAIN,
        word(int(scope_value["chain_id"]).to_bytes(8, "big")),
        word(bytes(scope_value["competition"])),
        bytes(scope_value["bounty_id"]),
        word(bytes(scope_value["solver"])),
        word(int(scope_value["solver_nonce"]).to_bytes(16, "big")),
        submission,
        evidence,
        bytes(scope_value["proof_system"]),
        bytes(scope_value["program_vkey"]),
        bytes(scope_value["source_hash"]),
        bytes(scope_value["elf_hash"]),
        b32(release.JOURNAL_SCHEMA_HASH),
        b32(release.METRIC_PROGRAM_HASH),
        bytes(scope_value["execution_policy_hash"]),
        policy,
        bytes(scope_value["settlement_policy_hash"]),
        bytes(scope_value["beta_risk_hash"]),
        word(bytes([int(passed)])),
        signed_word(score),
    )
    if any(len(item) != 32 for item in values):
        raise ValueError("prepared fixture produced a non-word journal field")
    return b"".join(values)


def write_fixture(work: Path, label: str, fixture: dict[str, Any]) -> dict[str, Any]:
    filename = f"{label}.json"
    encoded = (json.dumps(fixture, indent=2) + "\n").encode()
    (work / filename).write_bytes(encoded)
    passed, score = evaluate_fixture(fixture)
    return {
        "fixture": filename,
        "fixture_sha256": hashlib.sha256(encoded).hexdigest(),
        "journal_sha256": hashlib.sha256(expected_journal(fixture)).hexdigest(),
        "expected_pass": passed,
        "expected_score": score,
    }


def prepare_context(
    url: str,
    bundle: dict[str, Any],
    work: Path,
    *,
    creator: str = CREATOR,
    solver_a: str = SOLVER_A,
    solver_b: str = SOLVER_B,
    first_label: str = "groth16-first",
    best_label: str = "plonk-best",
    first_nonce_label: str = "agent-bounties/open-competition-v2/rehearsal/groth16-first",
    best_nonce_label: str = "agent-bounties/open-competition-v2/rehearsal/plonk-best",
    proof_window: int = 3_600,
    funding_window: int = 86_400,
) -> dict[str, Any]:
    work.mkdir(parents=True, exist_ok=True)
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

    factory = bundle["factory"]["address"]
    first_params = params(
        url, bundle, templates["first"], label=first_label, proof_system="groth16",
        winner_mode=0, solver_reward=250_000, keeper_reward=12_500,
        proof_window=proof_window, funding_window=funding_window,
    )
    first_nonce = b32(hash_label(first_nonce_label))
    first_address, first_id = predict(url, factory, creator, first_params, first_nonce)
    first_fixture = fixture_builder.bind(
        templates["first"], scope(bundle, first_params, first_address, first_id, solver_a, 1, "groth16")
    )

    best_params = params(
        url, bundle, templates["best_a"], label=best_label, proof_system="plonk",
        winner_mode=1, solver_reward=250_000, keeper_reward=12_500,
        proof_window=proof_window, funding_window=funding_window,
    )
    best_nonce = b32(hash_label(best_nonce_label))
    best_address, best_id = predict(url, factory, creator, best_params, best_nonce)
    best_a_fixture = fixture_builder.bind(
        templates["best_a"], scope(bundle, best_params, best_address, best_id, solver_a, 2, "plonk")
    )
    best_b_fixture = fixture_builder.bind(
        templates["best_b"], scope(bundle, best_params, best_address, best_id, solver_b, 1, "plonk")
    )

    proof_context = {}
    for name, fixture in {
        "groth16_first": first_fixture,
        "plonk_best_a": best_a_fixture,
        "plonk_best_b": best_b_fixture,
    }.items():
        mode, label = PROOF_SPECS[name]
        proof_context[name] = {"mode": mode, "label": label, **write_fixture(work, label, fixture)}
    context = {
        "schema_version": PREPARED_SCHEMA,
        "release": {
            "chain_id": bundle["chain_id"],
            "source_commit": bundle["source_commit"],
            "source_tree_hash": bundle["source_tree_hash"],
            "factory": factory,
            "program_vkey": bundle["metric_profile"]["program_vkey"],
            "elf_hash": bundle["metric_profile"]["elf_hash"],
            "elf_sha256": bundle["metric_profile"]["elf_sha256"],
        },
        "actors": {"creator": creator.lower(), "solver_a": solver_a.lower(), "solver_b": solver_b.lower()},
        "first": {
            "params": params_json(first_params), "nonce": "0x" + first_nonce.hex(),
            "address": first_address, "bounty_id": first_id,
        },
        "best": {
            "params": params_json(best_params), "nonce": "0x" + best_nonce.hex(),
            "address": best_address, "bounty_id": best_id,
        },
        "proofs": proof_context,
    }
    context["context_hash"] = canonical_hash(context)
    (work / "context.json").write_text(json.dumps(context, indent=2) + "\n", encoding="utf-8")
    return context


def load_context(bundle: dict[str, Any], work: Path) -> tuple[dict[str, Any], dict[str, dict[str, Any]]]:
    context = json.loads((work / "context.json").read_text(encoding="utf-8"))
    if context.get("schema_version") != PREPARED_SCHEMA:
        raise ValueError("prepared proof context schema mismatch")
    claimed_hash = context.pop("context_hash", None)
    observed_hash = canonical_hash(context)
    context["context_hash"] = claimed_hash
    if claimed_hash != observed_hash:
        raise ValueError("prepared proof context hash mismatch")
    release_context = context["release"]
    expected_release = {
        "chain_id": bundle["chain_id"],
        "source_commit": bundle["source_commit"],
        "source_tree_hash": bundle["source_tree_hash"],
        "factory": bundle["factory"]["address"],
        "program_vkey": bundle["metric_profile"]["program_vkey"],
        "elf_hash": bundle["metric_profile"]["elf_hash"],
        "elf_sha256": bundle["metric_profile"]["elf_sha256"],
    }
    if release_context != expected_release:
        raise ValueError("prepared proof context does not match the exact release bundle")
    fixtures: dict[str, dict[str, Any]] = {}
    if set(context.get("proofs", {})) != set(PROOF_SPECS):
        raise ValueError("prepared proof inventory mismatch")
    for name, (mode, label) in PROOF_SPECS.items():
        item = context["proofs"][name]
        if item.get("mode") != mode or item.get("label") != label or item.get("fixture") != f"{label}.json":
            raise ValueError(f"prepared proof identity mismatch: {name}")
        path = work / item["fixture"]
        raw = path.read_bytes()
        if hashlib.sha256(raw).hexdigest() != item["fixture_sha256"]:
            raise ValueError(f"prepared fixture hash mismatch: {name}")
        fixture = json.loads(raw)
        if hashlib.sha256(expected_journal(fixture)).hexdigest() != item["journal_sha256"]:
            raise ValueError(f"prepared journal hash mismatch: {name}")
        passed, score = evaluate_fixture(fixture)
        if passed != item["expected_pass"] or score != item["expected_score"]:
            raise ValueError(f"prepared metric expectation mismatch: {name}")
        fixtures[name] = fixture
    return context, fixtures


def validate_proof_evidence(
    bundle: dict[str, Any], context: dict[str, Any], name: str,
    fixture: dict[str, Any], evidence: dict[str, Any],
) -> dict[str, Any]:
    mode, _ = PROOF_SPECS[name]
    profile = bundle["metric_profile"]
    if evidence.get("mode") != mode:
        raise ValueError(f"proof mode mismatch: {name}")
    if evidence.get("program_vkey") != profile["program_vkey"]:
        raise ValueError(f"proof vkey mismatch: {name}")
    if evidence.get("elf_keccak256") != profile["elf_hash"]:
        raise ValueError(f"proof ELF Keccak mismatch: {name}")
    if evidence.get("elf_sha256") != profile["elf_sha256"]:
        raise ValueError(f"proof ELF SHA-256 mismatch: {name}")
    proof_hex = evidence.get("proof_hex")
    journal_hex = evidence.get("journal_hex")
    if not isinstance(proof_hex, str) or not re.fullmatch(r"0x(?:[0-9a-f]{2})+", proof_hex):
        raise ValueError(f"proof evidence is not canonical hex: {name}")
    if not isinstance(journal_hex, str) or not re.fullmatch(r"0x(?:[0-9a-f]{2})+", journal_hex):
        raise ValueError(f"proof journal is not canonical hex: {name}")
    try:
        proof = bytes.fromhex(proof_hex[2:])
        journal = bytes.fromhex(journal_hex[2:])
    except ValueError as error:
        raise ValueError(f"proof evidence is not valid hex: {name}") from error
    if not proof:
        raise ValueError(f"proof evidence is empty: {name}")
    if journal != expected_journal(fixture):
        raise ValueError(f"proof journal differs from the prepared fixture: {name}")
    if hashlib.sha256(journal).hexdigest() != context["proofs"][name]["journal_sha256"]:
        raise ValueError(f"proof journal digest differs from prepared context: {name}")
    return evidence


def load_proofs(
    bundle: dict[str, Any], context: dict[str, Any], fixtures: dict[str, dict[str, Any]], work: Path,
) -> dict[str, dict[str, Any]]:
    result = {}
    for name in PROOF_SPECS:
        evidence = json.loads((work / f"{name}.json").read_text(encoding="utf-8"))
        result[name] = validate_proof_evidence(bundle, context, name, fixtures[name], evidence)
    return result


def generate_proof(
    bundle: dict[str, Any], prepared: Path, name: str, output: Path, log_work: Path,
) -> dict[str, Any]:
    if name not in PROOF_SPECS:
        raise ValueError("unknown proof name")
    context, fixtures = load_context(bundle, prepared)
    mode, label = PROOF_SPECS[name]
    evidence = prove(fixtures[name], mode, label, log_work)
    validate_proof_evidence(bundle, context, name, fixtures[name], evidence)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    return evidence


def has_topic(receipt: dict[str, Any], topic: str) -> bool:
    return any(log.get("topics", [None])[0] == topic for log in receipt.get("logs", []))


def run(
    url: str, bundle: dict[str, Any], work: Path, *,
    prepared_dir: Path | None = None, proof_evidence_dir: Path | None = None,
) -> dict[str, Any]:
    work.mkdir(parents=True, exist_ok=True)
    prepared = prepared_dir or (work / "prepared")
    if prepared_dir is None:
        prepare_context(url, bundle, prepared)
    context, fixtures = load_context(bundle, prepared)
    if context["actors"] != {
        "creator": CREATOR.lower(), "solver_a": SOLVER_A.lower(), "solver_b": SOLVER_B.lower()
    }:
        raise ValueError("fork proof context actor set changed")
    if proof_evidence_dir is None:
        evidence_dir = work / "proof-evidence"
        evidence_dir.mkdir(parents=True, exist_ok=True)
        proofs = {}
        for name, (mode, label) in PROOF_SPECS.items():
            evidence = prove(fixtures[name], mode, label, work)
            proofs[name] = validate_proof_evidence(bundle, context, name, fixtures[name], evidence)
            (evidence_dir / f"{name}.json").write_text(
                json.dumps(evidence, indent=2) + "\n", encoding="utf-8"
            )
    else:
        proofs = load_proofs(bundle, context, fixtures, proof_evidence_dir)

    token = bundle["settlement_token"]
    factory = bundle["factory"]["address"]
    risk_hash = b32(bundle["risk"]["hash"])
    first_params = params_tuple(context["first"]["params"])
    first_nonce = b32(context["first"]["nonce"])
    first_address, first_id = predict(url, factory, CREATOR, first_params, first_nonce)
    best_params = params_tuple(context["best"]["params"])
    best_nonce = b32(context["best"]["nonce"])
    best_address, best_id = predict(url, factory, CREATOR, best_params, best_nonce)
    if (first_address, first_id) != (context["first"]["address"], context["first"]["bounty_id"]):
        raise RuntimeError("prepared first-proven competition identity changed")
    if (best_address, best_id) != (context["best"]["address"], context["best"]["bounty_id"]):
        raise RuntimeError("prepared best-score competition identity changed")

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

    expiry_template = json.loads((PROGRAM_ROOT / "fixtures/rehearsal-first-proven.json").read_text())
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
        "prepared_context_hash": context["context_hash"],
        "proofs": {
            name: {
                "mode": value["mode"],
                "proof_hash": keccak256(bytes.fromhex(value["proof_hex"].removeprefix("0x"))),
                "journal_hash": keccak256(bytes.fromhex(value["journal_hex"].removeprefix("0x"))),
                "elapsed_seconds": value.get("elapsed_seconds"),
            }
            for name, value in proofs.items()
        },
        "groth16_first_proven": {"competition": first_address, "bounty_id": first_id, "settled": True, "pooled_funding": True},
        "plonk_best_score": {"competition": best_address, "bounty_id": best_id, "entries": 2, "winner": SOLVER_B, "settled": True},
        "expiry_refund": {"competition": expiry_address, "keeper_paid": 5_000, "creator_refunded": 100_000, "third_party_withdrawal": True},
        "evidence_boundary": "Fork-only proof and accounting rehearsal. No live funds moved and no adoption metric may count these synthetic entries.",
    }
