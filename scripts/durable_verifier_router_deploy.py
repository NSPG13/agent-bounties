#!/usr/bin/env python3
"""Plan and deploy the durable policy-bound verifier router and routed V3 policy on Base."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time
import urllib.error
import urllib.request
from typing import Any, Mapping, Sequence


BASE_CHAIN_ID = 8453
BASE_RPC_DEFAULT = "https://mainnet.base.org"
API_DEFAULT = "https://api.agentbounties.app"
SINGLETON_FACTORY = "0xce0042b868300000d44a59004da54a005ffdcf9f"
CANONICAL_FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
NATIVE_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
PARTICIPANT_REGISTRY = "0x9875dcaf570bde8ff1aa62275d3c8985f4fd1294"
TERMS_REGISTRY = "0x35e5d49c12b75c119d33951c2c4f054c5732208c"
VERIFIER_ONE = "0xbe6292b9e465f549e2363b918d6dd9187038431e"
VERIFIER_TWO = "0xb7c2ce6430b66fb986e27b6140b29309550d487a"
KEEPER = "0xc26a630e85134ed30968735c8e7de4576cfa5dbc"
OWNER = "0x884834e884d6e93462655a2820140ad03e6747bc"
BOUNDED_WALLET = "0x1eaa1c68772cf76bc5f4e4174766076e33ace662"
ROUTER_ACTIVATION_DELAY = 7 * 24 * 60 * 60
ROUTER_SALT_TEXT = "agent-bounties/policy-bound-verifier-router/base-mainnet/v1"
ADAPTER_SALT_PREFIX = "agent-bounties/independent-child-v3-routed/base-mainnet/v1"
MIN_KEEPER_ETH_WEI = 100_000_000_000_000
PARENT_TARGET = 2_010_000
TOTAL_REPLACEMENT_FUNDING = 4 * PARENT_TARGET
TEMPLATE_PATH = Path("bounties/autonomous-v1/routed-v3-parent.template.json")
LANES: dict[int, tuple[str, str]] = {
    333: ("cli", "CLI"),
    334: ("api", "API"),
    335: ("mcp", "MCP"),
    336: ("wallet_ux", "wallet UX"),
}
ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")
BYTES32_RE = re.compile(r"^0x[0-9a-fA-F]{64}$")
UINT_RE = re.compile(r"^(?:0x[0-9a-fA-F]+|[0-9]+)")


class DeploymentError(RuntimeError):
    pass


def redact_command(command: Sequence[str]) -> str:
    result: list[str] = []
    hide_next = False
    for item in command:
        if hide_next:
            result.append("***")
            hide_next = False
            continue
        result.append(item)
        if item in {"--private-key", "--rpc-url"}:
            hide_next = True
    return " ".join(result)


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
            f"command failed ({completed.returncode}): {redact_command(command)}\n{completed.stdout[-6000:]}"
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


def parse_bool(value: object, label: str) -> bool:
    text = str(value).strip().lower()
    if text in {"true", "1", "0x1", "0x01"}:
        return True
    if text in {"false", "0", "0x0", "0x00"}:
        return False
    raise DeploymentError(f"{label} is not boolean: {text!r}")


def nonempty_lines(value: str, expected: int, label: str) -> list[str]:
    result = [line.strip() for line in value.splitlines() if line.strip()]
    if len(result) != expected:
        raise DeploymentError(f"{label} returned {len(result)} values; expected {expected}")
    return result


def http_json(method: str, url: str, body: Mapping[str, object] | None = None) -> Any:
    data = None if body is None else json.dumps(body, separators=(",", ":")).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={"content-type": "application/json", "user-agent": "agent-bounties-durable-router/1"},
    )
    try:
        with urllib.request.urlopen(request, timeout=45) as response:
            raw = response.read().decode("utf-8")
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")
        raise DeploymentError(f"{method} {url} failed with HTTP {error.code}: {detail[:2000]}") from error
    try:
        return json.loads(raw)
    except json.JSONDecodeError as error:
        raise DeploymentError(f"{method} {url} returned invalid JSON") from error


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

    def code(self, target: str) -> str:
        return self.rpc("code", target).strip().lower()

    def call(self, target: str, signature: str, *args: str) -> str:
        return self.rpc("call", target, signature, *args).strip()

    def balance(self, target: str) -> int:
        return parse_uint(self.rpc("balance", target), "native balance")

    def bytecode(self, contract: str) -> str:
        value = run([self.forge, "inspect", contract, "bytecode"], cwd=self.contracts).strip().lower()
        if not value.startswith("0x") or len(value) < 4:
            raise DeploymentError(f"forge did not return creation bytecode for {contract}")
        return value

    def abi_encode(self, signature: str, *args: str) -> str:
        value = self.command("abi-encode", signature, *args).strip().lower()
        if not value.startswith("0x"):
            raise DeploymentError("cast abi-encode returned malformed bytes")
        return value

    def keccak(self, value: str) -> str:
        return require_bytes32(self.command("keccak", value), "keccak result")

    def predict(self, init_code: str, salt_text: str) -> tuple[str, str, str]:
        salt = self.keccak(salt_text)
        init_hash = self.keccak(init_code)
        preimage = "0xff" + SINGLETON_FACTORY[2:] + salt[2:] + init_hash[2:]
        prediction_hash = self.keccak(preimage)
        predicted = require_address("0x" + prediction_hash[-40:], "predicted contract")
        return predicted, salt, init_hash

    def send(self, target: str, signature: str, *args: str, private_key: str) -> dict[str, Any]:
        raw = self.rpc(
            "send", target, signature, *args, "--private-key", private_key, "--json", timeout=180
        )
        try:
            transaction = json.loads(raw)
        except json.JSONDecodeError as error:
            raise DeploymentError("cast send did not return JSON") from error
        tx_hash = transaction.get("transactionHash") or transaction.get("transaction_hash")
        if not isinstance(tx_hash, str) or not BYTES32_RE.fullmatch(tx_hash):
            raise DeploymentError("cast send JSON is missing a transaction hash")
        receipt_raw = self.rpc("receipt", tx_hash, "--json", timeout=180)
        try:
            receipt = json.loads(receipt_raw)
        except json.JSONDecodeError as error:
            raise DeploymentError("cast receipt did not return JSON") from error
        if receipt.get("status") not in {"0x1", "0x01", 1}:
            raise DeploymentError(f"transaction reverted: {tx_hash}")
        transaction["receipt"] = receipt
        return transaction


def creation_nonce(issue: int) -> str:
    return "0x" + hashlib.sha256(f"agent-bounties/routed-v3/{issue}/v1".encode()).hexdigest()


def materialize_terms(repo: Path, router: str) -> dict[int, dict[str, Any]]:
    template = (repo / TEMPLATE_PATH).read_text(encoding="utf-8")
    result: dict[int, dict[str, Any]] = {}
    for issue, (lane, label) in LANES.items():
        rendered = (
            template.replace("__ROUTER_ADDRESS__", router)
            .replace("__CREATION_NONCE__", creation_nonce(issue))
            .replace("__LANE_LABEL__", label)
            .replace("__LANE__", lane)
            .replace("__ISSUE__", str(issue))
        )
        if "__" in rendered:
            raise DeploymentError(f"unresolved routed terms placeholder for issue #{issue}")
        try:
            document = json.loads(rendered)
        except json.JSONDecodeError as error:
            raise DeploymentError(f"rendered routed terms for issue #{issue} are invalid JSON") from error
        policy = document.get("verification_policy")
        if not isinstance(policy, dict) or str(policy.get("verifier_module", "")).lower() != router:
            raise DeploymentError(f"routed terms for issue #{issue} do not bind the router")
        result[issue] = document
    return result


def create_payload(document: Mapping[str, Any], published: Mapping[str, Any]) -> dict[str, Any]:
    terms = document["contract_terms"]
    policy = document["verification_policy"]
    return {
        "creator": terms["creator_wallet"],
        "solver_reward": terms["solver_reward"],
        "verifier_reward": terms["verifier_reward"],
        "terms_hash": published["terms_hash"],
        "policy_hash": published["policy_hash"],
        "acceptance_criteria_hash": published["acceptance_criteria_hash"],
        "benchmark_hash": published["benchmark_hash"],
        "evidence_schema_hash": published["evidence_schema_hash"],
        "funding_deadline": terms["funding_deadline"],
        "claim_window_seconds": terms["claim_window_seconds"],
        "verification_window_seconds": terms["verification_window_seconds"],
        "verification_mode": "deterministic_module",
        "verifier_module": policy["verifier_module"],
        "verifier_reward_recipient": policy["verifier_reward_recipient"],
        "verifiers": [],
        "threshold": 1,
        "initial_funding": terms["initial_funding"],
        "creation_nonce": terms["creation_nonce"],
    }


def build_router_plan(foundry: Foundry) -> dict[str, Any]:
    if foundry.chain_id() != BASE_CHAIN_ID:
        raise DeploymentError("durable verifier router is pinned to Base mainnet chain 8453")
    for label, target in {
        "ERC-2470 singleton factory": SINGLETON_FACTORY,
        "canonical bounty factory": CANONICAL_FACTORY,
        "native USDC": NATIVE_USDC,
        "participant registry": PARTICIPANT_REGISTRY,
        "terms registry": TERMS_REGISTRY,
        "bounded wallet": BOUNDED_WALLET,
    }.items():
        if foundry.code(target) in {"0x", "0x0"}:
            raise DeploymentError(f"{label} has no runtime code at {target}")

    constructor = foundry.abi_encode(
        "f(address,address,address,uint64)", CANONICAL_FACTORY, KEEPER, OWNER, str(ROUTER_ACTIVATION_DELAY)
    )
    init_code = foundry.bytecode(
        "src/PolicyBoundVerifierRouter.sol:PolicyBoundVerifierRouter"
    ) + constructor[2:]
    router, salt, init_hash = foundry.predict(init_code, ROUTER_SALT_TEXT)
    verifier_array = foundry.abi_encode("f(address[])", f"[{VERIFIER_ONE},{VERIFIER_TWO}]")
    verifier_set_hash = foundry.keccak(verifier_array)

    policy_lines = nonempty_lines(
        foundry.call(
            BOUNDED_WALLET,
            "policy()(address,uint64,uint64,uint64,uint256,uint256,uint256,uint256,uint8,uint8,address,bytes32,bytes32)",
        ),
        13,
        "bounded wallet policy",
    )
    wallet_balance = parse_uint(
        foundry.call(NATIVE_USDC, "balanceOf(address)(uint256)", BOUNDED_WALLET), "bounded wallet USDC"
    )
    return {
        "schema": "agent-bounties/durable-verifier-router-plan-v1",
        "network": "base-mainnet",
        "chain_id": BASE_CHAIN_ID,
        "singleton_factory": SINGLETON_FACTORY,
        "canonical_factory": CANONICAL_FACTORY,
        "settlement_token": NATIVE_USDC,
        "participant_registry": PARTICIPANT_REGISTRY,
        "terms_registry": TERMS_REGISTRY,
        "registrar": KEEPER,
        "guardian": OWNER,
        "activation_delay_seconds": ROUTER_ACTIVATION_DELAY,
        "router_salt": salt,
        "router_init_code_hash": init_hash,
        "predicted_router": router,
        "router_already_deployed": foundry.code(router) not in {"0x", "0x0"},
        "verifier_set_hash": verifier_set_hash,
        "verifier_threshold": 2,
        "keeper_eth_balance_wei": foundry.balance(KEEPER),
        "bounded_wallet": {
            "address": BOUNDED_WALLET,
            "owner": require_address(foundry.call(BOUNDED_WALLET, "owner()(address)"), "wallet owner"),
            "usdc_balance_base_units": wallet_balance,
            "can_fund_four_replacements": wallet_balance >= TOTAL_REPLACEMENT_FUNDING,
            "policy_version": parse_uint(
                foundry.call(BOUNDED_WALLET, "policyVersion()(uint64)"), "wallet policy version"
            ),
            "policy_hash": require_bytes32(
                foundry.call(BOUNDED_WALLET, "policyHash()(bytes32)"), "wallet policy hash"
            ),
            "policy": {
                "delegate": require_address(policy_lines[0], "policy delegate"),
                "valid_after": parse_uint(policy_lines[1], "valid after"),
                "valid_until": parse_uint(policy_lines[2], "valid until"),
                "period_seconds": parse_uint(policy_lines[3], "period seconds"),
                "max_per_action": parse_uint(policy_lines[4], "max per action"),
                "max_per_period": parse_uint(policy_lines[5], "max per period"),
                "max_lifetime_spend": parse_uint(policy_lines[6], "max lifetime spend"),
                "max_bounty_target": parse_uint(policy_lines[7], "max bounty target"),
                "allowed_actions": parse_uint(policy_lines[8], "allowed actions"),
                "allowed_verification_modes": parse_uint(policy_lines[9], "allowed verification modes"),
                "deterministic_verifier": require_address(policy_lines[10], "deterministic verifier"),
                "signed_quorum_hash": require_bytes32(policy_lines[11], "signed quorum hash"),
                "ai_quorum_hash": require_bytes32(policy_lines[12], "AI quorum hash"),
            },
        },
        "replacement_economics": {
            "count": 4,
            "target_each": PARENT_TARGET,
            "total_funding_required": TOTAL_REPLACEMENT_FUNDING,
            "solver_reward_each": 2_000_000,
            "verifier_reward_each": 10_000,
            "claim_bond_each": 10_000,
            "minimum_child_target_each": 1_000_000,
            "minimum_parent_margin_each": 1_000_000,
        },
        "router_init_code": init_code,
        "evidence_boundary": (
            "This plan is compiler and read-only chain evidence. It is not router deployment, wallet policy activation, "
            "replacement funding, claimability, or payment evidence."
        ),
    }


def wait_for_code(foundry: Foundry, target: str, timeout_seconds: int = 120) -> str:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        code = foundry.code(target)
        if code not in {"0x", "0x0"}:
            return code
        time.sleep(2)
    raise DeploymentError(f"no runtime code appeared at {target}")


def deploy_singleton(
    foundry: Foundry,
    target: str,
    init_code: str,
    salt: str,
    private_key: str,
) -> dict[str, Any] | None:
    if foundry.code(target) not in {"0x", "0x0"}:
        return None
    transaction = foundry.send(
        SINGLETON_FACTORY,
        "deploy(bytes,bytes32)(address)",
        init_code,
        salt,
        private_key=private_key,
    )
    wait_for_code(foundry, target)
    return transaction


def publish_terms(api: str, documents: Mapping[int, Mapping[str, Any]]) -> dict[int, dict[str, Any]]:
    published: dict[int, dict[str, Any]] = {}
    policy_hashes: set[str] = set()
    for issue, document in documents.items():
        response = http_json(
            "POST",
            f"{api.rstrip('/')}/v1/base/autonomous-bounties/terms",
            {"creator_wallet": BOUNDED_WALLET, "document": document},
        )
        if not isinstance(response, dict):
            raise DeploymentError(f"terms publication for issue #{issue} returned a non-object")
        for key in (
            "terms_hash",
            "policy_hash",
            "acceptance_criteria_hash",
            "benchmark_hash",
            "evidence_schema_hash",
        ):
            require_bytes32(response.get(key), f"issue #{issue} {key}")
        policy_hashes.add(str(response["policy_hash"]).lower())
        published[issue] = response
    if len(policy_hashes) != 1:
        raise DeploymentError("routed replacement terms did not produce one shared policy hash")
    return published


def build_adapter_plan(
    foundry: Foundry,
    router: str,
    policy_hash: str,
    verifier_set_hash: str,
) -> dict[str, Any]:
    constructor = foundry.abi_encode(
        "f(address,bytes32,address,address,address,bytes32,uint8)",
        router,
        policy_hash,
        CANONICAL_FACTORY,
        PARTICIPANT_REGISTRY,
        TERMS_REGISTRY,
        verifier_set_hash,
        "2",
    )
    init_code = foundry.bytecode(
        "src/CanonicalIndependentChildVerifierV3Routed.sol:CanonicalIndependentChildVerifierV3Routed"
    ) + constructor[2:]
    adapter, salt, init_hash = foundry.predict(init_code, f"{ADAPTER_SALT_PREFIX}:{policy_hash}")
    return {
        "policy_hash": policy_hash,
        "predicted_adapter": adapter,
        "adapter_salt": salt,
        "adapter_init_code_hash": init_hash,
        "adapter_init_code": init_code,
        "already_deployed": foundry.code(adapter) not in {"0x", "0x0"},
    }


def verify_router(foundry: Foundry, plan: Mapping[str, Any]) -> dict[str, Any]:
    router = require_address(plan["predicted_router"], "router")
    code = wait_for_code(foundry, router)
    checks: dict[str, tuple[object, object]] = {
        "canonical_factory": (
            require_address(foundry.call(router, "canonicalFactory()(address)"), "router factory"),
            CANONICAL_FACTORY,
        ),
        "registrar": (require_address(foundry.call(router, "registrar()(address)"), "router registrar"), KEEPER),
        "guardian": (require_address(foundry.call(router, "guardian()(address)"), "router guardian"), OWNER),
        "activation_delay": (
            parse_uint(foundry.call(router, "activationDelay()(uint64)"), "router activation delay"),
            ROUTER_ACTIVATION_DELAY,
        ),
    }
    mismatches = {
        key: {"observed": observed, "expected": expected}
        for key, (observed, expected) in checks.items()
        if observed != expected
    }
    if mismatches:
        raise DeploymentError(f"router immutable mismatch: {mismatches}")
    return {
        "address": router,
        "runtime_code_hash": foundry.keccak(code),
        "immutable_checks": {key: observed for key, (observed, _) in checks.items()},
    }


def verify_adapter(
    foundry: Foundry,
    adapter_plan: Mapping[str, Any],
    router: str,
    verifier_set_hash: str,
) -> dict[str, Any]:
    adapter = require_address(adapter_plan["predicted_adapter"], "adapter")
    policy_hash = require_bytes32(adapter_plan["policy_hash"], "adapter policy hash")
    code = wait_for_code(foundry, adapter)
    checks: dict[str, tuple[object, object]] = {
        "verifier_router": (
            require_address(foundry.call(adapter, "verifierRouter()(address)"), "adapter router"),
            router,
        ),
        "committed_policy_hash": (
            require_bytes32(foundry.call(adapter, "committedPolicyHash()(bytes32)"), "adapter policy"),
            policy_hash,
        ),
        "canonical_factory": (
            require_address(foundry.call(adapter, "canonicalFactory()(address)"), "adapter factory"),
            CANONICAL_FACTORY,
        ),
        "settlement_token": (
            require_address(foundry.call(adapter, "settlementToken()(address)"), "adapter token"),
            NATIVE_USDC,
        ),
        "participant_registry": (
            require_address(foundry.call(adapter, "participantRegistry()(address)"), "adapter participant registry"),
            PARTICIPANT_REGISTRY,
        ),
        "terms_registry": (
            require_address(foundry.call(adapter, "termsRegistry()(address)"), "adapter terms registry"),
            TERMS_REGISTRY,
        ),
        "verifier_set_hash": (
            require_bytes32(foundry.call(adapter, "taskVerifierSetHash()(bytes32)"), "adapter verifier set"),
            verifier_set_hash,
        ),
        "verifier_threshold": (
            parse_uint(foundry.call(adapter, "taskVerifierThreshold()(uint8)"), "adapter threshold"),
            2,
        ),
        "minimum_child_target": (
            parse_uint(foundry.call(adapter, "MINIMUM_CHILD_TARGET()(uint256)"), "minimum child target"),
            1_000_000,
        ),
        "minimum_parent_margin": (
            parse_uint(
                foundry.call(adapter, "MINIMUM_PARENT_GROSS_MARGIN()(uint256)"),
                "minimum parent margin",
            ),
            1_000_000,
        ),
    }
    mismatches = {
        key: {"observed": observed, "expected": expected}
        for key, (observed, expected) in checks.items()
        if observed != expected
    }
    if mismatches:
        raise DeploymentError(f"routed adapter immutable mismatch: {mismatches}")
    return {
        "address": adapter,
        "runtime_code_hash": foundry.keccak(code),
        "acceptance_criteria_hash": require_bytes32(
            foundry.call(adapter, "ACCEPTANCE_CRITERIA_HASH()(bytes32)"), "adapter acceptance hash"
        ),
        "immutable_checks": {key: observed for key, (observed, _) in checks.items()},
    }


def bootstrap_router(
    foundry: Foundry,
    router: str,
    policy_hash: str,
    adapter: str,
    private_key: str,
) -> dict[str, Any] | None:
    if parse_bool(foundry.call(router, "isPolicyActive(bytes32)(bool)", policy_hash), "router policy active"):
        return None
    transaction = foundry.send(
        router,
        "bootstrapPolicy(bytes32,address)",
        policy_hash,
        adapter,
        private_key=private_key,
    )
    if not parse_bool(foundry.call(router, "isPolicyActive(bytes32)(bool)", policy_hash), "router policy active"):
        raise DeploymentError("router bootstrap transaction confirmed without an active policy")
    return transaction


def deploy(
    foundry: Foundry,
    plan: dict[str, Any],
    api: str,
    output_dir: Path,
) -> dict[str, Any]:
    private_key = os.environ.get("BASE_KEEPER_PRIVATE_KEY", "").strip()
    if not private_key:
        raise DeploymentError("BASE_KEEPER_PRIVATE_KEY is required for deployment")
    deployer = require_address(
        foundry.command("wallet", "address", "--private-key", private_key), "keeper private-key address"
    )
    if deployer != KEEPER:
        raise DeploymentError(f"keeper key resolves to {deployer}, expected {KEEPER}")
    balance_before = foundry.balance(deployer)
    if balance_before < MIN_KEEPER_ETH_WEI:
        raise DeploymentError("keeper ETH reserve is below the protected deployment floor")

    router = require_address(plan["predicted_router"], "predicted router")
    router_tx = deploy_singleton(
        foundry,
        router,
        str(plan["router_init_code"]),
        str(plan["router_salt"]),
        private_key,
    )
    router_verification = verify_router(foundry, plan)

    documents = materialize_terms(foundry.repo, router)
    published = publish_terms(api, documents)
    policy_hash = require_bytes32(next(iter(published.values()))["policy_hash"], "shared policy hash")
    for issue, response in published.items():
        if str(response["policy_hash"]).lower() != policy_hash:
            raise DeploymentError(f"issue #{issue} policy hash drifted")

    adapter_plan = build_adapter_plan(foundry, router, policy_hash, str(plan["verifier_set_hash"]))
    adapter = require_address(adapter_plan["predicted_adapter"], "predicted adapter")
    adapter_tx = deploy_singleton(
        foundry,
        adapter,
        str(adapter_plan["adapter_init_code"]),
        str(adapter_plan["adapter_salt"]),
        private_key,
    )
    adapter_verification = verify_adapter(
        foundry,
        adapter_plan,
        router,
        str(plan["verifier_set_hash"]),
    )
    bootstrap_tx = bootstrap_router(foundry, router, policy_hash, adapter, private_key)

    output_dir.mkdir(parents=True, exist_ok=True)
    materialized: dict[str, str] = {}
    for issue, document in documents.items():
        path = output_dir / f"routed-v3-{issue}.json"
        path.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        materialized[str(issue)] = str(path)

    balance_after = foundry.balance(deployer)
    if balance_after < MIN_KEEPER_ETH_WEI:
        raise DeploymentError("deployment depleted the keeper below its protected ETH reserve")
    return {
        "schema": "agent-bounties/durable-verifier-router-deployment-v1",
        "network": "base-mainnet",
        "chain_id": BASE_CHAIN_ID,
        "deployer": deployer,
        "router_transaction": router_tx,
        "adapter_transaction": adapter_tx,
        "bootstrap_transaction": bootstrap_tx,
        "idempotent": {
            "router_existing": router_tx is None,
            "adapter_existing": adapter_tx is None,
            "policy_existing": bootstrap_tx is None,
        },
        "keeper_balance_before_wei": balance_before,
        "keeper_balance_after_wei": balance_after,
        "router": router_verification,
        "policy_hash": policy_hash,
        "adapter": adapter_verification,
        "published_terms": {str(key): value for key, value in published.items()},
        "materialized_terms": materialized,
        "wallet_policy_required": {
            "wallet": BOUNDED_WALLET,
            "owner": OWNER,
            "durable_router": router,
            "one_final_owner_transaction": True,
        },
        "replacement_funding_required_base_units": TOTAL_REPLACEMENT_FUNDING,
        "evidence_boundary": (
            "Confirmed router, adapter, and active append-only policy evidence proves verifier infrastructure only. "
            "The bounded wallet is not authorized for the router until PolicyConfigured is confirmed, and replacements "
            "are not funded or claimable until canonical lifecycle events reconcile."
        ),
    }


def markdown(report: Mapping[str, Any]) -> str:
    wallet = report["bounded_wallet"]
    economics = report["replacement_economics"]
    return "\n".join(
        [
            "## Durable verifier-router preflight",
            "",
            f"- Predicted stable router: `{report['predicted_router']}`",
            f"- Router already deployed: **{str(report['router_already_deployed']).lower()}**",
            f"- Registrar: `{report['registrar']}`",
            f"- Guardian: `{report['guardian']}`",
            f"- Future-policy activation delay: **{report['activation_delay_seconds'] // 86400} days**",
            f"- Bounded wallet: `{wallet['address']}`",
            f"- Bounded-wallet USDC: **{wallet['usdc_balance_base_units'] / 1_000_000:.6f}**",
            f"- Four replacement parents require: **{economics['total_funding_required'] / 1_000_000:.6f} USDC**",
            f"- Expected balance after funding: **{(wallet['usdc_balance_base_units'] - economics['total_funding_required']) / 1_000_000:.6f} USDC**",
            f"- Current wallet verifier: `{wallet['policy']['deterministic_verifier']}`",
            "",
            "The router is non-upgradeable, cannot move funds, and permanently pins each active policy hash to one implementation code hash.",
            "This preflight is not deployment, owner-policy activation, replacement funding, claimability, or payment evidence.",
            "",
        ]
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("plan", "deploy"))
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", BASE_RPC_DEFAULT))
    parser.add_argument("--api", default=os.environ.get("AGENT_BOUNTIES_API_URL", API_DEFAULT))
    parser.add_argument("--forge", default=os.environ.get("FORGE_BIN", "forge"))
    parser.add_argument("--cast", default=os.environ.get("CAST_BIN", "cast"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, default=Path("target/durable-verifier-router"))
    parser.add_argument("--markdown-output", type=Path)
    args = parser.parse_args()

    repo = Path(__file__).resolve().parents[1]
    foundry = Foundry(repo, args.rpc_url, args.forge, args.cast)
    plan = build_router_plan(foundry)
    report = plan if args.mode == "plan" else deploy(foundry, plan, args.api, args.output_dir)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if args.markdown_output:
        args.markdown_output.parent.mkdir(parents=True, exist_ok=True)
        args.markdown_output.write_text(markdown(plan), encoding="utf-8")
    print(
        json.dumps(
            {
                "mode": args.mode,
                "predicted_router": plan["predicted_router"],
                "wallet_usdc_base_units": plan["bounded_wallet"]["usdc_balance_base_units"],
                "output": str(args.output),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
