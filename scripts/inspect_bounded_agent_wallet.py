#!/usr/bin/env python3
"""Inspect a bounded agent wallet at one Base safe block."""

from __future__ import annotations

import argparse
import json
import subprocess
import urllib.error
import urllib.request
from pathlib import Path

from plan_bounded_agent_budget import CAST, ROOT, require_address, require_bytes32, validate_manifest


DEFAULT_MANIFEST = ROOT / "deployments" / "bounded-agent-wallet-base-mainnet.json"
ZERO_ADDRESS = "0x" + "00" * 20
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
    "allowed_verification_modes",
    "deterministic_verifier_module",
    "signed_quorum_verifier_set_hash",
    "ai_judge_verifier_set_hash",
)


def run(command: list[str], input_text: str | None = None) -> str:
    result = subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        input=input_text,
    )
    return result.stdout.strip()


def rpc(url: str, method: str, params: list, request_id: int) -> object:
    request = urllib.request.Request(
        url,
        data=json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}).encode(),
        headers={"content-type": "application/json", "user-agent": "agent-bounties/1.0"},
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            payload = json.loads(response.read().decode())
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
        raise RuntimeError(f"Base RPC {method} failed") from error
    if payload.get("error"):
        raise RuntimeError(f"Base RPC {method} returned {payload['error'].get('message', 'an error')}")
    if "result" not in payload:
        raise RuntimeError(f"Base RPC {method} omitted result")
    return payload["result"]


def selector(signature: str) -> str:
    return run([CAST, "sig", signature]).lower()


def call(url: str, target: str, signature: str, block: str, arguments: tuple[str, ...] = ()) -> str:
    data = run([CAST, "calldata", signature, *arguments]).lower()
    result = rpc(url, "eth_call", [{"to": target, "data": data}, block], 20)
    if not isinstance(result, str) or not result.startswith("0x"):
        raise RuntimeError(f"invalid {signature} result")
    return result.lower()


def words(value: str) -> list[str]:
    raw = value.removeprefix("0x")
    if len(raw) == 0 or len(raw) % 64:
        raise RuntimeError("ABI result is not word aligned")
    return [raw[index : index + 64] for index in range(0, len(raw), 64)]


def word_address(word: str) -> str:
    if len(word) != 64 or any(character not in "0123456789abcdef" for character in word):
        raise RuntimeError("invalid ABI address word")
    return f"0x{word[-40:]}"


def word_uint(word: str) -> int:
    return int(word, 16)


def code_hash(code: str) -> str | None:
    if code in {"0x", "0x0"}:
        return None
    return run([CAST, "keccak"], input_text=code).lower()


