#!/usr/bin/env python3
"""Plan and publish public claim-comment signals for GitHub bounty issues."""

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
import urllib.parse
from typing import Dict, List, Mapping, Optional, TextIO, Tuple

from reconcile_github_bounty_labels import (
    LabelReconciliationError,
    canonical_records,
    default_http_request,
    fetch_canonical_feeds,
    normalize_api_base_url,
)


MARKER = "<!-- agent-bounties-claim-comment -->"
CLAIM_COMMAND_RE = re.compile(r"(?im)^\s*/(?:agent-bounty\s+)?(claim|attempt)\b")
COMMENT_ID_RE = re.compile(r"Claim comment id:\s*`?([0-9]+)`?")
RESERVATION_RE = re.compile(r"Reservation id:\s*`?([^\s`]+)`?")
CONTRIBUTOR_RE = re.compile(r"Contributor:\s*`?([^\s`]+)`?")
DEFAULT_API_BASE_URL = "https://agent-bounties-api.onrender.com"
STATIC_EARN_PAGE_URL = "https://nspg13.github.io/agent-bounties/earn.html"


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
            raise UserError(f"claim planner output missing field: {field}")
        current = current[part]
    return current


def read_event(env: Mapping[str, str]) -> Dict[str, object]:
    event_path = env.get("GITHUB_EVENT_PATH")
    if not event_path:
        raise UserError("GITHUB_EVENT_PATH is required")
    return json.loads(pathlib.Path(event_path).read_text(encoding="utf-8"))


def write_issue_files(
    env: Mapping[str, str], event: Mapping[str, object], tmp_dir: pathlib.Path
) -> Tuple[Dict[str, object], pathlib.Path]:
    issue = event.get("issue") or {}
    comment = event.get("comment") or {}
    repository = event.get("repository") or {}
    if not isinstance(issue, dict) or not isinstance(comment, dict):
        raise UserError("issue_comment event is required")

    body_file = tmp_dir / "paid-bounty-claim-issue-body.md"
    body_file.write_text(str(issue.get("body") or ""), encoding="utf-8")

    comment_user = comment.get("user") if isinstance(comment.get("user"), dict) else {}
    labels = issue.get("labels") if isinstance(issue.get("labels"), list) else []
    label_names = sorted(
        {
            str(label.get("name") or "").strip().lower()
            for label in labels
            if isinstance(label, dict) and str(label.get("name") or "").strip()
        }
    )
    meta: Dict[str, object] = {
        "repo": env.get("GITHUB_REPOSITORY") or repository.get("full_name") or "",
        "number": issue.get("number"),
        "title": issue.get("title") or "",
        "url": issue.get("html_url") or "",
        "comment_body": comment.get("body") or "",
        "comment_id": str(comment.get("id") or ""),
        "comment_url": comment.get("html_url") or "",
        "contributor_login": comment_user.get("login") or "",
        "labels": label_names,
    }
    missing = [key for key, value in meta.items() if key != "comment_url" and value in ("", None)]
    if missing:
        raise UserError(f"claim comment event missing required metadata: {', '.join(missing)}")
    if not CLAIM_COMMAND_RE.search(str(meta["comment_body"])):
        raise UserError("comment does not contain a /claim, /attempt, or /agent-bounty claim command")

    return meta, body_file


def recovery_reserved_plan(meta: Mapping[str, object]) -> Dict[str, object]:
    details = "\n".join(
        [
            f"Issue: {meta['url']}",
            f"Contributor: {meta['contributor_login']}",
            "Decision: RecoveryReserved",
            "Settlement authority: false",
            "",
            "This issue is marked recovery-reserved after a platform incident.",
            "Do not connect a wallet, sign a claim, or post a solver bond for this round.",
            "The GitHub command is coordination evidence only and created no reservation or failed attempt.",
            "Use a different canonical feed entry with status=claimable and verification_ready=true.",
        ]
    )
    return {
        "ready": False,
        "signal": {
            "decision": "RecoveryReserved",
            "reservation_id": "none",
        },
        "check": {
            "conclusion": "ActionRequired",
            "title": "Bounty is reserved for incident recovery",
            "summary": "Do not sign a claim or post a bond for this recovery-reserved bounty.",
            "text": details,
        },
    }


