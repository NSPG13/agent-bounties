import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch


PATH = Path(__file__).with_name("fund_open_competition_v2_beta3_broker.py")
SPEC = importlib.util.spec_from_file_location("fund_open_competition_v2_beta3_broker", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class BrokerFundingTests(unittest.TestCase):
    def test_deficits_are_idempotent(self):
        self.assertEqual(
            MODULE.deficits(current_usdc=0, current_eth=0, target_usdc=110_000, target_eth=100),
            (110_000, 100),
        )
        self.assertEqual(
            MODULE.deficits(
                current_usdc=200_000,
                current_eth=200,
                target_usdc=110_000,
                target_eth=100,
            ),
            (0, 0),
        )

    def test_deficits_reject_invalid_targets(self):
        self.assertEqual(
            MODULE.deficits(current_usdc=0, current_eth=0, target_usdc=0, target_eth=100),
            (0, 100),
        )
        with self.assertRaisesRegex(MODULE.BrokerFundingError, "are invalid"):
            MODULE.deficits(current_usdc=0, current_eth=0, target_usdc=0, target_eth=0)

    def test_zero_usdc_seed_does_not_require_an_unrelated_usdc_reserve(self):
        MODULE.require_deployer_capacity(
            deployer_usdc=0,
            deployer_eth=300,
            usdc_deficit=0,
            eth_deficit=100,
            minimum_deployer_usdc_after=635_000,
            minimum_deployer_eth_after=200,
        )

    def test_positive_usdc_seed_still_preserves_the_canary_reserve(self):
        with self.assertRaisesRegex(MODULE.BrokerFundingError, "canary budget"):
            MODULE.require_deployer_capacity(
                deployer_usdc=635_000,
                deployer_eth=300,
                usdc_deficit=1,
                eth_deficit=100,
                minimum_deployer_usdc_after=635_000,
                minimum_deployer_eth_after=200,
            )

    def test_signing_address_accepts_canonical_lowercase_broker(self):
        destination = MODULE.signing_address("0x176f486a724720c4fdfc920d7c17dd1004c2bfb4")
        self.assertEqual(destination, "0x176f486A724720C4FDfc920d7c17Dd1004C2bfb4")
        signer = MODULE.Account.from_key("0x" + "01".zfill(64))
        signed = signer.sign_transaction(
            {
                "chainId": 84532,
                "to": destination,
                "nonce": 0,
                "value": 1,
                "gas": 21_000,
                "maxFeePerGas": 2_000_000,
                "maxPriorityFeePerGas": 1_000_000,
                "type": 2,
            }
        )
        self.assertTrue(signed.raw_transaction)

    def test_signing_address_rejects_malformed_destination(self):
        with self.assertRaisesRegex(MODULE.BrokerFundingError, "destination is invalid"):
            MODULE.signing_address("0x1234")

    def test_signed_rpc_falls_back_with_the_identical_raw_transaction(self):
        expected_hash = "0x" + "ab" * 32
        calls = []

        class Signer:
            address = "0x0000000000000000000000000000000000000001"

            @staticmethod
            def sign_transaction(_transaction):
                return SimpleNamespace(
                    raw_transaction=bytes.fromhex("1234"),
                    hash=bytes.fromhex("ab" * 32),
                )

        def fake_rpc(url, method, params):
            calls.append((url, method, params))
            if method == "eth_chainId":
                return hex(8453)
            if method == "eth_getTransactionCount":
                return "0x18"
            if method == "eth_getBlockByNumber":
                return {"baseFeePerGas": "0x1"}
            if method == "eth_maxPriorityFeePerGas":
                return "0xf4240"
            if method == "eth_estimateGas":
                return "0x5208"
            if method == "eth_sendRawTransaction" and url == "https://primary.invalid":
                raise RuntimeError("rate limit exceeded")
            if method == "eth_sendRawTransaction":
                return expected_hash
            if method == "eth_getTransactionReceipt" and url == "https://shadow.invalid":
                return {
                    "transactionHash": expected_hash,
                    "blockNumber": "0x2",
                    "status": "0x1",
                }
            if method == "eth_getTransactionReceipt":
                return None
            self.fail(f"unexpected RPC call: {url} {method} {params}")

        with patch.object(MODULE, "rpc", side_effect=fake_rpc):
            client = MODULE.SignedRpc(
                "https://primary.invalid",
                Signer(),
                8453,
                broadcast_urls=["https://shadow.invalid"],
            )
            receipt = client.send(
                to="0x0000000000000000000000000000000000000002"
            )

        self.assertEqual(receipt["transactionHash"], expected_hash)
        submissions = [call for call in calls if call[1] == "eth_sendRawTransaction"]
        self.assertEqual(
            submissions,
            [
                (
                    "https://primary.invalid",
                    "eth_sendRawTransaction",
                    ["0x1234"],
                ),
                (
                    "https://shadow.invalid",
                    "eth_sendRawTransaction",
                    ["0x1234"],
                ),
            ],
        )

    def test_signed_rpc_rejects_a_mismatched_transaction_hash(self):
        class Signer:
            address = "0x0000000000000000000000000000000000000001"

            @staticmethod
            def sign_transaction(_transaction):
                return SimpleNamespace(
                    raw_transaction=bytes.fromhex("1234"),
                    hash=bytes.fromhex("ab" * 32),
                )

        def fake_rpc(_url, method, _params):
            if method == "eth_chainId":
                return hex(8453)
            if method == "eth_getTransactionCount":
                return "0x18"
            if method == "eth_getBlockByNumber":
                return {"baseFeePerGas": "0x1"}
            if method == "eth_maxPriorityFeePerGas":
                return "0xf4240"
            if method == "eth_estimateGas":
                return "0x5208"
            if method == "eth_sendRawTransaction":
                return "0x" + "cd" * 32
            self.fail(f"unexpected RPC call: {method}")

        with patch.object(MODULE, "rpc", side_effect=fake_rpc):
            client = MODULE.SignedRpc("https://primary.invalid", Signer(), 8453)
            with self.assertRaisesRegex(
                MODULE.BrokerFundingError,
                "unexpected transaction hash",
            ):
                client.send(to="0x0000000000000000000000000000000000000002")

    def test_signed_rpc_accepts_an_already_known_receipt(self):
        expected_hash = "0x" + "ab" * 32

        class Signer:
            address = "0x0000000000000000000000000000000000000001"

            @staticmethod
            def sign_transaction(_transaction):
                return SimpleNamespace(
                    raw_transaction=bytes.fromhex("1234"),
                    hash=bytes.fromhex("ab" * 32),
                )

        def fake_rpc(_url, method, _params):
            if method == "eth_chainId":
                return hex(8453)
            if method == "eth_getTransactionCount":
                return "0x18"
            if method == "eth_getBlockByNumber":
                return {"baseFeePerGas": "0x1"}
            if method == "eth_maxPriorityFeePerGas":
                return "0xf4240"
            if method == "eth_estimateGas":
                return "0x5208"
            if method == "eth_sendRawTransaction":
                raise RuntimeError("already known")
            if method == "eth_getTransactionReceipt":
                return {
                    "transactionHash": expected_hash,
                    "blockNumber": "0x2",
                    "status": "0x1",
                }
            self.fail(f"unexpected RPC call: {method}")

        with patch.object(MODULE, "rpc", side_effect=fake_rpc):
            client = MODULE.SignedRpc(
                "https://primary.invalid",
                Signer(),
                8453,
                broadcast_urls=["https://shadow.invalid"],
            )
            receipt = client.send(
                to="0x0000000000000000000000000000000000000002"
            )

        self.assertEqual(receipt["transactionHash"], expected_hash)

    def test_signed_rpc_accepts_an_already_known_pending_transaction(self):
        expected_hash = "0x" + "ab" * 32

        class Signer:
            address = "0x0000000000000000000000000000000000000001"

            @staticmethod
            def sign_transaction(_transaction):
                return SimpleNamespace(
                    raw_transaction=bytes.fromhex("1234"),
                    hash=bytes.fromhex("ab" * 32),
                )

        receipt_calls = 0

        def fake_rpc(url, method, _params):
            nonlocal receipt_calls
            if method == "eth_chainId":
                return hex(8453)
            if method == "eth_getTransactionCount":
                return "0x18"
            if method == "eth_getBlockByNumber":
                return {"baseFeePerGas": "0x1"}
            if method == "eth_maxPriorityFeePerGas":
                return "0xf4240"
            if method == "eth_estimateGas":
                return "0x5208"
            if method == "eth_sendRawTransaction":
                raise RuntimeError("already known")
            if method == "eth_getTransactionReceipt":
                receipt_calls += 1
                if receipt_calls <= 2:
                    return None
                return {
                    "transactionHash": expected_hash,
                    "blockNumber": "0x2",
                    "status": "0x1",
                }
            if method == "eth_getTransactionByHash" and url == "https://primary.invalid":
                return None
            if method == "eth_getTransactionByHash":
                return {"hash": expected_hash, "blockNumber": None}
            self.fail(f"unexpected RPC call: {url} {method}")

        with patch.object(MODULE, "rpc", side_effect=fake_rpc), patch.object(
            MODULE.time, "sleep"
        ):
            client = MODULE.SignedRpc(
                "https://primary.invalid",
                Signer(),
                8453,
                broadcast_urls=["https://shadow.invalid"],
            )
            receipt = client.send(
                to="0x0000000000000000000000000000000000000002"
            )

        self.assertEqual(receipt["transactionHash"], expected_hash)

    def test_signed_rpc_fails_when_every_endpoint_rejects_without_a_receipt(self):
        class Signer:
            address = "0x0000000000000000000000000000000000000001"

            @staticmethod
            def sign_transaction(_transaction):
                return SimpleNamespace(
                    raw_transaction=bytes.fromhex("1234"),
                    hash=bytes.fromhex("ab" * 32),
                )

        def fake_rpc(_url, method, _params):
            if method == "eth_chainId":
                return hex(8453)
            if method == "eth_getTransactionCount":
                return "0x18"
            if method == "eth_getBlockByNumber":
                return {"baseFeePerGas": "0x1"}
            if method == "eth_maxPriorityFeePerGas":
                return "0xf4240"
            if method == "eth_estimateGas":
                return "0x5208"
            if method == "eth_sendRawTransaction":
                raise RuntimeError("rate limit exceeded")
            if method == "eth_getTransactionReceipt":
                return None
            if method == "eth_getTransactionByHash":
                return None
            self.fail(f"unexpected RPC call: {method}")

        with patch.object(MODULE, "rpc", side_effect=fake_rpc):
            client = MODULE.SignedRpc(
                "https://primary.invalid",
                Signer(),
                8453,
                broadcast_urls=["https://shadow.invalid"],
            )
            with self.assertRaisesRegex(
                MODULE.BrokerFundingError,
                "every approved RPC",
            ):
                client.send(to="0x0000000000000000000000000000000000000002")


if __name__ == "__main__":
    unittest.main()
