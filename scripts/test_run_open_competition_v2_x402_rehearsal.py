import importlib.util
import json
from pathlib import Path
import unittest

from eth_account import Account


PATH = Path(__file__).with_name("run_open_competition_v2_x402_rehearsal.py")
SPEC = importlib.util.spec_from_file_location("open_competition_v2_x402_rehearsal", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class X402RehearsalTests(unittest.TestCase):
    def test_payment_header_is_standard_exact_eip3009(self):
        actor = Account.from_key("0x" + "11" * 32)
        challenge = {
            "x402Version": 2,
            "resource": {"url": "https://example.test/proof"},
            "accepts": [
                {
                    "scheme": "exact",
                    "network": "eip155:84532",
                    "asset": "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
                    "amount": "100000",
                    "payTo": "0x2222222222222222222222222222222222222222",
                    "maxTimeoutSeconds": 300,
                    "extra": {
                        "assetTransferMethod": "eip3009",
                        "name": "USDC",
                        "version": "2",
                    },
                }
            ],
        }
        encoded = MODULE.sign_payment(actor, challenge)
        payload = json.loads(__import__("base64").b64decode(encoded))
        self.assertEqual(payload["accepted"]["scheme"], "exact")
        self.assertEqual(payload["payload"]["authorization"]["from"], actor.address)
        self.assertRegex(payload["payload"]["signature"], r"^0x[0-9a-f]{130}$")

    def test_payment_header_supports_base_mainnet_without_changing_asset(self):
        actor = Account.from_key("0x" + "11" * 32)
        challenge = {
            "x402Version": 2,
            "resource": {"url": "https://example.test/proof"},
            "accepts": [
                {
                    "scheme": "exact",
                    "network": "eip155:8453",
                    "asset": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                    "amount": "100000",
                    "payTo": "0x2222222222222222222222222222222222222222",
                    "maxTimeoutSeconds": 300,
                    "extra": {"assetTransferMethod": "eip3009", "name": "USDC", "version": "2"},
                }
            ],
        }
        encoded = MODULE.sign_payment(
            actor, challenge, chain_id=8453, network="eip155:8453"
        )
        payload = json.loads(__import__("base64").b64decode(encoded))
        self.assertEqual(payload["accepted"]["network"], "eip155:8453")
        self.assertEqual(payload["accepted"]["asset"], challenge["accepts"][0]["asset"])

    def test_header_decoder_rejects_non_json(self):
        with self.assertRaises(MODULE.X402RehearsalError):
            MODULE.decode_x402_header("bm90LWpzb24=")

    def test_quote_payload_uses_requested_network(self):
        spec = {
            "competition": "0x" + "11" * 20,
            "solver": "0x" + "22" * 20,
            "solver_nonce": 7,
            "artifact_hash": "0x" + "33" * 32,
            "metric": {"score": 42},
        }
        payload = MODULE.build_quote_payload(spec, "base-mainnet")
        self.assertEqual(payload["network"], "base-mainnet")
        self.assertEqual(payload["competition_contract"], spec["competition"])
        self.assertTrue(payload["relay"])


if __name__ == "__main__":
    unittest.main()
