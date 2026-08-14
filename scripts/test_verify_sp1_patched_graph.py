import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("verify_sp1_patched_graph.py")
SPEC = importlib.util.spec_from_file_location("sp1_patched_graph", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class Sp1PatchedGraphTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        source = SCRIPT.parents[1]
        for relative in (
            *MODULE.EXPECTED_LOCKS,
            *MODULE.EXPECTED_MANIFESTS,
            MODULE.IDENTITY_PATH,
        ):
            destination = self.root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source / relative, destination)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def test_current_graph_is_exact(self) -> None:
        report = MODULE.verify(self.root)
        self.assertEqual(report["status"], "patched_source_graph_pinned")
        self.assertEqual(report["sp1_commit"], MODULE.SP1_COMMIT)

    def test_registry_challenger_fails_closed(self) -> None:
        lock = self.root / MODULE.EXPECTED_LOCKS[0]
        value = lock.read_text(encoding="utf-8").replace(
            f'source = "git+{MODULE.SP1_REPOSITORY}?rev={MODULE.SP1_COMMIT}#{MODULE.SP1_COMMIT}"',
            'source = "registry+https://github.com/rust-lang/crates.io-index"',
            1,
        )
        lock.write_text(value, encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "patched SP1 commit"):
            MODULE.verify(self.root)

    def test_manifest_revision_drift_fails_closed(self) -> None:
        manifest = self.root / MODULE.EXPECTED_MANIFESTS[0]
        manifest.write_text(
            manifest.read_text(encoding="utf-8").replace(MODULE.SP1_COMMIT, "0" * 40, 1),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(ValueError, "must pin"):
            MODULE.verify(self.root)

    def test_release_identity_drift_fails_closed(self) -> None:
        identity_path = self.root / MODULE.IDENTITY_PATH
        identity = json.loads(identity_path.read_text(encoding="utf-8"))
        identity["sp1_commit"] = "0" * 40
        identity_path.write_text(json.dumps(identity), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "release identity"):
            MODULE.verify(self.root)


if __name__ == "__main__":
    unittest.main()
