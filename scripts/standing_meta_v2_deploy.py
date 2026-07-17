#!/usr/bin/env python3
"""Deploy and prove the standing-meta-v2 components without exposing signer keys."""

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
BASE_MAINNET_FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
BASE_MAINNET_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
BASE_SEPOLIA_USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e"
MIN_MAINNET_KEEPER_WEI = 100_000_000_000_000
MIN_MAINNET_DEPLOY_WEI = 500_000_000_000_000
MIN_SEPOLIA_DEPLOY_WEI = 90_000_000_000_000
MIN_COMPLETION_TIMESTAMP_DELTA = 8
ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")
BYTES32_RE = re.compile(r"^0x[0-9a-fA-F]{64}$")
CREATE_JSON_RE = re.compile(r"\{\s*\"deployer\".*?\}\s*$", re.DOTALL)
CAST_UINT_RE = re.compile(r"^(0x[0-9a-fA-F]+|[0-9]+)(?:\s|$)")


class DeploymentError(RuntimeError):
    pass


def parse_cast_uint(value: object, label: str = "cast uint") -> int:
    text = str(value).strip()
    match = CAST_UINT_RE.match(text)
    if not match:
        raise DeploymentError(f"{label} is not an unsigned integer")
    return int(match.group(1), 0)


def normalize_address(value: object, label: str) -> str:
    text = str(value).strip()
    if not ADDRESS_RE.fullmatch(text):
        raise DeploymentError(f"{label} is not an EVM address")
    return text.lower()


def require_bytes32(value: object, label: str) -> str:
    text = str(value).strip().lower()
    if not BYTES32_RE.fullmatch(text):
        raise DeploymentError(f"{label} is not bytes32")
    return text


def run(
    command: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str] | None = None,
    timeout: int = 300,
) -> str:
    completed = subprocess.run(
        list(command),
        cwd=cwd,
        env=dict(env) if env is not None else None,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
        check=False,
    )
    if completed.returncode != 0:
        tail = completed.stdout[-6_000:].strip()
        raise DeploymentError(f"command failed with exit {completed.returncode}: {tail}")
    return completed.stdout.strip()


class Foundry:
    def __init__(self, repo: Path, rpc_url: str, forge: str, cast: str) -> None:
        self.repo = repo
        self.contracts = repo / "contracts" / "base-escrow"
        self.rpc_url = rpc_url
        self.forge = forge
        self.cast = cast

    def cast_run(self, *args: str) -> str:
        return run([self.cast, *args, "--rpc-url", self.rpc_url], cwd=self.repo)

    def chain_id(self) -> int:
        return parse_cast_uint(self.cast_run("chain-id"), "chain id")

    def address_for_key(self, private_key: str) -> str:
        return normalize_address(
            run(
                [self.cast, "wallet", "address", "--private-key", private_key],
                cwd=self.repo,
            ),
            "private-key address",
        )

    def balance(self, address: str) -> int:
        return parse_cast_uint(self.cast_run("balance", address), "native balance")

    def call(self, address: str, signature: str, *args: str) -> str:
        return self.cast_run("call", address, signature, *args).strip()

    def code(self, address: str) -> str:
        return self.cast_run("code", address).strip().lower()

    def receipt(self, transaction_hash: str) -> dict[str, Any]:
        value = json.loads(self.cast_run("receipt", transaction_hash, "--json"))
        if not isinstance(value, dict) or value.get("status") not in {"0x1", "0x01", 1}:
            raise DeploymentError(f"transaction was not successful: {transaction_hash}")
        return value

    def create(
        self,
        source: str,
        private_key: str,
        constructor_args: Sequence[str] = (),
    ) -> dict[str, str]:
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
        output = run(command, cwd=self.contracts)
        match = CREATE_JSON_RE.search(output)
        if not match:
            raise DeploymentError("forge create did not return its canonical JSON receipt")
        value = json.loads(match.group(0))
        result = {
            "deployer": normalize_address(value.get("deployer"), "forge deployer"),
            "contract": normalize_address(value.get("deployedTo"), "deployed contract"),
            "transaction_hash": require_bytes32(value.get("transactionHash"), "deployment transaction"),
            "source": source,
        }
        self.receipt(result["transaction_hash"])
        wait_for_runtime_code(self, result["contract"], "deployed contract")
        return result

    def script(self, source: str, env: Mapping[str, str]) -> None:
        run(
            [
                self.forge,
                "script",
                source,
                "--rpc-url",
                self.rpc_url,
                "--broadcast",
                "--slow",
                "-v",
            ],
            cwd=self.contracts,
            env=env,
            timeout=600,
        )


