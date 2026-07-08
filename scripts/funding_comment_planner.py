#!/usr/bin/env python3
"""Wrapper around the canonical Rust funding-comment planner.

This script no longer re-implements funding-comment planning logic in
Python. All rail-alias resolution (e.g. ``BaseUsdcEscrow``,
``StripeFiatLedger``), currency validation, paid-bounty issue-form
validation, idempotency key derivation, and the
``requires_operator_reconciliation=true`` invariant are delegated to the
canonical planner implemented in the Rust CLI
(``cargo run -p cli -- github-funding-comment-plan``). This keeps a single
source of truth shared with the API/MCP/CLI deterministic path, so the
publicly documented commands (for example
``/agent-bounty fund 5 USDC via BaseUsdcEscrow`` and
``/agent-bounty fund 5 USD via StripeFiatLedger``) always resolve the same
way everywhere.

This script is a *planning* wrapper only. It never credits balances, marks a
bounty funded, authorizes claimability, or releases payout. All results are
surfaced as public, human-readable feedback that requires operator
reconciliation through the real Stripe/Base funding path.

The script can run in three modes:

1. ``--github-event`` mode, which reads comment/issue context from
   environment variables populated by the GitHub Actions workflow
   (``COMMENT_BODY``, ``COMMENT_AUTHOR``, ``COMMENT_ID``, ``ISSUE_NUMBER``,
   ``ISSUE_TITLE``, ``ISSUE_BODY``, ``REPO_FULL_NAME``, and optionally
   ``SEEN_IDEMPOTENCY_KEYS`` as a JSON array).
2. ``--fixture <path>`` mode, which reads a JSON fixture file with the same
   shape for local testing without any GitHub secrets.
3. ``--self-test`` mode, which runs the wrapper against the bundled fixtures
   in ``scripts/fixtures/funding-comment/`` and asserts the expected
   ok/action-required outcome for each documented scenario.

In all modes the script prints a Markdown-formatted planner result to
stdout and exits 0 on success. Invalid input never raises an unhandled
exception; it is always converted into constructive, action-required
Markdown feedback (rendered from the Rust planner's response).
"""
from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from typing import List, Mapping, Optional


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


@dataclass
class CommentContext:
    body: str
    author: str
    comment_id: str
    issue_number: str
    issue_title: str
    issue_body: str
    repo_full_name: str
    seen_idempotency_keys: list = field(default_factory=list)


def load_context_from_env() -> CommentContext:
    seen_raw = os.environ.get("SEEN_IDEMPOTENCY_KEYS", "[]")
    try:
        seen = json.loads(seen_raw)
    except json.JSONDecodeError:
        seen = []

    return CommentContext(
        body=os.environ.get("COMMENT_BODY", ""),
        author=os.environ.get("COMMENT_AUTHOR", "unknown"),
        comment_id=os.environ.get("COMMENT_ID", "0"),
        issue_number=os.environ.get("ISSUE_NUMBER", "0"),
        issue_title=os.environ.get("ISSUE_TITLE", ""),
        issue_body=os.environ.get("ISSUE_BODY", ""),
        repo_full_name=os.environ.get("REPO_FULL_NAME", "unknown/unknown"),
        seen_idempotency_keys=seen if isinstance(seen, list) else [],
    )


def load_context_from_fixture(path: str) -> CommentContext:
    with open(path, "r", encoding="utf-8") as handle:
        data = json.load(handle)

    return CommentContext(
        body=data.get("comment_body", ""),
        author=data.get("comment_author", "unknown"),
        comment_id=str(data.get("comment_id", "0")),
        issue_number=str(data.get("issue_number", "0")),
        issue_title=data.get("issue_title", ""),
        issue_body=data.get("issue_body", ""),
        repo_full_name=data.get("repo_full_name", "unknown/unknown"),
        seen_idempotency_keys=data.get("seen_idempotency_keys", []),
    )


