#!/usr/bin/env python3

from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

from eth_account import Account
from eth_account.messages import encode_typed_data


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

import build_open_competition_v2_gmv_relay as MODULE


class OpenCompetitionV2GmvRelayTests(unittest.TestCase):
    def setUp(self) -> None:
        self.account = Account.from_key("0x" + "11" * 32)
        self.reserve = "0x2222222222222222222222222222222222222222"
        self.bundle = {
            "schema_version": MODULE.ACTIVATION_SCHEMA,
            "owner": self.account.address.lower(),
            "reserve_factory": "0x3333333333333333333333333333333333333333",
            "reserve_wallet": self.reserve,
            "initial_funding_base_units": 77_668_098,
            "user_salt": "0x" + "44" * 32,
            "approved_creation_commitments": [
                "0x" + f"{index + 1:064x}" for index in range(20)
            ],
            "policy": {
                "delegate": "0x5555555555555555555555555555555555555555",
                "valid_after": 99,
                "valid_until": 10_000,
                "period_seconds": 86_400,
                "solver_reward": 3_000_000,
                "keeper_reward": 40_000,
                "exact_funding_per_competition": 3_040_000,
                "max_per_period": 30_400_000,
                "max_lifetime_spend": 77_668_098,
                "beta_risk_hash": "0x" + "66" * 32,
                "gmv_metric_program_hash": "0x" + "77" * 32,
                "gmv_journal_schema_hash": "0x" + "88" * 32,
            },
            "owner_authorization": {
                "typed_data": {
                    "types": {
                        "EIP712Domain": [
                            {"name": "name", "type": "string"},
                            {"name": "version", "type": "string"},
                            {"name": "chainId", "type": "uint256"},
                            {"name": "verifyingContract", "type": "address"},
                        ],
                        "TransferWithAuthorization": [
                            {"name": "from", "type": "address"},
                            {"name": "to", "type": "address"},
                            {"name": "value", "type": "uint256"},
                            {"name": "validAfter", "type": "uint256"},
                            {"name": "validBefore", "type": "uint256"},
                            {"name": "nonce", "type": "bytes32"},
                        ],
                    },
                    "primaryType": "TransferWithAuthorization",
                    "domain": {
                        "name": "USD Coin",
                        "version": "2",
                        "chainId": 8453,
                        "verifyingContract": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                    },
                    "message": {
                        "from": self.account.address,
                        "to": self.reserve,
                        "value": "77668098",
                        "validAfter": "100",
                        "validBefore": "1000",
                        "nonce": "0x" + "99" * 32,
                    },
                }
            },
        }
        message = encode_typed_data(
            full_message=self.bundle["owner_authorization"]["typed_data"]
        )
        self.signature = "0x" + Account.sign_message(message, self.account.key).signature.hex()

    def test_verified_signature_builds_exact_factory_call(self) -> None:
        relay = MODULE.build_relay(self.bundle, self.signature, now=500)
        self.assertEqual(relay["to"], self.bundle["reserve_factory"])
        self.assertEqual(relay["expected_owner"], self.account.address.lower())
        self.assertEqual(relay["expected_reserve_wallet"], self.reserve)
        self.assertEqual(relay["expected_funding_base_units"], 77_668_098)
        self.assertTrue(relay["data"].startswith("0x"))
        self.assertGreater(len(relay["data"]), 2_000)

    def test_wrong_signer_amount_or_time_fails_closed(self) -> None:
        other = Account.from_key("0x" + "22" * 32)
        message = encode_typed_data(
            full_message=self.bundle["owner_authorization"]["typed_data"]
        )
        wrong = "0x" + Account.sign_message(message, other.key).signature.hex()
        with self.assertRaises(MODULE.RelayError):
            MODULE.build_relay(self.bundle, wrong, now=500)
        changed = copy.deepcopy(self.bundle)
        changed["initial_funding_base_units"] += 1
        with self.assertRaises(MODULE.RelayError):
            MODULE.build_relay(changed, self.signature, now=500)
        with self.assertRaises(MODULE.RelayError):
            MODULE.build_relay(self.bundle, self.signature, now=1000)

    def test_superseded_activation_schema_fails_closed(self) -> None:
        legacy = copy.deepcopy(self.bundle)
        legacy["schema_version"] = (
            "agent-bounties/open-competition-v2-gmv-meta-activation-v1"
        )
        with self.assertRaisesRegex(MODULE.RelayError, "schema"):
            MODULE.build_relay(legacy, self.signature, now=500)


if __name__ == "__main__":
    unittest.main()
