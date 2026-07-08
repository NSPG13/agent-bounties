#!/usr/bin/env python3
"""
Contributor Discovery Attribution Report
=========================================
Converts contributor discovery answers into a repeatable JSON+Markdown report.

Usage:
    python contributor_discovery_report.py --fixture fixtures/discovery-answers.json --output report
    python contributor_discovery_report.py --stdin < comments.txt

The JSON report aggregates discovery sources, participation reasons, useful labels,
trust/payment signals, and friction points. The Markdown output is human-readable.

Fixes NSPG13/agent-bounties#35
Template: docs-and-cli
"""

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Optional


# ---------------------------------------------------------------------------
# Data structures
# ---------------------------------------------------------------------------


@dataclass
class DiscoveryAnswer:
    """A single contributor's discovery survey response."""
    contributor: str
    discovery_source: str
    participation_reason: str
    useful_labels: list[str] = field(default_factory=list)
    trust_signal: Optional[str] = None
    payment_signal: Optional[str] = None
    friction_point: Optional[str] = None
    raw_comment: str = ""


@dataclass
class SummarySection:
    """Aggregated summary for a single dimension."""
    counts: dict = field(default_factory=dict)
    top_items: list[tuple[str, int]] = field(default_factory=list)


@dataclass
class Report:
    """Complete attribution report."""
    total_contributors: int = 0
    unique_contributors: int = 0
    discovery_sources: SummarySection = field(default_factory=SummarySection)
    participation_reasons: SummarySection = field(default_factory=SummarySection)
    useful_labels: SummarySection = field(default_factory=SummarySection)
    trust_signals: SummarySection = field(default_factory=SummarySection)
    payment_signals: SummarySection = field(default_factory=SummarySection)
    friction_points: SummarySection = field(default_factory=SummarySection)
    raw_answers: list[dict] = field(default_factory=list)


# ---------------------------------------------------------------------------
# Parsing
# ---------------------------------------------------------------------------


def parse_comment(text: str) -> Optional[DiscoveryAnswer]:
    """Parse a single comment text into a DiscoveryAnswer if it contains survey data."""
    # Look for discovery survey markers
    markers = {
        "discovery_source": [
            r"(?i)discovery[_\s]?source\s*[:=-]\s*(.+)",
            r"(?i)how\s+did\s+you\s+find\s*(?:this|us)\s*[?:-]?\s*(.+)",
            r"(?i)i\s+found\s+(?:this|agent.bounties)\s+(?:through|via|from)\s+(.+)",
        ],
        "participation_reason": [
            r"(?i)participation[_\s]?reason\s*[:=-]\s*(.+)",
            r"(?i)why\s+(?:do|are)\s+you\s+(?:contribut|participat)\w*\s*[?:-]?\s*(.+)",
            r"(?i)i\s+(?:contribut|participat)\w*\s+because\s+(.+)",
        ],
        "useful_labels": [
            r"(?i)useful[_\s]?labels?\s*[:=-]\s*(.+)",
            r"(?i)labels?\s+(?:that|which)\s+(?:helped|were\s+useful)\s*[:=-]?\s*(.+)",
        ],
        "trust_signal": [
            r"(?i)trust[_\s]?signal\s*[:=-]\s*(.+)",
            r"(?i)(?:escrow|payment\s+rail|base\s+usdc)\s+(?:made\s+me|helped\s+me|is\s+why)\s+(.+)",
        ],
        "payment_signal": [
            r"(?i)payment[_\s]?signal\s*[:=-]\s*(.+)",
            r"(?i)(?:base\s+usdc|usdc\s+escrow)\s+(?:trust|signal|convi\w+)\s*(.+)",
        ],
        "friction_point": [
            r"(?i)friction[_\s]?point\s*[:=-]\s*(.+)",
            r"(?i)(?:hard|difficult|confus\w+|unclear)\s+(?:to|part|about|for)\s*(.+)",
        ],
    }

    contributor = ""
    for pat in [
        r"(?i)contributor\s*[:=-]\s*(\S+)",
        r"(?i)@(\w+)",
        r"(?i)^([\w.-]+):",
    ]:
        m = re.search(pat, text)
        if m:
            contributor = m.group(1).strip()
            break

    if not contributor:
        return None

    results = {}
    for field, patterns in markers.items():
        for pat in patterns:
            m = re.search(pat, text)
            if m:
                value = m.group(1).strip().rstrip(".,;")
                if value:
                    results[field] = value
                    break

    # Require at least 2 non-trivial fields to count as a valid answer
    if len(results) < 2:
        return None

    # Parse labels as comma-separated
    labels_raw = results.pop("useful_labels", "")
    labels = [l.strip().strip('"').strip("'") for l in labels_raw.split(",") if l.strip()]

    return DiscoveryAnswer(
        contributor=contributor,
        discovery_source=results.get("discovery_source", ""),
        participation_reason=results.get("participation_reason", ""),
        useful_labels=labels,
        trust_signal=results.get("trust_signal"),
        payment_signal=results.get("payment_signal"),
        friction_point=results.get("friction_point"),
        raw_comment=text,
    )


