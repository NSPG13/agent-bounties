#!/usr/bin/env python3
"""Plan and publish the sticky accepted-proof comment for GitHub issues."""

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
import urllib.error
import urllib.request
from typing import Dict, Mapping, Optional, TextIO, Tuple


MARKER = "<!-- agent-bounties-proof -->"
PROOF_COMMAND_RE = re.compile(
    r"(?im)^\s*/agent-bounty\s+proof\s+"
    r"([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})"
    r"(?:\s+(\S+))?\s*$"
)


class UserError(RuntimeError):
    pass


def script_repo_root() -> pathlib.Path:
    return pathlib.Path(__file__).resolve().parents[1]


def find_executable(name: str) -> Optional[str]:
    return shutil.which(name) or shutil.which(f"{name}.exe")


def normalize_base_url(value: str) -> str:
    trimmed = value.strip()
    if not trimmed:
        raise UserError("AGENT_BOUNTIES_API_BASE_URL or api_base_url input is required")
    return trimmed.rstrip("/")


def read_json_field(value: object, field: str) -> object:
    current = value
    for part in field.split("."):
        if not isinstance(current, dict) or part not in current:
            raise UserError(f"proof planner output missing field: {field}")
        current = current[part]
    return current


def parse_proof_command(body: str) -> Tuple[Optional[str], Optional[str]]:
    match = PROOF_COMMAND_RE.search(body or "")
    if not match:
        return None, None
    return match.group(1), match.group(2)


def read_event(env: Mapping[str, str]) -> Dict[str, object]:
    event_path = env.get("GITHUB_EVENT_PATH")
    if not event_path:
        return {}
    return json.loads(pathlib.Path(event_path).read_text(encoding="utf-8"))


def workflow_input(event: Mapping[str, object], name: str) -> str:
    inputs = event.get("inputs")
    if isinstance(inputs, dict):
        value = inputs.get(name)
        if value is not None:
            return str(value).strip()
    return ""


def resolve_request(env: Mapping[str, str], event: Mapping[str, object]) -> Dict[str, str]:
    repository = env.get("GITHUB_REPOSITORY", "").strip()
    event_repository = event.get("repository")
    if isinstance(event_repository, dict):
        repository = repository or str(event_repository.get("full_name") or "").strip()

    issue_number = env.get("ISSUE_NUMBER", "").strip() or workflow_input(event, "issue_number")
    issue = event.get("issue")
    if not issue_number and isinstance(issue, dict):
        issue_number = str(issue.get("number") or "").strip()

    proof_id = env.get("PROOF_ID", "").strip() or workflow_input(event, "proof_id")
    settlement_url = env.get("SETTLEMENT_URL", "").strip() or workflow_input(event, "settlement_url")

    comment = event.get("comment")
    if isinstance(comment, dict):
        parsed_proof_id, parsed_settlement_url = parse_proof_command(str(comment.get("body") or ""))
        proof_id = proof_id or (parsed_proof_id or "")
        settlement_url = settlement_url or (parsed_settlement_url or "")

    api_base_url = (
        env.get("AGENT_BOUNTIES_API_BASE_URL", "").strip()
        or env.get("API_BASE_URL", "").strip()
        or workflow_input(event, "api_base_url")
    )

    missing = []
    if not repository:
        missing.append("GITHUB_REPOSITORY")
    if not issue_number:
        missing.append("ISSUE_NUMBER or workflow input issue_number")
    if not proof_id:
        missing.append("PROOF_ID, workflow input proof_id, or /agent-bounty proof command")
    if not api_base_url and not env.get("AGENT_BOUNTIES_PROOF_PLAN_FILE"):
        missing.append("AGENT_BOUNTIES_API_BASE_URL or workflow input api_base_url")
    if missing:
        raise UserError(f"missing required proof comment metadata: {', '.join(missing)}")

    return {
        "repo": repository,
        "issue_number": issue_number,
        "proof_id": proof_id,
        "api_base_url": api_base_url,
        "settlement_url": settlement_url,
    }


def fetch_plan(env: Mapping[str, str], request: Mapping[str, str]) -> Dict[str, object]:
    plan_file = env.get("AGENT_BOUNTIES_PROOF_PLAN_FILE")
    if plan_file:
        return json.loads(pathlib.Path(plan_file).read_text(encoding="utf-8"))

    payload = json.dumps(
        {
            "proof_id": request["proof_id"],
            "settlement_url": request["settlement_url"] or None,
        }
    ).encode("utf-8")
    url = f"{normalize_base_url(request['api_base_url'])}/v1/github/proof-comment-plan-from-proof"
    http_request = urllib.request.Request(
        url,
        data=payload,
        method="POST",
        headers={
            "content-type": "application/json",
            "accept": "application/json",
            "user-agent": "agent-bounties-github-proof-comment",
        },
    )
    try:
        with urllib.request.urlopen(http_request, timeout=30) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        raise UserError(f"proof planner returned HTTP {error.code}: {body}") from error
    except urllib.error.URLError as error:
        raise UserError(f"proof planner request failed: {error}") from error


