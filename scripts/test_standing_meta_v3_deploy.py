#!/usr/bin/env python3

from __future__ import annotations

import ast
import importlib.util
from pathlib import Path
import re
import unittest


SCRIPT = Path(__file__).with_name("standing_meta_v3_deploy.py")
WORKFLOW = SCRIPT.parent.parent / ".github" / "workflows" / "standing-meta-v3-deploy.yml"
SPEC = importlib.util.spec_from_file_location("standing_meta_v3_deploy", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def workflow_job(name: str) -> str:
    match = re.search(
        rf"^  {re.escape(name)}:\n(.*?)(?=^  [\w-]+:|\Z)",
        WORKFLOW.read_text(),
        re.MULTILINE | re.DOTALL,
    )
    assert match, f"missing workflow job: {name}"
    return match.group(1)


def job_admits(name: str, event: str, *, user="NSPG13", issue=527,
               body="/agent-bounty deploy-v3-policy", pull_request=None) -> bool:
    """Evaluate only the checked-in job's simple boolean authorization syntax."""
    match = re.search(r"^    if: >-\n((?:      .*\n)+)", workflow_job(name), re.MULTILINE)
    assert match, f"missing folded job condition: {name}"
    expression = " ".join(match.group(1).splitlines()).strip()
    expression = expression.replace("&&", " and ").replace("||", " or ")
    context = {"github": {"repository": "NSPG13/agent-bounties", "ref": "refs/heads/main", "event_name": event, "event": {
        "issue": {"number": issue, "pull_request": pull_request},
        "comment": {"user": {"login": user}, "body": body},
    }}, "null": None}

    def evaluate(node):
        if isinstance(node, ast.Constant):
            return node.value
        if isinstance(node, ast.Name):
            return context[node.id]
        if isinstance(node, ast.Attribute):
            return evaluate(node.value)[node.attr]
        if isinstance(node, ast.BoolOp) and isinstance(node.op, (ast.And, ast.Or)):
            values = (evaluate(value) for value in node.values)
            return all(values) if isinstance(node.op, ast.And) else any(values)
        if isinstance(node, ast.Compare) and len(node.ops) == 1:
            left, right = evaluate(node.left), evaluate(node.comparators[0])
            if isinstance(node.ops[0], ast.Eq):
                return left == right
            if isinstance(node.ops[0], ast.NotEq):
                return left != right
        raise AssertionError(f"unsupported workflow authorization syntax: {ast.dump(node)}")

    return bool(evaluate(ast.parse(expression, mode="eval").body))


class StandingMetaV3DeployTests(unittest.TestCase):
    def test_automatic_events_cannot_send_mainnet_transactions(self) -> None:
        for event in ("push", "pull_request", "schedule", "workflow_run"):
            with self.subTest(event=event):
                self.assertFalse(job_admits("base-mainnet-deploy", event))
        for event in ("push", "pull_request"):
            with self.subTest(validation_event=event):
                self.assertTrue(job_admits("deterministic-gates", event))

    def test_existing_explicit_manual_authorization_is_preserved(self) -> None:
        for job in ("base-mainnet-deploy", "deterministic-gates"):
            self.assertTrue(job_admits(job, "workflow_dispatch"))
            self.assertTrue(job_admits(job, "issue_comment"))
            for changed in (
                {"user": "another-user"},
                {"issue": 528},
                {"pull_request": {"url": "https://example.test/pr/527"}},
                {"body": "please deploy"},
                {"body": "/agent-bounty deploy-v3-policy extra"},
            ):
                with self.subTest(job=job, changed=changed):
                    self.assertFalse(job_admits(job, "issue_comment", **changed))

    def test_validation_remains_separate_from_the_guarded_sender(self) -> None:
        validation = workflow_job("deterministic-gates")
        sender = workflow_job("base-mainnet-deploy")
        self.assertIn("python scripts/standing_meta_v3_deploy.py plan", validation)
        self.assertNotIn("BASE_KEEPER_PRIVATE_KEY", validation)
        self.assertNotIn("python scripts/standing_meta_v3_deploy.py deploy", validation)
        self.assertIn("needs: deterministic-gates", sender)
        self.assertIn("group: agent-bounties-shared-base-keeper", sender)
        self.assertIn("cancel-in-progress: false", sender)

    def test_deployment_notices_do_not_depend_on_checkout(self) -> None:
        issue_comment_commands = [
            line.strip()
            for line in WORKFLOW.read_text().splitlines()
            if "gh issue comment 527" in line
        ]
        self.assertEqual(len(issue_comment_commands), 3)
        for command in issue_comment_commands:
            self.assertIn('--repo "$GITHUB_REPOSITORY"', command)

    def test_economics_require_exact_four_parent_funding(self) -> None:
        self.assertEqual(MODULE.PARENT_TARGET_BASE_UNITS, 2_010_000)
        self.assertEqual(MODULE.REPLACEMENT_COUNT, 4)
        self.assertEqual(MODULE.REPLACEMENT_FUNDING_REQUIRED, 8_040_000)

    def test_addresses_and_salt_are_pinned(self) -> None:
        for value in (
            MODULE.SINGLETON_FACTORY,
            MODULE.CANONICAL_FACTORY,
            MODULE.NATIVE_USDC,
            MODULE.PARTICIPANT_REGISTRY,
            MODULE.TERMS_REGISTRY,
            MODULE.VERIFIER_ONE,
            MODULE.VERIFIER_TWO,
            MODULE.EXPECTED_KEEPER,
            MODULE.BOUNDED_WALLET,
        ):
            self.assertRegex(value, r"^0x[0-9a-f]{40}$")
        self.assertEqual(
            MODULE.DEPLOYMENT_SALT_TEXT,
            "agent-bounties/standing-meta-v3/base-mainnet/v1",
        )

    def test_parse_uint_accepts_cast_decimal_and_hex_prefixes(self) -> None:
        self.assertEqual(MODULE.parse_uint("8040000", "amount"), 8_040_000)
        self.assertEqual(MODULE.parse_uint("0x7ab620 [8.04e6]", "amount"), 0x7AB620)

    def test_parse_uint_rejects_non_numeric_output(self) -> None:
        with self.assertRaises(MODULE.DeploymentError):
            MODULE.parse_uint("not-a-number", "amount")

    def test_address_and_bytes32_validation_fail_closed(self) -> None:
        self.assertEqual(
            MODULE.require_address(MODULE.CANONICAL_FACTORY.upper().replace("0X", "0x"), "factory"),
            MODULE.CANONICAL_FACTORY,
        )
        self.assertEqual(MODULE.require_bytes32("0x" + "ab" * 32, "hash"), "0x" + "ab" * 32)
        with self.assertRaises(MODULE.DeploymentError):
            MODULE.require_address("0x1234", "factory")
        with self.assertRaises(MODULE.DeploymentError):
            MODULE.require_bytes32("0x1234", "hash")

    def test_markdown_discloses_funding_and_policy_boundaries(self) -> None:
        report = {
            "predicted_verifier_module": "0x" + "11" * 20,
            "already_deployed": False,
            "keeper": {
                "usdc_balance_base_units": 0,
                "can_fund_four_replacements": False,
            },
            "bounded_wallet": {
                "usdc_balance_base_units": 89_000_000,
                "deterministic_verifier_module": "0x" + "22" * 20,
                "requires_owner_policy_update": True,
            },
            "replacement_economics": {"total_funding_required": 8_040_000},
        }
        text = MODULE.markdown(report)
        self.assertIn("Keeper can fund all four directly: **false**", text)
        self.assertIn("Owner policy update required", text)
        self.assertIn("not deployment or funding evidence", text)


if __name__ == "__main__":
    unittest.main()
