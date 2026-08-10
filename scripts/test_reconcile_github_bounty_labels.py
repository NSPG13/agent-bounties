#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path
from unittest.mock import patch

from reconcile_github_bounty_labels import (
    LABEL_DEFINITIONS,
    MANAGED_START,
    HttpResult,
    LabelReconciliationError,
    build_plans,
    execute_plans,
    fetch_github_issues,
    issue_marker,
    main,
    plan_has_write,
    request_with_retry,
)
from github_claim_comment import open_competition_wrong_mode_plan


REPOSITORY = "NSPG13/agent-bounties"
NETWORK = "base-mainnet"
CHAIN_ID = 8453
NOW = "2026-08-10T19:00:00Z"
TX = "0x" + "a" * 64


def policy(*, required: list[str] | None = None) -> dict:
    return {
        "schema_version": "agent-bounties/github-bounty-discovery-policy-v1",
        "repository": REPOSITORY,
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "activation": {
            "timestamp": "2026-08-10T18:40:35Z",
            "safe_block": 49_798_944,
            "safe_block_hash": "0x" + "b" * 64,
        },
        "required_backfill_discovery_ids": required or [],
        "open_competition_compatibility_trial": {
            "starts_at": "2026-08-10T18:40:35Z",
            "ends_at": "2026-09-09T18:40:35Z",
            "labels": ["ready-to-earn", "claimable-live", "open-competition"],
            "post_trial_action": "hold_for_day_30_decision",
        },
        "publication_lag_target_minutes_p95": 10,
    }


def item(
    number: int,
    state: str = "ready_to_earn",
    *,
    mode: str = "exclusive_claim",
    source_url: str | None = None,
    created_block: int = 49_799_000,
    recovery: bool = False,
    verifier_ready: bool = True,
    difficulty: str | None = None,
) -> dict:
    contract = f"0x{number:040x}"
    protocol = (
        "agent-bounties/open-competition-v1"
        if mode == "first_valid_submission"
        else "agent-bounties/autonomous-v1"
    )
    identity = f"eip155:{CHAIN_ID}:{protocol}:{contract}"
    settlement = None
    if state == "settled":
        settlement = {
            "event_name": "BountySettled",
            "bounty_id": f"0x{number:064x}",
            "bounty_contract": contract,
            "transaction_hash": TX,
            "block_number": created_block + 5,
            "log_index": number,
            "solver_wallet": "0x" + "9" * 40,
            "solver_reward": "2000000",
            "returned_bond": "10000",
            "completion_bonus": "0",
            "solver_payout": "2010000",
            "verifier_reward": "10000",
            "confirmed_canonical": True,
        }
    funded = state != "funding_needed"
    return {
        "discovery_id": identity,
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "protocol_version": protocol,
        "source_id": contract,
        "visibility": "public",
        "bounty_id": f"0x{number:064x}",
        "bounty_contract": contract,
        "created_at": "2026-08-10T18:50:00Z" if created_block >= 49_798_944 else "2026-08-01T00:00:00Z",
        "created_block": created_block,
        "updated_at": NOW,
        "title": f"Digital work outcome {number}",
        "summary": "Deliver the exact public digital outcome described by the canonical terms.",
        "categories": ["engineering"],
        "skills": ["testing"],
        "difficulty": difficulty,
        "public_url": f"https://agentbounties.app/{'competition' if mode == 'first_valid_submission' else 'earn'}.html?bountyContract={contract}",
        "source_url": source_url,
        "competition_mode": mode,
        "lifecycle_state": state,
        "funded": funded,
        "verification_ready": verifier_ready,
        "ready_to_earn": state == "ready_to_earn",
        "reward_usdc_base_units": "2000000",
        "verifier_reward_usdc_base_units": "10000",
        "bond_usdc_base_units": "10000",
        "funded_usdc_base_units": "2010000" if funded else "0",
        "funding_target_usdc_base_units": "2010000",
        "deadline": "2026-08-11T18:50:00Z",
        "deadline_kind": "competition_deadline" if mode == "first_valid_submission" else "funding_deadline",
        "entry_count": 1 if mode == "first_valid_submission" else None,
        "max_entries": 4 if mode == "first_valid_submission" else None,
        "verifier": {
            "profile_id": "leading-zero-v1" if mode == "first_valid_submission" else None,
            "display_name": "Leading-zero deterministic verifier",
            "method": "deterministic",
            "address": "0x" + "8" * 40,
            "runtime_code_hash": "0x" + "7" * 64,
            "ready": verifier_ready,
        },
        "next_action": {
            "kind": "enter_competition" if mode == "first_valid_submission" else "claim",
            "label": "Enter competition" if mode == "first_valid_submission" else "Claim this bounty",
            "method": "POST",
            "url": "https://api.agentbounties.app/v1/base/open-competition-v1/commit-preparation"
            if mode == "first_valid_submission"
            else "https://api.agentbounties.app/v1/base/autonomous-bounties/claim-plan",
            "instructions": "Prepare the exact canonical action.",
        },
        "recovery_action_available": recovery,
        "identity_warning": "One wallet does not prove one independent person."
        if mode == "first_valid_submission"
        else None,
        "settlement_evidence": settlement,
        "evidence_boundary": "GitHub is not settlement evidence.",
    }