def render_comment(plan: Mapping[str, object]) -> str:
    markdown = str(read_json_field(plan, "markdown"))
    fingerprint = str(read_json_field(plan, "fingerprint"))
    proof_url = str(read_json_field(plan, "comment.proof_url"))
    bounty_id = str(read_json_field(plan, "comment.bounty_id"))
    return "\n".join(
        [
            MARKER,
            "### Agent bounty proof accepted",
            "",
            markdown,
            "",
            f"Proof fingerprint: `{fingerprint}`",
            f"Proof record: {proof_url}",
            f"Bounty id: `{bounty_id}`",
            "",
        ]
    )


def append_step_summary(env: Mapping[str, str], comment: str) -> None:
    summary_path = env.get("GITHUB_STEP_SUMMARY")
    if not summary_path:
        return
    with pathlib.Path(summary_path).open("a", encoding="utf-8") as handle:
        handle.write("## Agent bounty proof\n\n")
        handle.write(comment)
        handle.write("\n")


def publish_comment(env: Mapping[str, str], request: Mapping[str, str], comment: str) -> None:
    gh_path = find_executable("gh")
    if not gh_path:
        raise UserError("gh is required to publish the accepted-proof comment")

    repo = request["repo"]
    issue_number = request["issue_number"]
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
        comment_file = pathlib.Path(env.get("RUNNER_TEMP") or ".") / "paid-bounty-proof-comment.md"
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

    event = read_event(env)
    request = resolve_request(env, event)
    plan = fetch_plan(env, request)
    comment = render_comment(plan)

    plan_file = tmp_dir / "paid-bounty-proof-plan.json"
    plan_file.write_text(json.dumps(plan, indent=2, sort_keys=True), encoding="utf-8")
    comment_file = tmp_dir / "paid-bounty-proof-comment.md"
    comment_file.write_text(comment, encoding="utf-8")
    append_step_summary(env, comment)

    if env.get("DRY_RUN") == "1":
        stdout.write(json.dumps(plan, indent=2, sort_keys=True))
        stdout.write("\n\n")
        stdout.write(comment)
        return 0

    publish_comment(env, request, comment)
    return 0


def run_self_test() -> int:
    repo_root = script_repo_root()
    tmp_dir = repo_root / "target" / "tmp"
    tmp_dir.mkdir(parents=True, exist_ok=True)

    proof_id = "00000000-0000-0000-0000-000000000123"
    command = f"/agent-bounty proof {proof_id} https://basescan.org/tx/0xabc"
    event = {
        "repository": {"full_name": "agent-bounties/agent-bounties"},
        "issue": {"number": 1},
        "comment": {"body": command},
    }
    event_path = tmp_dir / "github-proof-event.json"
    event_path.write_text(json.dumps(event), encoding="utf-8")

    plan = {
        "comment": {
            "bounty_id": "00000000-0000-0000-0000-000000000001",
            "proof_url": f"https://agentbounties.local/public/proofs/{proof_id}",
            "verifier_summary": "JsonSchema: artifact accepted",
            "settlement_url": "https://basescan.org/tx/0xabc",
        },
        "markdown": (
            "Agent bounty completed.\n\n"
            f"Proof: https://agentbounties.local/public/proofs/{proof_id}\n\n"
            "Verifier: JsonSchema: artifact accepted\n\n"
            "Bounty: `00000000-0000-0000-0000-000000000001`\n\n"
            "Settlement: https://basescan.org/tx/0xabc"
        ),
        "fingerprint": "f" * 64,
        "check": {
            "title": "Agent bounty proof accepted",
            "summary": "Proof recorded for bounty `00000000-0000-0000-0000-000000000001`.",
            "text": "proof text",
            "conclusion": "Success",
        },
    }
    plan_path = tmp_dir / "github-proof-plan.json"
    plan_path.write_text(json.dumps(plan), encoding="utf-8")

    env = dict(os.environ)
    env.update(
        {
            "GITHUB_EVENT_PATH": str(event_path),
            "GITHUB_REPOSITORY": "agent-bounties/agent-bounties",
            "GITHUB_WORKSPACE": str(repo_root),
            "RUNNER_TEMP": str(tmp_dir),
            "AGENT_BOUNTIES_API_BASE_URL": "https://agentbounties.local",
            "AGENT_BOUNTIES_PROOF_PLAN_FILE": str(plan_path),
            "DRY_RUN": "1",
        }
    )

    buffer = io.StringIO()
    run_from_env(env, buffer)
    output = buffer.getvalue()
    output_path = tmp_dir / "github-proof-comment.out"
    output_path.write_text(output, encoding="utf-8")

    required = [
        MARKER,
        "Agent bounty proof accepted",
        f"Proof: https://agentbounties.local/public/proofs/{proof_id}",
        "Proof fingerprint: `ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff`",
    ]
    missing = [needle for needle in required if needle not in output]
    if missing:
        raise UserError(f"self-test output missing: {', '.join(missing)}")

    print(f"GitHub proof comment dry-run passed: {output_path}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run a deterministic dry-run using a fixture proof planner response",
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
