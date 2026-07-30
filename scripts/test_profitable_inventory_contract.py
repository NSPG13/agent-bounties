#!/usr/bin/env python3

from __future__ import annotations

import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "fixtures" / "profitable-canonical-opportunity.json"


class ProfitableInventoryContractTests(unittest.TestCase):
    def test_one_fixture_matches_every_public_inventory_surface(self) -> None:
        item = json.loads(FIXTURE.read_text(encoding="utf-8"))
        self.assertEqual(item["reward"]["amount"], "1990000")
        self.assertEqual(item["bond"]["amount"], "10000")
        self.assertEqual(item["funded_amount"], item["funding_target"])
        self.assertEqual(item["source_status"], "claimable")
        self.assertEqual(item["work_state"], "claimable")
        self.assertTrue(item["terms_hash"])
        self.assertTrue(item["verification_ready"])
        self.assertGreater(
            int(item["cash_economics"]["gross_cash_margin"]["amount"]), 0
        )

        api = (ROOT / "crates" / "api" / "src" / "opportunities.rs").read_text(
            encoding="utf-8"
        )
        mcp = (ROOT / "crates" / "mcp-server" / "src" / "main.rs").read_text(
            encoding="utf-8"
        )
        home = (ROOT / "site" / "home.js").read_text(encoding="utf-8")
        board = (ROOT / "site" / "bounty-board.js").read_text(encoding="utf-8")

        for field in (
            "cash_economics",
            "funded_amount",
            "funding_target",
            "terms_hash",
            "verification_ready",
        ):
            self.assertIn(field, api)
        self.assertIn("/v1/opportunities", mcp)
        self.assertIn("list_autonomous_bounties", mcp)
        for public_surface in (home, board):
            self.assertIn("cash_economics", public_surface)
            self.assertIn("gross_cash_margin", public_surface)
            self.assertIn("not net profit", public_surface)

    def test_claimed_fixture_leaves_claimable_only_without_corruption_claim(self) -> None:
        item = json.loads(FIXTURE.read_text(encoding="utf-8"))
        item["source_status"] = "claimed"
        item["work_state"] = "in_progress"
        claimable_only = [
            candidate
            for candidate in [item]
            if candidate["work_state"] == "claimable"
            and candidate["verification_ready"]
        ]
        self.assertEqual(claimable_only, [])
        self.assertEqual(item["payment_state"], "escrowed")
        self.assertTrue(item["terms_hash"])


if __name__ == "__main__":
    unittest.main()
