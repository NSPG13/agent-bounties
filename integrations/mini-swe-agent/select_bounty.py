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

FAIL-CLOSED DESIGN. Every unknown is a refusal, not a pass. Two distinct
refusals, and the difference is deliberate:

  refresh = the DATA is broken. A required `OpportunityItem` field is missing or
            has the wrong type, money is dimensionally invalid, the envelope
            breaches its contract, or the snapshot's age is unknowable. Nothing
            can be concluded, so re-fetch. A malformed record is NEVER given a
            permissive default.
  skip    = the data is well formed and says "not for you". The record is
            genuinely not canonical, not claimable, not escrowed, not
            verifier-ready, or has no positive margin.

  - missing / unparseable / FUTURE freshness    -> refresh (future clocks are NOT fresh)
  - any required OpportunityItem field missing
    or of the wrong type                        -> refresh
  - contract address not 0x + 40 HEX digits     -> refresh
  - money missing amount/decimals/currency/unit,
    or a unit mismatch between reward and spend -> refresh (never coerce to zero)
  - self-contradictory economics                -> refresh
  - source_type is not exactly canonical_base   -> skip
  - work_state is not exactly `claimable`       -> skip
  - payment_state is not exactly `escrowed`, or
    payment_committed is not exactly true       -> skip
  - funded_amount < funding_target              -> skip
  - verification_ready is not exactly true      -> skip
  - terms absent or invalid                     -> skip
  - claim expiry evaluated BEFORE occupancy, so
    an expired record is reclaimable, not blocked

WALLET SAFETY: never reads, stores, or transmits key material, and never
broadcasts. It emits an unsigned intent for an external signer.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import re
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

# `work_state` is produced by web_public::canonical_opportunity_state and is one
# of exactly: claimable | in_progress | submitted | completed | open. Only
# `claimable` is earnable -- and note that upstream only emits it when the
# bounty is ALSO fully funded, verification-ready and free of validation errors.
# "open" is the catch-all for everything that is not yet claimable (including
# unfunded), so accepting it, as an earlier version did, was wrong.
CLAIMABLE_WORK_STATE = "claimable"
OCCUPIED_STATES = ("in_progress", "submitted", "claimed", "exclusive")
TERMINAL_STATES = ("completed", "settled", "cancelled", "paid")

# `payment_state` is one of: paid | escrowed | seeking_funding. Only `escrowed`
# means the reward is in escrow and not yet paid out. This is the exact value
# the upstream ReadyToEarn view itself requires (opportunities.rs:1263).
ESCROWED_PAYMENT_STATE = "escrowed"

# A 20-byte address. `startswith("0x") and len == 42` is NOT this: "0x" plus 40
# arbitrary characters passes that and is not an address.
ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")


class Invalid(Exception):
    """Raised when a value cannot be trusted. Always becomes refresh/skip."""


