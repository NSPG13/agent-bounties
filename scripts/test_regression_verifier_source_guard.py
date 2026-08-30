#!/usr/bin/env python3
"""Deterministic tests for the regression-verifier source guard."""

from __future__ import annotations

import fnmatch
import importlib.util
import re
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("regression_verifier_source_guard.py")
REPOSITORY = SCRIPT.resolve().parents[1]
SHARED_KEEPER_CONCURRENCY = "agent-bounties-shared-base-keeper"
SPEC = importlib.util.spec_from_file_location("regression_verifier_source_guard", SCRIPT)
assert SPEC and SPEC.loader
GUARD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GUARD)


class RegressionVerifierSourceGuardTests(unittest.TestCase):
    def fixture(self, root: Path) -> None:
        (root / ".cargo").mkdir(parents=True)
        (root / "crates" / "worker" / "src").mkdir(parents=True)
        (root / "migrations").mkdir(parents=True)
        (root / "schemas").mkdir(parents=True)
        (root / "scripts").mkdir(parents=True)
        (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
        (root / "Cargo.lock").write_text("version = 4\n", encoding="utf-8")
        (root / ".cargo" / "config.toml").write_text("[net]\noffline = true\n", encoding="utf-8")
        (root / "crates" / "worker" / "Cargo.toml").write_text(
            "[package]\nname = 'worker'\nversion = '0.1.0'\n", encoding="utf-8"
        )
        (root / "crates" / "worker" / "src" / "main.rs").write_text(
            'const MIGRATION: &str = include_str!("../../../migrations/0001_core.sql");\n'
            'const SCHEMA: &[u8] = include_bytes!("../../../schemas/discovery.json");\n'
            "fn main() {}\n",
            encoding="utf-8",
        )
        (root / "migrations" / "0001_core.sql").write_text("select 1;\n", encoding="utf-8")
        (root / "schemas" / "discovery.json").write_text("{}\n", encoding="utf-8")
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

    def test_out_of_tree_compile_time_inputs_change_worker_digest(self) -> None:
        for relative in ("migrations/0001_core.sql", "schemas/discovery.json"):
            with self.subTest(relative=relative), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                self.fixture(root)
                before = GUARD.source_digest(root, "worker-build")
                (root / relative).write_text("changed\n", encoding="utf-8")
                self.assertNotEqual(before, GUARD.source_digest(root, "worker-build"))

    def test_non_literal_compile_time_input_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            (root / "crates" / "worker" / "src" / "main.rs").write_text(
                'const DATA: &[u8] = include_bytes!(env!("UNBOUND_FILE"));\nfn main() {}\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(GUARD.GuardError, "non-literal"):
                GUARD.source_digest(root, "worker-build")

    def test_toolchain_override_changes_worker_digest(self) -> None:
        for override in GUARD.OPTIONAL_BUILD_ROOTS:
            with self.subTest(override=override), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                self.fixture(root)
                before = GUARD.source_digest(root, "worker-build")
                (root / override).write_text("1.99.0\n", encoding="utf-8")
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

    def test_checked_in_workflows_scope_key_and_watch_all_external_build_inputs(self) -> None:
        reusable = (
            REPOSITORY / ".github/workflows/regression-verifier-signing-reusable.yml"
        ).read_text(encoding="utf-8")
        secret = "${{ secrets.verifier_private_key }}"
        self.assertEqual(reusable.count(secret), 1)
        self.assertGreater(
            reusable.index(secret),
            reusable.index("Re-fetch state and sign one exact candidate set"),
        )
        compile_time_inputs = GUARD._compile_time_inputs(
            REPOSITORY,
            set(GUARD._guarded_files(REPOSITORY, "worker-build")),
        )
        root_prefixes = tuple(
            f"{relative}/" for relative in GUARD.BUILD_ROOTS if (REPOSITORY / relative).is_dir()
        )
        external_inputs = sorted(
            path.relative_to(REPOSITORY).as_posix()
            for path in compile_time_inputs
            if not path.relative_to(REPOSITORY).as_posix().startswith(root_prefixes)
        )
        self.assertTrue(external_inputs)
        for relative in (
            ".github/workflows/regression-verifier-runner.yml",
            ".github/workflows/regression-verifier-signer.yml",
        ):
            workflow = (REPOSITORY / relative).read_text(encoding="utf-8")
            if "pull_request:" in workflow:
                self.assertIn('      - "rust-toolchain"', workflow)
                self.assertIn('      - "rust-toolchain.toml"', workflow)
                paths_block = workflow.split("    paths:\n", 1)[1].split("  schedule:", 1)[0]
                watched = re.findall(r'^\s+- "([^"]+)"$', paths_block, re.MULTILINE)
                for included in external_inputs:
                    self.assertTrue(
                        any(fnmatch.fnmatchcase(included, pattern) for pattern in watched),
                        f"{relative} does not watch compile-time input {included}",
                    )

        keeper_workflows = []
        for workflow_path in (REPOSITORY / ".github/workflows").glob("*.yml"):
            workflow = workflow_path.read_text(encoding="utf-8")
            if "BASE_KEEPER_PRIVATE_KEY" in workflow:
                keeper_workflows.append(workflow_path.name)
                self.assertIn(SHARED_KEEPER_CONCURRENCY, workflow, workflow_path.name)
                self.assertIn("cancel-in-progress", workflow, workflow_path.name)
        self.assertTrue(keeper_workflows)


if __name__ == "__main__":
    unittest.main()
