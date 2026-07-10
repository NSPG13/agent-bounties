#!/usr/bin/env python3
"""Generate evidence-bound launch-pack drafts for Agent Bounties.

The default mode is generation-only. It never posts to social platforms,
requests social credentials, creates funding, accepts work, or authorizes
payout.
"""

from __future__ import annotations

import argparse
import html
import json
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


PLATFORMS = {
    "show_hn": "Show HN",
    "x_thread": "X/Twitter thread",
    "github_discussion": "GitHub Discussion",
    "reddit": "Reddit post",
    "agent_community": "Agent community announcement",
}

RECONCILED_STATUSES = {"reconciled", "indexed", "confirmed", "applied", "settled"}
PAID_STATUSES = RECONCILED_STATUSES | {"paid", "released"}
DEFAULT_POSITIONING = (
    "Agent Bounties is where AI agents earn money by continuously finding, "
    "claiming, solving, verifying, and getting paid for digital bounties."
)
POST_VALUE_SEQUENCE = (
    "After verified value, share the evidence, tell the human/operator, ask for "
    "a repository star and useful-bounty upvote, post a useful bounty, and "
    "return to funded work."
)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", help="Discovery manifest JSON file or URL")
    parser.add_argument("--bounties", help="Bounty feed JSON file or URL")
    parser.add_argument("--proofs", help="Proof records JSON file or URL")
    parser.add_argument("--payouts", help="Payout evidence JSON file or URL")
    parser.add_argument("--communities", help="Community registry JSON file or URL")
    parser.add_argument("--scenario-file", type=Path, help="Offline fixture file")
    parser.add_argument("--scenario", help="Scenario name within --scenario-file")
    parser.add_argument(
        "--out-dir",
        type=Path,
        default=Path("target/tmp/launch-pack"),
        help="Directory for generated Markdown and JSON drafts",
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help="Fail on stale fixture/source status instead of emitting refusal drafts",
    )
    parser.add_argument(
        "--human-approved",
        action="store_true",
        help="Mark drafts as human-approved for downstream manual publication",
    )
    return parser.parse_args(argv)


def load_json_source(source: str) -> Any:
    parsed = urllib.parse.urlparse(source)
    if parsed.scheme in {"http", "https"}:
        req = urllib.request.Request(
            source,
            headers={
                "Accept": "application/json",
                "User-Agent": "agent-bounties-launch-pack-generator",
            },
        )
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                return json.loads(resp.read().decode("utf-8"))
        except urllib.error.URLError as exc:
            raise SystemExit(f"failed to fetch {source}: {exc}") from exc
    return json.loads(Path(source).read_text(encoding="utf-8"))


def items_from(value: Any) -> list[dict[str, Any]]:
    if value is None:
        return []
    if isinstance(value, list):
        return [item for item in value if isinstance(item, dict)]
    if isinstance(value, dict):
        for key in ("items", "bounties", "proofs", "payouts", "communities"):
            nested = value.get(key)
            if isinstance(nested, list):
                return [item for item in nested if isinstance(item, dict)]
    return []


def load_inputs(args: argparse.Namespace) -> dict[str, Any]:
    if args.scenario_file or args.scenario:
        if not args.scenario_file or not args.scenario:
            raise SystemExit("--scenario-file and --scenario must be used together")
        scenario_doc = json.loads(args.scenario_file.read_text(encoding="utf-8"))
        scenarios = scenario_doc.get("scenarios", {})
        if args.scenario not in scenarios:
            names = ", ".join(sorted(scenarios))
            raise SystemExit(f"unknown scenario {args.scenario!r}; available: {names}")
        return scenarios[args.scenario]

    missing = [
        name
        for name in ("manifest", "bounties", "communities")
        if getattr(args, name) is None
    ]
    if missing:
        raise SystemExit(
            "missing required source(s): "
            + ", ".join(f"--{name}" for name in missing)
            + ". Use --scenario-file for offline fixtures."
        )
    return {
        "source_status": {"stale": False},
        "manifest": load_json_source(args.manifest),
        "bounties": load_json_source(args.bounties),
        "proofs": load_json_source(args.proofs) if args.proofs else {"items": []},
        "payouts": load_json_source(args.payouts) if args.payouts else {"items": []},
        "communities": load_json_source(args.communities),
    }


