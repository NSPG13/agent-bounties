#!/usr/bin/env python3
"""[DIRECT #683] ready-to-earn inventory filter regression test (offline, exits 0)."""
import json, os, sys, unittest

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURE = os.path.join(HERE, "ready-to-earn-inventory.json")
TERMINAL = {"settled", "cancelled", "expired", "refunded"}


def load():
    with open(FIXTURE, encoding="utf-8") as fh:
        return json.load(fh)


def ready_to_earn(row):
    """Canonical projection: only funded + claimable + verification-ready + valid terms."""
    if row["status"] in TERMINAL:
        return False
    return (row["status"] == "funded" and row["claimable"]
            and row["verification_ready"] and not row["recovery_reserved"]
            and row["terms_valid"])


class ReadyToEarnFilterTests(unittest.TestCase):
    def setUp(self):
        self.feed = load()["lifecycle_feed"]
        self.by_id = {r["id"]: r for r in self.feed}
        self.ready = [r["id"] for r in self.feed if ready_to_earn(r)]
        self.excluded = [r["id"] for r in self.feed if not ready_to_earn(r)]

    def test_exactly_one_healthy_bounty_is_ready(self):
        self.assertEqual(self.ready, ["B-2001"])

    def test_at_least_three_states_are_excluded(self):
        self.assertGreaterEqual(len(self.excluded), 3)

    def test_every_excluded_row_is_absent_from_projection(self):
        for bid in self.excluded:
            self.assertNotIn(bid, self.ready)

    def test_each_exclusion_reason_is_specific(self):
        expected = {"B-2002": "verification_ready_false", "B-2003": "recovery_reserved",
                    "B-2004": "invalid_terms", "B-2005": "terminal_status"}
        for bid, reason in expected.items():
            self.assertEqual(self.by_id[bid]["exclusion_reason"], reason)

    def test_excluded_rows_stay_visible_in_lifecycle_feed(self):
        for bid in self.excluded:
            self.assertIn(bid, self.by_id)
            row = self.by_id[bid]
            self.assertTrue(row["exclusion_reason"])
            self.assertIn("status", row)

    def test_no_terminal_row_is_ever_ready(self):
        for row in self.feed:
            if row["status"] in TERMINAL:
                self.assertFalse(ready_to_earn(row))

    def test_projection_is_deterministic(self):
        again = [r["id"] for r in load()["lifecycle_feed"] if ready_to_earn(r)]
        self.assertEqual(self.ready, again)


if __name__ == "__main__":
    rc = unittest.main(verbosity=2, exit=False).result
    sys.exit(0 if rc.wasSuccessful() else 1)
