import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("verify_sp1_advisory_quarantine.py")
SPEC = importlib.util.spec_from_file_location("sp1_quarantine", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class Sp1AdvisoryQuarantineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        source = SCRIPT.parents[1]
        for relative in (*MODULE.EXPECTED_LOCKS, *MODULE.EXPECTED_MANIFESTS):
            destination = self.root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source / relative, destination)
        gates = self.root / "deployments/open-competition-v2-beta1-release-gates.json"
        gates.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(
            source / "deployments/open-competition-v2-beta1-release-gates.json", gates
        )

    def tearDown(self) -> None:
        self.temp.cleanup()

    def test_current_quarantine_is_exact(self) -> None:
        report = MODULE.verify(self.root)
        self.assertEqual(report["status"], "quarantined_unresolved_high")
        self.assertFalse(report["mainnet_creation_enabled"])

    def test_mainnet_or_resolved_high_fails_closed(self) -> None:
        path = self.root / "deployments/open-competition-v2-beta1-release-gates.json"
        gates = json.loads(path.read_text(encoding="utf-8"))
        gates["mainnet_creation_enabled"] = True
        path.write_text(json.dumps(gates), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "blocks V2 mainnet"):
            MODULE.verify(self.root)

    def test_version_checksum_or_extra_lock_fails_closed(self) -> None:
        lock = self.root / next(iter(MODULE.EXPECTED_LOCKS))
        lock.write_text(
            lock.read_text(encoding="utf-8").replace(
                MODULE.PACKAGE_CHECKSUM, "0" * 64, 1
            ),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(ValueError, "checksum"):
            MODULE.verify(self.root)


if __name__ == "__main__":
    unittest.main()
