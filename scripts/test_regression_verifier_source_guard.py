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

    def test_line_endings_are_exact_build_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            before = GUARD.source_digest(root, "worker-build")
            migration = root / "migrations" / "0001_core.sql"
            migration.write_bytes(migration.read_bytes().replace(b"\n", b"\r\n"))
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

    def test_commented_macro_tokens_cannot_hide_dynamic_include(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            (root / "crates" / "worker" / "src" / "main.rs").write_text(
                'const DATA: &[u8] = include_bytes /* nested /* legal */ comment */ '
                '!(env!("UNBOUND_FILE"));\nfn main() {}\n',
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

        keeper_workflows = GUARD.validate_keeper_workflow_locks(REPOSITORY)
        self.assertIn("regression-verifier-signer.yml", keeper_workflows)

    def test_keeper_lock_parser_rejects_yaml_extension_and_block_scalar_decoys(self) -> None:
        malicious_documents = {
            "unlocked.yaml": """name: Evil\non: workflow_dispatch\njobs:\n  send:\n    runs-on: ubuntu-latest\n    env:\n      KEY: ${{ secrets.BASE_KEEPER_PRIVATE_KEY }}\n    steps: []\n""",
            "decoy.yml": """name: Decoy\non: workflow_dispatch\njobs:\n  send:\n    runs-on: ubuntu-latest\n    steps:\n      - run: |\n          group: agent-bounties-shared-base-keeper\n          cancel-in-progress: false\n          echo '${{ secrets.BASE_KEEPER_PRIVATE_KEY }}'\n""",
            "workflow-level.yml": """name: Starvation\non: issue_comment\nconcurrency:\n  group: agent-bounties-shared-base-keeper\n  cancel-in-progress: false\njobs:\n  check:\n    runs-on: ubuntu-latest\n    steps: []\n  send:\n    runs-on: ubuntu-latest\n    env:\n      KEY: ${{ secrets.BASE_KEEPER_PRIVATE_KEY }}\n    steps: []\n""",
            "alias.yml": """name: Alias\non: workflow_dispatch\njobs:\n  locked:\n    runs-on: ubuntu-latest\n    concurrency:\n      group: agent-bounties-shared-base-keeper\n      cancel-in-progress: false\n    env:\n      KEY: &keeper ${{ secrets.BASE_KEEPER_PRIVATE_KEY }}\n    steps: []\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      KEY: *keeper\n    steps: []\n""",
            "unicode-alias.yml": """name: Alias\non: workflow_dispatch\njobs:\n  locked:\n    runs-on: ubuntu-latest\n    concurrency:\n      group: agent-bounties-shared-base-keeper\n      cancel-in-progress: false\n    env:\n      KEY: &κλειδί ${{ secrets.BASE_KEEPER_PRIVATE_KEY }}\n    steps: []\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      KEY: *κλειδί\n    steps: []\n""",
            "multiline-quoted.yml": "name: Folded secret\non: workflow_dispatch\njobs:\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      KEY: \"${{ secrets.BASE_KEEPER_\\\nPRIVATE_KEY }}\"\n    steps: []\n",
            "yaml-escaped-secret.yml": "name: Escaped secret\non: workflow_dispatch\njobs:\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      KEY: \"${{ secrets.BASE_KEEPER_\\x50RIVATE_KEY }}\"\n    steps: []\n",
            "json-escaped-secret.yml": '{"jobs":{"unlocked":{"runs-on":"ubuntu-latest","env":{"KEY":"${{ secrets.BASE_KEEPER_\\u0050RIVATE_KEY }}"},"steps":[]}}}',
            "bracket-whitespace.yml": """name: Bracket whitespace\non: workflow_dispatch\njobs:\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      KEY: ${{ secrets['BASE_KEEPER_PRIVATE_KEY' ] }}\n    steps: []\n""",
            "dynamic-secret-index.yml": """name: Dynamic index\non: workflow_dispatch\njobs:\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      KEY: ${{ secrets[format('BASE_KEEPER_{0}', 'PRIVATE_KEY')] }}\n    steps: []\n""",
            "folded-dynamic-index.yml": """name: Folded dynamic index\non: workflow_dispatch\njobs:\n  unlocked:\n    runs-on: ubuntu-latest\n    steps:\n      - run: >-\n          echo \"${{ secrets\n          [format('BASE_KEEPER_{0}', 'PRIVATE_KEY')] }}\"\n""",
        }
        for name, document in malicious_documents.items():
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                workflow_root = root / ".github" / "workflows"
                workflow_root.mkdir(parents=True)
                (workflow_root / name).write_text(document, encoding="utf-8")
                with self.assertRaises(GUARD.GuardError):
                    GUARD.validate_keeper_workflow_locks(root)

    def test_keeper_lock_parser_accepts_only_the_key_bearing_job(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            workflow = root / ".github" / "workflows" / "safe.yaml"
            workflow.parent.mkdir(parents=True)
            workflow.write_text(
                """name: Safe\non: issue_comment\njobs:\n  check:\n    runs-on: ubuntu-latest\n    steps: []\n  send:\n    if: github.event_name == 'workflow_dispatch'\n    runs-on: ubuntu-latest\n    concurrency:\n      group: agent-bounties-shared-base-keeper\n      cancel-in-progress: false\n    env:\n      KEY: ${{ secrets.BASE_KEEPER_PRIVATE_KEY }}\n    steps: []\n""",
                encoding="utf-8",
            )
            self.assertEqual(GUARD.validate_keeper_workflow_locks(root), ["safe.yaml"])


if __name__ == "__main__":
    unittest.main()
