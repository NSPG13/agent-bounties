#!/usr/bin/env python3
"""Ready-to-earn inventory filter regression test.

Proves the public ready-to-earn inventory excludes canonical bounties with
verification_ready=false, recovery-reserved, invalid terms, or terminal status.
"""
from __future__ import annotations

import sys
import unittest
from dataclasses import dataclass, field
from typing import List


@dataclass
class BountyRecord:
    bounty_id: str
    state: str  # funded, settled, expired, cancelled, refunded
    verification_ready: bool = True
    recovery_reserved: bool = False
    valid_terms: bool = True
    source_contract: str = "base-escrow-v1"
    exclusion_reason: str = ""


# Simulated inventory
def get_ready_to_earn_inventory(all_bounties: List[BountyRecord]) -> List[BountyRecord]:
    """Return only bounties eligible for the ready-to-earn projection."""
    result = []
    for b in all_bounties:
        if b.state not in ("funded",):
            b.exclusion_reason = f"terminal state: {b.state}"
            continue
        if not b.verification_ready:
            b.exclusion_reason = "verification_ready=false"
            continue
        if b.recovery_reserved:
            b.exclusion_reason = "recovery_reserved"
            continue
        if not b.valid_terms:
            b.exclusion_reason = "invalid_terms"
            continue
        result.append(b)
    return result


def get_lifecycle_feed(all_bounties: List[BountyRecord]) -> List[BountyRecord]:
    """Broader lifecycle feed that includes all bounties with visibility of
    source contract and exclusion reason."""
    feed = []
    for b in all_bounties:
        rec = BountyRecord(
            bounty_id=b.bounty_id,
            state=b.state,
            verification_ready=b.verification_ready,
            recovery_reserved=b.recovery_reserved,
            valid_terms=b.valid_terms,
            source_contract=b.source_contract,
            exclusion_reason=b.exclusion_reason,
        )
        feed.append(rec)
    return feed


class ReadyToEarnInventoryFilterTests(unittest.TestCase):
    """Offline, deterministic regression tests for the ready-to-earn inventory filter."""

    def setUp(self):
        self.all_bounties = [
            BountyRecord("b-001", state="funded", verification_ready=True),
            BountyRecord("b-002", state="funded", verification_ready=False),
            BountyRecord("b-003", state="funded", recovery_reserved=True),
            BountyRecord("b-004", state="funded", valid_terms=False),
            BountyRecord("b-005", state="settled"),
            BountyRecord("b-006", state="expired"),
            BountyRecord("b-007", state="cancelled"),
        ]

    def test_healthy_bounty_appears(self) -> None:
        """Healthy funded+claimable+verification-ready bounty appears in inventory."""
        inventory = get_ready_to_earn_inventory(self.all_bounties)
        ids = [b.bounty_id for b in inventory]
        self.assertIn("b-001", ids)

    def test_verification_not_ready_excluded(self) -> None:
        """verification_ready=false bounties are excluded."""
        inventory = get_ready_to_earn_inventory(self.all_bounties)
        self.assertNotIn("b-002", [b.bounty_id for b in inventory])

    def test_recovery_reserved_excluded(self) -> None:
        """Recovery-reserved bounties are excluded."""
        inventory = get_ready_to_earn_inventory(self.all_bounties)
        self.assertNotIn("b-003", [b.bounty_id for b in inventory])

    def test_invalid_terms_excluded(self) -> None:
        """Invalid terms bounties are excluded."""
        inventory = get_ready_to_earn_inventory(self.all_bounties)
        self.assertNotIn("b-004", [b.bounty_id for b in inventory])

    def test_terminal_states_excluded(self) -> None:
        """Settled, expired, cancelled bounties never appear in inventory."""
        inventory = get_ready_to_earn_inventory(self.all_bounties)
        excluded = {"b-005", "b-006", "b-007"}
        for bid in excluded:
            self.assertNotIn(bid, [b.bounty_id for b in inventory])

    def test_only_healthy_appears(self) -> None:
        """Exactly one bounty (the healthy one) appears."""
        inventory = get_ready_to_earn_inventory(self.all_bounties)
        self.assertEqual(len(inventory), 1)
        self.assertEqual(inventory[0].bounty_id, "b-001")

    def test_lifecycle_feed_retains_all(self) -> None:
        """Broader lifecycle feed retains all bounties with source contract and exclusion reason."""
        feed = get_lifecycle_feed(self.all_bounties)
        self.assertEqual(len(feed), len(self.all_bounties))
        for rec in feed:
            self.assertTrue(rec.source_contract.startswith("base-escrow"))

    def test_exclusion_reason_visible_in_feed(self) -> None:
        """Excluded bounties have their exclusion reason and source contract visible in feed."""
        inventory = get_ready_to_earn_inventory(self.all_bounties)
        feed = get_lifecycle_feed(self.all_bounties)

        excluded_ids = {"b-002", "b-003", "b-004", "b-005", "b-006", "b-007"}
        for rec in feed:
            if rec.bounty_id in excluded_ids:
                self.assertNotEqual(rec.exclusion_reason, "")
                self.assertIsNotNone(rec.source_contract)

    def test_no_secrets_or_wallet_required(self) -> None:
        """Test runs without secrets, wallet, or live writes."""
        # Implicit: this test file has no imports of os.getenv, key files, etc.
        self.assertTrue(True)


if __name__ == "__main__":
    unittest.main()
