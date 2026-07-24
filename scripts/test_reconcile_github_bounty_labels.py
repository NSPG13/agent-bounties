#!/usr/bin/env python3

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from reconcile_github_bounty_labels import (
    HttpResult,
    LabelReconciliationError,
    SETTLEMENT_RECEIPT_MARKER,
    build_plans,
    execute_plans,
    main,
    normalize_api_base_url,
    plan_receipt_actions,
)


REPOSITORY = "NSPG13/agent-bounties"
CONTRACTS = {
    1: "0x1111111111111111111111111111111111111111",
    2: "0x2222222222222222222222222222222222222222",
    3: "0x3333333333333333333333333333333333333333",
    4: "0x4444444444444444444444444444444444444444",
    5: "0x5555555555555555555555555555555555555555",
}
TX = "0x" + "a" * 64


def issue(number: int, *labels: str) -> dict:
    return {
        "number": number,
        "state": "open",
        "state_reason": None,
        "html_url": f"https://github.com/{REPOSITORY}/issues/{number}",
        "labels": [{"name": label} for label in labels],
        "comments": [],
    }


def feed_item(
    number: int,
    status: str,
    *,
    ready: bool = True,
    terms_valid: bool = True,
) -> dict:
    contract = CONTRACTS[number]
    events: list[dict] = []
    if status in {"claimed", "submitted"}:
        events.append(
            {
                "kind": "bounty_claimed",
                "contract_address": contract,
                "tx_hash": TX,
            }
        )
    if status == "submitted":
        events.append(
            {
                "kind": "submission_added",
                "contract_address": contract,
                "tx_hash": TX,
            }
        )
    if status == "paid":
        events.append(
            {
                "kind": "bounty_settled",
                "contract_address": contract,
                "tx_hash": TX,
                "bounty_id": f"0x{number:064x}",
                "log_index": number,
                "data": {
                    "solver": "0x9999999999999999999999999999999999999999",
                    "solver_reward": 2_000_000,
                    "claim_bond_returned": 10_000,
                    "timeout_bond_bonus": 0,
                    "solver_payout": 2_010_000,
                    "verifier_reward": 10_000,
                },
            }
        )
    return {
        "bounty_id": f"0x{number:064x}",
        "bounty_contract": contract,
        "status": status,
        "solver_reward": "2000000",
        "verifier_reward": "10000",
        "claim_bond": "10000",
        "timeout_bond_pool": "0",
        "target_amount": "2010000",
        "funded_amount": "0" if status == "open" else "2010000",
        "terms_valid": terms_valid,
        "verification_ready": ready,
        "terms": {
            "document": {
                "source_url": f"https://github.com/{REPOSITORY}/issues/{number}"
            }
        },
        "events": events,
    }


class FakeGitHub:
    def __init__(self, source_issue: dict) -> None:
        self.issue = source_issue
        self.calls: list[tuple[str, str, object]] = []
        self.next_comment_id = 101

    def __call__(self, method, url, body, headers):
        self.calls.append((method, url, body))
        if method == "DELETE" and "/labels/" in url:
            label = url.rsplit("/", 1)[-1]
            self.issue["labels"] = [
                item for item in self.issue["labels"] if item["name"] != label
            ]
            return HttpResult(204, "", {})
        if method == "POST" and url.endswith("/labels"):
            existing = {item["name"] for item in self.issue["labels"]}
            for label in body["labels"]:
                if label not in existing:
                    self.issue["labels"].append({"name": label})
            return HttpResult(200, self.issue, {})
        if method == "GET" and "/issues/1/comments?" in url:
            return HttpResult(200, self.issue["comments"], {})
        if method == "POST" and url.endswith("/issues/1/comments"):
            comment = {
                "id": self.next_comment_id,
                "body": body["body"],
                "user": {"login": "github-actions[bot]"},
            }
            self.next_comment_id += 1
            self.issue["comments"].append(comment)
            return HttpResult(201, comment, {})
        if method == "PATCH" and "/issues/comments/" in url:
            comment_id = int(url.rsplit("/", 1)[-1])
            comment = next(
                item for item in self.issue["comments"] if item["id"] == comment_id
            )
            comment["body"] = body["body"]
            return HttpResult(200, comment, {})
        if method == "PATCH" and url.endswith("/issues/1"):
            self.issue["state"] = body["state"]
            self.issue["state_reason"] = body["state_reason"]
            return HttpResult(200, self.issue, {})
        if method == "GET" and url.endswith("/issues/1"):
            return HttpResult(200, self.issue, {})
        raise AssertionError(f"unexpected request: {method} {url}")


