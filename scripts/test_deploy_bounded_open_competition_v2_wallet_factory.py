#!/usr/bin/env python3
"""Tests for exact bounded reserve-factory deployment."""

from __future__ import annotations

import copy
import tempfile
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest import mock

import deploy_bounded_open_competition_v2_wallet_factory as deploy


def fixture() -> tuple[dict, dict]:
    competition_factory = "0x" + "11" * 20
    reserve_factory = "0x" + "22" * 20
    implementation = "0x" + "33" * 20
    deterministic = "0x" + "44" * 20
    release_hash = "0x" + "55" * 32
    manifest = {
        "schema": deploy.MANIFEST_SCHEMA,
        "network": "base-mainnet",
        "chain_id": 8453,
        "contract_source_dirty": False,
        "canonical": {
            "protocol_version": deploy.PROTOCOL,
            "competition_factory": competition_factory,
            "settlement_token": "0x" + "66" * 20,
            "release_hash": release_hash,
        },
        "deterministic_deployer": {
            "address": deterministic,
            "runtime_code_hash": "0x" + "77" * 32,
        },
        "reserve_factory": {
            "address": reserve_factory,
            "implementation": implementation,
            "salt": "0x" + "88" * 32,
            "deployment_transaction": "0x" + "88" * 32 + "60",
            "runtime_code_hash": "0x" + "99" * 32,
            "implementation_runtime_code_hash": "0x" + "aa" * 32,
        },
    }
    release = {
        "protocol_version": deploy.PROTOCOL,
        "network": "base-mainnet",
        "factory_contract": competition_factory,
        "factory_runtime_code_hash": "0x" + "bb" * 32,
        "settlement_token": "0x" + "66" * 20,
        "release_hash": release_hash,
    }
    return manifest, release


