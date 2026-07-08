#!/usr/bin/env python3
"""Deterministic funding-comment planner for the Agent Bounties funding-comment Action.

This script parses issue comments of the form:

    /agent-bounty fund <amount> <currency> via <rail>

and produces a *planning* result only. It never credits balances, marks a
bounty funded, authorizes claimability, or releases payout. All results are
surfaced as public, human-readable feedback that requires operator
reconciliation through the real Stripe/Base funding path.

The script can run in two modes:

1. ``--github-event`` mode, which reads comment/issue context from environment
   variables populated by the GitHub Actions workflow (``COMMENT_BODY``,
   ``COMMENT_AUTHOR``, ``COMMENT_ID``, ``ISSUE_NUMBER``, ``ISSUE_LABELS``,
   ``REPO_FULL_NAME``).
2. ``--fixture <path>`` mode, which reads a JSON fixture file with the same
   shape for local testing without any GitHub secrets.

In both modes the script prints a Markdown-formatted planner result to
stdout and exits 0. Invalid input never raises an unhandled exception; it is
always converted into constructive, action-required Markdown feedback.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
from dataclasses import dataclass, field
from decimal import Decimal, InvalidOperation
from typing import Optional

SUPPORTED_RAILS = {
    "base": {"currencies": {"usdc"}},
    "stripe": {"currencies": {"usd", "eur", "gbp"}},
}

BOUNTY_LABEL = "bounty"

COMMAND_PATTERN = re.compile(
    r"^/agent-bounty\s+fund\s+(?P<amount>[^\s]+)\s+(?P<currency>[A-Za-z]+)\s+via\s+(?P<rail>[A-Za-z]+)\s*$",
    re.IGNORECASE,
)

# In-memory duplicate tracking is not persisted across workflow runs by
# design (the Action has no privileged storage). For local/replay testing,
# fixtures may supply a `seen_idempotency_keys` list to simulate duplicates
# that the operator reconciliation ledger would already have observed.


@dataclass
class CommentContext:
    body: str
    author: str
    comment_id: str
    issue_number: str
    issue_labels: list
    repo_full_name: str
    seen_idempotency_keys: list = field(default_factory=list)


@dataclass
class PlannerResult:
    ok: bool
    title: str
    lines: list

    def to_markdown(self) -> str:
        header = "### ✅ Funding signal planned" if self.ok else "### ⚠️ Action required"
        body = "\n".join(f"- {line}" for line in self.lines)
        return f"{header}\n\n**{self.title}**\n\n{body}\n"


def load_context_from_env() -> CommentContext:
    labels_raw = os.environ.get("ISSUE_LABELS", "[]")
    try:
        labels = json.loads(labels_raw)
    except json.JSONDecodeError:
        labels = []

    return CommentContext(
        body=os.environ.get("COMMENT_BODY", ""),
        author=os.environ.get("COMMENT_AUTHOR", "unknown"),
        comment_id=os.environ.get("COMMENT_ID", "0"),
        issue_number=os.environ.get("ISSUE_NUMBER", "0"),
        issue_labels=labels if isinstance(labels, list) else [],
        repo_full_name=os.environ.get("REPO_FULL_NAME", "unknown/unknown"),
        seen_idempotency_keys=[],
    )


def load_context_from_fixture(path: str) -> CommentContext:
    with open(path, "r", encoding="utf-8") as handle:
        data = json.load(handle)

    return CommentContext(
        body=data.get("comment_body", ""),
        author=data.get("comment_author", "unknown"),
        comment_id=str(data.get("comment_id", "0")),
        issue_number=str(data.get("issue_number", "0")),
        issue_labels=data.get("issue_labels", []),
        repo_full_name=data.get("repo_full_name", "unknown/unknown"),
        seen_idempotency_keys=data.get("seen_idempotency_keys", []),
    )


def derive_idempotency_key(ctx: CommentContext, amount: str, currency: str, rail: str) -> str:
    payload = "|".join(
        [
            ctx.repo_full_name,
            ctx.issue_number,
            ctx.comment_id,
            amount,
            currency.lower(),
            rail.lower(),
        ]
    )
    digest = hashlib.sha256(payload.encode("utf-8")).hexdigest()
    return f"fund-{digest[:16]}"


def plan_funding_comment(ctx: CommentContext) -> PlannerResult:
    if BOUNTY_LABEL not in [str(label).lower() for label in ctx.issue_labels]:
        return PlannerResult(
            ok=False,
            title="This issue is not labeled as a bounty",
            lines=[
                f"Issue #{ctx.issue_number} does not carry the `{BOUNTY_LABEL}` label.",
                "The funding-comment planner only runs for bounty issues.",
                "No amount, currency, or rail has been evaluated.",
            ],
        )

    match = COMMAND_PATTERN.match(ctx.body.strip())
    if not match:
        return PlannerResult(
            ok=False,
            title="Could not parse funding command",
            lines=[
                "Expected format: `/agent-bounty fund <amount> <currency> via <rail>`",
                f"Received: `{ctx.body.strip()}`",
                "Please retry with the exact command syntax, for example:",
                "`/agent-bounty fund 25 USDC via base`",
            ],
        )

    amount_raw = match.group("amount")
    currency = match.group("currency").lower()
    rail = match.group("rail").lower()

    try:
        amount = Decimal(amount_raw)
    except InvalidOperation:
        return PlannerResult(
            ok=False,
            title="Invalid amount",
            lines=[
                f"`{amount_raw}` is not a valid decimal amount.",
                "Please provide a positive numeric amount, e.g. `25` or `12.50`.",
            ],
        )

    if amount <= 0:
        return PlannerResult(
            ok=False,
            title="Invalid amount",
            lines=[
                f"Amount `{amount_raw}` must be greater than zero.",
            ],
        )

    if rail not in SUPPORTED_RAILS:
        supported = ", ".join(sorted(SUPPORTED_RAILS.keys()))
        return PlannerResult(
            ok=False,
            title="Unsupported funding rail",
            lines=[
                f"Rail `{rail}` is not supported by this planner.",
                f"Supported rails: {supported}.",
            ],
        )

    supported_currencies = SUPPORTED_RAILS[rail]["currencies"]
    if currency not in supported_currencies:
        supported = ", ".join(sorted(supported_currencies))
        return PlannerResult(
            ok=False,
            title="Unsupported currency for rail",
            lines=[
                f"Currency `{currency.upper()}` is not supported on rail `{rail}`.",
                f"Supported currencies for `{rail}`: {supported}.",
            ],
        )

    idempotency_key = derive_idempotency_key(ctx, amount_raw, currency, rail)

    if idempotency_key in ctx.seen_idempotency_keys:
        return PlannerResult(
            ok=False,
            title="Duplicate funding signal",
            lines=[
                f"Idempotency key `{idempotency_key}` has already been recorded.",
                "This funding comment appears to be a duplicate of a previously planned signal.",
                "No new planner result has been generated.",
            ],
        )

    return PlannerResult(
        ok=True,
        title=f"Funding signal from @{ctx.author}",
        lines=[
            f"amount: `{amount_raw}`",
            f"currency: `{currency.upper()}`",
            f"rail: `{rail}`",
            f"contributor_login: `@{ctx.author}`",
            f"idempotency_key: `{idempotency_key}`",
            "requires_operator_reconciliation: `true`",
            "",
            "This is a demand signal only. No balance has been credited, no bounty has been "
            "marked funded, no claimability has been authorized, and no payout has been released. "
            "An operator must reconcile this signal through the real Stripe/Base funding path.",
        ],
    )


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
    args = parser.parse_args(argv)

    if args.github_event:
        ctx = load_context_from_env()
    else:
        ctx = load_context_from_fixture(args.fixture)

    result = plan_funding_comment(ctx)
    print(result.to_markdown())
    return 0


if __name__ == "__main__":
    sys.exit(main())