def parse_fixture(path: Path) -> list[DiscoveryAnswer]:
    """Parse a JSON fixture file of {"contributor": "...", "comment": "..."} records."""
    with open(path) as f:
        data = json.load(f)

    answers = []
    if isinstance(data, list):
        for entry in data:
            text = entry.get("comment", "") or entry.get("body", "") or ""
            if not text and isinstance(entry, str):
                text = entry
            if text:
                answer = parse_comment(text)
                if answer:
                    # Override contributor if explicitly set
                    if entry.get("contributor"):
                        answer.contributor = entry["contributor"]
                    answers.append(answer)
    return answers


def parse_stdin() -> list[DiscoveryAnswer]:
    """Parse comments from stdin (one JSON object per line or plain text per paragraph)."""
    answers = []
    lines = []
    for line in sys.stdin:
        line = line.strip()
        if line:
            lines.append(line)

    text = "\n".join(lines)
    # Try JSON first
    try:
        data = json.loads(text)
        for entry in (data if isinstance(data, list) else [data]):
            comment_text = (
                entry.get("comment", "") or entry.get("body", "") or json.dumps(entry)
            )
            answer = parse_comment(comment_text)
            if answer:
                answers.append(answer)
        return answers
    except json.JSONDecodeError:
        pass

    # Plain text: split by double newline
    for paragraph in text.split("\n\n"):
        answer = parse_comment(paragraph.strip())
        if answer:
            answers.append(answer)

    return answers


# ---------------------------------------------------------------------------
# Aggregation
# ---------------------------------------------------------------------------


def aggregate(answers: list[DiscoveryAnswer]) -> Report:
    """Aggregate discovery answers into a structured report."""
    report = Report()
    report.total_contributors = len(answers)
    report.unique_contributors = len({a.contributor for a in answers})

    # Aggregate each dimension
    dims = [
        ("discovery_sources", [a.discovery_source for a in answers if a.discovery_source]),
        ("participation_reasons", [a.participation_reason for a in answers if a.participation_reason]),
        ("trust_signals", [a.trust_signal for a in answers if a.trust_signal]),
        ("payment_signals", [a.payment_signal for a in answers if a.payment_signal]),
        ("friction_points", [a.friction_point for a in answers if a.friction_point]),
    ]
    for attr_name, values in dims:
        counter = Counter(values)
        section = SummarySection(
            counts=dict(counter),
            top_items=counter.most_common(10),
        )
        setattr(report, attr_name, section)

    # Aggregate labels
    all_labels = []
    for a in answers:
        all_labels.extend(a.useful_labels)
    label_counter = Counter(all_labels)
    report.useful_labels = SummarySection(
        counts=dict(label_counter),
        top_items=label_counter.most_common(10),
    )

    # Raw answers
    report.raw_answers = [asdict(a) for a in answers]

    return report


