#!/usr/bin/env python3
"""Deterministic tests for stalled-bounty diagnostics (#873)."""
from __future__ import annotations

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from scripts.stalled_bounty_diagnostics import (  # noqa: E402
    CLAIM_EXPIRING, HEALTHY_CLAIMED, SETTLED, SUBMITTED, TERMINAL,
    VERIFICATION_EXPIRING, VERIFIER_UNAVAILABLE, classify, diagnose,
    next_action_for, SCHEMA_VERSION,
)

NOW = 1_800_000_000
HOUR = 3600


class ClassifyTests(unittest.TestCase):
    def test_healthy_claimed(self):
        self.assertEqual(classify({"bounty_id": "B1", "status": "claimed",
                                   "claim_deadline": NOW + 48 * HOUR,
                                   "verification_ready": True, "expiring_within": 6 * HOUR}, NOW),
                         HEALTHY_CLAIMED)

    def test_claim_expiring(self):
        self.assertEqual(classify({"bounty_id": "B2", "status": "claimed",
                                   "claim_deadline": NOW + 2 * HOUR,
                                   "verification_ready": True, "expiring_within": 6 * HOUR}, NOW),
                         CLAIM_EXPIRING)

    def test_submitted(self):
        self.assertEqual(classify({"bounty_id": "B3", "status": "submitted",
                                   "verification_deadline": NOW + 72 * HOUR,
                                   "verification_ready": True, "expiring_within": 6 * HOUR}, NOW),
                         SUBMITTED)

    def test_verification_expiring(self):
        self.assertEqual(classify({"bounty_id": "B4", "status": "submitted",
                                   "verification_deadline": NOW + HOUR,
                                   "verification_ready": True, "expiring_within": 6 * HOUR}, NOW),
                         VERIFICATION_EXPIRING)

    def test_verifier_unavailable(self):
        self.assertEqual(classify({"bounty_id": "B5", "status": "submitted",
                                   "verification_ready": False}, NOW),
                         VERIFIER_UNAVAILABLE)

    def test_settled_is_terminal(self):
        self.assertEqual(classify({"bounty_id": "B6", "status": "settled"}, NOW), SETTLED)
        self.assertEqual(classify({"bounty_id": "B7", "status": "cancelled"}, NOW), TERMINAL)


class BoundaryTests(unittest.TestCase):
    def test_exact_deadline_boundary_is_expiring(self):
        row = {"bounty_id": "B8", "status": "claimed", "claim_deadline": NOW + 6 * HOUR,
               "verification_ready": True, "expiring_within": 6 * HOUR}
        self.assertEqual(classify(row, NOW), CLAIM_EXPIRING)

    def test_one_second_past_boundary_is_healthy(self):
        row = {"bounty_id": "B9", "status": "claimed", "claim_deadline": NOW + 6 * HOUR + 1,
               "verification_ready": True, "expiring_within": 6 * HOUR}
        self.assertEqual(classify(row, NOW), HEALTHY_CLAIMED)

    def test_expired_deadline_is_expiring(self):
        row = {"bounty_id": "B10", "status": "claimed", "claim_deadline": NOW - HOUR,
               "verification_ready": True, "expiring_within": 6 * HOUR}
        self.assertEqual(classify(row, NOW), CLAIM_EXPIRING)


class MissingTermsTests(unittest.TestCase):
    """Missing terms: a row without explicit deadline terms must not be
    silently assigned a guessed deadline."""

    def test_missing_claim_deadline_never_guesses(self):
        row = {"bounty_id": "B11", "status": "claimed", "verification_ready": True}
        self.assertIn(classify(row, NOW), (HEALTHY_CLAIMED, VERIFIER_UNAVAILABLE))
        item = diagnose([row], NOW)["items"][0]
        self.assertTrue(item["missing_terms"])

    def test_missing_deadline_surfaces_as_blocker(self):
        items = diagnose([{"bounty_id": "B12", "status": "claimed",
                           "verification_ready": True}], NOW)["items"]
        self.assertTrue(items[0]["missing_terms"])
        self.assertTrue(items[0]["next_action"])


