#!/usr/bin/env python3
"""Tests for paid-bounty issue validation routing."""

from __future__ import annotations

import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import github_issue_plan_comment as planner


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "paid-bounty-issues.yml"


class PaidBountyIssueWorkflowTests(unittest.TestCase):
    def test_canonical_funded_inventory_skips_intake_validation(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn(
            "!contains(toJson(github.event.issue.labels), '\"funded-live\"')",
            workflow,
        )
        self.assertLess(
            workflow.index("!contains(toJson(github.event.issue.labels)"),
            workflow.index("startsWith(github.event.issue.title"),
        )

    def test_closed_and_mirrored_issues_skip_before_allocating_a_runner(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")
        job_condition = workflow.split("runs-on:", 1)[0]
        self.assertIn("github.event.issue.state == 'open'", job_condition)
        for marker in planner.DISCOVERY_MARKERS:
            self.assertIn(f"!contains(github.event.issue.body || '', '{marker}')", job_condition)


class PaidBountyIssueExecutionTests(unittest.TestCase):
    def run_event(self, directory: Path, overrides: dict, stdout: io.StringIO) -> int:
        issue = {
            "number": 1340,
            "title": "[bounty]: Flower-shaped rainwater collector",
            "html_url": "https://github.com/example/project/issues/1340",
            "state": "open",
            "body": "### Goal\nImprove the CAD design.\n### Suggested amount\n17 USDC",
            "labels": [{"name": "bounty"}],
            **overrides,
        }
        event_file = directory / "event.json"
        event_file.write_text(json.dumps({"issue": issue, "repository": {"full_name": "example/project"}}))
        return planner.run_from_env(
            {
                "GITHUB_EVENT_PATH": str(event_file),
                "GITHUB_WORKSPACE": str(directory),
                "RUNNER_TEMP": str(directory / "generated"),
            },
            stdout,
        )

    def test_terminal_and_mirror_events_never_build_or_publish(self) -> None:
        cases = [
            {"state": "closed"},
            {"labels": [{"name": "bounty"}, {"name": "funded-live"}]},
            # Mirrors remain mirrors after a refund removes funded-live.
            *[{"body": "Maintainer refund update.\n\n" + marker + "\nSaved bounty state.",
               "labels": [{"name": "bounty"}, {"name": "cancelled"}]}
              for marker in planner.DISCOVERY_MARKERS],
        ]
        for event in cases:
            with self.subTest(event=event), tempfile.TemporaryDirectory() as temp:
                directory = Path(temp)
                stdout = io.StringIO()
                with mock.patch.object(planner, "run_github_plan") as build, mock.patch.object(planner, "publish_comment") as publish:
                    self.assertEqual(self.run_event(directory, event, stdout), 0)
                build.assert_not_called()
                publish.assert_not_called()
                self.assertFalse((directory / "generated").exists())
                self.assertIn("Skipped paid-bounty form validation", stdout.getvalue())

    def test_new_bounty_form_still_builds_and_publishes_validation(self) -> None:
        plan = {"check": {"conclusion": "Success", "title": "Bounty ready", "summary": "Valid form", "text": "Exact form reviewed"}}
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            with mock.patch.object(planner, "run_github_plan", return_value=json.dumps(plan)) as build, mock.patch.object(planner, "publish_comment") as publish:
                self.assertEqual(self.run_event(directory, {}, io.StringIO()), 0)
            build.assert_called_once()
            publish.assert_called_once()
            self.assertEqual(build.call_args.args[2]["number"], 1340)
            self.assertIn("### Goal", build.call_args.args[3].read_text())
            self.assertIn("Agent bounty validation: Success", publish.call_args.args[2])


if __name__ == "__main__":
    unittest.main()
