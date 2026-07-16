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
EVM_ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")


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


def native_claim_request(
    signal: Mapping[str, object], api_base_url: str, contract: str
) -> Dict[str, object]:
    existing = (
        signal.get("claim_plan_request")
        if isinstance(signal.get("claim_plan_request"), dict)
        else {}
    )
    existing_body = (
        existing.get("body") if isinstance(existing.get("body"), dict) else {}
    )
    solver_wallet = str(
        existing_body.get("solver_wallet") or "0xYOUR_PUBLIC_BASE_WALLET"
    )
    return {
        "method": "POST",
        "url": f"{api_base_url}/v1/base/autonomous-bounties/claims",
        "body": {
            "idempotency_key": str(
                existing_body.get("idempotency_key")
                or signal.get("reservation_id")
                or "github-claim-comment"
            ),
            "network": "base-mainnet",
            "bounty_contract": contract,
            "solver_wallet": solver_wallet,
            "request_bond_sponsorship": True,
            "source": "github",
        },
        "result": (
            "The first response reserves an exclusive candidate or waitlist position and "
            "returns the exact indexed bond plus wallet_request. Send wallet_request to the "
            "solver wallet once, then copy its unchanged 65-byte result into "
            "next_request.body.wallet_signature. Only confirmed canonical BountyClaimed owns "
            "the round."
        ),
    }


def load_native_claim_handoff(
    env: Mapping[str, str], request: Mapping[str, object]
) -> Tuple[int, object]:
    fixture = env.get("AGENT_BOUNTIES_CLAIM_HANDOFF_FILE")
    if fixture:
        payload = json.loads(pathlib.Path(fixture).read_text(encoding="utf-8"))
        if not isinstance(payload, dict):
            raise UserError("claim handoff fixture must be an object")
        return int(payload.get("status") or 0), payload.get("body")

    response = default_http_request(
        str(request["method"]),
        str(request["url"]),
        request.get("body"),
        {"Accept": "application/json"},
    )
    return response.status, response.body