def load_canonical_claim_records(
    env: Mapping[str, str], repository: str
) -> Tuple[Dict[str, Dict[str, object]], set[Tuple[str, str]]]:
    fixture = env.get("AGENT_BOUNTIES_CLAIM_FEED_FILE")
    if fixture:
        payload = json.loads(pathlib.Path(fixture).read_text(encoding="utf-8"))
        if not isinstance(payload, dict):
            raise UserError("claim feed fixture must be an object")
        full = payload.get("full_feed")
        earning = payload.get("claimable_feed")
        if not isinstance(full, list) or not isinstance(earning, list):
            raise UserError("claim feed fixture requires full_feed and claimable_feed arrays")
        if not all(isinstance(item, dict) for item in [*full, *earning]):
            raise UserError("claim feed fixture entries must be objects")
    else:
        base_url = normalize_api_base_url(
            env.get("AGENT_BOUNTIES_API_BASE_URL") or DEFAULT_API_BASE_URL
        )
        full, earning = fetch_canonical_feeds(
            default_http_request, base_url, "base-mainnet"
        )
    records, earning_pairs = canonical_records(full, earning, repository)
    return records, earning_pairs


def canonical_unavailable_plan(
    meta: Mapping[str, object],
    *,
    status: str,
    contract: Optional[str],
    reason: str,
) -> Dict[str, object]:
    title_by_status = {
        "claimed": "Bounty already has an on-chain solver",
        "submitted": "Bounty is awaiting deterministic verification",
        "paid": "Bounty is already settled",
        "cancelled": "Bounty is cancelled",
        "open": "Bounty is not yet fully funded",
        "missing": "Canonical bounty is not indexed",
        "ambiguous": "Canonical bounty mapping is ambiguous",
        "unavailable": "Canonical bounty state is unavailable",
    }
    contract_line = (
        f"Canonical contract: {contract}"
        if contract
        else "Canonical contract: unavailable"
    )
    details = "\n".join(
        [
            f"Issue: {meta['url']}",
            f"Contributor: {meta['contributor_login']}",
            "Decision: CanonicalStateUnavailable",
            f"Canonical status: {status}",
            contract_line,
            f"Reason: {reason}",
            "Settlement authority: false",
            "",
            "Do not connect a wallet, sign a claim, or post a solver bond for this round.",
            "A future canonical state transition may make a new round claimable; rerun /claim then.",
            "Only a confirmed BountySettled event proves payment.",
        ]
    )
    return {
        "ready": False,
        "signal": {
            "decision": "CanonicalStateUnavailable",
            "reservation_id": "none",
            "bounty_contract": contract,
            "canonical_status": status,
        },
        "check": {
            "conclusion": "ActionRequired",
            "title": title_by_status.get(status, "Bounty is not currently claimable"),
            "summary": "Do not sign a claim or post a bond for the current canonical state.",
            "text": details,
        },
    }


