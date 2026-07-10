#!/usr/bin/env python3

from __future__ import annotations

import unittest
from urllib.parse import urlsplit

from sync_hosted_bounty_inventory import (
    HttpResult,
    InventorySyncError,
    execute_entries,
    main,
    normalize_api_base_url,
    plan_issue,
    public_report,
    server_preflight,
    validate_source_issue,
)


API_BASE_URL = "https://api.agentbounties.example"
BOUNTY_ID = "e23cda07-f8c9-30eb-984c-4e56aa10e38e"
ISSUE_BODY = """### Goal
Rebuild one hosted bounty record.

### Acceptance criteria
The stable record is available after restart.

### Template
small-code-change

### Suggested amount
5 USDC

### Funding mode
BaseUsdcEscrow

### Privacy
Public
"""


def issue_fixture() -> dict:
    return {
        "number": 115,
        "state": "open",
        "title": "[bounty]: Rebuild hosted inventory",
        "body": ISSUE_BODY,
        "html_url": "https://github.com/NSPG13/agent-bounties/issues/115",
        "updated_at": "2026-07-10T00:00:00Z",
        "labels": [{"name": "bounty"}],
    }


class FakeHostedService:
    def __init__(self, *, existing: bool = False, current_llms: bool = True) -> None:
        self.existing = existing
        self.current_llms = current_llms
        self.writes: list[dict] = []
        self.authorization_headers: list[str | None] = []
        self.status_override: int | None = None

    def __call__(self, method, url, body, headers):
        parsed = urlsplit(url)
        path = parsed.path
        if path == "/repos/NSPG13/agent-bounties/issues/115":
            return HttpResult(200, issue_fixture(), {})
        if path == "/health":
            return HttpResult(200, "ok", {})
        if path == "/llms.txt":
            text = "/v1/github/issue-api-sync"
            if self.current_llms:
                text += "\nmore and higher-value funded bounties"
            return HttpResult(200, text, {})
        if path == "/v1/github/issue-api-sync-plan":
            if body.get("hosted_api_error"):
                return HttpResult(
                    200,
                    {
                        "ready": False,
                        "operation": "HostedApiUnavailable",
                        "bounty_id": BOUNTY_ID,
                        "calls": [],
                        "error": body["hosted_api_error"],
                    },
                    {},
                )
            operation = "Update" if BOUNTY_ID in body.get("existing_bounty_ids", []) else "Create"
            return HttpResult(200, self._plan(operation, body), {})
        if path == f"/v1/bounties/{BOUNTY_ID}":
            if self.status_override is not None:
                return HttpResult(self.status_override, "unavailable", {})
            if not self.existing:
                return HttpResult(404, "not found", {})
            return HttpResult(
                200,
                {
                    "bounty": {
                        "id": BOUNTY_ID,
                        "status": "Unfunded",
                        "title": issue_fixture()["title"],
                    }
                },
                {},
            )
        if path == "/v1/github/issue-api-sync" and method == "POST":
            authorization = (headers or {}).get("Authorization")
            self.authorization_headers.append(authorization)
            if authorization != "Bearer operator-secret":
                return HttpResult(401, "unauthorized", {})
            self.writes.append(dict(body))
            self.existing = True
            return HttpResult(
                200,
                {
                    "id": BOUNTY_ID,
                    "status": "Unfunded",
                    "title": body["title"],
                },
                {},
            )
        raise AssertionError(f"unexpected request: {method} {url}")

    @staticmethod
    def _plan(operation: str, body: dict) -> dict:
        return {
            "ready": True,
            "operation": operation,
            "bounty_id": BOUNTY_ID,
            "idempotency_key": f"github-issue-sync:NSPG13/agent-bounties:{BOUNTY_ID}",
            "status_url": f"{API_BASE_URL}/v1/bounties/{BOUNTY_ID}",
            "public_bounty_url": f"{API_BASE_URL}/public/bounties/{BOUNTY_ID}",
            "funding_page_url": f"{API_BASE_URL}/public/funding?bountyId={BOUNTY_ID}",
            "error": None,
            "calls": [
                {
                    "method": "POST",
                    "url": f"{API_BASE_URL}/v1/github/issue-api-sync",
                    "body": body,
                    "idempotency_key": f"github-issue-sync:NSPG13/agent-bounties:{BOUNTY_ID}",
                    "settlement_authority": False,
                }
            ],
        }