def projection(*items: dict, degraded: bool = False) -> dict:
    autonomous_count = sum(
        record["protocol_version"] == "agent-bounties/autonomous-v1" for record in items
    )
    competition_count = sum(
        record["protocol_version"] == "agent-bounties/open-competition-v1" for record in items
    )
    return {
        "schema_version": "agent-bounties/github-bounty-discovery-v1",
        "generated_at": NOW,
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "safe_block": {
            "number": 49_799_500,
            "hash": "0x" + "c" * 64,
            "timestamp": 1_786_388_400,
            "age_seconds": 2,
            "fresh": not degraded,
        },
        "degraded": degraded,
        "source_statuses": [
            {
                "source_type": "canonical_autonomous",
                "protocol_version": "agent-bounties/autonomous-v1",
                "factory_contract": "0x" + "1" * 40,
                "available": not degraded,
                "fresh": not degraded,
                "item_count": autonomous_count,
                "persisted_cursor_block": 49_799_500,
                "error": None,
            },
            {
                "source_type": "open_competition",
                "protocol_version": "agent-bounties/open-competition-v1",
                "factory_contract": "0x" + "2" * 40,
                "available": not degraded,
                "fresh": not degraded,
                "item_count": competition_count,
                "persisted_cursor_block": 49_799_500,
                "error": None,
            },
        ],
        "items": list(items),
        "evidence_boundary": "Read only.",
    }


def issue(number: int, *, body: str = "Human-authored context.", labels: list[str] | None = None, state: str = "open") -> dict:
    return {
        "number": number,
        "html_url": f"https://github.com/{REPOSITORY}/issues/{number}",
        "created_at": "2026-08-10T18:55:00Z",
        "title": f"Existing issue {number}",
        "body": body,
        "labels": [{"name": label} for label in (labels or ["bounty"])],
        "state": state,
        "state_reason": None,
    }


class FakeGitHub:
    def __init__(self, issues: list[dict], comments: dict[int, list[dict]] | None = None) -> None:
        self.issues = {record["number"]: record for record in issues}
        self.comments = comments or {}
        self.calls: list[tuple[str, str, object]] = []
        self.labels = set(LABEL_DEFINITIONS)
        self.next_issue = max(self.issues, default=0) + 1
        self.next_comment = 1000

    def __call__(self, method, url, body, headers):
        self.calls.append((method, url, body))
        if method == "GET" and "/labels?" in url:
            return HttpResult(200, [{"name": name} for name in sorted(self.labels)], {})
        if method == "POST" and url.endswith("/labels"):
            self.labels.add(body["name"])
            return HttpResult(201, body, {})
        if method == "POST" and url.endswith("/issues"):
            number = self.next_issue
            self.next_issue += 1
            created = issue(number, body=body["body"], labels=body["labels"])
            created["title"] = body["title"]
            self.issues[number] = created
            return HttpResult(201, created, {})
        match = __import__("re").search(r"/issues/([0-9]+)$", url)
        if match and method == "GET":
            return HttpResult(200, self.issues[int(match.group(1))], {})
        if match and method == "PATCH":
            record = self.issues[int(match.group(1))]
            if "body" in body:
                record["body"] = body["body"]
            if "labels" in body:
                record["labels"] = [{"name": label} for label in body["labels"]]
            if "state" in body:
                record["state"] = body["state"]
                record["state_reason"] = body.get("state_reason")
            return HttpResult(200, record, {})
        comment_match = __import__("re").search(r"/issues/([0-9]+)/comments$", url)
        if comment_match and method == "POST":
            number = int(comment_match.group(1))
            comment = {
                "id": self.next_comment,
                "body": body["body"],
                "user": {"login": "github-actions[bot]"},
            }
            self.next_comment += 1
            self.comments.setdefault(number, []).append(comment)
            return HttpResult(201, comment, {})
        edit_match = __import__("re").search(r"/issues/comments/([0-9]+)$", url)
        if edit_match and method == "PATCH":
            comment_id = int(edit_match.group(1))
            for records in self.comments.values():
                for comment in records:
                    if comment["id"] == comment_id:
                        comment["body"] = body["body"]
                        return HttpResult(200, comment, {})
        raise AssertionError(f"unexpected request: {method} {url}")


