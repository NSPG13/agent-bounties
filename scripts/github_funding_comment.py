#!/usr/bin/env python3
"""Plan and publish public funding-comment signals for GitHub bounty issues."""

from __future__ import annotations

import argparse
import io
import json
import os
import pathlib
import re
import subprocess
import sys
from typing import Dict, List, Mapping, Optional, TextIO, Tuple

from _shared.github_actions import (
    append_step_summary as append_github_summary,
    cargo_body_path,
    find_executable,
    json_field,
    load_issue_comments,
    publish_issue_comment,
    read_event as read_github_event,
    repo_root,
)


MARKER = "<!-- agent-bounties-funding-comment -->"
FUNDING_COMMAND_RE = re.compile(r"(?im)^\s*/agent-bounty\s+fund\s+")
COMMENT_ID_RE = re.compile(r"Funding comment id:\s*`?([0-9]+)`?")
IDEMPOTENCY_RE = re.compile(r"Idempotency key:\s*`?([^\s`]+)`?")


class UserError(RuntimeError):
    pass


def script_repo_root() -> pathlib.Path:
    return repo_root(__file__)


def read_json_field(value: object, field: str) -> object:
    return json_field(value, field, UserError, "funding planner output missing field: {field}")


def read_event(env: Mapping[str, str]) -> Dict[str, object]:
    return read_github_event(env, UserError)


def write_issue_files(
    env: Mapping[str, str], event: Mapping[str, object], tmp_dir: pathlib.Path
) -> Tuple[Dict[str, object], pathlib.Path]:
    issue = event.get("issue") or {}
    comment = event.get("comment") or {}
    repository = event.get("repository") or {}
    if not isinstance(issue, dict) or not isinstance(comment, dict):
        raise UserError("issue_comment event is required")

    body_file = tmp_dir / "paid-bounty-funding-issue-body.md"
    body_file.write_text(str(issue.get("body") or ""), encoding="utf-8")

    comment_user = comment.get("user") if isinstance(comment.get("user"), dict) else {}
    meta: Dict[str, object] = {
        "repo": env.get("GITHUB_REPOSITORY") or repository.get("full_name") or "",
        "number": issue.get("number"),
        "title": issue.get("title") or "",
        "url": issue.get("html_url") or "",
        "comment_body": comment.get("body") or "",
        "comment_id": str(comment.get("id") or ""),
        "comment_url": comment.get("html_url") or "",
        "contributor_login": comment_user.get("login") or "",
    }
    missing = [key for key, value in meta.items() if key != "comment_url" and value in ("", None)]
    if missing:
        raise UserError(f"funding comment event missing required metadata: {', '.join(missing)}")
    if not FUNDING_COMMAND_RE.search(str(meta["comment_body"])):
        raise UserError("comment does not contain an /agent-bounty fund command")

    return meta, body_file


def load_existing_comments(env: Mapping[str, str], meta: Mapping[str, object]) -> List[Mapping[str, object]]:
    return load_issue_comments(
        env,
        meta["repo"],
        meta["number"],
        "AGENT_BOUNTIES_FUNDING_COMMENTS_FILE",
        "gh is required to inspect existing funding planner comments",
        UserError,
    )


def funding_comment_id(body: str) -> Optional[str]:
    match = COMMENT_ID_RE.search(body)
    return match.group(1) if match else None


def funding_idempotency_key(body: str) -> Optional[str]:
    match = IDEMPOTENCY_RE.search(body)
    return match.group(1) if match else None


def existing_idempotency_keys(
    existing_comments: List[Mapping[str, object]], current_comment_id: str
) -> List[str]:
    keys: List[str] = []
    for comment in existing_comments:
        body = str(comment.get("body") or "")
        if MARKER not in body:
            continue
        if funding_comment_id(body) == current_comment_id:
            continue
        key = funding_idempotency_key(body)
        if key:
            keys.append(key)
    return sorted(set(keys))


