#!/usr/bin/env python3
"""Tests for exact bounded reserve-factory deployment."""

from __future__ import annotations

import copy
import tempfile
from pathlib import Path
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
        }
        rpc_call.return_value = {"hash": "0x" + "ff" * 32}
        with tempfile.TemporaryDirectory() as directory:
            evidence = deploy.deploy(manifest, release, client, Path(directory) / "evidence.json")
        self.assertTrue(evidence["complete"])
        self.assertFalse(evidence["transaction"]["recovered_exact_deployment"])
        send.assert_called_once_with(client, manifest)


if __name__ == "__main__":
    unittest.main()