class GitHubDiscoveryReconciliationTests(unittest.TestCase):
    def test_workflow_is_least_privilege_concurrent_dry_run_by_default(self) -> None:
        workflow = Path(".github/workflows/bounty-inventory-guard.yml").read_text(encoding="utf-8")
        self.assertIn("issues: read", workflow)
        self.assertEqual(workflow.count("issues: write"), 1)
        self.assertIn("group: canonical-github-bounty-reconciliation", workflow)
        self.assertIn("cancel-in-progress: false", workflow)
        self.assertIn("default: false", workflow)
        self.assertIn("vars.GITHUB_BOUNTY_DISCOVERY_EXECUTE == 'true'", workflow)
        self.assertIn("bounty-label-reconciliation.log", workflow)
        self.assertIn("if: always()", workflow)

    def test_open_competition_creation_uses_all_trial_labels_and_enter_copy(self) -> None:
        record = item(1, mode="first_valid_submission")
        plan = build_plans(projection(record), [], policy(), REPOSITORY)[0]
        self.assertIsNone(plan.issue_number)
        self.assertTrue(plan.create_eligible)
        self.assertEqual(plan.mapping_action, "current_nonterminal_backfill")
        self.assertTrue(
            {"ready-to-earn", "claimable-live", "open-competition", "verifier"}.issubset(
                plan.desired_managed_labels
            )
        )
        self.assertIn("Enter competition", plan.desired_body)
        self.assertIn("First valid confirmed reveal wins", plan.desired_body)
        self.assertIn("discovery_id=", plan.desired_body)
        self.assertNotIn("competition", plan.desired_managed_labels)

    def test_github_claim_command_recovers_to_open_competition(self) -> None:
        contract = "0x" + "3" * 40
        recovery = open_competition_wrong_mode_plan(
            {
                "url": f"https://github.com/{REPOSITORY}/issues/88",
                "issue_body": f"https://agentbounties.app/competition.html?bountyContract={contract}",
            }
        )
        self.assertFalse(recovery["ready"])
        self.assertEqual(recovery["signal"]["error_code"], "wrong_competition_mode")
        self.assertEqual(recovery["signal"]["correct_action"], "enter_competition")
        self.assertIn("discovery_id=", recovery["signal"]["competition_url"])

    def test_reuses_same_repository_source_and_preserves_human_content(self) -> None:
        source = issue(42, body="Keep this human section.", labels=["bounty", "help wanted"])
        record = item(2, source_url=f"https://github.com/{REPOSITORY}/issues/42")
        plan = build_plans(projection(record), [source], policy(), REPOSITORY)[0]
        self.assertEqual(plan.issue_number, 42)
        self.assertEqual(plan.mapping_action, "reuse_source")
        self.assertTrue(plan.desired_body.startswith("Keep this human section."))
        self.assertIn(MANAGED_START, plan.desired_body)
        self.assertNotIn("help wanted", plan.remove_labels)

    def test_external_source_creates_central_mirror_and_source_collision_is_unambiguous(self) -> None:
        external = item(3, source_url="https://github.com/external/project/issues/7")
        first = item(4, source_url=f"https://github.com/{REPOSITORY}/issues/50")
        second = item(5, state="settled", source_url=f"https://github.com/{REPOSITORY}/issues/50")
        plans = build_plans(projection(external, first, second), [issue(50)], policy(), REPOSITORY)
        by_id = {plan.discovery_id: plan for plan in plans}
        self.assertEqual(by_id[first["discovery_id"]].mapping_action, "reuse_source")
        self.assertEqual(by_id[second["discovery_id"]].mapping_action, "post_activation_record")
        self.assertEqual(by_id[external["discovery_id"]].mapping_action, "current_nonterminal_backfill")

    def test_historical_terminal_is_not_created_but_existing_source_is_reconciled(self) -> None:
        historical = item(6, state="settled", created_block=49_000_000)
        plan = build_plans(projection(historical), [], policy(), REPOSITORY)[0]
        self.assertFalse(plan.create_eligible)
        self.assertEqual(plan.mapping_action, "excluded_historical_terminal")
        self.assertFalse(plan_has_write(plan))

        required = build_plans(
            projection(historical),
            [],
            policy(required=[historical["discovery_id"]]),
            REPOSITORY,
        )[0]
        self.assertTrue(required.create_eligible)
        self.assertEqual(required.mapping_action, "required_backfill")
        self.assertEqual(required.desired_state, "closed")
        self.assertEqual(required.desired_state_reason, "completed")
        self.assertEqual(required.desired_managed_labels, ["ai-agent-welcome", "bounty", "payments", "settled-paid"])

        historical["source_url"] = f"https://github.com/{REPOSITORY}/issues/60"
        reused = build_plans(projection(historical), [issue(60)], policy(), REPOSITORY)[0]
        self.assertTrue(reused.create_eligible)
        self.assertEqual(reused.mapping_action, "reuse_source")
        self.assertEqual(reused.desired_state, "closed")
        self.assertEqual(reused.desired_state_reason, "completed")

    def test_cancelled_recovery_stays_open_and_terminal_cancel_closes_not_planned(self) -> None:
        recovery = item(7, state="cancelled", recovery=True, created_block=49_000_000)
        no_recovery = item(8, state="cancelled", created_block=49_799_000)
        plans = build_plans(projection(recovery, no_recovery), [], policy(), REPOSITORY)
        by_id = {plan.discovery_id: plan for plan in plans}
        self.assertEqual(by_id[recovery["discovery_id"]].desired_state, "open")
        self.assertIn("refund-available", by_id[recovery["discovery_id"]].desired_managed_labels)
        self.assertEqual(by_id[no_recovery["discovery_id"]].desired_state, "closed")
        self.assertEqual(by_id[no_recovery["discovery_id"]].desired_state_reason, "not_planned")

    def test_degraded_duplicate_and_missing_required_projection_fail_closed(self) -> None:
        record = item(9)
        with self.assertRaisesRegex(LabelReconciliationError, "degraded"):
            build_plans(projection(record, degraded=True), [], policy(), REPOSITORY)
        with self.assertRaisesRegex(LabelReconciliationError, "malformed"):
            build_plans(projection(record, dict(record)), [], policy(), REPOSITORY)
        with self.assertRaisesRegex(LabelReconciliationError, "required backfill"):
            build_plans(projection(record), [], policy(required=[item(10)["discovery_id"]]), REPOSITORY)
        private = dict(record)
        private["discovery_id"] = item(99)["discovery_id"]
        private["bounty_contract"] = item(99)["bounty_contract"]
        private["source_id"] = item(99)["source_id"]
        private["visibility"] = "private"
        with self.assertRaisesRegex(LabelReconciliationError, "private record"):
            build_plans(projection(private), [], policy(), REPOSITORY)

    def test_duplicate_issue_markers_and_malformed_blocks_fail_closed(self) -> None:
        record = item(11)
        managed = build_plans(projection(record), [], policy(), REPOSITORY)[0].desired_body
        duplicate = [issue(1, body=managed), issue(2, body=managed)]
        with self.assertRaisesRegex(LabelReconciliationError, "duplicate discovery_id"):
            build_plans(projection(record), duplicate, policy(), REPOSITORY)
        with self.assertRaisesRegex(LabelReconciliationError, "malformed managed markers"):
            build_plans(projection(record), [issue(1, body=MANAGED_START)], policy(), REPOSITORY)

    def test_settlement_receipt_is_canonical_and_precedes_close(self) -> None:
        settled = item(12, state="settled")
        source = issue(12, labels=["bounty", "ready-to-earn"])
        settled["source_url"] = f"https://github.com/{REPOSITORY}/issues/12"
        plan = build_plans(projection(settled), [source], policy(), REPOSITORY)[0]
        self.assertEqual(plan.receipt_action, "create")
        self.assertIn("Canonical payout confirmed", plan.settlement_receipt.body)
        service = FakeGitHub([source])
        results, provisioned = execute_plans([plan], REPOSITORY, "token", service)
        self.assertEqual(provisioned, [])
        self.assertEqual(results[0]["state"], "closed")
        comment_index = next(i for i, call in enumerate(service.calls) if call[0] == "POST" and call[1].endswith("/comments"))
        close_index = next(
            i
            for i, call in enumerate(service.calls)
            if call[0] == "PATCH" and call[1].endswith("/issues/12") and call[2].get("state") == "closed"
        )
        self.assertLess(comment_index, close_index)
        self.assertEqual(service.issues[12]["state_reason"], "completed")

    def test_execution_is_idempotent_and_preserves_unmanaged_labels(self) -> None:
        record = item(13, difficulty="beginner")
        source = issue(13, labels=["bounty", "custom-human-label"])
        record["source_url"] = f"https://github.com/{REPOSITORY}/issues/13"
        first = build_plans(projection(record), [source], policy(), REPOSITORY)[0]
        service = FakeGitHub([source])
        execute_plans([first], REPOSITORY, "token", service)
        updated = service.issues[13]
        replay = build_plans(projection(record), [updated], policy(), REPOSITORY)[0]
        self.assertFalse(plan_has_write(replay))
        self.assertIn("custom-human-label", {entry["name"] for entry in updated["labels"]})
        self.assertIn("good-first-agent-bounty", {entry["name"] for entry in updated["labels"]})
        self.assertEqual(issue_marker(updated), record["discovery_id"])

    def test_pagination_follows_link_header_without_a_record_cap(self) -> None:
        calls = []

        def request(method, url, body, headers):
            calls.append(url)
            if "page=2" in url:
                return HttpResult(200, [issue(101)], {})
            return HttpResult(
                200,
                [issue(number) for number in range(1, 101)],
                {"Link": f'<https://api.github.com/repos/{REPOSITORY}/issues?labels=bounty&per_page=100&page=2>; rel="next"'},
            )

        records = fetch_github_issues(request, REPOSITORY, None)
        self.assertEqual(len(records), 101)
        self.assertEqual(len(calls), 2)

    def test_rate_limit_and_server_failures_retry_then_converge(self) -> None:
        statuses = [429, 503, 200]
        sleeps = []

        def request(method, url, body, headers):
            status = statuses.pop(0)
            return HttpResult(status, {}, {"Retry-After": "0"})

        result = request_with_retry(request, "GET", "https://api.github.com/test", sleep=sleeps.append)
        self.assertEqual(result.status, 200)
        self.assertEqual(sleeps, [0.0, 0.0])

    def test_fixture_main_is_dry_run_and_execute_is_refused(self) -> None:
        record = item(14)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = root / "fixture.json"
            policy_path = root / "policy.json"
            report = root / "report.json"
            fixture.write_text(json.dumps({"projection": projection(record), "issues": []}), encoding="utf-8")
            policy_path.write_text(json.dumps(policy()), encoding="utf-8")
            self.assertEqual(
                main(["--fixture", str(fixture), "--policy", str(policy_path), "--json-out", str(report)]),
                0,
            )
            payload = json.loads(report.read_text(encoding="utf-8"))
            self.assertEqual(payload["mode"], "dry-run")
            self.assertEqual(payload["coverage_percent"], 100.0)
            with self.assertRaisesRegex(LabelReconciliationError, "fixture mode"):
                with patch.dict(os.environ, {"GITHUB_TOKEN": "token"}):
                    main(
                        [
                            "--fixture",
                            str(fixture),
                            "--policy",
                            str(policy_path),
                            "--execute",
                            "--confirm-repository",
                            REPOSITORY,
                        ]
                    )


if __name__ == "__main__":
    unittest.main()
