import importlib.util
from pathlib import Path
import unittest


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


if __name__ == "__main__":
    unittest.main()
