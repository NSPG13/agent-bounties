#!/usr/bin/env python3
"""Plan and publish the sticky paid-bounty validation comment for GitHub issues."""

from __future__ import annotations

import argparse
import io
import json
import os
import pathlib
import subprocess
import sys
from typing import Dict, List, Mapping, Optional, TextIO, Tuple

from _shared.github_actions import (
    append_step_summary as append_github_summary,
    cargo_body_path,
    find_executable,
    json_field,
    publish_issue_comment,
    repo_root,
)


MARKER = "<!-- agent-bounties-plan -->"
DISCOVERY_MARKERS = (
    "<!-- agent-bounties/github-discovery-v1:start -->",
    "<!-- agent-bounties/github-discovery-archive-v1:start -->",
)
# These labels are set by repository maintainers/reconciliation, never the
# ordinary issue form. A public body marker alone is not trusted evidence.
DISCOVERY_LIFECYCLE_LABELS = frozenset({
    "funding-needed", "funded-live", "claimed-live", "in-progress",
    "verification-pending", "verification-unavailable", "refund-available",
    "cancelled", "expired", "settled-paid",
})


class UserError(RuntimeError):
    pass


def script_repo_root() -> pathlib.Path:
    return repo_root(__file__)


def read_json_field(value: object, field: str) -> object:
    return json_field(value, field, UserError, "planner output missing field: {field}")


def read_issue_event(env: Mapping[str, str]) -> Dict[str, object]:
    event_path = env.get("GITHUB_EVENT_PATH")
    if not event_path:
        raise UserError("GITHUB_EVENT_PATH is required")
    event = json.loads(pathlib.Path(event_path).read_text(encoding="utf-8"))
    if not isinstance(event, dict) or not isinstance(event.get("issue"), dict):
        raise UserError("GitHub event must contain an issue object")
    return event


def skip_intake_reason(issue: Mapping[str, object]) -> Optional[str]:
    if issue.get("state") == "closed":
        return "closed issue"
    labels = {
        label.get("name") for label in issue.get("labels") or []
        if isinstance(label, dict) and isinstance(label.get("name"), str)
    }
    if "funded-live" in labels:
        return "already funded canonical bounty"
    if (
        "payments" in labels
        and labels & DISCOVERY_LIFECYCLE_LABELS
        and any(marker in str(issue.get("body") or "") for marker in DISCOVERY_MARKERS)
    ):
        return "canonical bounty discovery mirror"
    return None


def write_issue_files(
    env: Mapping[str, str], tmp_dir: pathlib.Path, event: Optional[Mapping[str, object]] = None
) -> Tuple[Dict[str, object], pathlib.Path]:
    if event is None:
        event = read_issue_event(env)
    issue = event.get("issue") or {}
    repository = event.get("repository") or {}

    body_file = tmp_dir / "paid-bounty-issue-body.md"
    body_file.write_text(issue.get("body") or "", encoding="utf-8")

    meta: Dict[str, object] = {
        "repo": env.get("GITHUB_REPOSITORY") or repository.get("full_name") or "",
        "number": issue.get("number"),
        "title": issue.get("title") or "",
        "url": issue.get("html_url") or "",
    }
    missing = [key for key, value in meta.items() if value in ("", None)]
    if missing:
        raise UserError(f"issue event missing required metadata: {', '.join(missing)}")

    meta_file = tmp_dir / "paid-bounty-issue-meta.json"
    meta_file.write_text(json.dumps(meta), encoding="utf-8")
    return meta, body_file


def run_github_plan(
    env: Mapping[str, str],
    workspace: pathlib.Path,
    meta: Mapping[str, object],
    body_file: pathlib.Path,
) -> str:
    cargo_path = find_executable(["cargo", "cargo.exe"])
    if not cargo_path:
        raise UserError("cargo is required to plan a paid-bounty issue")

    body_arg = cargo_body_path(body_file, cargo_path)
    result = subprocess.run(
        [
            cargo_path,
            "run",
            "-p",
            "cli",
            "--",
            "github-plan",
            "--repository",
            str(meta["repo"]),
            "--issue-url",
            str(meta["url"]),
            "--title",
            str(meta["title"]),
            "--body-file",
            body_arg,
        ],
        cwd=workspace,
        env=dict(env),
        text=True,
        stdout=subprocess.PIPE,
        stderr=None,
        check=False,
    )
    if result.returncode != 0:
        raise UserError(f"github-plan failed with exit code {result.returncode}")
    return result.stdout