def run_github_funding_plan(
    env: Mapping[str, str],
    workspace: pathlib.Path,
    meta: Mapping[str, object],
    body_file: pathlib.Path,
    idempotency_keys: List[str],
) -> str:
    cargo_path = find_executable(["cargo", "cargo.exe"])
    if not cargo_path:
        raise UserError("cargo is required to plan a funding comment")

    command = [
        cargo_path,
        "run",
        "-p",
        "cli",
        "--",
        "github-funding-comment-plan",
        "--repository",
        str(meta["repo"]),
        "--issue-url",
        str(meta["url"]),
        "--title",
        str(meta["title"]),
        "--body-file",
        cargo_body_path(body_file, cargo_path),
        "--comment-body",
        str(meta["comment_body"]),
        "--contributor-login",
        str(meta["contributor_login"]),
        "--comment-id",
        str(meta["comment_id"]),
    ]
    for key in idempotency_keys:
        command.extend(["--existing-idempotency-key", key])

    funding_api_base_url = str(env.get("AGENT_BOUNTIES_API_BASE_URL") or "").strip()
    if funding_api_base_url:
        command.extend(["--funding-api-base-url", funding_api_base_url])

    result = subprocess.run(
        command,
        cwd=workspace,
        env=dict(env),
        text=True,
        stdout=subprocess.PIPE,
        stderr=None,
        check=False,
    )
    if result.returncode != 0:
        raise UserError(f"github-funding-comment-plan failed with exit code {result.returncode}")
    return result.stdout


def render_comment(meta: Mapping[str, object], plan: Mapping[str, object]) -> str:
    conclusion = str(read_json_field(plan, "check.conclusion"))
    summary = str(read_json_field(plan, "check.summary"))
    details = str(read_json_field(plan, "check.text"))
    ready = bool(plan.get("ready"))
    status_line = (
        "This comment is a public funding signal only; operators must reconcile actual Stripe/Base funding before claimability or payout."
        if ready
        else "This funding comment needs edits before it can become a public funding signal."
    )
    comment_url = str(meta.get("comment_url") or "").strip()
    comment_ref = comment_url or f"issue comment {meta['comment_id']}"

    return "\n".join(
        [
            MARKER,
            f"### Agent bounty funding signal: {conclusion}",
            "",
            summary,
            "",
            status_line,
            "",
            f"Funding comment id: `{meta['comment_id']}`",
            f"Funding comment: {comment_ref}",
            "",
            "<details><summary>Planner output</summary>",
            "",
            "```",
            details,
            "```",
            "",
            "</details>",
            "",
        ]
    )


def append_step_summary(env: Mapping[str, str], comment: str) -> None:
    append_github_summary(env, "Agent bounty funding signal", comment)


def publish_comment(
    env: Mapping[str, str],
    meta: Mapping[str, object],
    existing_comments: List[Mapping[str, object]],
    comment: str,
) -> None:
    publish_issue_comment(
        env,
        meta["repo"],
        meta["number"],
        MARKER,
        comment,
        "paid-bounty-funding-comment.md",
        "gh is required to publish the funding planner comment",
        UserError,
        existing_comments,
        lambda body: MARKER in body
        and funding_comment_id(body) == str(meta["comment_id"]),
    )


def run_from_env(env: Mapping[str, str], stdout: TextIO) -> int:
    repo_root = script_repo_root()
    workspace = pathlib.Path(env.get("GITHUB_WORKSPACE") or repo_root).resolve()
    tmp_dir = pathlib.Path(env.get("RUNNER_TEMP") or workspace / "target" / "tmp").resolve()
    tmp_dir.mkdir(parents=True, exist_ok=True)

    event = read_event(env)
    meta, body_file = write_issue_files(env, event, tmp_dir)
    existing_comments = load_existing_comments(env, meta)
    idempotency_keys = existing_idempotency_keys(existing_comments, str(meta["comment_id"]))
    plan_json = run_github_funding_plan(env, workspace, meta, body_file, idempotency_keys)
    plan = json.loads(plan_json)
    comment = render_comment(meta, plan)

    plan_file = tmp_dir / "paid-bounty-funding-plan.json"
    plan_file.write_text(plan_json, encoding="utf-8")
    comment_file = tmp_dir / "paid-bounty-funding-comment.md"
    comment_file.write_text(comment, encoding="utf-8")
    append_step_summary(env, comment)

    if env.get("DRY_RUN") == "1":
        stdout.write(plan_json)
        if not plan_json.endswith("\n"):
            stdout.write("\n")
        stdout.write("\n")
        stdout.write(comment)
        return 0

    publish_comment(env, meta, existing_comments, comment)
    return 0


