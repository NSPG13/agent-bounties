#!/usr/bin/env python3
"""Focused tests for prepare_low_bond_standing_meta_migration.py."""

from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

import prepare_low_bond_standing_meta_migration as migration


class LowBondStandingMetaMigrationTests(unittest.TestCase):
    def test_prepares_four_exact_low_bond_replacements_without_mutating_sources(self) -> None:
        repo_root = Path(__file__).resolve().parents[1]
        original_bytes = {
            issue: (repo_root / "bounties" / "autonomous-v1" / f"{issue}.json").read_bytes()
            for issue in migration.REPLACEMENTS
        }

        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            manifest = migration.prepare(repo_root, output)

            self.assertEqual(len(manifest["replacements"]), 4)
            self.assertEqual(
                manifest["reserved_contracts_csv"],
                ",".join(migration.REPLACEMENTS.values()),
            )
            self.assertEqual(
                manifest["required_confirmation_events"],
                ["CanonicalBountyCreated", "FundingAdded", "BountyBecameClaimable"],
            )

            hashes: set[str] = set()
            nonces: set[str] = set()
            for entry in manifest["replacements"]:
                issue = entry["source_issue_number"]
                document = json.loads((output / f"{issue}.json").read_text(encoding="utf-8"))
                terms = document["contract_terms"]
                self.assertEqual(terms["solver_reward"]["amount"], 990_000)
                self.assertEqual(terms["verifier_reward"]["amount"], 10_000)
                self.assertEqual(terms["claim_bond"]["amount"], 10_000)
                self.assertEqual(terms["initial_funding"]["amount"], 1_000_000)
                self.assertEqual(
                    terms["verifier_reward"]["amount"],
                    terms["claim_bond"]["amount"],
                )
                self.assertEqual(
                    terms["solver_reward"]["amount"] + terms["verifier_reward"]["amount"],
                    terms["initial_funding"]["amount"],
                )
                self.assertEqual(entry["replacement_terms_hash"], migration.terms_hash(document))
                hashes.add(entry["replacement_terms_hash"])
                nonces.add(entry["creation_nonce"])

            self.assertEqual(len(hashes), 4)
            self.assertEqual(len(nonces), 4)
            self.assertTrue((output / "manifest.json").is_file())

        for issue, before in original_bytes.items():
            path = repo_root / "bounties" / "autonomous-v1" / f"{issue}.json"
            self.assertEqual(path.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
