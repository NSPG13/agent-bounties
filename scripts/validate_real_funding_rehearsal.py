#!/usr/bin/env python3
"""Validate deterministic real-funding rehearsal artifacts.

The rehearsal artifacts are intentionally strict: they prove that Stripe and
Base plans do not mutate payment state until webhook/log evidence is reconciled.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any


EXPECTED_STRIPE_API_VERSION = "2026-02-25.clover"
EXPECTED_BASE_SEPOLIA_USDC = "0x036CbD53842c5426634e7929541eC2318f3dCF7e"
EXPECTED_BASE_CHAIN_ID = 84532


def load_json(path: Path) -> dict[str, Any]:
    try:
        return json.loads(path.read_text(encoding="utf-8-sig"))
    except FileNotFoundError:
        fail(f"missing artifact: {path}")
    except json.JSONDecodeError as error:
        fail(f"invalid JSON in {path}: {error}")


def fail(message: str) -> None:
    raise SystemExit(f"real funding rehearsal validation failed: {message}")


def expect(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def money(value: dict[str, Any]) -> tuple[int, str]:
    return int(value["amount"]), str(value["currency"])


def settlement_for(report: dict[str, Any], rail: str) -> dict[str, Any]:
    for settlement in report.get("settlements", []):
        if settlement.get("rail") == rail:
            return settlement
    fail(f"missing final settlement for {rail}")


def first_payout(settlement: dict[str, Any]) -> dict[str, Any]:
    payouts = settlement.get("payout_intents", [])
    expect(len(payouts) == 1, f"{settlement.get('rail')} must have one payout intent")
    return payouts[0]


def check_funding_targets(report: dict[str, Any]) -> None:
    targets = {
        (target["rail"], target["amount"]["currency"]): target["amount"]["amount"]
        for target in report["final_bounty"]["funding_targets"]
    }
    expect(targets.get(("StripeFiat", "usd")) == 500, "missing USD Stripe funding target")
    expect(targets.get(("BaseUsdc", "usdc")) == 1000, "missing USDC Base funding target")

    partitions = {
        (partition["rail"], partition["target"]["currency"]): partition
        for partition in report["funding_summary"]["partitions"]
    }
    for key in (("StripeFiat", "usd"), ("BaseUsdc", "usdc")):
        partition = partitions.get(key)
        expect(partition is not None, f"missing funding partition {key}")
        expect(partition["claimable"] is True, f"partition {key} must be claimable")
        expect(money(partition["remaining"])[0] == 0, f"partition {key} must be fully funded")


def check_stripe(report: dict[str, Any]) -> None:
    stripe = report["stripe"]
    checkout = stripe["checkout_request"]
    expect(checkout["method"] == "POST", "Stripe checkout request must use POST")
    expect(checkout["endpoint"] == "/v1/checkout/sessions", "Stripe funding must use Checkout Sessions")
    expect(checkout["api_version"] == EXPECTED_STRIPE_API_VERSION, "Stripe API version drifted")
    expect(checkout["body"]["mode"] == "payment", "Stripe checkout must be one-time payment mode")
    expect(
        checkout["body"]["metadata"]["bounty_id"] == report["final_bounty"]["id"],
        "Stripe checkout metadata must bind bounty_id",
    )
    expect(
        checkout["body"]["metadata"]["funding_intent_id"] == stripe["funding_intent"]["id"],
        "Stripe checkout metadata must bind funding_intent_id",
    )
    expect(
        stripe["funding_intent"]["status"] == "AwaitingEvidence",
        "Stripe funding intent must start awaiting evidence",
    )
    expect(
        stripe["funding_reconciliation"]["funding_intent"]["status"] == "Applied",
        "Stripe funding must apply only after checkout webhook reconciliation",
    )
    expect(
        stripe["connect_eligibility"]["payout_state"]["status"] == "Pending",
        "Stripe Connect eligibility should only unblock the payout intent",
    )
    expect(
        stripe["transfer_plan"]["requires_reconciliation"] is True,
        "Stripe transfer plan must require transfer.created reconciliation",
    )
    expect(
        stripe["transfer_reconciliation"]["settlement"]["payout_intents"][0]["status"] == "Paid",
        "Stripe payout must become paid after transfer.created reconciliation",
    )


def check_base(report: dict[str, Any]) -> None:
    base = report["base"]
    expect(
        base["funding_intent"]["status"] == "AwaitingEvidence",
        "Base funding intent must start awaiting evidence",
    )
    expect(
        base["funding_plan"]["network"]["chain_id"] == EXPECTED_BASE_CHAIN_ID,
        "Base funding plan must target Base Sepolia",
    )
    expect(
        base["created_reconciliation"]["event"]["kind"] == "Created",
        "Base funding must reconcile an EscrowCreated event",
    )
    expect(
        base["created_reconciliation"]["bounty"]["status"] == "Claimable",
        "Base EscrowCreated reconciliation must make mixed bounty claimable",
    )
    expect(
        base["release_plan"]["transaction"]["function"] == "release(uint256,address[],uint256[],bytes32)",
        "Base release plan must be an escrow release call",
    )
    recipients = base["release_plan"]["release_call"]["recipients"]
    expect(len(recipients) == 1, "Base release must pay the advertised amount to the solver")
    expect(
        base["released_reconciliation"]["event"]["kind"] == "Released",
        "Base payout must reconcile an EscrowReleased event",
    )
    released_base = settlement_for(base["released_reconciliation"], "BaseUsdc")
    expect(
        first_payout(released_base)["status"] == "Paid",
        "Base payout intent must be paid only after EscrowReleased reconciliation",
    )


def check_final_settlements(report: dict[str, Any]) -> None:
    stripe = settlement_for(report, "StripeFiat")
    base = settlement_for(report, "BaseUsdc")
    stripe_payout = first_payout(stripe)
    base_payout = first_payout(base)

    expect(stripe_payout["status"] == "Paid", "final Stripe payout must be paid")
    expect(base_payout["status"] == "Paid", "final Base payout must be paid")
    expect(money(stripe_payout["amount"]) == (500, "usd"), "unexpected Stripe solver payout")
    expect(money(stripe["platform_fee"]) == (0, "usd"), "open-beta Stripe fee must be zero")
    expect(money(base_payout["amount"]) == (1000, "usdc"), "unexpected Base solver payout")
    expect(money(base["platform_fee"]) == (0, "usdc"), "open-beta Base fee must be zero")


def check_readiness(readiness: dict[str, Any]) -> None:
    expect(readiness["local_rehearsal_ready"] is True, "local rehearsal must be ready")
    expect(readiness["network"] == "Base Sepolia", "readiness must describe Base Sepolia")
    expect(readiness["network_chain_id"] == 84532, "readiness must include Base Sepolia chain id")
    expect(
        readiness["network_native_usdc_token_address"] == EXPECTED_BASE_SEPOLIA_USDC,
        "readiness must include Base Sepolia native USDC token",
    )
    expect(
        readiness["supplied_usdc_token_matches_native"] is True,
        "readiness must validate supplied Base Sepolia USDC token",
    )
    expect(
        isinstance(readiness["stripe_payment_method_configuration_configured"], bool),
        "readiness must expose a boolean Stripe payment-method configuration indicator",
    )
    checks = {check["name"]: check for check in readiness["checks"]}
    expect(
        checks["local deterministic rehearsal"]["configured"] is True,
        "local deterministic rehearsal check must be configured",
    )
    expect(
        checks["Base escrow addresses"]["configured"] is True,
        "Base Sepolia escrow and native token addresses must be accepted for planning",
    )
    expect(
        "Stripe Checkout payment-method configuration" in checks,
        "readiness must include the optional Stripe Checkout payment-method configuration check",
    )
    boundaries = "\n".join(readiness["evidence_boundaries"])
    for phrase in (
        "Checkout Session creation is not funding",
        "Payment Method Configuration only changes eligible Checkout methods",
        "approve/createEscrow transaction planning is not funding",
        "verifier acceptance creates settlement intents",
        "EscrowReleased log marks USDC payout paid",
        "transfer planning are not payout",
    ):
        expect(phrase in boundaries, f"missing evidence boundary: {phrase}")


def main() -> None:
    out_dir = Path(sys.argv[1] if len(sys.argv) > 1 else "target/real-funding-rehearsal")
    report = load_json(out_dir / "funding-rehearsal-demo.json")
    readiness = load_json(out_dir / "real-funding-readiness.json")

    expect(
        report["rehearsal"] == "stripe-dev-plus-base-sepolia-mixed-funding",
        "unexpected rehearsal id",
    )
    expect(report["final_bounty"]["status"] == "Paid", "final mixed bounty must be paid")
    expect(report["final_bounty"]["funding_mode"] == "MixedRails", "must rehearse mixed funding")
    expect(
        report["ledger_entries"] == 5,
        "expected five funding and zero-fee payout ledger entries",
    )
    for invariant in (
        "Stripe Checkout Session creation does not credit balances.",
        "Base payout is paid only after indexed EscrowReleased reconciliation.",
        "Stripe payout is paid only after transfer.created reconciliation.",
    ):
        expect(invariant in report["invariants"], f"missing invariant: {invariant}")

    check_funding_targets(report)
    check_stripe(report)
    check_base(report)
    check_final_settlements(report)
    check_readiness(readiness)

    print(
        "validated real funding rehearsal: "
        "mixed bounty paid with StripeFiat and BaseUsdc settlements after evidence reconciliation"
    )


if __name__ == "__main__":
    main()