def money(obj, field):
    """Strictly decode an upstream `OpportunityAmount`.

    The production struct is
        { amount: String, currency: String, unit: String, decimals: u8 }
    (crates/api/src/opportunities.rs::OpportunityAmount) and EVERY field is
    non-optional, so a missing one is malformed data, not a default.

    Never silently yields 0 for malformed input — that is how dimensionally
    invalid economics get accepted. Returns (value, currency, decimals).
    """
    if obj in (None, {}):
        raise Invalid(f"{field} is absent")
    if not isinstance(obj, dict):
        raise Invalid(f"{field} is not an object")
    for key in ("amount", "currency", "unit", "decimals"):
        if key not in obj:
            raise Invalid(f"{field} has no {key} (not an OpportunityAmount)")
    # `amount` is a String upstream precisely because a u128 of base units does
    # not survive a JSON number. A float here means precision was already lost.
    if isinstance(obj["amount"], float) or isinstance(obj["amount"], bool):
        raise Invalid(f"{field} amount is {obj['amount']!r}, not an integral string")
    try:
        amount = int(str(obj["amount"]))
    except (TypeError, ValueError) as exc:
        raise Invalid(f"{field} amount is not integral: {exc}") from exc
    if not isinstance(obj["decimals"], int) or isinstance(obj["decimals"], bool):
        raise Invalid(f"{field} decimals is {obj['decimals']!r}, not an integer")
    decimals = obj["decimals"]
    if decimals < 0 or decimals > 36:
        raise Invalid(f"{field} decimals out of range: {decimals}")
    currency = str(obj.get("currency") or "").upper()
    if not currency:
        raise Invalid(f"{field} has no currency")
    if not str(obj.get("unit") or "").strip():
        raise Invalid(f"{field} has no unit")
    return amount, currency, decimals, str(obj["unit"])


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

    # Coverage. `item_count` is recorded when the source is READ, before any
    # view/query filtering, and `items` is what survives that filter:
    #
    #   main.rs:4357  item_count: canonical_items.len()   <-- pre-filter
    #   main.rs:4360  items.extend(canonical_items);
    #   main.rs:4362  let items = apply_opportunity_query(items, &query, view, now);
    #
    # `apply_query` retains on source_type/work_state/payment_state, then drops
    # everything the view rejects (ReadyToEarn keeps only claimable + escrowed +
    # payment_committed + verification_ready + positive gross cash margin), then
    # truncates to `limit`. So on a healthy production response delivered is
    # normally FEWER than declared -- 9 canonical bounties on chain, 3 of them
    # ready to earn -- and demanding equality would refuse all real inventory.
    #
    # The sound invariant is the one direction that cannot happen legitimately:
    # a filter can only ever remove items, so delivering MORE canonical items
    # than the source reported reading is an incoherent response, and coverage
    # claims about it cannot be trusted.
    declared = canonical.get("item_count")
    if not isinstance(declared, int) or isinstance(declared, bool) or declared < 0:
        raise Invalid(f"canonical item_count is {declared!r}, not a count")
    delivered = sum(
        1 for it in inv["items"]
        if isinstance(it, dict) and it.get("source_type") == CANONICAL_SOURCE_TYPE
    )
    if delivered > declared:
        raise Invalid(
            f"canonical coverage incoherent: item_count={declared} but "
            f"{delivered} canonical item(s) delivered; a view filter can only "
            "remove items, never add them"
        )

    if not str(inv.get("evidence_boundary") or "").strip():
        raise Invalid("response declares no evidence_boundary")


# ---------------------------------------------------------------------------
# The `OpportunityItem` production contract.
#
# These are the exact non-Option fields of crates/api/src/opportunities.rs::
# OpportunityItem. They are ALWAYS present on a genuine response, so a missing
# one means the payload is not a real projection item and nothing about it can
# be concluded -- including "it is probably fine".
#
# Validating them is the difference between "this record does not qualify" and
# "I cannot tell what this record is". The earlier version conflated the two by
# giving absent fields permissive defaults (`if state and state not in ...`,
# `if "payment_committed" in item and ...`), so DELETING a field made a record
# MORE likely to be claimed than setting it to a bad value. That is fail-open,
# and it is exactly what a compromised projection would exploit.
# ---------------------------------------------------------------------------
REQUIRED_ITEM_FIELDS = {
    "opportunity_id": str,
    "source_type": str,
    "source_id": str,
    "title": str,
    "work_state": str,
    "payment_state": str,
    "payment_committed": bool,
    "decision_authority": str,
    "verification_method": str,
    "verification_ready": bool,
    "reward": dict,
    "funded_amount": dict,
    "funding_target": dict,
    "bond": dict,
    "discovery_factors": list,
    "evidence_boundary": str,
}


class Malformed(Invalid):
    """The record breaches the OpportunityItem contract: refresh, never skip."""


