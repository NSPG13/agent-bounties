import copy
import json
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest import mock

import deploy_open_competition_v2_beta3 as deploy


def fixture() -> dict:
    subject = "0x" + "11" * 32
    gates = {name: True for name in deploy.release.PRELAUNCH_GATE_NAMES}
    evidence = {
        name: {
            "subject_hash": subject,
            "source_commit": "22" * 20,
            "evidence_hash": "0x" + "33" * 32,
            "uri": "https://example.test/evidence",
        }
        for name in deploy.release.PRELAUNCH_GATE_NAMES
    }
    components = ("groth16_verifier", "plonk_verifier", "factory")
    transactions = [
        {
            "component": name,
            "from_nonce": 4 + index,
            "predicted_address": f"0x{index + 1:040x}",
            "data": "0x01",
        }
        for index, name in enumerate(components)
    ]
    value = {
        "schema_version": "agent-bounties/open-competition-v2-beta3-release-bundle-v1",
        "protocol_version": "agent-bounties/open-competition-v2-beta3",
        "network": "base-mainnet",
        "chain_id": 8453,
        "source_commit": "22" * 20,
        "deployer": "0x1111111111111111111111111111111111111111",
        "repository_subject": {"hash": subject},
        "activation": {"mainnet_signing_allowed": True},
        "release_gates": {
            "prelaunch_complete": True,
            "gates": gates,
            "evidence": evidence,
        },
        "deployment_transactions": transactions,
    }
    for transaction in transactions:
        value[transaction["component"]] = {"address": transaction["predicted_address"]}
    return value


