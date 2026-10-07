#!/usr/bin/env python3
"""Deploy AgentBountyFactoryV2 to Base mainnet with the launch fee terms, then verify it on-chain.

`plan` is a read-only preflight:
- the chain is Base mainnet and USDC has code;
- the factory address is predicted from the keeper's nonce;
- the deployment cost is estimated (execution plus L1 data fee);
- the keeper's balance is compared with three times that cost.

`deploy` signs with the shared keeper and runs only from the manual workflow on main. It refuses
when:
- the key is not the pinned keeper;
- a mainnet v2 deployment is already recorded;
- the keeper is underfunded;
- any on-chain check after the deployment disagrees with the launch terms.

Public RPC endpoints are load-balanced, so reads that follow the deployment are retried. Private
keys are never printed or written.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACTS = ROOT / "contracts/base-escrow"
RECORD = ROOT / "deployments/autonomous-v2-base-mainnet.json"
CONTRACT = "src/AgentBountyFactoryV2.sol:AgentBountyFactoryV2"
CHAIN_ID = 8453
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
FEE_BPS = 750
FEE_RECIPIENT = "0x884834e884d6e93462655a2820140ad03e6747bc"
KEEPER = "0xc26a630e85134ed30968735c8e7de4576cfa5dbc"
PROTOCOL_LABEL = "agent-bounties/autonomous-v2"
GAS_PRICE_ORACLE = "0x420000000000000000000000000000000000000F"
COST_SAFETY_MULTIPLIER = 3
READ_ATTEMPTS = 30
READ_DELAY_SECONDS = 2


class DeploymentError(RuntimeError):
    pass


def run(*args: str, cwd: Path = ROOT, secret: bool = False) -> str:
    result = subprocess.run(args, cwd=cwd, text=True, capture_output=True)
    if result.returncode != 0:
        # Never echo argv, which can carry a private key.
        detail = "" if secret else result.stderr.strip()[-500:]
        raise DeploymentError(f"{args[0]} {args[1] if len(args) > 1 else ''} failed {detail}")
    return result.stdout.strip()


def last_json_object(output: str, required_key: str) -> dict | None:
    """The last JSON object in `output` that has `required_key`, compact or pretty-printed."""
    decoder = json.JSONDecoder()
    for start in reversed([index for index, char in enumerate(output) if char == "{"]):
        try:
            value, _ = decoder.raw_decode(output[start:])
        except ValueError:
            continue
        if isinstance(value, dict) and required_key in value:
            return value
    return None


def address(value: str, label: str) -> str:
    match = re.search(r"0x[0-9a-fA-F]{40}", value)
    if not match:
        raise DeploymentError(f"{label} is not an address: {value!r}")
    return match.group(0).lower()


class Chain:
    def __init__(self, rpc: str) -> None:
        self.rpc = rpc

    def read(self, subcommand: str, *args: str) -> str:
        """A read-only call, retried while a lagging RPC node catches up."""
        # The RPC flag goes right after the subcommand: `estimate --create` treats later arguments
        # as constructor arguments.
        for attempt in range(READ_ATTEMPTS):
            try:
                return run("cast", subcommand, "--rpc-url", self.rpc, *args)
            except DeploymentError:
                if attempt == READ_ATTEMPTS - 1:
                    raise
                time.sleep(READ_DELAY_SECONDS)
        raise AssertionError("unreachable")

    def call(self, target: str, signature: str, *args: str) -> str:
        return self.read("call", target, signature, *args).split()[0]

    def code(self, target: str) -> str:
        return self.read("code", target).lower()

    def wait_for_code(self, target: str) -> str:
        for _ in range(READ_ATTEMPTS):
            code = self.code(target)
            if len(code) > 2:
                return code
            time.sleep(READ_DELAY_SECONDS)
        raise DeploymentError(f"no code at {target} after waiting for the RPC to catch up")

    def keeper_nonce(self) -> int:
        # The highest of a few reads, in case one reaches a node that is behind.
        reads = []
        for _ in range(3):
            reads.append(int(self.read("nonce", KEEPER, "--block", "pending")))
            time.sleep(1)
        return max(reads)


def init_code() -> str:
    creation = run("forge", "inspect", CONTRACT, "bytecode", cwd=CONTRACTS).lower()
    arguments = run("cast", "abi-encode", "constructor(address,uint16,address)", USDC, str(FEE_BPS), FEE_RECIPIENT)
    if not creation.startswith("0x") or len(creation) < 100:
        raise DeploymentError("forge did not return the factory creation bytecode")
    return creation + arguments.removeprefix("0x").lower()


def predicted_address(nonce: int) -> str:
    return address(run("cast", "compute-address", KEEPER, "--nonce", str(nonce)), "predicted factory")


def plan(chain: Chain) -> dict:
    chain_id = int(chain.read("chain-id"))
    if chain_id != CHAIN_ID:
        raise DeploymentError(f"the RPC is chain {chain_id}, not Base mainnet ({CHAIN_ID})")
    if len(chain.code(USDC)) <= 2:
        raise DeploymentError("USDC has no code on this RPC")
    code = init_code()
    nonce = chain.keeper_nonce()
    predicted = predicted_address(nonce)
    gas = int(chain.read("estimate", "--from", KEEPER, "--create", code))
    gas_price = int(chain.read("gas-price"))
    l1_fee = int(chain.call(GAS_PRICE_ORACLE, "getL1Fee(bytes)(uint256)", code))
    estimated_wei = gas * gas_price + l1_fee
    required_wei = estimated_wei * COST_SAFETY_MULTIPLIER
    keeper_wei = int(chain.read("balance", KEEPER))
    return {
        "schema_version": "agent-bounties/autonomous-v2-mainnet-deploy-plan-v1",
        "chain_id": chain_id,
        "block": int(chain.read("block-number")),
        "factory_contract": CONTRACT,
        "init_code_hash": run("cast", "keccak", code),
        "constructor": {"settlement_token": USDC, "platform_fee_bps": FEE_BPS, "platform_fee_recipient": FEE_RECIPIENT},
        "keeper": KEEPER,
        "keeper_nonce": nonce,
        "predicted_factory": predicted,
        "predicted_factory_has_code": len(chain.code(predicted)) > 2,
        "estimated_gas": gas,
        "gas_price_wei": gas_price,
        "l1_fee_wei": l1_fee,
        "estimated_cost_wei": estimated_wei,
        "required_keeper_wei": required_wei,
        "keeper_eth_wei": keeper_wei,
        "keeper_funded": keeper_wei >= required_wei,
        "already_recorded": RECORD.exists(),
    }


def verify(chain: Chain, factory: str) -> dict:
    """Read the deployed factory back and compare it with the launch terms. Raises on any mismatch."""
    factory_code = chain.wait_for_code(factory)
    protocol = run("cast", "keccak", PROTOCOL_LABEL).lower()
    implementation = address(chain.call(factory, "implementation()(address)"), "implementation")
    implementation_code = chain.wait_for_code(implementation)
    observed = {
        "settlement_token": address(chain.call(factory, "settlementToken()(address)"), "settlement token"),
        "platform_fee_bps": int(chain.call(factory, "platformFeeBps()(uint16)")),
        "platform_fee_recipient": address(chain.call(factory, "platformFeeRecipient()(address)"), "fee recipient"),
        "factory_protocol_version": chain.call(factory, "SUPPORTED_PROTOCOL_VERSION()(bytes32)").lower(),
        "implementation_protocol_version": chain.call(implementation, "protocolVersion()(bytes32)").lower(),
    }
    expected = {
        "settlement_token": USDC,
        "platform_fee_bps": FEE_BPS,
        "platform_fee_recipient": FEE_RECIPIENT,
        "factory_protocol_version": protocol,
        "implementation_protocol_version": protocol,
    }
    mismatches = {key: {"expected": expected[key], "observed": observed[key]}
                  for key in expected if observed[key] != expected[key]}
    if mismatches:
        raise DeploymentError(f"the deployed factory disagrees with the launch terms: {json.dumps(mismatches)}")
    return {
        **observed,
        "implementation": implementation,
        "factory_runtime_code_hash": run("cast", "keccak", factory_code),
        "implementation_runtime_code_hash": run("cast", "keccak", implementation_code),
    }


def deploy(chain: Chain, key: str) -> dict:
    signer = address(run("cast", "wallet", "address", "--private-key", key, secret=True), "signer")
    if signer != KEEPER:
        raise DeploymentError(f"the signing key is {signer}, not the pinned keeper {KEEPER}; refusing to deploy")
    if RECORD.exists():
        raise DeploymentError(f"{RECORD.relative_to(ROOT)} already records a mainnet v2 deployment; refusing to deploy again")
    preflight = plan(chain)
    if not preflight["keeper_funded"]:
        raise DeploymentError(
            f"the keeper holds {preflight['keeper_eth_wei']} wei; deploying needs {preflight['required_keeper_wei']}")
    if preflight["predicted_factory_has_code"]:
        raise DeploymentError(f"{preflight['predicted_factory']} already has code; refusing to deploy")
    output = run("forge", "create", CONTRACT, "--rpc-url", chain.rpc, "--private-key", key, "--broadcast", "--json",
                 "--nonce", str(preflight["keeper_nonce"]), "--constructor-args", USDC, str(FEE_BPS), FEE_RECIPIENT,
                 cwd=CONTRACTS, secret=True)
    report = last_json_object(output, "deployedTo") or {}
    factory = address(str(report.get("deployedTo", "")), "deployed factory")
    transaction = str(report.get("transactionHash", ""))
    if not re.fullmatch(r"0x[0-9a-fA-F]{64}", transaction):
        raise DeploymentError("forge create did not report the deployment transaction")
    if factory != preflight["predicted_factory"]:
        raise DeploymentError(f"deployed to {factory}, expected {preflight['predicted_factory']}")
    receipt = json.loads(chain.read("receipt", transaction, "--json"))
    if int(str(receipt["status"]), 16) != 1:
        raise DeploymentError(f"the deployment reverted: {transaction}")
    return {
        "schema_version": "agent-bounties/autonomous-v2-mainnet-deployment-v1",
        "network": "base-mainnet",
        "chain_id": CHAIN_ID,
        "factory": factory,
        "deployment_transaction": transaction,
        "deployment_block": int(str(receipt["blockNumber"]), 16),
        "deployer": KEEPER,
        "gas_used": int(str(receipt["gasUsed"]), 16),
        "preflight": preflight,
        "verification": verify(chain, factory),
        "configuration": {
            "BASE_MAINNET_BOUNTY_V2_FACTORY": factory,
            "BASE_MAINNET_BOUNTY_V2_PLATFORM_FEE_BPS": str(FEE_BPS),
            "BASE_MAINNET_BOUNTY_V2_PLATFORM_FEE_RECIPIENT": FEE_RECIPIENT,
            "BASE_INDEXER_PROTOCOL": "autonomous-v2",
            "BASE_INDEXER_NETWORK": "base-mainnet",
            "BASE_INDEXER_START_BLOCK": str(int(str(receipt["blockNumber"]), 16)),
        },
        "evidence_boundary": "The deployment and these on-chain reads prove the factory's immutable terms only. "
                             "No bounty is created or funded, and only canonical events prove funding or settlement.",
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["plan", "deploy"])
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", "https://mainnet.base.org"))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    chain = Chain(args.rpc_url)
    if args.mode == "plan":
        document = plan(chain)
    else:
        key = os.environ.get("BASE_KEEPER_PRIVATE_KEY", "").strip()
        if not key:
            print("BASE_KEEPER_PRIVATE_KEY is required", file=sys.stderr)
            return 2
        document = deploy(chain, key)
        document["configuration"]["BASE_MAINNET_BOUNTY_V2_IMPLEMENTATION"] = document["verification"]["implementation"]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, indent=2) + "\n")
    print(json.dumps(document, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
