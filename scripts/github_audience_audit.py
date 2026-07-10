#!/usr/bin/env python3
"""Audit public GitHub participation and optionally sync it to the operator API.

The audit deliberately excludes email addresses, wallets, and raw comment text.
Natural-language discovery answers are emitted as curation candidates rather
than being interpreted or stored automatically.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable


DISCOVERY_QUESTION_MARKERS = (
    "how did you find agent bounties",
    "how exactly did you discover",
    "exactly how did you discover",
    "one-time distribution feedback request",
    "please answer the discovery questions",
    "what made this bounty or project worth participating",
)
DISCOVERY_ANSWER_MARKERS = (
    "## how i found",
    "how i found this",
    "discovery feedback",
    "i found this through",
    "found agent bounties",
)
MENTION_RE = re.compile(r"(?<![A-Za-z0-9-])@([A-Za-z0-9](?:[A-Za-z0-9-]{0,38}))")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def flatten_pages(value: Any) -> list[dict[str, Any]]:
    if not isinstance(value, list):
        raise ValueError("GitHub response must be a JSON array")
    if value and all(isinstance(page, list) for page in value):
        return [item for page in value for item in page if isinstance(item, dict)]
    return [item for item in value if isinstance(item, dict)]


def gh_api(repository: str, suffix: str, *, accept: str | None = None) -> list[dict[str, Any]]:
    command = [
        "gh",
        "api",
        "--paginate",
        "--slurp",
        f"repos/{repository}/{suffix.lstrip('/')}",
    ]
    if accept:
        command.extend(["-H", f"Accept: {accept}"])
    completed = subprocess.run(command, check=True, capture_output=True, text=True)
    return flatten_pages(json.loads(completed.stdout))


def collect_snapshot(repository: str) -> dict[str, Any]:
    issues = gh_api(repository, "issues?state=all&per_page=100")
    pulls = gh_api(repository, "pulls?state=all&per_page=100")
    issue_comments = gh_api(repository, "issues/comments?per_page=100")
    review_comments = gh_api(repository, "pulls/comments?per_page=100")
    reviews: list[dict[str, Any]] = []
    for pull in pulls:
        number = pull.get("number")
        if isinstance(number, int):
            for review in gh_api(repository, f"pulls/{number}/reviews?per_page=100"):
                review["pull_number"] = number
                reviews.append(review)

    bounty_issues = [
        issue
        for issue in issues
        if "pull_request" not in issue
        and "bounty" in {str(label.get("name", "")).lower() for label in issue.get("labels", [])}
    ]
    reactions: list[dict[str, Any]] = []
    for issue in bounty_issues:
        number = issue.get("number")
        if isinstance(number, int):
            for reaction in gh_api(
                repository,
                f"issues/{number}/reactions?per_page=100",
                accept="application/vnd.github+json",
            ):
                reaction["issue_number"] = number
                reactions.append(reaction)

    stargazers = gh_api(
        repository,
        "stargazers?per_page=100",
        accept="application/vnd.github.star+json",
    )
    return {
        "repository": repository,
        "fetched_at": utc_now(),
        "issues": issues,
        "pulls": pulls,
        "issue_comments": issue_comments,
        "review_comments": review_comments,
        "reviews": reviews,
        "reactions": reactions,
        "stargazers": stargazers,
    }


def is_external_user(user: Any, owner_login: str, include_owner: bool) -> bool:
    if not isinstance(user, dict):
        return False
    login = str(user.get("login", "")).strip()
    if not login or login.lower().endswith("[bot]") or user.get("type") == "Bot":
        return False
    return include_owner or login.lower() != owner_login.lower()


def participant_from_user(user: dict[str, Any]) -> dict[str, Any]:
    login = str(user["login"])
    external_id = str(user.get("id") or user.get("node_id") or login.lower())
    return {
        "provider": "github",
        "external_id": external_id,
        "handle": login,
        "public_profile_url": user.get("html_url") or f"https://github.com/{login}",
    }


def matched_marker(body: str, markers: tuple[str, ...]) -> str | None:
    lowered = body.lower()
    return next((marker for marker in markers if marker in lowered), None)


def issue_number_from_api_url(url: str) -> int | None:
    try:
        return int(url.rstrip("/").rsplit("/", 1)[-1])
    except (TypeError, ValueError):
        return None


def build_audit(
    snapshot: dict[str, Any], owner_login: str, *, include_owner: bool = False
) -> dict[str, Any]:
    participants: dict[str, dict[str, Any]] = {}
    interactions: dict[tuple[str, str], dict[str, Any]] = {}
    answer_candidates: dict[tuple[str, str], dict[str, Any]] = {}
    outreach: dict[tuple[str, str], dict[str, Any]] = {}
    issues_by_number = {
        issue["number"]: issue
        for issue in snapshot.get("issues", [])
        if isinstance(issue, dict) and isinstance(issue.get("number"), int)
    }

    def register(user: Any) -> str | None:
        if not is_external_user(user, owner_login, include_owner):
            return None
        participant = participant_from_user(user)
        key = participant["handle"].lower()
        participants[key] = participant
        return key

    def add_interaction(
        user: Any,
        provider_event_id: str,
        kind: str,
        public_url: str | None,
        occurred_at: str | None,
    ) -> None:
        login_key = register(user)
        if login_key is None:
            return
        interactions[(login_key, provider_event_id)] = {
            "handle": participants[login_key]["handle"],
            "provider_event_id": provider_event_id,
            "kind": kind,
            "public_url": public_url,
            "occurred_at": occurred_at,
            "referrer_url": None,
            "campaign": "github-public-activity",
            "source_interaction_id": None,
        }

    def consider_answer(
        user: Any, body: Any, provider_response_id: str, public_url: str | None
    ) -> None:
        if not isinstance(body, str):
            return
        marker = matched_marker(body, DISCOVERY_ANSWER_MARKERS)
        if marker is None:
            return
        login_key = register(user)
        if login_key is None or not public_url:
            return
        answer_candidates[(login_key, provider_response_id)] = {
            "handle": participants[login_key]["handle"],
            "provider_response_id": provider_response_id,
            "public_source_url": public_url,
            "matched_marker": marker,
            "curation_required": True,
        }

    for issue in snapshot.get("issues", []):
        if not isinstance(issue, dict):
            continue
        is_pull = "pull_request" in issue
        issue_id = issue.get("id") or issue.get("number")
        kind = "pull_request_opened" if is_pull else "issue_opened"
        add_interaction(
            issue.get("user"),
            f"github:issue:{issue_id}:{kind}",
            kind,
            issue.get("html_url"),
            issue.get("created_at"),
        )
        labels = {str(label.get("name", "")).lower() for label in issue.get("labels", [])}
        if not is_pull and "bounty" in labels:
            add_interaction(
                issue.get("user"),
                f"github:issue:{issue_id}:bounty_posted",
                "bounty_posted",
                issue.get("html_url"),
                issue.get("created_at"),
            )
        consider_answer(
            issue.get("user"),
            issue.get("body"),
            f"github:issue-body:{issue_id}",
            issue.get("html_url"),
        )

    last_external_commenter_by_issue: dict[int, str] = {}
    issue_comments = sorted(
        (comment for comment in snapshot.get("issue_comments", []) if isinstance(comment, dict)),
        key=lambda comment: (comment.get("created_at") or "", str(comment.get("id") or "")),
    )
    for comment in issue_comments:
        comment_id = comment.get("id")
        body = str(comment.get("body") or "")
        user = comment.get("user")
        issue_number = issue_number_from_api_url(str(comment.get("issue_url", "")))
        commenter_key = register(user)
        add_interaction(
            user,
            f"github:issue-comment:{comment_id}",
            "issue_commented",
            comment.get("html_url"),
            comment.get("created_at"),
        )
        command = body.strip().lower()
        if command.startswith("/agent-bounty fund"):
            add_interaction(
                user,
                f"github:issue-comment:{comment_id}:funding-signal",
                "funding_signaled",
                comment.get("html_url"),
                comment.get("created_at"),
            )
        if command.startswith("/claim") or command.startswith("/agent-bounty claim"):
            add_interaction(
                user,
                f"github:issue-comment:{comment_id}:claim-signal",
                "claim_signaled",
                comment.get("html_url"),
                comment.get("created_at"),
            )
        consider_answer(
            user,
            body,
            f"github:issue-comment:{comment_id}",
            comment.get("html_url"),
        )

        author_login = str((user or {}).get("login", ""))
        is_discovery_question = author_login.lower() == owner_login.lower() and matched_marker(
            body, DISCOVERY_QUESTION_MARKERS
        )
        if is_discovery_question:
            mentions = {mention.lower() for mention in MENTION_RE.findall(body)}
            target_keys = [key for key in mentions if key in participants]
            if not target_keys:
                issue = issues_by_number.get(issue_number)
                target_key = register((issue or {}).get("user"))
                if target_key is None and issue_number is not None:
                    target_key = last_external_commenter_by_issue.get(issue_number)
                target_keys = [target_key] if target_key else []
            for target_key in target_keys:
                outreach[(target_key, str(comment_id))] = {
                    "handle": participants[target_key]["handle"],
                    "provider_event_id": f"github:discovery-prompt:{comment_id}:{target_key}",
                    "channel": "github_public",
                    "public_url": comment.get("html_url"),
                    "prompt_version": "distribution-v1",
                    "status": "pending",
                    "sent_at": comment.get("created_at"),
                }
        if commenter_key is not None and issue_number is not None:
            last_external_commenter_by_issue[issue_number] = commenter_key

    for review_comment in snapshot.get("review_comments", []):
        if not isinstance(review_comment, dict):
            continue
        comment_id = review_comment.get("id")
        add_interaction(
            review_comment.get("user"),
            f"github:review-comment:{comment_id}",
            "pull_request_reviewed",
            review_comment.get("html_url"),
            review_comment.get("created_at"),
        )
        consider_answer(
            review_comment.get("user"),
            review_comment.get("body"),
            f"github:review-comment:{comment_id}",
            review_comment.get("html_url"),
        )

    for review in snapshot.get("reviews", []):
        if not isinstance(review, dict):
            continue
        review_id = review.get("id")
        pull_number = review.get("pull_number")
        public_url = (
            f"https://github.com/{snapshot.get('repository')}/pull/{pull_number}"
            if pull_number
            else None
        )
        add_interaction(
            review.get("user"),
            f"github:review:{review_id}",
            "pull_request_reviewed",
            public_url,
            review.get("submitted_at"),
        )
        consider_answer(
            review.get("user"),
            review.get("body"),
            f"github:review:{review_id}",
            public_url,
        )

    for reaction in snapshot.get("reactions", []):
        if not isinstance(reaction, dict) or reaction.get("content") != "+1":
            continue
        issue = issues_by_number.get(reaction.get("issue_number"))
        add_interaction(
            reaction.get("user"),
            f"github:reaction:{reaction.get('id')}",
            "bounty_upvoted",
            (issue or {}).get("html_url"),
            reaction.get("created_at"),
        )

    for stargazer in snapshot.get("stargazers", []):
        if not isinstance(stargazer, dict):
            continue
        user = stargazer.get("user") if isinstance(stargazer.get("user"), dict) else stargazer
        login = str((user or {}).get("login", "")).lower()
        add_interaction(
            user,
            f"github:star:{login}",
            "repo_starred",
            f"https://github.com/{snapshot.get('repository')}/stargazers",
            stargazer.get("starred_at"),
        )

    answered_keys = {key for key, _ in answer_candidates}
    asked_keys = {key for key, _ in outreach}
    for attempt in outreach.values():
        if attempt["handle"].lower() in answered_keys:
            attempt["status"] = "responded"
    participant_keys = set(participants)
    not_asked_or_answered = participant_keys - asked_keys - answered_keys
    asked_without_answer = asked_keys - answered_keys

    first_seen_by_handle: dict[str, str] = {}
    for interaction in interactions.values():
        occurred_at = interaction.get("occurred_at")
        handle_key = interaction["handle"].lower()
        if occurred_at and (
            handle_key not in first_seen_by_handle
            or occurred_at < first_seen_by_handle[handle_key]
        ):
            first_seen_by_handle[handle_key] = occurred_at
    for key, participant in participants.items():
        participant["observed_at"] = first_seen_by_handle.get(key)
        participant["roles"] = []

    return {
        "repository": snapshot.get("repository"),
        "generated_at": utc_now(),
        "privacy_boundary": {
            "public_identity_and_event_urls_only": True,
            "email_scraped": False,
            "wallet_inferred": False,
            "raw_comment_text_stored": False,
            "discovery_answers_require_human_curation": True,
        },
        "participants": sorted(participants.values(), key=lambda item: item["handle"].lower()),
        "interactions": sorted(
            interactions.values(),
            key=lambda item: (item.get("occurred_at") or "", item["provider_event_id"]),
        ),
        "outreach_attempts": sorted(
            outreach.values(), key=lambda item: (item["handle"].lower(), item["provider_event_id"])
        ),
        "discovery_answer_candidates": sorted(
            answer_candidates.values(),
            key=lambda item: (item["handle"].lower(), item["provider_response_id"]),
        ),
        "coverage": {
            "participant_count": len(participants),
            "asked_count": len(asked_keys),
            "answer_candidate_count": len(answered_keys),
            "not_asked_or_answered_handles": sorted(
                participants[key]["handle"] for key in not_asked_or_answered
            ),
            "asked_without_answer_handles": sorted(
                participants[key]["handle"] for key in asked_without_answer
            ),
        },
    }


def http_post_json(base_url: str, path: str, payload: dict[str, Any], token: str | None) -> Any:
    headers = {"Content-Type": "application/json"}
    if token:
        headers["x-operator-token"] = token
    request = urllib.request.Request(
        f"{base_url.rstrip('/')}{path}",
        data=json.dumps(payload).encode("utf-8"),
        headers=headers,
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"{path} returned HTTP {error.code}: {detail}") from error


def sync_audit(
    audit: dict[str, Any],
    api_base_url: str,
    token: str | None,
    *,
    curated_responses: list[dict[str, Any]] | None = None,
    post_json: Callable[[str, str, dict[str, Any], str | None], Any] = http_post_json,
) -> dict[str, int]:
    member_ids: dict[str, str] = {}
    for participant in audit["participants"]:
        stored = post_json(api_base_url, "/v1/audience/members", participant, token)
        member_ids[participant["handle"].lower()] = stored["id"]

    for interaction in audit["interactions"]:
        payload = dict(interaction)
        handle = payload.pop("handle").lower()
        payload["audience_member_id"] = member_ids[handle]
        post_json(api_base_url, "/v1/audience/interactions", payload, token)

    for attempt in audit["outreach_attempts"]:
        payload = dict(attempt)
        handle = payload.pop("handle").lower()
        payload["audience_member_id"] = member_ids[handle]
        post_json(api_base_url, "/v1/audience/outreach-attempts", payload, token)

    responses_synced = 0
    for response in curated_responses or []:
        payload = dict(response)
        handle = str(payload.pop("handle", "")).lower()
        if handle not in member_ids:
            raise ValueError(f"curated discovery response references unknown handle: {handle}")
        public_source_url = str(payload.get("public_source_url") or "")
        if not public_source_url.startswith(("https://", "http://")):
            raise ValueError("curated discovery responses must use a public source URL")
        payload["audience_member_id"] = member_ids[handle]
        payload["private_storage_consent"] = False
        post_json(api_base_url, "/v1/audience/discovery-responses", payload, token)
        responses_synced += 1

    return {
        "members_synced": len(audit["participants"]),
        "interactions_synced": len(audit["interactions"]),
        "outreach_attempts_synced": len(audit["outreach_attempts"]),
        "discovery_responses_synced": responses_synced,
        "discovery_candidates_requiring_curation": len(audit["discovery_answer_candidates"]),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", default="NSPG13/agent-bounties")
    parser.add_argument("--owner-login", default="NSPG13")
    parser.add_argument("--fixture", type=Path)
    parser.add_argument("--snapshot-output", type=Path)
    parser.add_argument("--output", type=Path, default=Path("target/github-audience-audit.json"))
    parser.add_argument("--include-owner", action="store_true")
    parser.add_argument("--curated-responses", type=Path)
    parser.add_argument("--sync", action="store_true")
    parser.add_argument("--api-base-url", default="http://127.0.0.1:8080")
    parser.add_argument("--operator-token-env", default="OPERATOR_API_TOKEN")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.fixture:
        snapshot = json.loads(args.fixture.read_text(encoding="utf-8"))
    else:
        snapshot = collect_snapshot(args.repository)
    snapshot.setdefault("repository", args.repository)
    if args.snapshot_output:
        args.snapshot_output.parent.mkdir(parents=True, exist_ok=True)
        args.snapshot_output.write_text(json.dumps(snapshot, indent=2) + "\n", encoding="utf-8")
    audit = build_audit(snapshot, args.owner_login, include_owner=args.include_owner)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(audit, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(audit["coverage"], indent=2))
    print(f"audit_output={args.output}")

    if args.sync:
        token = os.environ.get(args.operator_token_env)
        curated_responses = None
        if args.curated_responses:
            curated_responses = json.loads(args.curated_responses.read_text(encoding="utf-8"))
            if not isinstance(curated_responses, list):
                raise ValueError("--curated-responses must contain a JSON array")
        result = sync_audit(
            audit,
            args.api_base_url,
            token,
            curated_responses=curated_responses,
        )
        print(json.dumps(result, indent=2))
        if result["discovery_candidates_requiring_curation"]:
            print(
                "Discovery answer candidates were not auto-stored; curate their public source URLs "
                "through POST /v1/audience/discovery-responses.",
                file=sys.stderr,
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