def wait_for_runtime_code(
    foundry: Foundry,
    address: str,
    label: str,
    *,
    timeout_seconds: float = 90,
    poll_interval_seconds: float = 2,
) -> str:
    deadline = time.monotonic() + timeout_seconds
    while True:
        code = foundry.code(address)
        if code not in {"0x", "0x0"}:
            return code
        if time.monotonic() >= deadline:
            raise DeploymentError(f"{label} has no runtime code after a confirmed receipt: {address}")
        time.sleep(poll_interval_seconds)


def require_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise DeploymentError(f"{name} is required")
    return value


def verifier_set_hash(foundry: Foundry, one: str, two: str) -> str:
    encoded = run(
        [foundry.cast, "abi-encode", "f(address[])", f"[{one},{two}]"],
        cwd=foundry.repo,
    )
    return require_bytes32(
        run([foundry.cast, "keccak", encoded.strip()], cwd=foundry.repo),
        "verifier set hash",
    )


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise DeploymentError(f"expected a JSON object in {path}")
    return value


def read_broadcast(path: Path) -> list[dict[str, Any]]:
    value = load_json(path)
    transactions = value.get("transactions")
    receipts = value.get("receipts")
    if not isinstance(transactions, list) or not isinstance(receipts, list) or len(transactions) != len(receipts):
        raise DeploymentError("Foundry broadcast transactions and receipts do not align")
    result: list[dict[str, Any]] = []
    for transaction, receipt in zip(transactions, receipts, strict=True):
        if not isinstance(transaction, dict) or not isinstance(receipt, dict) or receipt.get("status") != "0x1":
            raise DeploymentError("Foundry broadcast contains an unsuccessful transaction")
        transaction_hash = require_bytes32(transaction.get("hash"), "broadcast transaction")
        if receipt.get("transactionHash", "").lower() != transaction_hash:
            raise DeploymentError("Foundry broadcast receipt hash mismatch")
        result.append(
            {
                "transaction_hash": transaction_hash,
                "from": normalize_address(receipt.get("from"), "broadcast sender"),
                "to": normalize_address(receipt.get("to"), "broadcast recipient")
                if receipt.get("to")
                else None,
                "function": transaction.get("function"),
                "block_number": int(str(receipt.get("blockNumber")), 16),
                "logs": receipt.get("logs", []),
            }
        )
    return result


def wait_for_later_timestamp(
    foundry: Foundry,
    published_at: int,
    timeout_seconds: float = 90,
    poll_interval_seconds: float = 2,
) -> int:
    deadline = time.monotonic() + timeout_seconds
    required_timestamp = published_at + MIN_COMPLETION_TIMESTAMP_DELTA
    while time.monotonic() < deadline:
        observed = parse_cast_uint(
            foundry.cast_run("block", "latest", "--field", "timestamp"),
            "block timestamp",
        )
        if observed >= required_timestamp:
            return observed
        time.sleep(poll_interval_seconds)
    raise DeploymentError(
        f"Base did not reach the completion timestamp margin: required {required_timestamp}"
    )


def broadcast_path(foundry: Foundry, script_name: str, chain_id: int) -> Path:
    return foundry.contracts / "broadcast" / f"{script_name}.s.sol" / str(chain_id) / "run-latest.json"


