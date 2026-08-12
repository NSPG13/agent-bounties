#!/usr/bin/env python3
"""Replay the frozen entrant-wallet release on an exact Base mainnet fork.

The script never broadcasts to Base. It pins one canonical Base safe block,
starts Anvil at that block, deploys the exact deterministic entrant-wallet
factory calldata from the frozen bundle, and runs the same two keeper-relayed
scenarios used by the live Sepolia rehearsal. Ephemeral fork-only keys are
created in a temporary directory and deleted before the final manifest is
written.
"""

from __future__ import annotations

import argparse
from argparse import Namespace
import json
from pathlib import Path
import subprocess
import tempfile
import time
from typing import Any

from web3 import Web3

import run_open_competition_entrant_wallet_sepolia_rehearsal as rehearsal


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "agent-bounties/open-competition-entrant-wallet-mainnet-fork-replay-v1"
DEPLOYMENT_SCHEMA = "agent-bounties/open-competition-entrant-wallet-deployment-v1"
CHAIN_ID = 8453
NETWORK = "base-mainnet"
FORK_NETWORK = "base-mainnet-fork"
ADMIN = "0x884834e884d6e93462655a2820140ad03e6747bc"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
COMPETITION_FACTORY = "0x9e9382beb8b1a45b737d484b5eafa7b8779d4ca5"
VERIFIER = "0xcc6059ceeda5bc4ba8a97ecfbffa7488c8fd579e"
VERIFIER_RUNTIME_HASH = "0xbaa3a8305c4b65d0dc20131d0ef207fdaf4763f345393a831370cd04077df9b3"
CREATE2_DEPLOYER = "0x4e59b44847b379578588920ca78fbf26c0b4956c"
CREATE2_DEPLOYER_RUNTIME_HASH = "0x2fa86add0aed31f33a762c9d88e807c475bd51d0f52bd0955754b2608f7e4989"


class ForkReplayError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ForkReplayError(message)


def code_hash(w3: Web3, address: str, block: int | str = "latest") -> str:
    return f"0x{Web3.keccak(w3.eth.get_code(Web3.to_checksum_address(address), block)).hex()}"


def tx_hex(value: Any) -> str:
    text = value.hex() if hasattr(value, "hex") else str(value)
    return text if text.startswith("0x") else f"0x{text}"


def receipt_row(receipt: Any) -> dict[str, Any]:
    return {
        "transaction_hash": tx_hex(receipt.transactionHash),
        "block_number": int(receipt.blockNumber),
        "block_hash": tx_hex(receipt.blockHash),
        "gas_used": int(receipt.gasUsed),
        "status": int(receipt.status),
    }


def wait_rpc(w3: Web3, process: subprocess.Popen[str]) -> None:
    for _ in range(120):
        if process.poll() is not None:
            stderr = process.stderr.read() if process.stderr else ""
            raise ForkReplayError(f"Anvil exited before becoming ready: {stderr}")
        try:
            if w3.is_connected() and w3.eth.chain_id == CHAIN_ID:
                return
        except Exception:
            pass
        time.sleep(0.25)
    raise ForkReplayError("Anvil fork did not become ready")


def send_impersonated(
    w3: Web3,
    *,
    sender: str,
    to: str,
    data: str = "0x",
    value: int = 0,
) -> Any:
    latest = w3.eth.get_block("latest")
    base_fee = int(latest.get("baseFeePerGas", 1_000_000))
    transaction: dict[str, Any] = {
        "chainId": CHAIN_ID,
        "from": Web3.to_checksum_address(sender),
        "to": Web3.to_checksum_address(to),
        "value": value,
        "data": data,
        "maxPriorityFeePerGas": 1,
        "maxFeePerGas": base_fee * 2 + 1,
    }
    estimated = w3.eth.estimate_gas(transaction)
    transaction["gas"] = estimated + estimated // 4 + 50_000
    tx_hash = w3.eth.send_transaction(transaction)
    receipt = w3.eth.wait_for_transaction_receipt(tx_hash, timeout=180, poll_latency=0.25)
    require(receipt.status == 1, f"fork setup transaction reverted: {tx_hex(tx_hash)}")
    return receipt


