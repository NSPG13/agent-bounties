#!/usr/bin/env python3
"""Run the Base Sepolia rehearsal for the bounded entrant-wallet relay path.

Preparation creates ephemeral actors and a bounded funding request. Execution
requires the frozen factory deployment and the exact recovery file, then runs:

1. a failed EOA reveal followed by a relayed entrant-wallet settlement; and
2. a separate EOA settlement followed by relayed losing-bond withdrawal.

The final manifest contains no keys, plaintext salts, or signatures. Recovery
keys remain in a temporary file until every receipt is canonical at a Base safe
block, then are deleted.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time
from typing import Any

from eth_abi import encode
from eth_account import Account
from eth_account.messages import encode_typed_data
from web3 import Web3
from web3.logs import DISCARD


SCHEMA = "agent-bounties/open-competition-entrant-wallet-rehearsal-manifest-v1"
RECOVERY_SCHEMA = "agent-bounties/open-competition-entrant-wallet-rehearsal-recovery-v1"
FUNDING_SCHEMA = "agent-bounties/open-competition-entrant-wallet-rehearsal-funding-v1"
DEPLOYMENT_SCHEMA = "agent-bounties/open-competition-entrant-wallet-deployment-v1"
CHAIN_ID = 84532
NETWORK = "base-sepolia"
ADMIN = "0x884834e884d6e93462655a2820140ad03e6747bc"
USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e"
COMPETITION_FACTORY = "0x7231f1312448fa60078fb56cdb6e2c392bd1269b"
VERIFIER = "0x9601a40b35ad6843846732c6cb73c4c82f9ba850"
COMMITMENT_DOMAIN = Web3.keccak(text="agent-bounties/open-competition-v1-solution")
ACTORS = ("creator", "owner", "delegate", "keeper", "failed_competitor", "passing_competitor")
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


class RehearsalError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RehearsalError(message)


def checksum(value: str) -> str:
    return Web3.to_checksum_address(value)


def tx_hex(value: Any) -> str:
    return Web3.to_hex(value).lower()


def hex32(value: Any) -> str:
    raw = bytes(value)
    require(len(raw) == 32, "expected bytes32")
    return "0x" + raw.hex()


def artifact(root: Path, source: str, contract: str) -> dict[str, Any]:
    path = root / "contracts" / "base-escrow" / "out" / source / f"{contract}.json"
    require(path.is_file(), f"missing Foundry artifact {path}")
    return json.loads(path.read_text(encoding="utf-8"))


def git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=root, check=True, capture_output=True, text=True, encoding="utf-8"
    ).stdout.strip()


def event_rows(event: Any, receipt: Any) -> list[dict[str, Any]]:
    rows = []
    for item in event.process_receipt(receipt, errors=DISCARD):
        rows.append(
            {
                "name": item["event"],
                "transaction_hash": tx_hex(item["transactionHash"]),
                "block_number": int(item["blockNumber"]),
                "block_hash": tx_hex(receipt.blockHash),
                "log_index": int(item["logIndex"]),
            }
        )
    return rows


def account_rows(actors: dict[str, Any]) -> dict[str, str]:
    return {name: actor.address.lower() for name, actor in actors.items()}


def walk_calls(call: dict[str, Any]) -> list[dict[str, Any]]:
    rows = [call]
    for child in call.get("calls", []):
        rows.extend(walk_calls(child))
    return rows


def prove_deployment_call(
    transaction: Any,
    trace: dict[str, Any] | None,
    deployer: str,
    calldata: str,
    admin: str,
) -> dict[str, Any]:
    transaction_to = transaction.get("to")
    transaction_input = tx_hex(transaction.get("input", b""))
    transaction_value = int(transaction.get("value", 0))
    if (
        transaction_to is not None
        and transaction_to.lower() == deployer.lower()
        and transaction_input == calldata.lower()
        and transaction_value == 0
    ):
        require(transaction["from"].lower() == admin.lower(), "direct deployment was not sent by the admin")
        return {
            "submission_mode": "direct_admin_transaction",
            "transaction_sender": transaction["from"].lower(),
            "execution_sender": transaction["from"].lower(),
            "admin_authorization": "transaction_sender",
            "exact_zero_value_deployer_call": True,
        }

    require(trace is not None, "relayed deployment requires an exact call trace")
    calls = walk_calls(trace)
    exact_calls = [
        call
        for call in calls
        if (call.get("to") or "").lower() == deployer.lower()
        and call.get("input", "").lower() == calldata.lower()
        and int(call.get("value", "0x0"), 16) == 0
        and not call.get("error")
    ]
    require(len(exact_calls) == 1, "relayed deployment trace must contain exactly one exact deployer call")
    admin_authorizations = [
        call
        for call in calls
        if (call.get("to") or "").lower() == admin.lower()
        and call.get("input", "").lower().startswith("0x1626ba7e")
        and call.get("output", "").lower().startswith("0x1626ba7e")
        and not call.get("error")
    ]
    require(admin_authorizations, "relayed deployment trace lacks successful admin EIP-1271 authorization")
    return {
        "submission_mode": "metamask_relayed_transaction",
        "transaction_sender": transaction["from"].lower(),
        "execution_sender": exact_calls[0]["from"].lower(),
        "admin_authorization": "successful_eip1271_trace",
        "exact_zero_value_deployer_call": True,
    }


def erc20_transfer_calldata(recipient: str, amount: int) -> str:
    return "0xa9059cbb" + encode(["address", "uint256"], [checksum(recipient), amount]).hex()


def prove_funding_call_batch(
    transaction: Any,
    trace: dict[str, Any],
    admin: str,
    keeper: str,
    eth_wei: int,
    token: str,
    token_amount: int,
) -> dict[str, Any]:
    calls = walk_calls(trace)
    native_calls = [
        call
        for call in calls
        if (call.get("to") or "").lower() == keeper.lower()
        and int(call.get("value", "0x0"), 16) == eth_wei
        and call.get("input", "0x").lower() in ("0x", "0x0")
        and not call.get("error")
    ]
    token_calldata = erc20_transfer_calldata(keeper, token_amount)
    token_calls = [
        call
        for call in calls
        if (call.get("to") or "").lower() == token.lower()
        and int(call.get("value", "0x0"), 16) == 0
        and call.get("input", "").lower() == token_calldata
        and not call.get("error")
    ]
    require(len(native_calls) == 1, "funding trace must contain exactly one bounded native transfer")
    require(len(token_calls) == 1, "funding trace must contain exactly one bounded token transfer")
    execution_sender = native_calls[0]["from"].lower()
    require(token_calls[0]["from"].lower() == execution_sender, "funding calls have different execution senders")
    require(execution_sender == admin.lower(), "funding calls were not executed by the admin account")
    transaction_sender = transaction["from"].lower()
    admin_authorization = "transaction_sender"
    if transaction_sender != admin.lower():
        authorizations = [
            call
            for call in calls
            if (call.get("to") or "").lower() == admin.lower()
            and call.get("input", "").lower().startswith("0x1626ba7e")
            and call.get("output", "").lower().startswith("0x1626ba7e")
            and not call.get("error")
        ]
        require(authorizations, "relayed funding trace lacks successful admin EIP-1271 authorization")
        admin_authorization = "successful_eip1271_trace"
    return {
        "submission_mode": "direct_admin_transaction" if transaction_sender == admin.lower() else "metamask_relayed_transaction",
        "transaction_sender": transaction_sender,
        "execution_sender": execution_sender,
        "admin_authorization": admin_authorization,
        "exact_native_transfer": True,
        "exact_token_transfer": True,
        "atomic_transaction": True,
    }


def recovery_commitment_salt(
    recovery_salt: str,
    bounty: str,
    solver: str,
    tag: str,
) -> bytes:
    raw = bytes.fromhex(recovery_salt.removeprefix("0x"))
    require(len(raw) == 32, "recovery salt must be bytes32")
    return Web3.keccak(
        encode(
            ["bytes32", "address", "address", "string"],
            [raw, checksum(bounty), checksum(solver), tag],
        )
    )


def create_recovery(path: Path | None) -> tuple[Path, dict[str, Any]]:
    actors = {name: Account.create(secrets.token_bytes(32)) for name in ACTORS}
    value = {
        "schema_version": RECOVERY_SCHEMA,
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "created_at_unix": int(time.time()),
        "user_salt": hex32(secrets.token_bytes(32)),
        "actors": {
            name: {"address": actor.address.lower(), "private_key": actor.key.hex()}
            for name, actor in actors.items()
        },
        "warning": "Ephemeral Base Sepolia rehearsal keys. Delete after safe-block reconciliation.",
    }
    if path is None:
        handle, raw = tempfile.mkstemp(prefix="agent-bounties-entrant-recovery-", suffix=".json")
        os.close(handle)
        path = Path(raw)
    path = path.resolve()
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass
    return path, value


def load_recovery(path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(value.get("schema_version") == RECOVERY_SCHEMA, "recovery schema mismatch")
    require(value.get("network") == NETWORK and value.get("chain_id") == CHAIN_ID, "recovery chain mismatch")
    require(set(value.get("actors", {})) == set(ACTORS), "recovery actor set mismatch")
    actors = {}
    for name in ACTORS:
        row = value["actors"][name]
        actor = Account.from_key(row["private_key"])
        require(actor.address.lower() == row["address"].lower(), f"recovery address mismatch for {name}")
        actors[name] = actor
    require(len(bytes.fromhex(value["user_salt"].removeprefix("0x"))) == 32, "recovery user salt mismatch")
    return value, actors


def prepare(args: argparse.Namespace) -> None:
    recovery_path, recovery = create_recovery(args.recovery_file)
    keeper = recovery["actors"]["keeper"]["address"]
    request = {
        "schema_version": FUNDING_SCHEMA,
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "from": ADMIN,
        "recipient": keeper,
        "eth_wei": Runner.ADMIN_FUNDING_ETH_WEI,
        "usdc_token": USDC,
        "usdc_base_units": Runner.ADMIN_FUNDING_USDC,
        "maximum_transactions": 2,
        "purpose": "Fund one ephemeral keeper that distributes bounded Base Sepolia rehearsal allocations.",
        "evidence_boundary": "This is testnet actor funding, not bounty funding, settlement, payment, or activation evidence.",
    }
    args.funding_request.parent.mkdir(parents=True, exist_ok=True)
    args.funding_request.write_text(json.dumps(request, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"recovery_file": str(recovery_path), "funding_request": str(args.funding_request), **request}))


class Runner:
    SOLVER_REWARD = 100_000
    VERIFIER_REWARD = 10_000
    TARGET = SOLVER_REWARD + VERIFIER_REWARD
    ADMIN_FUNDING_USDC = 400_000
    ADMIN_FUNDING_ETH_WEI = 500_000_000_000_000
    COMPETITOR_ETH_WEI = 50_000_000_000_000
    MIN_RESUME_KEEPER_ETH_WEI = 100_000_000_000_000
    SAFE_TIMEOUT_SECONDS = 600

    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.root = Path(__file__).resolve().parents[1]
        self.bundle = json.loads(args.bundle.read_text(encoding="utf-8"))
        require(self.bundle.get("schema_version") == DEPLOYMENT_SCHEMA, "deployment bundle schema mismatch")
        require(self.bundle.get("network") == NETWORK and self.bundle.get("chain_id") == CHAIN_ID, "bundle chain mismatch")
        require(self.bundle.get("contract_source_dirty") is False, "dirty contract-source bundle")
        require(not any(self.bundle.get("activation_gates", {}).values()), "source bundle has enabled activation gates")
        require(
            self.bundle["canonical"]["competition_factory"].lower() == COMPETITION_FACTORY,
            "competition factory mismatch",
        )
        require(self.bundle["canonical"]["settlement_token"].lower() == USDC, "settlement token mismatch")
        self.w3 = Web3(Web3.HTTPProvider(args.rpc, request_kwargs={"timeout": 30}))
        require(self.w3.is_connected() and self.w3.eth.chain_id == CHAIN_ID, "Base Sepolia RPC unavailable")
        self.recovery, self.actors = load_recovery(args.recovery_file.resolve())
        self.receipts: dict[str, Any] = {}
        self.relay_details: dict[str, Any] = {}
        self.resume_evidence: dict[str, Any] = {}
        self.reorg_reconciliations: dict[str, Any] = {}
        self.root_commit = git(self.root, "rev-parse", "HEAD")
        self.contract_tree = git(self.root, "rev-parse", "HEAD:contracts/base-escrow")
        require(self.contract_tree == self.bundle["contract_source_revision"], "bundle contract tree is not current")

        factory_artifact = artifact(
            self.root, "OpenCompetitionEntrantWalletFactoryV1.sol", "OpenCompetitionEntrantWalletFactoryV1"
        )
        wallet_artifact = artifact(
            self.root, "OpenCompetitionEntrantWalletV1.sol", "OpenCompetitionEntrantWalletV1"
        )
        competition_factory_artifact = artifact(
            self.root, "OpenCompetitionBountyFactoryV1.sol", "OpenCompetitionBountyFactoryV1"
        )
        bounty_artifact = artifact(self.root, "OpenCompetitionBountyV1.sol", "OpenCompetitionBountyV1")
        usdc_abi = [
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
            {"name": "name", "type": "function", "stateMutability": "view", "inputs": [], "outputs": [{"type": "string"}]},
            {"name": "version", "type": "function", "stateMutability": "view", "inputs": [], "outputs": [{"type": "string"}]},
        ]
        self.entrant_factory_address = checksum(self.bundle["entrant_wallet_factory"]["address"])
        self.entrant_implementation = checksum(self.bundle["entrant_wallet_factory"]["implementation"])
        self.entrant_factory = self.w3.eth.contract(address=self.entrant_factory_address, abi=factory_artifact["abi"])
        self.competition_factory = self.w3.eth.contract(
            address=checksum(COMPETITION_FACTORY), abi=competition_factory_artifact["abi"]
        )
        self.wallet_abi = wallet_artifact["abi"]
        self.bounty_abi = bounty_artifact["abi"]
        self.usdc = self.w3.eth.contract(address=checksum(USDC), abi=usdc_abi)
        self.usdc_name = self.usdc.functions.name().call()
        self.usdc_version = self.usdc.functions.version().call()
        self.verifier_address = checksum(VERIFIER)
        self.profile_hashes = {
            "policy": Web3.keccak(text=f"{self.contract_tree}:entrant-rehearsal:policy"),
            "criteria": Web3.keccak(text=f"{self.contract_tree}:entrant-rehearsal:criteria"),
            "benchmark": Web3.keccak(text=f"{self.contract_tree}:entrant-rehearsal:benchmark"),
            "evidence": Web3.keccak(text=f"{self.contract_tree}:entrant-rehearsal:evidence"),
        }
        self.wallet_address: str | None = None
        self.wallet: Any | None = None

    def fees(self) -> dict[str, int]:
        block = self.w3.eth.get_block("latest")
        base = int(block.get("baseFeePerGas", 1_000_000))
        try:
            priority = max(int(self.w3.eth.max_priority_fee), 1_000_000)
        except Exception:
            priority = 1_000_000
        local_cap = getattr(self.args, "local_priority_fee_cap_wei", None)
        if local_cap is not None:
            require(
                self.args.rpc.startswith("http://127.0.0.1:") or self.args.rpc.startswith("http://localhost:"),
                "the priority-fee cap is restricted to a local fork",
            )
            priority = min(priority, int(local_cap))
        return {"maxFeePerGas": base * 2 + priority, "maxPriorityFeePerGas": priority}

    def send(
        self, actor: Any, *, to: str | None, data: str = "0x", value: int = 0, gas: int | None = None
    ) -> Any:
        tx: dict[str, Any] = {
            "chainId": CHAIN_ID,
            "from": actor.address,
            "nonce": self.w3.eth.get_transaction_count(actor.address, "pending"),
            "value": value,
            "data": data,
            **self.fees(),
        }
        if to is not None:
            tx["to"] = checksum(to)
        estimate = self.w3.eth.estimate_gas(tx) if gas is None else gas
        tx["gas"] = max(estimate + estimate // 4, 21_000)
        signed = actor.sign_transaction(tx)
        tx_hash = self.w3.eth.send_raw_transaction(signed.raw_transaction)
        receipt = self.w3.eth.wait_for_transaction_receipt(tx_hash, timeout=180, poll_latency=1)
        require(receipt.status == 1, f"transaction {tx_hex(tx_hash)} reverted")
        return receipt

    def send_function(self, actor: Any, function: Any) -> Any:
        return self.send(actor, to=function.address, data=function._encode_transaction_data())

    def remember(self, name: str, receipt: Any) -> Any:
        require(name not in self.receipts, f"duplicate receipt label {name}")
        self.receipts[name] = receipt
        return receipt

    def call_after_receipt(
        self,
        function: Any,
        receipt: Any,
        label: str,
        predicate: Any = lambda _value: True,
    ) -> Any:
        deadline = time.time() + 45
        last_error: Exception | None = None
        while time.time() < deadline:
            try:
                value = function.call(block_identifier=int(receipt.blockNumber))
                if predicate(value):
                    return value
            except Exception as error:
                last_error = error
            time.sleep(1)
        detail = f": {last_error}" if last_error else ""
        raise RehearsalError(f"{label} was not readable at its confirmed receipt block{detail}")

    def history_start_block(self) -> int:
        deployment = self.w3.eth.get_transaction_receipt(self.args.deployment_tx)
        start = int(deployment.blockNumber)
        if self.args.funding_tx:
            start = int(self.w3.eth.get_transaction_receipt(self.args.funding_tx).blockNumber)
        return max(start, int(self.w3.eth.block_number) - 1_999)

    def sign_authorization(
        self, actor: Any, to: str, value: int, *, authorization_nonce: bytes | None = None
    ) -> tuple[Any, ...]:
        valid_after = 0
        valid_before = int(self.w3.eth.get_block("latest").timestamp) + 1800
        nonce = authorization_nonce or secrets.token_bytes(32)
        require(len(nonce) == 32, "authorization nonce must be bytes32")
        message = encode_typed_data(
            full_message={
                "types": {
                    "EIP712Domain": [
                        {"name": "name", "type": "string"},
                        {"name": "version", "type": "string"},
                        {"name": "chainId", "type": "uint256"},
                        {"name": "verifyingContract", "type": "address"},
                    ],
                    **TRANSFER_TYPES,
                },
                "primaryType": "TransferWithAuthorization",
                "domain": {
                    "name": self.usdc_name,
                    "version": self.usdc_version,
                    "chainId": CHAIN_ID,
                    "verifyingContract": checksum(USDC),
                },
                "message": {
                    "from": actor.address,
                    "to": checksum(to),
                    "value": value,
                    "validAfter": valid_after,
                    "validBefore": valid_before,
                    "nonce": nonce,
                },
            }
        )
        signed = actor.sign_message(message)
        return (
            valid_after,
            valid_before,
            nonce,
            signed.v,
            signed.r.to_bytes(32, "big"),
            signed.s.to_bytes(32, "big"),
        )

    def validate_deployment(self) -> dict[str, Any]:
        tx_hash = self.args.deployment_tx.lower()
        transaction = self.w3.eth.get_transaction(tx_hash)
        receipt = self.w3.eth.get_transaction_receipt(tx_hash)
        require(receipt.status == 1, "entrant factory deployment reverted")
        expected = self.bundle["entrant_wallet_factory"]
        direct = (
            transaction.get("to") is not None
            and transaction["to"].lower() == self.bundle["deterministic_deployer"]["address"].lower()
            and tx_hex(transaction["input"]) == expected["deployment_transaction"].lower()
        )
        trace = None
        if not direct:
            response = self.w3.provider.make_request(
                "debug_traceTransaction", [tx_hash, {"tracer": "callTracer"}]
            )
            require(not response.get("error") and isinstance(response.get("result"), dict), "deployment trace unavailable")
            trace = response["result"]
        provenance = prove_deployment_call(
            transaction,
            trace,
            self.bundle["deterministic_deployer"]["address"],
            expected["deployment_transaction"],
            ADMIN,
        )
        factory_code = self.w3.eth.get_code(self.entrant_factory_address, receipt.blockNumber)
        implementation_code = self.w3.eth.get_code(self.entrant_implementation, receipt.blockNumber)
        require(Web3.keccak(factory_code).hex() == expected["runtime_code_hash"].removeprefix("0x"), "factory runtime mismatch")
        require(
            Web3.keccak(implementation_code).hex() == expected["implementation_runtime_code_hash"].removeprefix("0x"),
            "implementation runtime mismatch",
        )
        require(
            self.entrant_factory.functions.competitionFactory().call().lower() == COMPETITION_FACTORY,
            "entrant factory competition binding mismatch",
        )
        require(self.entrant_factory.functions.settlementToken().call().lower() == USDC, "entrant factory token mismatch")
        return {
            "transaction_hash": tx_hash,
            "block_number": int(receipt.blockNumber),
            "block_hash": tx_hex(receipt.blockHash),
            "factory": self.entrant_factory_address.lower(),
            "factory_runtime_hash": expected["runtime_code_hash"],
            "implementation": self.entrant_implementation.lower(),
            "implementation_runtime_hash": expected["implementation_runtime_code_hash"],
            "provenance": provenance,
        }

    def validate_funding(self) -> dict[str, Any]:
        local_fork = self.args.rpc.startswith("http://127.0.0.1:") or self.args.rpc.startswith("http://localhost:")
        if local_fork and not self.args.funding_tx:
            return {
                "mode": "local_fork_state_override",
                "live_evidence": False,
                "eth_wei": self.ADMIN_FUNDING_ETH_WEI,
                "usdc_base_units": self.ADMIN_FUNDING_USDC,
            }
        require(isinstance(self.args.funding_tx, str) and len(self.args.funding_tx) == 66, "live run requires --funding-tx")
        tx_hash = self.args.funding_tx.lower()
        transaction = self.w3.eth.get_transaction(tx_hash)
        receipt = self.w3.eth.get_transaction_receipt(tx_hash)
        require(receipt.status == 1, "rehearsal funding transaction reverted")
        response = self.w3.provider.make_request("debug_traceTransaction", [tx_hash, {"tracer": "callTracer"}])
        require(not response.get("error") and isinstance(response.get("result"), dict), "funding trace unavailable")
        keeper = self.actors["keeper"].address
        provenance = prove_funding_call_batch(
            transaction,
            response["result"],
            ADMIN,
            keeper,
            self.ADMIN_FUNDING_ETH_WEI,
            USDC,
            self.ADMIN_FUNDING_USDC,
        )
        require(
            self.w3.eth.get_balance(keeper, receipt.blockNumber) >= self.ADMIN_FUNDING_ETH_WEI,
            "funding receipt did not produce keeper ETH",
        )
        require(
            self.usdc.functions.balanceOf(keeper).call(block_identifier=receipt.blockNumber) >= self.ADMIN_FUNDING_USDC,
            "funding receipt did not produce keeper test USDC",
        )
        return {
            "mode": "confirmed_atomic_admin_batch",
            "live_evidence": True,
            "transaction_hash": tx_hash,
            "block_number": int(receipt.blockNumber),
            "block_hash": tx_hex(receipt.blockHash),
            "keeper": keeper.lower(),
            "eth_wei": self.ADMIN_FUNDING_ETH_WEI,
            "usdc_token": USDC,
            "usdc_base_units": self.ADMIN_FUNDING_USDC,
            "provenance": provenance,
        }

    def require_funding(self) -> None:
        keeper = self.actors["keeper"]
        owner = self.actors["owner"]
        user_salt = bytes.fromhex(self.recovery["user_salt"].removeprefix("0x"))
        predicted_wallet = self.entrant_factory.functions.predictWallet(
            owner.address, self.policy(), user_salt
        ).call()
        wallet_preexisting = self.w3.eth.get_code(predicted_wallet) != b""
        minimum_keeper_eth = (
            self.MIN_RESUME_KEEPER_ETH_WEI if wallet_preexisting else self.ADMIN_FUNDING_ETH_WEI
        )
        require(
            self.w3.eth.get_balance(keeper.address) >= minimum_keeper_eth,
            "keeper ETH is below the bounded remaining rehearsal gas reserve",
        )
        actor_usdc = sum(
            self.usdc.functions.balanceOf(actor.address).call() for actor in self.actors.values()
        )
        if wallet_preexisting:
            actor_usdc += self.usdc.functions.balanceOf(predicted_wallet).call()
            created = self.competition_factory.events.CanonicalCompetitionCreated().get_logs(
                from_block=self.history_start_block(),
                to_block="latest",
                argument_filters={"creator": self.actors["creator"].address},
            )
            actor_usdc += sum(
                self.usdc.functions.balanceOf(checksum(event["args"]["bounty"])).call()
                for event in created
            )
        require(
            actor_usdc >= self.ADMIN_FUNDING_USDC,
            "ephemeral actor and entrant-wallet USDC no longer conserve the exact request",
        )

    def distribute(self) -> None:
        keeper = self.actors["keeper"]
        owner = self.actors["owner"]
        policy = self.policy()
        user_salt = bytes.fromhex(self.recovery["user_salt"].removeprefix("0x"))
        predicted_wallet = self.entrant_factory.functions.predictWallet(owner.address, policy, user_salt).call()
        wallet_preexisting = self.w3.eth.get_code(predicted_wallet) != b""
        existing_competitions = self.competition_factory.events.CanonicalCompetitionCreated().get_logs(
            from_block=self.history_start_block(),
            to_block="latest",
            argument_filters={"creator": self.actors["creator"].address},
        )
        if existing_competitions:
            self.resume_evidence.update(
                {
                    "scenario_state_recovered": True,
                    "existing_competition_count": len(existing_competitions),
                    "actor_allocations_reconciled_without_redistribution": True,
                }
            )
            return
        allocations = {
            "creator": self.TARGET * 2,
            # A restart after createWalletWithAuthorization has already moved
            # the owner's allocation into the wallet. Do not top the owner up
            # or create a second funding path.
            "owner": 0 if wallet_preexisting else self.VERIFIER_REWARD * 2,
            "failed_competitor": self.VERIFIER_REWARD,
            "passing_competitor": self.VERIFIER_REWARD,
        }
        for name, amount in allocations.items():
            current = self.usdc.functions.balanceOf(self.actors[name].address).call()
            require(current <= amount, f"{name} has an unexpected USDC balance")
            if current < amount:
                self.remember(
                    f"fund_usdc_{name}",
                    self.send_function(
                        keeper, self.usdc.functions.transfer(self.actors[name].address, amount - current)
                    ),
                )
        for name in ("failed_competitor", "passing_competitor"):
            actor = self.actors[name]
            current = self.w3.eth.get_balance(actor.address)
            require(current <= self.COMPETITOR_ETH_WEI, f"{name} has an unexpected ETH balance")
            if current < self.COMPETITOR_ETH_WEI:
                self.remember(
                    f"fund_eth_{name}",
                    self.send(keeper, to=actor.address, value=self.COMPETITOR_ETH_WEI - current),
                )
        if wallet_preexisting:
            wallet_balance = self.usdc.functions.balanceOf(predicted_wallet).call()
            self.resume_evidence.update(
                {
                    "resumed_after_confirmed_wallet_creation": True,
                    "predicted_wallet": predicted_wallet.lower(),
                    "actor_allocations_reconciled_without_redistribution": True,
                    "entrant_wallet_usdc_at_resume": int(wallet_balance),
                }
            )

    def recover_existing_wallet_policy(self) -> tuple[Any, ...] | None:
        if self.wallet is not None:
            return tuple(self.wallet.functions.policy().call())
        # Public Base endpoints cap eth_getLogs ranges. Wallet creation is
        # necessarily after actor funding, so the bounded history is complete.
        from_block = self.history_start_block()
        logs = self.entrant_factory.events.OpenCompetitionEntrantWalletCreated().get_logs(
            from_block=from_block,
            to_block="latest",
            argument_filters={
                "owner": self.actors["owner"].address,
                "delegate": self.actors["delegate"].address,
            },
        )
        if not logs:
            return None
        require(len(logs) == 1, "multiple entrant wallets match the recovery actors")
        address = checksum(logs[0]["args"]["wallet"])
        expected = self.bundle["entrant_wallet_factory"]["clone_runtime_code_hash"].removeprefix("0x")
        require(Web3.keccak(self.w3.eth.get_code(address)).hex() == expected, "recovered wallet runtime mismatch")
        wallet = self.w3.eth.contract(address=address, abi=self.wallet_abi)
        require(wallet.functions.owner().call() == self.actors["owner"].address, "recovered wallet owner mismatch")
        policy = tuple(wallet.functions.policy().call())
        now = int(self.w3.eth.get_block("latest").timestamp)
        require(
            policy[0] == self.actors["delegate"].address
            and int(policy[1]) <= now < int(policy[2])
            and int(policy[3]) == 3600
            and int(policy[4]) == self.VERIFIER_REWARD
            and int(policy[5]) == self.VERIFIER_REWARD * 2
            and int(policy[6]) == self.VERIFIER_REWARD * 2
            and int(policy[7]) == self.TARGET
            and int(policy[8]) == 7
            and policy[9] == self.verifier_address
            and bytes(policy[10]) == Web3.keccak(self.w3.eth.get_code(self.verifier_address))
            and bytes(policy[11]) == self.profile_hashes["policy"]
            and bytes(policy[12]) == self.profile_hashes["criteria"]
            and bytes(policy[13]) == self.profile_hashes["benchmark"]
            and bytes(policy[14]) == self.profile_hashes["evidence"],
            "recovered wallet policy mismatch",
        )
        self.wallet_address = address
        self.wallet = wallet
        self.resume_evidence.update(
            {
                "resumed_after_confirmed_wallet_creation": True,
                "predicted_wallet": address.lower(),
                "wallet_policy_recovered_from_chain": True,
            }
        )
        return policy

    def policy(self) -> tuple[Any, ...]:
        recovered = self.recover_existing_wallet_policy()
        if recovered is not None:
            return recovered
        now = int(self.w3.eth.get_block("latest").timestamp)
        verifier_code_hash = Web3.keccak(self.w3.eth.get_code(self.verifier_address))
        return (
            self.actors["delegate"].address,
            now - 60,
            now + 86_400,
            3600,
            self.VERIFIER_REWARD,
            self.VERIFIER_REWARD * 2,
            self.VERIFIER_REWARD * 2,
            self.TARGET,
            7,
            self.verifier_address,
            verifier_code_hash,
            self.profile_hashes["policy"],
            self.profile_hashes["criteria"],
            self.profile_hashes["benchmark"],
            self.profile_hashes["evidence"],
        )

    def create_wallet(self) -> dict[str, Any]:
        owner = self.actors["owner"]
        keeper = self.actors["keeper"]
        policy = self.policy()
        user_salt = bytes.fromhex(self.recovery["user_salt"].removeprefix("0x"))
        predicted = self.entrant_factory.functions.predictWallet(owner.address, policy, user_salt).call()
        preexisting = self.w3.eth.get_code(predicted) != b""
        if preexisting:
            created_logs = self.entrant_factory.events.OpenCompetitionEntrantWalletCreated().get_logs(
                from_block=self.history_start_block(),
                to_block="latest",
                argument_filters={"wallet": checksum(predicted)},
            )
            require(len(created_logs) == 1, "preexisting wallet creation event mismatch")
            receipt = self.remember(
                "create_and_fund_entrant_wallet",
                self.w3.eth.get_transaction_receipt(created_logs[0]["transactionHash"]),
            )
            self.resume_evidence.update(
                {
                    "wallet_creation_transaction": tx_hex(receipt.transactionHash),
                    "wallet_creation_block": int(receipt.blockNumber),
                    "wallet_creation_receipt_recovered": True,
                }
            )
        else:
            auth = self.sign_authorization(owner, predicted, self.VERIFIER_REWARD * 2)
            receipt = self.remember(
                "create_and_fund_entrant_wallet",
                self.send_function(
                    keeper,
                    self.entrant_factory.functions.createWalletWithAuthorization(
                        owner.address, policy, user_salt, self.VERIFIER_REWARD * 2, *auth
                    ),
                ),
            )
        deadline = time.time() + 45
        while time.time() < deadline:
            try:
                code = self.w3.eth.get_code(predicted)
                if code and self.w3.eth.contract(address=checksum(predicted), abi=self.wallet_abi).functions.owner().call():
                    break
            except Exception:
                pass
            time.sleep(1)
        else:
            raise RehearsalError("confirmed entrant wallet was not readable before the RPC propagation deadline")
        created = self.entrant_factory.events.OpenCompetitionEntrantWalletCreated().process_receipt(
            receipt, errors=DISCARD
        )
        require(len(created) == 1 and created[0]["args"]["wallet"].lower() == predicted.lower(), "wallet creation event mismatch")
        self.wallet_address = checksum(predicted)
        self.wallet = self.w3.eth.contract(address=self.wallet_address, abi=self.wallet_abi)
        require(self.wallet.functions.owner().call() == owner.address, "wallet owner mismatch")
        require(self.wallet.functions.policy().call()[0] == self.actors["delegate"].address, "wallet delegate mismatch")
        wallet_balance = self.usdc.functions.balanceOf(self.wallet_address).call()
        if not preexisting:
            require(wallet_balance == self.VERIFIER_REWARD * 2, "wallet funding mismatch")
        return {
            "address": self.wallet_address.lower(),
            "creation_transaction": tx_hex(receipt.transactionHash),
            "creation_block": int(receipt.blockNumber),
            "owner": owner.address.lower(),
            "delegate": self.actors["delegate"].address.lower(),
            "policy_hash": hex32(self.wallet.functions.policyHash().call()),
            "policy_version": int(self.wallet.functions.policyVersion().call()),
            "initial_funding_usdc_base_units": self.VERIFIER_REWARD * 2,
            "current_funding_usdc_base_units_at_resume": int(wallet_balance),
            "runtime_hash": self.bundle["entrant_wallet_factory"]["clone_runtime_code_hash"],
        }

    def competition_params(self, tag: str) -> tuple[Any, ...]:
        now = int(self.w3.eth.get_block("latest").timestamp)
        return (
            self.SOLVER_REWARD,
            self.VERIFIER_REWARD,
            Web3.keccak(text=f"{self.contract_tree}:{tag}:terms"),
            self.profile_hashes["policy"],
            self.profile_hashes["criteria"],
            self.profile_hashes["benchmark"],
            self.profile_hashes["evidence"],
            now + 3600,
            900,
            300,
            4,
            self.verifier_address,
            self.actors["keeper"].address,
        )

    def create_competition(self, tag: str) -> tuple[Any, Any]:
        creator = self.actors["creator"]
        keeper = self.actors["keeper"]
        expected_terms = Web3.keccak(text=f"{self.contract_tree}:{tag}:terms")
        existing = []
        for event in self.competition_factory.events.CanonicalCompetitionCreated().get_logs(
            from_block=self.history_start_block(),
            to_block="latest",
            argument_filters={"creator": creator.address},
        ):
            candidate = self.w3.eth.contract(address=checksum(event["args"]["bounty"]), abi=self.bounty_abi)
            if bytes(candidate.functions.termsHash().call()) == expected_terms:
                existing.append((candidate, event))
        require(len(existing) <= 1, f"multiple {tag} competitions match the recovery actors")
        if existing:
            bounty, event = existing[0]
            receipt = self.remember(
                f"create_competition_{tag}", self.w3.eth.get_transaction_receipt(event["transactionHash"])
            )
            require(bounty.functions.creator().call() == creator.address, f"{tag} recovered creator mismatch")
            require(
                int(bounty.functions.status().call()) in (1, 2),
                f"{tag} recovered competition is neither open nor settled",
            )
            self.resume_evidence[f"{tag}_competition_recovered"] = True
            return bounty, receipt
        params = self.competition_params(tag)
        nonce = Web3.keccak(text=f"{self.contract_tree}:{self.recovery['user_salt']}:{tag}")
        predicted = self.competition_factory.functions.predictCompetitionAddress(
            creator.address, params, nonce
        ).call()
        require(self.w3.eth.get_code(predicted) == b"", f"{tag} competition already exists")
        auth = self.sign_authorization(creator, predicted, self.TARGET)
        receipt = self.remember(
            f"create_competition_{tag}",
            self.send_function(
                keeper,
                self.competition_factory.functions.createCompetitionWithAuthorization(
                    creator.address, params, self.TARGET, nonce, auth
                ),
            ),
        )
        created = self.competition_factory.events.CanonicalCompetitionCreated().process_receipt(
            receipt, errors=DISCARD
        )
        require(len(created) == 1 and created[0]["args"]["bounty"].lower() == predicted.lower(), f"{tag} creation mismatch")
        bounty = self.w3.eth.contract(address=checksum(predicted), abi=self.bounty_abi)
        self.call_after_receipt(
            bounty.functions.status(), receipt, f"{tag} competition status", lambda value: int(value) == 1
        )
        return bounty, receipt

    def material(self, bounty: Any, solver: str, tag: str) -> dict[str, Any]:
        submission = Web3.keccak(text=f"{self.recovery['user_salt']}:{tag}:submission")
        evidence = Web3.keccak(text=f"{self.recovery['user_salt']}:{tag}:evidence")
        salt = recovery_commitment_salt(self.recovery["user_salt"], bounty.address, solver, tag)
        commitment = bounty.functions.solutionCommitment(solver, submission, evidence, salt).call()
        return {"submission": submission, "evidence": evidence, "salt": salt, "commitment": commitment}

    def mine_proof(self, bounty: Any, solver: str, material: dict[str, Any]) -> bytes:
        bounty_id = bounty.functions.bountyId().call()
        for nonce in range(2_000_000):
            digest = Web3.keccak(
                encode(
                    ["bytes32", "uint64", "address", "bytes32", "bytes32", "bytes32", "uint256"],
                    [
                        bounty_id,
                        1,
                        solver,
                        material["submission"],
                        material["evidence"],
                        self.profile_hashes["policy"],
                        nonce,
                    ],
                )
            )
            if int.from_bytes(digest, "big") >> 240 == 0:
                return nonce.to_bytes(32, "big")
        raise RehearsalError("difficulty-16 proof search exceeded safety bound")

    def relay_action(self, name: str, action: int, payload: bytes) -> Any:
        require(self.wallet is not None, "wallet is not initialized")
        keeper = self.actors["keeper"]
        delegate = self.actors["delegate"]
        payload_hash = Web3.keccak(payload)
        existing = []
        for event in self.wallet.events.EntrantActionExecuted().get_logs(
            from_block=self.history_start_block(),
            to_block="latest",
            argument_filters={"delegate": delegate.address, "relayer": keeper.address},
        ):
            row = event["args"]
            if int(row["action"]) == action and bytes(row["payloadHash"]) == payload_hash:
                existing.append(event)
        require(len(existing) <= 1, f"multiple {name} action events match")
        if existing:
            event = existing[0]
            receipt = self.remember(name, self.w3.eth.get_transaction_receipt(event["transactionHash"]))
            row = event["args"]
            nonce = int(row["nonce"])
            gas_cost = int(receipt.gasUsed) * int(receipt.effectiveGasPrice)
            self.relay_details[name] = {
                "action": action,
                "delegate_nonce": nonce,
                "payload_hash": hex32(payload_hash),
                "keeper_eth_delta_wei": -gas_cost,
                "receipt_recovered": True,
            }
            self.resume_evidence[f"{name}_recovered"] = True
            return receipt
        nonce = int(self.wallet.functions.delegateNonce().call())
        deadline = int(self.w3.eth.get_block("latest").timestamp) + 600
        digest = self.wallet.functions.actionDigest(action, payload_hash, nonce, deadline).call()
        signature = delegate.unsafe_sign_hash(digest).signature
        balance_before = self.w3.eth.get_balance(keeper.address)
        receipt = self.remember(
            name,
            self.send_function(
                keeper, self.wallet.functions.executeWithSignature(action, payload, nonce, deadline, signature)
            ),
        )
        balance_after = self.w3.eth.get_balance(keeper.address)
        events = self.wallet.events.EntrantActionExecuted().process_receipt(receipt, errors=DISCARD)
        require(len(events) == 1, f"{name} action event missing")
        row = events[0]["args"]
        require(
            int(row["action"]) == action
            and row["delegate"] == delegate.address
            and row["relayer"] == keeper.address
            and int(row["nonce"]) == nonce
            and bytes(row["payloadHash"]) == payload_hash,
            f"{name} action event mismatch",
        )
        self.call_after_receipt(
            self.wallet.functions.delegateNonce(), receipt, f"{name} nonce", lambda value: int(value) == nonce + 1
        )
        gas_cost = int(receipt.gasUsed) * int(receipt.effectiveGasPrice)
        self.relay_details[name] = {
            "action": action,
            "delegate_nonce": nonce,
            "payload_hash": hex32(payload_hash),
            "keeper_eth_delta_wei": -gas_cost,
            "observed_latest_balance_delta_wei": balance_after - balance_before,
        }
        return receipt

    def advance_later_block(self, name: str, block_number: int) -> None:
        if int(self.w3.eth.block_number) <= block_number:
            keeper = self.actors["keeper"]
            receipt = self.remember(name, self.send(keeper, to=keeper.address, value=0))
            require(int(receipt.blockNumber) > block_number, "later-block transaction was not mined later")
        else:
            require(int(self.w3.eth.block_number) > block_number, "later-block advance failed")

    def eoa_commit(self, name: str, bounty: Any, actor: Any, material: dict[str, Any]) -> Any:
        if bounty.functions.hasEntered(actor.address).call():
            entry = bounty.functions.entries(actor.address).call()
            require(bytes(entry[0]) == bytes(material["commitment"]), f"{name} recovered commitment mismatch")
            logs = bounty.events.SolutionCommitted().get_logs(
                from_block=self.history_start_block(),
                to_block="latest",
                argument_filters={"solver": actor.address},
            )
            matching = [row for row in logs if bytes(row["args"]["commitment"]) == bytes(material["commitment"])]
            require(len(matching) == 1, f"{name} recovered event mismatch")
            self.resume_evidence[f"{name}_recovered"] = True
            return self.remember(name, self.w3.eth.get_transaction_receipt(matching[0]["transactionHash"]))
        auth = self.sign_authorization(
            actor,
            bounty.address,
            self.VERIFIER_REWARD,
            authorization_nonce=bytes(material["commitment"]),
        )
        return self.remember(
            name,
            self.send_function(
                self.actors["keeper"],
                bounty.functions.commitSolutionWithAuthorization(actor.address, material["commitment"], *auth),
            ),
        )

    def snapshot(self, addresses: dict[str, str], block: int | str = "latest") -> dict[str, int]:
        deadline = time.time() + 45
        last_error: Exception | None = None
        while time.time() < deadline:
            try:
                return {
                    name: int(self.usdc.functions.balanceOf(checksum(address)).call(block_identifier=block))
                    for name, address in addresses.items()
                }
            except Exception as error:
                last_error = error
                time.sleep(1)
        raise RehearsalError(f"USDC snapshot block was not readable: {last_error}")

    def scenario_wallet_wins(self) -> dict[str, Any]:
        require(self.wallet_address is not None, "wallet is not initialized")
        bounty, create_receipt = self.create_competition("wallet-wins")
        failed = self.actors["failed_competitor"]
        before = self.snapshot(
            {
                "wallet": self.wallet_address,
                "failed_competitor": failed.address,
                "keeper_verifier_recipient": self.actors["keeper"].address,
                "bounty": bounty.address,
            },
            block=int(create_receipt.blockNumber),
        )
        failed_material = self.material(bounty, failed.address, "wallet-wins:failed")
        wallet_material = self.material(bounty, self.wallet_address, "wallet-wins:wallet")
        failed_commit = self.eoa_commit("wallet_wins_failed_eoa_commit", bounty, failed, failed_material)
        self.advance_later_block("wallet_wins_failed_reveal_block_advance", int(failed_commit.blockNumber))
        failed_entry = bounty.functions.entries(failed.address).call()
        if int(failed_entry[4]) == 2:
            rejected = bounty.events.CompetitionSubmissionRejected().get_logs(
                from_block=self.history_start_block(),
                to_block="latest",
                argument_filters={"solver": failed.address},
            )
            require(len(rejected) == 1, "recovered failed-reveal event mismatch")
            failed_reveal = self.remember(
                "wallet_wins_failed_eoa_reveal",
                self.w3.eth.get_transaction_receipt(rejected[0]["transactionHash"]),
            )
            self.resume_evidence["wallet_wins_failed_eoa_reveal_recovered"] = True
        else:
            require(int(failed_entry[4]) == 1, "failed competitor entry is not revealable")
            failed_reveal = self.remember(
                "wallet_wins_failed_eoa_reveal",
                self.send_function(
                    failed,
                    bounty.functions.revealSolution(
                        failed_material["submission"],
                        failed_material["evidence"],
                        failed_material["salt"],
                        b"invalid",
                    ),
                ),
            )
        commit_payload = encode(["address", "bytes32"], [bounty.address, wallet_material["commitment"]])
        wallet_commit = self.relay_action("wallet_wins_relayed_commit", 0, commit_payload)
        self.advance_later_block("wallet_wins_relayed_reveal_block_advance", int(wallet_commit.blockNumber))
        proof = self.mine_proof(bounty, self.wallet_address, wallet_material)
        reveal_payload = encode(
            ["address", "bytes32", "bytes32", "bytes32", "bytes"],
            [
                bounty.address,
                wallet_material["submission"],
                wallet_material["evidence"],
                wallet_material["salt"],
                proof,
            ],
        )
        wallet_reveal = self.relay_action("wallet_wins_relayed_reveal", 1, reveal_payload)
        after = self.snapshot(
            {
                "wallet": self.wallet_address,
                "failed_competitor": failed.address,
                "keeper_verifier_recipient": self.actors["keeper"].address,
                "bounty": bounty.address,
            },
            block=int(wallet_reveal.blockNumber),
        )
        require(
            int(self.call_after_receipt(bounty.functions.status(), wallet_reveal, "wallet-wins status")) == 2,
            "wallet-wins bounty did not settle",
        )
        require(
            self.call_after_receipt(bounty.functions.winner(), wallet_reveal, "wallet-wins winner").lower()
            == self.wallet_address.lower(),
            "entrant wallet is not winner",
        )
        require(after["bounty"] == 0, "wallet-wins bounty retained USDC")
        require(
            after["wallet"] - before["wallet"] == self.SOLVER_REWARD,
            "entrant-wallet settlement balance mismatch",
        )
        receipts = [create_receipt, failed_commit, failed_reveal, wallet_commit, wallet_reveal]
        events = []
        for receipt in receipts:
            for name in (
                "FundingAdded",
                "CompetitionOpened",
                "SolutionCommitted",
                "CompetitionSubmissionRejected",
                "BountySettled",
            ):
                events.extend(event_rows(getattr(bounty.events, name)(), receipt))
        require(sum(row["name"] == "BountySettled" for row in events) == 1, "wallet settlement event mismatch")
        return {
            "bounty": bounty.address.lower(),
            "winner": self.wallet_address.lower(),
            "transactions": [tx_hex(receipt.transactionHash) for receipt in receipts],
            "events": events,
            "balance_deltas": {name: after[name] - before[name] for name in before},
            "assertions": {
                "failed_eoa_reveal_rejected": True,
                "relayed_wallet_commit_confirmed": True,
                "relayed_wallet_reveal_settled": True,
                "canonical_bounty_settled_event": True,
                "escrow_conserved_to_zero": True,
            },
        }

    def scenario_wallet_loses_and_withdraws(self) -> dict[str, Any]:
        require(self.wallet_address is not None, "wallet is not initialized")
        bounty, create_receipt = self.create_competition("wallet-loses")
        winner = self.actors["passing_competitor"]
        before = self.snapshot(
            {"wallet": self.wallet_address, "winner": winner.address, "bounty": bounty.address},
            block=int(create_receipt.blockNumber),
        )
        wallet_material = self.material(bounty, self.wallet_address, "wallet-loses:wallet")
        winner_material = self.material(bounty, winner.address, "wallet-loses:winner")
        wallet_commit = self.relay_action(
            "wallet_loses_relayed_commit",
            0,
            encode(["address", "bytes32"], [bounty.address, wallet_material["commitment"]]),
        )
        winner_commit = self.eoa_commit("wallet_loses_winner_eoa_commit", bounty, winner, winner_material)
        self.advance_later_block("wallet_loses_winner_reveal_block_advance", int(winner_commit.blockNumber))
        proof = self.mine_proof(bounty, winner.address, winner_material)
        if int(bounty.functions.status().call()) == 2:
            settled = bounty.events.BountySettled().get_logs(
                from_block=self.history_start_block(),
                to_block="latest",
                argument_filters={"solver": winner.address},
            )
            require(len(settled) == 1, "recovered separate-solver settlement event mismatch")
            winner_reveal = self.remember(
                "wallet_loses_winner_eoa_reveal",
                self.w3.eth.get_transaction_receipt(settled[0]["transactionHash"]),
            )
            self.resume_evidence["wallet_loses_winner_eoa_reveal_recovered"] = True
        else:
            winner_reveal = self.remember(
                "wallet_loses_winner_eoa_reveal",
                self.send_function(
                    winner,
                    bounty.functions.revealSolution(
                        winner_material["submission"],
                        winner_material["evidence"],
                        winner_material["salt"],
                        proof,
                    ),
                ),
            )
        self.call_after_receipt(
            bounty.functions.winner(),
            winner_reveal,
            "wallet-loses settlement winner",
            lambda value: value == winner.address,
        )
        balance_before_withdraw = self.call_after_receipt(
            self.usdc.functions.balanceOf(self.wallet_address),
            winner_reveal,
            "wallet balance before bond withdrawal",
        )
        withdraw = self.relay_action(
            "wallet_loses_relayed_bond_withdrawal", 2, encode(["address"], [bounty.address])
        )
        balance_after_withdraw = self.call_after_receipt(
            self.usdc.functions.balanceOf(self.wallet_address),
            withdraw,
            "wallet balance after bond withdrawal",
        )
        after = self.snapshot(
            {"wallet": self.wallet_address, "winner": winner.address, "bounty": bounty.address},
            block=int(withdraw.blockNumber),
        )
        require(
            self.call_after_receipt(bounty.functions.winner(), withdraw, "wallet-loses winner") == winner.address,
            "separate EOA did not win",
        )
        require(balance_after_withdraw - balance_before_withdraw == self.VERIFIER_REWARD, "bond withdrawal mismatch")
        require(after["bounty"] == 0, "wallet-loses bounty retained USDC")
        receipts = [create_receipt, wallet_commit, winner_commit, winner_reveal, withdraw]
        events = []
        for receipt in receipts:
            for name in (
                "FundingAdded",
                "CompetitionOpened",
                "SolutionCommitted",
                "BountySettled",
                "EntryBondWithdrawn",
            ):
                events.extend(event_rows(getattr(bounty.events, name)(), receipt))
        require(sum(row["name"] == "BountySettled" for row in events) == 1, "separate settlement event mismatch")
        require(sum(row["name"] == "EntryBondWithdrawn" for row in events) == 1, "bond event mismatch")
        return {
            "bounty": bounty.address.lower(),
            "winner": winner.address.lower(),
            "transactions": [tx_hex(receipt.transactionHash) for receipt in receipts],
            "events": events,
            "balance_deltas": {name: after[name] - before[name] for name in before},
            "assertions": {
                "separate_solver_wallet_won": True,
                "entrant_wallet_remained_losing_committed_entry": True,
                "relayed_bond_withdrawal_confirmed": True,
                "full_bond_returned": True,
                "escrow_conserved_to_zero": True,
            },
        }

    def wait_safe(self) -> dict[str, Any]:
        deadline = time.monotonic() + self.SAFE_TIMEOUT_SECONDS
        while time.monotonic() < deadline:
            highest = max(int(receipt.blockNumber) for receipt in self.receipts.values())
            safe = self.w3.eth.get_block("safe")
            if int(safe.number) >= highest:
                refreshed_rows: dict[str, Any] = {}
                for name, receipt in list(self.receipts.items()):
                    refreshed = self.w3.eth.get_transaction_receipt(receipt.transactionHash)
                    canonical = self.w3.eth.get_block(refreshed.blockNumber)
                    require(
                        refreshed.status == 1
                        and bytes(canonical.hash) == bytes(refreshed.blockHash),
                        f"{name} is not canonical",
                    )
                    if (
                        int(refreshed.blockNumber) != int(receipt.blockNumber)
                        or bytes(refreshed.blockHash) != bytes(receipt.blockHash)
                    ):
                        self.reorg_reconciliations[name] = {
                            "transaction_hash": tx_hex(receipt.transactionHash),
                            "observed_block_number": int(receipt.blockNumber),
                            "observed_block_hash": tx_hex(receipt.blockHash),
                            "canonical_block_number": int(refreshed.blockNumber),
                            "canonical_block_hash": tx_hex(refreshed.blockHash),
                        }
                    refreshed_rows[name] = refreshed
                if int(safe.number) < max(int(row.blockNumber) for row in refreshed_rows.values()):
                    time.sleep(2)
                    continue
                self.receipts.update(refreshed_rows)
                return {
                    "number": int(safe.number),
                    "hash": tx_hex(safe.hash),
                    "timestamp": int(safe.timestamp),
                }
            time.sleep(2)
        raise RehearsalError("rehearsal receipts did not reach a Base safe block")

    def refresh_scenario_events(self, scenario: dict[str, Any], names: tuple[str, ...]) -> None:
        bounty = self.w3.eth.contract(address=checksum(scenario["bounty"]), abi=self.bounty_abi)
        receipts = [self.w3.eth.get_transaction_receipt(tx_hash) for tx_hash in scenario["transactions"]]
        events = []
        for receipt in receipts:
            for name in names:
                events.extend(event_rows(getattr(bounty.events, name)(), receipt))
        scenario["events"] = events

    def receipt_rows(self) -> dict[str, Any]:
        rows = {}
        for name, receipt in self.receipts.items():
            row = {
                "transaction_hash": tx_hex(receipt.transactionHash),
                "block_number": int(receipt.blockNumber),
                "block_hash": tx_hex(receipt.blockHash),
                "gas_used": int(receipt.gasUsed),
                "effective_gas_price_wei": int(receipt.effectiveGasPrice),
                "gas_cost_wei": int(receipt.gasUsed) * int(receipt.effectiveGasPrice),
                "status": int(receipt.status),
            }
            relay = self.relay_details.get(name)
            if relay:
                row["relay"] = relay
            rows[name] = row
        return rows

    def run(self) -> None:
        deployment = self.validate_deployment()
        funding = self.validate_funding()
        self.require_funding()
        self.distribute()
        wallet = self.create_wallet()
        wallet_wins = self.scenario_wallet_wins()
        wallet_loses = self.scenario_wallet_loses_and_withdraws()
        safe = self.wait_safe()
        self.refresh_scenario_events(
            wallet_wins,
            (
                "FundingAdded",
                "CompetitionOpened",
                "SolutionCommitted",
                "CompetitionSubmissionRejected",
                "BountySettled",
            ),
        )
        self.refresh_scenario_events(
            wallet_loses,
            ("FundingAdded", "CompetitionOpened", "SolutionCommitted", "BountySettled", "EntryBondWithdrawn"),
        )
        expected = self.bundle["entrant_wallet_factory"]
        require(
            Web3.keccak(self.w3.eth.get_code(self.entrant_factory_address, safe["number"])).hex()
            == expected["runtime_code_hash"].removeprefix("0x"),
            "safe-block factory runtime mismatch",
        )
        require(
            Web3.keccak(self.w3.eth.get_code(self.entrant_implementation, safe["number"])).hex()
            == expected["implementation_runtime_code_hash"].removeprefix("0x"),
            "safe-block implementation runtime mismatch",
        )
        require(self.wallet_address is not None, "wallet is not initialized")
        require(
            Web3.keccak(self.w3.eth.get_code(self.wallet_address, safe["number"])).hex()
            == expected["clone_runtime_code_hash"].removeprefix("0x"),
            "safe-block clone runtime mismatch",
        )
        manifest = {
            "schema_version": SCHEMA,
            "network": NETWORK,
            "chain_id": CHAIN_ID,
            "deployment_state": "sepolia_rehearsed_not_ready_to_earn",
            "source_commit": self.root_commit,
            "contract_source_revision": self.contract_tree,
            "compiler": self.bundle["compiler"],
            "admin": ADMIN,
            "settlement_token": USDC,
            "canonical_competition_factory": COMPETITION_FACTORY,
            "approved_canary_verifier": {
                "address": VERIFIER,
                "runtime_hash": hex32(Web3.keccak(self.w3.eth.get_code(self.verifier_address, safe["number"]))),
                "policy_hash": hex32(self.profile_hashes["policy"]),
                "acceptance_criteria_hash": hex32(self.profile_hashes["criteria"]),
                "benchmark_hash": hex32(self.profile_hashes["benchmark"]),
                "evidence_schema_hash": hex32(self.profile_hashes["evidence"]),
            },
            "deployment": deployment,
            "actor_funding": funding,
            "resume_evidence": self.resume_evidence or {"resumed_after_confirmed_wallet_creation": False},
            "entrant_wallet": wallet,
            "actors": account_rows(self.actors),
            "scenarios": {
                "relayed_wallet_settlement": wallet_wins,
                "separate_solver_and_relayed_bond_withdrawal": wallet_loses,
            },
            "canonical_safe_block": safe,
            "reorg_reconciliations": self.reorg_reconciliations,
            "receipts": self.receipt_rows(),
            "activation_gates": {
                "base_sepolia_rehearsal_passed": True,
                "keeper_relay_rehearsed": True,
                "keeper_gas_reserve_verified": True,
                "exact_mainnet_fork_replay_passed": False,
                "independent_review_complete": False,
                "relay_support_available": False,
                "gas_sponsorship_available": False,
                "public_creation_enabled": False,
                "public_inventory_enabled": False,
            },
            "assertions": {
                "factory_and_implementation_match_frozen_runtime": True,
                "wallet_clone_matches_frozen_runtime": True,
                "creator_owner_delegate_keeper_are_distinct": len(
                    {
                        self.actors["creator"].address,
                        self.actors["owner"].address,
                        self.actors["delegate"].address,
                        self.actors["keeper"].address,
                    }
                )
                == 4,
                "keeper_paid_all_relay_gas": True,
                "entrant_wallet_spent_zero_eth": self.w3.eth.get_balance(self.wallet_address) == 0,
                "plaintext_salts_and_signatures_omitted": True,
                "existing_competition_factory_unchanged": True,
                "public_activation_remains_disabled": True,
            },
            "evidence_boundary": (
                "This proves the bounded entrant-wallet relay path on Base Sepolia only. It is not mainnet "
                "readiness, hosted relay availability, gas sponsorship availability, public activation, or payment evidence."
            ),
        }
        self.args.output.parent.mkdir(parents=True, exist_ok=True)
        self.args.output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        self.args.recovery_file.unlink(missing_ok=True)
        print(json.dumps({"completed": True, "manifest": str(self.args.output), "recovery_file_deleted": True}))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true", help="Create ephemeral actors and a funding request.")
    parser.add_argument("--execute", action="store_true", help="Execute and reconcile the live rehearsal.")
    parser.add_argument("--rpc", default="https://sepolia.base.org")
    parser.add_argument(
        "--bundle",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-sepolia-deployment-regenerated.json"),
    )
    parser.add_argument("--deployment-tx", help="Confirmed entrant-wallet factory deployment transaction hash.")
    parser.add_argument("--funding-tx", help="Confirmed atomic admin funding transaction hash.")
    parser.add_argument(
        "--local-priority-fee-cap-wei",
        type=int,
        help="Local-fork-only priority-fee cap; never accepted for a live RPC.",
    )
    parser.add_argument("--recovery-file", type=Path)
    parser.add_argument(
        "--funding-request",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-sepolia-rehearsal-funding.json"),
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-sepolia-rehearsal.json"),
    )
    args = parser.parse_args()
    require(args.prepare != args.execute, "choose exactly one of --prepare or --execute")
    if args.prepare:
        prepare(args)
        return 0
    require(args.recovery_file is not None and args.recovery_file.is_file(), "--execute requires --recovery-file")
    require(isinstance(args.deployment_tx, str) and len(args.deployment_tx) == 66, "--execute requires --deployment-tx")
    Runner(args).run()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
