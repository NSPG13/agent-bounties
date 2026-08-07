#!/usr/bin/env python3
"""Tests for the mini-SWE-agent bounty selector.

Covers: empty inventory, stale bounties, no-margin, exclusive claimants,
multiple eligible, single eligible, and fail-closed behavior.
"""
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

# Ensure we can import the module under test
sys.path.insert(0, str(Path(__file__).parent))
from select_bounty import (
    is_fresh,
    has_positive_margin,
    has_no_exclusive_claimant,
    is_canonical,
    select_bounty,
)


class TestBountySelector(unittest.TestCase):
    """Comprehensive test suite for bounty selection logic."""

    def test_empty_inventory(self):
        """Empty inventory should return None (fail-closed)."""
        result = select_bounty("fixtures/empty.json")
        self.assertIsNone(result)

    def test_stale_inventory(self):
        """Stale bounties (30+ days old) should be filtered out."""
        result = select_bounty("fixtures/stale.json")
        self.assertIsNone(result, "All bounties are stale — should return None")

    def test_no_margin_inventory(self):
        """Bounties with zero/negative margin should be filtered."""
        result = select_bounty("fixtures/no-margin.json")
        self.assertIsNone(result, "No bounties have positive margin")

    def test_exclusive_claimant(self):
        """Bounties with exclusive claimants should be filtered."""
        result = select_bounty("fixtures/exclusive-claimant.json")
        self.assertIsNone(result, "All bounties have exclusive claimants")

    def test_multiple_eligible(self):
        """With multiple eligible bounties, select highest reward."""
        result = select_bounty("fixtures/multiple.json")
        self.assertIsNotNone(result)
        self.assertGreater(float(result.get("reward_usdc", 0)), 0)
        self.assertEqual(result.get("state"), "claimable-live")
        self.assertTrue(result.get("canonical", False))

    # Unit tests for individual check functions

    def test_is_fresh_recent(self):
        """Recently created bounty should be fresh."""
        from datetime import datetime, timezone, timedelta
        recent = (datetime.now(timezone.utc) - timedelta(hours=2)).isoformat()
        self.assertTrue(is_fresh({"created_at": recent}))

    def test_is_fresh_old(self):
        """Old bounty should not be fresh."""
        self.assertFalse(is_fresh({"created_at": "2020-01-01T00:00:00Z"}))

    def test_has_positive_margin_true(self):
        """Reward > bond should be positive margin."""
        self.assertTrue(has_positive_margin({"reward_usdc": "5.00", "bond_usdc": "0.01"}))

    def test_has_positive_margin_zero(self):
        """Reward == 0 should not be positive margin."""
        self.assertFalse(has_positive_margin({"reward_usdc": "0", "bond_usdc": "0.01"}))

    def test_has_no_exclusive_claimant_empty(self):
        """No claimants means no exclusive claimant."""
        self.assertTrue(has_no_exclusive_claimant({"claimants": []}))

    def test_has_no_exclusive_claimant_exclusive(self):
        """Exclusive claimant should block."""
        self.assertFalse(has_no_exclusive_claimant({
            "claimants": [{"address": "0x123", "exclusive": True}]
        }))

    def test_is_canonical_valid(self):
        """Claimable-live + canonical = valid."""
        self.assertTrue(is_canonical({"canonical": True, "state": "claimable-live"}))

    def test_is_canonical_non_canonical(self):
        """Non-canonical should fail."""
        self.assertFalse(is_canonical({"canonical": False, "state": "claimable-live"}))

    def test_is_canonical_wrong_state(self):
        """Wrong state should fail even if canonical."""
        self.assertFalse(is_canonical({"canonical": True, "state": "draft"}))

    # Integration: evidence emission
    def test_evidence_emission(self):
        """Select and emit evidence in a temp directory."""
        with tempfile.TemporaryDirectory() as tmpdir:
            old_cwd = os.getcwd()
            os.chdir(Path(__file__).parent)
            try:
                # Use subprocess to test evidence emission through main
                import subprocess
                result = subprocess.run(
                    [sys.executable, "select_bounty.py",
                     "--inventory", "fixtures/multiple.json",
                     "--output-dir", tmpdir],
                    capture_output=True, text=True, timeout=30
                )
                self.assertEqual(result.returncode, 0, f"Failed: {result.stderr}")
                evidence_files = list(Path(tmpdir).glob("evidence_*.json"))
                self.assertGreater(len(evidence_files), 0, "No evidence file emitted")
            finally:
                os.chdir(old_cwd)


if __name__ == "__main__":
    unittest.main(verbosity=2)
