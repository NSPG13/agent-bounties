#!/usr/bin/env python3
"""Evaluate event-driven paid-distribution activation and scale gates.

The evaluator consumes operator-reviewed aggregates. It cannot buy media, move
USDC, classify beneficial ownership, or turn noncanonical signals into funded
or settled outcomes.
"""

from __future__ import annotations

import argparse
import json
import sys
from decimal import Decimal, InvalidOperation, ROUND_HALF_UP
from pathlib import Path
from typing import Any


POLICY_SCHEMA = "agent-bounties/distribution-activation-policy-v1"
OBSERVATION_SCHEMA = "agent-bounties/distribution-observation-v1"
DECISION_SCHEMA = "agent-bounties/distribution-decision-v1"
ORDER_SCHEMA = "agent-bounties/vendor-orders-v1"


class DistributionGateError(ValueError):
    """A policy or observation is malformed or violates the evidence boundary."""


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise DistributionGateError(message)


def _object(value: Any, label: str) -> dict[str, Any]:
    _require(isinstance(value, dict), f"{label} must be an object")
    return value


def _list(value: Any, label: str) -> list[Any]:
    _require(isinstance(value, list), f"{label} must be an array")
    return value


def _whole(value: Any, label: str) -> int:
    _require(isinstance(value, int) and not isinstance(value, bool) and value >= 0, f"{label} must be a nonnegative integer")
    return value


def _decimal(value: Any, label: str) -> Decimal:
    _require(isinstance(value, (str, int)) and not isinstance(value, bool), f"{label} must be decimal text or an integer")
    try:
        parsed = Decimal(str(value))
    except InvalidOperation as error:
        raise DistributionGateError(f"{label} must be a decimal") from error
    _require(parsed.is_finite() and parsed >= 0, f"{label} must be finite and nonnegative")
    return parsed


def _money(value: Decimal) -> str:
    return str(value.quantize(Decimal("0.01"), rounding=ROUND_HALF_UP))


def load_json(path: str | Path) -> dict[str, Any]:
    with Path(path).open("r", encoding="utf-8") as handle:
        return _object(json.load(handle), str(path))


