import importlib.util
from pathlib import Path
import unittest


PATH = Path(__file__).with_name("verify_open_competition_v2_beta3_mainnet_recovery.py")
SPEC = importlib.util.spec_from_file_location("beta3_mainnet_recovery", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class RecoveryEvidenceTests(unittest.TestCase):
    def test_exact_transfer_matches(self):
        sender = "0x" + "11" * 20
        recipient = "0x" + "22" * 20
        token = "0x" + "33" * 20
        receipt = {
            "logs": [
                {
                    "address": token,
                    "topics": [
                        MODULE.TRANSFER_TOPIC,
                        MODULE.topic_address(sender),
                        MODULE.topic_address(recipient),
                    ],
                    "data": hex(110_000),
                }
            ]
        }
        MODULE.require_transfer(receipt, token, sender, recipient, 110_000)

    def test_wrong_transfer_is_rejected(self):
        sender = "0x" + "11" * 20
        recipient = "0x" + "22" * 20
        token = "0x" + "33" * 20
        receipt = {
            "logs": [
                {
                    "address": token,
                    "topics": [
                        MODULE.TRANSFER_TOPIC,
                        MODULE.topic_address(sender),
                        MODULE.topic_address(recipient),
                    ],
                    "data": hex(109_999),
                }
            ]
        }
        with self.assertRaises(MODULE.RecoveryEvidenceError):
            MODULE.require_transfer(receipt, token, sender, recipient, 110_000)


if __name__ == "__main__":
    unittest.main()
