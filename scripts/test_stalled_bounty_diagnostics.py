#!/usr/bin/env python3
"""Comprehensive test suite for stalled bounty diagnostics and recovery actions."""

from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

import stalled_bounty_diagnostics as diag


class StalledBountyDiagnosticsTests(unittest.TestCase):
    """Test suite verifying canonical lifecycle state diagnostics with zero mock assertions."""

    def setUp(self) -> None:
        self.fixtures_dir = SCRIPTS / "fixtures" / "stalled_bounties"

    def load_fixture(self, name: str) -> dict | list:
        fixture_path = self.fixtures_dir / f"{name}.json"
        return json.loads(fixture_path.read_text(encoding="utf-8"))

    def test_healthy_claimed_classification_and_next_action(self) -> None:
        """Verify healthy claimed bounty produces submit_work next_action and valid deadline."""
        item = self.load_fixture("healthy_claimed")
        observed_time = 1775000000  # Before claim_expires_at (1780000000)
        res = diag.diagnose_bounty(item, current_timestamp=observed_time)

        self.assertEqual(res.classification, diag.HEALTHY_CLAIMED)
        self.assertEqual(res.status, "claimed")
        self.assertEqual(res.next_action, diag.ACTION_SUBMIT_WORK)
        self.assertFalse(res.is_stalled)
        self.assertIsNotNone(res.deadline)
        self.assertEqual(res.deadline_unix, 1780000000)
        self.assertEqual(res.deadline, "2026-05-28T20:26:40+00:00")

    def test_claim_expiring_classification_and_next_action(self) -> None:
        """Verify claim expiring / expired bounty produces expire_claim next_action and deadline."""
        item = self.load_fixture("claim_expiring")
        observed_time = 1775000000  # After claim_expires_at (1770000000)
        res = diag.diagnose_bounty(item, current_timestamp=observed_time)

        self.assertEqual(res.classification, diag.CLAIM_EXPIRING)
        self.assertEqual(res.status, "claimed")
        self.assertEqual(res.next_action, diag.ACTION_EXPIRE_CLAIM)
        self.assertTrue(res.is_stalled)
        self.assertEqual(res.deadline_unix, 1770000000)
        self.assertEqual(res.deadline, "2026-02-02T02:40:00+00:00")

    def test_submitted_classification_and_next_action(self) -> None:
        """Verify active submitted bounty produces verify_submission next_action and deadline."""
        item = self.load_fixture("submitted")
        observed_time = 1780003600  # Between claim (1780000000) and verification expiry (1780007200)
        res = diag.diagnose_bounty(item, current_timestamp=observed_time)

        self.assertEqual(res.classification, diag.SUBMITTED)
        self.assertEqual(res.status, "submitted")
        self.assertEqual(res.next_action, diag.ACTION_VERIFY_SUBMISSION)
        self.assertFalse(res.is_stalled)
        self.assertEqual(res.deadline_unix, 1780007200)
        self.assertEqual(res.deadline, "2026-05-28T22:26:40+00:00")

    def test_verification_expiring_classification_and_next_action(self) -> None:
        """Verify verification expiring / expired bounty produces expire_submission next_action."""
        item = self.load_fixture("verification_expiring")
        observed_time = 1775000000  # After verification_expires_at (1770007200)
        res = diag.diagnose_bounty(item, current_timestamp=observed_time)

        self.assertEqual(res.classification, diag.VERIFICATION_EXPIRING)
        self.assertEqual(res.status, "submitted")
        self.assertEqual(res.next_action, diag.ACTION_EXPIRE_SUBMISSION)
        self.assertTrue(res.is_stalled)
        self.assertEqual(res.deadline_unix, 1770007200)
        self.assertEqual(res.deadline, "2026-02-02T04:40:00+00:00")

    def test_verifier_unavailable_outage_classification(self) -> None:
        """Verify verifier outage triggers verifier_unavailable classification and restore action."""
        item = self.load_fixture("verifier_outage")
        observed_time = 1780003600  # Within verification window, but verifier fleet is down
        res = diag.diagnose_bounty(item, current_timestamp=observed_time)

        self.assertEqual(res.classification, diag.VERIFIER_UNAVAILABLE)
        self.assertEqual(res.status, "submitted")
        self.assertEqual(res.next_action, diag.ACTION_RESTORE_VERIFIERS)
        self.assertTrue(res.is_stalled)
        self.assertEqual(res.deadline_unix, 1780007200)
        self.assertIsNotNone(res.verifier_status)

    def test_settled_with_canonical_bountysettled_event(self) -> None:
        """Verify settled bounty with confirmed BountySettled canonical event requires no action."""
        item = self.load_fixture("settled_work")
        res = diag.diagnose_bounty(item, current_timestamp=1780000000)

        self.assertEqual(res.classification, diag.SETTLED)
        self.assertEqual(res.status, "settled")
        self.assertIsNone(res.next_action)
        self.assertIsNone(res.deadline)
        self.assertFalse(res.is_stalled)
        self.assertIsNotNone(res.settlement_evidence)
        self.assertEqual(res.settlement_evidence["event_name"], "BountySettled")
        self.assertTrue(res.settlement_evidence["confirmed_canonical"])

    def test_settled_without_canonical_bountysettled_fails_closed(self) -> None:
        """Verify status=settled without canonical BountySettled fails closed to stale_indexer."""
        item = {
            "bounty_id": "0x6666666666666666666666666666666666666666666666666666666666666666",
            "bounty_contract": "0x6666666666666666666666666666666666666666",
            "status": "settled",
            "events": [],
            # No settlement_evidence and no confirmed BountySettled event!
        }
        res = diag.diagnose_bounty(item, current_timestamp=1780000000)
        self.assertEqual(res.classification, diag.STALE_INDEXER)
        self.assertEqual(res.next_action, diag.ACTION_SYNC_INDEXER)
        self.assertTrue(res.is_stalled)

    def test_boundary_timestamps_at_claim_expiration_threshold(self) -> None:
        """Verify boundary timestamp behavior at exactly claim_expires_at and claim_expires_at + 1."""
        fixtures = self.load_fixture("boundary_timestamps")
        claim_item = fixtures["exact_boundary_claim"]
        exp = claim_item["claim_expires_at"]  # 1780000000

        # At exactly exp (now == claim_expires_at): still healthy/within window (Solidity require(block.timestamp <= exp) allows submit)
        res_boundary_equal = diag.diagnose_bounty(claim_item, current_timestamp=exp)
        self.assertEqual(res_boundary_equal.classification, diag.HEALTHY_CLAIMED)
        self.assertEqual(res_boundary_equal.next_action, diag.ACTION_SUBMIT_WORK)
        self.assertFalse(res_boundary_equal.is_stalled)

        # At exp - 1 (now < claim_expires_at): healthy claimed
        res_before = diag.diagnose_bounty(claim_item, current_timestamp=exp - 1)
        self.assertEqual(res_before.classification, diag.HEALTHY_CLAIMED)
        self.assertEqual(res_before.next_action, diag.ACTION_SUBMIT_WORK)

        # At exp + 1 (now > claim_expires_at): expired boundary transition
        res_after = diag.diagnose_bounty(claim_item, current_timestamp=exp + 1)
        self.assertEqual(res_after.classification, diag.CLAIM_EXPIRING)
        self.assertEqual(res_after.next_action, diag.ACTION_EXPIRE_CLAIM)
        self.assertTrue(res_after.is_stalled)

    def test_boundary_timestamps_at_verification_expiration_threshold(self) -> None:
        """Verify boundary timestamp behavior at exactly verification_expires_at and + 1."""
        fixtures = self.load_fixture("boundary_timestamps")
        verify_item = fixtures["exact_boundary_verification"]
        exp = verify_item["verification_expires_at"]  # 1780007200

        # At exactly exp (now == verification_expires_at): still within submitted verification window
        res_boundary_equal = diag.diagnose_bounty(verify_item, current_timestamp=exp)
        self.assertEqual(res_boundary_equal.classification, diag.SUBMITTED)
        self.assertEqual(res_boundary_equal.next_action, diag.ACTION_VERIFY_SUBMISSION)
        self.assertFalse(res_boundary_equal.is_stalled)

        # At exp - 1: within submitted window
        res_before = diag.diagnose_bounty(verify_item, current_timestamp=exp - 1)
        self.assertEqual(res_before.classification, diag.SUBMITTED)
        self.assertEqual(res_before.next_action, diag.ACTION_VERIFY_SUBMISSION)

        # At exp + 1: expired verification boundary transition
        res_after = diag.diagnose_bounty(verify_item, current_timestamp=exp + 1)
        self.assertEqual(res_after.classification, diag.VERIFICATION_EXPIRING)
        self.assertEqual(res_after.next_action, diag.ACTION_EXPIRE_SUBMISSION)
        self.assertTrue(res_after.is_stalled)

    def test_missing_terms_document_fails_closed(self) -> None:
        """Verify missing terms or invalid terms document triggers missing_terms classification."""
        item = self.load_fixture("missing_terms")
        res = diag.diagnose_bounty(item, current_timestamp=1775000000)

        self.assertEqual(res.classification, diag.MISSING_TERMS)
        self.assertEqual(res.next_action, diag.ACTION_RECONCILE_TERMS)
        self.assertTrue(res.is_stalled)
        self.assertIsNone(res.deadline)

    def test_stale_indexer_lag_and_heartbeat_outage(self) -> None:
        """Verify stale index data triggers stale_indexer classification and sync_indexer next_action."""
        item = self.load_fixture("stale_index_data")
        res = diag.diagnose_bounty(item, current_timestamp=1775000000)

        self.assertEqual(res.classification, diag.STALE_INDEXER)
        self.assertEqual(res.next_action, diag.ACTION_SYNC_INDEXER)
        self.assertTrue(res.is_stalled)

    def test_mixed_backlog_report_aggregation_and_counts(self) -> None:
        """Verify full backlog diagnosis properly aggregates counts and separates stalled items."""
        items = self.load_fixture("mixed_backlog")
        observed_time = 1775000000

        report = diag.diagnose_backlog(items, current_timestamp=observed_time)

        self.assertEqual(report.schema, diag.DIAGNOSTIC_SCHEMA)
        self.assertEqual(report.version, diag.DIAGNOSTIC_VERSION)
        self.assertEqual(report.total_diagnosed, 6)
        self.assertEqual(report.counts[diag.HEALTHY_CLAIMED], 1)
        self.assertEqual(report.counts[diag.CLAIM_EXPIRING], 1)
        self.assertEqual(report.counts[diag.SUBMITTED], 1)
        self.assertEqual(report.counts[diag.VERIFICATION_EXPIRING], 1)
        self.assertEqual(report.counts[diag.VERIFIER_UNAVAILABLE], 1)
        self.assertEqual(report.counts[diag.SETTLED], 1)
        self.assertEqual(report.counts["stalled_total"], 3)
        self.assertEqual(len(report.stalled_backlog), 3)

        stalled_classes = {item["classification"] for item in report.stalled_backlog}
        self.assertEqual(
            stalled_classes,
            {diag.CLAIM_EXPIRING, diag.VERIFICATION_EXPIRING, diag.VERIFIER_UNAVAILABLE},
        )

    def test_markdown_report_formatting(self) -> None:
        """Verify markdown report rendering includes summary table, stalled backlog, and evidence boundary."""
        items = self.load_fixture("mixed_backlog")
        report = diag.diagnose_backlog(items, current_timestamp=1775000000)
        markdown = diag.render_markdown_report(report)

        self.assertIn("# Stalled Bounty Diagnostics Report", markdown)
        self.assertIn("healthy_claimed", markdown)
        self.assertIn("claim_expiring", markdown)
        self.assertIn("submitted", markdown)
        self.assertIn("verification_expiring", markdown)
        self.assertIn("verifier_unavailable", markdown)
        self.assertIn("settled", markdown)
        self.assertIn("## Stalled Action Backlog", markdown)
        self.assertIn("## Evidence Boundary", markdown)
        self.assertIn("BountySettled", markdown)

    def test_file_loading_and_roundtrip(self) -> None:
        """Verify load_bounty_items handles raw paths, dictionary wrappers, and list inputs."""
        items = self.load_fixture("mixed_backlog")
        with tempfile.TemporaryDirectory() as temp_dir:
            file_path = Path(temp_dir) / "test_input.json"
            file_path.write_text(json.dumps(items), encoding="utf-8")

            loaded = diag.load_bounty_items(file_path)
            self.assertEqual(len(loaded), 6)

            wrapped = {"items": items}
            loaded_wrapped = diag.load_bounty_items(wrapped)
            self.assertEqual(len(loaded_wrapped), 6)


if __name__ == "__main__":
    unittest.main()
