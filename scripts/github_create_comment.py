#!/usr/bin/env python3
"""Publish review-only bounty drafts for `/agent-bounty create` issue comments."""

from __future__ import annotations

import argparse
import io
import json
import os
import pathlib
import re
import subprocess
import sys
from typing import List, Mapping, Optional, TextIO

from _shared.github_actions import (
    cargo_body_path,
    find_executable,
    load_issue_comments,
    publish_issue_comment,
    repo_root as shared_repo_root,
)


MARKER = "<!-- agent-bounties-create-comment -->"
CREATE_COMMAND_RE = re.compile(
    r"(?im)^\s*/agent-bounty\s+create\s+\S+\s+USDC\s*$"
)
COMMENT_ID_RE = re.compile(r"Create comment id:\s*`?([0-9]+)`?")
IDEMPOTENCY_RE = re.compile(r"Idempotency key:\s*`?([^\s`]+)`?")


class UserError(RuntimeError):
    pass


def repo_root() -> pathlib.Path:
    return shared_repo_root(__file__)


def cargo_path(path: pathlib.Path, cargo: str) -> str:
    return cargo_body_path(path, cargo)


def read_event(env: Mapping[str, str]) -> Mapping[str, object]:
    event_path = env.get("GITHUB_EVENT_PATH")
    if not event_path:
        raise UserError("GITHUB_EVENT_PATH is required")
    value = json.loads(pathlib.Path(event_path).read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise UserError("issue_comment event must be a JSON object")
    return value


def issue_context(
    env: Mapping[str, str], event: Mapping[str, object], tmp_dir: pathlib.Path
) -> tuple[dict[str, object], pathlib.Path]:
    issue = event.get("issue") or {}
    comment = event.get("comment") or {}
    repository = event.get("repository") or {}
    if not isinstance(issue, dict) or not isinstance(comment, dict):
        raise UserError("issue_comment event is required")
    if not isinstance(repository, dict):
        repository = {}
    user = comment.get("user") if isinstance(comment.get("user"), dict) else {}
    body_file = tmp_dir / "agent-bounty-create-issue-body.md"
    body_file.write_text(str(issue.get("body") or ""), encoding="utf-8")
    meta: dict[str, object] = {
        "repo": env.get("GITHUB_REPOSITORY") or repository.get("full_name") or "",
        "number": issue.get("number"),
        "title": issue.get("title") or "",
        "url": issue.get("html_url") or "",
        "comment_body": comment.get("body") or "",
        "comment_id": str(comment.get("id") or ""),
        "comment_url": comment.get("html_url") or "",
        "contributor_login": user.get("login") or "",
    }
    missing = [
        key
        for key, value in meta.items()
        if key != "comment_url" and value in ("", None)
    ]
    if missing:
        raise UserError(f"create comment event missing: {', '.join(missing)}")
    if not CREATE_COMMAND_RE.search(str(meta["comment_body"])):
        raise UserError("comment must contain `/agent-bounty create <amount> USDC`")
    return meta, body_file


def load_comments(
    env: Mapping[str, str], meta: Mapping[str, object]
) -> List[Mapping[str, object]]:
    return load_issue_comments(
        env,
        meta["repo"],
        meta["number"],
        "AGENT_BOUNTIES_CREATE_COMMENTS_FILE",
        "gh is required to inspect existing create planner comments",
        UserError,
    )


def tagged_value(pattern: re.Pattern[str], body: str) -> Optional[str]:
    match = pattern.search(body)
    return match.group(1) if match else None


def existing_keys(
    comments: List[Mapping[str, object]], current_comment_id: str
) -> List[str]:
    keys: List[str] = []
    for comment in comments:
        body = str(comment.get("body") or "")
        if MARKER not in body:
            continue
        if tagged_value(COMMENT_ID_RE, body) == current_comment_id:
            continue
        key = tagged_value(IDEMPOTENCY_RE, body)
        if key:
            keys.append(key)
    return sorted(set(keys))


def existing_reply_id(
    comments: List[Mapping[str, object]], current_comment_id: str
) -> object | None:
    return next(
        (
            comment.get("id")
            for comment in comments
            if MARKER in str(comment.get("body") or "")
            and tagged_value(COMMENT_ID_RE, str(comment.get("body") or ""))
            == current_comment_id
        ),
        None,
    )


def run_plan(
    env: Mapping[str, str],
    workspace: pathlib.Path,
    meta: Mapping[str, object],
    body_file: pathlib.Path,
    idempotency_keys: List[str],
) -> str:
    cargo = find_executable(["cargo", "cargo.exe"])
    if not cargo:
        raise UserError("cargo is required to plan a create comment")
    command = [
        cargo,
        "run",
        "-p",
        "cli",
        "--",
        "github-create-comment-plan",
        "--repository",
        str(meta["repo"]),
        "--issue-url",
        str(meta["url"]),
        "--title",
        str(meta["title"]),
        "--body-file",
        cargo_path(body_file, cargo),
        "--comment-body",
        str(meta["comment_body"]),
        "--contributor-login",
        str(meta["contributor_login"]),
        "--comment-id",
        str(meta["comment_id"]),
    ]
    for key in idempotency_keys:
        command.extend(["--existing-idempotency-key", key])
    result = subprocess.run(
        command,
        cwd=workspace,
        env=dict(env),
        text=True,
        stdout=subprocess.PIPE,
        check=False,
    )
    if result.returncode:
        raise UserError(
            f"github-create-comment-plan failed with exit code {result.returncode}"
        )
    return result.stdout


def render_comment(meta: Mapping[str, object], plan: Mapping[str, object]) -> str:
    check = plan.get("check") if isinstance(plan.get("check"), dict) else {}
    signal = plan.get("signal") if isinstance(plan.get("signal"), dict) else {}
    draft = signal.get("draft") if isinstance(signal.get("draft"), dict) else {}
    conclusion = str(check.get("conclusion") or "ActionRequired")
    summary = str(check.get("summary") or plan.get("error") or "Draft unavailable")
    details = str(check.get("text") or "")
    handoff = str(draft.get("draft_handoff_url") or "")
    idempotency = str(signal.get("idempotency_key") or "unavailable")
    lines = [
        MARKER,
        f"### Agent bounty issue draft: {conclusion}",
        "",
        summary,
        "",
    ]
    if handoff:
        lines.extend(
            [
                f"[Review the draft and continue to the canonical wallet handoff]({handoff})",
                "",
                "The issue text is advisory draft context. Acceptance criteria, verifier scope, rewards, deadlines, and the wallet transaction all require review.",
                "",
            ]
        )
    lines.extend(
        [
            "This bot reply is not a bounty, funding, claimability, acceptance, payout authorization, or settlement evidence.",
            "",
            f"Create comment id: `{meta['comment_id']}`",
            f"Idempotency key: `{idempotency}`",
            "",
            "<details><summary>Planner evidence boundaries</summary>",
            "",
            "```",
            details,
            "```",
            "",
            "</details>",
            "",
        ]
    )
    return "\n".join(lines)


def publish(
    env: Mapping[str, str],
    meta: Mapping[str, object],
    comments: List[Mapping[str, object]],
    body: str,
) -> None:
    publish_issue_comment(
        env,
        meta["repo"],
        meta["number"],
        MARKER,
        body,
        "agent-bounty-create-comment.md",
        "gh is required to publish the create planner comment",
        UserError,
        comments,
        lambda text: MARKER in text
        and tagged_value(COMMENT_ID_RE, text) == str(meta["comment_id"]),
    )


def run_from_env(env: Mapping[str, str], stdout: TextIO) -> int:
    workspace = pathlib.Path(env.get("GITHUB_WORKSPACE") or repo_root()).resolve()
    tmp_dir = pathlib.Path(
        env.get("RUNNER_TEMP") or workspace / "target" / "tmp"
    ).resolve()
    tmp_dir.mkdir(parents=True, exist_ok=True)
    meta, body_file = issue_context(env, read_event(env), tmp_dir)
    comments = load_comments(env, meta)
    plan_json = run_plan(
        env,
        workspace,
        meta,
        body_file,
        existing_keys(comments, str(meta["comment_id"])),
    )
    plan = json.loads(plan_json)
    comment = render_comment(meta, plan)
    (tmp_dir / "agent-bounty-create-plan.json").write_text(plan_json, encoding="utf-8")
    (tmp_dir / "agent-bounty-create-comment.md").write_text(comment, encoding="utf-8")
    summary = env.get("GITHUB_STEP_SUMMARY")
    if summary:
        with pathlib.Path(summary).open("a", encoding="utf-8") as handle:
            handle.write("## Agent bounty issue draft\n\n" + comment + "\n")
    if env.get("DRY_RUN") == "1":
        stdout.write(plan_json.rstrip() + "\n\n" + comment)
    else:
        publish(env, meta, comments, comment)
    return 0


def self_test() -> int:
    root = repo_root()
    tmp_dir = root / "target" / "tmp"
    tmp_dir.mkdir(parents=True, exist_ok=True)
    event = {
        "repository": {"full_name": "agent-bounties/agent-bounties"},
        "issue": {
            "number": 501,
            "title": "Fix canonical receipt reconciliation",
            "html_url": "https://github.com/agent-bounties/agent-bounties/issues/501",
            "body": "The receipt worker drops a confirmed log after restart.",
        },
        "comment": {
            "id": 9001,
            "html_url": "https://github.com/agent-bounties/agent-bounties/issues/501#issuecomment-9001",
            "body": "/agent-bounty create 25 USDC",
            "user": {"login": "maintainer"},
        },
    }
    event_file = tmp_dir / "agent-bounty-create-event.json"
    event_file.write_text(json.dumps(event), encoding="utf-8")
    comments_file = tmp_dir / "agent-bounty-create-existing-comments.json"
    existing_comments = [
        {
            "id": 7001,
            "body": "\n".join(
                [
                    MARKER,
                    "Create comment id: `9001`",
                    "Idempotency key: `github-create-comment:prior-replay`",
                ]
            ),
        }
    ]
    comments_file.write_text(json.dumps(existing_comments), encoding="utf-8")
    if existing_reply_id(existing_comments, "9001") != 7001:
        raise UserError("self-test failed to select the existing sticky reply")
    env = dict(os.environ)
    env.update(
        {
            "GITHUB_EVENT_PATH": str(event_file),
            "GITHUB_REPOSITORY": "agent-bounties/agent-bounties",
            "GITHUB_WORKSPACE": str(root),
            "RUNNER_TEMP": str(tmp_dir),
            "AGENT_BOUNTIES_CREATE_COMMENTS_FILE": str(comments_file),
            "DRY_RUN": "1",
        }
    )
    output = io.StringIO()
    run_from_env(env, output)
    rendered = output.getvalue()
    required = [
        MARKER,
        "Agent bounty issue draft: Success",
        "Review the draft and continue to the canonical wallet handoff",
        "state\": \"review_required_not_published",
        "acceptance_criteria\": []",
        "bounty_created\": false",
        "canonical_funding_confirmed\": false",
        "github-create-comment:agent-bounties/agent-bounties:https://github.com/agent-bounties/agent-bounties/issues/501:comment:9001",
    ]
    missing = [item for item in required if item not in rendered]
    if missing:
        raise UserError(f"self-test output missing: {', '.join(missing)}")
    print("GitHub create comment dry-run passed")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    try:
        return self_test() if args.self_test else run_from_env(os.environ, sys.stdout)
    except UserError as error:
        print(error, file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
