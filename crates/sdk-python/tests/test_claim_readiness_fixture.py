#!/usr/bin/env python3
"""[DIRECT #682] portable claim-readiness diagnostics fixture (offline, exits 0)."""
import json, os, sys, unittest

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURE = os.path.join(HERE, "claim-readiness-fixture.json")

REQUIRED = ("reward_usdc", "refundable_bond_usdc", "external_spend_usdc",
            "gross_cash_margin_usdc", "next_action")
SECRET_WORDS = ("private key", "private_key", "seed phrase", "seed_phrase", "mnemonic")
PAYMENT_WORDS = ("signature", "tx_hash", "transaction hash", "hosted row")
NEXT_ACTIONS = {"claim", "wait_for_recovery_release", "skip", "report_non_creator_failure"}


def load():
    with open(FIXTURE, encoding="utf-8") as fh:
        return json.load(fh)


class ClaimReadinessFixtureTests(unittest.TestCase):
    def setUp(self):
        self.doc = load()
        self.cases = {c["name"]: c for c in self.doc["cases"]}

    def test_fixture_covers_all_four_states(self):
        self.assertEqual(
            {"healthy_direct", "recovery_reserved", "unprofitable", "non_creator_failure"},
            set(self.cases))

    def test_every_case_exposes_required_fields_and_next_action(self):
        for name, case in self.cases.items():
            for field in REQUIRED:
                self.assertIn(field, case, f"{name} missing {field}")
            self.assertIn(case["next_action"], NEXT_ACTIONS, f"{name} bad next_action")

    def test_no_case_requests_a_secret(self):
        for name, case in self.cases.items():
            blob = json.dumps(case).lower()
            for word in SECRET_WORDS:
                self.assertNotIn(word, blob, f"{name} mentions {word}")

    def test_gross_margin_is_distinguished_from_net_profit(self):
        healthy = self.cases["healthy_direct"]
        self.assertAlmostEqual(
            healthy["gross_cash_margin_usdc"],
            healthy["reward_usdc"] - healthy["refundable_bond_usdc"] - healthy["external_spend_usdc"],
            places=6)
        self.assertIn("gross cash margin is not guaranteed net profit", healthy["notes"].lower())

    def test_unprofitable_case_has_negative_margin(self):
        self.assertLess(self.cases["unprofitable"]["gross_cash_margin_usdc"], 0)

    def test_recovery_reserved_is_not_actionable_as_claim(self):
        self.assertNotEqual(self.cases["recovery_reserved"]["next_action"], "claim")

    def test_no_result_describes_non_settlement_as_payment(self):
        for name, case in self.cases.items():
            blob = json.dumps(case).lower()
            for word in PAYMENT_WORDS:
                self.assertNotIn(word, blob, f"{name} describes {word} as payment")

    def test_fixture_is_deterministic(self):
        self.assertEqual(json.dumps(load(), sort_keys=True),
                         json.dumps(load(), sort_keys=True))


if __name__ == "__main__":
    rc = unittest.main(verbosity=2, exit=False).result
    sys.exit(0 if rc.wasSuccessful() else 1)