class HostedInventorySyncTests(unittest.TestCase):
    def test_api_base_url_rejects_remote_http_and_credentials(self) -> None:
        self.assertEqual(normalize_api_base_url("http://127.0.0.1:8080/"), "http://127.0.0.1:8080")
        with self.assertRaises(InventorySyncError):
            normalize_api_base_url("http://api.example.com")
        with self.assertRaises(InventorySyncError):
            normalize_api_base_url("https://user:secret@api.example.com")

    def test_source_issue_must_be_open_canonical_bounty(self) -> None:
        validate_source_issue(issue_fixture(), "NSPG13/agent-bounties")
        closed = issue_fixture()
        closed["state"] = "closed"
        with self.assertRaisesRegex(InventorySyncError, "not open"):
            validate_source_issue(closed, "NSPG13/agent-bounties")
        unlabeled = issue_fixture()
        unlabeled["labels"] = []
        with self.assertRaisesRegex(InventorySyncError, "bounty label"):
            validate_source_issue(unlabeled, "NSPG13/agent-bounties")

    def test_absent_issue_plans_create_without_writing(self) -> None:
        service = FakeHostedService()
        entry = plan_issue(issue_fixture(), "NSPG13/agent-bounties", API_BASE_URL, service)
        self.assertTrue(entry["ready"])
        self.assertEqual(entry["operation"], "Create")
        self.assertEqual(entry["status_probe"]["state"], "absent")
        self.assertEqual(service.writes, [])

    def test_existing_issue_plans_stable_update(self) -> None:
        service = FakeHostedService(existing=True)
        entry = plan_issue(issue_fixture(), "NSPG13/agent-bounties", API_BASE_URL, service)
        self.assertEqual(entry["operation"], "Update")
        self.assertEqual(entry["bounty_id"], BOUNTY_ID)
        self.assertEqual(entry["status_probe"]["state"], "existing")

    def test_local_planner_can_generate_production_bound_plan(self) -> None:
        service = FakeHostedService()
        entry = plan_issue(
            issue_fixture(),
            "NSPG13/agent-bounties",
            API_BASE_URL,
            service,
            planner_base_url="http://127.0.0.1:18080",
        )
        self.assertEqual(entry["operation"], "Create")
        self.assertEqual(
            entry["status_url"], f"{API_BASE_URL}/v1/bounties/{BOUNTY_ID}"
        )

    def test_non_404_status_failure_blocks_plan(self) -> None:
        service = FakeHostedService()
        service.status_override = 503
        entry = plan_issue(issue_fixture(), "NSPG13/agent-bounties", API_BASE_URL, service)
        self.assertFalse(entry["ready"])
        self.assertEqual(entry["operation"], "HostedApiUnavailable")
        self.assertIn("returned 503", entry["error"])

    def test_preflight_requires_current_growth_contract(self) -> None:
        stale = server_preflight(FakeHostedService(current_llms=False), API_BASE_URL)
        self.assertFalse(stale["ready_for_execute"])
        current = server_preflight(FakeHostedService(current_llms=True), API_BASE_URL)
        self.assertTrue(current["ready_for_execute"])

    def test_execute_writes_metadata_then_verifies_without_leaking_token(self) -> None:
        service = FakeHostedService()
        entry = plan_issue(issue_fixture(), "NSPG13/agent-bounties", API_BASE_URL, service)
        report = {"entries": [entry]}
        self.assertTrue(execute_entries(report, API_BASE_URL, "operator-secret", service))
        self.assertEqual(entry["execution"]["status"], "metadata-synced")
        self.assertEqual(entry["execution"]["bounty_status"], "Unfunded")
        self.assertEqual(len(service.writes), 1)
        self.assertEqual(service.authorization_headers, ["Bearer operator-secret"])
        rendered = str(public_report(report))
        self.assertNotIn("operator-secret", rendered)
        self.assertNotIn("_execution_payload", rendered)

    def test_execute_rejects_fixtures_and_separate_planner(self) -> None:
        service = FakeHostedService()
        common = [
            "--api-base-url",
            API_BASE_URL,
            "--execute",
            "--confirm-api-base-url",
            API_BASE_URL,
            "--confirm-issue-count",
            "1",
        ]
        with self.assertRaisesRegex(InventorySyncError, "cannot use an issue fixture"):
            main([*common, "--fixture", "not-read.json"], request=service)
        with self.assertRaisesRegex(InventorySyncError, "planner and target"):
            main(
                [
                    *common,
                    "--issue",
                    "115",
                    "--planner-base-url",
                    "http://127.0.0.1:18180",
                ],
                request=service,
            )

    def test_execute_requires_exact_live_issue_count_confirmation(self) -> None:
        service = FakeHostedService()
        with self.assertRaisesRegex(InventorySyncError, "confirm-issue-count"):
            main(
                [
                    "--api-base-url",
                    API_BASE_URL,
                    "--issue",
                    "115",
                    "--execute",
                    "--confirm-api-base-url",
                    API_BASE_URL,
                    "--confirm-issue-count",
                    "2",
                ],
                request=service,
            )


if __name__ == "__main__":
    unittest.main()
