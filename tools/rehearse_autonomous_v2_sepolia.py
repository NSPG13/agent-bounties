#!/usr/bin/env python3
"""Rehearse the full gasless autonomous-v2 loop on Base Sepolia with real test USDC.

The keeper (BASE_KEEPER_PRIVATE_KEY) deploys AgentBountyFactoryV2 with the launch fee terms. It
funds throwaway poster and solver wallets with test USDC and relays every transaction. The poster,
solver and two verifiers are fresh in-memory wallets that only sign `cli autonomous-v2-plan` typed
data with `cast wallet sign --data`; they never hold ETH. The tool records:
- the deployment;
- every relayed transaction and the canonical events it emitted;
- the fee recipient's balance change;
- the API and indexer settings for this factory.

Base Sepolia only. Private keys are never printed or written. Requires forge, cast and cargo on
PATH.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import secrets
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACTS = ROOT / "contracts/base-escrow"
CHAIN_ID = "84532"
USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e"
FEE_BPS = 750
FEE_RECIPIENT = "0x884834e884d6e93462655a2820140ad03e6747bc"
SOLVER_REWARD = 100_000  # 0.10 USDC
VERIFIER_REWARD = 20_000  # 0.02 USDC, split across a two-verifier quorum
PLATFORM_FEE = -(-SOLVER_REWARD * FEE_BPS // 10_000)
TARGET = SOLVER_REWARD + VERIFIER_REWARD + PLATFORM_FEE
MIN_KEEPER_WEI = 300_000_000_000_000  # 0.0003 ETH covers deployment, transfers and four relays
SECP256K1_ORDER = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
EVENTS = {
    "CanonicalBountyCreated(bytes32,address,address,bytes32,bytes32,bytes32)": "CanonicalBountyCreated",
    "FundingAdded(bytes32,address,uint256,uint256,uint256)": "FundingAdded",
    "BountyClaimed(bytes32,uint64,address,bytes32,bytes32,uint256,uint64)": "BountyClaimed",
    "SubmissionAdded(bytes32,uint64,address,bytes32,bytes32,uint64)": "SubmissionAdded",
    "BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)": "BountySettled",
    "PlatformFeePaid(bytes32,uint64,address,uint256)": "PlatformFeePaid",
}


class RehearsalError(RuntimeError):
    pass


def run(*args: str, cwd: Path = ROOT, secret: bool = False) -> str:
    result = subprocess.run(args, cwd=cwd, text=True, capture_output=True)
    if result.returncode != 0:
        # Never echo argv, which can carry a private key.
        detail = "" if secret else result.stderr.strip()[-500:]
        raise RehearsalError(f"{args[0]} {args[1] if len(args) > 1 else ''} failed {detail}")
    return result.stdout.strip()


class Chain:
    def __init__(self, rpc: str, keeper_key: str) -> None:
        self.rpc, self.keeper_key = rpc, keeper_key
        self.keeper = self.address(keeper_key)

    def address(self, key: str) -> str:
        return run("cast", "wallet", "address", "--private-key", key, secret=True).lower()

    @staticmethod
    def new_key() -> str:
        # A throwaway signer from the OS CSPRNG. It never depends on a Foundry output format.
        while True:
            value = secrets.randbits(256)
            if 0 < value < SECP256K1_ORDER:
                return f"0x{value:064x}"

    def usdc_balance(self, wallet: str) -> int:
        value = run("cast", "call", USDC, "balanceOf(address)(uint256)", wallet, "--rpc-url", self.rpc)
        return int(value.split()[0])

    def send(self, to: str, data: str) -> dict:
        receipt = json.loads(run("cast", "send", "--rpc-url", self.rpc, "--private-key", self.keeper_key, "--json",
                                 to, "--data", data, secret=True))
        if int(str(receipt["status"]), 16) != 1:
            raise RehearsalError(f"transaction to {to} reverted: {receipt['transactionHash']}")
        return receipt

    def transfer_usdc(self, wallet: str, amount: int) -> str:
        receipt = json.loads(run("cast", "send", "--rpc-url", self.rpc, "--private-key", self.keeper_key, "--json",
                                 USDC, "transfer(address,uint256)", wallet, str(amount), secret=True))
        return receipt["transactionHash"]

    def deploy_factory(self) -> tuple[str, str, str, int]:
        output = run("forge", "create", "src/AgentBountyFactoryV2.sol:AgentBountyFactoryV2", "--rpc-url", self.rpc,
                     "--private-key", self.keeper_key, "--broadcast", "--json", "--constructor-args", USDC,
                     str(FEE_BPS), FEE_RECIPIENT, cwd=CONTRACTS, secret=True)
        # `--json` is stable across Foundry releases; the human-readable output is not.
        report = last_json_object(output, "deployedTo")
        factory = str((report or {}).get("deployedTo", ""))
        transaction = str((report or {}).get("transactionHash", ""))
        if not re.fullmatch(r"0x[0-9a-fA-F]{40}", factory) or not re.fullmatch(r"0x[0-9a-fA-F]{64}", transaction):
            raise RehearsalError("forge create did not report the factory deployment")
        receipt = json.loads(run("cast", "receipt", transaction, "--rpc-url", self.rpc, "--json"))
        implementation = run("cast", "call", factory, "implementation()(address)", "--rpc-url", self.rpc)
        return factory.lower(), implementation.lower(), transaction, int(str(receipt["blockNumber"]), 16)


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


def event_names(receipt: dict) -> list[str]:
    topics = {run("cast", "keccak", signature): name for signature, name in EVENTS.items()}
    names = []
    for log in receipt["logs"]:
        topic = log["topics"][0].lower() if log["topics"] else ""
        names.append(topics.get(topic, topic[:10]))
    return names


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_SEPOLIA_RPC_URL", "https://sepolia.base.org"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--keeper-key-env",
        default="BASE_KEEPER_PRIVATE_KEY",
        help="environment variable holding the relaying key (fork mode passes a local Anvil key)",
    )
    args = parser.parse_args(argv)
    keeper_key = os.environ.get(args.keeper_key_env, "").strip()
    if not keeper_key:
        print(f"{args.keeper_key_env} is required", file=sys.stderr)
        return 2
    chain = Chain(args.rpc_url, keeper_key)
    if run("cast", "chain-id", "--rpc-url", chain.rpc) != CHAIN_ID:
        print("the RPC is not Base Sepolia (84532); refusing to run", file=sys.stderr)
        return 2
    keeper_wei = int(run("cast", "balance", chain.keeper, "--rpc-url", chain.rpc))
    keeper_usdc = chain.usdc_balance(chain.keeper)
    if keeper_wei < MIN_KEEPER_WEI or keeper_usdc < TARGET + VERIFIER_REWARD:
        print(json.dumps({
            "status": "keeper_underfunded",
            "keeper": chain.keeper,
            "eth_wei": keeper_wei,
            "required_eth_wei": MIN_KEEPER_WEI,
            "usdc": keeper_usdc,
            "required_usdc": TARGET + VERIFIER_REWARD,
            "next_action": "Send Base Sepolia ETH (and test USDC if short) to the keeper, then rerun.",
        }, indent=2))
        return 3

    subprocess.check_call(["cargo", "build", "--quiet", "-p", "cli"], cwd=ROOT)
    cli = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / "debug" / "cli"
    work = ROOT / "target" / "rehearse-autonomous-v2"
    work.mkdir(parents=True, exist_ok=True)

    fee_before = chain.usdc_balance(FEE_RECIPIENT)
    factory, implementation, deployment_tx, deployment_block = chain.deploy_factory()
    base = {"network": "base-sepolia", "factory_contract": factory, "implementation_contract": implementation}
    factory_fee = {"platform_fee_bps": FEE_BPS, "platform_fee_recipient": FEE_RECIPIENT}

    def plan(request: dict) -> dict:
        output = subprocess.run([str(cli), "autonomous-v2-plan", "--request", "-"], input=json.dumps({**base, **request}),
                                text=True, capture_output=True, check=True).stdout
        return json.loads(output)

    def sign(typed: dict, key: str) -> str:
        path = work / "typed-data.json"
        path.write_text(json.dumps(typed))
        try:
            return run("cast", "wallet", "sign", "--data", "--from-file", str(path), "--private-key", key,
                       secret=True).lower()
        finally:
            path.unlink(missing_ok=True)

    def split(signature: str) -> dict:
        raw = signature.removeprefix("0x")
        return {"v": int(raw[128:130], 16), "r": "0x" + raw[:64], "s": "0x" + raw[64:128]}

    poster_key, solver_key = chain.new_key(), chain.new_key()
    verifier_keys = [chain.new_key(), chain.new_key()]
    poster, solver = chain.address(poster_key), chain.address(solver_key)
    verifiers = [chain.address(key) for key in verifier_keys]
    funding_txs = [chain.transfer_usdc(poster, TARGET), chain.transfer_usdc(solver, VERIFIER_REWARD)]
    now = int(run("cast", "block", "latest", "--field", "timestamp", "--rpc-url", chain.rpc))
    usdc = lambda amount: {"amount": amount, "currency": "USDC"}  # noqa: E731
    word = lambda label: run("cast", "keccak", f"{label} {factory} {now}")  # noqa: E731
    create = {
        "creator": poster, "solver_reward": usdc(SOLVER_REWARD), "verifier_reward": usdc(VERIFIER_REWARD),
        "terms_hash": word("terms"), "policy_hash": word("policy"), "acceptance_criteria_hash": word("criteria"),
        "benchmark_hash": word("benchmark"), "evidence_schema_hash": word("evidence schema"),
        "funding_deadline": now + 86_400, "claim_window_seconds": 3_600, "verification_window_seconds": 3_600,
        "verification_mode": "signed_quorum", "verifier_module": None, "verifier_reward_recipient": None,
        "verifiers": verifiers, "threshold": 2, "initial_funding": usdc(TARGET), "creation_nonce": word("nonce"),
    }
    steps = []

    def relay(name: str, request: dict, pointer: str | None = None) -> dict:
        planned = plan(request)
        intent = planned[pointer] if pointer else planned
        receipt = chain.send(intent["to"], intent["data"])
        steps.append({"step": name, "transaction_hash": receipt["transactionHash"],
                      "block": int(str(receipt["blockNumber"]), 16), "events": event_names(receipt)})
        return planned

    typed = plan({"action": "create", "create": create, "factory_fee": factory_fee})["eip3009_authorization"]
    created = relay("authorized_create", {"action": "authorized_create", "create": create, "factory_fee": factory_fee,
                                          "signature": split(sign(typed, poster_key)), "relayer": chain.keeper},
                    "relay_transaction")
    bounty, bounty_id = created["predicted_bounty_contract"], created["bounty_id"]
    claim = {"bounty_contract": bounty, "solver": solver, "claim_bond": usdc(VERIFIER_REWARD),
             "authorization_nonce": word("claim bond"), "authorization_valid_before": now + 3_600}
    typed = plan({"action": "claim", **claim})["eip3009_authorization"]
    relay("authorized_claim", {"action": "authorized_claim", **claim, "signature": split(sign(typed, solver_key)),
                               "relayer": chain.keeper}, "relay_transaction")
    submission = {"bounty_contract": bounty, "bounty_id": bounty_id, "round": 1, "solver": solver,
                  "submission_hash": word("artifact"), "evidence_hash": word("evidence"),
                  "policy_hash": create["policy_hash"], "deadline": now + 3_600}
    typed = plan({"action": "submission_authorization", "submission": submission})
    relay("submission_relay", {"action": "submission_relay", "bounty_contract": bounty,
                               "submission_hash": submission["submission_hash"],
                               "evidence_hash": submission["evidence_hash"], "deadline": submission["deadline"],
                               "signature": sign(typed, solver_key), "relayer": chain.keeper})
    attestations = []
    for index, key in enumerate(verifier_keys):
        attestation = {"bounty_contract": bounty, "bounty_id": bounty_id, "round": 1, "verifier": verifiers[index],
                       "submission_hash": submission["submission_hash"],
                       "evidence_hash": submission["evidence_hash"], "policy_hash": create["policy_hash"],
                       "passed": True, "response_hash": word(f"verdict {index}"), "deadline": now + 3_600}
        typed = plan({"action": "verification_attestation", "attestation": attestation})
        attestations.append({"verifier": verifiers[index], "passed": True,
                             "response_hash": attestation["response_hash"], "deadline": attestation["deadline"],
                             "signature": sign(typed, key)})
    relay("attestation_settlement", {"action": "attestation_settlement", "bounty_contract": bounty,
                                     "caller": chain.keeper, "attestations": attestations})

    time.sleep(2)
    fee_after = chain.usdc_balance(FEE_RECIPIENT)
    expected = {
        "authorized_create": ["CanonicalBountyCreated", "FundingAdded"],
        "authorized_claim": ["BountyClaimed"],
        "submission_relay": ["SubmissionAdded"],
        "attestation_settlement": ["PlatformFeePaid", "BountySettled"],
    }
    missing = [f"{step['step']}:{name}" for step in steps for name in expected[step["step"]] if name not in step["events"]]
    evidence = {
        "schema_version": "agent-bounties/autonomous-v2-sepolia-rehearsal-v1",
        "network": "base-sepolia",
        "chain_id": int(CHAIN_ID),
        "factory": factory,
        "implementation": implementation,
        "deployment_transaction": deployment_tx,
        "deployment_block": deployment_block,
        "deployer": chain.keeper,
        "platform_fee_bps": FEE_BPS,
        "platform_fee_recipient": FEE_RECIPIENT,
        "economics": {"solver_reward": SOLVER_REWARD, "verifier_reward": VERIFIER_REWARD,
                      "platform_fee": PLATFORM_FEE, "target": TARGET},
        "participants": {"poster": poster, "solver": solver, "verifiers": verifiers, "relayer": chain.keeper},
        "participant_funding_transactions": funding_txs,
        "bounty": bounty,
        "bounty_id": bounty_id,
        "steps": steps,
        "fee_recipient_usdc_delta": fee_after - fee_before,
        "missing_events": missing,
        "status": "passed" if not missing and fee_after - fee_before == PLATFORM_FEE else "failed",
        "configuration": {
            "BASE_SEPOLIA_BOUNTY_V2_FACTORY": factory,
            "BASE_SEPOLIA_BOUNTY_V2_IMPLEMENTATION": implementation,
            "BASE_SEPOLIA_BOUNTY_V2_PLATFORM_FEE_BPS": str(FEE_BPS),
            "BASE_SEPOLIA_BOUNTY_V2_PLATFORM_FEE_RECIPIENT": FEE_RECIPIENT,
            "BASE_INDEXER_PROTOCOL": "autonomous-v2",
            "BASE_INDEXER_NETWORK": "base-sepolia",
            "BASE_INDEXER_START_BLOCK": str(deployment_block),
        },
        "evidence_boundary": "Only these canonical events prove funding and settlement; the fee delta is read from the USDC contract.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps({key: evidence[key] for key in ("status", "factory", "bounty", "fee_recipient_usdc_delta",
                                                      "missing_events")}, indent=2))
    return 0 if evidence["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
