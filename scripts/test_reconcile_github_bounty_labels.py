#!/usr/bin/env python3

from __future__ import annotations

import unittest

from reconcile_github_bounty_labels import (
    HttpResult,
    LabelReconciliationError,
    build_plans,
    execute_plans,
    main,
    normalize_api_base_url,
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
        "html_url": f"https://github.com/{REPOSITORY}/issues/{number}",
        "labels": [{"name": label} for label in labels],
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
            }
        )
    return {
        "bounty_contract": contract,
        "status": status,
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