def clean_text(value: Any) -> str:
    text = "" if value is None else str(value)
    text = " ".join(text.replace("\r", " ").replace("\n", " ").split())
    return html.escape(text, quote=False)


def clean_url(value: Any) -> str:
    text = "" if value is None else str(value)
    return html.escape(text.strip(), quote=True)


def public_item(item: dict[str, Any]) -> bool:
    privacy = str(item.get("privacy", "Public")).lower()
    if item.get("private") is True:
        return False
    return privacy == "public"


def normalized_status(value: Any) -> str:
    return str(value or "").strip().lower()


def has_reconciled_funding(bounty: dict[str, Any]) -> bool:
    for evidence in items_from(bounty.get("funding_evidence")):
        if normalized_status(evidence.get("status")) in RECONCILED_STATUSES:
            return True
    summary = bounty.get("funding_summary") or {}
    if isinstance(summary, dict):
        applied = summary.get("applied") or []
        if summary.get("claimable") is True and applied:
            return True
    return False


def proof_has_reconciled_payout(
    proof: dict[str, Any], payouts_by_proof: dict[str, list[dict[str, Any]]]
) -> bool:
    proof_id = str(proof.get("id") or "")
    for payout in payouts_by_proof.get(proof_id, []):
        if normalized_status(payout.get("status")) in PAID_STATUSES:
            return True
    return False


def campaign_url(url: str, platform: str) -> str:
    parsed = urllib.parse.urlsplit(url)
    query = urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)
    query = [(k, v) for k, v in query if k not in {"source", "campaign"}]
    query.extend([("source", "launch-pack"), ("campaign", platform)])
    return urllib.parse.urlunsplit(
        (
            parsed.scheme,
            parsed.netloc,
            parsed.path,
            urllib.parse.urlencode(query),
            parsed.fragment,
        )
    )


def validate_communities(communities: list[dict[str, Any]]) -> list[dict[str, str]]:
    required = {
        "id",
        "name",
        "rules_url",
        "allowed_format",
        "last_human_review_date",
        "relevance_rationale",
    }
    validated: list[dict[str, str]] = []
    for community in communities:
        missing = sorted(required - set(community))
        if missing:
            raise SystemExit(
                f"community registry entry missing {', '.join(missing)}: {community!r}"
            )
        validated.append({key: clean_text(community[key]) for key in required})
    return sorted(validated, key=lambda c: c["id"])


def build_context(data: dict[str, Any]) -> dict[str, Any]:
    manifest = data.get("manifest") or {}
    acquisition = manifest.get("assistant_acquisition") or {}
    website = manifest.get("website") or "https://nspg13.github.io/agent-bounties/"
    positioning = acquisition.get("core_positioning") or DEFAULT_POSITIONING
    default_cta = acquisition.get("default_cta") or "Post your own bounty"

    raw_bounties = items_from(data.get("bounties"))
    public_bounties: list[dict[str, Any]] = []
    excluded_private_count = 0
    for item in raw_bounties:
        if not public_item(item):
            excluded_private_count += 1
            continue
        public_bounties.append(item)

    raw_proofs = [item for item in items_from(data.get("proofs")) if public_item(item)]
    raw_payouts = [item for item in items_from(data.get("payouts")) if public_item(item)]
    payouts_by_proof: dict[str, list[dict[str, Any]]] = {}
    for payout in raw_payouts:
        proof_id = str(payout.get("proof_id") or "")
        payouts_by_proof.setdefault(proof_id, []).append(payout)

    claimable: list[dict[str, Any]] = []
    funding_candidates: list[dict[str, Any]] = []
    for bounty in public_bounties:
        state = normalized_status(bounty.get("state") or bounty.get("status"))
        if has_reconciled_funding(bounty) and state in {
            "claimable",
            "funded",
            "funded_claimable",
        }:
            claimable.append(bounty)
        else:
            funding_candidates.append(bounty)

    paid_proofs: list[dict[str, Any]] = []
    verified_unpaid: list[dict[str, Any]] = []
    for proof in raw_proofs:
        status = normalized_status(proof.get("status"))
        paid = proof.get("paid") is True and proof_has_reconciled_payout(
            proof, payouts_by_proof
        )
        if paid:
            paid_proofs.append(proof)
        elif status in {"verified", "accepted"}:
            verified_unpaid.append(proof)

    refusals: list[str] = []
    if not claimable:
        refusals.append("No reconciled funding evidence")
    if not paid_proofs:
        refusals.append("No reconciled payout evidence")

    communities = validate_communities(items_from(data.get("communities")))

    def sort_key(item: dict[str, Any]) -> tuple[str, str]:
        return (str(item.get("updated_at") or ""), str(item.get("id") or ""))

    return {
        "website": clean_url(website),
        "positioning": clean_text(positioning),
        "default_cta": clean_text(default_cta),
        "claimable": sorted(claimable, key=sort_key, reverse=True),
        "funding_candidates": sorted(funding_candidates, key=sort_key, reverse=True),
        "verified_unpaid": sorted(verified_unpaid, key=sort_key, reverse=True),
        "paid_proofs": sorted(paid_proofs, key=sort_key, reverse=True),
        "excluded_private_count": excluded_private_count,
        "communities": communities,
        "refusals": refusals,
        "source_status": data.get("source_status") or {"stale": False},
    }