def validate_policy(policy: dict[str, Any]) -> None:
    _require(policy.get("schema_version") == POLICY_SCHEMA, "policy schema is invalid")
    _require(policy.get("decision_mode") == "event_driven", "policy must be event driven")

    attribution = _object(policy.get("attribution"), "policy.attribution")
    minimum_coverage = _decimal(attribution.get("minimum_scale_coverage"), "minimum scale coverage")
    _require(Decimal("0") < minimum_coverage <= Decimal("1"), "minimum scale coverage must be in (0, 1]")
    for field in ("first_touch_is_immutable", "assists_are_separate", "server_observed_rail_required"):
        _require(attribution.get(field) is True, f"policy.attribution.{field} must be true")

    evidence = _object(policy.get("evidence"), "policy.evidence")
    _require(evidence.get("funded_event") == "BountyBecameClaimable", "funded evidence must be BountyBecameClaimable")
    _require(evidence.get("settled_event") == "BountySettled", "settled evidence must be BountySettled")
    _require(evidence.get("require_valid_verifier_evidence") is True, "verifier evidence must be required")

    exclusions = _object(policy.get("exclusions"), "policy.exclusions")
    required_classes = _list(exclusions.get("required_classes"), "policy.exclusions.required_classes")
    _require(required_classes and len(required_classes) == len(set(required_classes)), "required exclusion classes must be nonempty and unique")
    _require(bool(exclusions.get("identity_boundary")), "wallet identity boundary is required")

    canary = _object(policy.get("canary"), "policy.canary")
    _require(_whole(canary.get("minimum_dry_runs"), "minimum dry runs") > 0, "minimum dry runs must be positive")
    _require(_whole(canary.get("minimum_mainnet_settled_runs"), "minimum mainnet settled runs") > 0, "minimum mainnet settled runs must be positive")
    _require(_decimal(canary.get("mainnet_bounty_principal_usdc"), "mainnet bounty principal") >= Decimal("2"), "mainnet bounty principal must respect the 2 USDC floor")
    _require(canary.get("require_all_canaries_excluded_from_external_metrics") is True, "canaries must be excluded from external metrics")

    scale = _object(policy.get("scale"), "policy.scale")
    _require(_decimal(scale.get("maximum_funded_poster_cac_mxn"), "poster CAC cap") > 0, "poster CAC cap must be positive")
    _require(_decimal(scale.get("maximum_settled_bounty_cac_mxn"), "settlement CAC cap") > 0, "settlement CAC cap must be positive")
    _require(_decimal(scale.get("next_tranche_multiplier"), "tranche multiplier") > 1, "tranche multiplier must exceed one")

    budget = _object(policy.get("budget"), "policy.budget")
    _require(budget.get("bounty_principal_is_distribution_spend") is False, "bounty principal must be excluded from distribution spend")
    _require(budget.get("deadline_is_review_cadence") is False, "the spend backstop cannot become a review cadence")
    _require(budget.get("deadline_never_overrides_evidence_gates") is True, "the spend backstop cannot override evidence gates")
    backstop = budget.get("spend_backstop_date")
    _require(isinstance(backstop, str) and len(backstop) == 10 and backstop[4] == "-" and backstop[7] == "-", "spend backstop must be an ISO date")
    minimum_spend = _decimal(budget.get("minimum_spend_mxn_by_backstop"), "minimum backstop spend")
    _require(minimum_spend >= Decimal("40000"), "minimum backstop spend must be at least 40000 MXN")
    rate = _decimal(budget.get("mxn_per_usd"), "MXN per USD")
    vendors = _list(budget.get("vendors"), "policy.budget.vendors")
    _require(len(vendors) == 3, "the activation policy requires exactly three vendors")
    vendor_ids: set[str] = set()
    rail_ids: set[str] = set()
    total_usd = Decimal("0")
    total_mxn = Decimal("0")
    for index, raw_vendor in enumerate(vendors):
        vendor = _object(raw_vendor, f"vendor[{index}]")
        vendor_id = vendor.get("vendor_id")
        rail_id = vendor.get("rail_id")
        _require(isinstance(vendor_id, str) and vendor_id and vendor_id not in vendor_ids, "vendor ids must be nonempty and unique")
        _require(isinstance(rail_id, str) and rail_id and rail_id not in rail_ids, "vendor rail ids must be nonempty and unique")
        vendor_ids.add(vendor_id)
        rail_ids.add(rail_id)
        price_usd = _decimal(vendor.get("price_usd"), f"{vendor_id}.price_usd")
        spend_mxn = _decimal(vendor.get("initial_spend_mxn"), f"{vendor_id}.initial_spend_mxn")
        _require(_money(price_usd * rate) == _money(spend_mxn), f"{vendor_id} MXN spend does not match the frozen FX conversion")
        _require(_whole(vendor.get("minimum_external_funded_posters"), f"{vendor_id} poster sample") > 0, "poster sample must be positive")
        _require(_whole(vendor.get("minimum_verified_settlements"), f"{vendor_id} settlement sample") > 0, "settlement sample must be positive")
        total_usd += price_usd
        total_mxn += spend_mxn
    _require(_money(total_usd) == _money(_decimal(budget.get("total_price_usd"), "total USD")), "total USD is inconsistent")
    _require(_money(total_mxn) == _money(_decimal(budget.get("total_initial_spend_mxn"), "total MXN")), "total MXN is inconsistent")
    _require(total_mxn >= minimum_spend, "planned vendor spend does not meet the backstop minimum")