class MainnetForkRunner(rehearsal.Runner):
    """Mine local confirmations so Anvil's safe tag covers every receipt."""

    SAFE_TIMEOUT_SECONDS = 30

    def wait_safe(self) -> dict[str, Any]:
        highest = max(int(receipt.blockNumber) for receipt in self.receipts.values())
        current = int(self.w3.eth.block_number)
        count = max(64, highest + 64 - current)
        response = self.w3.provider.make_request("anvil_mine", [hex(count)])
        require(not response.get("error"), "Anvil confirmation mining failed")
        safe = self.w3.eth.get_block("safe")
        if int(safe.number) < highest:
            response = self.w3.provider.make_request("anvil_mine", [hex(64)])
            require(not response.get("error"), "Anvil safe-block advancement failed")
            safe = self.w3.eth.get_block("safe")
        require(int(safe.number) >= highest, "Anvil safe tag did not cover all replay receipts")
        return {
            "number": int(safe.number),
            "hash": tx_hex(safe.hash),
            "timestamp": int(safe.timestamp),
            "local_confirmation_depth": int(safe.number) - highest,
        }


def configure_rehearsal_module() -> None:
    rehearsal.CHAIN_ID = CHAIN_ID
    rehearsal.NETWORK = NETWORK
    rehearsal.ADMIN = ADMIN
    rehearsal.USDC = USDC
    rehearsal.COMPETITION_FACTORY = COMPETITION_FACTORY
    rehearsal.VERIFIER = VERIFIER


def validate_bundle(bundle: dict[str, Any]) -> None:
    require(bundle.get("schema_version") == DEPLOYMENT_SCHEMA, "deployment bundle schema mismatch")
    require(bundle.get("network") == NETWORK and bundle.get("chain_id") == CHAIN_ID, "bundle is not Base mainnet")
    require(bundle.get("contract_source_dirty") is False, "bundle was generated from dirty contract sources")
    require(not any(bundle.get("activation_gates", {}).values()), "frozen bundle has an enabled activation gate")
    require(bundle.get("canonical", {}).get("competition_factory", "").lower() == COMPETITION_FACTORY, "competition factory mismatch")
    require(bundle.get("canonical", {}).get("settlement_token", "").lower() == USDC, "settlement token mismatch")
    require(bundle.get("deterministic_deployer", {}).get("address", "").lower() == CREATE2_DEPLOYER, "CREATE2 deployer mismatch")
    require(
        bundle.get("deterministic_deployer", {}).get("runtime_code_hash", "").lower()
        == CREATE2_DEPLOYER_RUNTIME_HASH,
        "CREATE2 deployer runtime pin mismatch",
    )