def run_github_funding_comment_plan(
    ctx: CommentContext, workspace: pathlib.Path, tmp_dir: pathlib.Path
) -> str:
    """Invoke the canonical Rust planner and return its raw JSON stdout."""
    cargo_path = find_executable(["cargo", "cargo.exe"])
    if not cargo_path:
        raise UserError("cargo is required to plan a funding comment")

    issue_body_file = tmp_dir / "funding-comment-issue-body.md"
    issue_body_file.write_text(ctx.issue_body, encoding="utf-8")

    comment_body_file = tmp_dir / "funding-comment-body.md"
    comment_body_file.write_text(ctx.body, encoding="utf-8")

    seen_keys_file = tmp_dir / "funding-comment-seen-idempotency-keys.json"
    seen_keys_file.write_text(json.dumps(ctx.seen_idempotency_keys), encoding="utf-8")

    result = subprocess.run(
        [
            cargo_path,
            "run",
            "-p",
            "cli",
            "--",
            "github-funding-comment-plan",
            "--repository",
            ctx.repo_full_name,
            "--issue-number",
            ctx.issue_number,
            "--issue-title",
            ctx.issue_title,
            "--issue-body-file",
            str(issue_body_file),
            "--comment-body-file",
            str(comment_body_file),
            "--comment-author",
            ctx.author,
            "--comment-id",
            ctx.comment_id,
            "--seen-idempotency-keys-file",
            str(seen_keys_file),
        ],
        cwd=workspace,
        env=dict(os.environ),
        text=True,
        stdout=subprocess.PIPE,
        stderr=None,
        check=False,
    )
    if result.returncode != 0:
        raise UserError(f"github-funding-comment-plan failed with exit code {result.returncode}")
    return result.stdout


def render_markdown(plan: Mapping[str, object]) -> str:
    """Render the Rust planner's `{ok, title, lines}` response as Markdown."""
    ok = bool(plan.get("ok"))
    title = str(plan.get("title", ""))
    lines = plan.get("lines") or []
    header = "### \u2705 Funding signal planned" if ok else "### \u26a0\ufe0f Action required"
    body = "\n".join(f"- {line}" for line in lines)
    return f"{header}\n\n**{title}**\n\n{body}\n"


def plan_funding_comment_json(ctx: CommentContext) -> Mapping[str, object]:
    repo_root = script_repo_root()
    workspace = pathlib.Path(os.environ.get("GITHUB_WORKSPACE") or repo_root).resolve()
    tmp_dir = pathlib.Path(os.environ.get("RUNNER_TEMP") or workspace / "target" / "tmp").resolve()
    tmp_dir.mkdir(parents=True, exist_ok=True)

    plan_json = run_github_funding_comment_plan(ctx, workspace, tmp_dir)
    return json.loads(plan_json)


def plan_funding_comment(ctx: CommentContext) -> str:
    plan = plan_funding_comment_json(ctx)
    return render_markdown(plan)


def run_self_test() -> int:
    repo_root = script_repo_root()
    fixtures_dir = repo_root / "scripts" / "fixtures" / "funding-comment"

    scenarios = [
        ("valid-base-usdc.json", True),
        ("valid-stripe-usd.json", True),
        ("invalid-issue-body.json", False),
        ("duplicate-idempotency-key.json", False),
    ]

    failures = []
    for fixture_name, expect_ok in scenarios:
        fixture_path = fixtures_dir / fixture_name
        ctx = load_context_from_fixture(str(fixture_path))
        plan = plan_funding_comment_json(ctx)
        actual_ok = bool(plan.get("ok"))
        if actual_ok != expect_ok:
            failures.append(
                f"{fixture_name}: expected ok={expect_ok}, got ok={actual_ok} ({plan})"
            )

    if failures:
        raise UserError("self-test failures:\n" + "\n".join(failures))

    print(f"Funding comment planner self-test passed for {len(scenarios)} fixtures")
    return 0


def main(argv: Optional[list] = None) -> int:
    parser = argparse.ArgumentParser(description="Funding-comment planner for Agent Bounties.")
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument(
        "--github-event",
        action="store_true",
        help="Read comment context from GitHub Actions environment variables.",
    )
    group.add_argument(
        "--fixture",
        type=str,
        help="Path to a JSON fixture file with comment/issue context for local testing.",
    )
    group.add_argument(
        "--self-test",
        action="store_true",
        help="Run the wrapper against the bundled fixtures and assert expected outcomes.",
    )
    args = parser.parse_args(argv)

    try:
        if args.self_test:
            return run_self_test()

        if args.github_event:
            ctx = load_context_from_env()
        else:
            ctx = load_context_from_fixture(args.fixture)

        print(plan_funding_comment(ctx))
    except UserError as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