def apply_canonical_claim_state(
    env: Mapping[str, str],
    meta: Mapping[str, object],
    plan: Dict[str, object],
) -> Dict[str, object]:
    signal = plan.get("signal") if isinstance(plan.get("signal"), dict) else None
    if not signal or signal.get("decision") != "OnChainClaimRequired":
        return plan
    try:
        api_base_url = normalize_api_base_url(
            env.get("AGENT_BOUNTIES_API_BASE_URL") or DEFAULT_API_BASE_URL
        )
        records, earning_pairs = load_canonical_claim_records(
            env, str(meta["repo"])
        )
    except (OSError, ValueError, UserError, LabelReconciliationError) as error:
        return canonical_unavailable_plan(
            meta,
            status="unavailable",
            contract=None,
            reason=str(error),
        )

    issue_url = str(meta["url"])
    record = records.get(issue_url)
    if record is None:
        return canonical_unavailable_plan(
            meta,
            status="missing",
            contract=None,
            reason="the full canonical feed has no exact source_url match",
        )
    contract = str(record.get("bounty_contract") or "").lower()
    status = str(record.get("status") or "unknown").lower()
    executable = (
        status == "claimable"
        and record.get("terms_valid") is True
        and record.get("verification_ready") is True
        and (issue_url, contract) in earning_pairs
    )
    if not executable:
        if status != "claimable":
            reason = f"canonical status is {status}; only claimable permits a new solver"
        elif record.get("terms_valid") is not True:
            reason = "the canonical terms record is missing or invalid"
        elif record.get("verification_ready") is not True:
            reason = str(
                record.get("verification_readiness_reason")
                or "the committed verification path is not executable"
            )
        elif (issue_url, contract) not in earning_pairs:
            reason = "the exact contract is absent from the executable earning feed"
        else:
            reason = "the canonical record is not executable"
        return canonical_unavailable_plan(
            meta,
            status=status,
            contract=contract,
            reason=reason,
        )

    handoff = (
        f"{STATIC_EARN_PAGE_URL}?bountyContract={urllib.parse.quote(contract, safe='')}"
        f"&source=github-claim&issue={urllib.parse.quote(issue_url, safe='')}"
    )
    signal.update(
        {
            "bounty_contract": contract,
            "claim_handoff_url": handoff,
            "claim_plan_request": {
                "method": "POST",
                "url": f"{api_base_url}/v1/base/autonomous-bounties/claim-plan",
                "body": {
                    "network": "base-mainnet",
                    "bounty_contract": contract,
                    "solver": "0xYOUR_BASE_WALLET",
                },
                "result": "The planner returns the exact indexed bond and bounded wallet calls.",
            },
            "operator_note": (
                f"Canonical contract: {contract}. The exact record is claimable, "
                "terms-valid, verification-ready, and present in the earning feed. "
                "A GitHub comment does not reserve or claim the round."
            ),
        }
    )
    plan["signal"] = signal
    plan["check"] = {
        "conclusion": "ActionRequired",
        "title": "Autonomous bounty requires an on-chain claim",
        "summary": "Connect the payout wallet, verify the exact indexed bond, and sign the bounded claim.",
        "text": "\n".join(
            [
                f"Issue: {issue_url}",
                f"Contributor: {meta['contributor_login']}",
                "Decision: OnChainClaimRequired",
                f"Canonical contract: {contract}",
                "Canonical status: claimable",
                "Verification ready: true",
                "Settlement authority: false",
                "",
                "The wallet signature and confirmed contract event, not this comment, claim the round.",
                "Only a confirmed BountySettled event proves payment.",
            ]
        ),
    }
    return plan


def load_existing_comments(env: Mapping[str, str], meta: Mapping[str, object]) -> List[Mapping[str, object]]:
    fixture = env.get("AGENT_BOUNTIES_CLAIM_COMMENTS_FILE")
    if fixture:
        value = json.loads(pathlib.Path(fixture).read_text(encoding="utf-8"))
        return value if isinstance(value, list) else []

    if env.get("DRY_RUN") == "1":
        return []

    gh_path = find_executable(["gh", "gh.exe"])
    if not gh_path:
        raise UserError("gh is required to inspect existing claim planner comments")

    comments = subprocess.check_output(
        [gh_path, "api", f"repos/{meta['repo']}/issues/{meta['number']}/comments"],
        env=dict(env),
        text=True,
    )
    value = json.loads(comments)
    return value if isinstance(value, list) else []


