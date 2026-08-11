#!/usr/bin/env python3
"""Tests for PR-first funded-bounty recovery guidance."""

from __future__ import annotations

import unittest

import github_pr_claim_guard as guard


CONTRACT = "0x" + "12" * 20


class GitHubPrClaimGuardTests(unittest.TestCase):
    def test_linked_issue_parser_is_bounded_and_deduplicated(self) -> None:
        self.assertEqual(
            guard.linked_issue_numbers("Fixes #635\nCloses #635\nResolves #637"),
            [635, 637],
        )

    def test_claimable_funded_issue_gets_one_exact_recovery_action(self) -> None:
        issues = {
            635: {
                "labels": [{"name": "bounty"}, {"name": "funded-live"}],
                "body": f"Contract: `{CONTRACT}`",
                "html_url": "https://github.com/example/repo/issues/635",
            }
        }
        inventory = [
            {
                "bounty_contract": CONTRACT,
                "status": "claimable",
                "terms_valid": True,
                "verification_ready": True,
            }
        ]

        links = guard.claimable_links([635], issues, inventory)
        comment = guard.render_comment(links)

        self.assertEqual(len(links), 1)
        self.assertIn("/claim #635 wallet: 0xYOUR_PUBLIC_BASE_ADDRESS", comment)
        self.assertIn("only canonical `BountySettled` proves payment", comment)

    def test_noncanonical_or_already_claimed_work_does_not_get_claim_prompt(self) -> None:
        issues = {
            635: {
                "labels": [{"name": "bounty"}, {"name": "funded-live"}],
                "body": f"Contract: `{CONTRACT}`",
            }
        }
        inventory = [
            {
                "bounty_contract": CONTRACT,
                "status": "claimed",
                "terms_valid": True,
                "verification_ready": True,
            }
        ]

        self.assertEqual(guard.claimable_links([635], issues, inventory), [])
        self.assertEqual(guard.claimable_links([999], issues, inventory), [])


if __name__ == "__main__":
    unittest.main()