def summarize_native_claim_handoff(
    status: int,
    payload: object,
    *,
    contract: str,
    solver_wallet: str,
) -> Tuple[Optional[Dict[str, object]], Optional[Dict[str, object]]]:
    if not isinstance(payload, dict):
        return None, {
            "http_status": status,
            "error": "claim_handoff_unreadable",
            "next_action": "Replay the machine claim request later; do not sign anything from this response.",
        }
    if status not in {200, 202}:
        return None, {
            "http_status": status,
            "schema_version": payload.get("schema_version"),
            "state": payload.get("state"),
            "failed_transition": payload.get("failed_transition"),
            "error": payload.get("error") or "claim_handoff_failed",
            "next_action": payload.get("next_action")
            or "Replay the same machine request after the reported condition is resolved.",
        }

    candidate = payload.get("candidate")
    if not isinstance(candidate, dict):
        raise UserError("hosted claim handoff omitted candidate state")
    if str(candidate.get("bounty_contract") or "").lower() != contract.lower():
        raise UserError("hosted claim handoff returned a different bounty contract")
    if str(candidate.get("solver_wallet") or "").lower() != solver_wallet.lower():
        raise UserError("hosted claim handoff returned a different solver wallet")

    handoff = {
        "http_status": status,
        "schema_version": payload.get("schema_version"),
        "candidate": {
            "id": candidate.get("id"),
            "status": candidate.get("status"),
            "exclusive_until": candidate.get("exclusive_until"),
        },
        "waitlist_position": payload.get("waitlist_position"),
        "claim_bond": payload.get("claim_bond"),
        "sponsorship_requested": payload.get("sponsorship_requested"),
        "sponsorship_available": payload.get("sponsorship_available"),
        "sponsorship_protocol": payload.get("sponsorship_protocol"),
        "sponsor_contract": payload.get("sponsor_contract"),
        "wallet_request": payload.get("wallet_request"),
        "next_request": payload.get("next_request"),
        "next_action": payload.get("next_action"),
        "evidence_boundary": payload.get("evidence_boundary"),
    }
    return handoff, None


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
    request = native_claim_request(signal, api_base_url, contract)
    signal.update(
        {
            "bounty_contract": contract,
            "claim_handoff_url": handoff,
            "claim_plan_request": request,
            "operator_note": (
                f"Canonical contract: {contract}. The exact record is claimable, "
                "terms-valid, verification-ready, and present in the earning feed. "
                "A hosted candidate is coordination state; only canonical BountyClaimed "
                "owns the round."
            ),
        }
    )
    solver_wallet = str(request["body"]["solver_wallet"])
    if EVM_ADDRESS_RE.fullmatch(solver_wallet):
        try:
            status_code, response_body = load_native_claim_handoff(env, request)
            response, problem = summarize_native_claim_handoff(
                status_code,
                response_body,
                contract=contract,
                solver_wallet=solver_wallet,
            )
            if response is not None:
                signal["claim_handoff_response"] = response
            if problem is not None:
                signal["claim_handoff_problem"] = problem
        except (OSError, ValueError, UserError, LabelReconciliationError) as error:
            signal["claim_handoff_problem"] = {
                "error": "claim_handoff_unavailable",
                "next_action": (
                    f"{error}. Replay the published machine request with the same "
                    "idempotency_key; do not sign an unverified payload."
                ),
            }
    plan["signal"] = signal
    handoff_response = signal.get("claim_handoff_response")
    wallet_request_ready = isinstance(handoff_response, dict) and isinstance(
        handoff_response.get("wallet_request"), dict
    )
    plan["check"] = {
        "conclusion": "ActionRequired",
        "title": (
            "Exact wallet signature requested"
            if wallet_request_ready
            else "Autonomous bounty requires an on-chain claim"
        ),
        "summary": (
            "Send the exact wallet_request to the payout wallet once, then replay its unchanged signature."
            if wallet_request_ready
            else "Provide a public Base payout wallet or run the published machine request."
        ),
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
    handoff_response = signal.get("claim_handoff_response")
    handoff_problem = signal.get("claim_handoff_problem")
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
        if isinstance(handoff_response, dict) and isinstance(
            handoff_response.get("wallet_request"), dict
        ):
            status_line = (
                "The hosted service reserved this candidate and returned the exact wallet "
                "request. This is not yet an on-chain claim: sign once, replay the unchanged "
                "65-byte result privately through next_request, and wait for confirmed "
                "BountyClaimed."
            )
        elif isinstance(handoff_response, dict):
            status_line = (
                "The hosted service recorded the candidate state below but did not request a "
                "signature. Follow next_action; do not sign while waitlisted or after a "
                "terminal state."
            )
        elif isinstance(handoff_problem, dict):
            status_line = (
                "The hosted handoff did not reach signature-ready state. Follow the exact "
                "failed transition below or replay the same machine request; do not invent or "
                "post a signature."
            )
        else:
            status_line = (
                "GitHub recorded claim intent but no valid public payout wallet was supplied. "
                "Add `wallet: 0xYOUR_PUBLIC_BASE_ADDRESS` to a new `/claim` comment; never post "
                "a private key or seed phrase."
            )
    elif ready:
        status_line = "This claim is a temporary coordination signal only; it never authorizes bounty acceptance, escrow release, or payout."
    else:
        status_line = "This claim comment needs a concrete progress signal before it should reserve attention."

    claim_actions = []
    if isinstance(machine_request, dict):
        claim_actions.extend(
            [
                "**Machine claim request:**",
                "",
                "```json",
                json.dumps(machine_request, indent=2, sort_keys=True),
                "```",
                "",
            ]
        )
    if isinstance(handoff_response, dict):
        claim_actions.extend(
            [
                "**Hosted claim handoff:**",
                "",
                "```json",
                json.dumps(handoff_response, indent=2, sort_keys=True),
                "```",
                "",
            ]
        )
    if isinstance(handoff_problem, dict):
        claim_actions.extend(
            [
                "**Hosted handoff problem:**",
                "",
                "```json",
                json.dumps(handoff_problem, indent=2, sort_keys=True),
                "```",
                "",
            ]
        )
    if wallet_handoff:
        claim_actions.extend(
            [
                f"Optional browser fallback: {wallet_handoff}",
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
            "Feedback (never eligibility or payment authority): reply with `discovery_source`, "
            "`participation_reason`, and `improvement_feedback`.",
            "",
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
                    "url": "https://agent-bounties-api.onrender.com/v1/base/autonomous-bounties/claims",
                    "body": {
                        "idempotency_key": "routing-only",
                        "network": "base-mainnet",
                        "bounty_contract": "0x1111111111111111111111111111111111111111",
                        "solver_wallet": "0xYOUR_PUBLIC_BASE_WALLET",
                        "request_bond_sponsorship": True,
                        "source": "github",
                    },
                    "result": "Returns the exact indexed bond and wallet_request.",
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
        "no valid public payout wallet was supplied",
        "Machine claim request",
        "exact indexed bond",
        "0xYOUR_PUBLIC_BASE_WALLET",
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
    for required_text in [contract, "Machine claim request", "0xYOUR_PUBLIC_BASE_WALLET"]:
        if required_text not in executable_comment:
            raise UserError(f"self-test canonical claim route missing: {required_text}")
    if "not published yet" in executable_comment:
        raise UserError("self-test canonical claim route retained stale issue guidance")

    solver_wallet = "0x2222222222222222222222222222222222222222"
    handoff_fixture = tmp_dir / "github-claim-native-handoff.json"
    handoff_fixture.write_text(
        json.dumps(
            {
                "status": 200,
                "body": {
                    "schema_version": "agent-bounties/agent-native-claim-v1",
                    "candidate": {
                        "id": "11111111-1111-4111-8111-111111111111",
                        "status": "authorization_ready",
                        "exclusive_until": "2026-07-16T14:00:00Z",
                        "bounty_contract": contract,
                        "solver_wallet": solver_wallet,
                    },
                    "waitlist_position": None,
                    "claim_bond": "10000",
                    "sponsorship_requested": True,
                    "sponsorship_available": True,
                    "sponsorship_protocol": "agent-bounties/atomic-claim-sponsor-v1",
                    "sponsor_contract": "0x3333333333333333333333333333333333333333",
                    "wallet_request": {
                        "method": "eth_signTypedData_v4",
                        "params": [solver_wallet, "{\"domain\":{}}"],
                    },
                    "next_request": {
                        "method": "POST",
                        "url": f"{DEFAULT_API_BASE_URL}/v1/base/autonomous-bounties/claims",
                        "body": {
                            "wallet_signature": "<replace with the unchanged 0x-prefixed result from wallet_request>"
                        },
                    },
                    "next_action": "Sign once and replay wallet_signature unchanged.",
                    "evidence_boundary": "Only confirmed canonical BountyClaimed owns the round.",
                },
            }
        ),
        encoding="utf-8",
    )
    wallet_plan = json.loads(json.dumps(base_autonomous_plan))
    wallet_plan["signal"]["claim_plan_request"] = {
        "method": "POST",
        "url": f"{DEFAULT_API_BASE_URL}/v1/base/autonomous-bounties/claims",
        "body": {
            "idempotency_key": "github-wallet-self-test",
            "network": "base-mainnet",
            "bounty_contract": contract,
            "solver_wallet": solver_wallet,
            "request_bond_sponsorship": True,
            "source": "github",
        },
    }
    wallet_env = {
        **canonical_env,
        "AGENT_BOUNTIES_CLAIM_HANDOFF_FILE": str(handoff_fixture),
    }
    wallet_plan = apply_canonical_claim_state(
        wallet_env, canonical_meta, wallet_plan
    )
    wallet_comment = render_comment(canonical_meta, wallet_plan)
    for required_text in [
        "Exact wallet signature requested",
        "Hosted claim handoff",
        "eth_signTypedData_v4",
        "wallet_signature",
        "10000",
        "Only confirmed canonical BountyClaimed",
        "discovery_source",
    ]:
        if required_text not in wallet_comment:
            raise UserError(f"self-test native claim handoff missing: {required_text}")

    response, problem = summarize_native_claim_handoff(
        503,
        {
            "schema_version": "agent-bounties/claim-problem-v1",
            "state": "sponsorship_unavailable",
            "failed_transition": "evaluate_sponsorship",
            "error": "sponsorship_unavailable",
            "next_action": "Fund the exact bond and retry without sponsorship.",
        },
        contract=contract,
        solver_wallet=solver_wallet,
    )
    if response is not None or problem != {
        "http_status": 503,
        "schema_version": "agent-bounties/claim-problem-v1",
        "state": "sponsorship_unavailable",
        "failed_transition": "evaluate_sponsorship",
        "error": "sponsorship_unavailable",
        "next_action": "Fund the exact bond and retry without sponsorship.",
    }:
        raise UserError("self-test did not preserve the exact hosted claim failure")

    mismatched_body = json.loads(handoff_fixture.read_text(encoding="utf-8"))["body"]
    mismatched_body["candidate"]["solver_wallet"] = (
        "0x4444444444444444444444444444444444444444"
    )
    try:
        summarize_native_claim_handoff(
            200,
            mismatched_body,
            contract=contract,
            solver_wallet=solver_wallet,
        )
    except UserError:
        pass
    else:
        raise UserError("self-test accepted a handoff for a different solver wallet")

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
        if "Machine claim request" in blocked_comment:
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
    for forbidden_text in ["Machine claim request", "Hosted claim handoff"]:
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
