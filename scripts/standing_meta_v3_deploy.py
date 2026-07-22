#!/usr/bin/env python3
"""Plan, preflight, and idempotently deploy profitable standing-meta-v3 on Base."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time
from typing import Any, Sequence


BASE_CHAIN_ID = 8453
BASE_RPC_DEFAULT = "https://mainnet.base.org"
SINGLETON_FACTORY = "0xce0042b868300000d44a59004da54a005ffdcf9f"
CANONICAL_FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
NATIVE_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
PARTICIPANT_REGISTRY = "0x9875dcaf570bde8ff1aa62275d3c8985f4fd1294"
TERMS_REGISTRY = "0x35e5d49c12b75c119d33951c2c4f054c5732208c"
VERIFIER_ONE = "0xbe6292b9e465f549e2363b918d6dd9187038431e"
VERIFIER_TWO = "0xb7c2ce6430b66fb986e27b6140b29309550d487a"
EXPECTED_KEEPER = "0xc26a630e85134ed30968735c8e7de4576cfa5dbc"
BOUNDED_WALLET = "0x1eaa1c68772cf76bc5f4e4174766076e33ace662"
DEPLOYMENT_SALT_TEXT = "agent-bounties/standing-meta-v3/base-mainnet/v1"
MIN_KEEPER_ETH_WEI = 100_000_000_000_000
REPLACEMENT_COUNT = 4
PARENT_TARGET_BASE_UNITS = 2_010_000
REPLACEMENT_FUNDING_REQUIRED = REPLACEMENT_COUNT * PARENT_TARGET_BASE_UNITS
ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")
BYTES32_RE = re.compile(r"^0x[0-9a-fA-F]{64}$")
UINT_RE = re.compile(r"^(?:0x[0-9a-fA-F]+|[0-9]+)")


class DeploymentError(RuntimeError):
    pass


def run(command: Sequence[str], *, cwd: Path, timeout: int = 300) -> str:
    completed = subprocess.run(
        list(command),
        cwd=cwd,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
        check=False,
    )
    if completed.returncode != 0:
        raise DeploymentError(
            f"command failed ({completed.returncode}): {' '.join(command)}\n{completed.stdout[-6000:]}"
        )
    return completed.stdout.strip()


def require_address(value: object, label: str) -> str:
    text = str(value).strip().lower()
    if not ADDRESS_RE.fullmatch(text):
        raise DeploymentError(f"{label} is not an EVM address")
    return text


def require_bytes32(value: object, label: str) -> str:
    text = str(value).strip().lower()
    if not BYTES32_RE.fullmatch(text):
        raise DeploymentError(f"{label} is not bytes32")
    return text


def parse_uint(value: object, label: str) -> int:
    text = str(value).strip()
    match = UINT_RE.match(text)
    if not match:
        raise DeploymentError(f"{label} is not an unsigned integer: {text!r}")
    return int(match.group(0), 0)


class Foundry:
    def __init__(self, repo: Path, rpc_url: str, forge: str, cast: str) -> None:
        self.repo = repo
        self.contracts = repo / "contracts" / "base-escrow"
        self.rpc_url = rpc_url
        self.forge = forge
        self.cast = cast

    def command(self, *args: str, cwd: Path | None = None, timeout: int = 300) -> str:
        return run([self.cast, *args], cwd=cwd or self.repo, timeout=timeout)

    def rpc(self, *args: str, timeout: int = 300) -> str:
        return self.command(*args, "--rpc-url", self.rpc_url, timeout=timeout)

    def chain_id(self) -> int:
        return parse_uint(self.rpc("chain-id"), "chain id")

    def code(self, address: str) -> str:
        return self.rpc("code", address).strip().lower()

    def call(self, address: str, signature: str, *args: str) -> str:
        return self.rpc("call", address, signature, *args).strip()

    def balance(self, address: str) -> int:
        return parse_uint(self.rpc("balance", address), "native balance")

    def bytecode(self) -> str:
        value = run(
            [
                self.forge,
                "inspect",
                "src/CanonicalIndependentChildVerifierV3.sol:CanonicalIndependentChildVerifierV3",
                "bytecode",
            ],
            cwd=self.contracts,
        ).strip()
        if not value.startswith("0x") or len(value) < 4:
            raise DeploymentError("forge did not return V3 creation bytecode")
        return value.lower()

    def abi_encode(self, signature: str, *args: str) -> str:
        value = self.command("abi-encode", signature, *args).strip().lower()
        if not value.startswith("0x"):
            raise DeploymentError("cast abi-encode returned malformed bytes")
        return value

    def keccak(self, value: str) -> str:
        return require_bytes32(self.command("keccak", value), "keccak result")


def build_plan(foundry: Foundry) -> dict[str, Any]:
    if foundry.chain_id() != BASE_CHAIN_ID:
        raise DeploymentError("standing-meta-v3 is pinned to Base mainnet chain 8453")
    for label, address in {
        "ERC-2470 singleton factory": SINGLETON_FACTORY,
        "canonical bounty factory": CANONICAL_FACTORY,
        "participant registry": PARTICIPANT_REGISTRY,
        "terms registry": TERMS_REGISTRY,
        "native USDC": NATIVE_USDC,
    }.items():
        if foundry.code(address) in {"0x", "0x0"}:
            raise DeploymentError(f"{label} has no runtime code at {address}")

    verifier_array = foundry.abi_encode("f(address[])", f"[{VERIFIER_ONE},{VERIFIER_TWO}]")
    verifier_set_hash = foundry.keccak(verifier_array)
    constructor = foundry.abi_encode(
        "f(address,address,address,bytes32,uint8)",
        CANONICAL_FACTORY,
        PARTICIPANT_REGISTRY,
        TERMS_REGISTRY,
        verifier_set_hash,
        "2",
    )
    init_code = foundry.bytecode() + constructor[2:]
    init_code_hash = foundry.keccak(init_code)
    salt = foundry.keccak(DEPLOYMENT_SALT_TEXT)
    create2_preimage = "0xff" + SINGLETON_FACTORY[2:] + salt[2:] + init_code_hash[2:]
    prediction_hash = foundry.keccak(create2_preimage)
    predicted = require_address("0x" + prediction_hash[-40:], "predicted verifier")

    keeper_usdc = parse_uint(
        foundry.call(NATIVE_USDC, "balanceOf(address)(uint256)", EXPECTED_KEEPER),
        "keeper USDC balance",
    )
    bounded_wallet_usdc = parse_uint(
        foundry.call(NATIVE_USDC, "balanceOf(address)(uint256)", BOUNDED_WALLET),
        "bounded wallet USDC balance",
    )
    policy_hash = require_bytes32(
        foundry.call(BOUNDED_WALLET, "policyHash()(bytes32)"), "bounded wallet policy hash"
    )
    policy_version = parse_uint(
        foundry.call(BOUNDED_WALLET, "policyVersion()(uint64)"), "bounded wallet policy version"
    )
    current_module = require_address(
        foundry.call(
            BOUNDED_WALLET,
            "policy()(address,uint64,uint64,uint64,uint256,uint256,uint256,uint256,uint8,uint8,address,bytes32,bytes32)",
        ).splitlines()[10],
        "bounded wallet deterministic verifier",
    )
    existing_code = foundry.code(predicted)

    return {
        "schema": "agent-bounties/standing-meta-v3-deployment-plan-v1",
        "network": "base-mainnet",
        "chain_id": BASE_CHAIN_ID,
        "singleton_factory": SINGLETON_FACTORY,
        "canonical_factory": CANONICAL_FACTORY,
        "participant_registry": PARTICIPANT_REGISTRY,
        "terms_registry": TERMS_REGISTRY,
        "settlement_token": NATIVE_USDC,
        "verifier_wallets": [VERIFIER_ONE, VERIFIER_TWO],
        "verifier_set_hash": verifier_set_hash,
        "verifier_threshold": 2,
        "deployment_salt": salt,
        "init_code_hash": init_code_hash,
        "predicted_verifier_module": predicted,
        "already_deployed": existing_code not in {"0x", "0x0"},
        "existing_runtime_code_hash": None
        if existing_code in {"0x", "0x0"}
        else foundry.keccak(existing_code),
        "keeper": {
            "address": EXPECTED_KEEPER,
            "eth_balance_wei": foundry.balance(EXPECTED_KEEPER),
            "usdc_balance_base_units": keeper_usdc,
            "can_fund_four_replacements": keeper_usdc >= REPLACEMENT_FUNDING_REQUIRED,
        },
        "bounded_wallet": {
            "address": BOUNDED_WALLET,
            "usdc_balance_base_units": bounded_wallet_usdc,
            "policy_version": policy_version,
            "policy_hash": policy_hash,
            "deterministic_verifier_module": current_module,
            "requires_owner_policy_update": current_module != predicted,
        },
        "replacement_economics": {
            "count": REPLACEMENT_COUNT,
            "solver_reward_each": 2_000_000,
            "verifier_reward_each": 10_000,
            "claim_bond_each": 10_000,
            "total_parent_funding_each": PARENT_TARGET_BASE_UNITS,
            "total_funding_required": REPLACEMENT_FUNDING_REQUIRED,
            "required_child_target_each": 1_000_000,
            "guaranteed_gross_margin_each": 1_000_000,
        },
        "init_code": init_code,
        "evidence_boundary": (
            "This is read-only chain and compiler evidence. It is not deployment, policy activation, "
            "bounty funding, claimability, or payout evidence."
        ),
    }


def wait_for_code(foundry: Foundry, address: str, timeout_seconds: int = 90) -> str:
    deadline = time.monotonic() + timeout_seconds
    while True:
        code = foundry.code(address)
        if code not in {"0x", "0x0"}:
            return code
        if time.monotonic() >= deadline:
            raise DeploymentError(f"no runtime code appeared at {address}")
        time.sleep(2)


def verify_deployment(foundry: Foundry, plan: dict[str, Any]) -> dict[str, Any]:
    module = plan["predicted_verifier_module"]
    code = wait_for_code(foundry, module)
    checks: dict[str, tuple[object, object]] = {
        "canonical_factory": (
            require_address(foundry.call(module, "canonicalFactory()(address)"), "module factory"),
            CANONICAL_FACTORY,
        ),
        "settlement_token": (
            require_address(foundry.call(module, "settlementToken()(address)"), "module token"),
            NATIVE_USDC,
        ),
        "participant_registry": (
            require_address(foundry.call(module, "participantRegistry()(address)"), "module participant registry"),
            PARTICIPANT_REGISTRY,
        ),
        "terms_registry": (
            require_address(foundry.call(module, "termsRegistry()(address)"), "module terms registry"),
            TERMS_REGISTRY,
        ),
        "verifier_set_hash": (
            require_bytes32(foundry.call(module, "taskVerifierSetHash()(bytes32)"), "module verifier set"),
            plan["verifier_set_hash"],
        ),
        "verifier_threshold": (
            parse_uint(foundry.call(module, "taskVerifierThreshold()(uint8)"), "module verifier threshold"),
            2,
        ),
        "minimum_child_target": (
            parse_uint(foundry.call(module, "MINIMUM_CHILD_TARGET()(uint256)"), "minimum child target"),
            1_000_000,
        ),
        "minimum_parent_gross_margin": (
            parse_uint(
                foundry.call(module, "MINIMUM_PARENT_GROSS_MARGIN()(uint256)"),
                "minimum parent gross margin",
            ),
            1_000_000,
        ),
    }
    mismatches = {
        label: {"observed": observed, "expected": expected}
        for label, (observed, expected) in checks.items()
        if observed != expected
    }
    if mismatches:
        raise DeploymentError(f"V3 immutable mismatch: {mismatches}")
    return {
        "verifier_module": module,
        "runtime_code_hash": foundry.keccak(code),
        "acceptance_criteria_hash": require_bytes32(
            foundry.call(module, "ACCEPTANCE_CRITERIA_HASH()(bytes32)"),
            "acceptance criteria hash",
        ),
        "immutable_checks": {label: observed for label, (observed, _) in checks.items()},
    }


def deploy(foundry: Foundry, plan: dict[str, Any]) -> dict[str, Any]:
    private_key = os.environ.get("BASE_KEEPER_PRIVATE_KEY", "").strip()
    if not private_key:
        raise DeploymentError("BASE_KEEPER_PRIVATE_KEY is required for deployment")
    deployer = require_address(
        foundry.command("wallet", "address", "--private-key", private_key), "keeper private-key address"
    )
    if deployer != EXPECTED_KEEPER:
        raise DeploymentError(f"keeper key resolves to {deployer}, expected {EXPECTED_KEEPER}")
    balance_before = foundry.balance(deployer)
    if balance_before < MIN_KEEPER_ETH_WEI:
        raise DeploymentError("keeper ETH reserve is below the protected deployment floor")

    transaction: dict[str, Any] | None = None
    module = plan["predicted_verifier_module"]
    if foundry.code(module) in {"0x", "0x0"}:
        raw = foundry.rpc(
            "send",
            SINGLETON_FACTORY,
            "deploy(bytes,bytes32)(address)",
            plan["init_code"],
            plan["deployment_salt"],
            "--private-key",
            private_key,
            "--json",
            timeout=180,
        )
        try:
            transaction = json.loads(raw)
        except json.JSONDecodeError as error:
            raise DeploymentError("cast send did not return JSON") from error

    verified = verify_deployment(foundry, plan)
    balance_after = foundry.balance(deployer)
    if balance_after < MIN_KEEPER_ETH_WEI:
        raise DeploymentError("deployment depleted the keeper below its protected ETH reserve")
    return {
        "schema": "agent-bounties/standing-meta-v3-deployment-v1",
        "network": "base-mainnet",
        "chain_id": BASE_CHAIN_ID,
        "deployer": deployer,
        "transaction": transaction,
        "idempotent_existing_deployment": transaction is None,
        "keeper_balance_before_wei": balance_before,
        "keeper_balance_after_wei": balance_after,
        "plan": {key: value for key, value in plan.items() if key != "init_code"},
        "verification": verified,
        "evidence_boundary": (
            "Confirmed runtime code and immutable getter checks prove policy deployment. They do not prove "
            "replacement bounty creation, funding, claimability, or payout."
        ),
    }


def markdown(report: dict[str, Any]) -> str:
    keeper = report["keeper"]
    wallet = report["bounded_wallet"]
    economics = report["replacement_economics"]
    lines = [
        "## Standing-meta-v3 activation preflight",
        "",
        f"- Predicted verifier: `{report['predicted_verifier_module']}`",
        f"- Already deployed: **{str(report['already_deployed']).lower()}**",
        f"- Keeper USDC: **{keeper['usdc_balance_base_units'] / 1_000_000:.6f}**",
        f"- Four replacement parents require: **{economics['total_funding_required'] / 1_000_000:.6f} USDC**",
        f"- Keeper can fund all four directly: **{str(keeper['can_fund_four_replacements']).lower()}**",
        f"- Bounded wallet USDC: **{wallet['usdc_balance_base_units'] / 1_000_000:.6f}**",
        f"- Current bounded-wallet deterministic verifier: `{wallet['deterministic_verifier_module']}`",
        f"- Owner policy update required before bounded-wallet funding: **{str(wallet['requires_owner_policy_update']).lower()}**",
        "",
        "Deployment is deterministic through the ERC-2470 singleton factory and reuses the live V2 participant and terms registries.",
        "This preflight is not deployment or funding evidence.",
    ]
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("plan", "deploy"))
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", BASE_RPC_DEFAULT))
    parser.add_argument("--forge", default=os.environ.get("FORGE_BIN", "forge"))
    parser.add_argument("--cast", default=os.environ.get("CAST_BIN", "cast"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--markdown-output", type=Path)
    args = parser.parse_args()

    repo = Path(__file__).resolve().parents[1]
    foundry = Foundry(repo, args.rpc_url, args.forge, args.cast)
    plan = build_plan(foundry)
    report = plan if args.mode == "plan" else deploy(foundry, plan)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if args.markdown_output:
        args.markdown_output.parent.mkdir(parents=True, exist_ok=True)
        args.markdown_output.write_text(markdown(plan), encoding="utf-8")
    print(json.dumps({
        "mode": args.mode,
        "predicted_verifier_module": plan["predicted_verifier_module"],
        "keeper_can_fund_four_replacements": plan["keeper"]["can_fund_four_replacements"],
        "bounded_wallet_requires_owner_policy_update": plan["bounded_wallet"]["requires_owner_policy_update"],
        "output": str(args.output),
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