def render_comment(plan: Mapping[str, object]) -> str:
    conclusion = str(read_json_field(plan, "check.conclusion"))
    title = str(read_json_field(plan, "check.title"))
    summary = str(read_json_field(plan, "check.summary"))
    details = str(read_json_field(plan, "check.text"))
    ready = conclusion == "Success"
    if ready and title == "Autonomous bounty metadata ready":
        status_line = "This metadata is valid. Canonical contract events, not GitHub, control funding and claimability."
    elif ready:
        status_line = "This issue can be routed into a funded bounty."
    else:
        status_line = "This issue needs edits before it can be routed into a funded bounty."
    return "\n".join(
        [
            MARKER,
            f"### Agent bounty validation: {conclusion}",
            "",
            summary,
            "",
            status_line,
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
    append_github_summary(env, "Agent bounty validation", comment)


def publish_comment(env: Mapping[str, str], meta: Mapping[str, object], comment: str) -> None:
    publish_issue_comment(
        env,
        meta["repo"],
        meta["number"],
        MARKER,
        comment,
        "paid-bounty-comment.md",
        "gh is required to publish the paid-bounty validation comment",
        UserError,
    )


def run_from_env(env: Mapping[str, str], stdout: TextIO) -> int:
    event = read_issue_event(env)
    reason = skip_intake_reason(event["issue"])
    if reason:
        stdout.write(f"Skipped paid-bounty form validation: {reason}.\n")
        return 0
    repo_root = script_repo_root()
    workspace = pathlib.Path(env.get("GITHUB_WORKSPACE") or repo_root).resolve()
    tmp_dir = pathlib.Path(env.get("RUNNER_TEMP") or workspace / "target" / "tmp").resolve()
    tmp_dir.mkdir(parents=True, exist_ok=True)

    meta, body_file = write_issue_files(env, tmp_dir, event)
    plan_json = run_github_plan(env, workspace, meta, body_file)
    plan_file = tmp_dir / "paid-bounty-plan.json"
    plan_file.write_text(plan_json, encoding="utf-8")

    plan = json.loads(plan_json)
    comment = render_comment(plan)
    comment_file = tmp_dir / "paid-bounty-comment.md"
    comment_file.write_text(comment, encoding="utf-8")
    append_step_summary(env, comment)

    if env.get("DRY_RUN") == "1":
        stdout.write(plan_json)
        if not plan_json.endswith("\n"):
            stdout.write("\n")
        stdout.write("\n")
        stdout.write(comment)
        return 0

    publish_comment(env, meta, comment)
    return 0


def run_self_test() -> int:
    repo_root = script_repo_root()
    tmp_dir = repo_root / "target" / "tmp"
    tmp_dir.mkdir(parents=True, exist_ok=True)

    body = (repo_root / "examples" / "github-paid-bounty-issue.md").read_text(encoding="utf-8")
    event = {
        "repository": {"full_name": "agent-bounties/agent-bounties"},
        "issue": {
            "number": 1,
            "title": "[bounty]: Fix CI",
            "html_url": "https://github.com/agent-bounties/agent-bounties/issues/1",
            "body": body,
        },
    }
    event_path = tmp_dir / "github-issue-event.json"
    event_path.write_text(json.dumps(event), encoding="utf-8")

    env = dict(os.environ)
    env.update(
        {
            "GITHUB_EVENT_PATH": str(event_path),
            "GITHUB_REPOSITORY": "agent-bounties/agent-bounties",
            "GITHUB_WORKSPACE": str(repo_root),
            "RUNNER_TEMP": str(tmp_dir),
            "DRY_RUN": "1",
        }
    )

    buffer = io.StringIO()
    run_from_env(env, buffer)
    output = buffer.getvalue()
    output_path = tmp_dir / "github-issue-plan-comment.out"
    output_path.write_text(output, encoding="utf-8")

    required = [MARKER, "Agent bounty validation: Success", "This issue can be routed into a funded bounty."]
    missing = [needle for needle in required if needle not in output]
    if missing:
        raise UserError(f"self-test output missing: {', '.join(missing)}")

    autonomous = render_comment(
        {
            "check": {
                "conclusion": "Success",
                "title": "Autonomous bounty metadata ready",
                "summary": "Canonical contract events control funding and claims.",
                "text": "Amount: 1 USDC\nCanonical contract: funding pending.",
            }
        }
    )
    if "Canonical contract events, not GitHub" not in autonomous:
        raise UserError("self-test autonomous metadata implied that funding was ready")
    if "can be routed into a funded bounty" in autonomous:
        raise UserError("self-test autonomous metadata used legacy funding-ready copy")

    print(f"GitHub issue plan comment dry-run passed: {output_path}")
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
