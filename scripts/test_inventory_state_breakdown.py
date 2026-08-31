import importlib.util
from datetime import datetime, timezone
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "inventory_breakdown", ROOT / "scripts" / "check-inventory-state-breakdown.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class InventoryBreakdownTests(unittest.TestCase):
    def load(self, name):
        path = ROOT / "scripts" / "fixtures" / "inventory-state-breakdown" / f"{name}.json"
        return MODULE._load(path)

    def test_mixed_snapshot_counts_each_state_once(self):
        result = MODULE.project(self.load("mixed"), now=datetime(2026, 8, 27, 12, tzinfo=timezone.utc))
        self.assertEqual(result["counts"], {
            "ready_to_earn": 1, "in_progress": 1, "submitted": 1,
            "paid": 1, "verification_unavailable": 0,
        })

    def test_degraded_snapshot_does_not_fabricate_zero(self):
        result = MODULE.project(self.load("degraded"), now=datetime(2026, 8, 27, 12, tzinfo=timezone.utc))
        self.assertTrue(result["degraded"])
        self.assertEqual(result["counts"]["verification_unavailable"], 2)

    def test_stale_snapshot_is_unavailable(self):
        result = MODULE.project(self.load("stale"), now=datetime(2026, 8, 27, 12, tzinfo=timezone.utc))
        self.assertTrue(result["stale"])
        self.assertEqual(result["counts"]["verification_unavailable"], 1)


if __name__ == "__main__":
    unittest.main()
