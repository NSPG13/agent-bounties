#!/usr/bin/env python3
"""Select one canonically claimable coding bounty from an inventory snapshot.

Reads an inventory JSON file and emits a single JSON object on stdout carrying an
`action` and exactly one `next_action` string.

Actions
-------
claim    one canonical, claimable, funded, verifier-ready, positive-margin bounty
wait     the inventory is empty; nothing to act on
refresh  coverage is missing, stale, or dimensionally invalid — re-fetch before deciding
skip     candidates exist but none are actionable

FAIL-CLOSED DESIGN. Every unknown is a refusal, not a pass:
  - missing / unparseable / FUTURE freshness    -> refresh (future clocks are NOT fresh)
  - missing canonical Base source               -> skip
  - work_state not canonically claimable        -> skip
  - funding incomplete or payment not escrowed  -> skip
  - verifier not ready                          -> skip
  - terms absent or invalid                     -> skip
  - money missing decimals/currency, or a unit
    mismatch between reward and spend           -> refresh (never coerce to zero)
  - claim expiry evaluated BEFORE occupancy, so
    an expired record is reclaimable, not blocked

WALLET SAFETY: never reads, stores, or transmits key material, and never
broadcasts. It emits an unsigned intent for an external signer.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import sys
import time
from pathlib import Path

# The canonical ready-to-earn view. These query parameters are the ones the API
# actually deserializes (crates/api/src/opportunities.rs::OpportunityQuery:
# network, view, source_type, work_state, payment_state, limit) and they match
# docs/posting-a-usable-bounty.md and docs/agent-quickstart.md.
#
# An invented parameter such as `?ready_to_earn=true` is NOT rejected by the
# server -- unknown query keys are simply ignored, so it silently returns the
# UNFILTERED inventory while looking filtered. That is the exact failure this
# selector exists to prevent, so the filter is spelled out canonically here and
# referenced everywhere instead of being retyped per call site.
READY_TO_EARN_FEED = (
    "https://api.agentbounties.app/v1/opportunities"
    "?network=base-mainnet&view=ready_to_earn&source_type=canonical_base"
)

# The response-level production contract. These are NOT invented: they are the
# exact fields of `OpportunityProjectionResponse` and `OpportunitySourceStatus`
# in crates/api/src/opportunities.rs, and the schema string is the upstream
# constant `OPPORTUNITY_PROJECTION_SCHEMA`.
#
#   pub struct OpportunityProjectionResponse {
#       schema_version: String, generated_at: String, network: String,
#       applied_view: Option<String>, degraded: bool,
#       source_statuses: Vec<OpportunitySourceStatus>,
#       items: Vec<OpportunityItem>, evidence_boundary: String }
#   pub struct OpportunitySourceStatus {
#       source_type: String, available: bool, authoritative_urls: Vec<String>,
#       item_count: usize, error: Option<String> }
#
# Validating only the items array lets a partial, degraded, wrong-view or
# non-canonical response be treated as safe inventory -- and lets a broken
# source be reported as "no work".
PROJECTION_SCHEMA = "agent-bounties/opportunity-projection-v1"
CANONICAL_SOURCE_TYPE = "canonical_base"
READY_TO_EARN_VIEW = "ready_to_earn"

DEFAULT_STALENESS_SECONDS = 900
# A snapshot timestamped in the future means a broken clock somewhere; treating it
# as "fresh" would let arbitrarily stale data through.
MAX_FUTURE_SKEW_SECONDS = 60

CODING_HINTS = (
    "code", "coding", "api", "cli", "mcp", "sdk", "integration", "environment",
    "checker", "failover", "harness", "benchmark", "agent", "tooling", "fix",
    "implement", "repair", "add", "deterministic", "software", "test", "source",
)

CANONICAL_NETWORKS = ("base-mainnet", "eip155:8453", "base")
CLAIMABLE_STATES = ("open", "claimable", "ready", "ready_to_earn")
OCCUPIED_STATES = ("in_progress", "submitted", "claimed", "exclusive")
ESCROWED_STATES = ("escrowed", "funded", "committed")


class Invalid(Exception):
    """Raised when a value cannot be trusted. Always becomes refresh/skip."""


def money(obj, field):
    """Strictly decode a {amount, decimals, currency} object.

    Never silently yields 0 for malformed input — that is how dimensionally
    invalid economics get accepted. Returns (value, currency, decimals).
    """
    if obj in (None, {}):
        raise Invalid(f"{field} is absent")
    if not isinstance(obj, dict):
        raise Invalid(f"{field} is not an object")
    if "amount" not in obj:
        raise Invalid(f"{field} has no amount")
    if "decimals" not in obj:
        raise Invalid(f"{field} has no decimals (dimensionally invalid)")
    try:
        amount = int(str(obj["amount"]))
        decimals = int(obj["decimals"])
    except (TypeError, ValueError) as exc:
        raise Invalid(f"{field} amount/decimals not integral: {exc}") from exc
    if decimals < 0 or decimals > 36:
        raise Invalid(f"{field} decimals out of range: {decimals}")
    currency = str(obj.get("currency") or "").upper()
    if not currency:
        raise Invalid(f"{field} has no currency")
    return amount / (10 ** decimals), currency, decimals


def snapshot_age(inv):
    """Age in seconds. Raises Invalid when absent, unparseable, or in the future."""
    for key in ("age_seconds", "snapshot_age_seconds"):
        if key in inv:
            value = inv[key]
            if not isinstance(value, (int, float)):
                raise Invalid(f"{key} is not numeric")
            if value < -MAX_FUTURE_SKEW_SECONDS:
                raise Invalid(f"{key} is negative ({value}s): clock skew")
            return float(max(0.0, value))

    stamp = inv.get("generated_at") or inv.get("snapshot_at") or inv.get("as_of")
    if stamp in (None, ""):
        raise Invalid("snapshot carries no freshness field")
    if isinstance(stamp, (int, float)):
        age = time.time() - float(stamp)
    elif isinstance(stamp, str):
        try:
            parsed = _dt.datetime.fromisoformat(stamp.replace("Z", "+00:00"))
        except ValueError as exc:
            raise Invalid(f"freshness not ISO-8601: {stamp!r}") from exc
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=_dt.timezone.utc)
        age = time.time() - parsed.timestamp()
    else:
        raise Invalid("freshness field has an unsupported type")

    # A FUTURE timestamp must not be clamped to "fresh".
    if age < -MAX_FUTURE_SKEW_SECONDS:
        raise Invalid(f"snapshot is timestamped {abs(age):.0f}s in the FUTURE")
    return max(0.0, age)


def is_coding(item):
    blob = " ".join([
        str(item.get("title", "")), str(item.get("goal", "")),
        " ".join(item.get("categories") or []), " ".join(item.get("skills") or []),
    ]).lower()
    return any(hint in blob for hint in CODING_HINTS)


def validate_envelope(inv):
    """Validate the whole OpportunityProjectionResponse before reading items.

    Raises Invalid on any breach. Called BEFORE the items array is inspected so
    that a degraded, wrong-view, non-canonical or partially-covered response can
    never be mistaken for trustworthy inventory -- including the case where the
    list is empty, which would otherwise be reported as a calm "no work".
    """
    if not isinstance(inv, dict):
        raise Invalid("response root is not an object")

    schema = inv.get("schema_version")
    if schema != PROJECTION_SCHEMA:
        raise Invalid(f"schema_version is {schema!r}, expected {PROJECTION_SCHEMA!r}")

    # generated_at is non-optional upstream; without it freshness is unknowable.
    stamp = inv.get("generated_at")
    if not (isinstance(stamp, str) and stamp.strip()):
        raise Invalid("generated_at is absent or not a string")

    network = str(inv.get("network") or "").lower()
    if network not in CANONICAL_NETWORKS:
        raise Invalid(f"response network is {network or 'absent'}, not canonical Base")

    # applied_view is Option<String>: null means the server did NOT apply the
    # ready-to-earn filter, so the payload is unfiltered inventory.
    view = inv.get("applied_view")
    if view != READY_TO_EARN_VIEW:
        raise Invalid(
            f"applied_view is {view!r}, not {READY_TO_EARN_VIEW!r}: "
            "the ready-to-earn filter was not applied"
        )

    degraded = inv.get("degraded")
    if degraded is not False:
        raise Invalid(f"degraded is {degraded!r}, not exactly false")

    statuses = inv.get("source_statuses")
    if not isinstance(statuses, list) or not statuses:
        raise Invalid("source_statuses is absent or empty: coverage is unknown")

    canonical = None
    for status in statuses:
        if not isinstance(status, dict):
            raise Invalid("source_statuses contains a non-object entry")
        if status.get("source_type") == CANONICAL_SOURCE_TYPE:
            canonical = status
        # Any source reporting an error means the projection is incomplete,
        # even if the canonical source itself looks healthy.
        if status.get("error") not in (None, ""):
            raise Invalid(
                f"source {status.get('source_type')!r} reports error "
                f"{status.get('error')!r}: coverage incomplete"
            )
        if status.get("available") is not True:
            raise Invalid(f"source {status.get('source_type')!r} is not available")

    if canonical is None:
        raise Invalid(f"no {CANONICAL_SOURCE_TYPE!r} entry in source_statuses")

    if not isinstance(inv.get("items"), list):
        raise Invalid("items is absent or not an array")

    # Coverage: the canonical source's declared item_count must match the number
    # of canonical items actually delivered. A truncated page is not "no work".
    declared = canonical.get("item_count")
    if not isinstance(declared, int) or isinstance(declared, bool) or declared < 0:
        raise Invalid(f"canonical item_count is {declared!r}, not a count")
    delivered = sum(
        1 for it in inv["items"]
        if isinstance(it, dict) and it.get("source_type") == CANONICAL_SOURCE_TYPE
    )
    if delivered != declared:
        raise Invalid(
            f"canonical coverage incomplete: item_count={declared} but "
            f"{delivered} canonical item(s) delivered"
        )

    if not str(inv.get("evidence_boundary") or "").strip():
        raise Invalid("response declares no evidence_boundary")


def canonical_source(item):
    """Require an explicit canonical Base source, not merely absence of evidence."""
    # source_type is non-optional in OpportunityItem and is the authoritative
    # marker. A Base network plus a 42-char 0x string is NOT canonicity.
    source_type = item.get("source_type")
    if source_type != CANONICAL_SOURCE_TYPE:
        return False, f"source_type is {source_type!r}, not {CANONICAL_SOURCE_TYPE!r}"
    network = str(item.get("network") or "").lower()
    if network not in CANONICAL_NETWORKS:
        return False, f"network is {network or 'absent'}, not canonical Base"
    # discovery_factors is Vec<String> (never null upstream) and the canonical
    # builder always pushes source_type=canonical_base. Absence is suspicious,
    # so require the assertion rather than skipping the check when empty.
    factors = [str(f).lower() for f in (item.get("discovery_factors") or [])]
    if not any(f"source_type={CANONICAL_SOURCE_TYPE}" in f for f in factors):
        return False, "discovery_factors do not assert source_type=canonical_base"
    contract = item.get("source_id") or item.get("bounty_contract") or ""
    if not (isinstance(contract, str) and contract.startswith("0x") and len(contract) == 42):
        return False, "no canonical bounty contract address"
    return True, contract


def verifier_ready(item):
    verifier = item.get("verifier")
    if isinstance(verifier, dict):
        if verifier.get("ready") is not True:
            return False, f"verifier not ready ({verifier.get('reason') or 'unspecified'})"
        return True, str(verifier.get("mode") or "")
    mode = item.get("verification_method") or item.get("verification_mode")
    if not mode:
        return False, "no verification method declared"
    if item.get("verifier_ready") is False:
        return False, "verifier_ready is false"
    if not str(item.get("decision_authority") or "").strip():
        return False, "no decision authority declared"
    return True, str(mode)


def funding_ok(item):
    if item.get("funding_complete") is False or item.get("funded") is False:
        return False, "funding incomplete"
    state = str(item.get("payment_state") or "").lower()
    if state and state not in ESCROWED_STATES:
        return False, f"payment_state is {state}, not escrowed"
    if "payment_committed" in item and item["payment_committed"] is not True:
        return False, "payment not committed"
    if item.get("funded_amount") is not None and item.get("funding_target") is not None:
        have, have_cur, _ = money(item["funded_amount"], "funded_amount")
        want, want_cur, _ = money(item["funding_target"], "funding_target")
        if have_cur != want_cur:
            raise Invalid(f"funding currency mismatch: {have_cur} vs {want_cur}")
        if have < want:
            return False, f"underfunded: {have} of {want} {want_cur}"
    funding = item.get("funding")
    if isinstance(funding, dict) and funding.get("confirmed") and funding.get("required"):
        have, hc, _ = money(funding["confirmed"], "funding.confirmed")
        want, wc, _ = money(funding["required"], "funding.required")
        if hc != wc:
            raise Invalid(f"funding currency mismatch: {hc} vs {wc}")
        if have < want:
            return False, f"underfunded: {have} of {want} {wc}"
    return True, ""


def terms_valid(item):
    terms = item.get("terms")
    if isinstance(terms, dict):
        if not str(terms.get("terms_hash") or "").startswith("0x"):
            return False, "terms present but terms_hash is not a hash"
        return True, ""
    if not str(item.get("evidence_boundary") or "").strip():
        return False, "no terms and no evidence boundary declared"
    if not (item.get("evidence_requirements") or {}):
        return False, "no evidence requirements declared"
    return True, ""


def claim_status(item):
    """(occupied, reclaimable, note) — expiry is evaluated BEFORE occupancy."""
    now = time.time()
    expires = item.get("claim_expires_at")
    if isinstance(expires, (int, float)) and expires > 0:
        if expires <= now:
            return False, True, f"claim lapsed {int(now - expires)}s ago"
        return True, False, f"claim live for {int(expires - now)}s"
    if item.get("claim_expired") is True or item.get("reclaimable") is True:
        return False, True, "record marks the claim expired"
    if item.get("exclusive_claimant") or item.get("active_claimant"):
        return True, False, "exclusive claimant present with no expiry data"
    state = str(item.get("work_state") or "").lower()
    if state in OCCUPIED_STATES:
        return True, False, f"work_state={state}"
    return False, False, ""


def margin_of(item):
    econ = item.get("cash_economics") or {}
    if econ.get("gross_cash_margin") is not None:
        value, currency, _ = money(econ["gross_cash_margin"], "gross_cash_margin")
        return value, currency
    reward_obj = item.get("reward") or econ.get("solver_reward")
    reward, rcur, _ = money(reward_obj, "reward")
    spend_obj = econ.get("required_external_spend")
    if spend_obj in (None, {}):
        return reward, rcur
    spend, scur, _ = money(spend_obj, "required_external_spend")
    if scur != rcur:
        raise Invalid(f"unit mismatch: reward in {rcur}, external spend in {scur}")
    return reward - spend, rcur


def select(inv):
    # ORDER MATTERS. The envelope is validated first, then freshness, and only
    # then the items array. Previously an empty `items` returned `wait` before
    # any coverage check, so a stale/degraded/wrong-view response with a broken
    # canonical source was silently reported as "no work" -- the most dangerous
    # possible false negative for an agent whose job is to find paid work.
    try:
        validate_envelope(inv)
    except Invalid as exc:
        return {
            "action": "refresh",
            "reason": f"projection response failed the production contract: {exc}",
            "next_action": (
                f"Re-fetch {READY_TO_EARN_FEED} and require a complete "
                "OpportunityProjectionResponse (schema_version, Base network, "
                "applied_view=ready_to_earn, degraded=false, available canonical "
                "source with matching item_count). Refusing to read items from a "
                "response that does not meet the contract."
            ),
            "selected": None,
        }

    try:
        age = snapshot_age(inv)
    except Invalid as exc:
        return {
            "action": "refresh",
            "reason": f"freshness coverage unusable: {exc}",
            "next_action": (f"Re-fetch {READY_TO_EARN_FEED} "
                            "with a valid generated_at/age_seconds; failing closed because a snapshot "
                            "of unknown age can hide a live exclusive claim."),
            "selected": None,
        }

    max_age = inv.get("staleness_seconds") or DEFAULT_STALENESS_SECONDS
    try:
        max_age = float(max_age)
    except (TypeError, ValueError):
        max_age = float(DEFAULT_STALENESS_SECONDS)
    if age > max_age:
        return {
            "action": "refresh",
            "reason": f"inventory snapshot is {int(age)}s old (limit {int(max_age)}s)",
            "next_action": ("Re-fetch the canonical ready-to-earn view to obtain a fresh snapshot; "
                            "a stale inventory can hide a live exclusive claim."),
            "selected": None,
        }

    items = inv.get("items") or []

    # Only now -- envelope verified complete and snapshot verified fresh -- is an
    # empty list genuinely "no funded work right now" rather than a hidden fault.
    if not items:
        return {
            "action": "wait",
            "reason": ("inventory contains no opportunities (verified against a complete, "
                       "fresh, non-degraded canonical projection)"),
            "next_action": (f"Re-poll {READY_TO_EARN_FEED} "
                            "and wait for newly funded canonical work before claiming."),
            "selected": None,
        }

    candidates, skipped, invalid = [], [], []

    for item in items:
        title = item.get("title") or item.get("opportunity_id") or "(untitled)"
        state = str(item.get("work_state") or "").lower()

        if state in ("completed", "settled", "cancelled"):
            skipped.append((title, f"work_state={state}"))
            continue
        if not is_coding(item):
            skipped.append((title, "not coding work"))
            continue

        ok, detail = canonical_source(item)
        if not ok:
            skipped.append((title, detail))
            continue
        contract = detail

        occupied, reclaimable, note = claim_status(item)
        if occupied:
            skipped.append((title, f"exclusive claimant active: {note}"))
            continue
        if not reclaimable and state and state not in CLAIMABLE_STATES:
            skipped.append((title, f"work_state={state} is not canonically claimable"))
            continue

        try:
            ok, detail = funding_ok(item)
            if not ok:
                skipped.append((title, detail))
                continue
            margin, currency = margin_of(item)
            bond, bond_cur, _ = money(item.get("bond") or {"amount": "0", "decimals": 6, "currency": currency},
                                      "bond")
            if bond_cur != currency:
                raise Invalid(f"bond in {bond_cur} but reward in {currency}")
            if currency != "USDC":
                raise Invalid(f"reward currency {currency} is not USDC")
        except Invalid as exc:
            invalid.append((title, str(exc)))
            continue

        ok, detail = verifier_ready(item)
        if not ok:
            skipped.append((title, detail))
            continue

        ok, detail = terms_valid(item)
        if not ok:
            skipped.append((title, detail))
            continue

        if margin <= 0:
            skipped.append((title, f"no positive margin ({margin:.6f} {currency})"))
            continue

        candidates.append((margin, bond, item, contract, reclaimable, note))

    # A dimensionally invalid record is a data problem, not a business decision:
    # refresh rather than quietly treating broken money as zero.
    if invalid and not candidates:
        detail = "; ".join(f"{t}: {r}" for t, r in invalid[:4])
        return {
            "action": "refresh",
            "reason": f"dimensionally invalid economics: {detail}",
            "next_action": ("Re-fetch the canonical ready-to-earn view; refusing to claim against "
                            "records whose amounts, decimals, or currencies cannot be validated."),
            "selected": None,
            "invalid": [{"title": t, "reason": r} for t, r in invalid],
        }

    if not candidates:
        detail = "; ".join(f"{t}: {r}" for t, r in skipped[:5]) or "no eligible items"
        return {
            "action": "skip",
            "reason": detail,
            "next_action": ("Skip this inventory: no canonical, funded, verifier-ready coding bounty "
                            "with positive margin and a free claim slot. Re-poll after the next "
                            "funding round."),
            "selected": None,
            "skipped": [{"title": t, "reason": r} for t, r in skipped],
        }

    candidates.sort(key=lambda row: (-row[0], row[1]))
    margin, bond, best, contract, reclaimable, note = candidates[0]

    prefix = ("POST https://api.agentbounties.app/v1/base/autonomous-bounties/expire-claim-plan then "
              if reclaimable else "")
    return {
        "action": "claim",
        "reason": (f"highest positive margin ({margin:.6f} USDC) on canonical funded work with a ready "
                   f"verifier and no live claimant" + (f"; {note}" if note else "")),
        "next_action": (
            prefix
            + "POST https://api.agentbounties.app/v1/base/autonomous-bounties/claim-plan with "
            f'{{"network":"base-mainnet","bounty_contract":"{contract}","solver":"<PUBLIC_WALLET>"}}, '
            "approve the exact bond, then call claim(). Sign externally; never expose key material."
        ),
        "selected": {
            "opportunity_id": best.get("opportunity_id"),
            "title": best.get("title"),
            "bounty_contract": contract,
            "reward_usdc": round(margin, 6),
            "bond_usdc": round(bond, 6),
            "margin_usdc": round(margin, 6),
            "reclaimable": reclaimable,
        },
        "considered": len(items),
        "eligible": len(candidates),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, help="path to an inventory JSON file")
    args = parser.parse_args(argv)

    path = Path(args.input)
    if not path.is_file():
        print(json.dumps({
            "action": "refresh",
            "reason": f"missing inventory file: {path}",
            "next_action": "Fetch a fresh canonical ready-to-earn snapshot before selecting.",
            "selected": None,
        }))
        return 0

    try:
        inv = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        print(json.dumps({
            "action": "refresh",
            "reason": f"invalid inventory JSON: {exc}",
            "next_action": "Re-fetch the inventory; the snapshot is unparseable.",
            "selected": None,
        }))
        return 0

    if not isinstance(inv, dict):
        print(json.dumps({
            "action": "refresh",
            "reason": "inventory root is not an object",
            "next_action": "Re-fetch the canonical ready-to-earn view.",
            "selected": None,
        }))
        return 0

    print(json.dumps(select(inv), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
