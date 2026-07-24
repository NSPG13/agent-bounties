#!/usr/bin/env python3
"""Plan, stage, resume, and RPC-verify standing-meta-v4 deployments on Base."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time
from typing import Any, Mapping, Sequence


BASE_MAINNET_CHAIN_ID = 8453
BASE_SEPOLIA_CHAIN_ID = 84532
BASE_MAINNET_RPC = "https://mainnet.base.org"
BASE_SEPOLIA_RPC = "https://sepolia.base.org"
BASE_MAINNET_FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
BASE_MAINNET_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
BASE_SEPOLIA_USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e"
BASE_MAINNET_VRF = "0xd5d517abe5cf79b7e95ec98db0f0277788aff634"
BASE_SEPOLIA_VRF = "0x5c210ef41cd1a72de73bf76ec39637bb0d3d7bee"
BASE_MAINNET_KEY_HASH = "0x00b81b5a830cb0a4009fbd8904de511e28631e62ce5ad231373d3cdad373ccab"
BASE_SEPOLIA_KEY_HASH = "0x9e1344a1247c8a1785d0a4681a27152bffdb43666ae5bf7d14d24a5efd44bf71"
EXPECTED_KEEPER = "0xc26a630e85134ed30968735c8e7de4576cfa5dbc"
BOUNDED_WALLET = "0x1eaa1c68772cf76bc5f4e4174766076e33ace662"
EXPECTED_BOUNDED_WALLET_OWNER = "0x884834e884d6e93462655a2820140ad03e6747bc"
MAINNET_SOURCE_USDC_CAP = 7_000_000
EIP170_RUNTIME_LIMIT = 24_576
EIP3860_INITCODE_LIMIT = 49_152
SUBSCRIPTION_CREATED_EVENT = "SubscriptionCreated(uint256,address)"
ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")
BYTES32_RE = re.compile(r"^0x[0-9a-fA-F]{64}$")
CAST_UINT_RE = re.compile(r"^(0x[0-9a-fA-F]+|[0-9]+)(?:\s|$)")
CREATE_JSON_RE = re.compile(r"\{\s*\"deployer\".*?\}\s*$", re.DOTALL)

COMPONENT_SPECS: tuple[tuple[str, str], ...] = (
    ("controller", "src/AnonymousProtocolControllerV1.sol:AnonymousProtocolControllerV1"),
    ("stake_pool", "src/AnonymousStakePoolV1.sol:AnonymousStakePoolV1"),
    ("verifier_sortition", "src/VrfSortitionCoordinatorV1.sol:VrfSortitionCoordinatorV1"),
    ("solver_sortition", "src/VrfSortitionCoordinatorV1.sol:VrfSortitionCoordinatorV1"),
    ("appealable_verifier", "src/AppealableVerifierV1.sol:AppealableVerifierV1"),
    ("standing_meta_child_factory", "src/StandingMetaChildFactoryV4.sol:StandingMetaChildFactoryV4"),
    ("standing_meta_parent_factory", "src/StandingMetaParentFactoryV4.sol:StandingMetaParentFactoryV4"),
    ("standing_meta_v4_bundle", "src/StandingMetaV4Bundle.sol:StandingMetaV4Bundle"),
)
EXPECTED_CANONICAL_COMPONENTS = (
    "anonymous_protocol_controller",
    "anonymous_stake_pool",
    "verifier_sortition",
    "solver_sortition",
    "appealable_verifier",
    "standing_meta_child_factory",
    "standing_meta_parent_factory",
    "onchain_terms_registry",
    "canonical_independent_child_verifier",
    "standing_meta_v4_bundle",
)
REQUIRED_R4_GATES = (
    "independent_review_complete",
    "base_sepolia_rehearsal_complete",
    "base_mainnet_fork_test_complete",
    "exact_bytecode_evidence_complete",
    "bounded_wallet_policy_review_complete",
    "repository_environment_protection_complete",
)
EXPECTED_CONFIGURATION: dict[str, Any] = {
    "minimum_request_confirmations": 3,
    "random_words": 1,
    "payment": "native",
    "fulfillment_deadline_seconds": 7_200,
    "solver_assignment_seconds": 120,
    "per_bounty_solver_enrollment_seconds": 0,
    "stake_activation_seconds": 604_800,
    "stake_unbonding_seconds": 604_800,
    "primary_response_seconds": 1_800,
    "primary_ranked_backups": 3,
    "appeal_filing_seconds": 14_400,
    "appeal_voting_seconds": 7_200,
    "bounty_verification_seconds": 86_400,
    "fast_path": "immediate_after_vrf_or_waiver_or_decisive_majority",
}


class DeploymentError(RuntimeError):
    pass


def redacted_command(command: Sequence[str]) -> str:
    redacted = list(command)
    for secret_flag in ("--private-key", "--rpc-url"):
        for index, item in enumerate(redacted[:-1]):
            if item == secret_flag:
                redacted[index + 1] = "[redacted]"
    return " ".join(redacted)


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
            f"command failed ({completed.returncode}): {redacted_command(command)}\n{completed.stdout[-6000:]}"
        )
    return completed.stdout.strip()


def normalize_address(value: object, label: str) -> str:
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
    match = CAST_UINT_RE.match(str(value).strip())
    if not match:
        raise DeploymentError(f"{label} is not an unsigned integer")
    return int(match.group(1), 0)


def load_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise DeploymentError(f"expected a JSON object in {path}")
    return value


def write_object(path: Path, value: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def validate_readiness_manifest(path: Path) -> dict[str, Any]:
    readiness = load_object(path)
    if readiness.get("schema") != "agent-bounties/standing-meta-v4-deployment-readiness-v1":
        raise DeploymentError("standing-meta-v4 readiness schema mismatch")
    if readiness.get("protocol_version") != "standing-meta-v4":
        raise DeploymentError("standing-meta-v4 protocol version mismatch")
    configuration = readiness.get("configuration")
    if not isinstance(configuration, dict):
        raise DeploymentError("standing-meta-v4 readiness configuration missing")
    mismatches = [
        name
        for name, expected in EXPECTED_CONFIGURATION.items()
        if configuration.get(name) != expected
    ]
    if mismatches:
        raise DeploymentError(f"standing-meta-v4 latency configuration drift: {', '.join(mismatches)}")
    required_components = readiness.get("required_components")
    if (
        not isinstance(required_components, list)
        or len(required_components) != len(EXPECTED_CANONICAL_COMPONENTS)
        or set(required_components) != set(EXPECTED_CANONICAL_COMPONENTS)
    ):
        raise DeploymentError("standing-meta-v4 required component schema drift")
    mainnet = readiness.get("networks", {}).get("base-mainnet", {})
    source_cap = mainnet.get("sponsorship_intent", {}).get("maximum_source_amount_base_units")
    if source_cap != MAINNET_SOURCE_USDC_CAP:
        raise DeploymentError("standing-meta-v4 mainnet source cap drift")
    return readiness


def network_config(chain_id: int) -> dict[str, Any]:
    if chain_id == BASE_MAINNET_CHAIN_ID:
        return {
            "network": "base-mainnet",
            "chain_id": chain_id,
            "rpc_default": BASE_MAINNET_RPC,
            "settlement_token": BASE_MAINNET_USDC,
            "vrf_coordinator": BASE_MAINNET_VRF,
            "key_hash": BASE_MAINNET_KEY_HASH,
            "base_child_factory": BASE_MAINNET_FACTORY,
        }
    if chain_id == BASE_SEPOLIA_CHAIN_ID:
        return {
            "network": "base-sepolia",
            "chain_id": chain_id,
            "rpc_default": BASE_SEPOLIA_RPC,
            "settlement_token": BASE_SEPOLIA_USDC,
            "vrf_coordinator": BASE_SEPOLIA_VRF,
            "key_hash": BASE_SEPOLIA_KEY_HASH,
            "base_child_factory": None,
        }
    raise DeploymentError(f"V4 deployment is restricted to Base mainnet or Base Sepolia, got {chain_id}")


class Foundry:
    def __init__(self, repo: Path, rpc_url: str, forge: str, cast: str) -> None:
        self.repo = repo
        self.contracts = repo / "contracts" / "base-escrow"
        self.rpc_url = rpc_url
        self.forge = forge
        self.cast = cast

    def command(self, *args: str, timeout: int = 300) -> str:
        return run([self.cast, *args], cwd=self.repo, timeout=timeout)

    def rpc(self, *args: str, timeout: int = 300) -> str:
        return self.command(*args, "--rpc-url", self.rpc_url, timeout=timeout)

    def chain_id(self) -> int:
        return parse_uint(self.rpc("chain-id"), "chain id")

    def code(self, address: str) -> str:
        return self.rpc("code", address).strip().lower()

    def balance(self, address: str) -> int:
        return parse_uint(self.rpc("balance", address), "native balance")

    def call(self, address: str, signature: str, *args: str) -> str:
        return self.rpc("call", address, signature, *args).strip()

    def address_for_key(self, private_key: str) -> str:
        return normalize_address(
            self.command("wallet", "address", "--private-key", private_key), "private-key address"
        )

    def keccak_text(self, value: str) -> str:
        completed = subprocess.run(
            [self.cast, "keccak"],
            cwd=self.repo,
            input=value,
            text=True,
            encoding="utf-8",
            errors="replace",
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=60,
            check=False,
        )
        if completed.returncode != 0:
            raise DeploymentError(f"cast keccak failed: {completed.stdout[-2000:]}")
        return require_bytes32(completed.stdout, "keccak result")

    def receipt(self, transaction_hash: str) -> dict[str, Any]:
        value = json.loads(self.rpc("receipt", transaction_hash, "--json"))
        if not isinstance(value, dict) or value.get("status") not in {"0x1", "0x01", 1}:
            raise DeploymentError(f"transaction was not successful: {transaction_hash}")
        return value

    def send(self, address: str, signature: str, private_key: str, *args: str, value: int = 0) -> dict[str, Any]:
        command = ["send", address, signature, *args, "--private-key", private_key]
        if value:
            command.extend(["--value", str(value)])
        command.append("--json")
        raw = self.rpc(*command, timeout=180)
        try:
            payload = json.loads(raw)
        except json.JSONDecodeError as error:
            raise DeploymentError("cast send did not return JSON") from error
        transaction_hash = payload.get("transactionHash") or payload.get("transaction_hash")
        if not transaction_hash and isinstance(payload.get("hash"), str):
            transaction_hash = payload["hash"]
        transaction_hash = require_bytes32(transaction_hash, "transaction hash")
        receipt = payload if payload.get("status") is not None else self.receipt(transaction_hash)
        if receipt.get("status") not in {"0x1", "0x01", 1}:
            raise DeploymentError(f"transaction reverted: {transaction_hash}")
        return {"transaction_hash": transaction_hash, "receipt": receipt}

    def create(self, source: str, private_key: str, constructor_args: Sequence[str]) -> dict[str, Any]:
        command = [
            self.forge,
            "create",
            "--broadcast",
            "--json",
            "--rpc-url",
            self.rpc_url,
            "--private-key",
            private_key,
            source,
        ]
        if constructor_args:
            command.extend(["--constructor-args", *constructor_args])
        output = run(command, cwd=self.contracts, timeout=600)
        match = CREATE_JSON_RE.search(output)
        if not match:
            raise DeploymentError("forge create did not return canonical deployment JSON")
        value = json.loads(match.group(0))
        result = {
            "deployer": normalize_address(value.get("deployer"), "deployment sender"),
            "address": normalize_address(value.get("deployedTo"), "deployed contract"),
            "transaction_hash": require_bytes32(value.get("transactionHash"), "deployment transaction"),
            "source": source,
        }
        self.receipt(result["transaction_hash"])
        wait_for_code(self, result["address"], source)
        result["runtime_code_hash"] = self.keccak_text(self.code(result["address"]))
        return result

    def runtime_size(self, source: str) -> int:
        runtime = run([self.forge, "inspect", source, "deployedBytecode"], cwd=self.contracts)
        raw = runtime.strip().removeprefix("0x")
        if not raw or len(raw) % 2:
            raise DeploymentError(f"invalid runtime bytecode for {source}")
        return len(raw) // 2

    def creation_size(self, source: str) -> int:
        creation = run([self.forge, "inspect", source, "bytecode"], cwd=self.contracts)
        raw = creation.strip().removeprefix("0x")
        if not raw or len(raw) % 2:
            raise DeploymentError(f"invalid creation bytecode for {source}")
        return len(raw) // 2

    def runtime_hash(self, source: str) -> str:
        runtime = run([self.forge, "inspect", source, "deployedBytecode"], cwd=self.contracts).strip()
        return self.keccak_text(runtime)

    def creation_hash(self, source: str) -> str:
        creation = run([self.forge, "inspect", source, "bytecode"], cwd=self.contracts).strip()
        return self.keccak_text(creation)


def wait_for_code(foundry: Foundry, address: str, label: str, timeout_seconds: float = 90) -> None:
    deadline = time.monotonic() + timeout_seconds
    while foundry.code(address) in {"0x", "0x0"}:
        if time.monotonic() >= deadline:
            raise DeploymentError(f"{label} has no runtime code at {address}")
        time.sleep(2)


def subscription_created_id(foundry: Foundry, receipt: Mapping[str, Any], coordinator: str) -> int:
    topic = foundry.keccak_text(SUBSCRIPTION_CREATED_EVENT)
    matching: list[Mapping[str, Any]] = []
    for item in receipt.get("logs", []):
        if not isinstance(item, dict) or str(item.get("address", "")).lower() != coordinator:
            continue
        topics = item.get("topics")
        if isinstance(topics, list) and len(topics) >= 2 and str(topics[0]).lower() == topic:
            matching.append(item)
    if len(matching) != 1:
        raise DeploymentError("createSubscription receipt did not contain exactly one SubscriptionCreated event")
    sub_id = int(require_bytes32(matching[0]["topics"][1], "subscription topic"), 16)
    if sub_id == 0:
        raise DeploymentError("created subscription id is zero")
    return sub_id


def parse_subscription(foundry: Foundry, coordinator: str, subscription_id: int) -> dict[str, Any]:
    output = foundry.call(
        coordinator,
        "getSubscription(uint256)(uint96,uint96,uint64,address,address[])",
        str(subscription_id),
    )
    lines = [line.strip() for line in output.splitlines() if line.strip()]
    if len(lines) < 5:
        raise DeploymentError("getSubscription returned an unexpected shape")
    consumers_text = lines[4]
    if not (consumers_text.startswith("[") and consumers_text.endswith("]")):
        raise DeploymentError("getSubscription consumers are malformed")
    body = consumers_text[1:-1].strip()
    consumers = [] if not body else [normalize_address(item.strip(), "subscription consumer") for item in body.split(",")]
    return {
        "link_balance": parse_uint(lines[0], "subscription LINK balance"),
        "native_balance": parse_uint(lines[1], "subscription native balance"),
        "request_count": parse_uint(lines[2], "subscription request count"),
        "owner": normalize_address(lines[3], "subscription owner"),
        "consumers": consumers,
    }


def artifact_evidence(foundry: Foundry) -> dict[str, Any]:
    evidence: dict[str, Any] = {}
    specs = dict(COMPONENT_SPECS)
    specs.update(
        {
            "standing_meta_child": "src/StandingMetaChildV4.sol:StandingMetaChildV4",
            "canonical_independent_child_verifier": (
                "src/CanonicalIndependentChildVerifierV4.sol:CanonicalIndependentChildVerifierV4"
            ),
            "onchain_terms_registry": "src/OnchainTermsRegistryV4.sol:OnchainTermsRegistryV4",
        }
    )
    for name, source in specs.items():
        size = foundry.runtime_size(source)
        creation_size = foundry.creation_size(source)
        if size > EIP170_RUNTIME_LIMIT:
            raise DeploymentError(f"{name} runtime exceeds EIP-170: {size}")
        if creation_size > EIP3860_INITCODE_LIMIT:
            raise DeploymentError(f"{name} creation code exceeds EIP-3860: {creation_size}")
        evidence[name] = {
            "source": source,
            "creation_size_bytes": creation_size,
            "compiled_creation_hash": foundry.creation_hash(source),
            "runtime_size_bytes": size,
            "runtime_margin_bytes": EIP170_RUNTIME_LIMIT - size,
            "initcode_margin_bytes": EIP3860_INITCODE_LIMIT - creation_size,
            "compiled_runtime_hash": foundry.runtime_hash(source),
        }
    return evidence


def build_plan(foundry: Foundry, readiness_path: Path) -> dict[str, Any]:
    chain_id = foundry.chain_id()
    config = network_config(chain_id)
    for label in ("settlement_token", "vrf_coordinator"):
        if foundry.code(config[label]) in {"0x", "0x0"}:
            raise DeploymentError(f"{label} has no runtime code at {config[label]}")
    if config["base_child_factory"] and foundry.code(config["base_child_factory"]) in {"0x", "0x0"}:
        raise DeploymentError("canonical mainnet child factory has no runtime code")
    readiness = validate_readiness_manifest(readiness_path)
    r4 = readiness.get("r4_evidence", {})
    r4_complete = all(r4.get(item) is True for item in REQUIRED_R4_GATES)
    return {
        "schema": "agent-bounties/standing-meta-v4-deployment-plan-v1",
        **config,
        "expected_keeper": EXPECTED_KEEPER,
        "keeper_native_balance_wei": foundry.balance(EXPECTED_KEEPER),
        "bounded_wallet": BOUNDED_WALLET,
        "bounded_wallet_source_usdc_cap": MAINNET_SOURCE_USDC_CAP,
        "r4_gates": {item: r4.get(item) is True for item in REQUIRED_R4_GATES},
        "mainnet_deploy_allowed": chain_id != BASE_MAINNET_CHAIN_ID or r4_complete,
        "artifacts": artifact_evidence(foundry),
        "evidence_boundary": "Read-only compiler and RPC evidence; not deployment, funding, rehearsal, readiness, or payment proof.",
    }


def checkpoint_base(config: Mapping[str, Any], deployer: str) -> dict[str, Any]:
    return {
        "schema": "agent-bounties/standing-meta-v4-deployment-v1",
        "network": config["network"],
        "chain_id": config["chain_id"],
        "deployer": deployer,
        "settlement_token": config["settlement_token"],
        "vrf_coordinator": config["vrf_coordinator"],
        "key_hash": config["key_hash"],
        "subscription_id": None,
        "components": {},
        "transactions": [],
        "status": "partial",
    }


def require_mainnet_release_gate(readiness_path: Path, acknowledged: bool) -> None:
    if not acknowledged:
        raise DeploymentError("mainnet deployment requires --acknowledge-r4-release-gate")
    r4 = validate_readiness_manifest(readiness_path).get("r4_evidence", {})
    gate_names = set(REQUIRED_R4_GATES)
    gate_names.update(name for name in r4 if name.endswith("_complete"))
    incomplete = [name for name in sorted(gate_names) if r4.get(name) is not True]
    if incomplete:
        raise DeploymentError(f"mainnet R4 gates are incomplete: {', '.join(incomplete)}")


def component_args(name: str, report: Mapping[str, Any]) -> list[str]:
    c = report["components"]
    token = report["settlement_token"]
    vrf = report["vrf_coordinator"]
    sub_id = str(report["subscription_id"])
    key_hash = report["key_hash"]
    deployer = report["deployer"]
    if name == "controller":
        return [deployer]
    if name == "stake_pool":
        return [token, c["controller"]["address"]]
    if name in {"verifier_sortition", "solver_sortition"}:
        return [vrf, c["controller"]["address"], sub_id, key_hash]
    if name == "appealable_verifier":
        return [token, c["controller"]["address"], c["verifier_sortition"]["address"]]
    if name == "standing_meta_child_factory":
        return [c["base_child_factory"]["address"], c["appealable_verifier"]["address"], deployer]
    if name == "standing_meta_parent_factory":
        return [
            c["base_child_factory"]["address"],
            c["standing_meta_child_factory"]["address"],
            c["controller"]["address"],
            c["appealable_verifier"]["address"],
        ]
    if name == "standing_meta_v4_bundle":
        return [
            c["base_child_factory"]["address"],
            c["controller"]["address"],
            c["stake_pool"]["address"],
            c["verifier_sortition"]["address"],
            c["solver_sortition"]["address"],
            c["appealable_verifier"]["address"],
            c["standing_meta_child_factory"]["address"],
            c["standing_meta_parent_factory"]["address"],
        ]
    raise DeploymentError(f"unknown component {name}")


def append_transaction(report: dict[str, Any], label: str, result: Mapping[str, Any]) -> None:
    report["transactions"].append({"label": label, "transaction_hash": result["transaction_hash"]})


def deploy(
    foundry: Foundry,
    output: Path,
    readiness_path: Path,
    acknowledge_mainnet: bool,
) -> dict[str, Any]:
    config = network_config(foundry.chain_id())
    validate_readiness_manifest(readiness_path)
    if config["chain_id"] == BASE_MAINNET_CHAIN_ID:
        require_mainnet_release_gate(readiness_path, acknowledge_mainnet)
    private_key = os.environ.get("BASE_KEEPER_PRIVATE_KEY", "").strip()
    if not private_key:
        raise DeploymentError("BASE_KEEPER_PRIVATE_KEY is required for deployment")
    deployer = foundry.address_for_key(private_key)
    if deployer != EXPECTED_KEEPER:
        raise DeploymentError(f"signer resolves to {deployer}, expected keeper {EXPECTED_KEEPER}")

    report = load_object(output) if output.exists() else checkpoint_base(config, deployer)
    if report.get("chain_id") != config["chain_id"] or report.get("deployer") != deployer:
        raise DeploymentError("deployment checkpoint belongs to another network or deployer")
    components = report["components"]

    if config["base_child_factory"]:
        components.setdefault(
            "base_child_factory",
            {
                "address": config["base_child_factory"],
                "transaction_hash": None,
                "source": "canonical-base-mainnet-agent-bounty-factory",
            },
        )
    elif "base_child_factory" not in components:
        components["base_child_factory"] = foundry.create(
            "src/AgentBountyFactory.sol:AgentBountyFactory", private_key, [config["settlement_token"]]
        )
        write_object(output, report)

    if not report.get("subscription_id"):
        result = foundry.send(config["vrf_coordinator"], "createSubscription()(uint256)", private_key)
        report["subscription_id"] = subscription_created_id(foundry, result["receipt"], config["vrf_coordinator"])
        append_transaction(report, "create_vrf_subscription", result)
        write_object(output, report)

    for name, source in COMPONENT_SPECS:
        if name == "standing_meta_v4_bundle":
            continue
        if name in components:
            wait_for_code(foundry, normalize_address(components[name]["address"], name), name)
            continue
        components[name] = foundry.create(source, private_key, component_args(name, report))
        write_object(output, report)

    child_factory = components["standing_meta_child_factory"]["address"]
    parent_factory = components["standing_meta_parent_factory"]["address"]
    child_configured = foundry.call(child_factory, "configured()(bool)").lower() == "true"
    if not child_configured:
        result = foundry.send(
            child_factory, "configureParentFactory(address)", private_key, parent_factory
        )
        append_transaction(report, "configure_child_factory", result)
        write_object(output, report)
    else:
        assert_call(foundry, child_factory, "parentFactory()(address)", parent_factory)

    controller = components["controller"]["address"]
    controller_configured = foundry.call(controller, "configured()(bool)").lower() == "true"
    if not controller_configured:
        result = foundry.send(
            controller,
            "configure(address,address,address,address,address)",
            private_key,
            components["stake_pool"]["address"],
            components["verifier_sortition"]["address"],
            components["solver_sortition"]["address"],
            components["appealable_verifier"]["address"],
            parent_factory,
        )
        append_transaction(report, "configure_protocol_controller", result)
        write_object(output, report)

    if "standing_meta_v4_bundle" not in components:
        bundle_source = dict(COMPONENT_SPECS)["standing_meta_v4_bundle"]
        components["standing_meta_v4_bundle"] = foundry.create(
            bundle_source, private_key, component_args("standing_meta_v4_bundle", report)
        )
        write_object(output, report)
    else:
        wait_for_code(
            foundry,
            normalize_address(components["standing_meta_v4_bundle"]["address"], "standing_meta_v4_bundle"),
            "standing_meta_v4_bundle",
        )

    subscription = parse_subscription(foundry, config["vrf_coordinator"], report["subscription_id"])
    for label in ("verifier_sortition", "solver_sortition"):
        consumer = components[label]["address"]
        if consumer not in subscription["consumers"]:
            result = foundry.send(
                config["vrf_coordinator"],
                "addConsumer(uint256,address)",
                private_key,
                str(report["subscription_id"]),
                consumer,
            )
            append_transaction(report, f"authorize_{label}", result)
            subscription = parse_subscription(foundry, config["vrf_coordinator"], report["subscription_id"])
            write_object(output, report)

    report["verification"] = verify_deployment(foundry, report)
    report["status"] = "deployed_consumers_authorized_unfunded"
    report["evidence_boundary"] = (
        "Confirmed runtime code, immutable wiring, subscription ownership, and both consumer entries. "
        "This is not a funded reserve, completed rehearsal, ready-to-earn status, or payment proof."
    )
    write_object(output, report)
    return report


def assert_call(foundry: Foundry, address: str, signature: str, expected: str, *args: str) -> None:
    observed = foundry.call(address, signature, *args).strip().lower()
    if observed != expected.lower():
        raise DeploymentError(f"{address} {signature} mismatch: expected {expected}, got {observed}")


def assert_uint_call(foundry: Foundry, address: str, signature: str, expected: int) -> None:
    observed = parse_uint(foundry.call(address, signature), signature)
    if observed != expected:
        raise DeploymentError(f"{address} {signature} mismatch: expected {expected}, got {observed}")


def canonical_component_addresses(foundry: Foundry, components: Mapping[str, Any]) -> dict[str, str]:
    required_report_components = {
        "controller",
        "stake_pool",
        "verifier_sortition",
        "solver_sortition",
        "appealable_verifier",
        "standing_meta_child_factory",
        "standing_meta_parent_factory",
        "standing_meta_v4_bundle",
    }
    missing = sorted(required_report_components - components.keys())
    if missing:
        raise DeploymentError(f"canonical component derivation missing: {', '.join(missing)}")
    addresses = {
        "anonymous_protocol_controller": normalize_address(
            components["controller"]["address"], "anonymous_protocol_controller"
        ),
        "anonymous_stake_pool": normalize_address(
            components["stake_pool"]["address"], "anonymous_stake_pool"
        ),
        "verifier_sortition": normalize_address(
            components["verifier_sortition"]["address"], "verifier_sortition"
        ),
        "solver_sortition": normalize_address(
            components["solver_sortition"]["address"], "solver_sortition"
        ),
        "appealable_verifier": normalize_address(
            components["appealable_verifier"]["address"], "appealable_verifier"
        ),
        "standing_meta_child_factory": normalize_address(
            components["standing_meta_child_factory"]["address"], "standing_meta_child_factory"
        ),
        "standing_meta_parent_factory": normalize_address(
            components["standing_meta_parent_factory"]["address"], "standing_meta_parent_factory"
        ),
        "standing_meta_v4_bundle": normalize_address(
            components["standing_meta_v4_bundle"]["address"], "standing_meta_v4_bundle"
        ),
    }
    parent_factory = addresses["standing_meta_parent_factory"]
    addresses["onchain_terms_registry"] = normalize_address(
        foundry.call(parent_factory, "termsRegistry()(address)"), "onchain_terms_registry"
    )
    addresses["canonical_independent_child_verifier"] = normalize_address(
        foundry.call(parent_factory, "verifierModule()(address)"),
        "canonical_independent_child_verifier",
    )
    if set(addresses) != set(EXPECTED_CANONICAL_COMPONENTS):
        raise DeploymentError("derived canonical component schema drift")
    return addresses


def verify_deployment(foundry: Foundry, report: Mapping[str, Any]) -> dict[str, Any]:
    components = report.get("components")
    if not isinstance(components, dict):
        raise DeploymentError("deployment report has no components")
    required = {"base_child_factory", *(name for name, _ in COMPONENT_SPECS)}
    missing = sorted(required - components.keys())
    if missing:
        raise DeploymentError(f"deployment components missing: {', '.join(missing)}")
    for name in required:
        wait_for_code(foundry, normalize_address(components[name]["address"], name), name)

    controller = components["controller"]["address"]
    pool = components["stake_pool"]["address"]
    verifier_sortition = components["verifier_sortition"]["address"]
    solver_sortition = components["solver_sortition"]["address"]
    appeal = components["appealable_verifier"]["address"]
    child_factory = components["standing_meta_child_factory"]["address"]
    parent_factory = components["standing_meta_parent_factory"]["address"]
    bundle = components["standing_meta_v4_bundle"]["address"]
    base_factory = components["base_child_factory"]["address"]
    token = report["settlement_token"]
    sub_id = int(report["subscription_id"])
    expected_network = network_config(foundry.chain_id())
    if normalize_address(token, "settlement token") != expected_network["settlement_token"]:
        raise DeploymentError("settlement token drift from pinned network configuration")
    if normalize_address(report["vrf_coordinator"], "VRF coordinator") != expected_network["vrf_coordinator"]:
        raise DeploymentError("VRF coordinator drift from pinned network configuration")
    if require_bytes32(report["key_hash"], "VRF key hash") != expected_network["key_hash"]:
        raise DeploymentError("VRF key hash drift from pinned network configuration")
    if expected_network["base_child_factory"] and normalize_address(
        base_factory, "base child factory"
    ) != expected_network["base_child_factory"]:
        raise DeploymentError("base child factory drift from pinned network configuration")
    wait_for_code(foundry, normalize_address(token, "settlement token"), "settlement token")
    wait_for_code(
        foundry,
        normalize_address(report["vrf_coordinator"], "VRF coordinator"),
        "VRF coordinator",
    )
    canonical_components = canonical_component_addresses(foundry, components)
    terms_registry = canonical_components["onchain_terms_registry"]
    verifier_module = canonical_components["canonical_independent_child_verifier"]
    for name, address in canonical_components.items():
        wait_for_code(foundry, address, name)

    assert_call(foundry, base_factory, "settlementToken()(address)", token)
    assert_call(foundry, controller, "configured()(bool)", "true")
    assert_call(foundry, controller, "stakePool()(address)", pool)
    assert_call(foundry, controller, "verifierSortition()(address)", verifier_sortition)
    assert_call(foundry, controller, "solverSortition()(address)", solver_sortition)
    assert_call(foundry, controller, "appealableVerifier()(address)", appeal)
    assert_call(foundry, controller, "standingMetaParentFactory()(address)", parent_factory)
    assert_call(foundry, pool, "settlementToken()(address)", token)
    assert_call(foundry, pool, "controller()(address)", controller)
    assert_uint_call(foundry, pool, "ACTIVATION_DELAY()(uint64)", 604_800)
    assert_uint_call(foundry, pool, "UNBONDING_DELAY()(uint64)", 604_800)
    for sortition in (verifier_sortition, solver_sortition):
        assert_call(foundry, sortition, "vrfCoordinator()(address)", report["vrf_coordinator"])
        assert_call(foundry, sortition, "controller()(address)", controller)
        if parse_uint(foundry.call(sortition, "subscriptionId()(uint256)"), "sortition subscription") != sub_id:
            raise DeploymentError("sortition subscription id drift")
        assert_call(foundry, sortition, "keyHash()(bytes32)", report["key_hash"])
        assert_uint_call(foundry, sortition, "REQUEST_CONFIRMATIONS()(uint16)", 3)
        assert_uint_call(foundry, sortition, "NUM_WORDS()(uint32)", 1)
        assert_uint_call(foundry, sortition, "FULFILLMENT_DEADLINE()(uint64)", 7_200)
    assert_call(foundry, appeal, "settlementToken()(address)", token)
    assert_call(foundry, appeal, "controller()(address)", controller)
    assert_call(foundry, appeal, "sortition()(address)", verifier_sortition)
    assert_uint_call(foundry, appeal, "RESPONSE_WINDOW()(uint64)", 1_800)
    assert_uint_call(foundry, appeal, "APPEAL_WINDOW()(uint64)", 14_400)
    assert_uint_call(foundry, appeal, "VOTING_WINDOW()(uint64)", 7_200)
    assert_uint_call(foundry, appeal, "REQUIRED_BOUNTY_VERIFICATION_WINDOW()(uint64)", 86_400)
    assert_call(foundry, child_factory, "configured()(bool)", "true")
    assert_call(foundry, child_factory, "baseChildFactory()(address)", base_factory)
    assert_call(foundry, child_factory, "appealableVerifier()(address)", appeal)
    assert_call(foundry, child_factory, "parentFactory()(address)", parent_factory)
    assert_uint_call(foundry, child_factory, "CHILD_VERIFICATION_WINDOW()(uint64)", 86_400)
    assert_call(foundry, parent_factory, "childFactory()(address)", base_factory)
    assert_call(foundry, parent_factory, "standingMetaChildFactory()(address)", child_factory)
    assert_call(foundry, parent_factory, "controller()(address)", controller)
    assert_call(foundry, parent_factory, "appealableVerifier()(address)", appeal)
    assert_call(foundry, parent_factory, "termsRegistry()(address)", terms_registry)
    assert_call(foundry, parent_factory, "verifierModule()(address)", verifier_module)
    assert_uint_call(foundry, parent_factory, "ASSIGNMENT_WINDOW()(uint64)", 120)
    assert_uint_call(foundry, parent_factory, "CHILD_VERIFICATION_WINDOW()(uint64)", 86_400)
    assert_call(foundry, terms_registry, "publisherAuthority()(address)", parent_factory)
    assert_call(foundry, verifier_module, "parentFactory()(address)", parent_factory)
    assert_call(foundry, verifier_module, "canonicalChildFactory()(address)", child_factory)
    assert_call(foundry, verifier_module, "settlementToken()(address)", token)
    assert_call(foundry, verifier_module, "termsRegistry()(address)", terms_registry)
    assert_call(foundry, verifier_module, "appealableVerifier()(address)", appeal)
    assert_uint_call(foundry, verifier_module, "MINIMUM_CHILD_TARGET()(uint256)", 1_000_000)
    assert_uint_call(foundry, verifier_module, "MINIMUM_PARENT_MARGIN()(uint256)", 1_000_000)
    assert_uint_call(foundry, verifier_module, "CHILD_SOLVER_REWARD()(uint256)", 990_000)
    assert_uint_call(foundry, verifier_module, "CHILD_VERIFIER_REWARD()(uint256)", 10_000)
    assert_uint_call(foundry, verifier_module, "CHILD_WORK_WINDOW()(uint64)", 604_800)
    assert_uint_call(foundry, verifier_module, "CHILD_VERIFICATION_WINDOW()(uint64)", 86_400)
    assert_call(foundry, bundle, "childFactory()(address)", base_factory)
    assert_call(foundry, bundle, "controller()(address)", controller)
    assert_call(foundry, bundle, "stakePool()(address)", pool)
    assert_call(foundry, bundle, "verifierSortition()(address)", verifier_sortition)
    assert_call(foundry, bundle, "solverSortition()(address)", solver_sortition)
    assert_call(foundry, bundle, "appealableVerifier()(address)", appeal)
    assert_call(foundry, bundle, "standingMetaChildFactory()(address)", child_factory)
    assert_call(foundry, bundle, "parentFactory()(address)", parent_factory)

    subscription = parse_subscription(foundry, report["vrf_coordinator"], sub_id)
    if subscription["owner"] != report["deployer"]:
        raise DeploymentError("subscription owner drift")
    expected_consumers = {verifier_sortition, solver_sortition}
    if set(subscription["consumers"]) != expected_consumers or len(subscription["consumers"]) != 2:
        raise DeploymentError("subscription must authorize exactly the two V4 sortition coordinators")
    return {
        "rpc_confirmed": True,
        "subscription": subscription,
        "canonical_component_addresses": canonical_components,
        "runtime_code_hashes": {
            name: foundry.keccak_text(foundry.code(address))
            for name, address in canonical_components.items()
        },
        "dependency_addresses": {
            "base_child_factory": normalize_address(base_factory, "base child factory"),
            "settlement_token": normalize_address(token, "settlement token"),
            "vrf_coordinator": normalize_address(report["vrf_coordinator"], "VRF coordinator"),
        },
        "dependency_runtime_code_hashes": {
            "base_child_factory": foundry.keccak_text(foundry.code(base_factory)),
            "settlement_token": foundry.keccak_text(foundry.code(token)),
            "vrf_coordinator": foundry.keccak_text(foundry.code(report["vrf_coordinator"])),
        },
        "consumers_authorized": True,
        "native_subscription_reserve_funded": subscription["native_balance"] > 0,
    }


def verify_mode(foundry: Foundry, deployment_path: Path, output: Path) -> dict[str, Any]:
    report = load_object(deployment_path)
    if report.get("chain_id") != foundry.chain_id():
        raise DeploymentError("deployment evidence chain does not match RPC")
    evidence = {
        "schema": "agent-bounties/standing-meta-v4-rpc-verification-v1",
        "network": report["network"],
        "chain_id": report["chain_id"],
        "deployment": str(deployment_path),
        "verification": verify_deployment(foundry, report),
        "ready_to_earn": False,
        "evidence_boundary": (
            "RPC proof of components and subscription state only. Readiness remains false until the funded reserve, "
            "full Base Sepolia rehearsal, independent review, pool-size, sponsorship, and monitoring gates pass."
        ),
    }
    write_object(output, evidence)
    return evidence


def prepare_owner_withdrawal(
    foundry: Foundry,
    readiness_path: Path,
    source_usdc_base_units: int,
) -> dict[str, Any]:
    readiness = validate_readiness_manifest(readiness_path)
    if foundry.chain_id() != BASE_MAINNET_CHAIN_ID:
        raise DeploymentError("bounded-wallet withdrawal preparation is Base mainnet only")
    cap = readiness["networks"]["base-mainnet"]["sponsorship_intent"][
        "maximum_source_amount_base_units"
    ]
    if source_usdc_base_units <= 0 or source_usdc_base_units > cap:
        raise DeploymentError("bounded-wallet withdrawal amount must be positive and within the seven USDC cap")
    for label, address in (("bounded wallet", BOUNDED_WALLET), ("native USDC", BASE_MAINNET_USDC)):
        if foundry.code(address) in {"0x", "0x0"}:
            raise DeploymentError(f"{label} has no runtime code")
    owner = normalize_address(foundry.call(BOUNDED_WALLET, "owner()(address)"), "bounded-wallet owner")
    if owner != EXPECTED_BOUNDED_WALLET_OWNER:
        raise DeploymentError("bounded-wallet owner drift")
    wallet_balance = parse_uint(
        foundry.call(BASE_MAINNET_USDC, "balanceOf(address)(uint256)", BOUNDED_WALLET),
        "bounded-wallet native-USDC balance",
    )
    if wallet_balance < source_usdc_base_units:
        raise DeploymentError("bounded-wallet native-USDC balance is below the requested amount")
    calldata = foundry.command(
        "calldata",
        "withdrawToken(address,address,uint256)",
        BASE_MAINNET_USDC,
        EXPECTED_KEEPER,
        str(source_usdc_base_units),
    ).strip().lower()
    if not calldata.startswith("0x") or len(calldata) != 2 + 4 * 2 + 32 * 2 * 3:
        raise DeploymentError("unexpected withdrawToken calldata")
    block_number = parse_uint(foundry.rpc("block-number"), "observation block")
    return {
        "schema": "agent-bounties/standing-meta-v4-owner-withdrawal-request-v1",
        "status": "unsigned_not_authorized",
        "network": "base-mainnet",
        "chain_id": BASE_MAINNET_CHAIN_ID,
        "observed_block": block_number,
        "bounded_wallet": BOUNDED_WALLET,
        "wallet_owner": owner,
        "token": BASE_MAINNET_USDC,
        "recipient": EXPECTED_KEEPER,
        "amount_base_units": source_usdc_base_units,
        "maximum_approved_base_units": cap,
        "wallet_balance_base_units": wallet_balance,
        "runtime_code_hashes": {
            "bounded_wallet": foundry.keccak_text(foundry.code(BOUNDED_WALLET)),
            "native_usdc": foundry.keccak_text(foundry.code(BASE_MAINNET_USDC)),
        },
        "unsigned_transaction": {
            "from": owner,
            "to": BOUNDED_WALLET,
            "chainId": BASE_MAINNET_CHAIN_ID,
            "value": "0x0",
            "data": calldata,
        },
        "ready_to_submit": False,
        "required_confirmation": (
            "The wallet owner must compare the chain, wallet, token, recipient, amount, and calldata in an "
            "independent signer at action time. The signer supplies nonce, fees, and final approval without "
            "exporting its private key."
        ),
        "evidence_boundary": (
            "This is an unsigned read-only request. It moves no value and proves no authorization, withdrawal, "
            "swap, VRF funding, deployment, settlement, or payment."
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("plan", "deploy", "verify", "prepare-owner-withdrawal"))
    parser.add_argument("--network", choices=("base-mainnet", "base-sepolia"), required=True)
    parser.add_argument("--rpc-url")
    parser.add_argument("--forge", default=os.environ.get("FORGE_BIN", "forge"))
    parser.add_argument("--cast", default=os.environ.get("CAST_BIN", "cast"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--deployment", type=Path)
    parser.add_argument("--readiness", type=Path, default=Path("deployments/standing-meta-v4-config.json"))
    parser.add_argument("--acknowledge-r4-release-gate", action="store_true")
    parser.add_argument("--source-usdc-base-units", type=int)
    args = parser.parse_args()

    repo = Path(__file__).resolve().parents[1]
    expected_chain = BASE_MAINNET_CHAIN_ID if args.network == "base-mainnet" else BASE_SEPOLIA_CHAIN_ID
    default_rpc = BASE_MAINNET_RPC if expected_chain == BASE_MAINNET_CHAIN_ID else BASE_SEPOLIA_RPC
    readiness = args.readiness if args.readiness.is_absolute() else repo / args.readiness
    foundry = Foundry(repo, args.rpc_url or default_rpc, args.forge, args.cast)
    if foundry.chain_id() != expected_chain:
        raise DeploymentError("selected network does not match RPC chain id")

    if args.mode == "plan":
        result = build_plan(foundry, readiness)
        write_object(args.output, result)
    elif args.mode == "deploy":
        result = deploy(foundry, args.output, readiness, args.acknowledge_r4_release_gate)
    elif args.mode == "verify":
        if args.deployment is None:
            raise DeploymentError("--deployment is required for verify mode")
        result = verify_mode(foundry, args.deployment, args.output)
    else:
        if args.network != "base-mainnet":
            raise DeploymentError("prepare-owner-withdrawal requires --network base-mainnet")
        if args.source_usdc_base_units is None:
            raise DeploymentError("--source-usdc-base-units is required for prepare-owner-withdrawal")
        result = prepare_owner_withdrawal(foundry, readiness, args.source_usdc_base_units)
        write_object(args.output, result)
    print(
        json.dumps(
            {
                "mode": args.mode,
                "network": args.network,
                "status": result.get("status", "planned" if args.mode == "plan" else "verified"),
                "output": str(args.output),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
