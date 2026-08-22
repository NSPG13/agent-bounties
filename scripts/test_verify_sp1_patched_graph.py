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
            *MODULE.IDENTITY_PATHS,
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
        self.assertEqual(report["sp1_runtime_commit"], MODULE.SP1_RUNTIME_COMMIT)

    def test_registry_challenger_fails_closed(self) -> None:
        lock = self.root / MODULE.EXPECTED_LOCKS[1]
        value = lock.read_text(encoding="utf-8").replace(
            f'source = "git+{MODULE.SP1_REPOSITORY}?rev={MODULE.SP1_COMMIT}#{MODULE.SP1_COMMIT}"',
            'source = "registry+https://github.com/rust-lang/crates.io-index"',
            1,
        )
        lock.write_text(value, encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "expected SP1 graph"):
            MODULE.verify(self.root)

    def test_forked_field_fails_closed(self) -> None:
        manifest = self.root / MODULE.EXPECTED_MANIFESTS[1]
        manifest.write_text(
            manifest.read_text(encoding="utf-8")
            + f'\np3-field = {{ git = "{MODULE.SP1_REPOSITORY}", rev = "{MODULE.SP1_COMMIT}" }}\n',
            encoding="utf-8",
        )
        with self.assertRaisesRegex(ValueError, "must not replace"):
            MODULE.verify(self.root)

    def test_field_checksum_drift_fails_closed(self) -> None:
        lock = self.root / MODULE.EXPECTED_LOCKS[0]
        lock.write_text(
            lock.read_text(encoding="utf-8").replace(
                MODULE.P3_FIELD_CHECKSUM, "0" * 64, 1
            ),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(ValueError, "canonical"):
            MODULE.verify(self.root)

    def test_manifest_revision_drift_fails_closed(self) -> None:
        manifest = self.root / MODULE.EXPECTED_MANIFESTS[1]
        manifest.write_text(
            manifest.read_text(encoding="utf-8").replace(MODULE.SP1_COMMIT, "0" * 40, 1),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(ValueError, "must pin"):
            MODULE.verify(self.root)

    def test_rust_version_drift_fails_closed(self) -> None:
        manifest = self.root / MODULE.EXPECTED_MANIFESTS[1]
        manifest.write_text(
            manifest.read_text(encoding="utf-8").replace(
                f'rust-version = "{MODULE.GUEST_RUST_MIN_VERSION}"',
                'rust-version = "1.95"',
                1,
            ),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(ValueError, "rust-version"):
            MODULE.verify(self.root)

    def test_release_identity_drift_fails_closed(self) -> None:
        identity_path = self.root / MODULE.IDENTITY_PATHS[0]
        identity = json.loads(identity_path.read_text(encoding="utf-8"))
        identity["sp1_commit"] = "0" * 40
        identity_path.write_text(json.dumps(identity), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "does not pin the patched SP1 commit"):
            MODULE.verify(self.root)

    def test_runtime_identity_drift_fails_closed(self) -> None:
        identity_path = self.root / MODULE.IDENTITY_PATHS[0]
        identity = json.loads(identity_path.read_text(encoding="utf-8"))
        identity["sp1_runtime_commit"] = "0" * 40
        identity_path.write_text(json.dumps(identity), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "corrected SP1 runtime commit"):
            MODULE.verify(self.root)

    def test_guest_toolchain_identity_drift_fails_closed(self) -> None:
        identity_path = self.root / MODULE.IDENTITY_PATHS[0]
        identity = json.loads(identity_path.read_text(encoding="utf-8"))
        identity["sp1_guest_rust_version"] = "1.95.0-dev"
        identity_path.write_text(json.dumps(identity), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "SP1 guest Rust toolchain"):
            MODULE.verify(self.root)


if __name__ == "__main__":
    unittest.main()
