#!/usr/bin/env python3
"""Regenerate crates/chain-base/tests/fixtures/autonomous-v2-loop.json from the compiled v2 contracts.

Runs contracts/base-escrow/script/CaptureAutonomousV2Fixture.s.sol on a throwaway local Anvil chain
that uses Base Sepolia's chain id (84532), so planner bounty ids and CREATE2 predictions can be
checked exactly. The capture token is placed at Base Sepolia's USDC address so the planner's
EIP-3009 domain matches it.

It then runs a gasless quorum loop driven only by `cli autonomous-v2-plan`: every typed-data payload
and calldata comes from crates/chain-base, every signature from `cast wallet sign --data` (the
standard EIP-712 wallet path), and a separate relayer sends every transaction. Requires anvil,
forge, cast, and cargo on PATH. Never touches a public network.
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
LABELS = {1: "paid_after_reject", 2: "fee_deferred_then_forwarded", 3: "contractor_gated", 4: "cancelled_and_refunded",
          5: "gasless_quorum", 6: "cancel_requested_then_expired"}
BASE_SEPOLIA_USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e"
# Anvil's default development keys. Never use them outside a local chain.
DEPLOYER_KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
POSTER_KEY = "0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba"
SOLVER_KEY = "0x92db14e403b83dfe3df233f83dfa3a0d7096f21ca9b0d6d6b8d88b2b4ec1564e"
VERIFIER_KEYS = ("0x4bbbf85ce3377467afe5d46f804f221813b2bb87f24d81f60f1fcdbf7cbf4356",
                 "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97")
RELAYER_KEY = "0x2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6"
ATTESTER_ADDRESS = "0x90F79bf6EB2c4f870365E785982E1f101E93b906"
FEE_RECIPIENT = "0xfEE0000000000000000000000000000000000fee"
FACTORY_FEE = {"platform_fee_bps": 750, "platform_fee_recipient": FEE_RECIPIENT}


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


def word(label: str) -> str:
    return run("cast", "keccak", label)


def address_of(key: str) -> str:
    return run("cast", "wallet", "address", "--private-key", key).lower()


class GaslessLoop:
    """Drives one quorum v2 bounty from creation to settlement through `cli autonomous-v2-plan`.

    Each recorded step holds the planning requests whose typed data was signed, the signer and
    signature, and the relay request whose planned calldata a separate relayer sent.
    """

    def __init__(self, cli: Path, factory: str, implementation: str, work: Path) -> None:
        self.cli, self.work = cli, work
        self.base = {"network": "base-sepolia", "factory_contract": factory, "implementation_contract": implementation}
        self.relayer = address_of(RELAYER_KEY)
        self.steps: list[dict[str, object]] = []

    def plan(self, request: dict[str, object]) -> dict[str, object]:
        body = json.dumps({**self.base, **request})
        output = subprocess.run([str(self.cli), "autonomous-v2-plan", "--request", "-"], input=body, text=True,
                                capture_output=True, check=True).stdout
        return json.loads(output)

    def authorize(self, request: dict[str, object], key: str, pointer: str | None = None) -> dict[str, object]:
        planned = self.plan(request)
        typed_data = planned[pointer] if pointer else planned
        path = self.work / "typed-data.json"
        path.write_text(json.dumps(typed_data))
        signature = run("cast", "wallet", "sign", "--data", "--from-file", str(path), "--private-key", key).lower()
        return {"request": request, "typed_data_field": pointer, "typed_data": typed_data,
                "signer": address_of(key), "signature": signature}

    @staticmethod
    def split(signature: object) -> dict[str, object]:
        raw = str(signature).removeprefix("0x")
        return {"v": int(raw[128:130], 16), "r": "0x" + raw[:64], "s": "0x" + raw[64:128]}

    def relay(self, name: str, authorizations: list[dict[str, object]], request: dict[str, object],
              pointer: str | None = None) -> dict[str, object]:
        planned = self.plan(request)
        intent = planned[pointer] if pointer else planned
        receipt = json.loads(run("cast", "send", "--rpc-url", RPC, "--private-key", RELAYER_KEY, "--json",
                                 intent["to"], "--data", intent["data"], quiet=True))
        if int(str(receipt["status"]), 16) != 1:
            raise RuntimeError(f"{name} relay reverted")
        self.steps.append({"step": name, "authorizations": authorizations,
                           "relay": {"request": request, "intent_field": pointer, "calldata": intent["data"],
                                     "to": intent["to"], "relayer": self.relayer,
                                     "transaction_hash": receipt["transactionHash"]}})
        return planned

    def run_loop(self) -> str:
        poster, solver = address_of(POSTER_KEY), address_of(SOLVER_KEY)
        verifiers = [address_of(key) for key in VERIFIER_KEYS]
        now = int(run("cast", "block", "latest", "--field", "timestamp", "--rpc-url", RPC))
        usdc = lambda amount: {"amount": amount, "currency": "USDC"}  # noqa: E731
        for wallet, amount in ((poster, 1_175_000), (solver, 100_000)):
            run("cast", "send", "--rpc-url", RPC, "--private-key", DEPLOYER_KEY, BASE_SEPOLIA_USDC,
                "mint(address,uint256)", wallet, str(amount), quiet=True)

        create = {
            "creator": poster, "solver_reward": usdc(1_000_000), "verifier_reward": usdc(100_000),
            "terms_hash": word("gasless terms"), "policy_hash": word("gasless policy"),
            "acceptance_criteria_hash": word("gasless criteria"), "benchmark_hash": word("gasless benchmark"),
            "evidence_schema_hash": word("gasless evidence schema"), "funding_deadline": now + 7 * 86_400,
            "claim_window_seconds": 86_400, "verification_window_seconds": 86_400,
            "verification_mode": "signed_quorum", "verifier_module": None, "verifier_reward_recipient": None,
            "verifiers": verifiers, "threshold": 2, "initial_funding": usdc(1_175_000),
            "creation_nonce": "0x" + format(5, "064x"),
        }
        funding = self.authorize({"action": "create", "create": create, "factory_fee": FACTORY_FEE}, POSTER_KEY,
                                 "eip3009_authorization")
        created = self.relay("authorized_create", [funding], {
            "action": "authorized_create", "create": create, "factory_fee": FACTORY_FEE,
            "signature": self.split(funding["signature"]), "relayer": self.relayer}, "relay_transaction")
        bounty, bounty_id = created["predicted_bounty_contract"], created["bounty_id"]

        claim = {"bounty_contract": bounty, "solver": solver, "claim_bond": usdc(100_000),
                 "claim_round": 1, "authorization_valid_before": now + 86_400}
        bond = self.authorize({"action": "claim", **claim}, SOLVER_KEY, "eip3009_authorization")
        self.relay("authorized_claim", [bond], {"action": "authorized_claim", **claim,
                                                "signature": self.split(bond["signature"]),
                                                "relayer": self.relayer}, "relay_transaction")

        submission = {"bounty_contract": bounty, "bounty_id": bounty_id, "round": 1, "solver": solver,
                      "submission_hash": word("gasless artifact"), "evidence_hash": word("gasless evidence"),
                      "policy_hash": create["policy_hash"], "deadline": now + 86_400}
        signed = self.authorize({"action": "submission_authorization", "submission": submission}, SOLVER_KEY)
        self.relay("submission_relay", [signed], {
            "action": "submission_relay", "bounty_contract": bounty,
            "submission_hash": submission["submission_hash"], "evidence_hash": submission["evidence_hash"],
            "deadline": submission["deadline"], "signature": signed["signature"], "relayer": self.relayer})

        verdicts = []
        for index, key in enumerate(VERIFIER_KEYS):
            attestation = {"bounty_contract": bounty, "bounty_id": bounty_id, "round": 1, "verifier": verifiers[index],
                           "submission_hash": submission["submission_hash"],
                           "evidence_hash": submission["evidence_hash"], "policy_hash": create["policy_hash"],
                           "passed": True, "response_hash": word(f"gasless verdict {index}"), "deadline": now + 86_400}
            verdicts.append(self.authorize({"action": "verification_attestation", "attestation": attestation}, key))
        attestations = [{"verifier": verdict["signer"], "passed": True,
                         "response_hash": verdict["request"]["attestation"]["response_hash"],
                         "deadline": verdict["request"]["attestation"]["deadline"], "signature": verdict["signature"]}
                        for verdict in verdicts]
        self.relay("attestation_settlement", verdicts, {
            "action": "attestation_settlement", "bounty_contract": bounty, "caller": self.relayer,
            "attestations": attestations})
        return bounty


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
        deployed_token = deploy(f"{script}:CaptureToken")
        token = BASE_SEPOLIA_USDC
        run("cast", "rpc", "anvil_setCode", token, run("cast", "code", deployed_token, "--rpc-url", RPC),
            "--rpc-url", RPC)
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
        factory = environment["CAPTURE_FACTORY"].lower()
        implementation = run("cast", "call", factory, "implementation()(address)", "--rpc-url", RPC).lower()
        subprocess.check_call(["cargo", "build", "--quiet", "-p", "cli"], cwd=ROOT)
        target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
        work = ROOT / "target" / "capture-autonomous-v2"
        work.mkdir(parents=True, exist_ok=True)
        gasless = GaslessLoop(target / "debug" / "cli", factory, implementation, work)
        gasless_bounty = gasless.run_loop()
        created_topic = run("cast", "keccak", "CanonicalBountyCreated(bytes32,address,address,bytes32,bytes32,bytes32)")

        def created_bounties() -> tuple[list[dict[str, object]], dict[str, str]]:
            logs = json.loads(run("cast", "logs", "--rpc-url", RPC, "--from-block", "0", "--json"))
            creations = [log for log in logs if log["topics"][0].lower() == created_topic.lower()]
            return logs, {LABELS[int(log["data"][2 + 128: 2 + 192], 16)]: "0x" + log["topics"][2][-40:].lower()
                          for log in creations}

        # The cancel request lets the round run out; its expiry returns the bond and cancels the bounty.
        _, bounties = created_bounties()
        run("cast", "rpc", "evm_increaseTime", str(86_400 + 2), "--rpc-url", RPC)
        run("cast", "rpc", "evm_mine", "--rpc-url", RPC)
        run("cast", "send", bounties["cancel_requested_then_expired"], "expireSubmission()", "--private-key",
            RELAYER_KEY, "--rpc-url", RPC, quiet=True)
        logs, bounties = created_bounties()
        creations = [log for log in logs if log["topics"][0].lower() == created_topic.lower()]
        bounty_ids = {LABELS[int(log["data"][2 + 128: 2 + 192], 16)]: log["topics"][1].lower() for log in creations}
        if bounties["gasless_quorum"] != gasless_bounty.lower():
            raise RuntimeError("gasless bounty was not created at the planner's predicted address")
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
    # The factory accepts only an authorization whose nonce is the bounty id.
    reference = run("cast", "calldata", CREATE_WITH_AUTH, creator, params.replace(" ", ""), verifiers, funding, nonce,
                    f"(0,{deadline},{bounty_ids['contractor_gated']},27,{r},{s})")
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
        "settlement_token": BASE_SEPOLIA_USDC,
        "gasless_loop": {"scenario": "gasless_quorum", "bounty": gasless_bounty.lower(),
                         "signer": "cast wallet sign --data (EIP-712)", "steps": gasless.steps},
        "logs": keep,
    }
    FIXTURE.write_text(json.dumps(fixture, indent=2) + "\n")
    print(f"wrote {FIXTURE.relative_to(ROOT)}: {len(keep)} logs, bounties {sorted(bounties)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