def inspect(
    rpc_url: str,
    wallet: str,
    manifest: dict,
    expected_owner: str | None = None,
    expected_delegate: str | None = None,
    expected_policy_hash: str | None = None,
) -> dict:
    validate_manifest(manifest)
    safe = rpc(rpc_url, "eth_getBlockByNumber", ["safe", False], 1)
    if not isinstance(safe, dict) or not safe.get("number") or not safe.get("hash"):
        raise RuntimeError("Base safe block is unavailable")
    block = str(safe["number"])
    timestamp = int(str(safe["timestamp"]), 16)
    expected_chain = int(manifest["chain_id"])
    chain_id = int(str(rpc(rpc_url, "eth_chainId", [], 2)), 16)
    factory = require_address(manifest["wallet_factory"]["address"], "wallet factory")
    implementation = require_address(manifest["wallet_factory"]["implementation"], "implementation")
    bounty_factory = require_address(manifest["canonical"]["bounty_factory"], "bounty factory")
    settlement_token = require_address(manifest["canonical"]["settlement_token"], "settlement token")
    addresses = [factory, implementation, wallet]
    observed_code = {
        address: str(rpc(rpc_url, "eth_getCode", [address, block], 10 + index)).lower()
        for index, address in enumerate(addresses)
    }
    hashes = {address: code_hash(code) for address, code in observed_code.items()}
    failures: list[str] = []
    if chain_id != expected_chain:
        failures.append("chain_id_mismatch")
    if hashes[factory] != manifest["wallet_factory"]["runtime_code_hash"]:
        failures.append("wallet_factory_code_mismatch")
    if hashes[implementation] != manifest["wallet_factory"]["implementation_runtime_code_hash"]:
        failures.append("wallet_implementation_code_mismatch")
    if hashes[wallet] != manifest["wallet_factory"]["clone_runtime_code_hash"]:
        failures.append("wallet_clone_code_mismatch")

    state: dict = {}
    if not failures:
        factory_implementation = word_address(words(call(rpc_url, factory, "implementation()(address)", block))[0])
        factory_bounty = word_address(words(call(rpc_url, factory, "bountyFactory()(address)", block))[0])
        factory_token = word_address(words(call(rpc_url, factory, "settlementToken()(address)", block))[0])
        registered = bool(
            word_uint(words(call(rpc_url, factory, "isFactoryWallet(address)(bool)", block, (wallet,)))[0])
        )
        if factory_implementation != implementation:
            failures.append("factory_implementation_mismatch")
        if factory_bounty != bounty_factory:
            failures.append("factory_bounty_binding_mismatch")
        if factory_token != settlement_token:
            failures.append("factory_token_binding_mismatch")
        if not registered:
            failures.append("wallet_not_registered")

        policy_result = call(
            rpc_url,
            wallet,
            (
                "policy()(address,uint64,uint64,uint64,uint256,uint256,uint256,uint256,uint8,uint8,"
                "address,bytes32,bytes32)"
            ),
            block,
        )
        policy_words = words(policy_result)
        policy = dict(zip(POLICY_FIELDS, policy_words, strict=True))
        for name in ("delegate", "deterministic_verifier_module"):
            policy[name] = word_address(str(policy[name]))
        for name in POLICY_FIELDS[1:10]:
            policy[name] = word_uint(str(policy[name]))
        policy["signed_quorum_verifier_set_hash"] = f"0x{policy['signed_quorum_verifier_set_hash']}"
        policy["ai_judge_verifier_set_hash"] = f"0x{policy['ai_judge_verifier_set_hash']}"
        policy_hash = run([CAST, "keccak"], input_text=policy_result).lower()
        owner = word_address(words(call(rpc_url, wallet, "owner()(address)", block))[0])
        delegate_nonce = word_uint(words(call(rpc_url, wallet, "delegateNonce()(uint256)", block))[0])
        policy_version = word_uint(words(call(rpc_url, wallet, "policyVersion()(uint64)", block))[0])
        period_bucket = word_uint(words(call(rpc_url, wallet, "periodBucket()(uint256)", block))[0])
        period_spent = word_uint(words(call(rpc_url, wallet, "periodSpent()(uint256)", block))[0])
        lifetime_spent = word_uint(words(call(rpc_url, wallet, "lifetimeSpent()(uint256)", block))[0])
        revoked = bool(word_uint(words(call(rpc_url, wallet, "revoked()(bool)", block))[0]))
        balance = word_uint(
            words(call(rpc_url, settlement_token, "balanceOf(address)(uint256)", block, (wallet,)))[0]
        )
        if owner == ZERO_ADDRESS:
            failures.append("owner_zero")
        if policy["delegate"] == ZERO_ADDRESS:
            failures.append("delegate_zero")
        if expected_owner is not None and owner != expected_owner:
            failures.append("owner_mismatch")
        if expected_delegate is not None and policy["delegate"] != expected_delegate:
            failures.append("delegate_mismatch")
        if expected_policy_hash is not None and policy_hash != expected_policy_hash:
            failures.append("policy_hash_mismatch")
        if revoked:
            failures.append("policy_revoked")
        if timestamp < policy["valid_after"]:
            failures.append("policy_not_active")
        if timestamp > policy["valid_until"]:
            failures.append("policy_expired")
        if period_spent > policy["max_per_period"]:
            failures.append("period_counter_exceeds_policy")
        if lifetime_spent > policy["max_lifetime_spend"]:
            failures.append("lifetime_counter_exceeds_policy")
        state = {
            "owner": owner,
            "policy": policy,
            "policy_hash": policy_hash,
            "policy_version": policy_version,
            "delegate_nonce": delegate_nonce,
            "period_bucket": str(period_bucket),
            "period_spent": str(period_spent),
            "lifetime_spent": str(lifetime_spent),
            "wallet_usdc_balance": str(balance),
            "revoked": revoked,
            "registered": registered,
        }

    return {
        "schema": "agent-bounties/bounded-agent-wallet-inspection-v1",
        "ready": not failures,
        "failures": failures,
        "network": manifest["network"],
        "chain_id": chain_id,
        "safe_block": {"number": int(block, 16), "hash": safe["hash"], "timestamp": timestamp},
        "wallet": wallet,
        "wallet_factory": factory,
        "runtime_code_hashes": hashes,
        "state": state,
        "evidence_boundary": (
            "This same-block read proves configuration only. It does not authorize an action or prove funding, "
            "completion, payout, or settlement."
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--wallet", required=True)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--rpc-url")
    parser.add_argument("--expect-owner")
    parser.add_argument("--expect-delegate")
    parser.add_argument("--expect-policy-hash")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    manifest = validate_manifest(json.loads(args.manifest.read_text(encoding="utf-8")))
    wallet = require_address(args.wallet, "wallet")
    expected_owner = require_address(args.expect_owner, "expected owner") if args.expect_owner else None
    expected_delegate = require_address(args.expect_delegate, "expected delegate") if args.expect_delegate else None
    expected_policy_hash = (
        require_bytes32(args.expect_policy_hash, "expected policy hash") if args.expect_policy_hash else None
    )
    report = inspect(
        args.rpc_url or manifest["rpc_url"],
        wallet,
        manifest,
        expected_owner,
        expected_delegate,
        expected_policy_hash,
    )
    output = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(output, encoding="utf-8")
        print(args.output)
    else:
        print(output, end="")
    raise SystemExit(0 if report["ready"] else 1)


if __name__ == "__main__":
    main()