class GitHubBountyLabelReconciliationTests(unittest.TestCase):
    def test_api_base_url_rejects_remote_http_and_credentials(self) -> None:
        self.assertEqual(
            normalize_api_base_url("http://127.0.0.1:8080/"),
            "http://127.0.0.1:8080",
        )
        with self.assertRaises(LabelReconciliationError):
            normalize_api_base_url("http://api.example.com")
        with self.assertRaises(LabelReconciliationError):
            normalize_api_base_url("https://user:secret@api.example.com")

    def test_claimable_requires_exact_earning_feed_membership(self) -> None:
        canonical = feed_item(1, "claimable")
        plans = build_plans(
            [issue(1, "bounty", "claimed-live", "verification-unavailable")],
            [canonical],
            [dict(canonical)],
            REPOSITORY,
        )
        self.assertEqual(plans[0].desired_managed_labels, ["claimable-live", "funded-live"])
        self.assertEqual(plans[0].add_labels, ["claimable-live", "funded-live"])
        self.assertEqual(
            plans[0].remove_labels, ["claimed-live", "verification-unavailable"]
        )

    def test_unready_claimable_is_funded_but_not_advertised(self) -> None:
        canonical = feed_item(1, "claimable", ready=False)
        plan = build_plans(
            [issue(1, "bounty", "claimable-live")],
            [canonical],
            [],
            REPOSITORY,
        )[0]
        self.assertEqual(
            plan.desired_managed_labels, ["funded-live", "verification-unavailable"]
        )
        self.assertEqual(plan.remove_labels, ["claimable-live"])

    def test_claimed_and_submitted_states_are_unavailable_to_new_solvers(self) -> None:
        claimed = feed_item(1, "claimed")
        submitted = feed_item(2, "submitted")
        plans = build_plans(
            [issue(1, "bounty", "claimable-live"), issue(2, "bounty")],
            [claimed, submitted],
            [],
            REPOSITORY,
        )
        for plan in plans:
            self.assertEqual(plan.desired_managed_labels, ["claimed-live", "funded-live"])
            self.assertNotIn("claimable-live", plan.desired_managed_labels)

    def test_paid_requires_settlement_evidence_and_removes_live_labels(self) -> None:
        paid = feed_item(1, "paid")
        plan = build_plans(
            [issue(1, "bounty", "funded-live", "claimable-live")],
            [paid],
            [],
            REPOSITORY,
        )[0]
        self.assertEqual(plan.desired_managed_labels, ["settled-paid"])
        self.assertEqual(plan.add_labels, ["settled-paid"])
        self.assertEqual(plan.remove_labels, ["claimable-live", "funded-live"])

        paid["events"] = []
        with self.assertRaisesRegex(LabelReconciliationError, "bounty_settled"):
            build_plans([issue(1, "bounty")], [paid], [], REPOSITORY)

    def test_paid_receipt_is_posted_before_close_and_replay_is_a_noop(self) -> None:
        paid = feed_item(1, "paid")
        source = issue(1, "bounty", "funded-live")
        plans = plan_receipt_actions(
            build_plans([source], [paid], [], REPOSITORY),
            {1: source["comments"]},
            REPOSITORY,
        )
        plan = plans[0]
        self.assertEqual(plan.receipt_action, "create")
        self.assertTrue(plan.complete_issue)
        self.assertIn("Solver reward: **2.00 USDC**", plan.settlement_receipt.body)
        self.assertIn("Returned solver bond: **0.01 USDC**", plan.settlement_receipt.body)
        self.assertIn("post your own bounty", plan.settlement_receipt.body.lower())

        service = FakeGitHub(source)
        results = execute_plans(plans, REPOSITORY, "secret", service)
        self.assertEqual(results[0]["receipt_action"], "create")
        self.assertEqual(source["state"], "closed")
        self.assertEqual(source["state_reason"], "completed")
        self.assertEqual(len(source["comments"]), 1)
        comment_write = next(
            index
            for index, call in enumerate(service.calls)
            if call[0] == "POST" and call[1].endswith("/comments")
        )
        close_write = next(
            index
            for index, call in enumerate(service.calls)
            if call[0] == "PATCH" and call[1].endswith("/issues/1")
        )
        self.assertLess(comment_write, close_write)

        replay = plan_receipt_actions(
            build_plans([source], [paid], [], REPOSITORY),
            {1: source["comments"]},
            REPOSITORY,
        )[0]
        self.assertEqual(replay.receipt_action, "none")
        self.assertFalse(replay.complete_issue)
        service.calls.clear()
        self.assertEqual(execute_plans([replay], REPOSITORY, "secret", service), [])
        self.assertEqual(service.calls, [])

    def test_stale_trusted_receipt_updates_but_external_marker_is_ignored(self) -> None:
        paid = feed_item(1, "paid")
        source = issue(1, "settled-paid")
        source["state"] = "closed"
        source["state_reason"] = "completed"
        source["comments"] = [
            {
                "id": 7,
                "body": f"{SETTLEMENT_RECEIPT_MARKER}\nstale",
                "user": {"login": "github-actions[bot]"},
            },
            {
                "id": 8,
                "body": f"{SETTLEMENT_RECEIPT_MARKER}\nspoof",
                "user": {"login": "external-user"},
            },
        ]
        plan = plan_receipt_actions(
            build_plans([source], [paid], [], REPOSITORY),
            {1: source["comments"]},
            REPOSITORY,
        )[0]
        self.assertEqual(plan.receipt_action, "update")
        self.assertEqual(plan.receipt_comment_id, 7)
        service = FakeGitHub(source)
        execute_plans([plan], REPOSITORY, "secret", service)
        self.assertEqual(source["comments"][0]["body"], plan.settlement_receipt.body)
        self.assertEqual(source["comments"][1]["body"], f"{SETTLEMENT_RECEIPT_MARKER}\nspoof")

    def test_nonterminal_and_malformed_settlement_records_cannot_close(self) -> None:
        claimed = feed_item(1, "claimed")
        source = issue(1, "bounty")
        plan = build_plans([source], [claimed], [], REPOSITORY)[0]
        self.assertIsNone(plan.settlement_receipt)
        self.assertFalse(
            plan_receipt_actions([plan], {}, REPOSITORY)[0].complete_issue
        )

        paid = feed_item(1, "paid")
        paid["events"][0]["data"]["solver_payout"] += 1
        with self.assertRaisesRegex(LabelReconciliationError, "amounts"):
            build_plans([source], [paid], [], REPOSITORY)

        paid = feed_item(1, "paid")
        paid["terms"]["document"]["source_url"] = (
            f"https://github.com/{REPOSITORY}/issues/not-a-number"
        )
        with self.assertRaisesRegex(LabelReconciliationError, "positive issue"):
            build_plans([source], [paid], [], REPOSITORY)

    def test_dry_run_reports_exact_receipt_and_closure_without_writing(self) -> None:
        source = issue(1, "funded-live")
        payload = {
            "issues": [source],
            "full_feed": [feed_item(1, "paid")],
            "claimable_feed": [],
        }
        with tempfile.TemporaryDirectory() as directory:
            fixture_path = Path(directory, "fixture.json")
            report_path = Path(directory, "report.json")
            fixture_path.write_text(json.dumps(payload), encoding="utf-8")
            self.assertEqual(
                main(
                    [
                        "--fixture",
                        str(fixture_path),
                        "--json-out",
                        str(report_path),
                    ]
                ),
                0,
            )
            report = json.loads(report_path.read_text(encoding="utf-8"))
        plan = report["plans"][0]
        self.assertEqual(report["mode"], "dry-run")
        self.assertEqual(report["drift_count"], 1)
        self.assertFalse(report["settlement_authority"])
        self.assertEqual(plan["receipt_action"], "create")
        self.assertTrue(plan["complete_issue"])
        self.assertEqual(plan["settlement_receipt"]["transaction_hash"], TX)
        self.assertEqual(source["comments"], [])

    def test_unmapped_managed_issue_fails_closed(self) -> None:
        plan = build_plans(
            [issue(1, "bounty", "funded-live", "claimable-live")],
            [],
            [],
            REPOSITORY,
        )[0]
        self.assertEqual(plan.desired_managed_labels, [])
        self.assertEqual(plan.remove_labels, ["claimable-live", "funded-live"])

    def test_duplicate_issue_mapping_and_invalid_earning_record_are_rejected(self) -> None:
        first = feed_item(1, "claimable")
        duplicate = feed_item(2, "claimable")
        duplicate["terms"]["document"]["source_url"] = first["terms"]["document"][
            "source_url"
        ]
        with self.assertRaisesRegex(LabelReconciliationError, "multiple canonical"):
            build_plans([issue(1, "bounty")], [first, duplicate], [], REPOSITORY)

        invalid_earning = dict(first)
        invalid_earning["verification_ready"] = False
        with self.assertRaisesRegex(LabelReconciliationError, "not an exact executable"):
            build_plans(
                [issue(1, "bounty")], [first], [invalid_earning], REPOSITORY
            )

    def test_execute_mutates_only_managed_labels_and_verifies_result(self) -> None:
        canonical = feed_item(1, "claimed")
        source = issue(1, "bounty", "distribution", "claimable-live")
        plan = build_plans([source], [canonical], [], REPOSITORY)[0]
        service = FakeGitHub(source)
        results = execute_plans([plan], REPOSITORY, "secret", service)
        self.assertEqual(results[0]["managed_labels"], ["claimed-live", "funded-live"])
        self.assertEqual(
            {item["name"] for item in source["labels"]},
            {"bounty", "distribution", "claimed-live", "funded-live"},
        )
        self.assertFalse(any("secret" in str(call) for call in service.calls))

    def test_execute_rejects_fixture_mode(self) -> None:
        with self.assertRaisesRegex(LabelReconciliationError, "cannot use a fixture"):
            main(
                [
                    "--fixture",
                    "unused.json",
                    "--execute",
                    "--confirm-repository",
                    REPOSITORY,
                ]
            )


if __name__ == "__main__":
    unittest.main()