class DeploymentValidationTests(unittest.TestCase):
    def test_accepts_exact_approved_bundle(self) -> None:
        deploy.validate_bundle(fixture(), "0x1111111111111111111111111111111111111111")

    def test_rejects_false_gate(self) -> None:
        value = fixture()
        value["release_gates"]["gates"][deploy.release.PRELAUNCH_GATE_NAMES[0]] = False
        with self.assertRaisesRegex(RuntimeError, "prelaunch gate is false"):
            deploy.validate_bundle(value, value["deployer"])

    def test_rejects_wrong_signer_or_nonce_order(self) -> None:
        value = fixture()
        with self.assertRaisesRegex(RuntimeError, "signer differs"):
            deploy.validate_bundle(value, "0x2222222222222222222222222222222222222222")
        value["deployment_transactions"][1]["from_nonce"] = 9
        with self.assertRaisesRegex(RuntimeError, "nonces are not contiguous"):
            deploy.validate_bundle(value, value["deployer"])

    def test_resume_evidence_must_match_the_exact_release(self) -> None:
        value = fixture()
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "deployment.json"
            evidence = deploy.initial_evidence(value)
            output.write_text(json.dumps(evidence), encoding="utf-8")
            self.assertEqual(deploy.load_evidence(value, output), evidence)
            value["repository_subject"]["hash"] = "0x" + "ff" * 32
            with self.assertRaisesRegex(RuntimeError, "another release"):
                deploy.load_evidence(value, output)

    def test_refreshes_a_reincluded_transaction_to_its_canonical_receipt(self) -> None:
        transaction_hash = "0x" + "12" * 32
        contract = "0x" + "34" * 20
        evidence = {
            "transactions": [
                {
                    "transaction_hash": transaction_hash,
                    "block_number": 10,
                    "block_hash": "0x" + "56" * 32,
                    "contract_address": contract,
                    "gas_used": 1,
                },
                {
                    "transaction_hash": None,
                    "block_number": 11,
                    "block_hash": "0x" + "78" * 32,
                    "contract_address": "0x" + "90" * 20,
                    "gas_used": None,
                },
            ]
        }
        client = SimpleNamespace(
            receipt=mock.Mock(
                return_value={
                    "transactionHash": transaction_hash,
                    "status": "0x1",
                    "blockNumber": "0xc",
                    "blockHash": "0x" + "ab" * 32,
                    "contractAddress": contract,
                    "gasUsed": "0x2a",
                }
            )
        )

        deployment_block = deploy.reconcile_canonical_receipts(evidence, client)

        self.assertEqual(deployment_block, 12)
        self.assertEqual(evidence["transactions"][0]["block_number"], 12)
        self.assertEqual(evidence["transactions"][0]["block_hash"], "0x" + "ab" * 32)
        self.assertEqual(evidence["transactions"][0]["gas_used"], 42)

    def test_rejects_a_missing_canonical_receipt(self) -> None:
        evidence = {
            "transactions": [
                {
                    "transaction_hash": "0x" + "12" * 32,
                    "block_number": 10,
                    "block_hash": "0x" + "56" * 32,
                    "contract_address": "0x" + "34" * 20,
                    "gas_used": 1,
                }
            ]
        }
        client = SimpleNamespace(receipt=mock.Mock(return_value=None))

        with self.assertRaisesRegex(RuntimeError, "canonical deployment receipt is absent"):
            deploy.reconcile_canonical_receipts(evidence, client)

    def test_safe_runtime_check_uses_one_canonical_safe_block(self) -> None:
        client = object.__new__(deploy.SignedRpc)
        client.wait_safe = mock.Mock(return_value={"number": "0x2a"})
        client.code_hash = mock.Mock(return_value="0xexact")

        addresses = ("0x" + "12" * 20, "0x" + "34" * 20)
        client.require_safe_code_hashes(
            {address: "0xexact" for address in addresses}, 41
        )

        client.wait_safe.assert_called_once_with(41)
        self.assertEqual(
            client.code_hash.call_args_list,
            [mock.call(address, "0x2a") for address in addresses],
        )

    @mock.patch.object(deploy.time, "sleep")
    @mock.patch.object(deploy.time, "time", side_effect=[0, 1, 2])
    @mock.patch.object(
        deploy,
        "rpc",
        side_effect=[RuntimeError("HTTP 408"), {"number": "0x2a"}],
    )
    def test_safe_block_polling_recovers_from_transport_error(
        self, rpc_call: mock.Mock, _time: mock.Mock, sleep: mock.Mock
    ) -> None:
        client = object.__new__(deploy.SignedRpc)
        client.url = "https://rpc.example"

        result = client.wait_safe(41, timeout_seconds=10)

        self.assertEqual(result["number"], "0x2a")
        self.assertEqual(rpc_call.call_count, 2)
        sleep.assert_called_once_with(5)

    def test_broadcasts_identical_raw_transaction_to_shadow_after_primary_failure(
        self,
    ) -> None:
        primary = "https://primary.example"
        shadow = "https://shadow.example"
        expected_hash = "0x" + "12" * 32
        raw_transaction = "0xaabb"
        calls: list[tuple[str, str, list[object]]] = []

        def fake_rpc(url: str, method: str, params: list[object]):
            calls.append((url, method, params))
            if method == "eth_chainId":
                return "0x2105"
            if method == "eth_sendRawTransaction" and url == primary:
                raise RuntimeError("HTTP 429")
            if method == "eth_sendRawTransaction":
                return expected_hash
            if method == "eth_getTransactionReceipt" and url == primary:
                raise RuntimeError("HTTP 429")
            if method == "eth_getTransactionReceipt":
                return {
                    "transactionHash": expected_hash,
                    "status": "0x1",
                }
            raise AssertionError((url, method, params))

        signed = SimpleNamespace(
            raw_transaction=bytes.fromhex(raw_transaction[2:]),
            hash=bytes.fromhex(expected_hash[2:]),
        )
        with mock.patch.object(deploy, "rpc", side_effect=fake_rpc):
            client = deploy.SignedRpc(primary, SimpleNamespace(address="0x" + "11" * 20), shadow)
            self.assertEqual(client.broadcast(signed), expected_hash)
            self.assertEqual(client.receipt(expected_hash)["status"], "0x1")

        submissions = [
            (url, params[0])
            for url, method, params in calls
            if method == "eth_sendRawTransaction"
        ]
        self.assertEqual(
            submissions,
            [(primary, raw_transaction), (shadow, raw_transaction)],
        )

    def test_rejects_rpc_transaction_hash_mismatch(self) -> None:
        primary = "https://primary.example"
        expected_hash = "0x" + "12" * 32

        def fake_rpc(_url: str, method: str, _params: list[object]):
            if method == "eth_chainId":
                return "0x2105"
            if method == "eth_sendRawTransaction":
                return "0x" + "34" * 32
            raise AssertionError(method)

        signed = SimpleNamespace(
            raw_transaction=bytes.fromhex("aabb"),
            hash=bytes.fromhex(expected_hash[2:]),
        )
        with mock.patch.object(deploy, "rpc", side_effect=fake_rpc):
            client = deploy.SignedRpc(primary, SimpleNamespace(address="0x" + "11" * 20))
            with self.assertRaisesRegex(RuntimeError, "unexpected transaction hash"):
                client.broadcast(signed)

    def test_accepts_already_mined_transaction_when_broadcast_responses_fail(self) -> None:
        primary = "https://primary.example"
        shadow = "https://shadow.example"
        expected_hash = "0x" + "12" * 32

        def fake_rpc(url: str, method: str, _params: list[object]):
            if method == "eth_chainId":
                return "0x2105"
            if method == "eth_sendRawTransaction":
                raise RuntimeError("already known")
            if method == "eth_getTransactionReceipt" and url == primary:
                return None
            if method == "eth_getTransactionReceipt":
                return {"transactionHash": expected_hash, "status": "0x1"}
            raise AssertionError((url, method))

        signed = SimpleNamespace(
            raw_transaction=bytes.fromhex("aabb"),
            hash=bytes.fromhex(expected_hash[2:]),
        )
        with mock.patch.object(deploy, "rpc", side_effect=fake_rpc):
            client = deploy.SignedRpc(primary, SimpleNamespace(address="0x" + "11" * 20), shadow)
            self.assertEqual(client.broadcast(signed), expected_hash)

    def test_accepts_pending_transaction_when_broadcast_responses_fail(self) -> None:
        primary = "https://primary.example"
        shadow = "https://shadow.example"
        expected_hash = "0x" + "12" * 32

        def fake_rpc(url: str, method: str, _params: list[object]):
            if method == "eth_chainId":
                return "0x2105"
            if method == "eth_sendRawTransaction":
                raise RuntimeError("already known")
            if method == "eth_getTransactionReceipt":
                return None
            if method == "eth_getTransactionByHash" and url == primary:
                return None
            if method == "eth_getTransactionByHash":
                return {"hash": expected_hash, "blockNumber": None}
            raise AssertionError((url, method))

        signed = SimpleNamespace(
            raw_transaction=bytes.fromhex("aabb"),
            hash=bytes.fromhex(expected_hash[2:]),
        )
        with mock.patch.object(deploy, "rpc", side_effect=fake_rpc):
            client = deploy.SignedRpc(primary, SimpleNamespace(address="0x" + "11" * 20), shadow)
            self.assertEqual(client.broadcast(signed), expected_hash)

    def test_fails_when_every_rpc_rejects_and_transaction_is_absent(self) -> None:
        primary = "https://primary.example"
        shadow = "https://shadow.example"
        expected_hash = "0x" + "12" * 32

        def fake_rpc(_url: str, method: str, _params: list[object]):
            if method == "eth_chainId":
                return "0x2105"
            if method == "eth_sendRawTransaction":
                raise RuntimeError("rate limit")
            if method in {"eth_getTransactionReceipt", "eth_getTransactionByHash"}:
                return None
            raise AssertionError(method)

        signed = SimpleNamespace(
            raw_transaction=bytes.fromhex("aabb"),
            hash=bytes.fromhex(expected_hash[2:]),
        )
        with mock.patch.object(deploy, "rpc", side_effect=fake_rpc):
            client = deploy.SignedRpc(primary, SimpleNamespace(address="0x" + "11" * 20), shadow)
            with self.assertRaisesRegex(RuntimeError, "every approved RPC"):
                client.broadcast(signed)


if __name__ == "__main__":
    unittest.main()
