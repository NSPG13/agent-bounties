#!/usr/bin/env python3
"""Regenerate crates/chain-base/tests/fixtures/autonomous-v2-loop.json from the compiled v2 contracts.

Runs contracts/base-escrow/script/CaptureAutonomousV2Fixture.s.sol on a throwaway local Anvil chain
that uses Base Sepolia's chain id (84532), so planner bounty ids and CREATE2 predictions can be
checked exactly. Requires anvil, forge, and cast on PATH. Never touches a public network.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACTS = ROOT / "contracts/base-escrow"
FIXTURE = ROOT / "crates/chain-base/tests/fixtures/autonomous-v2-loop.json"
RPC = "http://127.0.0.1:18547"
CHAIN_ID = "84532"
PARAMS = "(uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8,address,bytes32)"
CREATE = f"createBounty({PARAMS},address[],uint256,bytes32)"
CREATE_WITH_AUTH = f"createBountyWithAuthorization(address,{PARAMS},address[],uint256,bytes32,(uint256,uint256,bytes32,uint8,bytes32,bytes32))"
LABELS = {1: "paid_after_reject", 2: "fee_deferred_then_forwarded", 3: "contractor_gated", 4: "cancelled_and_refunded"}
# Anvil's default development keys (accounts 0 and 3). Never use them outside a local chain.
DEPLOYER_KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
ATTESTER_ADDRESS = "0x90F79bf6EB2c4f870365E785982E1f101E93b906"
FEE_RECIPIENT = "0xfEE0000000000000000000000000000000000fee"


def run(*args: str, cwd: Path = ROOT, quiet: bool = False) -> str:
    stderr = subprocess.DEVNULL if quiet else None
    return subprocess.check_output(args, cwd=cwd, text=True, stderr=stderr).strip()


def deploy(contract: str, *constructor_args: str) -> str:
    args = ["forge", "create", contract, "--rpc-url", RPC, "--private-key", DEPLOYER_KEY, "--broadcast"]
    if constructor_args:
        args += ["--constructor-args", *constructor_args]
    match = re.search(r"Deployed to: (0x[0-9a-fA-F]{40})", run(*args, cwd=CONTRACTS))
    if not match:
        raise RuntimeError(f"forge create did not report an address for {contract}")
    return match.group(1)


def main() -> int:
    anvil = subprocess.Popen(["anvil", "--port", RPC.rsplit(":", 1)[1], "--chain-id", CHAIN_ID, "--silent"])
    try:
        for _ in range(40):
            try:
                if run("cast", "chain-id", "--rpc-url", RPC, quiet=True) == CHAIN_ID:
                    break
            except subprocess.CalledProcessError:
                time.sleep(0.25)
        script = "script/CaptureAutonomousV2Fixture.s.sol"
        token = deploy(f"{script}:CaptureToken")
        environment = {
            "CAPTURE_TOKEN": token,
            "CAPTURE_MODULE": deploy(f"{script}:CaptureVerdictModule"),
            "CAPTURE_REGISTRY": deploy("src/ParticipantEligibilityRegistry.sol:ParticipantEligibilityRegistry", ATTESTER_ADDRESS),
            "CAPTURE_FACTORY": deploy("src/AgentBountyFactoryV2.sol:AgentBountyFactoryV2", token, "750", FEE_RECIPIENT),
        }
        subprocess.check_call(
            ["forge", "script", script, "--tc", "CaptureAutonomousV2Fixture", "--rpc-url", RPC, "--broadcast", "--slow"],
            cwd=CONTRACTS, env={**os.environ, **environment}, stdout=subprocess.DEVNULL,
        )
        logs = json.loads(run("cast", "logs", "--rpc-url", RPC, "--from-block", "0", "--json"))
        created_topic = run("cast", "keccak", "CanonicalBountyCreated(bytes32,address,address,bytes32,bytes32,bytes32)")
        creations = [log for log in logs if log["topics"][0].lower() == created_topic.lower()]
        factory = creations[0]["address"].lower()
        bounties = {LABELS[int(log["data"][2 + 128: 2 + 192], 16)]: "0x" + log["topics"][2][-40:].lower() for log in creations}
        implementation = run("cast", "call", factory, "implementation()(address)", "--rpc-url", RPC).lower()
        broadcast = json.loads((CONTRACTS / f"broadcast/CaptureAutonomousV2Fixture.s.sol/{CHAIN_ID}/run-latest.json").read_text())
        calls = [tx for tx in broadcast["transactions"] if (tx.get("function") or "").startswith("createBounty(")]
        calldata = {LABELS[int(tx["arguments"][3], 16)]: tx["transaction"]["input"] for tx in calls}
        creator = calls[0]["transaction"]["from"].lower()
    finally:
        anvil.terminate()
        anvil.wait()

    clean = lambda value: re.sub(r" \[[0-9.e+]+\]", "", value)  # noqa: E731 - cast numeric annotations
    params, verifiers, funding, nonce = (clean(line) for line in run("cast", "decode-calldata", CREATE, calldata["contractor_gated"]).split("\n"))
    deadline = params.strip("()").split(", ")[7]
    r, s = "0x" + "11" * 32, "0x" + "22" * 32
    reference = run("cast", "calldata", CREATE_WITH_AUTH, creator, params.replace(" ", ""), verifiers, funding, nonce,
                    f"(0,{deadline},{nonce},27,{r},{s})")
    keep = [{key: log[key] for key in ("address", "topics", "data", "transactionHash", "blockNumber", "logIndex")}
            for log in logs if log["address"].lower() in {factory, *bounties.values()}]
    fixture = {
        "source": "tools/capture_autonomous_v2_fixture.py: CaptureAutonomousV2Fixture.s.sol on local Anvil (chain id 84532); factory and bounty logs only",
        "chain_id": int(CHAIN_ID),
        "factory": factory,
        "implementation": implementation,
        "creator": creator,
        "platform_fee_bps": 750,
        "platform_fee_recipient": "0xfee0000000000000000000000000000000000fee",
        "solver_reward": 1_000_000,
        "verifier_reward": 100_000,
        "platform_fee": 75_000,
        "target_amount": 1_175_000,
        "bounties": bounties,
        "create_bounty_calldata": calldata,
        "authorized_create_reference": {"scenario": "contractor_gated", "v": 27, "r": r, "s": s, "calldata": reference,
                                        "oracle": "cast calldata (Foundry ABI encoder)"},
        "logs": keep,
    }
    FIXTURE.write_text(json.dumps(fixture, indent=2) + "\n")
    print(f"wrote {FIXTURE.relative_to(ROOT)}: {len(keep)} logs, bounties {sorted(bounties)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
