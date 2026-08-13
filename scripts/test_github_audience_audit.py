#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import unittest
from copy import deepcopy
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


SCRIPT_DIR = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "github_audience_audit", SCRIPT_DIR / "github_audience_audit.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class GitHubAudienceAuditTests(unittest.TestCase):
    def setUp(self) -> None:
        fixture = SCRIPT_DIR / "fixtures" / "github_audience_audit.json"
        self.snapshot = json.loads(fixture.read_text(encoding="utf-8"))
        self.audit = MODULE.build_audit(self.snapshot, "NSPG13")

    def test_github_api_output_is_decoded_as_utf8_on_every_platform(self) -> None:
        response = '[[{"id":1,"body":"autonomous café"}]]'
        with patch.object(
            MODULE.subprocess,
            "run",
            return_value=SimpleNamespace(stdout=response),
        ) as run:
            records = MODULE.gh_api("NSPG13/agent-bounties", "issues")

        self.assertEqual(records[0]["body"], "autonomous café")
        self.assertEqual(run.call_args.kwargs["encoding"], "utf-8")
        self.assertEqual(run.call_args.kwargs["errors"], "strict")

    def test_public_metrics_are_namespaced_aggregate_only_and_use_exact_boundaries(self) -> None:
        def user(login: str, *, user_type: str = "User") -> dict:
            return {"id": login, "login": login, "type": user_type}

        snapshot = {
            "repository": "NSPG13/agent-bounties",
            "fetched_at": "2026-08-12T20:22:19Z",
            "issues": [
                {"id": 1, "created_at": "2026-07-08T20:22:18Z", "user": user("before")},
                {"id": 2, "created_at": "2026-07-08T20:22:19Z", "user": user("alice")},
                {"id": 3, "created_at": "2026-08-01T00:00:00Z", "user": user("charlie")},
                {
                    "id": 4,
                    "created_at": "2026-08-06T00:00:00Z",
                    "pull_request": {},
                    "user": user("bob"),
                },
                {"id": 5, "created_at": "2026-08-08T20:22:19Z", "user": user("diana")},
                {"id": 6, "created_at": "2026-08-09T00:00:00Z", "user": user("NSPG13")},
                {"id": 7, "created_at": "2026-08-09T00:00:00Z", "user": user("ci[bot]", user_type="Bot")},
                {"id": 8, "created_at": "2026-08-09T00:00:00Z", "user": user("maintainer-two")},
            ],
            "issue_comments": [
                {"id": 10, "created_at": "2026-08-07T00:00:00Z", "user": user("alice")},
                {"id": 10, "created_at": "2026-08-07T00:00:00Z", "user": user("alice")},
                {"id": 11, "created_at": None, "user": user("missing-time")},
            ],
            "review_comments": [
                {"id": 12, "created_at": "2026-08-07T01:00:00Z", "user": user("bob")}
            ],
            "reviews": [
                {"id": 13, "submitted_at": "2026-08-07T02:00:00Z", "user": user("bob")}
            ],
        }

        metrics = MODULE.build_public_participation_metrics(
            snapshot,
            "NSPG13",
            excluded_logins={"maintainer-two"},
        )
        serialized = json.dumps(metrics).lower()

        self.assertEqual(metrics["namespace"], "github")
        self.assertNotIn("repository", metrics)
        self.assertEqual(metrics["periods"]["7d"]["active_identities"], 3)
        self.assertEqual(metrics["periods"]["7d"]["previous_active_identities"], 1)
        self.assertEqual(metrics["periods"]["7d"]["qualifying_actions"], 5)
        self.assertEqual(metrics["first_month"]["active_identities"], 3)
        self.assertEqual(metrics["periods"]["lifetime"]["active_identities"], 4)
        self.assertEqual(metrics["coverage"]["status"], "partial")
        self.assertEqual(metrics["coverage"]["missing_timestamp_records"], 1)
        self.assertFalse(metrics["coverage"]["raw_identifiers_included"])
        for forbidden in (
            "alice",
            "bob",
            "charlie",
            "diana",
            "maintainer-two",
            "nspg13",
            "missing-time",
            "github.com/",
        ):
            self.assertNotIn(forbidden, serialized)

    def test_public_metrics_collection_skips_reactions_stars_and_old_pr_reviews(self) -> None:
        pulls = [
            {"number": 1, "updated_at": "2026-07-07T00:00:00Z"},
            {"number": 2, "updated_at": "2026-07-09T00:00:00Z"},
        ]

        def fake_api(repository: str, suffix: str, *, accept: str | None = None) -> list[dict]:
            self.assertEqual(repository, "NSPG13/agent-bounties")
            self.assertIsNone(accept)
            if suffix.startswith("issues?"):
                return []
            if suffix.startswith("pulls?state"):
                return pulls
            if suffix.startswith("issues/comments") or suffix.startswith("pulls/comments"):
                return []
            if suffix.startswith("pulls/2/reviews"):
                return []
            self.fail(f"unexpected public-metrics API call: {suffix}")

        with patch.object(MODULE, "gh_api", side_effect=fake_api):
            snapshot = MODULE.collect_snapshot(
                "NSPG13/agent-bounties",
                include_enrichment=False,
                activity_since=MODULE.PLATFORM_LAUNCH_AT,
            )

        self.assertEqual(snapshot["reactions"], [])
        self.assertEqual(snapshot["stargazers"], [])

    def test_repository_traffic_is_aggregate_only_and_preserves_window_scope(self) -> None:
        snapshot = {
            "fetched_at": "2026-08-12T20:22:19Z",
            "repository_traffic": {
                "status": "ready",
                "clones": {
                    "count": 9654,
                    "uniques": 446,
                    "clones": [
                        {"timestamp": "2026-07-30T00:00:00Z", "count": 4000, "uniques": 250},
                        {"timestamp": "2026-08-12T00:00:00Z", "count": 5654, "uniques": 300},
                    ],
                },
                "views": {
                    "count": 883,
                    "uniques": 218,
                    "views": [
                        {"timestamp": "2026-07-30T00:00:00Z", "count": 300, "uniques": 100},
                        {"timestamp": "2026-08-12T00:00:00Z", "count": 583, "uniques": 140},
                    ],
                },
            },
        }

        acquisition = MODULE.build_public_repository_acquisition(
            snapshot, MODULE.parse_utc_timestamp(snapshot["fetched_at"])
        )
        serialized = json.dumps(acquisition).lower()

        self.assertEqual(acquisition["coverage"]["status"], "ready")
        self.assertEqual(acquisition["clone_events"], 9654)
        self.assertEqual(acquisition["unique_cloners"], 446)
        self.assertEqual(acquisition["page_views"], 883)
        self.assertEqual(acquisition["unique_visitors"], 218)
        self.assertEqual(acquisition["started_at"], "2026-07-30T00:00:00Z")
        self.assertEqual(acquisition["ended_at"], "2026-08-13T00:00:00Z")
        self.assertFalse(acquisition["coverage"]["unique_audiences_are_additive"])
        self.assertNotIn("login", serialized)
        self.assertNotIn("github.com/", serialized)

    def test_repository_traffic_fails_closed_when_counts_are_invalid(self) -> None:
        generated_at = MODULE.parse_utc_timestamp("2026-08-12T20:22:19Z")
        acquisition = MODULE.build_public_repository_acquisition(
            {
                "repository_traffic": {
                    "status": "ready",
                    "clones": {"count": 2, "uniques": 3, "clones": []},
                    "views": {"count": 1, "uniques": 1, "views": []},
                }
            },
            generated_at,
        )
        self.assertEqual(acquisition["coverage"]["status"], "unavailable")
        self.assertNotIn("clone_events", acquisition)

    def test_repository_traffic_collection_uses_only_aggregate_endpoints(self) -> None:
        payloads = {
            "traffic/clones?per=day": {"count": 9, "uniques": 4, "clones": []},
            "traffic/views?per=day": {"count": 8, "uniques": 3, "views": []},
        }
        with patch.object(
            MODULE, "gh_api_json", side_effect=lambda _repo, suffix: payloads[suffix]
        ) as api:
            traffic = MODULE.collect_repository_traffic("NSPG13/agent-bounties")

        self.assertEqual(traffic["status"], "ready")
        self.assertEqual(api.call_count, 2)

    def test_public_activity_is_deduplicated_and_bots_are_excluded(self) -> None:
        handles = [participant["handle"] for participant in self.audit["participants"]]
        self.assertEqual(handles, ["alice-agent", "bob-human"])
        self.assertNotIn("NSPG13", handles)
        self.assertNotIn("claim-bot[bot]", handles)

        kinds = [interaction["kind"] for interaction in self.audit["interactions"]]
        self.assertIn("bounty_posted", kinds)
        self.assertIn("funding_signaled", kinds)
        self.assertIn("repo_starred", kinds)
        self.assertIn("bounty_upvoted", kinds)
        self.assertNotIn("bounty_funded", kinds)

    def test_volunteered_and_requested_answers_close_coverage_without_inference(self) -> None:
        coverage = self.audit["coverage"]
        self.assertEqual(coverage["participant_count"], 2)
        self.assertEqual(coverage["asked_count"], 1)
        self.assertEqual(coverage["answer_candidate_count"], 2)
        self.assertEqual(coverage["not_asked_or_answered_handles"], [])
        self.assertEqual(coverage["asked_without_answer_handles"], [])
        self.assertTrue(
            all(
                candidate["curation_required"]
                for candidate in self.audit["discovery_answer_candidates"]
            )
        )
        self.assertTrue(self.audit["privacy_boundary"]["raw_comment_text_stored"] is False)

    def test_sync_writes_members_events_and_outreach_but_not_unstructured_answers(self) -> None:
        calls: list[tuple[str, dict]] = []

        def fake_post(base_url: str, path: str, payload: dict, token: str | None) -> dict:
            self.assertEqual(base_url, "https://api.example")
            self.assertEqual(token, "operator-token")
            calls.append((path, payload))
            if path == "/v1/audience/members":
                return {"id": f"member:{payload['handle'].lower()}"}
            return {"id": "stored"}

        result = MODULE.sync_audit(
            self.audit,
            "https://api.example",
            "operator-token",
            post_json=fake_post,
        )

        self.assertEqual(result["members_synced"], 2)
        self.assertEqual(
            result["discovery_candidates_requiring_curation"],
            len(self.audit["discovery_answer_candidates"]),
        )
        paths = [path for path, _ in calls]
        self.assertEqual(paths.count("/v1/audience/members"), 2)
        self.assertEqual(
            paths.count("/v1/audience/interactions"), len(self.audit["interactions"])
        )
        self.assertEqual(
            paths.count("/v1/audience/outreach-attempts"),
            len(self.audit["outreach_attempts"]),
        )
        self.assertNotIn("/v1/audience/discovery-responses", paths)

    def test_maintainer_prompt_targets_latest_external_commenter_on_owner_issue(self) -> None:
        snapshot = deepcopy(self.snapshot)
        owner = {
            "id": 1,
            "login": "NSPG13",
            "type": "User",
            "html_url": "https://github.com/NSPG13",
        }
        charlie = {
            "id": 5,
            "login": "charlie-agent",
            "type": "User",
            "html_url": "https://github.com/charlie-agent",
        }
        snapshot["issues"].append(
            {
                "id": 103,
                "number": 12,
                "html_url": "https://github.com/NSPG13/agent-bounties/issues/12",
                "created_at": "2026-07-06T10:00:00Z",
                "body": "Maintainer-posted bounty.",
                "labels": [{"name": "bounty"}],
                "user": owner,
            }
        )
        snapshot["issue_comments"].extend(
            [
                {
                    "id": 205,
                    "html_url": "https://github.com/NSPG13/agent-bounties/issues/12#issuecomment-205",
                    "issue_url": "https://api.github.com/repos/NSPG13/agent-bounties/issues/12",
                    "created_at": "2026-07-06T11:00:00Z",
                    "body": "I can take this.",
                    "user": charlie,
                },
                {
                    "id": 206,
                    "html_url": "https://github.com/NSPG13/agent-bounties/issues/12#issuecomment-206",
                    "issue_url": "https://api.github.com/repos/NSPG13/agent-bounties/issues/12",
                    "created_at": "2026-07-06T12:00:00Z",
                    "body": "How did you find Agent Bounties? What made this bounty or project worth participating in?",
                    "user": owner,
                },
            ]
        )

        audit = MODULE.build_audit(snapshot, "NSPG13")
        attempts = [
            attempt
            for attempt in audit["outreach_attempts"]
            if attempt["provider_event_id"].startswith("github:discovery-prompt:206")
        ]
        self.assertEqual([attempt["handle"] for attempt in attempts], ["charlie-agent"])

    def test_one_time_feedback_prompt_counts_each_mentioned_participant(self) -> None:
        snapshot = deepcopy(self.snapshot)
        owner = {
            "id": 1,
            "login": "NSPG13",
            "type": "User",
            "html_url": "https://github.com/NSPG13",
        }
        snapshot["issue_comments"].append(
            {
                "id": 207,
                "html_url": "https://github.com/NSPG13/agent-bounties/issues/12#issuecomment-207",
                "issue_url": "https://api.github.com/repos/NSPG13/agent-bounties/issues/12",
                "created_at": "2026-07-06T13:00:00Z",
                "body": (
                    "@alice-agent @bob-human one-time distribution feedback request: "
                    "Exactly how did you first find the project, why did you join, and "
                    "what prevented you from posting or funding a bounty?"
                ),
                "user": owner,
            }
        )

        audit = MODULE.build_audit(snapshot, "NSPG13")
        attempts = [
            attempt
            for attempt in audit["outreach_attempts"]
            if attempt["provider_event_id"].startswith("github:discovery-prompt:207")
        ]
        self.assertEqual(
            [attempt["handle"] for attempt in attempts], ["alice-agent", "bob-human"]
        )

    def test_explicit_curated_public_answer_can_be_synced(self) -> None:
        calls: list[tuple[str, dict]] = []

        def fake_post(base_url: str, path: str, payload: dict, token: str | None) -> dict:
            calls.append((path, payload))
            if path == "/v1/audience/members":
                return {"id": f"member:{payload['handle'].lower()}"}
            return {"id": "stored"}

        result = MODULE.sync_audit(
            self.audit,
            "https://api.example",
            None,
            curated_responses=[
                {
                    "handle": "alice-agent",
                    "provider_response_id": "github:issue-body:101",
                    "public_source_url": "https://github.com/NSPG13/agent-bounties/pull/10",
                    "found_via": "GitHub bounty label search",
                    "motivation": "Clear scope",
                    "improvement_suggestion": "Show exact payout state",
                    "agent_or_tool": "coding agent",
                }
            ],
            post_json=fake_post,
        )

        self.assertEqual(result["discovery_responses_synced"], 1)
        response_calls = [
            payload
            for path, payload in calls
            if path == "/v1/audience/discovery-responses"
        ]
        self.assertEqual(len(response_calls), 1)
        self.assertEqual(response_calls[0]["audience_member_id"], "member:alice-agent")
        self.assertFalse(response_calls[0]["private_storage_consent"])

    def test_repository_curated_answers_reference_known_public_participants(self) -> None:
        curated_path = (
            SCRIPT_DIR / "fixtures" / "github_discovery_responses.curated.json"
        )
        curated = json.loads(curated_path.read_text(encoding="utf-8"))
        self.assertEqual(len(curated), 8)
        self.assertEqual(
            len({response["provider_response_id"] for response in curated}), len(curated)
        )
        for response in curated:
            self.assertTrue(response["handle"].strip())
            self.assertTrue(response["public_source_url"].startswith("https://github.com/"))
            self.assertTrue(response["found_via"].strip())
            self.assertTrue(response["motivation"].strip())
            self.assertTrue(response["improvement_suggestion"].strip())


if __name__ == "__main__":
    unittest.main()