def settlement_topic(foundry: Foundry) -> str:
    return require_bytes32(
        run(
            [
                foundry.cast,
                "keccak",
                "BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)",
            ],
            cwd=foundry.repo,
        ),
        "BountySettled topic",
    )


def verify_rehearsal(
    foundry: Foundry,
    deployments: Mapping[str, Mapping[str, Any]],
    final: Mapping[str, Any],
    complete_transactions: Sequence[Mapping[str, Any]],
    expected_verifier_set_hash: str,
) -> dict[str, Any]:
    parent = normalize_address(final.get("parent_bounty"), "parent bounty")
    child = normalize_address(final.get("child_bounty"), "child bounty")
    factory = normalize_address(deployments["factory"]["contract"], "rehearsal factory")
    module = normalize_address(deployments["verifier_module"]["contract"], "rehearsal module")
    token = normalize_address(deployments["token"]["contract"], "rehearsal token")
    exact_calls = {
        "factory_token": (
            normalize_address(foundry.call(factory, "settlementToken()(address)"), "factory token"),
            token,
        ),
        "module_factory": (
            normalize_address(foundry.call(module, "canonicalFactory()(address)"), "module factory"),
            factory,
        ),
        "module_verifier_set": (
            require_bytes32(foundry.call(module, "taskVerifierSetHash()(bytes32)"), "module verifier set"),
            expected_verifier_set_hash,
        ),
        "parent_status": (parse_cast_uint(foundry.call(parent, "status()(uint8)")), 4),
        "child_status": (parse_cast_uint(foundry.call(child, "status()(uint8)")), 4),
        "parent_canonical": (foundry.call(factory, "isCanonicalBounty(address)(bool)", parent).lower(), "true"),
        "child_canonical": (foundry.call(factory, "isCanonicalBounty(address)(bool)", child).lower(), "true"),
    }
    for label, (observed, expected) in exact_calls.items():
        if observed != expected:
            raise DeploymentError(f"rehearsal {label} mismatch: expected {expected}, got {observed}")

    topic = settlement_topic(foundry)
    settled_addresses: set[str] = set()
    for transaction in complete_transactions:
        for log in transaction.get("logs", []):
            if not isinstance(log, dict):
                continue
            topics = log.get("topics")
            if isinstance(topics, list) and topics and str(topics[0]).lower() == topic:
                settled_addresses.add(normalize_address(log.get("address"), "settlement log address"))
    if settled_addresses != {parent, child}:
        raise DeploymentError("rehearsal did not emit exactly one canonical settlement for parent and child")
    return {
        "parent_status": 4,
        "child_status": 4,
        "settlement_event_contracts": sorted(settled_addresses),
        "evidence_boundary": "Confirmed Base receipts and canonical BountySettled events prove this rehearsal.",
    }


def common_public_env() -> dict[str, str]:
    return {
        "PARTICIPANT_ATTESTER_ADDRESS": normalize_address(
            require_env("PARTICIPANT_ATTESTER_ADDRESS"), "participant attester"
        ),
        "REGRESSION_VERIFIER_ONE_ADDRESS": normalize_address(
            require_env("REGRESSION_VERIFIER_ONE_ADDRESS"), "verifier one"
        ),
        "REGRESSION_VERIFIER_TWO_ADDRESS": normalize_address(
            require_env("REGRESSION_VERIFIER_TWO_ADDRESS"), "verifier two"
        ),
    }


