#!/usr/bin/env python3

from __future__ import annotations

import json
from pathlib import Path
import subprocess
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
        # Exercise the shared readiness contract instead of requiring a second
        # copy of its implementation inside the homepage source.
        result = subprocess.run(
            ["node", "-e", """
const fs = require('node:fs');
// The economics fixture is mechanism-only; supply a synthetic projection identity.
const item = {network:'base-mainnet', source_id:'0x' + '1'.repeat(40),
  ...JSON.parse(fs.readFileSync(0, 'utf8'))};
const predicates = [require('./site/marketplace-workflow.js').ready,
  require('./site/marketplace.js').isReadyToEarn,
  require('./site/solarpunk-home.js').isReadyToEarn];
const changes = [{}, {work_state:'in_progress'}, {payment_state:'unfunded'},
  {payment_committed:false}, {verification_ready:false}, {terms_hash:null},
  {funded_amount:{...item.funded_amount, amount:'0'}},
  {network:null}, {source_id:null}];
console.log(JSON.stringify(changes.map(change =>
  predicates.map(ready => ready({...item, ...change})))));
"""],
            input=json.dumps(item), text=True, capture_output=True, cwd=ROOT,
            check=True,
        )
        self.assertEqual(json.loads(result.stdout), [[True] * 3] + [[False] * 3] * 8)

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