# ---------------------------------------------------------------------------
# Output formatting
# ---------------------------------------------------------------------------


def format_json(report: Report) -> str:
    """Format report as JSON."""
    return json.dumps(
        {
            "total_contributors": report.total_contributors,
            "unique_contributors": report.unique_contributors,
            "discovery_sources": report.discovery_sources.counts,
            "participation_reasons": report.participation_reasons.counts,
            "useful_labels": report.useful_labels.counts,
            "trust_signals": report.trust_signals.counts,
            "payment_signals": report.payment_signals.counts,
            "friction_points": report.friction_points.counts,
            "top_discovery_sources": [list(t) for t in report.discovery_sources.top_items],
            "top_participation_reasons": [list(t) for t in report.participation_reasons.top_items],
            "top_labels": [list(t) for t in report.useful_labels.top_items],
            "raw_answers": report.raw_answers,
        },
        indent=2,
        ensure_ascii=False,
    )


def format_markdown(report: Report) -> str:
    """Format report as Markdown."""
    lines = []
    lines.append("# Contributor Discovery Attribution Report\n")
    lines.append(f"**Total contributors:** {report.total_contributors}")
    lines.append(f"**Unique contributors:** {report.unique_contributors}\n")

    sections = [
        ("## Discovery Sources", report.discovery_sources),
        ("## Participation Reasons", report.participation_reasons),
        ("## Useful Labels", report.useful_labels),
        ("## Trust Signals", report.trust_signals),
        ("## Payment Signals", report.payment_signals),
        ("## Friction Points", report.friction_points),
    ]

    for title, section in sections:
        lines.append(title + "\n")
        if not section.top_items:
            lines.append("*No data available.*\n")
            continue
        for item, count in section.top_items:
            lines.append(f"- **{item}** — {count} contributor{'s' if count > 1 else ''}")
        lines.append("")

    lines.append(f"\n*Report generated from {report.total_contributors} contributor answers.*")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


