from __future__ import annotations

import sys
import unittest
from pathlib import Path

from eth_abi import encode


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import inspect_open_competition_v2_reward_policy as MODULE  # noqa: E402


class InspectRewardPolicyTests(unittest.TestCase):
    def test_policy_output_schema_decodes_every_enforced_field(self) -> None:
        values = (
            "0x" + "11" * 20,
            1,
            2,
            86_400,
            3_000_000,
            40_000,
            3_040_000,
            30_400_000,
            77_668_098,
            bytes.fromhex("22" * 32),
            bytes.fromhex("33" * 32),
            bytes.fromhex("44" * 32),
        )
        encoded = encode(list(MODULE.POLICY_OUTPUTS), list(values))
        from eth_abi import decode

        self.assertEqual(decode(MODULE.POLICY_OUTPUTS, encoded), values)

    def test_selector_is_exact_and_stable(self) -> None:
        self.assertEqual(MODULE.selector("owner()").hex(), "8da5cb5b")
        self.assertEqual(MODULE.selector("policy()").hex(), "0505c8c9")


if __name__ == "__main__":
    unittest.main()
