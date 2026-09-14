"""Artifact-root allowlist, symlink rejection, regular-file and size checks;
network allowlist."""
import os
import pathlib
import tempfile
import unittest

from taskmarket_adapter import security
from taskmarket_adapter.errors import SecurityError


class SecurityTests(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(self._cleanup)
        self.root = self.tmp / "artifacts"
        (self.root / "sub").mkdir(parents=True)

    def _cleanup(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    # ---------- network ----------
    def test_network_allowlist(self):
        for ok in ("base-mainnet", "base-sepolia"):
            self.assertEqual(security.validate_network(ok), ok)
        self.assertEqual(security.validate_network(None), "base-mainnet")

    def test_unknown_network_refused(self):
        for bad in ("ethereum", "base", "BASE-MAINNET", "", "base-mainnet; rm -rf /"):
            with self.assertRaises(SecurityError, msg=bad):
                security.validate_network(bad)

    # ---------- roots ----------
    def test_roots_required(self):
        with self.assertRaises(SecurityError):
            security.artifact_roots({})

    def test_relative_root_refused(self):
        with self.assertRaises(SecurityError):
            security.artifact_roots({security.ENV_ARTIFACT_ROOTS: "relative/path"})

    # ---------- resolve_artifact ----------
    def test_file_inside_root_resolves(self):
        target = self.root / "sub" / "work.zip"
        target.write_bytes(b"data")
        resolved = security.resolve_artifact(str(target), [self.root], 1000)
        self.assertEqual(resolved, target.resolve())

    def test_outside_root_refused(self):
        outside = self.tmp / "secret.txt"
        outside.write_text("keys")
        with self.assertRaises(SecurityError):
            security.resolve_artifact(str(outside), [self.root], 1000)

    def test_traversal_via_parent_refused(self):
        outside = self.tmp / "secret.txt"
        outside.write_text("keys")
        sneaky = self.root / ".." / "secret.txt"
        with self.assertRaises(SecurityError):
            security.resolve_artifact(str(sneaky), [self.root], 1000)

    def test_symlink_inside_root_pointing_outside_refused(self):
        outside = self.tmp / "wallet.dat"
        outside.write_text("seed")
        link = self.root / "deliverable.zip"
        os.symlink(outside, link)
        with self.assertRaises(SecurityError):
            security.resolve_artifact(str(link), [self.root], 1000)

    def test_symlink_directory_component_refused(self):
        outside_dir = self.tmp / "elsewhere"
        outside_dir.mkdir()
        (outside_dir / "a.txt").write_text("x")
        link_dir = self.root / "shortcut"
        os.symlink(outside_dir, link_dir)
        with self.assertRaises(SecurityError):
            security.resolve_artifact(str(link_dir / "a.txt"), [self.root], 1000)

    def test_non_regular_file_refused(self):
        fifo = self.root / "pipe"
        os.mkfifo(fifo)
        with self.assertRaises(SecurityError):
            security.resolve_artifact(str(fifo), [self.root], 1000)
        with self.assertRaises(SecurityError):
            security.resolve_artifact(str(self.root), [self.root], 1000)  # directory

    def test_missing_file_refused(self):
        with self.assertRaises(SecurityError):
            security.resolve_artifact(str(self.root / "nope.bin"), [self.root], 1000)

    def test_oversized_file_refused(self):
        big = self.root / "big.bin"
        big.write_bytes(b"x" * 11)
        with self.assertRaises(SecurityError):
            security.resolve_artifact(str(big), [self.root], 10)
        security.resolve_artifact(str(big), [self.root], 11)  # exactly at limit passes

    def test_relative_path_refused(self):
        with self.assertRaises(SecurityError):
            security.resolve_artifact("artifacts/work.zip", [self.root], 1000)

    def test_size_limit_from_env(self):
        self.assertEqual(security.max_artifact_bytes({}), security.DEFAULT_MAX_ARTIFACT_BYTES)
        self.assertEqual(security.max_artifact_bytes({security.ENV_MAX_ARTIFACT_BYTES: "42"}), 42)
        with self.assertRaises(SecurityError):
            security.max_artifact_bytes({security.ENV_MAX_ARTIFACT_BYTES: "-1"})


if __name__ == "__main__":
    unittest.main()
