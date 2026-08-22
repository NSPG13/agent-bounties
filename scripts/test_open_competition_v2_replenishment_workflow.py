#!/usr/bin/env python3
"""Static safety contract for the private V2 replenishment workflow."""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "open-competition-v2-replenishment.yml"


class ReplenishmentWorkflowContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text = WORKFLOW.read_text(encoding="utf-8")

    def test_runs_every_five_minutes_and_serializes_workers(self) -> None:
        self.assertIn('cron: "*/5 * * * *"', self.text)
        self.assertIn("cancel-in-progress: false", self.text)
        self.assertIn(
            "vars.V2_REPLENISHMENT_ENABLED == 'true'",
            self.text,
        )

    def test_uses_remote_signer_without_repository_private_key(self) -> None:
        self.assertNotRegex(self.text, re.compile(r"PRIVATE[_-]?KEY", re.IGNORECASE))
        self.assertIn("V2_REPLENISHMENT_SIGNER_URL", self.text)
        self.assertIn("V2_REPLENISHMENT_SIGNER_TOKEN", self.text)
        self.assertIn("https://*", self.text)
        self.assertIn("V2_REPLENISHMENT_EXECUTE == 'true'", self.text)

    def test_private_artifacts_are_never_uploaded_or_printed(self) -> None:
        self.assertNotIn("upload-artifact", self.text)
        for sensitive in (
            "private-inventory.json",
            "private-ranking.json",
            "execution-ledger.json",
            "replenishment-plan.json",
            "replenishment-request.json",
        ):
            self.assertNotIn(f"cat {sensitive}", self.text)
            self.assertNotIn(f"tee {sensitive}", self.text)

    def test_exact_private_floor_target_and_policy_are_exercised(self) -> None:
        self.assertIn('PRIVATE_V2_INVENTORY_FLOOR: "5"', self.text)
        self.assertIn('PRIVATE_V2_INVENTORY_TARGET: "10"', self.text)
        self.assertIn("plan_open_competition_v2_replenishment.py", self.text)
        self.assertIn("materialize_open_competition_v2_replenishment.py", self.text)
        self.assertIn("test_build_open_competition_v2_gmv_snapshots", self.text)
        self.assertIn("isolated signer", self.text)
        self.assertIn("canonical activation remains required", self.text)

    def test_snapshot_hashing_dependency_is_pinned(self) -> None:
        self.assertIn(
            "python -m pip install -r scripts/requirements-attest.txt",
            self.text,
        )


if __name__ == "__main__":
    unittest.main()
