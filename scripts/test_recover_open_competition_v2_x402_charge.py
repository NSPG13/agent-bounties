import unittest

from recover_open_competition_v2_x402_charge import (
    TRANSFER_TOPIC,
    address_topic,
    is_transfer,
)


class X402ChargeRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.asset = "0x" + "11" * 20
        self.sender = "0x" + "22" * 20
        self.recipient = "0x" + "33" * 20
        self.log = {
            "address": self.asset,
            "topics": [
                TRANSFER_TOPIC,
                address_topic(self.sender),
                address_topic(self.recipient),
            ],
            "data": hex(110_000),
        }

    def test_matches_only_exact_transfer(self):
        self.assertTrue(
            is_transfer(
                self.log,
                asset=self.asset,
                sender=self.sender,
                recipient=self.recipient,
                amount=110_000,
            )
        )
        self.assertFalse(
            is_transfer(
                self.log,
                asset=self.asset,
                sender=self.sender,
                recipient=self.recipient,
                amount=109_999,
            )
        )

    def test_rejects_wrong_counterparty(self):
        self.assertFalse(
            is_transfer(
                self.log,
                asset=self.asset,
                sender=self.recipient,
                recipient=self.sender,
                amount=110_000,
            )
        )


if __name__ == "__main__":
    unittest.main()