def validate_item_schema(item):
    """Check one candidate against the current `OpportunityItem` contract.

    Raises Malformed on any breach. A malformed record can never be ranked, and
    the caller turns it into a visible `refresh` rather than a quiet `skip`,
    because "the API sent me something that is not an OpportunityItem" is an
    integration fault the operator needs to see.
    """
    if not isinstance(item, dict):
        raise Malformed("item is not an object")

    for field, expected in REQUIRED_ITEM_FIELDS.items():
        if field not in item:
            raise Malformed(f"required field {field!r} is absent")
        value = item[field]
        # bool is a subclass of int; str/list/dict need an exact-ish check.
        if expected is bool:
            if not isinstance(value, bool):
                raise Malformed(f"{field} is {value!r}, not a boolean")
        elif not isinstance(value, expected):
            raise Malformed(
                f"{field} is {type(value).__name__}, not {expected.__name__}"
            )

    # `source_id` is the canonical bounty CONTRACT (opportunities.rs:813,
    # `source_id: item.bounty_contract.clone()`), so it must be a real 20-byte
    # address. "0x" + 40 non-hex characters is not one.
    contract = item["source_id"]
    if not ADDRESS_RE.match(contract):
        raise Malformed(
            f"source_id {contract!r} is not a 20-byte hexadecimal contract address"
        )

    # Every amount must be a well-formed OpportunityAmount. `money` raises
    # Invalid; re-raise as Malformed so a broken amount is a data fault.
    amounts = {}
    for field in ("reward", "funded_amount", "funding_target", "bond"):
        try:
            amounts[field] = money(item[field], field)
        except Invalid as exc:
            raise Malformed(str(exc)) from exc

    # Dimensional coherence: escrow maths across two different currencies or two
    # different decimal scales is meaningless, so refuse to perform it.
    _, funded_cur, funded_dec, funded_unit = amounts["funded_amount"]
    _, target_cur, target_dec, target_unit = amounts["funding_target"]
    if (funded_cur, funded_dec, funded_unit) != (target_cur, target_dec, target_unit):
        raise Malformed(
            f"funded_amount is {funded_cur}/{funded_dec}dp/{funded_unit} but "
            f"funding_target is {target_cur}/{target_dec}dp/{target_unit}"
        )
    _, reward_cur, reward_dec, reward_unit = amounts["reward"]
    _, bond_cur, bond_dec, bond_unit = amounts["bond"]
    if (bond_cur, bond_dec, bond_unit) != (reward_cur, reward_dec, reward_unit):
        raise Malformed(
            f"bond is {bond_cur}/{bond_dec}dp/{bond_unit} but reward is "
            f"{reward_cur}/{reward_dec}dp/{reward_unit}"
        )

    validate_cash_economics(item, amounts)
    return amounts


# Every field of `OpportunityCashEconomics` (opportunities.rs:87-95). All six
# are NON-OPTIONAL in the struct, so a partial object is malformed data, not a
# projection that merely told us less.
CASH_ECONOMICS_AMOUNTS = (
    "solver_reward",
    "refundable_claim_bond",
    "required_external_spend",
    "gross_cash_margin",
)
CASH_ECONOMICS_FIELDS = CASH_ECONOMICS_AMOUNTS + (
    "gross_cash_margin_positive",
    "scope_disclaimer",
)


