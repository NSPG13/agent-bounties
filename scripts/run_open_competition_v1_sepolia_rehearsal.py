#!/usr/bin/env python3
"""Run the live Open Competition V1 Base Sepolia rehearsal.

The runner creates ephemeral actors, pauses for one bounded funding transfer to
the relayer, exercises the required scenarios, and writes a secret-free
manifest. Actor keys are kept in a temporary recovery file outside the repo and
deleted after a successful run.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import secrets
import tempfile
import time
from typing import Any

from eth_abi import decode, encode
from eth_account import Account
from eth_account.messages import encode_typed_data
from web3 import Web3
from web3.logs import DISCARD


PROTOCOL = "agent-bounties/open-competition-v1"
CHAIN_ID = 84532
NETWORK = "base-sepolia"
ADMIN = "0x884834e884d6e93462655a2820140ad03e6747bc"
USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e"
COMMITMENT_DOMAIN = Web3.keccak(text="agent-bounties/open-competition-v1-solution")
TRANSFER_TYPES = {
    "TransferWithAuthorization": [
        {"name": "from", "type": "address"},
        {"name": "to", "type": "address"},
        {"name": "value", "type": "uint256"},
        {"name": "validAfter", "type": "uint256"},
        {"name": "validBefore", "type": "uint256"},
        {"name": "nonce", "type": "bytes32"},
    ]
}
ACTOR_NAMES = (
    "creator",
    "failed_competitor",
    "passing_competitor",
    "expiring_competitor",
    "relayer",
)


class RehearsalError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RehearsalError(message)


def artifact(root: Path, source: str, contract: str) -> dict[str, Any]:
    path = root / "contracts" / "base-escrow" / "out" / source / f"{contract}.json"
    return json.loads(path.read_text(encoding="utf-8"))


def bytes32(value: Any) -> bytes:
    raw = bytes(value)
    require(len(raw) == 32, "expected bytes32")
    return raw


def hex32(value: Any) -> str:
    return "0x" + bytes32(value).hex()


def tx_hex(value: Any) -> str:
    return "0x" + bytes(value).hex()


def checksum(value: str) -> str:
    return Web3.to_checksum_address(value)


def event_rows(event: Any, receipt: Any) -> list[dict[str, Any]]:
    return [
        {
            "name": item["event"],
            "transaction_hash": tx_hex(item["transactionHash"]),
            "block_number": int(item["blockNumber"]),
            "log_index": int(item["logIndex"]),
        }
        for item in event.process_receipt(receipt, errors=DISCARD)
    ]


def revert_reason(error: Exception) -> str:
    values: list[Any] = [error, getattr(error, "args", ())]
    seen: set[int] = set()
    while values:
        value = values.pop()
        if id(value) in seen:
            continue
        seen.add(id(value))
        if isinstance(value, dict):
            values.extend(value.values())
        elif isinstance(value, (tuple, list)):
            values.extend(value)
        elif isinstance(value, str):
            if value.startswith("0x08c379a0") and len(value) >= 10 + 64:
                try:
                    return str(decode(["string"], bytes.fromhex(value[10:]))[0])
                except Exception:
                    pass
            if "execution reverted" in value.lower():
                return value
    return str(error)


class Runner:
    SOLVER_REWARD = 100_000
    VERIFIER_REWARD = 10_000
    TARGET = SOLVER_REWARD + VERIFIER_REWARD
    ADMIN_FUNDING_USDC = 500_000
    ADMIN_FUNDING_ETH_WEI = 500_000_000_000_000
    ACTOR_ETH_WEI = 50_000_000_000_000

    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.root = Path(__file__).resolve().parents[1]
        self.bundle = json.loads(args.bundle.read_text(encoding="utf-8"))
        self.w3 = Web3(Web3.HTTPProvider(args.rpc, request_kwargs={"timeout": 30}))
        require(self.w3.is_connected(), "Base Sepolia RPC is unavailable")
        require(self.w3.eth.chain_id == CHAIN_ID, "RPC is not Base Sepolia")

        factory_artifact = artifact(
            self.root, "OpenCompetitionBountyFactoryV1.sol", "OpenCompetitionBountyFactoryV1"
        )
        bounty_artifact = artifact(self.root, "OpenCompetitionBountyV1.sol", "OpenCompetitionBountyV1")
        verifier_artifact = artifact(self.root, "LeadingZeroWorkVerifier.sol", "LeadingZeroWorkVerifier")
        retry_artifact = artifact(
            self.root, "OpenCompetitionBountyV1.t.sol", "RetryableCompetitionVerifier"
        )
        self.bounty_abi = bounty_artifact["abi"]
        self.retry_bytecode = retry_artifact["bytecode"]["object"]

        self.factory_address = checksum(self.bundle["factory"])
        self.verifier_address = checksum(self.bundle["verifier_profile"]["verifier_address"])
        self.implementation_address = checksum(self.bundle["implementation"])
        self.usdc_address = checksum(USDC)
        self.factory = self.w3.eth.contract(address=self.factory_address, abi=factory_artifact["abi"])
        self.verifier = self.w3.eth.contract(address=self.verifier_address, abi=verifier_artifact["abi"])
        self.usdc = self.w3.eth.contract(
            address=self.usdc_address,
            abi=[
                {
                    "name": "transfer",
                    "type": "function",
                    "stateMutability": "nonpayable",
                    "inputs": [{"name": "to", "type": "address"}, {"name": "value", "type": "uint256"}],
                    "outputs": [{"type": "bool"}],
                },
                {
                    "name": "balanceOf",
                    "type": "function",
                    "stateMutability": "view",
                    "inputs": [{"name": "account", "type": "address"}],
                    "outputs": [{"type": "uint256"}],
                },
                {
                    "name": "name",
                    "type": "function",
                    "stateMutability": "view",
                    "inputs": [],
                    "outputs": [{"type": "string"}],
                },
                {
                    "name": "version",
                    "type": "function",
                    "stateMutability": "view",
                    "inputs": [],
                    "outputs": [{"type": "string"}],
                },
            ],
        )
        self.usdc_name = self.usdc.functions.name().call()
        self.usdc_version = self.usdc.functions.version().call()
        if args.recovery_file:
            self.recovery_path = args.recovery_file.resolve()
            recovery = json.loads(self.recovery_path.read_text(encoding="utf-8"))
            require(recovery.get("network") == NETWORK, "recovery file network mismatch")
            recovered = recovery.get("actors", {})
            require(set(recovered) == set(ACTOR_NAMES), "recovery file actor set mismatch")
            self.actors = {
                name: Account.from_key(recovered[name]["key_hex"])
                for name in ACTOR_NAMES
            }
            for name, actor in self.actors.items():
                require(
                    actor.address.lower() == recovered[name]["address"].lower(),
                    f"recovery file address mismatch for {name}",
                )
            self.resuming = True
        else:
            self.actors = {name: Account.create(secrets.token_bytes(32)) for name in ACTOR_NAMES}
            self.recovery_path = self._write_recovery_file()
            self.resuming = False
        self.receipts: dict[str, Any] = {}
        self.recovery_cleanup: dict[str, Any] | None = None

    def _write_recovery_file(self) -> Path:
        fd, raw_path = tempfile.mkstemp(prefix="agent-bounties-ocv1-recovery-", suffix=".json")
        path = Path(raw_path)
        os.close(fd)
        path.write_text(
            json.dumps(
                {
                    "network": NETWORK,
                    "warning": "Ephemeral Base Sepolia rehearsal keys. Delete after reconciliation.",
                    "actors": {
                        name: {"address": actor.address.lower(), "key_hex": actor.key.hex()}
                        for name, actor in self.actors.items()
                    },
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        try:
            os.chmod(path, 0o600)
        except OSError:
            pass
        return path

    def funding_handshake(self) -> None:
        request = {
            "schema_version": "agent-bounties/open-competition-v1-rehearsal-funding-v1",
            "network": NETWORK,
            "chain_id": CHAIN_ID,
            "from": ADMIN,
            "recipient": self.actors["relayer"].address.lower(),
            "eth_wei": self.ADMIN_FUNDING_ETH_WEI,
            "usdc_token": USDC,
            "usdc_base_units": self.ADMIN_FUNDING_USDC,
            "evidence_boundary": "This request funds ephemeral testnet actors only and does not fund a bounty.",
        }
        self.args.funding_request.parent.mkdir(parents=True, exist_ok=True)
        self.args.funding_request.write_text(json.dumps(request, indent=2) + "\n", encoding="utf-8")
        print(json.dumps({"actors": {name: actor.address.lower() for name, actor in self.actors.items()}}))
        print(json.dumps({"funding_request": str(self.args.funding_request), **request}))
        print(json.dumps({"temporary_recovery_file": str(self.recovery_path)}))
        if not self.resuming:
            input("Fund the relayer with the exact ETH and USDC amounts, then press Enter: ")
        relayer = self.actors["relayer"]
        relayer_eth = self.w3.eth.get_balance(relayer.address)
        relayer_usdc = self.usdc.functions.balanceOf(relayer.address).call()
        if self.resuming:
            require(relayer_eth > 0, "recovered relayer has no ETH")
            require(relayer_usdc > 0, "recovered relayer has no USDC")
        else:
            require(relayer_eth >= self.ADMIN_FUNDING_ETH_WEI, "relayer ETH funding is missing")
            require(relayer_usdc >= self.ADMIN_FUNDING_USDC, "relayer USDC funding is missing")

    def fees(self) -> dict[str, int]:
        block = self.w3.eth.get_block("latest")
        base = int(block.get("baseFeePerGas", 1_000_000))
        try:
            priority = max(int(self.w3.eth.max_priority_fee), 1_000_000)
        except Exception:
            priority = 1_000_000
        return {"maxFeePerGas": base * 2 + priority, "maxPriorityFeePerGas": priority}

    def send(self, actor: Any, *, to: str | None, data: str = "0x", value: int = 0,
             gas: int | None = None, allow_revert: bool = False) -> Any:
        transaction: dict[str, Any] = {
            "chainId": CHAIN_ID,
            "from": actor.address,
            "nonce": self.w3.eth.get_transaction_count(actor.address, "pending"),
            "value": value,
            "data": data,
            **self.fees(),
        }
        if to is not None:
            transaction["to"] = checksum(to)
        if gas is None:
            estimate = self.w3.eth.estimate_gas(transaction)
            gas = max(estimate + estimate // 4, 21_000)
        transaction["gas"] = gas
        signed = actor.sign_transaction(transaction)
        tx_hash = self.w3.eth.send_raw_transaction(signed.raw_transaction)
        receipt = self.w3.eth.wait_for_transaction_receipt(tx_hash, timeout=180, poll_latency=1)
        if not allow_revert:
            require(receipt.status == 1, f"transaction {tx_hex(tx_hash)} reverted")
        return self.wait_confirmations(receipt)

    def send_function(self, actor: Any, function: Any, *, gas: int | None = None,
                      allow_revert: bool = False) -> Any:
        return self.send(
            actor,
            to=function.address,
            data=function._encode_transaction_data(),
            gas=gas,
            allow_revert=allow_revert,
        )

    def wait_confirmations(self, receipt: Any, confirmations: int = 3) -> Any:
        deadline = time.time() + 180
        tx_hash = receipt.transactionHash
        while True:
            require(time.time() < deadline, "confirmation wait timed out")
            try:
                refreshed = self.w3.eth.get_transaction_receipt(tx_hash)
            except Exception:
                time.sleep(1)
                continue
            target = int(refreshed.blockNumber) + confirmations
            if self.w3.eth.block_number < target:
                time.sleep(1)
                continue
            canonical = self.w3.eth.get_block(refreshed.blockNumber)
            if Web3.to_hex(canonical.hash).lower() == Web3.to_hex(refreshed.blockHash).lower():
                return refreshed
            time.sleep(1)

    def distribute(self) -> None:
        relayer = self.actors["relayer"]
        for name in ("creator", "failed_competitor", "passing_competitor", "expiring_competitor"):
            current = self.w3.eth.get_balance(self.actors[name].address)
            require(current <= self.ACTOR_ETH_WEI, f"{name} ETH exceeds the rehearsal allocation")
            if current < self.ACTOR_ETH_WEI:
                self.send(
                    relayer,
                    to=self.actors[name].address,
                    value=self.ACTOR_ETH_WEI - current,
                )
        allocations = {
            "creator": self.TARGET * 3,
            "failed_competitor": self.VERIFIER_REWARD * 3,
            "passing_competitor": self.VERIFIER_REWARD,
            "expiring_competitor": self.VERIFIER_REWARD * 2,
        }
        for name, amount in allocations.items():
            current = self.usdc.functions.balanceOf(self.actors[name].address).call()
            require(current <= amount, f"{name} USDC exceeds the rehearsal allocation")
            if current < amount:
                self.send_function(
                    relayer,
                    self.usdc.functions.transfer(self.actors[name].address, amount - current),
                )
        for name, amount in allocations.items():
            require(self.usdc.functions.balanceOf(self.actors[name].address).call() == amount, f"{name} USDC allocation mismatch")

    def reclaim_partial_settlement(self, bounty_address: str) -> None:
        """Recover the losing bond and restore actor allocations after an interrupted run."""
        bounty_address = checksum(bounty_address)
        require(
            self.factory.functions.isCanonicalCompetition(bounty_address).call(),
            "recovery bounty is not factory canonical",
        )
        bounty = self.w3.eth.contract(address=bounty_address, abi=self.bounty_abi)
        require(
            bounty.functions.winner().call() == self.actors["passing_competitor"].address,
            "recovery bounty winner mismatch",
        )
        relayer = self.actors["relayer"]
        expiring = self.actors["expiring_competitor"]
        transactions: list[str] = []
        current_eth = self.w3.eth.get_balance(expiring.address)
        if current_eth < self.ACTOR_ETH_WEI:
            receipt = self.send(
                relayer,
                to=expiring.address,
                value=self.ACTOR_ETH_WEI - current_eth,
            )
            transactions.append(tx_hex(receipt.transactionHash))
        bounty_balance = self.usdc.functions.balanceOf(bounty_address).call()
        require(
            bounty_balance in (0, self.VERIFIER_REWARD),
            "recovery bounty has an unexpected token balance",
        )
        if bounty_balance == self.VERIFIER_REWARD:
            receipt = self.send_function(expiring, bounty.functions.withdrawEntryBond())
            transactions.append(tx_hex(receipt.transactionHash))
        require(self.usdc.functions.balanceOf(bounty_address).call() == 0, "recovery bounty was not drained")

        allocations = {
            "creator": self.TARGET * 3,
            "failed_competitor": self.VERIFIER_REWARD * 3,
            "passing_competitor": self.VERIFIER_REWARD,
            "expiring_competitor": self.VERIFIER_REWARD * 2,
        }
        for name, target in allocations.items():
            actor = self.actors[name]
            current = self.usdc.functions.balanceOf(actor.address).call()
            if current > target:
                receipt = self.send_function(
                    actor,
                    self.usdc.functions.transfer(relayer.address, current - target),
                )
                transactions.append(tx_hex(receipt.transactionHash))
        for name, target in allocations.items():
            actor = self.actors[name]
            current = self.usdc.functions.balanceOf(actor.address).call()
            if current < target:
                receipt = self.send_function(
                    relayer,
                    self.usdc.functions.transfer(actor.address, target - current),
                )
                transactions.append(tx_hex(receipt.transactionHash))
        for name, target in allocations.items():
            require(
                self.usdc.functions.balanceOf(self.actors[name].address).call() == target,
                f"{name} recovery allocation mismatch",
            )
        self.recovery_cleanup = {
            "superseded_bounty": bounty_address.lower(),
            "transactions": transactions,
            "losing_bond_recovered": True,
            "actor_allocations_restored": True,
        }

    def sign_authorization(self, actor: Any, to: str, value: int, nonce: bytes,
                           valid_before: int) -> tuple[int, int, bytes, bytes, bytes]:
        valid_after = 0
        message = encode_typed_data(
            full_message={
                "types": {"EIP712Domain": [
                    {"name": "name", "type": "string"},
                    {"name": "version", "type": "string"},
                    {"name": "chainId", "type": "uint256"},
                    {"name": "verifyingContract", "type": "address"},
                ], **TRANSFER_TYPES},
                "primaryType": "TransferWithAuthorization",
                "domain": {
                    "name": self.usdc_name,
                    "version": self.usdc_version,
                    "chainId": CHAIN_ID,
                    "verifyingContract": self.usdc_address,
                },
                "message": {
                    "from": actor.address,
                    "to": checksum(to),
                    "value": value,
                    "validAfter": valid_after,
                    "validBefore": valid_before,
                    "nonce": bytes32(nonce),
                },
            }
        )
        signed = actor.sign_message(message)
        return valid_after, valid_before, bytes32(nonce), signed.v, signed.r.to_bytes(32, "big"), signed.s.to_bytes(32, "big")

    def params(self, verifier: str, tag: str, *, max_entries: int,
               competition_window: int, reveal_window: int) -> tuple[Any, ...]:
        now = int(self.w3.eth.get_block("latest").timestamp)
        hashes = [Web3.keccak(text=f"{self.bundle['source_commit']}:{tag}:{name}") for name in (
            "terms", "policy", "criteria", "benchmark", "evidence-schema"
        )]
        return (
            self.SOLVER_REWARD,
            self.VERIFIER_REWARD,
            *hashes,
            now + 3600,
            competition_window,
            reveal_window,
            max_entries,
            checksum(verifier),
            self.actors["relayer"].address,
        )

    def create(self, tag: str, verifier: str, *, max_entries: int,
               competition_window: int, reveal_window: int) -> tuple[Any, Any, tuple[Any, ...]]:
        creator = self.actors["creator"]
        relayer = self.actors["relayer"]
        params = self.params(
            verifier,
            tag,
            max_entries=max_entries,
            competition_window=competition_window,
            reveal_window=reveal_window,
        )
        creation_nonce = Web3.keccak(text=f"{self.bundle['source_commit']}:{tag}:creation")
        predicted = self.factory.functions.predictCompetitionAddress(
            creator.address, params, creation_nonce
        ).call()
        valid_before = int(self.w3.eth.get_block("latest").timestamp) + 1800
        authorization = self.sign_authorization(
            creator, predicted, self.TARGET, secrets.token_bytes(32), valid_before
        )
        receipt = self.send_function(
            relayer,
            self.factory.functions.createCompetitionWithAuthorization(
                creator.address, params, self.TARGET, creation_nonce, authorization
            ),
        )
        created = self.factory.events.CanonicalCompetitionCreated().process_receipt(receipt, errors=DISCARD)
        require(len(created) == 1, f"{tag} creation event missing")
        actual = created[0]["args"]["bounty"]
        require(checksum(actual) == checksum(predicted), f"{tag} prediction mismatch")
        require(self.factory.functions.isCanonicalCompetition(actual).call(), f"{tag} not factory registered")
        bounty = self.w3.eth.contract(address=checksum(actual), abi=self.bounty_abi)
        require(bounty.functions.fundedAmount().call() == self.TARGET, f"{tag} funding mismatch")
        return bounty, receipt, params

    def commitment(self, bounty: Any, actor: Any, tag: str) -> dict[str, Any]:
        submission = Web3.keccak(text=f"{tag}:submission")
        evidence = Web3.keccak(text=f"{tag}:evidence")
        salt = secrets.token_bytes(32)
        commitment = bounty.functions.solutionCommitment(
            actor.address, submission, evidence, salt
        ).call()
        return {"submission": submission, "evidence": evidence, "salt": salt, "commitment": commitment}

    def commit_authorized(self, bounty: Any, actor: Any, material: dict[str, Any]) -> Any:
        relayer = self.actors["relayer"]
        valid_before = int(self.w3.eth.get_block("latest").timestamp) + 1200
        auth = self.sign_authorization(
            actor, bounty.address, self.VERIFIER_REWARD, bytes32(material["commitment"]), valid_before
        )
        return self.send_function(
            relayer,
            bounty.functions.commitSolutionWithAuthorization(
                actor.address, material["commitment"], *auth
            ),
        )

    def expect_revert(self, function: Any, sender: str, expected: str,
                      block_identifier: int | str = "latest") -> str:
        try:
            self.w3.eth.call(
                {"from": checksum(sender), "to": function.address, "data": function._encode_transaction_data()},
                block_identifier=block_identifier,
            )
        except Exception as error:
            reason = revert_reason(error)
            require(expected.lower() in reason.lower(), f"expected '{expected}', received '{reason}'")
            return reason
        raise RehearsalError(f"call unexpectedly succeeded; expected {expected}")

    def mine_proof(self, bounty: Any, actor: Any, material: dict[str, Any], policy_hash: bytes) -> bytes:
        bounty_id = bounty.functions.bountyId().call()
        for nonce in range(2_000_000):
            digest = Web3.keccak(encode(
                ["bytes32", "uint64", "address", "bytes32", "bytes32", "bytes32", "uint256"],
                [bounty_id, 1, actor.address, material["submission"], material["evidence"], policy_hash, nonce],
            ))
            if int.from_bytes(digest, "big") >> 240 == 0:
                return nonce.to_bytes(32, "big")
        raise RehearsalError("difficulty-16 proof search exceeded safety bound")

    def balances(self, addresses: dict[str, str]) -> dict[str, int]:
        return {name: int(self.usdc.functions.balanceOf(address).call()) for name, address in addresses.items()}

    def scenario_settlement(self) -> tuple[dict[str, Any], dict[str, Any]]:
        bounty, create_receipt, params = self.create(
            "settlement", self.verifier_address, max_entries=4, competition_window=300, reveal_window=120
        )
        failed = self.actors["failed_competitor"]
        passing = self.actors["passing_competitor"]
        losing = self.actors["expiring_competitor"]
        before = self.balances({
            "creator": self.actors["creator"].address,
            "failed_competitor": failed.address,
            "passing_competitor": passing.address,
            "losing_competitor": losing.address,
            "verifier_recipient": self.actors["relayer"].address,
            "bounty": bounty.address,
        })
        failed_material = self.commitment(bounty, failed, "settlement:failed")
        passing_material = self.commitment(bounty, passing, "settlement:passing")
        losing_material = self.commitment(bounty, losing, "settlement:losing")

        fake_nonce = Web3.keccak(text="authorization-substitution")
        substitution_reason = self.expect_revert(
            bounty.functions.commitSolutionWithAuthorization(
                failed.address,
                failed_material["commitment"],
                0,
                int(self.w3.eth.get_block("latest").timestamp) + 1200,
                fake_nonce,
                27,
                (1).to_bytes(32, "big"),
                (2).to_bytes(32, "big"),
            ),
            self.actors["relayer"].address,
            "authorization not commitment-bound",
        )

        failed_commit = self.commit_authorized(bounty, failed, failed_material)
        same_block_reason = self.expect_revert(
            bounty.functions.revealSolution(
                failed_material["submission"], failed_material["evidence"], failed_material["salt"], b"\x00"
            ),
            failed.address,
            "reveal requires later block",
            int(failed_commit.blockNumber),
        )
        losing_commit = self.commit_authorized(bounty, losing, losing_material)
        passing_commit = self.commit_authorized(bounty, passing, passing_material)
        copied_reason = self.expect_revert(
            bounty.functions.revealSolution(
                failed_material["submission"], failed_material["evidence"], failed_material["salt"], b"\x00"
            ),
            passing.address,
            "commitment mismatch",
        )
        failed_reveal = self.send_function(
            failed,
            bounty.functions.revealSolution(
                failed_material["submission"], failed_material["evidence"], failed_material["salt"], b"invalid"
            ),
        )
        funded_after_failure = bounty.functions.fundedAmount().call()
        passing_proof = self.mine_proof(bounty, passing, passing_material, bytes32(params[3]))
        passing_reveal = self.send_function(
            passing,
            bounty.functions.revealSolution(
                passing_material["submission"], passing_material["evidence"], passing_material["salt"], passing_proof
            ),
        )
        losing_before_withdraw = self.usdc.functions.balanceOf(losing.address).call()
        losing_withdraw = self.send_function(losing, bounty.functions.withdrawEntryBond())
        losing_after_withdraw = self.usdc.functions.balanceOf(losing.address).call()
        after = self.balances({
            "creator": self.actors["creator"].address,
            "failed_competitor": failed.address,
            "passing_competitor": passing.address,
            "losing_competitor": losing.address,
            "verifier_recipient": self.actors["relayer"].address,
            "bounty": bounty.address,
        })
        receipts = [create_receipt, failed_commit, losing_commit, passing_commit, failed_reveal, passing_reveal, losing_withdraw]
        events: list[dict[str, Any]] = []
        for receipt in receipts:
            for name in (
                "FundingAdded", "CompetitionOpened", "SolutionCommitted", "SolutionRevealed",
                "CompetitionSubmissionRejected", "BountySettled", "EntryBondWithdrawn",
            ):
                events.extend(event_rows(getattr(bounty.events, name)(), receipt))
        require(sum(1 for row in events if row["name"] == "BountySettled") == 1, "settlement event count mismatch")
        require(any(row["name"] == "EntryBondWithdrawn" for row in events), "losing bond event missing")
        require(after["bounty"] == 0, "settlement bounty retained USDC")
        scenario = {
            "bounty_contract": bounty.address.lower(),
            "transactions": [tx_hex(receipt.transactionHash) for receipt in receipts],
            "events": events,
            "balance_deltas": {name: after[name] - before[name] for name in before},
            "reconciled": True,
            "assertions": {
                "authorized_creation_funded_exact_target": before["bounty"] == self.TARGET,
                "failed_entry_preserved_reward": funded_after_failure == self.TARGET,
                "first_confirmed_passing_reveal_won": bounty.functions.winner().call() == passing.address,
                "losing_bond_returned": losing_after_withdraw - losing_before_withdraw == self.VERIFIER_REWARD,
                "escrow_conserved_to_zero": after["bounty"] == 0,
            },
        }
        evidence = {
            "same_block_reveal_rejected": {"passed": True, "block_number": int(failed_commit.blockNumber), "reason": same_block_reason},
            "copied_reveal_rejected": {"passed": True, "reason": copied_reason},
            "authorization_substitution_rejected": {"passed": True, "reason": substitution_reason},
        }
        return scenario, evidence

    def wait_until(self, timestamp: int) -> None:
        while int(self.w3.eth.get_block("latest").timestamp) <= timestamp:
            remaining = timestamp - int(self.w3.eth.get_block("latest").timestamp) + 1
            print(json.dumps({"waiting_for_competition_expiry_seconds": remaining}))
            time.sleep(min(15, max(2, remaining)))

    def scenario_cancellation(self) -> tuple[dict[str, Any], dict[str, Any]]:
        bounty, create_receipt, _ = self.create(
            "cancellation", self.verifier_address, max_entries=2, competition_window=75, reveal_window=30
        )
        expiring = self.actors["expiring_competitor"]
        failed = self.actors["failed_competitor"]
        passing = self.actors["passing_competitor"]
        before = self.balances({"creator": self.actors["creator"].address, "bounty": bounty.address})
        expiring_material = self.commitment(bounty, expiring, "cancellation:expiring")
        failed_material = self.commitment(bounty, failed, "cancellation:failed")
        passing_material = self.commitment(bounty, passing, "cancellation:capacity")
        expiring_commit = self.commit_authorized(bounty, expiring, expiring_material)
        failed_commit = self.commit_authorized(bounty, failed, failed_material)
        capacity_reason = self.expect_revert(
            bounty.functions.commitSolution(passing_material["commitment"]),
            passing.address,
            "entry capacity reached",
        )
        ends_at = int(bounty.functions.competitionEndsAt().call())
        self.wait_until(ends_at)
        relayer = self.actors["relayer"]
        expire_one = self.send_function(relayer, bounty.functions.expireCommitment(expiring.address))
        expire_two = self.send_function(relayer, bounty.functions.expireCommitment(failed.address))
        cancel = self.send_function(relayer, bounty.functions.cancelExpiredCompetition())
        withdraw = self.send_function(self.actors["creator"], bounty.functions.withdrawRefund())
        after = self.balances({"creator": self.actors["creator"].address, "bounty": bounty.address})
        receipts = [create_receipt, expiring_commit, failed_commit, expire_one, expire_two, cancel, withdraw]
        events: list[dict[str, Any]] = []
        for receipt in receipts:
            for name in (
                "FundingAdded", "CompetitionOpened", "SolutionCommitted", "CommitmentExpired",
                "BountyCancelled", "RefundWithdrawn", "BountySettled",
            ):
                events.extend(event_rows(getattr(bounty.events, name)(), receipt))
        require(sum(1 for row in events if row["name"] == "CommitmentExpired") == 2, "expiry event count mismatch")
        require(any(row["name"] == "BountyCancelled" for row in events), "cancellation event missing")
        require(any(row["name"] == "RefundWithdrawn" for row in events), "refund event missing")
        require(not any(row["name"] == "BountySettled" for row in events), "cancelled bounty settled")
        expected_refund = self.TARGET + 2 * self.VERIFIER_REWARD
        scenario = {
            "bounty_contract": bounty.address.lower(),
            "transactions": [tx_hex(receipt.transactionHash) for receipt in receipts],
            "events": events,
            "balance_deltas": {name: after[name] - before[name] for name in before},
            "reconciled": True,
            "assertions": {
                "capacity_enforced_before_token_transfer": True,
                "all_commitments_expired_before_cancel": bounty.functions.lockedBondTotal().call() == 0,
                "principal_refunded": after["creator"] - before["creator"] == expected_refund,
                "expired_bond_refunded_as_bonus": bounty.functions.refundBonusRemaining().call() == 0,
                "escrow_conserved_to_zero": after["bounty"] == 0,
            },
        }
        return scenario, {"capacity_enforced": {"passed": True, "reason": capacity_reason}}

    def verifier_revert_check(self) -> dict[str, Any]:
        relayer = self.actors["relayer"]
        deployment = self.send(relayer, to=None, data=self.retry_bytecode)
        retry_verifier = checksum(deployment.contractAddress)
        bounty, create_receipt, _ = self.create(
            "verifier-revert", retry_verifier, max_entries=2, competition_window=300, reveal_window=120
        )
        solver = self.actors["failed_competitor"]
        material = self.commitment(bounty, solver, "verifier-revert:solver")
        commit = self.commit_authorized(bounty, solver, material)
        reverted = self.send_function(
            solver,
            bounty.functions.revealSolution(
                material["submission"], material["evidence"], material["salt"], b""
            ),
            gas=500_000,
            allow_revert=True,
        )
        require(reverted.status == 0, "reverting verifier transaction unexpectedly passed")
        entry = bounty.functions.entries(solver.address).call()
        require(int(entry[4]) == 1, "verifier revert consumed commitment")
        require(bounty.functions.submissionSequence().call() == 0, "verifier revert consumed sequence")
        retry = self.send_function(
            solver,
            bounty.functions.revealSolution(
                material["submission"], material["evidence"], material["salt"], b"retry"
            ),
        )
        require(bounty.functions.winner().call() == solver.address, "verifier retry did not settle")
        return {
            "passed": True,
            "helper_verifier": retry_verifier.lower(),
            "helper_deployment_transaction": tx_hex(deployment.transactionHash),
            "bounty_contract": bounty.address.lower(),
            "creation_transaction": tx_hex(create_receipt.transactionHash),
            "commit_transaction": tx_hex(commit.transactionHash),
            "reverted_reveal_transaction": tx_hex(reverted.transactionHash),
            "reverted_reveal_block": int(reverted.blockNumber),
            "retry_transaction": tx_hex(retry.transactionHash),
            "commitment_remained_retryable": True,
        }

    def deployment_record(self, name: str, address: str, tx_hash: str, runtime_hash: str) -> dict[str, Any]:
        receipt = self.wait_confirmations(self.w3.eth.get_transaction_receipt(tx_hash))
        runtime = self.w3.eth.get_code(checksum(address), block_identifier=receipt.blockNumber)
        observed_hash = Web3.keccak(runtime).hex()
        require(observed_hash.lower() == runtime_hash.removeprefix("0x").lower(), f"{name} runtime hash mismatch")
        return {
            "address": address.lower(),
            "transaction_hash": tx_hash.lower(),
            "block_number": int(receipt.blockNumber),
            "block_hash": tx_hex(receipt.blockHash),
            "runtime_code_hash": "0x" + observed_hash.lower(),
            "runtime_matches": True,
        }

    def manifest(self, settlement: dict[str, Any], cancellation: dict[str, Any],
                 evidence: dict[str, Any]) -> dict[str, Any]:
        actions = {action["name"]: action for action in self.bundle["actions"]}
        verifier_action = actions["deploy_leading_zero_work_verifier_16"]
        factory_action = actions["deploy_open_competition_factory_v1"]
        deployments = {
            "verifier": self.deployment_record(
                "verifier", verifier_action["expected_contract"], self.args.verifier_tx,
                verifier_action["runtime_code_hash"],
            ),
            "factory": self.deployment_record(
                "factory", factory_action["expected_contract"], self.args.factory_tx,
                factory_action["runtime_code_hash"],
            ),
            "implementation": self.deployment_record(
                "implementation", factory_action["expected_implementation"], self.args.factory_tx,
                factory_action["implementation_runtime_code_hash"],
            ),
        }
        manifest = {
            "schema_version": "agent-bounties/open-competition-v1-rehearsal-manifest-v1",
            "protocol_version": PROTOCOL,
            "network": NETWORK,
            "chain_id": CHAIN_ID,
            "deployment_state": "sepolia_rehearsed_not_ready_to_earn",
            "public_inventory_eligible": False,
            "source_commit": self.bundle["source_commit"],
            "compiler": self.bundle["compiler"],
            "deployer": ADMIN,
            "settlement_token": USDC,
            "deployments": deployments,
            "actors": {name: actor.address.lower() for name, actor in self.actors.items()},
            "scenarios": {
                "settlement_and_losing_bond_withdrawal": settlement,
                "expiry_cancellation_and_refund": cancellation,
            },
            "adversarial_checks": {
                "copied_reveal_rejected": evidence["copied_reveal_rejected"]["passed"],
                "same_block_reveal_rejected": evidence["same_block_reveal_rejected"]["passed"],
                "authorization_substitution_rejected": evidence["authorization_substitution_rejected"]["passed"],
                "capacity_enforced": evidence["capacity_enforced"]["passed"],
                "verifier_revert_recoverable": evidence["verifier_revert_recoverable"]["passed"],
            },
            "adversarial_evidence": evidence,
            "bytecode_freeze": {
                "verifier_runtime_code_hash": verifier_action["runtime_code_hash"],
                "factory_runtime_code_hash": factory_action["runtime_code_hash"],
                "implementation_runtime_code_hash": factory_action["implementation_runtime_code_hash"],
            },
            "evidence_boundary": "This manifest proves the Base Sepolia rehearsal only. It is not mainnet readiness, activation, or payment evidence.",
        }
        if self.recovery_cleanup:
            manifest["superseded_rehearsal_cleanup"] = self.recovery_cleanup
        return manifest

    def run(self) -> None:
        self.funding_handshake()
        if self.args.reclaim_bounty:
            self.reclaim_partial_settlement(self.args.reclaim_bounty)
        self.distribute()
        settlement, settlement_evidence = self.scenario_settlement()
        cancellation, cancellation_evidence = self.scenario_cancellation()
        verifier_evidence = self.verifier_revert_check()
        evidence = {**settlement_evidence, **cancellation_evidence, "verifier_revert_recoverable": verifier_evidence}
        manifest = self.manifest(settlement, cancellation, evidence)
        self.args.output.parent.mkdir(parents=True, exist_ok=True)
        self.args.output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        self.recovery_path.unlink(missing_ok=True)
        print(json.dumps({"completed": True, "manifest": str(self.args.output), "recovery_file_deleted": True}))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc", default="https://sepolia.base.org")
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--verifier-tx", required=True)
    parser.add_argument("--factory-tx", required=True)
    parser.add_argument("--funding-request", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--recovery-file",
        type=Path,
        help="Resume with the exact ephemeral actors from a prior interrupted run.",
    )
    parser.add_argument(
        "--reclaim-bounty",
        help="Recover a settled rehearsal losing bond and restore actor allocations before restarting.",
    )
    args = parser.parse_args()
    Runner(args).run()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