class ReserveFactoryDeploymentTests(unittest.TestCase):
    def test_accepts_exact_clean_release_binding(self) -> None:
        manifest, release = fixture()
        deploy.validate_manifest(manifest, release)

    def test_rejects_dirty_or_different_release(self) -> None:
        manifest, release = fixture()
        manifest["contract_source_dirty"] = True
        with self.assertRaisesRegex(RuntimeError, "source is dirty"):
            deploy.validate_manifest(manifest, release)
        manifest, release = fixture()
        release["release_hash"] = "0x" + "cc" * 32
        with self.assertRaisesRegex(RuntimeError, "another release"):
            deploy.validate_manifest(manifest, release)

    def test_existing_evidence_is_content_bound(self) -> None:
        manifest, release = fixture()
        signer = "0x" + "dd" * 20
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "evidence.json"
            evidence = deploy.initial_evidence(manifest, release, signer)
            deploy.write_evidence(output, evidence)
            self.assertEqual(deploy.load_evidence(output, manifest, release, signer), evidence)
            changed = copy.deepcopy(manifest)
            changed["reserve_factory"]["deployment_transaction"] += "00"
            with self.assertRaisesRegex(RuntimeError, "manifest_hash"):
                deploy.load_evidence(output, changed, release, signer)

    @mock.patch.object(deploy, "rpc")
    def test_create2_signing_checksums_the_lowercase_deployer(
        self, rpc_call: mock.Mock
    ) -> None:
        manifest, _release = fixture()
        manifest["deterministic_deployer"][
            "address"
        ] = "0x4e59b44847b379578588920ca78fbf26c0b4956c"
        signer = deploy.Account.from_key("0x" + "01".zfill(64))
        client = SimpleNamespace(
            url="https://primary.example",
            signer=signer,
            pending_nonce=mock.Mock(return_value=0),
            broadcast=mock.Mock(return_value="0x" + "12" * 32),
            wait_receipt=mock.Mock(return_value={"status": "0x1"}),
        )

        def fake_rpc(_url: str, method: str, _params: list[object]):
            if method == "eth_getBlockByNumber":
                return {"baseFeePerGas": "0x1"}
            if method == "eth_maxPriorityFeePerGas":
                return "0xf4240"
            if method == "eth_estimateGas":
                return "0x5208"
            self.fail(f"unexpected RPC call: {method}")

        rpc_call.side_effect = fake_rpc
        receipt = deploy.send_create2(client, manifest)

        self.assertEqual(receipt, {"status": "0x1"})
        client.broadcast.assert_called_once()
        client.wait_receipt.assert_called_once_with("0x" + "12" * 32)

    def test_new_deployment_fails_when_the_bounded_nonce_has_moved(self) -> None:
        manifest, _release = fixture()
        client = SimpleNamespace(pending_nonce=mock.Mock(return_value=38))

        with self.assertRaisesRegex(
            RuntimeError, "nonce moved without the exact reserve factory"
        ):
            deploy.send_create2(
                client,
                manifest,
                maximum_new_deployment_nonce=37,
            )

    def test_refreshes_reincluded_reserve_receipt(self) -> None:
        transaction_hash = "0x" + "12" * 32
        evidence = {
            "transaction": {
                "transaction_hash": transaction_hash,
                "block_number": 10,
                "block_hash": "0x" + "34" * 32,
                "gas_used": 1,
            }
        }
        client = SimpleNamespace(
            receipt=mock.Mock(
                return_value={
                    "transactionHash": transaction_hash,
                    "status": "0x1",
                    "blockNumber": "0xc",
                    "blockHash": "0x" + "56" * 32,
                    "gasUsed": "0x2a",
                }
            )
        )

        deployment_block = deploy.reconcile_canonical_receipt(evidence, client)

        self.assertEqual(deployment_block, 12)
        self.assertEqual(evidence["transaction"]["block_number"], 12)
        self.assertEqual(evidence["transaction"]["block_hash"], "0x" + "56" * 32)
        self.assertEqual(evidence["transaction"]["gas_used"], 42)

    @mock.patch.object(deploy, "rpc")
    def test_exact_existing_deployment_is_recovered_without_broadcast(self, rpc_call: mock.Mock) -> None:
        manifest, release = fixture()
        expected_by_address = {
            manifest["deterministic_deployer"]["address"]: manifest["deterministic_deployer"]["runtime_code_hash"],
            release["factory_contract"]: release["factory_runtime_code_hash"],
            manifest["reserve_factory"]["address"]: manifest["reserve_factory"]["runtime_code_hash"],
            manifest["reserve_factory"]["implementation"]: manifest["reserve_factory"]["implementation_runtime_code_hash"],
        }
        client = mock.Mock()
        client.signer.address = "0x" + "dd" * 20
        client.code_hash.side_effect = lambda address, _block="latest": expected_by_address[address]
        client.wait_safe.return_value = {
            "number": "0x2a",
            "hash": "0x" + "ee" * 32,
            "timestamp": "0x64",
        }
        rpc_call.return_value = {"number": "0x29", "hash": "0x" + "ff" * 32}
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "evidence.json"
            evidence = deploy.deploy(manifest, release, client, output)
        self.assertTrue(evidence["complete"])
        self.assertTrue(evidence["transaction"]["recovered_exact_deployment"])
        client.wait_receipt.assert_not_called()

    @mock.patch.object(deploy, "send_create2")
    @mock.patch.object(deploy, "rpc")
    def test_absent_factory_broadcasts_once_and_verifies_safe_runtime(
        self, rpc_call: mock.Mock, send: mock.Mock
    ) -> None:
        manifest, release = fixture()
        expected_by_address = {
            manifest["deterministic_deployer"]["address"]: manifest["deterministic_deployer"]["runtime_code_hash"],
            release["factory_contract"]: release["factory_runtime_code_hash"],
            manifest["reserve_factory"]["address"]: manifest["reserve_factory"]["runtime_code_hash"],
            manifest["reserve_factory"]["implementation"]: manifest["reserve_factory"]["implementation_runtime_code_hash"],
        }
        latest_calls = {manifest["reserve_factory"]["address"]: 0, manifest["reserve_factory"]["implementation"]: 0}

        def code_hash(address: str, block: str = "latest") -> str | None:
            if block == "latest" and address in latest_calls and latest_calls[address] == 0:
                latest_calls[address] += 1
                return None
            return expected_by_address[address]

        client = mock.Mock()
        client.signer.address = "0x" + "dd" * 20
        client.code_hash.side_effect = code_hash
        client.wait_safe.return_value = {
            "number": "0x2a",
            "hash": "0x" + "ee" * 32,
            "timestamp": "0x64",
        }
        send.return_value = {
            "transactionHash": "0x" + "12" * 32,
            "blockNumber": "0x29",
            "blockHash": "0x" + "ff" * 32,
            "gasUsed": "0x100",
            "status": "0x1",
        }
        client.receipt.return_value = send.return_value
        rpc_call.return_value = {"hash": "0x" + "ff" * 32}
        with tempfile.TemporaryDirectory() as directory:
            evidence = deploy.deploy(manifest, release, client, Path(directory) / "evidence.json")
        self.assertTrue(evidence["complete"])
        self.assertFalse(evidence["transaction"]["recovered_exact_deployment"])
        send.assert_called_once_with(client, manifest, None)


if __name__ == "__main__":
    unittest.main()
