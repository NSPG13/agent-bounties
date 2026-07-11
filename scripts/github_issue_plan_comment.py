#!/usr/bin/env python3
"""Plan and publish the sticky paid-bounty validation comment for GitHub issues."""

from __future__ import annotations

import argparse
import io
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
from typing import Dict, List, Mapping, Optional, TextIO, Tuple


MARKER = "<!-- agent-bounties-plan -->"


class UserError(RuntimeError):
    pass


def script_repo_root() -> pathlib.Path:
    return pathlib.Path(__file__).resolve().parents[1]


def find_executable(names: List[str]) -> Optional[str]:
    for name in names:
        path = shutil.which(name)
        if path:
            return path
    return None


def is_windows_executable(path: str) -> bool:
    return path.lower().endswith(".exe")


def convert_posix_path_for_windows_tool(path: pathlib.Path) -> str:
    path_text = str(path)
    if not path_text.startswith("/"):
        return path_text

    for converter, args in (("cygpath", ["-w"]), ("wslpath", ["-w"])):
        converter_path = shutil.which(converter)
        if not converter_path:
            continue
        try:
            return subprocess.check_output(
                [converter_path, *args, path_text],
                text=True,
                stderr=subprocess.DEVNULL,
            ).strip()
        except (OSError, subprocess.CalledProcessError):
            continue

    match = re.match(r"^/mnt/([a-zA-Z])/(.*)$", path_text)
    if match:
        drive, rest = match.groups()
        rest_windows = rest.replace("/", "\\")
        return f"{drive.upper()}:\\{rest_windows}"

    return path_text


def cargo_body_path(path: pathlib.Path, cargo_path: str) -> str:
    if is_windows_executable(cargo_path):
        return convert_posix_path_for_windows_tool(path)
    return str(path)


def read_json_field(value: object, field: str) -> object:
    current = value
    for part in field.split("."):
        if not isinstance(current, dict) or part not in current:
            raise UserError(f"planner output missing field: {field}")
        current = current[part]
    return current


def write_issue_files(env: Mapping[str, str], tmp_dir: pathlib.Path) -> Tuple[Dict[str, object], pathlib.Path]:
    event_path = env.get("GITHUB_EVENT_PATH")
    if not event_path:
        raise UserError("GITHUB_EVENT_PATH is required")

    event = json.loads(pathlib.Path(event_path).read_text(encoding="utf-8"))
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
    summary_path = env.get("GITHUB_STEP_SUMMARY")
    if not summary_path:
        return
    with pathlib.Path(summary_path).open("a", encoding="utf-8") as handle:
        handle.write("## Agent bounty validation\n\n")
        handle.write(comment)
        handle.write("\n")


def publish_comment(env: Mapping[str, str], meta: Mapping[str, object], comment: str) -> None:
    gh_path = find_executable(["gh", "gh.exe"])
    if not gh_path:
        raise UserError("gh is required to publish the paid-bounty validation comment")

    repo = str(meta["repo"])
    issue_number = str(meta["number"])
    comments = subprocess.check_output(
        [gh_path, "api", f"repos/{repo}/issues/{issue_number}/comments"],
        env=dict(env),
        text=True,
    )
    existing_id = None
    for existing in json.loads(comments):
        body = existing.get("body") or ""
        if MARKER in body:
            existing_id = existing.get("id")
            break

    if existing_id:
        subprocess.run(
            [
                gh_path,
                "api",
                "--method",
                "PATCH",
                f"repos/{repo}/issues/comments/{existing_id}",
                "--field",
                f"body={comment}",
            ],
            env=dict(env),
            check=True,
            stdout=subprocess.DEVNULL,
        )
    else:
        comment_file = pathlib.Path(env.get("RUNNER_TEMP") or ".") / "paid-bounty-comment.md"
        comment_file.write_text(comment, encoding="utf-8")
        subprocess.run(
            [
                gh_path,
                "issue",
                "comment",
                issue_number,
                "--repo",
                repo,
                "--body-file",
                str(comment_file),
            ],
            env=dict(env),
            check=True,
            stdout=subprocess.DEVNULL,
        )


def run_from_env(env: Mapping[str, str], stdout: TextIO) -> int:
    repo_root = script_repo_root()
    workspace = pathlib.Path(env.get("GITHUB_WORKSPACE") or repo_root).resolve()
    tmp_dir = pathlib.Path(env.get("RUNNER_TEMP") or workspace / "target" / "tmp").resolve()
    tmp_dir.mkdir(parents=True, exist_ok=True)

    meta, body_file = write_issue_files(env, tmp_dir)
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
