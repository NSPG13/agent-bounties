#!/usr/bin/env python3
"""Deterministic tests for stalled-bounty diagnostics (#873).

Required fixture families: boundary timestamps, missing terms, stale index
data, verifier outage, and already settled work -- plus full coverage of
every classification (healthy_claimed, claim_expiring, submitted,
verification_expiring, verifier_unavailable, settled, terminal).
"""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

from stalled_bounty_diagnostics import diagnose

FIXTURES = SCRIPTS / "fixtures" / "stalled-bounty-diagnostics"

NOW = "2026-08-17T12:00:00+00:00"


def load_fixture(name: str) -> dict:
    return json.loads((FIXTURES / f"{name}.json").read_text(encoding="utf-8"))


class StalledWorkDiagnosticsTest(unittest.TestCase):
    def test_healthy_claimed_wait_for_submission(self) -> None:
        result = diagnose(load_fixture("healthy"), now=NOW)
        self.assertEqual(result["classification"], "healthy_claimed")
        self.assertEqual(result["next_action"], "wait_for_submission")
        self.assertIsNotNone(result["deadline"])

    def test_claim_expiring_escalate(self) -> None:
        result = diagnose(load_fixture("expiring"), now=NOW)
        self.assertEqual(result["classification"], "claim_expiring")
        self.assertEqual(result["next_action"], "escalate_claim")
        self.assertEqual(result["deadline"], "2026-08-17T12:30:00+00:00")

    def test_boundary_timestamp_is_expiring(self) -> None:
        # Remaining time equals the expiry grace exactly: boundary -> expiring.
        result = diagnose(load_fixture("boundary"), now=NOW)
        self.assertEqual(result["classification"], "claim_expiring")
        self.assertEqual(result["next_action"], "escalate_claim")

    def test_submitted_wait_for_verification(self) -> None:
        result = diagnose(load_fixture("submitted"), now=NOW)
        self.assertEqual(result["classification"], "submitted")
        self.assertEqual(result["next_action"], "wait_for_verification")
        self.assertEqual(result["deadline"], "2026-08-20T11:00:00+00:00")

    def test_verification_expiring_escalate(self) -> None:
        result = diagnose(load_fixture("verification-expiring"), now=NOW)
        self.assertEqual(result["classification"], "verification_expiring")
        self.assertEqual(result["next_action"], "escalate_verification")

    def test_verifier_unavailable_during_outage(self) -> None:
        result = diagnose(load_fixture("outage"), now=NOW)
        self.assertEqual(result["classification"], "verifier_unavailable")
        self.assertEqual(result["next_action"], "schedule_verifier")

    def test_bountysettled_event_drives_settled(self) -> None:
        # Canonical BountySettled event (lowercased: bountysettled).
        result = diagnose(load_fixture("settled"), now=NOW)
        self.assertEqual(result["classification"], "settled")
        self.assertEqual(result["next_action"], "none")
        self.assertIsNone(result["deadline"])

    def test_terminal_event_drives_terminal(self) -> None:
        result = diagnose(load_fixture("terminal"), now=NOW)
        self.assertEqual(result["classification"], "terminal")
        self.assertEqual(result["next_action"], "none")

    def test_missing_terms_blocker(self) -> None:
        result = diagnose(load_fixture("missing-terms"), now=NOW)
        self.assertEqual(result["next_action"], "restore_terms")
        self.assertTrue(any("claim window missing" in b for b in result["blockers"]))

    def test_stale_index_blocker(self) -> None:
        result = diagnose(load_fixture("stale"), now=NOW)
        self.assertEqual(result["classification"], "submitted")
        self.assertTrue(any("stale index data" in b for b in result["blockers"]))

    def test_never_infer_from_github_or_tx(self) -> None:
        # A transaction hash and GitHub issue state in the item are ignored:
        # only canonical lifecycle events drive the verdict.
        item = load_fixture("healthy")
        item["tx_hash"] = "0xdeadbeef"
        item["github_state"] = "closed"
        result = diagnose(item, now=NOW)
        self.assertEqual(result["classification"], "healthy_claimed")
        self.assertIn("canonical lifecycle events", result["disclaimer"])

    def test_deterministic_given_same_inputs(self) -> None:
        first = diagnose(load_fixture("submitted"), now=NOW)
        second = diagnose(load_fixture("submitted"), now=NOW)
        self.assertEqual(first, second)


if __name__ == "__main__":
    unittest.main()
