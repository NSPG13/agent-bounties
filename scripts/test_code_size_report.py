#!/usr/bin/env python3
"""Tests for the read-only code-size reporter."""

from __future__ import annotations

import json
import pathlib
import subprocess
import tempfile
import unittest

import code_size_report


POLICY = {
    "schema_version": "agent-bounties/code-size-policy-v1",
    "metric": "nonblank_physical_lines",
    "minimum_duplicate_lines": 3,
    "groups": [
        {"name": "code", "prefixes": ["src/"], "extensions": [".py"]},
        {
            "name": "contract-tests",
            "prefixes": ["contracts/test/"],
            "extensions": [".sol"],
        },
    ],
    "pattern_groups": [{"name": "config", "patterns": ["Cargo.toml"]}],
    "protected_prefixes": ["contracts/src/", "migrations/"],
    "protected_patterns": ["**/fixtures/**", "*.lock", "**/*.lock", "*.md", "**/*.md"],
}


class CodeSizeReportTests(unittest.TestCase):
    def test_classification_keeps_contract_tests_and_freezes_sources(self) -> None:
        self.assertEqual(code_size_report.classify_path("src/main.py", POLICY), "code")
        self.assertEqual(
            code_size_report.classify_path("contracts/test/Thing.t.sol", POLICY),
            "contract-tests",
        )
        self.assertIsNone(code_size_report.classify_path("contracts/src/Thing.sol", POLICY))
        self.assertIsNone(code_size_report.classify_path("docs/design.md", POLICY))
        self.assertEqual(code_size_report.classify_path("Cargo.toml", POLICY), "config")
        self.assertTrue(code_size_report.is_protected("src/fixtures/input.py", POLICY))
        self.assertTrue(code_size_report.is_protected("Cargo.lock", POLICY))
        self.assertTrue(code_size_report.is_protected("README.md", POLICY))

    def test_duplicate_blocks_are_coalesced(self) -> None:
        shared = "\n".join(
            f"perform_shared_operation_{index}(validated_repository_value)" for index in range(6)
        )
        files = {
            "src/a.py": {"text": f"before_a()\n{shared}\nafter_a()", "group": "code"},
            "src/b.py": {"text": f"before_b()\n{shared}\nafter_b()", "group": "code"},
        }
        blocks = code_size_report.duplicate_blocks(files, 3)
        self.assertEqual(len(blocks), 1)
        self.assertEqual(blocks[0]["lines"], 6)
        self.assertEqual(blocks[0]["left"]["start"], 2)

    def test_duplicate_blocks_include_nonoverlapping_repetition_in_one_file(self) -> None:
        shared = "\n".join(
            f"perform_shared_operation_{index}(validated_repository_value)" for index in range(4)
        )
        files = {
            "src/a.py": {"text": f"{shared}\nbetween()\n{shared}", "group": "code"},
        }
        blocks = code_size_report.duplicate_blocks(files, 3)
        self.assertEqual(len(blocks), 1)
        self.assertEqual(blocks[0]["lines"], 4)
        self.assertEqual(blocks[0]["left"]["path"], "src/a.py")
        self.assertEqual(blocks[0]["right"]["start"], 6)

    def test_report_compares_git_refs_without_mutating_them(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = pathlib.Path(directory)
            subprocess.run(["git", "init", "-q"], cwd=repo, check=True)
            subprocess.run(
                ["git", "config", "user.email", "test@example.com"], cwd=repo, check=True
            )
            subprocess.run(
                ["git", "config", "user.name", "Code Size Test"], cwd=repo, check=True
            )
            (repo / "src").mkdir()
            (repo / "src/main.py").write_text("one()\n\ntwo()\n", encoding="utf-8")
            (repo / "contracts/src").mkdir(parents=True)
            (repo / "contracts/src/Thing.sol").write_text("protected\n", encoding="utf-8")
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-qm", "base"], cwd=repo, check=True)
            base = subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=repo, text=True
            ).strip()

            (repo / "src/main.py").write_text("one()\ntwo()\nthree()\n", encoding="utf-8")
            (repo / "contracts/src/Thing.sol").write_text("changed\n", encoding="utf-8")
            subprocess.run(["git", "commit", "-qam", "head"], cwd=repo, check=True)

            report = code_size_report.build_report(repo, POLICY, "policy", "HEAD", base, [7, 30])
            self.assertEqual(report["base"]["nonblank_lines"], 2)
            self.assertEqual(report["head"]["nonblank_lines"], 3)
            self.assertEqual(report["delta"], 1)
            self.assertEqual(report["protected_changes"][0]["path"], "contracts/src/Thing.sol")
            self.assertEqual([trend["days"] for trend in report["trends"]], [7, 30])
            json.dumps(report)

    def test_worktree_reports_untracked_protected_files(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = pathlib.Path(directory)
            subprocess.run(["git", "init", "-q"], cwd=repo, check=True)
            subprocess.run(
                ["git", "config", "user.email", "test@example.com"], cwd=repo, check=True
            )
            subprocess.run(
                ["git", "config", "user.name", "Code Size Test"], cwd=repo, check=True
            )
            (repo / "src").mkdir()
            (repo / "src/main.py").write_text("one()\n", encoding="utf-8")
            subprocess.run(["git", "add", "."], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-qm", "base"], cwd=repo, check=True)
            (repo / "src/fixtures").mkdir()
            (repo / "src/fixtures/contract.json").write_text("{}\n", encoding="utf-8")

            changes = code_size_report.changed_protected_paths(
                repo, "HEAD", code_size_report.WORKTREE_REF, POLICY
            )
            self.assertEqual(
                changes,
                [{"status": "??", "path": "src/fixtures/contract.json"}],
            )


if __name__ == "__main__":
    unittest.main()
