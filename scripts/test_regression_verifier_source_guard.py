#!/usr/bin/env python3
"""Deterministic tests for the regression-verifier source guard."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("regression_verifier_source_guard.py")
SPEC = importlib.util.spec_from_file_location("regression_verifier_source_guard", SCRIPT)
assert SPEC and SPEC.loader
GUARD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GUARD)


class RegressionVerifierSourceGuardTests(unittest.TestCase):
    def fixture(self, root: Path) -> None:
        (root / ".cargo").mkdir(parents=True)
        (root / "crates" / "worker" / "src").mkdir(parents=True)
        (root / "scripts").mkdir(parents=True)
        (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
        (root / "Cargo.lock").write_text("version = 4\n", encoding="utf-8")
        (root / ".cargo" / "config.toml").write_text("[net]\noffline = true\n", encoding="utf-8")
        (root / "crates" / "worker" / "Cargo.toml").write_text(
            "[package]\nname = 'worker'\nversion = '0.1.0'\n", encoding="utf-8"
        )
        (root / "crates" / "worker" / "src" / "main.rs").write_text(
            "fn main() {}\n", encoding="utf-8"
        )
        for relative in GUARD.RUNTIME_FILES:
            target = root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(f"runtime:{relative}\n", encoding="utf-8")

    def test_build_digest_is_stable_and_path_bound(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            first = GUARD.source_digest(root, "worker-build")
            second = GUARD.source_digest(root, "worker-build")
            self.assertEqual(first, second)
            source = root / "crates" / "worker" / "src" / "main.rs"
            source.write_text("fn main() { panic!(); }\n", encoding="utf-8")
            self.assertNotEqual(first, GUARD.source_digest(root, "worker-build"))

    def test_new_build_script_changes_worker_digest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            before = GUARD.source_digest(root, "worker-build")
            (root / "crates" / "worker" / "build.rs").write_text(
                "fn main() { println!(\"cargo:warning=unexpected\"); }\n",
                encoding="utf-8",
            )
            self.assertNotEqual(before, GUARD.source_digest(root, "worker-build"))

    def test_runtime_digest_detects_post_build_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            before = GUARD.source_digest(root, "signing-runtime")
            pipeline = root / "scripts" / "regression_verifier_pipeline.py"
            pipeline.write_text("print('replaced')\n", encoding="utf-8")
            self.assertNotEqual(before, GUARD.source_digest(root, "signing-runtime"))

    def test_missing_guarded_input_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            (root / "Cargo.lock").unlink()
            with self.assertRaisesRegex(GUARD.GuardError, "missing guarded build input"):
                GUARD.source_digest(root, "worker-build")


if __name__ == "__main__":
    unittest.main()
