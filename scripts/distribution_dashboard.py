#!/usr/bin/env python3
"""Build the live operator distribution dashboard from canonical API evidence.

The report endpoint supplies cumulative server-observed and canonical lifecycle
counts. The control file supplies reviewed spend, canary, incident, exclusion,
and origin-completion evidence that the API cannot infer. No token, wallet, or
vendor credential is written to the output.
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import sys
import urllib.error
import urllib.request
from decimal import Decimal
from pathlib import Path
from typing import Any

from scripts import distribution_gate as gate


REPORT_SCHEMA = "agent-bounties/distribution-operator-report-v1"
DASHBOARD_SCHEMA = "agent-bounties/distribution-dashboard-v1"
OPERATOR_REPORT_PATH = "/v1/operator/distribution/report"


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise gate.DistributionGateError(message)


def _count(value: Any, label: str) -> int:
    _require(isinstance(value, int) and not isinstance(value, bool) and value >= 0, f"{label} must be a nonnegative integer")
    return value


def _base_units(value: Any, label: str) -> int:
    text = str(value)
    _require(text.isdigit(), f"{label} must be nonnegative base-unit text")
    return int(text)


def fetch_operator_report(api_base: str, token: str) -> dict[str, Any]:
    base = api_base.strip().rstrip("/")
    _require(
        base.startswith("https://") or base.startswith("http://127.0.0.1:") or base.startswith("http://localhost:"),
        "API base must use HTTPS except for loopback development",
    )
    _require(bool(token.strip()), "OPERATOR_API_TOKEN is required for a live dashboard")
    request = urllib.request.Request(
        f"{base}{OPERATOR_REPORT_PATH}",
        headers={
            "accept": "application/json",
            "x-operator-token": token,
            "user-agent": "agent-bounties-distribution-dashboard/1",
        },
    )
    with urllib.request.urlopen(request, timeout=30) as response:  # nosec B310 - URL scheme is checked above
        return gate._object(json.load(response), "operator distribution report")


def build_dashboard(
    policy: dict[str, Any],
    control: dict[str, Any],
    report: dict[str, Any],
) -> dict[str, Any]:
    gate.validate_policy(policy)
    _require(report.get("schema_version") == REPORT_SCHEMA, "operator report schema is invalid")
    _require(report.get("protocol_scope") == "agent-bounties/autonomous-v1", "operator report protocol scope is unsupported")
    _require(isinstance(report.get("generated_at"), str) and report["generated_at"], "operator report generated_at is required")
    excluded_wallet_classes = gate._list(
        report.get("excluded_wallet_classes"),
        "operator report excluded wallet classes",
    )
    _require(
        all(isinstance(value, str) and value for value in excluded_wallet_classes),
        "operator report excluded wallet classes must be nonempty strings",
    )
    _require(
        len(set(excluded_wallet_classes)) == len(excluded_wallet_classes),
        "operator report excluded wallet classes must be unique",
    )
    required_exclusions = set(policy["exclusions"]["required_classes"])
    _require(
        required_exclusions.issubset(set(excluded_wallet_classes)),
        "operator report is missing required excluded wallet classes",
    )

    report_rows: dict[str, dict[str, Any]] = {}
    for raw in gate._list(report.get("rails"), "operator report rails"):
        row = gate._object(raw, "operator report rail")
        rail = row.get("rail")
        _require(isinstance(rail, str) and rail and rail not in report_rows, "operator report rails must be named and unique")
        report_rows[rail] = row

    merged = copy.deepcopy(control)
    merged["attribution"] = {
        "eligible_external_funded_bounties": _count(
            report.get("total_external_funded_bounties"),
            "total external funded bounties",
        ),
        "attributed_external_funded_bounties": _count(
            report.get("attributed_external_funded_bounties"),
            "attributed external funded bounties",
        ),
    }
    control_rows = {
        row.get("rail_id"): row
        for row in gate._list(merged.get("rails"), "control rails")
        if isinstance(row, dict)
    }
    _require(len(control_rows) == len(merged["rails"]), "control rail ids must be named and unique")

    dashboard_rows = []
    for vendor in policy["budget"]["vendors"]:
        rail = vendor["rail_id"]
        _require(rail in report_rows, f"operator report is missing paid rail {rail}")
        _require(rail in control_rows, f"control file is missing paid rail {rail}")
        source = report_rows[rail]
        destination = control_rows[rail]
        posters = _count(source.get("unique_external_funded_posters"), f"{rail} funded posters")
        settlements = _count(source.get("externally_funded_settled_bounties"), f"{rail} settlements")
        verified_evidence = _count(source.get("verified_settlements_with_evidence"), f"{rail} evidence-backed settlements")
        useful = _count(destination.get("verified_useful_settlements"), f"{rail} verified useful settlements")
        _require(useful <= verified_evidence, f"{rail} useful settlements exceed evidence-backed canonical settlements")
        destination["unique_external_funded_poster_wallets"] = posters
        destination["external_funded_canonical_settlements"] = settlements

        funnel = {
            "connected": _count(source.get("acquisitions"), f"{rail} acquisitions"),
            "mcp_requests": _count(source.get("mcp_requests"), f"{rail} MCP requests"),
            "draft_prepared": _count(source.get("prepared_handoffs"), f"{rail} prepared handoffs"),
            "wallet_review": _count(source.get("wallet_reviewed_handoffs"), f"{rail} wallet review"),
            "terms_attributed": _count(source.get("attributed_terms"), f"{rail} attributed terms"),
            "funded": _count(source.get("externally_funded_bounties"), f"{rail} funded bounties"),
            "claimed": _count(source.get("externally_funded_claimed_bounties"), f"{rail} claimed bounties"),
            "submitted": _count(source.get("externally_funded_submitted_bounties"), f"{rail} submitted bounties"),
            "verified_with_evidence": verified_evidence,
            "verified_useful": useful,
            "settled": settlements,
        }
        dashboard_rows.append(
            {
                "vendor_id": vendor["vendor_id"],
                "rail_id": rail,
                "funnel": funnel,
                "failures": {
                    "mcp_requests": _count(source.get("failed_mcp_requests"), f"{rail} failed MCP requests"),
                    "mcp_failure_rate_basis_points": _count(source.get("mcp_failure_rate_basis_points"), f"{rail} MCP failure rate"),
                    "handoff": _count(source.get("handoff_failure_count"), f"{rail} handoff failures"),
                },
                "external_funding_usdc": str(Decimal(_base_units(source.get("external_funding_base_units"), f"{rail} external funding")) / Decimal(1_000_000)),
                "settled_gmv_usdc": str(Decimal(_base_units(source.get("settled_gmv_base_units"), f"{rail} settled GMV")) / Decimal(1_000_000)),
            }
        )

    decision = gate.evaluate(policy, merged)
    decisions = {row["rail_id"]: row for row in decision["decisions"]}
    for row in dashboard_rows:
        row["activation"] = decisions[row["rail_id"]]

    return {
        "schema_version": DASHBOARD_SCHEMA,
        "generated_at": report["generated_at"],
        "mode": "cumulative_event_driven",
        "protocol_scope": report["protocol_scope"],
        "metrics": {
            "unique_external_funded_poster_wallets": _count(
                report.get("unique_external_funded_posters"),
                "unique external funded posters",
            ),
            "externally_funded_settled_bounties": sum(row["funnel"]["settled"] for row in dashboard_rows),
            "attribution_coverage_basis_points": _count(report.get("attribution_coverage_basis_points"), "attribution coverage"),
            "ltv": None,
        },
        "unavailable_metrics": report.get("unavailable_metrics", []),
        "activation_gate": {
            "exclusion_gate_passed": decision["exclusion_gate_passed"],
            "attribution_scale_gate_passed": decision["attribution_scale_gate_passed"],
            "open_critical_incidents": decision["open_critical_incidents"],
        },
        "rails": dashboard_rows,
        "evidence_boundary": "Live canonical counts, including the globally distinct funded-poster wallet count, come from the operator report. Spend, canaries, incidents, exclusion review, and origin-completion usefulness come from the separately reviewed control file. The report must attest every required wallet-exclusion class. Wallets are external-wallet proxies, not people. LTV remains unavailable until platform revenue exists.",
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--policy", type=Path, default=Path("ops/distribution/activation-policy-v1.json"))
    parser.add_argument("--control", type=Path, default=Path("ops/distribution/activation-observation-template.json"))
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--report", type=Path)
    source.add_argument("--api-base")
    parser.add_argument("--output", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        policy = gate.load_json(args.policy)
        control = gate.load_json(args.control)
        report = (
            gate.load_json(args.report)
            if args.report
            else fetch_operator_report(args.api_base, os.environ.get("OPERATOR_API_TOKEN", ""))
        )
        rendered = json.dumps(build_dashboard(policy, control, report), indent=2, sort_keys=True) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
        else:
            sys.stdout.write(rendered)
        return 0
    except (gate.DistributionGateError, OSError, json.JSONDecodeError, urllib.error.URLError) as error:
        print(f"distribution dashboard failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
