import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch

from eth_account import Account
from eth_account.typed_transactions import TypedTransaction
from hexbytes import HexBytes

spec = importlib.util.spec_from_file_location("release_executor", Path(__file__).with_name("execute-posting-sepolia-release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class FakeRpc:
    def __init__(self, bundle, path):
        self.bundle, self.path = bundle, path
        self.receipts, self.sent = {}, []
        self.fail = None
        self.priority, self.balance, self.nonce_offset = 2, release.CAP, 0
        self.runtime_bad, self.reorg, self.chain = False, False, "0x14a34"

    def __call__(self, method, params):
        if method == "eth_chainId":
            return self.chain
        if method == "eth_getBlockByNumber":
            return {"number": params[0] if params[0].startswith("0x") else "0x64", "timestamp": hex(int(time.time())),
                    "hash": "0x" + ("2" if self.reorg else "1") * 64, "baseFeePerGas": "0x2"}
        if method == "eth_getTransactionCount":
            return hex(len(self.receipts) + self.nonce_offset)
        if method == "eth_getBalance":
            return hex(self.balance)
        if method == "eth_maxPriorityFeePerGas":
            return hex(self.priority)
        if method == "eth_call":
            return "0x" + "0" * 63 + "1"
        if method == "eth_estimateGas":
            return hex(100)
        if method == "eth_getTransactionReceipt":
            return self.receipts.get(params[0])
        if method == "eth_getCode":
            address = params[0]
            if address == self.bundle["native_usdc"]:
                return "0x6000"
            deployed = {r["contractAddress"] for r in self.receipts.values()}
            return ("0x6001" if self.runtime_bad else "0x6000") if address in deployed else "0x"
        if method == "eth_sendRawTransaction":
            raw = params[0]
            record = json.loads(self.path.read_text())["records"][-1]
            assert record["raw"] == raw and record["attempts"] >= 1
            assert self.path.stat().st_mode & 0o077 == 0
            self.sent.append(raw)
            if self.fail == "before":
                self.fail = None
                raise RuntimeError("lost request")
            tx = TypedTransaction.from_bytes(HexBytes(raw)).as_dict()
            intent = self.bundle["transactions"][tx["nonce"]]
            digest = release.keccak256(bytes.fromhex(raw[2:]))
            self.receipts[digest] = {"status": "0x1", "blockNumber": "0x60", "blockHash": "0x" + "1" * 64,
                "transactionHash": digest, "from": release.DEPLOYER, "contractAddress": intent["expected_contract"],
                "gasUsed": "0x64", "effectiveGasPrice": "0x2", "l1Fee": "0x1"}
            if self.fail == "after":
                self.fail = None
                raise RuntimeError("lost reply")
            return digest
        raise AssertionError(method)


class ExecutorTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = Path(self.tmp.name) / "journal.json"
        self.signer = Account.from_key("0x" + "01" * 32)
        self.address_patch = patch.object(release, "DEPLOYER", self.signer.address.lower())
        self.address_patch.start()
        self.sleep_patch = patch.object(release.time, "sleep")
        self.sleep_patch.start()
        self.bundle = {"native_usdc": "0x" + "a" * 40, "implementations": {}, "transactions": []}
        for i in range(4):
            self.bundle["transactions"].append({"nonce": i, "name": "fixture-" + str(i), "data": "0x6000",
                "gas_limit": 1000, "expected_contract": release.create_address(release.DEPLOYER, i),
                "expected_runtime_code": "0x6000", "runtime_code_hash": release.keccak256(b"\x60\x00")})
        self.journal = {"bundle_sha256": release.APPROVED, "cap_wei": release.CAP, "records": [], "complete": False}
        self.rpc = FakeRpc(self.bundle, self.path)

    def tearDown(self):
        self.sleep_patch.stop()
        self.address_patch.stop()
        self.tmp.cleanup()

    def run_release(self, **kwargs):
        return release.run(self.rpc, self.bundle, self.journal, self.path, self.signer, **kwargs)

    def test_read_only_never_signs_or_broadcasts(self):
        release.run(self.rpc, self.bundle, self.journal, self.path)
        self.assertEqual(self.rpc.sent, [])
        self.assertFalse(self.path.exists())

    def test_each_broadcast_is_durable_and_reconciled(self):
        self.assertEqual(self.run_release()["confirmed_contracts"], 4)
        self.assertEqual(len(self.rpc.sent), 4)
        self.assertTrue(all(r["receipt"] for r in self.journal["records"]))
        self.run_release()
        self.assertEqual(len(self.rpc.sent), 4)

    def test_lost_reply_recovers_without_duplicate_broadcast(self):
        self.rpc.fail = "after"
        with self.assertRaisesRegex(RuntimeError, "outcome unknown"):
            self.run_release()
        self.journal = json.loads(self.path.read_text())
        self.run_release()
        self.assertEqual(len(self.rpc.sent), 4)

    def test_complete_release_allows_later_fixture_funding_without_redeployment(self):
        self.run_release()
        self.rpc.nonce_offset = 1
        report = release.run(self.rpc, self.bundle, self.journal, self.path)
        self.assertEqual(report["confirmed_contracts"], 4)
        self.assertEqual(report["remaining_fee_bound_wei"], "0")
        self.assertEqual(len(self.rpc.sent), 4)

    def test_unknown_request_requires_explicit_identical_retry(self):
        self.rpc.fail = "before"
        with self.assertRaises(RuntimeError):
            self.run_release()
        saved = self.rpc.sent[0]
        self.journal = json.loads(self.path.read_text())
        with self.assertRaisesRegex(ValueError, "pending/unknown"):
            self.run_release()
        self.assertEqual(self.rpc.sent, [saved])
        self.run_release(retry_saved=True)
        self.assertEqual(self.rpc.sent[1], saved)
        self.assertEqual(len(self.rpc.sent), 5)

    def test_three_attempt_limit(self):
        for _ in range(3):
            self.rpc.fail = "before"
            with self.assertRaises(RuntimeError):
                self.run_release(retry_saved=True)
        with self.assertRaisesRegex(ValueError, "three-attempt"):
            self.run_release(retry_saved=True)
        self.assertEqual(len(self.rpc.sent), 3)

    def test_wrong_chain_nonce_balance_or_cap_stop_before_signing(self):
        for attr, value in [("chain", "0x2105"), ("nonce_offset", 1), ("balance", 0), ("priority", release.CAP)]:
            with self.subTest(attr=attr):
                rpc = FakeRpc(self.bundle, self.path)
                setattr(rpc, attr, value)
                with self.assertRaises(ValueError):
                    release.run(rpc, self.bundle, self.journal, self.path, self.signer)
                self.assertEqual(rpc.sent, [])
                self.assertEqual(self.journal["records"], [])

    def test_runtime_mismatch_or_reorg_stops_later_nonces(self):
        for attr in ["runtime_bad", "reorg"]:
            with self.subTest(attr=attr):
                self.rpc = FakeRpc(self.bundle, self.path)
                self.journal["records"] = []
                setattr(self.rpc, attr, True)
                with self.assertRaises(ValueError):
                    self.run_release()
                self.assertEqual(len(self.rpc.sent), 1)

    def test_saved_signed_scope_cannot_be_changed(self):
        self.run_release()
        record = copy.deepcopy(self.journal["records"][0])
        record["transaction"]["value"] = 1
        with self.assertRaisesRegex(ValueError, "metadata differs"):
            release.validate_saved(record, self.bundle["transactions"][0])


if __name__ == "__main__":
    unittest.main()