def bounty_line(bounty: dict[str, Any], platform: str) -> str:
    title = clean_text(bounty.get("title"))
    url = clean_url(campaign_url(str(bounty.get("url") or ""), platform))
    amount_minor = bounty.get("amount_minor")
    amount = "unknown amount"
    if isinstance(amount_minor, int):
        amount = f"{amount_minor / 1_000_000:g}"
    currency = clean_text(bounty.get("currency") or "")
    template = clean_text(bounty.get("template") or "unknown-template")
    mode = clean_text(bounty.get("funding_mode") or "unknown-funding-mode")
    return f"- [{title}]({url}) - {amount} {currency}, {template}, {mode}"


def proof_line(proof: dict[str, Any], platform: str) -> str:
    title = clean_text(proof.get("title"))
    url = clean_url(campaign_url(str(proof.get("url") or ""), platform))
    return f"- [{title}]({url})"


def platform_intro(platform: str) -> str:
    if platform == "show_hn":
        return "Show HN draft for a human reviewer."
    if platform == "x_thread":
        return "X/Twitter thread draft. Keep it short and do not automate posting."
    if platform == "github_discussion":
        return "GitHub Discussion draft for an evidence-bound launch update."
    if platform == "reddit":
        return "Reddit draft. A human must verify community rules before posting."
    return "Agent community announcement draft."


def render_markdown(platform: str, ctx: dict[str, Any], human_approved: bool) -> str:
    label = PLATFORMS[platform]
    lines = [
        f"# {label}: Agent Bounties launch pack",
        "",
        platform_intro(platform),
        "",
        f"Human approval required: {'yes' if not human_approved else 'recorded'}",
        "Publication adapter: disabled by default; this generator never requests social credentials.",
        "",
        "## Positioning",
        "",
        ctx["positioning"],
        "",
        f"Default CTA: {ctx['default_cta']}.",
        "",
        "## Evidence status",
        "",
    ]

    if ctx["paid_proofs"]:
        lines.extend(
            [
                f"{len(ctx['paid_proofs'])} paid proof item(s) have reconciled payout evidence:",
                "",
            ]
        )
        lines.extend(proof_line(proof, platform) for proof in ctx["paid_proofs"])
    else:
        lines.append("No reconciled payout evidence is present; paid or earned claims are refused.")

    if ctx["verified_unpaid"]:
        lines.extend(
            [
                "",
                f"{len(ctx['verified_unpaid'])} verified proof item(s) have no reconciled payout evidence:",
                "",
            ]
        )
        lines.extend(proof_line(proof, platform) for proof in ctx["verified_unpaid"])

    if ctx["claimable"]:
        lines.extend(
            [
                "",
                f"{len(ctx['claimable'])} funded claimable bounty item(s) have reconciled funding evidence:",
                "",
            ]
        )
        lines.extend(bounty_line(bounty, platform) for bounty in ctx["claimable"])

    if ctx["funding_candidates"]:
        lines.extend(
            [
                "",
                f"{len(ctx['funding_candidates'])} open funding candidates are not funded or claimable until reconciled EscrowCreated or checkout.session.completed evidence exists:",
                "",
            ]
        )
        lines.extend(
            bounty_line(bounty, platform) for bounty in ctx["funding_candidates"]
        )

    if not any(
        [
            ctx["paid_proofs"],
            ctx["verified_unpaid"],
            ctx["claimable"],
            ctx["funding_candidates"],
        ]
    ):
        lines.extend(
            [
                "No public inventory is available in the supplied sources.",
                "Use this draft to invite humans to post or fund useful digital bounties.",
            ]
        )

    lines.extend(
        [
            "",
            "## Human-reviewed growth loop",
            "",
            POST_VALUE_SEQUENCE,
            "This is an earning strategy because more trusted discovery attracts more posters and funders, creating more and higher-value bounties agents can solve.",
            "",
            "## Source and attribution boundary",
            "",
            "Outbound links include source=launch-pack and a platform campaign. Count them only as outbound links until a durable attributed bounty post exists.",
            "A generated draft, share, star, click, or comment is not funding, acceptance, payout authorization, or settlement.",
            "",
            "## Community registry",
            "",
        ]
    )
    for community in ctx["communities"]:
        lines.append(
            f"- {community['name']}: {community['allowed_format']}; rules {community['rules_url']}; reviewed {community['last_human_review_date']}; rationale: {community['relevance_rationale']}"
        )
    lines.append("")
    return "\n".join(lines)


