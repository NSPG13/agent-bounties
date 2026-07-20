#!/usr/bin/env python3
"""Pure security-contract tests for external PR classification and feedback."""

from __future__ import annotations

import pathlib
import unittest

import review_external_pr as review


class ExternalPrReviewTests(unittest.TestCase):
    def test_path_classification_is_fail_closed(self) -> None:
        cases = (
            ("docs/guide.md", True, False),
            (".github/ISSUE_TEMPLATE/bounty.yml", True, False),
            ("crates/api/src/main.rs", False, True),
            ("scripts/review-external-pr.sh", False, True),
            ("Cargo.lock", False, True),
            ("site/home.js", False, False),
        )
        for path, docs, risky in cases:
            with self.subTest(path=path):
                self.assertEqual(review.is_docs_path(path), docs)
                self.assertEqual(review.is_risky_path(path), risky)

    def test_branch_names_are_bounded_and_safe(self) -> None:
        branch = review.collaboration_branch_name(42, "Fix API / Payment Safety!" * 8)
        self.assertRegex(branch, r"^collab/pr-42-[a-z0-9-]+$")
        self.assertLessEqual(len(branch.removeprefix("collab/pr-42-")), 48)
        with self.assertRaisesRegex(ValueError, "must be named collab"):
            review.validate_collaboration_branch("feature/untrusted")

    def test_feedback_and_issue_extraction_remain_constructive(self) -> None:
        issues = review.docs_contract_issues("docs/a.md:12: stale route\nnoise\ndocs/b.md:3: stale tool")
        items = review.feedback(False, ["scripts/a.py"], False, issues)
        self.assertEqual(issues, ["docs/a.md:12: stale route", "docs/b.md:3: stale tool"])
        self.assertEqual(len(items), 4)
        self.assertIn("line-by-line maintainer review", items[1])

    def test_platform_wrappers_delegate_only_to_trusted_engine(self) -> None:
        root = pathlib.Path(__file__).resolve().parent
        for name in ("review-external-pr.ps1", "review-external-pr.sh"):
            with self.subTest(name=name):
                text = (root / name).read_text(encoding="utf-8")
                self.assertIn("review_external_pr.py", text)
                self.assertNotIn("git worktree add", text)


if __name__ == "__main__":
    unittest.main()