def marker_field(pattern: re.Pattern[str], body: str) -> Optional[str]:
    match = pattern.search(body)
    return match.group(1) if match else None


def claim_comment_id(body: str) -> Optional[str]:
    return marker_field(COMMENT_ID_RE, body)


def reservation_id(body: str) -> Optional[str]:
    return marker_field(RESERVATION_RE, body)


def contributor_login(body: str) -> Optional[str]:
    return marker_field(CONTRIBUTOR_RE, body)


def active_claim_login(existing_comments: List[Mapping[str, object]], current_comment_id: str) -> Optional[str]:
    for comment in reversed(existing_comments):
        body = str(comment.get("body") or "")
        if MARKER not in body or claim_comment_id(body) == current_comment_id:
            continue
        if "Agent bounty claim reserved" in body:
            contributor = contributor_login(body)
            if contributor and contributor != "unknown":
                return contributor
    return None


def progress_signal_count(existing_comments: List[Mapping[str, object]], current_reservation_id: Optional[str]) -> int:
    if not current_reservation_id:
        return 0
    count = 0
    for comment in existing_comments:
        body = str(comment.get("body") or "")
        if MARKER in body and reservation_id(body) == current_reservation_id and "Has progress signal: true" in body:
            count += 1
    return count


def run_github_claim_plan(
    env: Mapping[str, str],
    workspace: pathlib.Path,
    meta: Mapping[str, object],
    body_file: pathlib.Path,
    active_login: Optional[str],
    prior_progress_count: int,
) -> str:
    cargo_path = find_executable(["cargo", "cargo.exe"])
    if not cargo_path:
        raise UserError("cargo is required to plan a claim comment")

    command = [
        cargo_path,
        "run",
        "-p",
        "cli",
        "--",
        "github-claim-comment-plan",
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
        "--claim-age-minutes",
        "0",
        "--progress-signal-count",
        str(prior_progress_count),
    ]
    if active_login:
        command.extend(["--active-claim-login", active_login])

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
        raise UserError(f"github-claim-comment-plan failed with exit code {result.returncode}")
    return result.stdout


