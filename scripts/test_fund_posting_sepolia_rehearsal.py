"""Regression gates for exact test-only funding intents and aggregate fee limits."""
import copy
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

from eth_account import Account
from eth_utils import to_checksum_address

spec = importlib.util.spec_from_file_location("funding", Path(__file__).with_name("fund-posting-sepolia-rehearsal.py"))
funding = importlib.util.module_from_spec(spec)
spec.loader.exec_module(funding)


class FundingTests(unittest.TestCase):
    def setUp(self):
        self.signer = Account.from_key(bytes.fromhex("11" * 32))
        self.plan = funding.intents()[0]
        self.addresses = patch.dict(funding.ADDRESSES, deployer=self.signer.address.lower())
        self.addresses.start()
        self.addCleanup(self.addresses.stop)

    def record(self, **changes):
        tx = {k: v for k, v in self.plan.items() if k != "role"}
        tx.update(chainId=84532, type=2, to=to_checksum_address(tx["to"]), maxFeePerGas=100, maxPriorityFeePerGas=1)
        tx.update(changes)
        raw = self.signer.sign_transaction(tx).raw_transaction
        return {"raw": "0x" + raw.hex(), "hash": funding.keccak256(raw)}

    def test_exact_intent(self):
        self.assertEqual(funding.validate_saved(self.record(), self.plan), 100)

    def test_mutations_rejected(self):
        for change in ({"chainId": 8453}, {"nonce": 5}, {"value": 10**18}, {"gas": 22000},
                       {"data": "0x1234"}, {"to": self.signer.address}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                funding.validate_saved(self.record(**change), self.plan)
        record = self.record()
        record["hash"] = "0x" + "00" * 32
        with self.assertRaises(ValueError):
            funding.validate_saved(record, self.plan)

    def test_cumulative_cap_includes_confirmed_fees(self):
        records = [{"receipt": {"fee_wei": str(funding.FEE_CAP - 2)}}]
        with patch.object(funding.executor, "quote", return_value=2), self.assertRaises(ValueError):
            funding.reserve(None, funding.intents(), records, 1, 100)

    def test_three_destinations_and_no_customer_eth(self):
        plans = funding.intents()
        self.assertEqual([p["nonce"] for p in plans], [4, 0, 1])
        self.assertEqual(plans[0]["value"], 5 * 10**14)
        self.assertEqual(plans[0]["to"], funding.ADDRESSES["relayer"])
        self.assertEqual([p["value"] for p in plans[1:]], [0, 0])
        self.assertTrue(all(p["to"] == funding.USDC for p in plans[1:]))


if __name__ == "__main__":
    unittest.main()