def validate_orders(policy: dict[str, Any], packet: dict[str, Any]) -> None:
    """Fail closed when a purchase packet drifts from the activation policy."""

    validate_policy(policy)
    _require(packet.get("schema_version") == ORDER_SCHEMA, "order packet schema is invalid")
    _require(packet.get("policy_id") == policy["policy_id"], "order packet policy id is invalid")
    orders = _list(packet.get("orders"), "orders")
    expected = {vendor["vendor_id"]: vendor for vendor in policy["budget"]["vendors"]}
    _require(len(orders) == len(expected), "order packet must contain every policy vendor exactly once")
    observed: set[str] = set()
    for index, raw_order in enumerate(orders):
        order = _object(raw_order, f"orders[{index}]")
        vendor_id = order.get("vendor_id")
        _require(isinstance(vendor_id, str) and vendor_id in expected, "order vendor is not in policy")
        _require(vendor_id not in observed, "order vendors must be unique")
        observed.add(vendor_id)
        vendor = expected[vendor_id]
        _require(order.get("rail_id") == vendor["rail_id"], f"{vendor_id} rail does not match policy")
        _require(
            _money(_decimal(order.get("maximum_initial_spend_mxn"), f"{vendor_id} maximum spend"))
            == _money(_decimal(vendor["initial_spend_mxn"], f"{vendor_id} policy spend")),
            f"{vendor_id} maximum spend does not match policy",
        )
        inventory = _list(order.get("planned_inventory"), f"{vendor_id}.planned_inventory")
        _require(inventory, f"{vendor_id} must define purchase inventory")
        inventory_usd = Decimal("0")
        for item_index, raw_item in enumerate(inventory):
            item = _object(raw_item, f"{vendor_id}.planned_inventory[{item_index}]")
            units = _whole(item.get("units"), f"{vendor_id} inventory units")
            _require(units > 0, f"{vendor_id} inventory units must be positive")
            unit_price = _decimal(
                item.get("public_price_usd_per_unit"),
                f"{vendor_id} public inventory unit price",
            )
            _require(unit_price > 0, f"{vendor_id} public inventory unit price must be positive")
            _require(
                str(item.get("public_price_status", "")).startswith("verified_public_"),
                f"{vendor_id} inventory needs a verified public price status",
            )
            inventory_usd += Decimal(units) * unit_price
        _require(
            _money(inventory_usd)
            == _money(_decimal(vendor["price_usd"], f"{vendor_id} policy price")),
            f"{vendor_id} inventory does not match policy price",
        )
        public_source = order.get("public_inventory_source")
        _require(
            isinstance(public_source, str)
            and public_source.startswith("https://")
            and "@" not in public_source.split("/", 3)[2],
            f"{vendor_id} public inventory source must be credential-free HTTPS",
        )
        _require(
            order.get("source_endpoint")
            == f"https://mcp.agentbounties.app/r/{vendor['rail_id']}/mcp",
            f"{vendor_id} source endpoint is not the immutable attributed route",
        )
        _require(
            order.get("install_destination")
            == f"https://agentbounties.app/install/{vendor['rail_id']}/",
            f"{vendor_id} install destination is not the deployed rail-specific route",
        )
        _require(
            order.get("preferred_install_alias_after_dns")
            == f"https://install.agentbounties.app/{vendor['rail_id']}",
            f"{vendor_id} preferred install alias is not rail specific",
        )
        required = set(_list(order.get("required_before_purchase"), f"{vendor_id}.required_before_purchase"))
        for gate in (
            "attribution_endpoint_deployed",
            "dry_run_canaries_joined",
            "two_usdc_mainnet_canary_canonically_settled",
            "canaries_excluded_from_external_metrics",
            "owner_purchase_approval_recorded",
        ):
            _require(gate in required, f"{vendor_id} is missing purchase gate {gate}")
        _require(
            str(order.get("purchase_state", "")).startswith("blocked_on_"),
            f"{vendor_id} checked-in purchase state must fail closed",
        )

    glama = next(order for order in orders if order["vendor_id"] == "glama")
    copy = _object(glama.get("placement_copy"), "glama.placement_copy")
    _require(0 < len(str(copy.get("headline", ""))) <= 40, "Glama headline must be at most 40 characters")
    _require(0 < len(str(copy.get("sentence", ""))) <= 160, "Glama sentence must be at most 160 characters")


def _canary_passes(policy: dict[str, Any], row: dict[str, Any]) -> tuple[bool, list[str]]:
    requirements = policy["canary"]
    canary = _object(row.get("canary"), f"{row.get('rail_id')}.canary")
    dry_total = _whole(canary.get("dry_runs_total"), "dry run total")
    dry_joined = _whole(canary.get("dry_runs_joined"), "dry run joined")
    mainnet_total = _whole(canary.get("mainnet_runs_total"), "mainnet run total")
    mainnet_settled = _whole(canary.get("mainnet_runs_settled"), "mainnet run settled")
    _require(dry_joined <= dry_total, "joined dry runs cannot exceed total dry runs")
    _require(mainnet_settled <= mainnet_total, "settled mainnet runs cannot exceed total mainnet runs")
    refs = _list(canary.get("evidence_refs"), "canary.evidence_refs")
    reasons: list[str] = []
    if dry_total < requirements["minimum_dry_runs"] or dry_joined != dry_total:
        reasons.append("dry_run_canaries_incomplete")
    if mainnet_total < requirements["minimum_mainnet_settled_runs"] or mainnet_settled != mainnet_total:
        reasons.append("mainnet_settlement_canary_incomplete")
    if canary.get("excluded_from_external_metrics") is not True:
        reasons.append("canaries_not_excluded_from_external_metrics")
    if len(refs) < dry_total + mainnet_total:
        reasons.append("canary_evidence_refs_incomplete")
    return not reasons, reasons