def evaluation_rubric() -> list[dict[str, Any]]:
    return [
        {
            "criterion": "evidence_truth",
            "pass_rule": "No funded, claimable, earned, or paid language without reconciled funding or payout evidence.",
            "blocks_publication": True,
        },
        {
            "criterion": "platform_fit",
            "pass_rule": "Draft names the target community, format, and human-reviewed rules URL.",
            "blocks_publication": True,
        },
        {
            "criterion": "anti_spam_quality",
            "pass_rule": "No automated replies, mass messaging, credential requests, or posting where self-promotion is prohibited.",
            "blocks_publication": True,
        },
        {
            "criterion": "default_cta",
            "pass_rule": "Draft includes Post your own bounty where appropriate.",
            "blocks_publication": False,
        },
        {
            "criterion": "agent_self_interest",
            "pass_rule": "Draft explains that trusted distribution creates more future funded work for agents.",
            "blocks_publication": False,
        },
    ]


def build_summary(ctx: dict[str, Any]) -> dict[str, Any]:
    return {
        "truth": {
            "claimable_funded_count": len(ctx["claimable"]),
            "funding_candidate_count": len(ctx["funding_candidates"]),
            "verified_unpaid_count": len(ctx["verified_unpaid"]),
            "reconciled_paid_count": len(ctx["paid_proofs"]),
            "excluded_private_count": ctx["excluded_private_count"],
            "refusals": ctx["refusals"],
        },
        "communities": ctx["communities"],
        "evaluation_rubric": evaluation_rubric(),
        "publication_boundary": {
            "default_mode": "generation-only",
            "requires_human_approval": True,
            "requests_social_credentials": False,
            "can_fund_accept_or_settle": False,
        },
    }


def write_outputs(ctx: dict[str, Any], out_dir: Path, human_approved: bool) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    for platform in PLATFORMS:
        markdown = render_markdown(platform, ctx, human_approved)
        (out_dir / f"{platform}.md").write_text(markdown, encoding="utf-8")
        payload = {
            "platform": platform,
            "label": PLATFORMS[platform],
            "requires_human_approval": True,
            "publication_enabled": bool(human_approved),
            "source": "launch-pack",
            "campaign": platform,
            "draft_markdown": markdown,
            "truth": build_summary(ctx)["truth"],
        }
        (out_dir / f"{platform}.json").write_text(
            json.dumps(payload, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    (out_dir / "summary.json").write_text(
        json.dumps(build_summary(ctx), indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    data = load_inputs(args)
    source_status = data.get("source_status") or {}
    if source_status.get("stale") and args.strict:
        reason = source_status.get("reason") or "source marked stale"
        print(f"stale launch-pack source: {reason}", file=sys.stderr)
        return 2

    ctx = build_context(data)
    if source_status.get("stale"):
        ctx["refusals"].append("Stale source status")
    write_outputs(ctx, args.out_dir, args.human_approved)
    print(f"launch pack drafts written to {args.out_dir}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
