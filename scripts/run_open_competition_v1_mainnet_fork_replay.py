#!/usr/bin/env python3
"""Replay the exact unsigned mainnet factory deployment and bounded canary on an Anvil fork."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import time
from typing import Any
from urllib.error import URLError
from urllib.request import Request, urlopen

from Crypto.Hash import keccak


ADMIN = "0x884834e884d6e93462655a2820140ad03e6747bc"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
VERIFIER = "0xcc6059ceeda5bc4ba8a97ecfbffa7488c8fd579e"
SOLVER = "0x2000000000000000000000000000000000000002"
SOLVER_GAS_FUNDING_WEI = 100_000_000_000_000
CHAIN_ID = 8_453
COMMITMENT_DOMAIN_TEXT = b"agent-bounties/open-competition-v1-solution"


def keccak_bytes(value: bytes) -> bytes:
    digest = keccak.new(digest_bits=256)
    digest.update(value)
    return digest.digest()


def keccak_hex(value: bytes) -> str:
    return f"0x{keccak_bytes(value).hex()}"


def selector(signature: str) -> bytes:
    return keccak_bytes(signature.encode())[:4]


def word_uint(value: int) -> bytes:
    if value < 0 or value >= 1 << 256:
        raise ValueError("uint256 out of range")
    return value.to_bytes(32, "big")


def word_address(value: str) -> bytes:
    raw = bytes.fromhex(value.removeprefix("0x"))
    if len(raw) != 20:
        raise ValueError(f"invalid address: {value}")
    return raw.rjust(32, b"\0")


def word_bytes32(value: bytes) -> bytes:
    if len(value) != 32:
        raise ValueError("bytes32 value required")
    return value


def calldata(signature: str, *words: bytes) -> str:
    return f"0x{(selector(signature) + b''.join(words)).hex()}"


def rpc(url: str, method: str, params: list[Any]) -> Any:
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    request = Request(
        url,
        data=payload,
        headers={
            "content-type": "application/json",
            "user-agent": "agent-bounties-open-competition-v1-fork-replay/1",
        },
    )
    try:
        with urlopen(request, timeout=30) as response:
            body = json.load(response)
    except (OSError, URLError) as error:
        raise RuntimeError(f"RPC transport failed for {method}: {error}") from error
    if body.get("error"):
        raise RuntimeError(f"RPC {method} failed: {json.dumps(body['error'], sort_keys=True)}")
    return body.get("result")


def receipt(url: str, transaction_hash: str) -> dict[str, Any]:
    for _ in range(120):
        value = rpc(url, "eth_getTransactionReceipt", [transaction_hash])
        if value:
            if int(value["status"], 16) != 1:
                raise RuntimeError(f"transaction reverted: {transaction_hash}")
            return value
        time.sleep(0.25)
    raise RuntimeError(f"receipt timed out: {transaction_hash}")


def send(url: str, sender: str, *, to: str | None = None, data: str | None = None, value: int = 0) -> dict[str, Any]:
    transaction: dict[str, Any] = {
        "from": sender,
        "value": hex(value),
        # Base's pinned block fee is far below Anvil's legacy 1 gwei default.
        # Pinning a conservative 0.01 gwei keeps the fork replay faithful to
        # the real wallet's bounded gas balance while remaining above base fee.
        "gasPrice": hex(10_000_000),
    }
    if to is not None:
        transaction["to"] = to
    if data is not None:
        transaction["data"] = data
    estimated_gas = int(rpc(url, "eth_estimateGas", [transaction]), 16)
    transaction["gas"] = hex(min(15_000_000, estimated_gas * 5 // 4 + 50_000))
    return receipt(url, rpc(url, "eth_sendTransaction", [transaction]))


def call_uint(url: str, to: str, data: str) -> int:
    return int(rpc(url, "eth_call", [{"to": to, "data": data}, "latest"]), 16)


def token_balance(url: str, account: str) -> int:
    return call_uint(url, USDC, calldata("balanceOf(address)", word_address(account)))


def hash_text(value: str) -> bytes:
    return keccak_bytes(value.encode())


def create_competition_data(bundle: dict[str, Any], block_timestamp: int) -> tuple[str, dict[str, Any]]:
    canary = bundle["hidden_canary"]
    terms = hash_text("agent-bounties/open-competition-v1/mainnet-canary/terms-v1")
    policy = hash_text("agent-bounties/open-competition-v1/mainnet-canary/policy-v1")
    criteria = hash_text("leading-zero-work-v1/difficulty-16/canary-criteria-v1")
    benchmark = hash_text("leading-zero-work-v1/difficulty-16/canary-benchmark-v1")
    evidence_schema = hash_text("agent-bounties/leading-zero-work-evidence-v1")
    creation_nonce = hash_text("agent-bounties/open-competition-v1/mainnet-hidden-canary-v1")
    funding_deadline = block_timestamp + 7 * 24 * 60 * 60
    params = [
        word_uint(canary["solver_reward_usdc_base_units"]),
        word_uint(canary["verifier_reward_usdc_base_units"]),
        word_bytes32(terms),
        word_bytes32(policy),
        word_bytes32(criteria),
        word_bytes32(benchmark),
        word_bytes32(evidence_schema),
        word_uint(funding_deadline),
        word_uint(canary["competition_window_seconds"]),
        word_uint(canary["reveal_window_seconds"]),
        word_uint(canary["max_entries"]),
        word_address(VERIFIER),
        word_address(ADMIN),
    ]
    signature = (
        "createCompetition((uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,"
        "uint64,uint64,uint64,uint8,address,address),uint256,bytes32)"
    )
    data = calldata(
        signature,
        *params,
        word_uint(canary["initial_funding_usdc_base_units"]),
        word_bytes32(creation_nonce),
    )
    return data, {
        "terms_hash": f"0x{terms.hex()}",
        "policy_hash": f"0x{policy.hex()}",
        "acceptance_criteria_hash": f"0x{criteria.hex()}",
        "benchmark_hash": f"0x{benchmark.hex()}",
        "evidence_schema_hash": f"0x{evidence_schema.hex()}",
        "creation_nonce": f"0x{creation_nonce.hex()}",
        "funding_deadline": funding_deadline,
    }


def commitment(bounty: str, solver: str, submission: bytes, evidence: bytes, salt: bytes) -> bytes:
    return keccak_bytes(
        b"".join(
            [
                word_bytes32(keccak_bytes(COMMITMENT_DOMAIN_TEXT)),
                word_uint(CHAIN_ID),
                word_address(bounty),
                word_address(solver),
                word_bytes32(submission),
                word_bytes32(evidence),
                word_bytes32(salt),
            ]
        )
    )


def mine_proof(
    bounty_id: bytes, solver: str, submission: bytes, evidence: bytes, policy: bytes, cap: int = 1_000_000
) -> tuple[int, bytes]:
    prefix = b"".join(
        [
            word_bytes32(bounty_id),
            word_uint(1),
            word_address(solver),
            word_bytes32(submission),
            word_bytes32(evidence),
            word_bytes32(policy),
        ]
    )
    for nonce in range(cap):
        work_hash = keccak_bytes(prefix + word_uint(nonce))
        if int.from_bytes(work_hash, "big") >> 240 == 0:
            return nonce, work_hash
    raise RuntimeError("leading-zero proof search cap reached")


def dynamic_reveal_data(submission: bytes, evidence: bytes, salt: bytes, nonce: int) -> str:
    proof = word_uint(nonce)
    head = b"".join([submission, evidence, salt, word_uint(128)])
    tail = word_uint(len(proof)) + proof
    return f"0x{(selector('revealSolution(bytes32,bytes32,bytes32,bytes)') + head + tail).hex()}"


def indexed_address(topic: str) -> str:
    return f"0x{topic[-40:]}".lower()


def run(bundle: dict[str, Any], args: argparse.Namespace) -> dict[str, Any]:
    if bundle.get("schema_version") != "agent-bounties/open-competition-v1-mainnet-bundle-v1":
        raise ValueError("mainnet bundle schema mismatch")
    if bundle.get("network") != "base-mainnet" or bundle.get("chain_id") != CHAIN_ID:
        raise ValueError("bundle is not Base mainnet")
    if bundle.get("deployer") != ADMIN or bundle.get("settlement_token") != USDC:
        raise ValueError("bundle deployer or settlement token mismatch")
    action = bundle["actions"][0]
    fork_block = bundle["preflight_block"]
    upstream = rpc(args.rpc_url, "eth_getBlockByNumber", [hex(fork_block["number"]), False])
    if not upstream or upstream.get("hash", "").lower() != fork_block["hash"]:
        raise RuntimeError("upstream fork block hash mismatch")

    local_url = f"http://127.0.0.1:{args.port}"
    process = subprocess.Popen(
        [
            args.anvil,
            "--fork-url",
            args.rpc_url,
            "--fork-block-number",
            str(fork_block["number"]),
            "--chain-id",
            str(CHAIN_ID),
            "--host",
            "127.0.0.1",
            "--port",
            str(args.port),
            "--silent",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        for _ in range(120):
            try:
                if int(rpc(local_url, "eth_chainId", []), 16) == CHAIN_ID:
                    break
            except RuntimeError:
                time.sleep(0.25)
        else:
            detail = process.stderr.read() if process.stderr else ""
            raise RuntimeError(f"Anvil fork did not start: {detail}")

        rpc(local_url, "anvil_impersonateAccount", [ADMIN])
        admin_start = token_balance(local_url, ADMIN)
        if admin_start < bundle["hidden_canary"]["total_admin_usdc_budget_base_units"]:
            raise RuntimeError("pinned admin USDC balance cannot fund exact canary replay")
        if int(rpc(local_url, "eth_getTransactionCount", [ADMIN, "latest"]), 16) != action["from_nonce"]:
            raise RuntimeError("fork admin nonce does not match deployment action")

        deployment = send(local_url, ADMIN, data=action["data"])
        if str(deployment.get("contractAddress", "")).lower() != action["expected_contract"]:
            raise RuntimeError("factory deployed at unexpected address")
        if rpc(local_url, "eth_getCode", [action["expected_contract"], "latest"]).lower() != action["expected_runtime_code"].lower():
            raise RuntimeError("factory runtime mismatch on exact fork replay")
        if rpc(local_url, "eth_getCode", [action["expected_implementation"], "latest"]).lower() != action["expected_implementation_runtime_code"].lower():
            raise RuntimeError("implementation runtime mismatch on exact fork replay")

        rpc(local_url, "anvil_impersonateAccount", [SOLVER])
        solver_start = token_balance(local_url, SOLVER)
        gas_funding = send(local_url, ADMIN, to=SOLVER, value=SOLVER_GAS_FUNDING_WEI)
        solver_funding = send(
            local_url,
            ADMIN,
            to=USDC,
            data=calldata("transfer(address,uint256)", word_address(SOLVER), word_uint(100_000)),
        )
        approval = send(
            local_url,
            ADMIN,
            to=USDC,
            data=calldata("approve(address,uint256)", word_address(action["expected_contract"]), word_uint(1_100_000)),
        )
        latest = rpc(local_url, "eth_getBlockByNumber", ["latest", False])
        create_data, profile = create_competition_data(bundle, int(latest["timestamp"], 16))
        creation = send(local_url, ADMIN, to=action["expected_contract"], data=create_data)
        created_topic = keccak_hex(
            b"CanonicalCompetitionCreated(bytes32,address,address,bytes32,bytes32,bytes32)"
        )
        created_logs = [log for log in creation["logs"] if log["topics"][0].lower() == created_topic]
        if len(created_logs) != 1:
            raise RuntimeError("canonical competition creation event missing")
        bounty_id = bytes.fromhex(created_logs[0]["topics"][1][2:])
        bounty = indexed_address(created_logs[0]["topics"][2])
        if indexed_address(created_logs[0]["topics"][3]) != ADMIN:
            raise RuntimeError("canary creator mismatch")

        solver_approval = send(
            local_url,
            SOLVER,
            to=USDC,
            data=calldata("approve(address,uint256)", word_address(bounty), word_uint(100_000)),
        )
        submission = hash_text("agent-bounties/open-competition-v1/mainnet-canary/submission-v1")
        evidence = hash_text("agent-bounties/open-competition-v1/mainnet-canary/evidence-v1")
        salt = hash_text("agent-bounties/open-competition-v1/mainnet-fork-only-salt-v1")
        committed_value = commitment(bounty, SOLVER, submission, evidence, salt)
        committed = send(
            local_url,
            SOLVER,
            to=bounty,
            data=calldata("commitSolution(bytes32)", word_bytes32(committed_value)),
        )
        rpc(local_url, "evm_mine", [])
        nonce, work_hash = mine_proof(
            bounty_id,
            SOLVER,
            submission,
            evidence,
            bytes.fromhex(profile["policy_hash"][2:]),
        )
        settled = send(
            local_url,
            SOLVER,
            to=bounty,
            data=dynamic_reveal_data(submission, evidence, salt, nonce),
        )
        settled_topic = keccak_hex(
            b"BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)"
        )
        settled_logs = [log for log in settled["logs"] if log["topics"][0].lower() == settled_topic]
        if len(settled_logs) != 1 or settled_logs[0]["address"].lower() != bounty:
            raise RuntimeError("canonical BountySettled event missing")
        if indexed_address(settled_logs[0]["topics"][3]) != SOLVER:
            raise RuntimeError("settled winner mismatch")

        admin_final = token_balance(local_url, ADMIN)
        solver_final = token_balance(local_url, SOLVER)
        bounty_final = token_balance(local_url, bounty)
        status = call_uint(local_url, bounty, calldata("competitionStatus()"))
        winner = indexed_address(rpc(local_url, "eth_call", [{"to": bounty, "data": calldata("winner()")}, "latest"]))
        assertions = {
            "factory_address_matches": deployment["contractAddress"].lower() == action["expected_contract"],
            "factory_runtime_matches": keccak_hex(bytes.fromhex(rpc(local_url, "eth_getCode", [action["expected_contract"], "latest"])[2:])) == action["runtime_code_hash"],
            "implementation_runtime_matches": keccak_hex(bytes.fromhex(rpc(local_url, "eth_getCode", [action["expected_implementation"], "latest"])[2:])) == action["implementation_runtime_code_hash"],
            "creator_is_not_solver": ADMIN != SOLVER,
            "settled_status": status == 2,
            "winner_is_separate_solver": winner == SOLVER,
            "admin_usdc_delta_matches": admin_final - admin_start == -1_100_000,
            "solver_usdc_delta_matches": solver_final - solver_start == 1_100_000,
            "bounty_balance_zero": bounty_final == 0,
            "canonical_settlement_event": len(settled_logs) == 1,
        }
        if not all(assertions.values()):
            raise RuntimeError(f"fork replay reconciliation failed: {json.dumps(assertions, sort_keys=True)}")
        transactions = {
            "factory_deployment": deployment,
            "solver_gas_funding": gas_funding,
            "solver_usdc_funding": solver_funding,
            "factory_approval": approval,
            "competition_creation": creation,
            "solver_bond_approval": solver_approval,
            "solution_commitment": committed,
            "solution_reveal_and_settlement": settled,
        }
        return {
            "schema_version": "agent-bounties/open-competition-v1-mainnet-fork-replay-v1",
            "protocol_version": "agent-bounties/open-competition-v1",
            "network": "base-mainnet-fork",
            "chain_id": CHAIN_ID,
            "source_commit": bundle["source_commit"],
            "fork_block": fork_block,
            "deployer": ADMIN,
            "factory": action["expected_contract"],
            "implementation": action["expected_implementation"],
            "verifier": VERIFIER,
            "solver": SOLVER,
            "bounty": bounty,
            "bounty_id": f"0x{bounty_id.hex()}",
            "canary_profile": profile,
            "proof": {"nonce": nonce, "work_hash": f"0x{work_hash.hex()}", "difficulty_bits": 16},
            "transactions": {
                name: {
                    "transaction_hash": value["transactionHash"],
                    "block_number": int(value["blockNumber"], 16),
                    "block_hash": value["blockHash"],
                    "gas_used": int(value["gasUsed"], 16),
                }
                for name, value in transactions.items()
            },
            "settlement_event": {
                "name": "BountySettled",
                "transaction_hash": settled_logs[0]["transactionHash"],
                "block_number": int(settled_logs[0]["blockNumber"], 16),
                "block_hash": settled_logs[0]["blockHash"],
                "log_index": int(settled_logs[0]["logIndex"], 16),
            },
            "usdc_reconciliation": {
                "admin_before": admin_start,
                "admin_after": admin_final,
                "admin_delta": admin_final - admin_start,
                "solver_before": solver_start,
                "solver_after": solver_final,
                "solver_delta": solver_final - solver_start,
                "bounty_after": bounty_final,
            },
            "assertions": assertions,
            "passed": True,
            "broadcast": False,
            "evidence_boundary": "This is a deterministic local Anvil replay at an exact canonical Base block. No mainnet transaction was broadcast.",
        }
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rpc-url", default="https://mainnet.base.org")
    parser.add_argument("--anvil", default="/home/pooln/.foundry/bin/anvil")
    parser.add_argument("--port", type=int, default=8547)
    args = parser.parse_args()
    manifest = run(json.loads(args.bundle.read_text(encoding="utf-8")), args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(args.output), "passed": manifest["passed"], "factory": manifest["factory"], "bounty": manifest["bounty"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