def evaluate(policy: dict[str, Any], observation: dict[str, Any]) -> dict[str, Any]:
    validate_policy(policy)
    _require(observation.get("schema_version") == OBSERVATION_SCHEMA, "observation schema is invalid")

    review = _object(observation.get("exclusion_review"), "observation.exclusion_review")
    reviewed_classes = _list(review.get("required_classes_reviewed"), "required classes reviewed")
    expected_classes = set(policy["exclusions"]["required_classes"])
    exclusion_ready = (
        review.get("status") == "complete"
        and set(reviewed_classes) == expected_classes
        and len(reviewed_classes) == len(expected_classes)
        and review.get("external_wallet_proxy_disclosed") is True
        and review.get("operator_funded_development_excluded") is True
    )

    attribution = _object(observation.get("attribution"), "observation.attribution")
    eligible = _whole(attribution.get("eligible_external_funded_bounties"), "eligible attributed outcomes")
    attributed = _whole(attribution.get("attributed_external_funded_bounties"), "attributed outcomes")
    _require(attributed <= eligible, "attributed outcomes cannot exceed eligible outcomes")
    coverage = None if eligible == 0 else Decimal(attributed) / Decimal(eligible)
    coverage_ready = coverage is not None and coverage >= _decimal(policy["attribution"]["minimum_scale_coverage"], "minimum coverage")

    safety = _object(observation.get("safety"), "observation.safety")
    critical_incidents = _whole(safety.get("open_critical_incidents"), "open critical incidents")
    rail_rows = _list(observation.get("rails"), "observation.rails")
    rows_by_id: dict[str, dict[str, Any]] = {}
    for raw_row in rail_rows:
        row = _object(raw_row, "observation rail")
        rail_id = row.get("rail_id")
        _require(isinstance(rail_id, str) and rail_id and rail_id not in rows_by_id, "observation rail ids must be nonempty and unique")
        rows_by_id[rail_id] = row

    decisions: list[dict[str, Any]] = []
    for vendor in policy["budget"]["vendors"]:
        rail_id = vendor["rail_id"]
        _require(rail_id in rows_by_id, f"observation is missing {rail_id}")
        row = rows_by_id[rail_id]
        spend = _decimal(row.get("spend_mxn"), f"{rail_id}.spend_mxn")
        posters = _whole(row.get("unique_external_funded_poster_wallets"), f"{rail_id}.posters")
        settlements = _whole(row.get("external_funded_canonical_settlements"), f"{rail_id}.settlements")
        useful = _whole(row.get("verified_useful_settlements"), f"{rail_id}.useful settlements")
        _require(useful <= settlements, "verified useful settlements cannot exceed canonical settlements")
        useful_refs = _list(
            row.get("verified_useful_evidence_refs"),
            f"{rail_id}.verified_useful_evidence_refs",
        )
        _require(
            len(useful_refs) == useful
            and len(set(useful_refs)) == useful
            and all(isinstance(reference, str) and reference.strip() for reference in useful_refs),
            "every verified useful settlement requires one unique evidence reference",
        )
        canary_ready, reasons = _canary_passes(policy, row)
        poster_cac = None if posters == 0 else spend / Decimal(posters)
        settlement_cac = None if useful == 0 else spend / Decimal(useful)

        if not exclusion_ready:
            decision = "blocked_exclusion_review"
            reasons.append("external_exclusion_review_incomplete")
        elif critical_incidents:
            decision = "halt_critical_incident"
            reasons.append("open_critical_incident")
        elif not canary_ready:
            decision = "blocked_canary"
        elif eligible > 0 and not coverage_ready:
            decision = "hold_attribution_coverage"
            reasons.append("attribution_coverage_below_scale_gate")
        elif spend == 0:
            decision = "activate_initial_placement"
        elif posters < vendor["minimum_external_funded_posters"] or useful < vendor["minimum_verified_settlements"]:
            decision = "hold_no_incremental_spend"
            reasons.append("minimum_outcome_sample_not_reached")
        elif useful != settlements:
            decision = "do_not_renew"
            reasons.append("settlement_without_complete_usefulness_evidence")
        elif poster_cac is not None and poster_cac <= _decimal(policy["scale"]["maximum_funded_poster_cac_mxn"], "poster CAC cap") and settlement_cac is not None and settlement_cac <= _decimal(policy["scale"]["maximum_settled_bounty_cac_mxn"], "settlement CAC cap"):
            decision = "scale_next_tranche"
        else:
            decision = "do_not_renew"
            if poster_cac is None or poster_cac > _decimal(policy["scale"]["maximum_funded_poster_cac_mxn"], "poster CAC cap"):
                reasons.append("funded_poster_cac_above_cap")
            if settlement_cac is None or settlement_cac > _decimal(policy["scale"]["maximum_settled_bounty_cac_mxn"], "settlement CAC cap"):
                reasons.append("settled_bounty_cac_above_cap")

        proposed_spend = Decimal("0")
        if decision == "activate_initial_placement":
            proposed_spend = _decimal(vendor["initial_spend_mxn"], "initial spend")
        elif decision == "scale_next_tranche":
            proposed_spend = spend * _decimal(policy["scale"]["next_tranche_multiplier"], "tranche multiplier")
        decisions.append(
            {
                "vendor_id": vendor["vendor_id"],
                "rail_id": rail_id,
                "decision": decision,
                "reason_codes": sorted(set(reasons)),
                "observed_spend_mxn": _money(spend),
                "proposed_next_tranche_mxn": _money(proposed_spend),
                "owner_purchase_approval_required": True,
                "unique_external_funded_poster_wallets": posters,
                "verified_useful_settlements": useful,
                "funded_poster_cac_mxn": None if poster_cac is None else _money(poster_cac),
                "settled_bounty_cac_mxn": None if settlement_cac is None else _money(settlement_cac),
                "canary_gate_passed": canary_ready,
            }
        )

    return {
        "schema_version": DECISION_SCHEMA,
        "policy_id": policy["policy_id"],
        "mode": "event_driven",
        "exclusion_gate_passed": exclusion_ready,
        "attribution_coverage": None if coverage is None else str(coverage.quantize(Decimal("0.0001"))),
        "attribution_scale_gate_passed": coverage_ready,
        "open_critical_incidents": critical_incidents,
        "decisions": decisions,
        "evidence_boundary": "This decision uses reviewed aggregate wallet proxies and canonical lifecycle evidence. It does not purchase media, authorize a wallet, prove beneficial ownership, or treat a noncanonical signal as funding or settlement.",
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    validate = subparsers.add_parser("validate-policy")
    validate.add_argument("--policy", required=True, type=Path)
    validate_orders_parser = subparsers.add_parser("validate-orders")
    validate_orders_parser.add_argument("--policy", required=True, type=Path)
    validate_orders_parser.add_argument("--orders", required=True, type=Path)
    evaluate_parser = subparsers.add_parser("evaluate")
    evaluate_parser.add_argument("--policy", required=True, type=Path)
    evaluate_parser.add_argument("--observation", required=True, type=Path)
    evaluate_parser.add_argument("--output", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        policy = load_json(args.policy)
        if args.command == "validate-policy":
            validate_policy(policy)
            print(json.dumps({"status": "valid", "policy_id": policy["policy_id"]}, sort_keys=True))
            return 0
        if args.command == "validate-orders":
            validate_orders(policy, load_json(args.orders))
            print(json.dumps({"status": "valid", "policy_id": policy["policy_id"]}, sort_keys=True))
            return 0
        decision = evaluate(policy, load_json(args.observation))
        rendered = json.dumps(decision, indent=2, sort_keys=True) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
        else:
            sys.stdout.write(rendered)
        return 0
    except (DistributionGateError, OSError, json.JSONDecodeError) as error:
        print(f"distribution gate failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
