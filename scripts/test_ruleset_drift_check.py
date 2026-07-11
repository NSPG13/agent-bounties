#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import unittest
from copy import deepcopy
from pathlib import Path
from subprocess import CompletedProcess, TimeoutExpired
from unittest.mock import patch


SCRIPT_DIR = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "ruleset_drift_check", SCRIPT_DIR / "ruleset_drift_check.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def _rule(ruleset: dict, rule_type: str) -> dict:
    return next(rule for rule in ruleset["rules"] if rule["type"] == rule_type)


class RulesetDriftCheckTests(unittest.TestCase):
    def setUp(self) -> None:
        canonical_path = SCRIPT_DIR.parent / ".github" / "rulesets" / "main.json"
        live_path = SCRIPT_DIR / "fixtures" / "ruleset_drift" / "live_ruleset.json"
        self.canonical = json.loads(canonical_path.read_text(encoding="utf-8"))
        self.live = json.loads(live_path.read_text(encoding="utf-8"))

    def test_equal_ruleset_reports_no_drift_and_no_problems(self) -> None:
        # The live fixture carries server-owned ids, timestamps, links, per-rule
        # source metadata, and a shuffled rule order. None of that is drift.
        result = MODULE.evaluate(self.canonical, self.live)
        self.assertEqual(result["drift"], [])
        self.assertEqual(result["canonical_semantic_problems"], [])
        self.assertEqual(result["live_semantic_problems"], [])
        self.assertTrue(MODULE.is_clean(result))

    def test_missing_rule_is_flagged_as_drift_and_semantic_gap(self) -> None:
        live = deepcopy(self.live)
        live["rules"] = [rule for rule in live["rules"] if rule["type"] != "non_fast_forward"]
        result = MODULE.evaluate(self.canonical, live)
        self.assertTrue(result["drift"])
        self.assertIn(
            "non-fast-forward protection rule is missing",
            result["live_semantic_problems"],
        )
        self.assertFalse(MODULE.is_clean(result))

    def test_wrong_integration_id_is_flagged(self) -> None:
        live = deepcopy(self.live)
        checks = _rule(live, "required_status_checks")["parameters"]["required_status_checks"]
        for check in checks:
            if check["context"] == "full-check":
                check["integration_id"] = 99999
        result = MODULE.evaluate(self.canonical, live)
        self.assertTrue(result["drift"])
        self.assertTrue(
            any(
                "full-check" in problem and "15368" in problem
                for problem in result["live_semantic_problems"]
            )
        )

    def test_strict_status_mode_is_flagged(self) -> None:
        live = deepcopy(self.live)
        _rule(live, "required_status_checks")["parameters"][
            "strict_required_status_checks_policy"
        ] = True
        result = MODULE.evaluate(self.canonical, live)
        self.assertTrue(result["drift"])
        self.assertIn(
            "status checks must use loose mode (strict policy must be false)",
            result["live_semantic_problems"],
        )

    def test_over_broad_target_is_flagged(self) -> None:
        live = deepcopy(self.live)
        live["conditions"]["ref_name"]["include"] = ["~ALL"]
        result = MODULE.evaluate(self.canonical, live)
        self.assertTrue(result["drift"])
        self.assertTrue(
            any("~DEFAULT_BRANCH" in problem for problem in result["live_semantic_problems"])
        )

    def test_normalize_strips_only_server_owned_fields(self) -> None:
        normalized = MODULE.normalize_ruleset(self.live)
        self.assertNotIn("id", normalized)
        self.assertNotIn("_links", normalized)
        self.assertNotIn("updated_at", normalized)
        for rule in normalized["rules"]:
            self.assertNotIn("ruleset_id", rule)
            self.assertNotIn("ruleset_source", rule)
        # Policy-bearing fields survive normalization.
        self.assertEqual(normalized["enforcement"], "active")
        self.assertEqual(len(normalized["rules"]), len(self.live["rules"]))

    def test_nonempty_required_reviewers_remain_policy_drift(self) -> None:
        live = deepcopy(self.live)
        _rule(live, "pull_request")["parameters"]["required_reviewers"] = [
            {"file_patterns": ["src/**"], "minimum_approvals": 1}
        ]
        result = MODULE.evaluate(self.canonical, live)
        self.assertTrue(result["drift"])

    def test_admin_bypass_must_be_pull_request_mode_only(self) -> None:
        live = deepcopy(self.live)
        live["bypass_actors"][0]["bypass_mode"] = "always"
        result = MODULE.evaluate(self.canonical, live)
        self.assertIn(
            "admin bypass must use pull_request mode only",
            result["live_semantic_problems"],
        )

    def test_latest_push_and_single_approval_required(self) -> None:
        live = deepcopy(self.live)
        params = _rule(live, "pull_request")["parameters"]
        params["require_last_push_approval"] = False
        params["required_approving_review_count"] = 2
        result = MODULE.evaluate(self.canonical, live)
        self.assertIn(
            "pull request rule must require exactly one approval",
            result["live_semantic_problems"],
        )
        self.assertIn(
            "pull request rule must require latest-push approval",
            result["live_semantic_problems"],
        )

    def test_fetch_requires_one_repository_owned_match(self) -> None:
        owned = {
            "id": 10,
            "name": self.canonical["name"],
            "source_type": "Repository",
            "source": "NSPG13/agent-bounties",
        }
        with patch.object(MODULE, "_gh_json", return_value=[owned, {**owned, "id": 11}]):
            with self.assertRaisesRegex(RuntimeError, "found 2"):
                MODULE.fetch_live_ruleset("NSPG13/agent-bounties", self.canonical["name"])

    def test_fetch_rejects_zero_repository_owned_matches(self) -> None:
        inherited = {
            "id": 10,
            "name": self.canonical["name"],
            "source_type": "Organization",
            "source": "NSPG13",
        }
        with patch.object(MODULE, "_gh_json", return_value=[inherited]):
            with self.assertRaisesRegex(RuntimeError, "found 0"):
                MODULE.fetch_live_ruleset("NSPG13/agent-bounties", self.canonical["name"])

    def test_fetch_ignores_same_named_inherited_ruleset(self) -> None:
        owned = {
            "id": 10,
            "name": self.canonical["name"],
            "source_type": "Repository",
            "source": "nspg13/AGENT-BOUNTIES",
        }
        inherited = {
            "id": 20,
            "name": self.canonical["name"],
            "source_type": "Organization",
            "source": "NSPG13",
        }
        detail = {**self.live, "source_type": "Repository", "source": "NSPG13/agent-bounties"}
        with patch.object(MODULE, "_gh_json", side_effect=[[inherited, owned], detail]) as gh_json:
            result = MODULE.fetch_live_ruleset("NSPG13/agent-bounties", self.canonical["name"])
        self.assertIs(result, detail)
        self.assertEqual(
            gh_json.call_args_list[1].args[0],
            ["repos/NSPG13/agent-bounties/rulesets/10"],
        )

    def test_gh_timeout_fails_closed(self) -> None:
        with patch.object(
            MODULE.subprocess,
            "run",
            side_effect=TimeoutExpired(cmd=["gh", "api"], timeout=MODULE.GH_TIMEOUT_SECONDS),
        ):
            with self.assertRaisesRegex(RuntimeError, "timed out"):
                MODULE._gh_json(["repos/owner/repo/rulesets"])

    def test_gh_nonzero_exit_fails_closed_without_stderr(self) -> None:
        completed = CompletedProcess(
            args=["gh", "api"], returncode=7, stdout="", stderr="sensitive detail"
        )
        with patch.object(MODULE.subprocess, "run", return_value=completed):
            with self.assertRaisesRegex(RuntimeError, "exit code 7") as error:
                MODULE._gh_json(["repos/owner/repo/rulesets"])
        self.assertNotIn("sensitive", str(error.exception))

    def test_gh_invalid_json_fails_closed(self) -> None:
        completed = CompletedProcess(
            args=["gh", "api"], returncode=0, stdout="not-json", stderr=""
        )
        with patch.object(MODULE.subprocess, "run", return_value=completed) as run:
            with self.assertRaisesRegex(RuntimeError, "invalid JSON"):
                MODULE._gh_json(["repos/owner/repo/rulesets"])
        self.assertEqual(run.call_args.kwargs["encoding"], "utf-8")
        self.assertEqual(run.call_args.kwargs["errors"], "strict")


if __name__ == "__main__":
    unittest.main()