def validate_cash_economics(item, amounts):
    """Enforce the whole `OpportunityCashEconomics` contract, or refuse.

    `cash_economics` is `Option<OpportunityCashEconomics>` on the item, but the
    struct it wraps has NO optional members. An earlier version validated each
    inner field only `if field in econ`, which is the same fail-open class the
    top-level escrow fields already had removed: replacing the object with just
    `{"gross_cash_margin": ...}` dropped `required_external_spend` from view and
    the record still returned `claim`. An unknown external spend is precisely
    the number that decides whether the work is worth doing, so "absent" must
    never read as "zero".

    Both canonical constructors (opportunities.rs:794 and :1064) always emit
    `Some(...)`; only the `unfunded_offchain` (:547) and `legacy_bounty` (:672)
    paths emit `None`. So for a canonical_base record the object is REQUIRED,
    while a non-canonical record may legitimately omit it (it is skipped for
    not being canonical long before its economics matter).
    """
    econ = item.get("cash_economics")
    canonical = item.get("source_type") == CANONICAL_SOURCE_TYPE
    if econ is None:
        if canonical:
            raise Malformed(
                "canonical_base record has no cash_economics; the canonical "
                "projection always emits it, so this record is not what it claims"
            )
        return None
    if not isinstance(econ, dict):
        raise Malformed("cash_economics is present but is not an object")

    missing = [f for f in CASH_ECONOMICS_FIELDS if f not in econ]
    if missing:
        raise Malformed(
            "cash_economics is present but incomplete: missing "
            + ", ".join(missing)
            + ". OpportunityCashEconomics has no optional members, so a partial "
            "object hides economics rather than reporting them"
        )

    _, reward_cur, reward_dec, reward_unit = amounts["reward"]
    decoded = {}
    for field in CASH_ECONOMICS_AMOUNTS:
        try:
            value, currency, decimals, unit = money(econ[field],
                                                    f"cash_economics.{field}")
        except Invalid as exc:
            raise Malformed(str(exc)) from exc
        # Denomination must match `reward`; otherwise the subtraction below and
        # the ranking that consumes it are comparing different currencies.
        if (currency, decimals, unit) != (reward_cur, reward_dec, reward_unit):
            raise Malformed(
                f"cash_economics.{field} is {currency}/{decimals}dp/{unit} but "
                f"reward is {reward_cur}/{reward_dec}dp/{reward_unit}"
            )
        decoded[field] = value

    if not isinstance(econ["gross_cash_margin_positive"], bool):
        raise Malformed(
            f"cash_economics.gross_cash_margin_positive is "
            f"{econ['gross_cash_margin_positive']!r}, not a boolean"
        )
    if not isinstance(econ["scope_disclaimer"], str):
        raise Malformed("cash_economics.scope_disclaimer is not a string")
    if not econ["scope_disclaimer"].strip():
        raise Malformed(
            "cash_economics.scope_disclaimer is empty; the projection is "
            "contractually required to state what the margin excludes"
        )

    # The two amounts that upstream builds from the SAME source value as their
    # top-level twins must still agree with them:
    #   reward / solver_reward         <- item.solver_reward (:795, :842)
    #   bond   / refundable_claim_bond <- item.claim_bond     (:796, :848)
    # A disagreement means one of the pair was rewritten after construction.
    reward_amount = amounts["reward"][0]
    if decoded["solver_reward"] != reward_amount:
        raise Malformed(
            f"cash_economics.solver_reward is {decoded['solver_reward']} but the "
            f"item reward is {reward_amount}; upstream builds both from the same "
            "value, so they cannot legitimately differ"
        )
    bond_amount = amounts["bond"][0]
    if decoded["refundable_claim_bond"] != bond_amount:
        raise Malformed(
            f"cash_economics.refundable_claim_bond is "
            f"{decoded['refundable_claim_bond']} but the item bond is "
            f"{bond_amount}; upstream builds both from the same value"
        )

    # gross_cash_margin = solver_reward - required_external_spend (:793-800).
    computed = decoded["solver_reward"] - decoded["required_external_spend"]
    if decoded["gross_cash_margin"] != computed:
        raise Malformed(
            f"cash_economics is self-contradictory: gross_cash_margin="
            f"{decoded['gross_cash_margin']} but solver_reward - "
            f"required_external_spend = {computed}"
        )
    # `gross_cash_margin_positive: gross_cash_margin > 0` (:801). The flag is
    # what a careless consumer reads instead of doing the arithmetic, so it is
    # the single most useful field to lie in.
    if econ["gross_cash_margin_positive"] != (decoded["gross_cash_margin"] > 0):
        raise Malformed(
            f"cash_economics.gross_cash_margin_positive is "
            f"{econ['gross_cash_margin_positive']} but gross_cash_margin is "
            f"{decoded['gross_cash_margin']}"
        )
    # Negative external spend would turn a cost into a bonus.
    if decoded["required_external_spend"] < 0:
        raise Malformed(
            f"cash_economics.required_external_spend is "
            f"{decoded['required_external_spend']}; a required spend cannot be negative"
        )
    return decoded


def canonical_source(item):
    """Require an explicit canonical Base source, not merely absence of evidence."""
    # source_type is non-optional in OpportunityItem and is the authoritative
    # marker. A Base network plus a 42-char 0x string is NOT canonicity.
    source_type = item["source_type"]
    if source_type != CANONICAL_SOURCE_TYPE:
        return False, f"source_type is {source_type!r}, not {CANONICAL_SOURCE_TYPE!r}"
    network = str(item.get("network") or "").lower()
    if network not in CANONICAL_NETWORKS:
        return False, f"network is {network or 'absent'}, not canonical Base"
    # discovery_factors is Vec<String> (never null upstream) and the canonical
    # builder always pushes source_type=canonical_base. Absence is suspicious,
    # so require the assertion rather than skipping the check when empty.
    factors = [str(f).lower() for f in item["discovery_factors"]]
    if not any(f"source_type={CANONICAL_SOURCE_TYPE}" in f for f in factors):
        return False, "discovery_factors do not assert source_type=canonical_base"
    return True, item["source_id"]


