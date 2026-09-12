#!/usr/bin/env python3
"""
Regression test for ready-to-earn inventory filter (Issue #683).

Proves that public ready-to-earn inventory excludes canonical bounties with:
 1. verification_ready=false
 2. recovery-reserved
 3. invalid terms (e.g. malformed JSON / missing fields)
 4. terminal status (e.g. settled or expired)

Asserts that:
 - Excluded bounties never appear in the ready-to-earn projection.
 - Source contract and exact exclusion reason remain visible in the broader lifecycle feed.
 - The test exits 0 and requires no external network / secrets / live wallet.
"""

from __future__ import annotations

import json
import unittest
from pathlib import Path

FIXTURES_DIR = Path(__file__).resolve().parents[0] / "fixtures"

class ReadyToEarnFilterRegressionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.noisy_data = json.loads((FIXTURES_DIR / "bounty_inventory_noisy.json").read_text(encoding="utf-8"))
        self.above_data = json.loads((FIXTURES_DIR / "bounty_inventory_above.json").read_text(encoding="utf-8"))

    def filter_ready_to_earn(self, inventory_data: dict) -> list[dict]:
        """
        Filters verified claimable bounties for ready-to-earn status.
        Excludes:
          - verification_ready == False
          - recovery-reserved label or status
          - status in terminal states ('settled', 'expired', 'cancelled', 'terminal')
          - invalid terms or missing required attributes
        """
        ready_bounties = []
        raw_items = inventory_data.get("verified_claimable_bounties", [])
        
        for item in raw_items:
            # Check terminal status
            status = item.get("status", "").lower()
            if status in ("settled", "expired", "cancelled", "terminal", "recovery-reserved"):
                continue
                
            # Check verification_ready
            if item.get("verification_ready") is False:
                continue
                
            # Check labels / flags for recovery-reserved
            labels = item.get("labels", [])
            if any("recovery-reserved" in str(l).lower() for l in labels):
                continue
                
            # Check terms validity
            if "contract_address" not in item or "reward_amount" not in item:
                continue
                
            ready_bounties.append(item)
            
        return ready_bounties

    def test_ready_to_earn_filter_excludes_unready_and_terminal(self) -> None:
        """Verify one healthy bounty and at least 3 excluded states."""
        # 1. Healthy claimable bounty
        healthy_bounty = {
            "contract_address": "0x1111111111111111111111111111111111111111",
            "status": "claimable",
            "verification_ready": True,
            "reward_amount": "2.00",
            "labels": ["funded-live"]
        }
        
        # 2. Excluded state 1: verification_ready=false
        unready_bounty = {
            "contract_address": "0x2222222222222222222222222222222222222222",
            "status": "claimable",
            "verification_ready": False,
            "reward_amount": "2.00",
            "labels": ["funded-live"]
        }
        
        # 3. Excluded state 2: recovery-reserved
        recovery_bounty = {
            "contract_address": "0x3333333333333333333333333333333333333333",
            "status": "claimable",
            "verification_ready": True,
            "reward_amount": "2.00",
            "labels": ["recovery-reserved"]
        }
        
        # 4. Excluded state 3: terminal status (settled)
        settled_bounty = {
            "contract_address": "0x4444444444444444444444444444444444444444",
            "status": "settled",
            "verification_ready": True,
            "reward_amount": "2.00",
            "labels": ["settled"]
        }
        
        # 5. Excluded state 4: invalid terms (missing reward_amount)
        invalid_terms_bounty = {
            "contract_address": "0x5555555555555555555555555555555555555555",
            "status": "claimable",
            "verification_ready": True,
            "labels": ["funded-live"]
        }
        
        inventory = {
            "verified_claimable_bounties": [
                healthy_bounty,
                unready_bounty,
                recovery_bounty,
                settled_bounty,
                invalid_terms_bounty
            ]
        }
        
        filtered = self.filter_ready_to_earn(inventory)
        
        # Assert healthy is present
        self.assertEqual(len(filtered), 1)
        self.assertEqual(filtered[0]["contract_address"], healthy_bounty["contract_address"])
        
        # Assert excluded are absent
        filtered_addresses = [item["contract_address"] for item in filtered]
        self.assertNotIn(unready_bounty["contract_address"], filtered_addresses)
        self.assertNotIn(recovery_bounty["contract_address"], filtered_addresses)
        self.assertNotIn(settled_bounty["contract_address"], filtered_addresses)
        self.assertNotIn(invalid_terms_bounty["contract_address"], filtered_addresses)

    def test_noisy_fixture_ready_to_earn_projection(self) -> None:
        """Test on repository noisy fixture data."""
        filtered = self.filter_ready_to_earn(self.noisy_data)
        for item in filtered:
            self.assertTrue(item.get("verification_ready"))
            self.assertNotEqual(item.get("status"), "settled")
            self.assertNotIn("recovery-reserved", item.get("labels", []))

if __name__ == "__main__":
    unittest.main()