class StaleIndexTests(unittest.TestCase):
    def test_stale_evidence_is_surfaced_not_silently_trusted(self):
        row = {"bounty_id": "B13", "status": "submitted", "verification_ready": True,
               "verification_deadline": NOW + 10 * HOUR, "evidence_stale": True}
        item = diagnose([row], NOW)["items"][0]
        self.assertTrue(item["evidence_stale"])

    def test_stale_index_data_does_not_fabricate_payment(self):
        row = {"bounty_id": "B14", "status": "submitted", "verification_ready": True,
               "evidence_stale": True}
        item = diagnose([row], NOW, canonical_events=[])["items"][0]
        self.assertFalse(item["settled"])


class VerifierOutageTests(unittest.TestCase):
    def test_outage_yields_escalation_action(self):
        row = {"bounty_id": "B15", "status": "submitted", "verification_ready": False}
        item = diagnose([row], NOW)["items"][0]
        self.assertEqual(item["stall_class"], VERIFIER_UNAVAILABLE)
        self.assertEqual(item["next_action"],
                         "escalate_verifier_unavailable_and_preserve_escrow")

    def test_outage_keeps_item_in_backlog(self):
        diag = diagnose([{"bounty_id": "B16", "status": "submitted",
                          "verification_ready": False}], NOW)
        self.assertEqual(len(diag["backlog"]), 1)


class PaymentInferenceTests(unittest.TestCase):
    def test_tx_hash_alone_is_never_payment(self):
        row = {"bounty_id": "B17", "status": "submitted", "verification_ready": True}
        diag = diagnose([row], NOW, canonical_events=[
            {"bounty_id": "B17", "event": "TransactionBroadcast", "tx_hash": "0xabc"}])
        self.assertFalse(diag["items"][0]["settled"])

    def test_canonical_bountysettled_is_payment(self):
        row = {"bounty_id": "B18", "status": "submitted", "verification_ready": True}
        diag = diagnose([row], NOW, canonical_events=[
            {"bounty_id": "B18", "event": "BountySettled", "tx_hash": "0xdef"}])
        self.assertTrue(diag["items"][0]["settled"])

    def test_already_settled_work_is_not_stalled(self):
        diag = diagnose([{"bounty_id": "B19", "status": "settled"}], NOW)
        self.assertEqual(diag["backlog"], [])


class DiagnosticShapeTests(unittest.TestCase):
    ROWS = [
        {"bounty_id": "C1", "status": "claimed", "claim_deadline": NOW + 48 * HOUR,
         "verification_ready": True, "expiring_within": 6 * HOUR},
        {"bounty_id": "C2", "status": "claimed", "claim_deadline": NOW + HOUR,
         "verification_ready": True, "expiring_within": 6 * HOUR},
        {"bounty_id": "C3", "status": "submitted", "verification_deadline": NOW + 72 * HOUR,
         "verification_ready": True, "expiring_within": 6 * HOUR},
        {"bounty_id": "C4", "status": "submitted", "verification_ready": False,
          "submitted_at": NOW - 5 * HOUR},
        {"bounty_id": "C5", "status": "settled"},
    ]

    def test_every_non_terminal_item_has_one_action_and_deadline(self):
        diag = diagnose(self.ROWS, NOW)
        for item in diag["backlog"]:
            self.assertTrue(item["next_action"])
            self.assertIsNotNone(item["deadline"])

    def test_versioned_and_deterministic(self):
        a = diagnose(self.ROWS, NOW)
        self.assertEqual(a["schema"], SCHEMA_VERSION)
        self.assertEqual(a, diagnose(self.ROWS, NOW))

    def test_next_action_covers_every_class(self):
        for cls in (HEALTHY_CLAIMED, CLAIM_EXPIRING, SUBMITTED,
                    VERIFICATION_EXPIRING, VERIFIER_UNAVAILABLE, SETTLED, TERMINAL):
            self.assertTrue(next_action_for(cls))


if __name__ == "__main__":
    unittest.main(verbosity=2)