def verifier_ready(item):
    """`verification_ready` is the API's field name, and it is REQUIRED.

    An earlier version checked a non-schema key `verifier_ready`, which no real
    response ever carries, so the check could never fire on production data.
    The real field is `verification_ready: bool` (opportunities.rs:202) and it
    is the same flag the upstream ReadyToEarn view itself gates on. It must be
    exactly True; the schema validator has already proved it is a boolean.
    """
    if item["verification_ready"] is not True:
        return False, "verification_ready is false: a correct submission cannot settle"
    if not item["verification_method"].strip():
        return False, "no verification_method declared"
    if not item["decision_authority"].strip():
        return False, "no decision authority declared"
    # A nested advisory `verifier` block, when a deployment adds one, may only
    # ever narrow the decision -- it can veto, never overrule verification_ready.
    verifier = item.get("verifier")
    if isinstance(verifier, dict) and verifier.get("ready") is False:
        return False, f"verifier vetoed ({verifier.get('reason') or 'unspecified'})"
    return True, item["verification_method"]


def funding_ok(item, amounts):
    """Require explicit escrow. Every field here was proved present by the schema."""
    state = item["payment_state"].lower()
    if state != ESCROWED_PAYMENT_STATE:
        return False, f"payment_state is {state!r}, not {ESCROWED_PAYMENT_STATE!r}"
    if item["payment_committed"] is not True:
        return False, "payment_committed is false: the reward is not committed"
    have = amounts["funded_amount"][0]
    want = amounts["funding_target"][0]
    if want <= 0:
        return False, f"funding_target is {want} base units: nothing is actually funded"
    if have < want:
        return False, f"underfunded: {have} of {want} base units"
    return True, ""


def terms_valid(item):
    """Require content-addressed terms, or an explicit evidence contract."""
    terms_hash = item.get("terms_hash")
    if terms_hash is not None:
        if not (isinstance(terms_hash, str) and terms_hash.startswith("0x")
                and len(terms_hash) == 66):
            return False, f"terms_hash {terms_hash!r} is not a 32-byte hash"
        return True, ""
    terms = item.get("terms")
    if isinstance(terms, dict):
        if not str(terms.get("terms_hash") or "").startswith("0x"):
            return False, "terms present but terms_hash is not a hash"
        return True, ""
    if not item["evidence_boundary"].strip():
        return False, "no terms and no evidence boundary declared"
    if not (item.get("evidence_requirements") or {}):
        return False, "no evidence requirements declared"
    return True, ""


def claim_status(item):
    """(occupied, reclaimable, note) — expiry is evaluated BEFORE occupancy."""
    now = time.time()
    expires = item.get("claim_expires_at")
    if isinstance(expires, (int, float)) and not isinstance(expires, bool) and expires > 0:
        if expires <= now:
            return False, True, f"claim lapsed {int(now - expires)}s ago"
        return True, False, f"claim live for {int(expires - now)}s"
    if item.get("claim_expired") is True or item.get("reclaimable") is True:
        return False, True, "record marks the claim expired"
    if item.get("exclusive_claimant") or item.get("active_claimant"):
        return True, False, "exclusive claimant present with no expiry data"
    state = item["work_state"].lower()
    if state in OCCUPIED_STATES:
        return True, False, f"work_state={state}"
    return False, False, ""