def run_self_test() -> int:
    repo_root = script_repo_root()
    tmp_dir = repo_root / "target" / "tmp"
    tmp_dir.mkdir(parents=True, exist_ok=True)

    issue_body = (repo_root / "examples" / "github-paid-bounty-issue.md").read_text(
        encoding="utf-8"
    )
    issue_body = issue_body.replace("10 USDC", "10 USD").replace(
        "BaseUsdcEscrow", "StripeFiatLedger"
    )
    event = {
        "repository": {"full_name": "agent-bounties/agent-bounties"},
        "issue": {
            "number": 1,
            "title": "[bounty]: Fix CI",
            "html_url": "https://github.com/agent-bounties/agent-bounties/issues/1",
            "body": issue_body,
            "labels": [{"name": "bounty"}],
        },
        "comment": {
            "id": 12345,
            "html_url": "https://github.com/agent-bounties/agent-bounties/issues/1#issuecomment-12345",
            "body": "/agent-bounty fund 5 USD via StripeFiatLedger",
            "user": {"login": "example-agent"},
        },
    }
    event_path = tmp_dir / "github-funding-event.json"
    event_path.write_text(json.dumps(event), encoding="utf-8")

    existing_comments = [
        {
            "id": 999,
            "body": "\n".join(
                [
                    MARKER,
                    "Funding comment id: `99999`",
                    "Idempotency key: github-funding-comment:other",
                ]
            ),
        }
    ]
    existing_path = tmp_dir / "github-funding-existing-comments.json"
    existing_path.write_text(json.dumps(existing_comments), encoding="utf-8")

    env = dict(os.environ)
    env.update(
        {
            "GITHUB_EVENT_PATH": str(event_path),
            "GITHUB_REPOSITORY": "agent-bounties/agent-bounties",
            "GITHUB_WORKSPACE": str(repo_root),
            "RUNNER_TEMP": str(tmp_dir),
            "AGENT_BOUNTIES_FUNDING_COMMENTS_FILE": str(existing_path),
            "AGENT_BOUNTIES_API_BASE_URL": "https://api.agentbounties.example",
            "DRY_RUN": "1",
        }
    )

    buffer = io.StringIO()
    run_from_env(env, buffer)
    output = buffer.getvalue()
    output_path = tmp_dir / "github-funding-comment.out"
    output_path.write_text(output, encoding="utf-8")

    required = [
        MARKER,
        "Agent bounty funding signal: Success",
        "requires operator reconciliation",
        "Idempotency key: github-funding-comment:agent-bounties/agent-bounties:https://github.com/agent-bounties/agent-bounties/issues/1:comment:12345",
        "Stripe Checkout funding handoff",
        "apiBaseUrl=https%3A%2F%2Fapi.agentbounties.example",
        "rail=StripeFiat",
        "Distribution feedback requested",
    ]
    missing = [needle for needle in required if needle not in output]
    if missing:
        raise UserError(f"self-test output missing: {', '.join(missing)}")

    print(f"GitHub funding comment dry-run passed: {output_path}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run a deterministic dry-run using examples/github-paid-bounty-issue.md",
    )
    args = parser.parse_args()

    try:
        if args.self_test:
            return run_self_test()
        return run_from_env(os.environ, sys.stdout)
    except UserError as error:
        print(error, file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