def deploy_sepolia(foundry: Foundry, output: Path) -> dict[str, Any]:
    if foundry.chain_id() != BASE_SEPOLIA_CHAIN_ID:
        raise DeploymentError("Sepolia deployment is pinned to Base chain 84532")
    private_key = require_env("BASE_KEEPER_PRIVATE_KEY")
    deployer = foundry.address_for_key(private_key)
    balance_before = foundry.balance(deployer)
    if balance_before < MIN_SEPOLIA_DEPLOY_WEI:
        raise DeploymentError(
            f"Base Sepolia keeper {deployer} needs at least {MIN_SEPOLIA_DEPLOY_WEI} wei; has {balance_before}"
        )
    public = common_public_env()
    set_hash = verifier_set_hash(
        foundry,
        public["REGRESSION_VERIFIER_ONE_ADDRESS"],
        public["REGRESSION_VERIFIER_TWO_ADDRESS"],
    )
    if foundry.code(BASE_SEPOLIA_USDC) in {"0x", "0x0"}:
        raise DeploymentError("canonical Base Sepolia USDC has no runtime code")
    if parse_cast_uint(
        foundry.call(BASE_SEPOLIA_USDC, "balanceOf(address)(uint256)", deployer),
        "Base Sepolia USDC balance",
    ) < 2_200_000:
        raise DeploymentError(f"Base Sepolia keeper {deployer} needs at least 2.2 canonical test USDC")
    deployments: dict[str, dict[str, Any]] = {
        "token": {
            "contract": BASE_SEPOLIA_USDC,
            "transaction_hash": None,
            "source": "canonical-base-sepolia-usdc",
        }
    }
    deployments["factory"] = foundry.create(
        "src/AgentBountyFactory.sol:AgentBountyFactory",
        private_key,
        [deployments["token"]["contract"]],
    )
    deployments["bundle"] = foundry.create(
        "src/StandingMetaV2Bundle.sol:StandingMetaV2Bundle",
        private_key,
        [
            deployments["factory"]["contract"],
            public["PARTICIPANT_ATTESTER_ADDRESS"],
            public["REGRESSION_VERIFIER_ONE_ADDRESS"],
            public["REGRESSION_VERIFIER_TWO_ADDRESS"],
        ],
    )
    bundle_address = deployments["bundle"]["contract"]
    linked_components = {
        "participant_registry": "participantRegistry()(address)",
        "terms_registry": "termsRegistry()(address)",
        "verifier_module": "verifierModule()(address)",
    }
    for name, signature in linked_components.items():
        component = normalize_address(foundry.call(bundle_address, signature), f"bundle {name}")
        wait_for_runtime_code(foundry, component, f"bundle {name}")
        deployments[name] = {
            "contract": component,
            "transaction_hash": deployments["bundle"]["transaction_hash"],
            "source": f"StandingMetaV2Bundle.{signature.split('(')[0]}",
        }

    target = foundry.repo / "target"
    target.mkdir(parents=True, exist_ok=True)
    prepare_path = target / "base-sepolia-standing-meta-v2-prepare.json"
    complete_path = target / "base-sepolia-standing-meta-v2-complete.json"
    script_env = dict(os.environ)
    script_env.update(public)
    script_env.update(
        {
            "REHEARSAL_TOKEN": deployments["token"]["contract"],
            "REHEARSAL_FACTORY": deployments["factory"]["contract"],
            "REHEARSAL_PARTICIPANT_REGISTRY": deployments["participant_registry"]["contract"],
            "REHEARSAL_TERMS_REGISTRY": deployments["terms_registry"]["contract"],
            "REHEARSAL_VERIFIER_MODULE": deployments["verifier_module"]["contract"],
            "REHEARSAL_PHASE": "prepare",
            "REHEARSAL_EVIDENCE_PATH": "../../target/base-sepolia-standing-meta-v2-prepare.json",
        }
    )
    script_source = (
        "script/BaseSepoliaStandingMetaV2Rehearsal.s.sol:BaseSepoliaStandingMetaV2Rehearsal"
    )
    foundry.script(script_source, script_env)
    prepare = load_json(prepare_path)
    prepare_transactions = read_broadcast(
        broadcast_path(foundry, "BaseSepoliaStandingMetaV2Rehearsal", BASE_SEPOLIA_CHAIN_ID)
    )
    observed_timestamp = wait_for_later_timestamp(foundry, int(prepare["terms_published_at"]))
    script_env.update(
        {
            "REHEARSAL_PHASE": "complete",
            "REHEARSAL_PARENT_BOUNTY": normalize_address(prepare["parent_bounty"], "prepared parent"),
            "REHEARSAL_EVIDENCE_PATH": "../../target/base-sepolia-standing-meta-v2-complete.json",
        }
    )
    foundry.script(script_source, script_env)
    final = load_json(complete_path)
    complete_transactions = read_broadcast(
        broadcast_path(foundry, "BaseSepoliaStandingMetaV2Rehearsal", BASE_SEPOLIA_CHAIN_ID)
    )
    verification = verify_rehearsal(foundry, deployments, final, complete_transactions, set_hash)
    report = {
        "schema": "agent-bounties/standing-meta-v2-rehearsal-v1",
        "network": "base-sepolia",
        "chain_id": BASE_SEPOLIA_CHAIN_ID,
        "deployer": deployer,
        "keeper_balance_before_wei": balance_before,
        "keeper_balance_after_wei": foundry.balance(deployer),
        "verifier_set_hash": set_hash,
        "deployments": deployments,
        "preparation": prepare,
        "completion": final,
        "preparation_transactions": prepare_transactions,
        "completion_transactions": complete_transactions,
        "timestamp_after_preparation": observed_timestamp,
        "verification": verification,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


def deploy_mainnet(foundry: Foundry, output: Path) -> dict[str, Any]:
    if foundry.chain_id() != BASE_MAINNET_CHAIN_ID:
        raise DeploymentError("mainnet deployment is pinned to Base chain 8453")
    private_key = require_env("BASE_KEEPER_PRIVATE_KEY")
    deployer = foundry.address_for_key(private_key)
    balance_before = foundry.balance(deployer)
    if balance_before < MIN_MAINNET_DEPLOY_WEI:
        raise DeploymentError(
            f"Base keeper {deployer} needs at least {MIN_MAINNET_DEPLOY_WEI} wei; has {balance_before}"
        )
    public = common_public_env()
    set_hash = verifier_set_hash(
        foundry,
        public["REGRESSION_VERIFIER_ONE_ADDRESS"],
        public["REGRESSION_VERIFIER_TWO_ADDRESS"],
    )
    target = foundry.repo / "target"
    target.mkdir(parents=True, exist_ok=True)
    raw_path = target / "base-mainnet-standing-meta-v2-deployment-raw.json"
    script_env = dict(os.environ)
    script_env.update(public)
    script_env["DEPLOYMENT_EVIDENCE_PATH"] = (
        "../../target/base-mainnet-standing-meta-v2-deployment-raw.json"
    )
    foundry.script("script/DeployStandingMetaV2.s.sol:DeployStandingMetaV2", script_env)
    raw = load_json(raw_path)
    transactions = read_broadcast(
        broadcast_path(foundry, "DeployStandingMetaV2", BASE_MAINNET_CHAIN_ID)
    )
    if len(transactions) != 1 or transactions[0]["to"] is not None:
        raise DeploymentError("mainnet policy components were not deployed atomically in one creation")
    expected = {
        "canonical_factory": BASE_MAINNET_FACTORY,
        "settlement_token": BASE_MAINNET_USDC,
        "participant_attester": public["PARTICIPANT_ATTESTER_ADDRESS"],
        "verifier_one": public["REGRESSION_VERIFIER_ONE_ADDRESS"],
        "verifier_two": public["REGRESSION_VERIFIER_TWO_ADDRESS"],
        "verifier_set_hash": set_hash,
    }
    for field, wanted in expected.items():
        observed = str(raw.get(field, "")).lower()
        if observed != wanted:
            raise DeploymentError(f"mainnet {field} mismatch: expected {wanted}, got {observed}")
    module = normalize_address(raw.get("verifier_module"), "mainnet verifier module")
    bundle = normalize_address(raw.get("bundle"), "mainnet deployment bundle")
    participant_registry = normalize_address(raw.get("participant_registry"), "mainnet participant registry")
    terms_registry = normalize_address(raw.get("terms_registry"), "mainnet terms registry")
    for label, address in {
        "deployment bundle": bundle,
        "verifier module": module,
        "participant registry": participant_registry,
        "terms registry": terms_registry,
    }.items():
        wait_for_runtime_code(foundry, address, label)
    onchain = {
        "bundle_factory": normalize_address(
            foundry.call(bundle, "canonicalFactory()(address)"), "bundle factory"
        ),
        "bundle_participant_registry": normalize_address(
            foundry.call(bundle, "participantRegistry()(address)"), "bundle participant registry"
        ),
        "bundle_terms_registry": normalize_address(
            foundry.call(bundle, "termsRegistry()(address)"), "bundle terms registry"
        ),
        "bundle_verifier_module": normalize_address(
            foundry.call(bundle, "verifierModule()(address)"), "bundle verifier module"
        ),
        "factory": normalize_address(foundry.call(module, "canonicalFactory()(address)"), "module factory"),
        "token": normalize_address(foundry.call(module, "settlementToken()(address)"), "module token"),
        "participant_registry": normalize_address(
            foundry.call(module, "participantRegistry()(address)"), "module participant registry"
        ),
        "terms_registry": normalize_address(foundry.call(module, "termsRegistry()(address)"), "module terms registry"),
        "verifier_set_hash": require_bytes32(
            foundry.call(module, "taskVerifierSetHash()(bytes32)"), "module verifier set"
        ),
        "verifier_threshold": parse_cast_uint(
            foundry.call(module, "taskVerifierThreshold()(uint8)"),
            "task verifier threshold",
        ),
    }
    expected_onchain: dict[str, object] = {
        "bundle_factory": BASE_MAINNET_FACTORY,
        "bundle_participant_registry": participant_registry,
        "bundle_terms_registry": terms_registry,
        "bundle_verifier_module": module,
        "factory": BASE_MAINNET_FACTORY,
        "token": BASE_MAINNET_USDC,
        "participant_registry": participant_registry,
        "terms_registry": terms_registry,
        "verifier_set_hash": set_hash,
        "verifier_threshold": 2,
    }
    if onchain != expected_onchain:
        raise DeploymentError(f"mainnet module immutable mismatch: expected {expected_onchain}, got {onchain}")
    balance_after = foundry.balance(deployer)
    if balance_after < MIN_MAINNET_KEEPER_WEI:
        raise DeploymentError("mainnet deployment depleted the keeper below its operational reserve")
    report = {
        "schema": "agent-bounties/standing-meta-v2-deployment-v1",
        "network": "base-mainnet",
        "chain_id": BASE_MAINNET_CHAIN_ID,
        "deployer": deployer,
        "keeper_balance_before_wei": balance_before,
        "keeper_balance_after_wei": balance_after,
        "components": raw,
        "transactions": transactions,
        "onchain": onchain,
        "evidence_boundary": "Confirmed Base deployment receipts and exact immutable getter checks.",
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("network", choices=("sepolia", "mainnet"))
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--forge", default=os.environ.get("FORGE_BIN", "forge"))
    parser.add_argument("--cast", default=os.environ.get("CAST_BIN", "cast"))
    parser.add_argument("--output", type=Path, required=True)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    repo = Path(__file__).resolve().parents[1]
    output = args.output if args.output.is_absolute() else repo / args.output
    foundry = Foundry(repo, args.rpc_url, args.forge, args.cast)
    try:
        report = deploy_sepolia(foundry, output) if args.network == "sepolia" else deploy_mainnet(foundry, output)
    except (DeploymentError, OSError, ValueError, KeyError, json.JSONDecodeError, subprocess.SubprocessError) as error:
        print(f"standing_meta_v2_deployment=failed network={args.network} error={error}")
        return 1
    print(
        f"standing_meta_v2_deployment=passed network={report['network']} "
        f"output={output}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
