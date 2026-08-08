from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import plan_open_competition_entrant_action as planner  # noqa: E402
import relay_open_competition_entrant_action as relay  # noqa: E402
from test_plan_open_competition_entrant_action import envelope, report  # noqa: E402


class EntrantActionRelayTests(unittest.TestCase):
    def test_plan_validation_rejects_unknown_fields_and_payload_mutation(self) -> None:
        plan = planner.build_plan(report(), "commit", envelope(), None, 300)
        plan["unexpected"] = True
        with self.assertRaisesRegex(Exception, "keys are incomplete or unexpected"):
            relay.validate_plan(plan)
        plan.pop("unexpected")
        plan["payload"] = plan["payload"][:-2] + "00"
        with self.assertRaisesRegex(Exception, "payload hash"):
            relay.validate_plan(plan)

    def test_commit_revalidation_requires_no_plaintext_envelope(self) -> None:
        plan = planner.build_plan(report(), "commit", envelope(), None, 300)
        with self.assertRaisesRegex(Exception, "accepts no plaintext"):
            relay.revalidate_plan("unused", {}, plan, envelope(), None)

    def test_revalidated_fields_cover_every_signed_and_executable_field(self) -> None:
        self.assertIn("payload", relay.REVALIDATED_FIELDS)
        self.assertIn("payload_hash", relay.REVALIDATED_FIELDS)
        self.assertIn("signing_payload", relay.REVALIDATED_FIELDS)
        self.assertIn("relay_call", relay.REVALIDATED_FIELDS)
        self.assertIn("nonce", relay.REVALIDATED_FIELDS)
        self.assertIn("deadline", relay.REVALIDATED_FIELDS)
        self.assertNotIn("safe_block", relay.REVALIDATED_FIELDS)

    def test_validate_action_events_rejects_nonce_that_did_not_advance(self) -> None:
        before = report()
        plan = planner.build_plan(before, "commit", envelope(), None, 300)
        after = copy.deepcopy(before)
        with self.assertRaisesRegex(Exception, "nonce did not advance"):
            relay.validate_action_events_and_balances({"logs": []}, plan, before, after)

    def test_wallet_action_event_layout_matches_the_solidity_event(self) -> None:
        plan = planner.build_plan(report(), "commit", envelope(), None, 300)
        keeper = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        receipt = {
            "logs": [
                {
                    "address": plan["wallet"],
                    "topics": [
                        planner.run_cast(
                            "keccak",
                            input_text="EntrantActionExecuted(uint8,address,address,uint256,bytes32)",
                        ),
                        relay.topic_uint(plan["action_code"]),
                        relay.topic_address(plan["delegate"]),
                        relay.topic_address(keeper),
                    ],
                    "data": relay.topic_uint(plan["nonce"]) + plan["payload_hash"][2:],
                }
            ]
        }
        relay.require_wallet_action_event(receipt, plan, keeper)
        receipt["logs"][0]["topics"][3] = relay.topic_address(
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        )
        with self.assertRaisesRegex(Exception, "exact entrant-wallet action event"):
            relay.require_wallet_action_event(receipt, plan, keeper)


if __name__ == "__main__":
    unittest.main()