def render_comment(meta: Mapping[str, object], plan: Mapping[str, object]) -> str:
    conclusion = str(read_json_field(plan, "check.conclusion"))
    title = str(read_json_field(plan, "check.title"))
    summary = str(read_json_field(plan, "check.summary"))
    details = str(read_json_field(plan, "check.text"))
    ready = bool(plan.get("ready"))
    signal = plan.get("signal") if isinstance(plan.get("signal"), dict) else {}
    decision = str(signal.get("decision") or "")
    reservation = str(signal.get("reservation_id") or "none")
    contributor = str(meta.get("contributor_login") or "unknown")
    comment_url = str(meta.get("comment_url") or "").strip()
    comment_ref = comment_url or f"issue comment {meta['comment_id']}"
    wallet_handoff = str(signal.get("claim_handoff_url") or "").strip()
    machine_request = signal.get("claim_plan_request")
    if decision == "RecoveryReserved":
        status_line = (
            "This issue is reserved for incident recovery. The claim command created no "
            "on-chain reservation; do not connect a wallet, sign a claim, or post a bond."
        )
    elif decision == "CanonicalStateUnavailable":
        status_line = (
            "Canonical state does not permit a new claim. Do not connect a wallet, "
            "sign a claim, or post a bond for this round."
        )
    elif decision == "OnChainClaimRequired":
        status_line = (
            "GitHub recorded claim intent but cannot reserve the autonomous contract. "
            "Use the handoff below to connect the payout wallet, review the exact indexed bond, "
            "and sign the bounded claim request."
        )
    elif ready:
        status_line = "This claim is a temporary coordination signal only; it never authorizes bounty acceptance, escrow release, or payout."
    else:
        status_line = "This claim comment needs a concrete progress signal before it should reserve attention."

    claim_actions = []
    if wallet_handoff:
        claim_actions.extend(
            [
                f"**Connect wallet and sign claim:** {wallet_handoff}",
                "",
                "The handoff retrieves canonical state and shows the exact refundable bond before requesting a signature.",
                "",
            ]
        )
    if isinstance(machine_request, dict):
        claim_actions.extend(
            [
                "<details><summary>Machine claim-plan request</summary>",
                "",
                "```json",
                json.dumps(machine_request, indent=2, sort_keys=True),
                "```",
                "",
                "</details>",
                "",
            ]
        )

    return "\n".join(
        [
            MARKER,
            f"### {title}: {conclusion}",
            "",
            summary,
            "",
            status_line,
            "",
            *claim_actions,
            f"Claim comment id: `{meta['comment_id']}`",
            f"Claim comment: {comment_ref}",
            f"Contributor: `{contributor}`",
            f"Reservation id: `{reservation}`",
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
        handle.write("## Agent bounty claim signal\n\n")
        handle.write(comment)
        handle.write("\n")


def publish_comment(
    env: Mapping[str, str],
    meta: Mapping[str, object],
    existing_comments: List[Mapping[str, object]],
    comment: str,
) -> None:
    gh_path = find_executable(["gh", "gh.exe"])
    if not gh_path:
        raise UserError("gh is required to publish the claim planner comment")

    existing_id = None
    for existing in existing_comments:
        body = str(existing.get("body") or "")
        if MARKER in body and claim_comment_id(body) == str(meta["comment_id"]):
            existing_id = existing.get("id")
            break

    if existing_id:
        subprocess.run(
            [
                gh_path,
                "api",
                "--method",
                "PATCH",
                f"repos/{meta['repo']}/issues/comments/{existing_id}",
                "--field",
                f"body={comment}",
            ],
            env=dict(env),
            check=True,
            stdout=subprocess.DEVNULL,
        )
    else:
        comment_file = pathlib.Path(env.get("RUNNER_TEMP") or ".") / "paid-bounty-claim-comment.md"
        comment_file.write_text(comment, encoding="utf-8")
        subprocess.run(
            [
                gh_path,
                "issue",
                "comment",
                str(meta["number"]),
                "--repo",
                str(meta["repo"]),
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
    meta, body_file = write_issue_files(env, event, tmp_dir)
    existing_comments = load_existing_comments(env, meta)
    active_login = active_claim_login(existing_comments, str(meta["comment_id"]))
    prior_progress_count = progress_signal_count(existing_comments, None)
    labels = meta.get("labels") if isinstance(meta.get("labels"), list) else []
    if "recovery-reserved" in labels:
        plan = recovery_reserved_plan(meta)
        plan_json = json.dumps(plan, indent=2, sort_keys=True) + "\n"
    else:
        plan_json = run_github_claim_plan(
            env, workspace, meta, body_file, active_login, prior_progress_count
        )
        plan = json.loads(plan_json)
        plan = apply_canonical_claim_state(env, meta, plan)
        plan_json = json.dumps(plan, indent=2, sort_keys=True) + "\n"
    comment = render_comment(meta, plan)

    plan_file = tmp_dir / "paid-bounty-claim-plan.json"
    plan_file.write_text(plan_json, encoding="utf-8")
    comment_file = tmp_dir / "paid-bounty-claim-comment.md"
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
            "id": 12346,
            "html_url": "https://github.com/agent-bounties/agent-bounties/issues/1#issuecomment-12346",
            "body": "/agent-bounty claim\nPlan: inspect CI logs and open a focused PR with local test output.",
            "user": {"login": "example-agent"},
        },
    }
    event_path = tmp_dir / "github-claim-event.json"
    event_path.write_text(json.dumps(event), encoding="utf-8")

    env = dict(os.environ)
    env.update(
        {
            "GITHUB_EVENT_PATH": str(event_path),
            "GITHUB_REPOSITORY": "agent-bounties/agent-bounties",
            "GITHUB_WORKSPACE": str(repo_root),
            "RUNNER_TEMP": str(tmp_dir),
            "AGENT_BOUNTIES_CLAIM_COMMENTS_FILE": str(tmp_dir / "github-claim-existing-comments.json"),
            "DRY_RUN": "1",
        }
    )
    pathlib.Path(env["AGENT_BOUNTIES_CLAIM_COMMENTS_FILE"]).write_text("[]", encoding="utf-8")

    buffer = io.StringIO()
    run_from_env(env, buffer)
    output = buffer.getvalue()
    output_path = tmp_dir / "github-claim-comment.out"
    output_path.write_text(output, encoding="utf-8")

    required = [
        MARKER,
        "Agent bounty claim reserved",
        "Settlement authority: false",
        "Distribution feedback requested",
        "Reservation id:",
    ]
    missing = [needle for needle in required if needle not in output]
    if missing:
        raise UserError(f"self-test output missing: {', '.join(missing)}")

    if not CLAIM_COMMAND_RE.search("/attempt #187"):
        raise UserError("self-test short autonomous attempt command was not recognized")
    routed = render_comment(
        {
            "comment_id": "1873",
            "comment_url": "",
            "contributor_login": "organic-agent",
        },
        {
            "ready": False,
            "signal": {
                "decision": "OnChainClaimRequired",
                "reservation_id": "routing-only",
                "claim_handoff_url": "https://nspg13.github.io/agent-bounties/earn.html?bountyContract=0x1111111111111111111111111111111111111111",
                "claim_plan_request": {
                    "method": "POST",
                    "url": "https://agent-bounties-api.onrender.com/v1/base/autonomous-bounties/claim-plan",
                    "body": {
                        "network": "base-mainnet",
                        "bounty_contract": "0x1111111111111111111111111111111111111111",
                        "solver": "0xYOUR_BASE_WALLET",
                    },
                },
            },
            "check": {
                "conclusion": "ActionRequired",
                "title": "Autonomous bounty requires an on-chain claim",
                "summary": "GitHub cannot reserve this bounty.",
                "text": "Wait for the canonical funded contract.",
            },
        },
    )
    for required_text in [
        "cannot reserve the autonomous contract",
        "Connect wallet and sign claim",
        "exact indexed bond",
        "Machine claim-plan request",
    ]:
        if required_text not in routed:
            raise UserError(f"self-test autonomous route missing: {required_text}")

    canonical_meta = {
        "repo": "agent-bounties/agent-bounties",
        "url": "https://github.com/agent-bounties/agent-bounties/issues/187",
        "contributor_login": "organic-agent",
        "comment_id": "1874",
        "comment_url": "",
    }
    contract = "0x1111111111111111111111111111111111111111"
    canonical_record = {
        "bounty_contract": contract,
        "status": "claimable",
        "target_amount": "1010000",
        "funded_amount": "1010000",
        "terms_valid": True,
        "verification_ready": True,
        "verification_readiness_reason": "deterministic verifier is executable",
        "terms": {"document": {"source_url": canonical_meta["url"]}},
        "events": [],
    }
    canonical_fixture = tmp_dir / "github-claim-canonical-feed.json"
    canonical_fixture.write_text(
        json.dumps(
            {
                "full_feed": [canonical_record],
                "claimable_feed": [canonical_record],
            }
        ),
        encoding="utf-8",
    )
    canonical_env = {
        "AGENT_BOUNTIES_CLAIM_FEED_FILE": str(canonical_fixture),
    }
    base_autonomous_plan = {
        "ready": False,
        "signal": {
            "decision": "OnChainClaimRequired",
            "reservation_id": "routing-only",
        },
        "check": {
            "conclusion": "ActionRequired",
            "title": "Autonomous bounty requires an on-chain claim",
            "summary": "Canonical state must be resolved.",
            "text": "Static issue metadata is not authority.",
        },
    }
    executable_plan = apply_canonical_claim_state(
        canonical_env,
        canonical_meta,
        json.loads(json.dumps(base_autonomous_plan)),
    )
    executable_comment = render_comment(canonical_meta, executable_plan)
    for required_text in [contract, "Connect wallet and sign claim", "Machine claim-plan request"]:
        if required_text not in executable_comment:
            raise UserError(f"self-test canonical claim route missing: {required_text}")
    if "not published yet" in executable_comment:
        raise UserError("self-test canonical claim route retained stale issue guidance")

    for blocked_status in ["open", "claimed", "submitted", "paid", "cancelled"]:
        blocked_record = json.loads(json.dumps(canonical_record))
        blocked_record["status"] = blocked_status
        blocked_record["funded_amount"] = (
            "0" if blocked_status == "open" else blocked_record["target_amount"]
        )
        if blocked_status in {"claimed", "submitted"}:
            blocked_record["events"] = [
                {
                    "kind": "bounty_claimed",
                    "contract_address": contract,
                    "tx_hash": "0x" + "1" * 64,
                }
            ]
        if blocked_status == "submitted":
            blocked_record["events"].append(
                {
                    "kind": "submission_added",
                    "contract_address": contract,
                    "tx_hash": "0x" + "2" * 64,
                }
            )
        if blocked_status == "paid":
            blocked_record["events"] = [
                {
                    "kind": "bounty_settled",
                    "contract_address": contract,
                    "tx_hash": "0x" + "3" * 64,
                }
            ]
        canonical_fixture.write_text(
            json.dumps({"full_feed": [blocked_record], "claimable_feed": []}),
            encoding="utf-8",
        )
        blocked_plan = apply_canonical_claim_state(
            canonical_env,
            canonical_meta,
            json.loads(json.dumps(base_autonomous_plan)),
        )
        blocked_comment = render_comment(canonical_meta, blocked_plan)
        if "CanonicalStateUnavailable" not in blocked_comment:
            raise UserError(f"self-test did not block canonical {blocked_status} state")
        if "Connect wallet and sign claim" in blocked_comment:
            raise UserError(f"self-test exposed a claim CTA for {blocked_status}")

    unavailable_env = {
        "AGENT_BOUNTIES_CLAIM_FEED_FILE": str(tmp_dir / "missing-claim-feed.json")
    }
    unavailable_plan = apply_canonical_claim_state(
        unavailable_env,
        canonical_meta,
        json.loads(json.dumps(base_autonomous_plan)),
    )
    if unavailable_plan["signal"]["decision"] != "CanonicalStateUnavailable":
        raise UserError("self-test claim feed outage did not fail closed")

    recovery_event = json.loads(json.dumps(event))
    recovery_event["issue"]["body"] = "Funding: pending in this stale issue body."
    recovery_event["issue"]["labels"] = [
        {"name": "bounty"},
        {"name": "funded-live"},
        {"name": "recovery-reserved"},
    ]
    recovery_event["comment"]["id"] = 12347
    recovery_event["comment"]["body"] = "/claim #1"
    event_path.write_text(json.dumps(recovery_event), encoding="utf-8")
    recovery_buffer = io.StringIO()
    run_from_env(env, recovery_buffer)
    recovery_output = recovery_buffer.getvalue()
    for required_text in [
        "Bounty is reserved for incident recovery",
        "do not connect a wallet, sign a claim, or post a bond",
        "Decision: RecoveryReserved",
        "created no reservation or failed attempt",
    ]:
        if required_text not in recovery_output:
            raise UserError(f"self-test recovery guard missing: {required_text}")
    for forbidden_text in ["Connect wallet and sign claim", "Machine claim-plan request"]:
        if forbidden_text in recovery_output:
            raise UserError(f"self-test recovery guard exposed claim action: {forbidden_text}")

    print(f"GitHub claim comment dry-run passed: {output_path}")
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