def run(args: argparse.Namespace) -> dict[str, Any]:
    bundle = json.loads(args.bundle.read_text(encoding="utf-8"))
    validate_bundle(bundle)
    upstream = Web3(Web3.HTTPProvider(args.rpc, request_kwargs={"timeout": 30}))
    require(upstream.is_connected() and upstream.eth.chain_id == CHAIN_ID, "Base mainnet RPC unavailable")
    pinned = upstream.eth.get_block(args.fork_block_number if args.fork_block_number is not None else "safe")
    if args.fork_block_hash:
        require(tx_hex(pinned.hash).lower() == args.fork_block_hash.lower(), "requested fork block hash mismatch")
    fork_block = {
        "number": int(pinned.number),
        "hash": tx_hex(pinned.hash),
        "timestamp": int(pinned.timestamp),
        "source_tag": "explicit" if args.fork_block_number is not None else "safe",
    }
    local_rpc = f"http://127.0.0.1:{args.port}"
    process = subprocess.Popen(
        [
            str(args.anvil),
            "--fork-url",
            args.rpc,
            "--fork-block-number",
            str(fork_block["number"]),
            "--chain-id",
            str(CHAIN_ID),
            "--host",
            "127.0.0.1",
            "--port",
            str(args.port),
            "--silent",
        ],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        local = Web3(Web3.HTTPProvider(local_rpc, request_kwargs={"timeout": 30}))
        wait_rpc(local, process)
        require(int(local.eth.block_number) == fork_block["number"], "Anvil fork block mismatch")
        require(
            tx_hex(local.eth.get_block(fork_block["number"]).hash).lower() == fork_block["hash"].lower(),
            "Anvil fork hash mismatch",
        )
        existing_factory_hash_before = code_hash(local, COMPETITION_FACTORY)
        verifier_hash = code_hash(local, VERIFIER)
        create2_hash = code_hash(local, CREATE2_DEPLOYER)
        require(existing_factory_hash_before != Web3.keccak(b"").hex(), "canonical competition factory has no code")
        require(verifier_hash == VERIFIER_RUNTIME_HASH, "approved verifier runtime mismatch")
        require(create2_hash == CREATE2_DEPLOYER_RUNTIME_HASH, "CREATE2 deployer runtime mismatch")
        entrant = bundle["entrant_wallet_factory"]
        require(local.eth.get_code(Web3.to_checksum_address(entrant["address"])) == b"", "entrant factory address is occupied at fork block")
        require(local.eth.get_code(Web3.to_checksum_address(entrant["implementation"])) == b"", "entrant implementation address is occupied at fork block")

        admin = Web3.to_checksum_address(ADMIN)
        local.provider.make_request("anvil_impersonateAccount", [admin])
        admin_eth_before = int(local.eth.get_balance(admin))
        usdc = local.eth.contract(
            address=Web3.to_checksum_address(USDC),
            abi=[
                {
                    "name": "balanceOf",
                    "type": "function",
                    "stateMutability": "view",
                    "inputs": [{"name": "account", "type": "address"}],
                    "outputs": [{"type": "uint256"}],
                },
                {
                    "name": "transfer",
                    "type": "function",
                    "stateMutability": "nonpayable",
                    "inputs": [{"name": "to", "type": "address"}, {"name": "amount", "type": "uint256"}],
                    "outputs": [{"type": "bool"}],
                },
            ],
        )
        admin_usdc_before = int(usdc.functions.balanceOf(admin).call())
        require(admin_eth_before >= rehearsal.Runner.ADMIN_FUNDING_ETH_WEI, "pinned admin ETH cannot fund the fork replay")
        require(admin_usdc_before >= rehearsal.Runner.ADMIN_FUNDING_USDC, "pinned admin USDC cannot fund the fork replay")

        with tempfile.TemporaryDirectory(prefix="agent-bounties-entrant-mainnet-fork-") as temporary:
            temporary_root = Path(temporary)
            recovery_file = temporary_root / "recovery.json"
            raw_output = temporary_root / "raw-replay.json"
            configure_rehearsal_module()
            rehearsal.create_recovery(recovery_file)
            _, actors = rehearsal.load_recovery(recovery_file)
            keeper = actors["keeper"].address
            deployment_receipt = send_impersonated(
                local,
                sender=ADMIN,
                to=CREATE2_DEPLOYER,
                data=entrant["deployment_transaction"],
            )
            require(
                code_hash(local, entrant["address"]) == entrant["runtime_code_hash"],
                "deployed entrant factory runtime mismatch",
            )
            require(
                code_hash(local, entrant["implementation"]) == entrant["implementation_runtime_code_hash"],
                "deployed entrant implementation runtime mismatch",
            )
            eth_funding_receipt = send_impersonated(
                local,
                sender=ADMIN,
                to=keeper,
                value=rehearsal.Runner.ADMIN_FUNDING_ETH_WEI,
            )
            usdc_funding_receipt = send_impersonated(
                local,
                sender=ADMIN,
                to=USDC,
                data=usdc.functions.transfer(
                    Web3.to_checksum_address(keeper), rehearsal.Runner.ADMIN_FUNDING_USDC
                )._encode_transaction_data(),
            )
            runner_args = Namespace(
                bundle=args.bundle,
                deployment_tx=tx_hex(deployment_receipt.transactionHash),
                funding_tx=None,
                recovery_file=recovery_file,
                funding_request=temporary_root / "unused-funding-request.json",
                output=raw_output,
                rpc=local_rpc,
                local_priority_fee_cap_wei=1,
            )
            MainnetForkRunner(runner_args).run()
            require(not recovery_file.exists(), "fork recovery envelope was not deleted")
            replay = json.loads(raw_output.read_text(encoding="utf-8"))

        canonical_after = upstream.eth.get_block(fork_block["number"])
        require(tx_hex(canonical_after.hash).lower() == fork_block["hash"].lower(), "upstream fork block reorganized during replay")
        existing_factory_hash_after = code_hash(local, COMPETITION_FACTORY)
        require(existing_factory_hash_after == existing_factory_hash_before, "canonical competition factory runtime changed")
        require(replay.get("assertions", {}).get("public_activation_remains_disabled") is True, "raw replay enabled public activation")
        replay["schema_version"] = SCHEMA
        replay["network"] = FORK_NETWORK
        replay["deployment_state"] = "mainnet_fork_replayed_not_ready_to_earn"
        replay["fork_block"] = fork_block
        replay["broadcast"] = False
        replay["fork_setup"] = {
            "mode": "local_anvil_impersonation",
            "admin": ADMIN,
            "admin_eth_before_wei": admin_eth_before,
            "admin_usdc_before_base_units": admin_usdc_before,
            "deployment": receipt_row(deployment_receipt),
            "keeper_eth_funding": receipt_row(eth_funding_receipt),
            "keeper_usdc_funding": receipt_row(usdc_funding_receipt),
            "keeper": keeper.lower(),
            "live_evidence": False,
        }
        replay["actor_funding"] = {
            "mode": "local_fork_admin_setup",
            "live_evidence": False,
            "keeper": keeper.lower(),
            "eth_wei": rehearsal.Runner.ADMIN_FUNDING_ETH_WEI,
            "usdc_token": USDC,
            "usdc_base_units": rehearsal.Runner.ADMIN_FUNDING_USDC,
        }
        replay["canonical_runtime_evidence"] = {
            "competition_factory": COMPETITION_FACTORY,
            "competition_factory_runtime_before": existing_factory_hash_before,
            "competition_factory_runtime_after": existing_factory_hash_after,
            "approved_verifier": VERIFIER,
            "approved_verifier_runtime_hash": verifier_hash,
            "deterministic_deployer": CREATE2_DEPLOYER,
            "deterministic_deployer_runtime_hash": create2_hash,
        }
        replay["activation_gates"] = {
            "base_sepolia_rehearsal_passed": False,
            "exact_mainnet_fork_replay_passed": True,
            "static_analysis_passed": False,
            "independent_review_complete": False,
            "keeper_relay_rehearsed": True,
            "keeper_gas_reserve_verified": True,
            "relay_support_available": False,
            "gas_sponsorship_available": False,
            "public_creation_enabled": False,
            "public_inventory_enabled": False,
        }
        replay["assertions"].update(
            {
                "exact_canonical_fork_block_pinned": True,
                "upstream_block_remained_canonical": True,
                "approved_mainnet_verifier_runtime_matched": True,
                "deterministic_deployer_runtime_matched": True,
                "existing_competition_factory_runtime_unchanged": True,
                "frozen_mainnet_factory_and_implementation_runtimes_matched": True,
                "no_mainnet_transaction_broadcast": True,
                "temporary_recovery_envelope_deleted": True,
            }
        )
        replay["passed"] = all(replay["assertions"].values()) and all(
            all(scenario["assertions"].values()) for scenario in replay["scenarios"].values()
        )
        require(replay["passed"], "mainnet fork replay assertions failed")
        replay["evidence_boundary"] = (
            "This proves exact local behavior on an Anvil fork pinned to one canonical Base mainnet safe block. "
            "It is not a Base transaction, live entrant deployment, hosted relay, gas sponsorship, public activation, "
            "bounty settlement, or payment evidence."
        )
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(replay, indent=2) + "\n", encoding="utf-8")
        return replay
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--bundle",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-mainnet-deployment-regenerated.json"),
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-mainnet-fork-replay.json"),
    )
    parser.add_argument("--rpc", default="https://mainnet.base.org")
    parser.add_argument("--fork-block-number", type=int)
    parser.add_argument("--fork-block-hash")
    parser.add_argument("--port", type=int, default=9553)
    parser.add_argument("--anvil", type=Path, default=ROOT / ".tools" / "foundry" / "anvil.exe")
    args = parser.parse_args()
    replay = run(args)
    print(
        json.dumps(
            {
                "completed": True,
                "manifest": str(args.output),
                "fork_block": replay["fork_block"],
                "entrant_factory": replay["deployment"]["factory"],
                "recovery_file_deleted": True,
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