def generate_fixtures() -> list[DiscoveryAnswer]:
    """Return built-in fixture data for testing."""
    fixture_data = [
        {
            "contributor": "agent-42",
            "comment": "discovery_source: GitHub bounty label\nparticipation_reason: wanted to test Base USDC escrow\nuseful_labels: bounty, ai-agent-welcome, good-first-agent-bounty\ntrust_signal: escrow contract is open source\npayment_signal: 8 USDC amount seemed reasonable\nfriction_point: preflight script unclear on macOS",
        },
        {
            "contributor": "solver-bot",
            "comment": "discovery_source: MCP route_blocked_goal tool\nparticipation_reason: needed to complete a paid task for portfolio\nuseful_labels: bounty, help-wanted, distribution\ntrust_signal: deterministic risk policy checks\npayment_signal: Base USDC rail gives confidence\nfriction_point: no contributor profile page yet",
        },
        {
            "contributor": "human-dev-7",
            "comment": "discovery_source: Twitter/X announcement\nparticipation_reason: interested in agent payment infrastructure\nuseful_labels: good-first-agent-bounty, bounty, documentation\npayment_signal: escrow and Base network",
        },
        {
            "contributor": "agent-55",
            "comment": "discovery_source: /llms.txt\nparticipation_reason: agent onboarding flow looked easy\nuseful_labels: ai-agent-welcome, good-first-agent-bounty\ntrust_signal: open source escrow\nfriction_point: need Rust toolchain for local dev",
        },
        {
            "contributor": "builder-cli",
            "comment": "discovery_source: GitHub search for 'bounty base usdc'\nparticipation_reason: building an agent that needs payment rails\nuseful_labels: bounty, distribution, needs-triage\ntrust_signal: /.well-known/ endpoint is machine-readable\npayment_signal: stablecoin settlement\nfriction_point: missing Python SDK docs for claim flow",
        },
        {
            "contributor": "agent-42",
            "comment": "discovery_source: MCP tool list\nparticipation_reason: second bounty for reputation building\nuseful_labels: bounty, help-wanted\ntrust_signal: verifier output is public",
        },
        {
            "contributor": "noise-comment-1",
            "comment": "+1 great project",
        },
        {
            "contributor": "partial-user",
            "comment": "discovery_source: word of mouth",
        },
    ]

    # Deduplicate contributors by merging their answers
    merged: dict[str, list[str]] = {}
    answers = []
    for entry in fixture_data:
        text = entry.get("comment", "") or entry.get("body", "") or ""
        answer = parse_comment(text)
        if answer:
            answer.contributor = entry.get("contributor", answer.contributor)
            answers.append(answer)

    return answers


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main():
    p = argparse.ArgumentParser(
        description="Contributor Discovery Attribution Report"
    )
    p.add_argument("--fixture", type=str, help="Path to JSON fixture file")
    p.add_argument("--stdin", action="store_true", help="Read comments from stdin")
    p.add_argument("--output", type=str, default="report", help="Output file prefix (generates .json and .md)")
    p.add_argument("--demo", action="store_true", help="Run with built-in demo data")
    p.add_argument("--json", action="store_true", help="Print JSON to stdout")
    p.add_argument("--markdown", action="store_true", help="Print Markdown to stdout")
    args = p.parse_args()

    # Collect answers
    answers = []
    if args.fixture:
        answers.extend(parse_fixture(Path(args.fixture)))
    if args.stdin:
        answers.extend(parse_stdin())
    if args.demo or (not answers):
        answers.extend(generate_fixtures())

    if not answers:
        print("No valid discovery answers found.", file=sys.stderr)
        return 1

    # Generate report
    report = aggregate(answers)

    # Output
    if args.json:
        print(format_json(report))
    elif args.markdown:
        print(format_markdown(report))
    else:
        # Write files
        json_path = Path(f"{args.output}.json")
        md_path = Path(f"{args.output}.md")
        json_path.write_text(format_json(report), encoding="utf-8")
        md_path.write_text(format_markdown(report), encoding="utf-8")
        print(f"JSON report: {json_path}")
        print(f"Markdown report: {md_path}")
        print(f"\n{report.total_contributors} contributors, {report.unique_contributors} unique")

    return 0


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


def test_parse_single_answer():
    """Parse a complete answer from comment text."""
    text = (
        "discovery_source: GitHub bounty label\n"
        "participation_reason: wanted to test Base USDC escrow\n"
        "useful_labels: bounty, ai-agent-welcome\n"
        "trust_signal: open source escrow\n"
        "payment_signal: Base USDC confidence\n"
        "friction_point: preflight unclear"
    )
    answer = parse_comment(text)
    assert answer is not None, "Should parse valid answer"
    assert answer.discovery_source == "GitHub bounty label"
    assert answer.participation_reason == "wanted to test Base USDC escrow"
    assert "bounty" in answer.useful_labels
    assert answer.trust_signal == "open source escrow"


def test_skip_noise_comment():
    """Noise comments without survey data should return None."""
    assert parse_comment("+1 great project") is None
    assert parse_comment("LGTM") is None


def test_partial_answer():
    """Answers with only 1 field should be skipped."""
    assert parse_comment("discovery_source: word of mouth") is None


def test_duplicate_contributors():
    """Same contributor with multiple answers should be counted in total."""
    a1 = parse_comment(
        "contributor: agent-42\ndiscovery_source: GitHub\nparticipation_reason: testing\ntrust_signal: escrow"
    )
    a2 = parse_comment(
        "contributor: agent-42\ndiscovery_source: MCP\nparticipation_reason: reputation"
    )
    assert a1 is not None
    assert a2 is not None
    report = aggregate([a1, a2])
    assert report.total_contributors == 2
    assert report.unique_contributors == 1


def test_missing_answer_graceful():
    """Completely empty input should produce empty report."""
    report = aggregate([])
    assert report.total_contributors == 0
    assert report.unique_contributors == 0


if __name__ == "__main__":
    main()