def margin_of(item, amounts):
    """Gross cash margin in base units.

    By the time this runs, `validate_item_schema` has already enforced the whole
    `OpportunityCashEconomics` contract for a canonical record -- all six fields
    present, denominations matching `reward`, and
    `gross_cash_margin == solver_reward - required_external_spend` agreeing with
    `gross_cash_margin_positive`. So this function does not need permissive
    fallbacks, and must not have them: the old version defaulted a missing
    `required_external_spend` to "no spend" and a missing object to
    `reward_amount`, which is how an omitted cost became an advertised margin.

    Non-canonical records may legitimately carry no `cash_economics`; they are
    skipped for not being canonical, and are never ranked on this number.
    """
    econ = item.get("cash_economics")
    reward_amount, reward_cur, reward_dec, reward_unit = amounts["reward"]

    if econ is None:
        # Only reachable for a non-canonical record. Refuse rather than invent a
        # margin: an unknown external spend is not a zero external spend.
        raise Invalid(
            "cash_economics is absent, so gross cash margin is unknown; a "
            "margin is never assumed from the reward alone"
        )

    # Re-decode rather than trusting a value threaded through from elsewhere, so
    # this function is correct on its own terms. Validation already proved these
    # parse, agree dimensionally, and are internally consistent.
    declared, currency, decimals, unit = money(econ["gross_cash_margin"],
                                               "gross_cash_margin")
    if (currency, decimals, unit) != (reward_cur, reward_dec, reward_unit):
        raise Invalid(
            f"gross_cash_margin is {currency}/{decimals}dp/{unit} but reward is "
            f"{reward_cur}/{reward_dec}dp/{reward_unit}"
        )
    solver, _, _, _ = money(econ["solver_reward"], "solver_reward")
    spend, _, _, _ = money(econ["required_external_spend"],
                           "required_external_spend")
    computed = solver - spend
    if declared != computed:
        raise Invalid(
            f"cash_economics is self-contradictory: gross_cash_margin={declared} "
            f"but solver_reward - required_external_spend = {computed}"
        )
    return declared / (10 ** reward_dec), reward_cur, reward_dec


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

    candidates, skipped, malformed = [], [], []

    for raw in items:
        title = "(untitled)"
        if isinstance(raw, dict):
            title = raw.get("title") or raw.get("opportunity_id") or "(untitled)"

        # SCHEMA FIRST. Nothing about a record may be read as a business signal
        # until it is proved to BE an OpportunityItem. A missing required field
        # is a data fault (refresh), never a permissive default, and never a
        # quiet skip -- the earlier ordering let a record with `verification_ready`,
        # `payment_state`, `payment_committed`, `funded_amount`, `funding_target`
        # and `bond` all DELETED reach `claim`.
        try:
            amounts = validate_item_schema(raw)
        except Malformed as exc:
            malformed.append((title, str(exc)))
            continue
        item = raw
        state = item["work_state"].lower()

        if state in TERMINAL_STATES:
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
        # `claimable` is the ONLY earnable work_state upstream emits, and it
        # already implies fully funded + verification-ready + no validation
        # errors. A reclaimable record is the one exception: its own state is
        # stale precisely because the previous claim lapsed.
        if not reclaimable and state != CLAIMABLE_WORK_STATE:
            skipped.append((title, f"work_state={state!r} is not "
                                   f"{CLAIMABLE_WORK_STATE!r}"))
            continue

        try:
            ok, detail = funding_ok(item, amounts)
            if not ok:
                skipped.append((title, detail))
                continue
            margin, currency, _ = margin_of(item, amounts)
            bond_units, bond_cur, bond_dec, _ = amounts["bond"]
            bond = bond_units / (10 ** bond_dec)
            if currency != "USDC":
                raise Invalid(f"reward currency {currency} is not USDC")
            # The solver bond is posted from the agent's own wallet. A record
            # advertising a zero bond is either not a real canonical bounty or
            # is understating what the claim will actually cost.
            if bond_units <= 0:
                skipped.append((title, f"bond is {bond_units} base units; a canonical "
                                       "claim always requires a positive bond"))
                continue
        except Invalid as exc:
            malformed.append((title, str(exc)))
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

    # A record that breaches the OpportunityItem contract is a DATA fault, not a
    # business decision, and it is reported even when other records look fine.
    #
    # This is deliberately dominant. Every item in a genuine response is built by
    # one upstream constructor, so a single malformed record means the payload is
    # not what it claims to be -- truncated, rewritten in transit, or served by
    # something that is not the canonical API. Ranking the "good-looking" rest of
    # such a payload would be trusting an attacker to have corrupted only the
    # parts that do not matter.
    if malformed:
        detail = "; ".join(f"{t}: {r}" for t, r in malformed[:4])
        return {
            "action": "refresh",
            "reason": f"response contains records that are not valid OpportunityItems: {detail}",
            "next_action": (
                f"Re-fetch {READY_TO_EARN_FEED} and require every item to satisfy the "
                "OpportunityItem contract (source_type, source_id as a 20-byte hex "
                "address, work_state, payment_state, payment_committed, "
                "verification_ready, and well-formed reward/funded_amount/"
                "funding_target/bond amounts). Refusing to rank any record from a "
                "payload that contains a malformed one."
            ),
            "selected": None,
            "malformed": [{"title": t, "reason": r} for t, r in malformed],
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
