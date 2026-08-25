#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from reconcile_github_bounty_labels import (
    BETA3_PROTOCOL,
    LABEL_DEFINITIONS,
    MANAGED_END,
    MANAGED_START,
    HttpResult,
    LabelReconciliationError,
    augment_projection_with_beta3,
    beta3_discovery_competition_mode,
    build_plans,
    execute_plans,
    fetch_github_issues,
    issue_marker,
    is_same_repository_reviewed_beta3_artifact,
    load_landing_copy,
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
    beta3_count = sum(record["protocol_version"] == BETA3_PROTOCOL for record in items)
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
            {
                "source_type": "open_competition_v2",
                "protocol_version": BETA3_PROTOCOL,
                "factory_contract": "0x" + "3" * 40,
                "available": not degraded,
                "fresh": not degraded,
                "item_count": beta3_count,
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


def landing(record: dict, issue_number: int = 42) -> dict[str, dict]:
    return {
        record["discovery_id"]: {
            "issue_number": issue_number,
            "outcome_title": "Build a replayable compatibility report for the pinned fixtures",
            "intent_summary": "Produce a replayable compatibility report for the pinned fixtures. Let maintainers ship the change without guessing about downstream behavior.",
            "skills": ["python", "research"],
            "canonical_opportunity_url": record["public_url"],
            "acceptance_criteria": [
                "Run the pinned checker and capture exit code zero.",
                "Publish the fixture digest and exact reproduction command.",
            ],
            "safe_start": {
                "label": "Inspect the pinned fixtures",
                "url": "https://github.com/NSPG13/agent-bounties/tree/main/fixtures",
                "instructions": "Read the fixtures before claiming or signing anything.",
            },
            "reviewed_by": "NSPG13",
            "reviewed_at": NOW,
        }
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
            if "title" in body:
                record["title"] = body["title"]
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
    def test_reviewed_landing_copy_controls_title_skills_and_safe_start(self) -> None:
        record = item(90, source_url=f"https://github.com/{REPOSITORY}/issues/42")
        source = issue(42, body="Keep this human section.", labels=["bounty"])
        plan = build_plans(
            projection(record),
            [source],
            policy(),
            REPOSITORY,
            landing_entries=landing(record),
        )[0]
        self.assertEqual(
            plan.title,
            "Build a replayable compatibility report for the pinned fixtures",
        )
        self.assertIn("skill:python", plan.desired_managed_labels)
        self.assertIn("skill:research", plan.desired_managed_labels)
        self.assertIn("### Replayable acceptance criteria", plan.desired_body)
        self.assertIn("### One safe start", plan.desired_body)
        self.assertIn("Canonical opportunity URL", plan.desired_body)
        self.assertIn("utm_source=github", plan.desired_body)
        self.assertTrue(plan.desired_body.startswith("Keep this human section."))

    def test_missing_reviewed_copy_fails_closed_without_solver_invitation(self) -> None:
        record = item(91, source_url=f"https://github.com/{REPOSITORY}/issues/42")
        source = issue(
            42,
            labels=["bounty", "ai-agent-welcome", "ready-to-earn", "claimable-live"],
        )
        plan = build_plans(
            projection(record),
            [source],
            policy(),
            REPOSITORY,
            landing_entries={},
        )[0]
        self.assertEqual(plan.mapping_action, "action_required_landing_copy")
        self.assertIn("funded-live", plan.desired_managed_labels)
        for invitation in ("ai-agent-welcome", "ready-to-earn", "claimable-live"):
            self.assertNotIn(invitation, plan.desired_managed_labels)
        self.assertIn("Action required before solver invitation", plan.desired_body)
        self.assertEqual(plan.title, source["title"])

    def test_missing_reviewed_copy_does_not_create_a_new_solver_mirror(self) -> None:
        record = item(92)
        plan = build_plans(
            projection(record), [], policy(), REPOSITORY, landing_entries={}
        )[0]
        self.assertFalse(plan.create_eligible)
        self.assertFalse(plan_has_write(plan))

    def test_generic_reviewed_title_is_rejected(self) -> None:
        record = item(93)
        manifest = {
            "schema_version": "agent-bounties/github-bounty-landing-copy-v1",
            "repository": REPOSITORY,
            "entries": landing(record),
        }
        manifest["entries"][record["discovery_id"]]["outcome_title"] = "Fix bug"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, "landing.json")
            path.write_text(json.dumps(manifest), encoding="utf-8")
            with self.assertRaisesRegex(LabelReconciliationError, "outcome-specific"):
                load_landing_copy(path, REPOSITORY)

    def test_reviewed_landing_copy_rejects_non_allowlisted_skills(self) -> None:
        record = item(94)
        manifest = {
            "schema_version": "agent-bounties/github-bounty-landing-copy-v1",
            "repository": REPOSITORY,
            "entries": landing(record),
        }
        manifest["entries"][record["discovery_id"]]["skills"] = ["java"]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, "landing.json")
            path.write_text(json.dumps(manifest), encoding="utf-8")
            with self.assertRaisesRegex(LabelReconciliationError, "allowlisted"):
                load_landing_copy(path, REPOSITORY)

    def test_reviewed_landing_copy_requires_exactly_two_intent_sentences(self) -> None:
        record = item(95)
        manifest = {
            "schema_version": "agent-bounties/github-bounty-landing-copy-v1",
            "repository": REPOSITORY,
            "entries": landing(record),
        }
        manifest["entries"][record["discovery_id"]]["intent_summary"] = (
            "Produce one replayable compatibility report. "
            "Explain the impact. Include a release recommendation."
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, "landing.json")
            path.write_text(json.dumps(manifest), encoding="utf-8")
            with self.assertRaisesRegex(LabelReconciliationError, "exactly two sentences"):
                load_landing_copy(path, REPOSITORY)

    def test_beta3_discovery_represents_supported_winner_modes(self) -> None:
        self.assertEqual(
            beta3_discovery_competition_mode("first_proven"),
            "first_valid_submission",
        )
        self.assertEqual(beta3_discovery_competition_mode("best_score"), "best_score")
        with self.assertRaisesRegex(LabelReconciliationError, "winner mode safely"):
            beta3_discovery_competition_mode("subjective_judgment")

    def test_beta3_artifact_source_requires_pinned_allowlisted_same_repo_url(self) -> None:
        pinned = (
            f"https://github.com/{REPOSITORY}/blob/"
            + "a" * 40
            + "/ops/open-competition-v2-forward-gmv-reward-cohort-v1.json"
        )
        self.assertTrue(is_same_repository_reviewed_beta3_artifact(pinned, REPOSITORY))
        self.assertFalse(
            is_same_repository_reviewed_beta3_artifact(
                f"https://github.com/{REPOSITORY}/blob/main/ops/open-competition-v2-forward-gmv-reward-cohort-v1.json",
                REPOSITORY,
            )
        )
        self.assertFalse(
            is_same_repository_reviewed_beta3_artifact(
                f"https://github.com/{REPOSITORY}/blob/{'a' * 40}/README.md",
                REPOSITORY,
            )
        )
        self.assertFalse(
            is_same_repository_reviewed_beta3_artifact(
                f"https://github.com/another/repo/blob/{'a' * 40}/ops/open-competition-v2-forward-gmv-reward-cohort-v1.json",
                REPOSITORY,
            )
        )

    def test_beta3_augmentation_includes_pinned_best_score_artifact(self) -> None:
        contract = "0x" + "4" * 40
        bounty_id = "0x" + "5" * 64
        source_url = (
            f"https://github.com/{REPOSITORY}/blob/"
            + "a" * 40
            + "/ops/open-competition-v2-forward-gmv-reward-cohort-v1.json"
        )
        base = projection()
        base["source_statuses"] = [
            source
            for source in base["source_statuses"]
            if source["protocol_version"] != BETA3_PROTOCOL
        ]
        amount = lambda value: {
            "amount": str(value),
            "currency": "USDC",
            "unit": "base_units",
        }
        inventory = {
            "network": NETWORK,
            "protocol_version": BETA3_PROTOCOL,
            "competitions": [
                {
                    "record": {
                        "network": NETWORK,
                        "factory_contract": "0x" + "3" * 40,
                        "safe_block_number": 49_799_500,
                        "safe_block_hash": "0x" + "d" * 64,
                        "projection": {
                            "competition": contract,
                            "bounty_id": bounty_id,
                            "state": "active",
                            "winner_mode": "best_score",
                        },
                    }
                }
            ],
        }
        events = {
            "network": NETWORK,
            "protocol_version": BETA3_PROTOCOL,
            "events": [
                {
                    "contract_address": contract,
                    "bounty_id": bounty_id,
                    "block_number": 49_799_100,
                    "kind": "competition_activated",
                }
            ],
        }
        opportunity = {
            "source_id": contract,
            "source_url": source_url,
            "created_at": "2026-08-24T18:00:00Z",
            "updated_at": "2026-08-24T18:00:00Z",
            "title": "6 USDC prize — Highest externally funded canonical GMV — daily 20260825",
            "goal": "Create and fund useful demand. Highest eligible score wins.",
            "categories": ["research"],
            "skills": ["browser"],
            "public_url": f"https://agentbounties.app/competition.html?bountyContract={contract}&network=base-mainnet",
            "verification_ready": True,
            "winner_mode": "best_score",
            "reward": amount(6_000_000),
            "completion_bonus": amount(40_000),
            "bond": amount(0),
            "funded_amount": amount(6_040_000),
            "funding_target": amount(6_040_000),
            "entry_count": 0,
            "deadline": "2026-11-22T00:00:00Z",
            "deadline_kind": "proof_deadline",
            "verifier_profile_id": "forward-canonical-gmv-attribution-metric-v2",
            "verifier_profile_name": "forward-canonical-gmv-attribution-metric-v2",
            "verification_method": "sp1_plonk",
            "cash_economics": {"required_external_spend": amount(110_000)},
            "evidence_requirements": {
                "protocol_version": BETA3_PROTOCOL,
                "participation_phase": "upcoming",
                "scoring_window": {
                    "starts_at": "2026-08-25T00:00:00Z",
                    "ends_at": "2026-08-26T00:00:00Z",
                    "minimum_score_base_units": "1",
                },
                "scoring_formula": "sum(settlement_gmv * entrant_funding / total_funding)",
                "qualifying_action": {
                    "objective": "Post or fund useful marketplace demand that reaches canonical settlement inside the scoring window.",
                    "excluded": ["operator or reserve wallets"],
                },
            },
            "next_action": {
                "action": "prepare_open_competition_v2_score",
                "method": "GET",
                "url": f"https://agentbounties.app/competition.html?bountyContract={contract}&network=base-mainnet",
                "instructions": "Prepare the exact child-bounty brief.",
            },
        }

        def request(method, url, body, headers):
            if "/v1/metrics/platform" in url:
                return HttpResult(
                    200,
                    {
                        "coverage": {
                            "marketplace_indexers_fresh": True,
                            "awaiting_block_time_events": 0,
                        }
                    },
                    {},
                )
            if "/inventory?" in url:
                return HttpResult(200, inventory, {})
            if "/events?" in url:
                return HttpResult(200, events, {})
            if "/v1/opportunities?" in url:
                return HttpResult(200, {"items": [opportunity]}, {})
            raise AssertionError(url)

        augmented = augment_projection_with_beta3(
            request,
            "https://api.agentbounties.app",
            NETWORK,
            REPOSITORY,
            base,
        )
        beta3 = [
            record
            for record in augmented["items"]
            if record["protocol_version"] == BETA3_PROTOCOL
        ]
        self.assertEqual(len(beta3), 1)
        self.assertEqual(beta3[0]["competition_mode"], "best_score")
        self.assertEqual(beta3[0]["participation_phase"], "upcoming")
        self.assertEqual(beta3[0]["next_action"]["label"], "Prepare scoring work")

    def test_workflow_is_least_privilege_concurrent_dry_run_by_default(self) -> None:
        workflow = Path(".github/workflows/bounty-inventory-guard.yml").read_text(encoding="utf-8")
        self.assertIn("issues: read", workflow)
        self.assertEqual(workflow.count("issues: write"), 1)
        self.assertIn("group: canonical-github-bounty-reconciliation", workflow)
        self.assertIn("cancel-in-progress: false", workflow)
        self.assertIn("default: false", workflow)
        self.assertIn("vars.BOUNTY_DISCOVERY_EXECUTE == 'true'", workflow)
        self.assertNotIn("vars.GITHUB_", workflow)
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

    def test_best_score_beta3_artifact_creates_decision_grade_mirror(self) -> None:
        record = item(1060)
        contract = record["bounty_contract"]
        record.update(
            {
                "discovery_id": f"eip155:{CHAIN_ID}:{BETA3_PROTOCOL}:{contract}",
                "protocol_version": BETA3_PROTOCOL,
                "source_url": (
                    f"https://github.com/{REPOSITORY}/blob/"
                    + "a" * 40
                    + "/ops/open-competition-v2-forward-gmv-reward-cohort-v1.json"
                ),
                "competition_mode": "best_score",
                "title": "6 USDC prize — Highest externally funded canonical GMV — daily 20260825",
                "public_url": f"https://agentbounties.app/competition.html?bountyContract={contract}&network=base-mainnet",
                "reward_usdc_base_units": "6000000",
                "verifier_reward_usdc_base_units": "40000",
                "bond_usdc_base_units": "0",
                "funded_usdc_base_units": "6040000",
                "funding_target_usdc_base_units": "6040000",
                "entry_count": 0,
                "max_entries": None,
                "participation_phase": "upcoming",
                "scoring_window": {
                    "starts_at": "2026-08-25T00:00:00Z",
                    "ends_at": "2026-08-26T00:00:00Z",
                    "minimum_score_base_units": "1",
                },
                "scoring_formula": "sum(settlement_gmv * entrant_funding / total_funding)",
                "qualifying_action": {
                    "objective": "Post or fund useful marketplace demand that reaches canonical settlement inside the scoring window.",
                    "excluded": ["operator or reserve wallets", "excluded reward contracts"],
                },
                "cash_economics": {
                    "required_external_spend": {
                        "amount": "110000",
                        "currency": "USDC",
                        "unit": "base_units",
                    }
                },
                "next_action": {
                    "kind": "prepare_open_competition_v2_score",
                    "label": "Prepare scoring work",
                    "method": "GET",
                    "url": f"https://agentbounties.app/competition.html?bountyContract={contract}&network=base-mainnet",
                    "instructions": "Prepare now; do not fund score before the UTC window starts.",
                },
            }
        )
        plan = build_plans(
            projection(record),
            [],
            policy(),
            REPOSITORY,
            landing_entries={},
        )[0]
        self.assertTrue(plan.create_eligible)
        self.assertEqual(plan.mapping_action, "current_nonterminal_backfill")
        self.assertTrue(
            {"ready-to-earn", "claimable-live", "open-competition", "verifier"}.issubset(
                plan.desired_managed_labels
            )
        )
        self.assertIn("Current competition state:** `upcoming`", plan.desired_body)
        self.assertIn("2026-08-25T00:00:00Z", plan.desired_body)
        self.assertIn("Hosted proof and relay cost:** 0.11 USDC", plan.desired_body)
        self.assertIn("still spent if this competition entry loses", plan.desired_body)
        self.assertIn("a `/claim` comment, GitHub PR", plan.desired_body)
        self.assertIn("accepted entries normally remain at zero during scoring", plan.desired_body)
        self.assertIn("one concrete digital deliverable with binary acceptance tests", plan.desired_body)
        self.assertIn("Prepare scoring work", plan.desired_body)
        self.assertTrue(plan.title.startswith("Generate qualifying GMV"))

    def test_github_claim_command_recovers_to_v1_open_competition(self) -> None:
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

    def test_github_claim_command_recovers_to_v2_best_score_phase(self) -> None:
        contract = "0x" + "4" * 40
        discovery_id = f"eip155:{CHAIN_ID}:{BETA3_PROTOCOL}:{contract}"
        recovery = open_competition_wrong_mode_plan(
            {
                "url": f"https://github.com/{REPOSITORY}/issues/1214",
                "issue_body": "\n".join(
                    [
                        MANAGED_START,
                        f'<!-- agent-bounties/github-discovery-v1 {{"discovery_id":"{discovery_id}"}} -->',
                        "- **Current competition state:** `scoring`",
                        "- **Verifier:** forward-canonical-gmv-attribution-metric-v2 (`sp1_plonk`; ready: `true`)",
                        "### Best-score competition rules",
                        "### Next action",
                        MANAGED_END,
                    ]
                ),
            }
        )
        self.assertFalse(recovery["ready"])
        self.assertEqual(recovery["signal"]["protocol_version"], BETA3_PROTOCOL)
        self.assertEqual(recovery["signal"]["competition_mode"], "best_score")
        self.assertEqual(recovery["signal"]["participation_phase"], "scoring")
        self.assertEqual(
            recovery["signal"]["correct_action"],
            "generate_open_competition_v2_score",
        )
        self.assertEqual(recovery["signal"]["bounty_contract"], contract)
        self.assertEqual(recovery["signal"]["discovery_id"], discovery_id)
        self.assertIn("CompetitionEntryQualifiedV2", recovery["check"]["text"])
        self.assertIn("GitHub PR is not a competition entry", recovery["check"]["text"])
        self.assertNotIn("first_valid_submission", recovery["check"]["text"])
        self.assertNotIn("open-competition-v1", recovery["check"]["text"])
        self.assertNotIn("commitment recovery envelope", recovery["check"]["text"])

    def test_reuses_same_repository_source_and_preserves_human_content(self) -> None:
        source = issue(42, body="Keep this human section.", labels=["bounty", "help wanted"])
        record = item(2, source_url=f"https://github.com/{REPOSITORY}/issues/42")
        plan = build_plans(projection(record), [source], policy(), REPOSITORY)[0]
        self.assertEqual(plan.issue_number, 42)
        self.assertEqual(plan.mapping_action, "reuse_source")
        self.assertTrue(plan.desired_body.startswith("Keep this human section."))
        self.assertIn(MANAGED_START, plan.desired_body)
        self.assertNotIn("help wanted", plan.remove_labels)

    def test_legacy_non_ready_issue_reconciles_labels_without_reformatting_content(self) -> None:
        record = item(
            96,
            state="unavailable",
            source_url=f"https://github.com/{REPOSITORY}/issues/96",
            verifier_ready=False,
        )
        legacy_body = (
            "Human-authored context.\n\n"
            f"{MANAGED_START}\n"
            f'<!-- agent-bounties/github-discovery-v1 {{"discovery_id":"{record["discovery_id"]}"}} -->\n'
            "## Canonical bounty discovery\n\n"
            "- **Mode:** Exclusive claim\n"
            "- **Lifecycle:** `ready_to_earn`\n"
            f"{MANAGED_END}\n"
        )
        source = issue(
            96,
            body=legacy_body,
            labels=["bounty", "payments", "ai-agent-welcome", "ready-to-earn"],
        )
        plan = build_plans(projection(record), [source], policy(), REPOSITORY)[0]
        self.assertEqual(plan.original_title, plan.title)
        self.assertEqual(plan.original_body, plan.desired_body)
        self.assertIn("ai-agent-welcome", plan.remove_labels)
        self.assertIn("ready-to-earn", plan.remove_labels)
        self.assertIn("verification-unavailable", plan.add_labels)
        self.assertTrue(plan_has_write(plan))

    def test_current_discovery_block_keeps_reconciling_after_work_starts(self) -> None:
        ready = item(97, source_url=f"https://github.com/{REPOSITORY}/issues/97")
        source = issue(97, body="Human-authored context.", labels=["bounty"])
        ready_plan = build_plans(
            projection(ready),
            [source],
            policy(),
            REPOSITORY,
            landing_entries=landing(ready, issue_number=97),
        )[0]
        active = item(
            97,
            state="in_progress",
            source_url=f"https://github.com/{REPOSITORY}/issues/97",
        )
        upgraded = issue(
            97,
            body=ready_plan.desired_body,
            labels=ready_plan.desired_managed_labels,
        )
        upgraded["title"] = ready_plan.title
        active_plan = build_plans(projection(active), [upgraded], policy(), REPOSITORY)[0]
        self.assertEqual(active_plan.title, ready_plan.title)
        self.assertNotEqual(active_plan.original_body, active_plan.desired_body)
        self.assertIn("- **Current work state:** `in_progress`", active_plan.desired_body)
        self.assertNotIn("ready-to-earn", active_plan.desired_managed_labels)
        self.assertIn("claimed-live", active_plan.desired_managed_labels)

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
        self.assertEqual(required.desired_managed_labels, ["bounty", "payments", "settled-paid"])

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

    def test_closed_historical_issue_is_not_reformatted_or_rewritten(self) -> None:
        settled = item(120, state="settled")
        settled["source_url"] = f"https://github.com/{REPOSITORY}/issues/120"
        source = issue(
            120,
            body="Historical human text and an earlier managed block stay untouched.",
            labels=["bounty", "settled-paid"],
            state="closed",
        )
        source["state_reason"] = "completed"
        plan = build_plans(projection(settled), [source], policy(), REPOSITORY)[0]
        self.assertEqual(plan.mapping_action, "preserve_closed_historical")
        self.assertEqual(plan.original_body, plan.desired_body)
        self.assertEqual(plan.original_title, plan.title)
        self.assertFalse(plan_has_write(plan))

    def test_beta3_settlement_removes_earning_labels_and_records_keeper(self) -> None:
        settled = item(
            1059,
            state="settled",
            mode="first_valid_submission",
            source_url=f"https://github.com/{REPOSITORY}/issues/1059",
        )
        contract = settled["bounty_contract"]
        settled.update(
            {
                "discovery_id": f"eip155:{CHAIN_ID}:{BETA3_PROTOCOL}:{contract}",
                "protocol_version": BETA3_PROTOCOL,
                "reward_usdc_base_units": "3000000",
                "verifier_reward_usdc_base_units": "40000",
                "bond_usdc_base_units": "0",
                "funded_usdc_base_units": "3040000",
                "funding_target_usdc_base_units": "3040000",
                "settlement_evidence": {
                    "event_name": "CompetitionSettledV2",
                    "bounty_id": settled["bounty_id"],
                    "bounty_contract": contract,
                    "transaction_hash": TX,
                    "block_number": 49_799_005,
                    "log_index": 435,
                    "solver_wallet": "0x" + "9" * 40,
                    "solver_reward": "3000000",
                    "keeper_wallet": "0x" + "6" * 40,
                    "keeper_reward": "40000",
                    "confirmed_canonical": True,
                },
            }
        )
        source = issue(
            1059,
            labels=["bounty", "funded-live", "ready-to-earn", "claimable-live"],
        )
        plan = build_plans(projection(settled), [source], policy(), REPOSITORY)[0]
        self.assertEqual(plan.desired_state, "closed")
        self.assertEqual(plan.desired_state_reason, "completed")
        self.assertIn("settled-paid", plan.add_labels)
        self.assertEqual(
            set(plan.remove_labels),
            {"claimable-live", "funded-live", "ready-to-earn"},
        )
        self.assertIn("CompetitionSettledV2", plan.settlement_receipt.body)
        self.assertIn("Keeper wallet", plan.settlement_receipt.body)
        self.assertIn("0.04 USDC", plan.settlement_receipt.body)

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
            markdown = root / "report.md"
            fixture.write_text(json.dumps({"projection": projection(record), "issues": []}), encoding="utf-8")
            policy_path.write_text(json.dumps(policy()), encoding="utf-8")
            self.assertEqual(
                main(
                    [
                        "--fixture",
                        str(fixture),
                        "--policy",
                        str(policy_path),
                        "--json-out",
                        str(report),
                        "--md-out",
                        str(markdown),
                    ]
                ),
                0,
            )
            payload = json.loads(report.read_text(encoding="utf-8"))
            self.assertEqual(payload["mode"], "dry-run")
            self.assertEqual(payload["coverage_percent"], 100.0)
            self.assertEqual(payload["covered_record_count"], 1)
            self.assertIn("Covered records: `1`", markdown.read_text(encoding="utf-8"))
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
