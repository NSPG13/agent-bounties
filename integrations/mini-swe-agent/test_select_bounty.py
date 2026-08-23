#!/usr/bin/env python3
"""Focused selector tests for the mini-SWE-agent paid-work environment.

The immutable acceptance benchmark only exercises the five original fixtures.
These tests cover the adversarial cases the review asked for, plus the
canonical-feed invariant, and are runnable standalone:

    python -B integrations/mini-swe-agent/test_select_bounty.py

Every assertion here is a *fail-closed* assertion: the selector must refuse
(skip/refresh) rather than emit `claim` whenever a canonical precondition is
missing or unverifiable.
"""

from __future__ import annotations

import importlib.util
import json
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from urllib.parse import parse_qs, urlparse

HERE = Path(__file__).resolve().parent
SELECTOR = HERE / "select_bounty.py"
FIXTURES = HERE / "fixtures"
CONFIG = HERE / "config.yaml"

config_src = CONFIG.read_text(encoding="utf-8")

# Import the selector once, up front. Several assertions below need the runtime
# constants (the feed URL is built by concatenation, so a text scan cannot see
# it; the item contract is a dict). Scanning source text instead would let a
# renamed or deleted constant pass silently.
spec = importlib.util.spec_from_file_location("select_bounty", SELECTOR)
if spec is None or spec.loader is None:
    raise SystemExit(f"cannot import selector module from {SELECTOR}")
selector_mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(selector_mod)

# fixture -> (expected action, why refusing matters)
#
# TWO DISTINCT REFUSALS, and mixing them up is the bug this suite exists to
# catch:
#   skip    -> the record is a VALID OpportunityItem that does not qualify.
#   refresh -> the record BREACHES the OpportunityItem contract, so nothing can
#              be concluded about it. A missing required field must land here,
#              never in `skip` and certainly never in `claim`.
EXPECTATIONS = {
    # original five
    "multiple.json": ("claim", "the one fully canonical, funded, verifier-ready record"),
    "empty.json": ("wait", "no inventory to act on"),
    "stale.json": ("refresh", "snapshot older than the staleness budget"),
    "no-margin.json": ("skip", "margin is not positive after bond and spend"),
    "exclusive-claimant.json": ("skip", "a live exclusive claim must never be contested"),
    # adversarial cases added for the review
    "adversarial-future-timestamp.json": (
        "refresh",
        "a future timestamp is clock skew, never 'fresh'",
    ),
    "adversarial-missing-freshness.json": (
        "refresh",
        "unknown snapshot age can hide a live claim",
    ),
    "adversarial-unit-mismatch.json": (
        "refresh",
        "amounts in different currencies are not comparable",
    ),
    "adversarial-malformed-money.json": (
        "refresh",
        "malformed money must raise, never coerce to zero",
    ),
    "adversarial-non-canonical-source.json": (
        "skip",
        "absence of contrary evidence is not proof of canonicity",
    ),
    "adversarial-verifier-unready.json": (
        "skip",
        "a correct submission cannot settle against an unready verifier",
    ),
    "adversarial-underfunded.json": (
        "skip",
        "funded_amount below funding_target risks the solver bond",
    ),
    "adversarial-expired-claim.json": (
        "claim",
        "a lapsed claim_expires_at deadline makes the row reclaimable",
    ),
    # `deadline_kind` is what makes a timestamp mean "claim expiry". A past
    # FUNDING deadline says nothing about occupancy, so this record stays
    # occupied. Without this case, reading `deadline` and ignoring its kind
    # would pass.
    "adversarial-past-funding-deadline.json": (
        "skip",
        "a past funding_deadline is not a lapsed claim; only deadline_kind decides",
    ),
    "adversarial-item-deadline-without-kind.json": (
        "refresh",
        "the projection sets deadline and deadline_kind together or not at all",
    ),
    "adversarial-item-deadline-kind-unknown.json": (
        "refresh",
        "an unrecognised deadline_kind leaves the timestamp's meaning unknown",
    ),
    # One fixture per field the real schema does not define. These run through
    # the CLI, so they cover the shipped end-to-end path as well as the
    # in-process probes further down.
    "adversarial-item-ghost-field-claim-expires-at.json": (
        "refresh", "claim_expires_at is not an OpportunityItem field"),
    "adversarial-item-ghost-field-claim-expired.json": (
        "refresh", "claim_expired is not an OpportunityItem field"),
    "adversarial-item-ghost-field-reclaimable.json": (
        "refresh", "reclaimable is not an OpportunityItem field"),
    "adversarial-item-ghost-field-exclusive-claimant.json": (
        "refresh", "exclusive_claimant is not an OpportunityItem field"),
    "adversarial-item-ghost-field-active-claimant.json": (
        "refresh", "active_claimant is not an OpportunityItem field"),
    "adversarial-item-ghost-field-terms.json": (
        "refresh", "a nested terms object is not an OpportunityItem field"),
    "adversarial-item-ghost-field-verifier.json": (
        "refresh", "a nested verifier block is not an OpportunityItem field"),
    "adversarial-item-ghost-field-verifier-ready.json": (
        "refresh", "verifier_ready was always a misspelling of verification_ready"),
    "adversarial-item-ghost-field-ready-to-earn.json": (
        "refresh", "ready_to_earn is not an OpportunityItem field"),
    # response-level production contract (OpportunityProjectionResponse)
    "adversarial-envelope-bad-schema.json": (
        "refresh", "an unknown schema_version cannot be interpreted safely"),
    "adversarial-envelope-wrong-network.json": (
        "refresh", "a non-Base projection is not canonical inventory"),
    "adversarial-envelope-wrong-view.json": (
        "refresh", "applied_view=null means the ready-to-earn filter was never applied"),
    "adversarial-envelope-degraded.json": (
        "refresh", "degraded=true means the projection is knowingly partial"),
    "adversarial-envelope-source-error.json": (
        "refresh", "a source reporting an error makes coverage incomplete"),
    "adversarial-envelope-source-unavailable.json": (
        "refresh", "an unavailable canonical source cannot back a claim decision"),
    "adversarial-envelope-incoherent-coverage.json": (
        "refresh", "more canonical items delivered than read is incoherent"),
    "adversarial-envelope-no-source-statuses.json": (
        "refresh", "without source_statuses, coverage is entirely unknown"),
    "adversarial-item-spoofed-canonicity.json": (
        # skip, not refresh: the record is a COMPLETE, well-formed
        # OpportunityItem -- Base network, real 20-byte address,
        # discovery_factors even asserting canonical_base -- and only
        # source_type, the authoritative marker, disagrees. That is a merits
        # refusal, not a data fault. The refusal must still be absolute.
        "skip", "network + 0x address is not canonicity; source_type is authoritative"),
    # the headline regression: an empty list must NOT be reported as "no work"
    # when the response itself is untrustworthy.
    "adversarial-empty-with-broken-source.json": (
        "refresh", "an empty list from a broken source is a fault, not 'no work'"),
    "adversarial-empty-degraded.json": (
        "refresh", "an empty list from a degraded projection is a fault, not 'no work'"),
    "adversarial-empty-stale.json": (
        "refresh", "an empty list from a stale snapshot is a fault, not 'no work'"),

    # ------------------------------------------------------------------
    # ITEM-LEVEL production contract (OpportunityItem). The review found the
    # selector still returned `claim` when verification_ready was false, when
    # the escrow fields were DELETED, when the contract address was "0x" + 40
    # non-hex characters, and when the bond was removed. One case each.
    # ------------------------------------------------------------------
    # (a) well-formed but disqualifying -> skip
    "adversarial-item-not-escrowed.json": (
        "skip", "payment_state must be exactly 'escrowed'; the reward is not in escrow"),
    "adversarial-item-payment-uncommitted.json": (
        "skip", "payment_committed=false means the escrow is not committed"),
    "adversarial-item-zero-bond.json": (
        "skip", "a canonical claim always costs a positive bond; zero understates it"),
    "adversarial-item-work-state-open.json": (
        "skip", "'open' is upstream's catch-all for not-yet-claimable, incl. unfunded"),
    # (b) contract breaches -> refresh, never a permissive default
    "adversarial-item-no-verification-ready.json": (
        "refresh", "a DELETED required field must not be safer than a false one"),
    "adversarial-item-verification-ready-not-boolean.json": (
        "refresh", "the string 'true' is not the boolean the schema requires"),
    "adversarial-item-no-escrow-fields.json": (
        "refresh", "deleting payment_state/committed/funded/target proves nothing"),
    "adversarial-item-no-bond.json": (
        "refresh", "an absent bond cannot be reported as a zero bond"),
    "adversarial-item-bad-contract-address.json": (
        "refresh", "'0x' + 40 non-hex characters is not a 20-byte address"),
    "adversarial-item-short-contract-address.json": (
        "refresh", "a truncated address is not a 20-byte address"),
    "adversarial-item-amount-no-unit.json": (
        "refresh", "an OpportunityAmount without `unit` is dimensionally incomplete"),
    "adversarial-item-contradictory-economics.json": (
        "refresh", "a precomputed margin that contradicts its own components"),
    "adversarial-item-funding-decimal-mismatch.json": (
        "refresh", "comparing 6dp funding against an 18dp target is meaningless"),
    # (c) partial/incoherent `cash_economics`. The review reproduced a `claim`
    # on an object rewritten down to `gross_cash_margin` alone, which hides the
    # external spend that decides whether the work is worth doing.
    "adversarial-item-economics-only-margin.json": (
        "refresh", "an object with 1 of 6 required members reports nothing"),
    "adversarial-item-economics-no-external-spend.json": (
        "refresh", "an unknown external spend is not a zero external spend"),
    "adversarial-item-economics-empty.json": (
        "refresh", "an empty object is not 'economics unavailable'"),
    "adversarial-item-economics-absent.json": (
        "refresh", "both canonical constructors emit Some(...), so absence contradicts source_type"),
    "adversarial-item-economics-flag-lies.json": (
        "refresh", "gross_cash_margin_positive=true over a negative margin"),
    "adversarial-item-bond-disagrees-with-economics.json": (
        "refresh", "bond and refundable_claim_bond come from one upstream value"),
}

# The parameters the API actually deserializes. See
# crates/api/src/opportunities.rs::OpportunityQuery.
SUPPORTED_QUERY_KEYS = {
    "network",
    "view",
    "source_type",
    "work_state",
    "payment_state",
    "limit",
}
REQUIRED_FILTER = {
    "network": "base-mainnet",
    "view": "ready_to_earn",
    "source_type": "canonical_base",
}

failures: list[str] = []
passes = 0


def check(condition: bool, label: str) -> None:
    global passes
    if condition:
        passes += 1
        print(f"  PASS  {label}")
    else:
        failures.append(label)
        print(f"  FAIL  {label}")


def run(fixture: Path) -> dict:
    completed = subprocess.run(
        [sys.executable, "-B", str(SELECTOR), "--input", str(fixture)],
        cwd=HERE.parents[1],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=20,
        check=False,
    )
    if completed.returncode != 0:
        raise AssertionError(
            f"selector exited {completed.returncode} for {fixture.name}:\n"
            f"{completed.stdout[-2000:]}"
        )
    return json.loads(completed.stdout)


print("=== every fixture returns its exact expected action ===")
for name, (expected, rationale) in EXPECTATIONS.items():
    path = FIXTURES / name
    if not path.is_file():
        check(False, f"{name} exists")
        continue
    try:
        result = run(path)
    except (AssertionError, json.JSONDecodeError) as error:
        check(False, f"{name} runs and emits JSON ({error})")
        continue
    check(result.get("action") == expected, f"{name} -> {expected} ({rationale})")
    check(
        bool(str(result.get("next_action", "")).strip()),
        f"{name} emits exactly one next_action",
    )

print()
print("=== no adversarial fixture is ever allowed to reach 'claim' ===")
for name, (expected, _rationale) in EXPECTATIONS.items():
    if not name.startswith("adversarial-") or expected == "claim":
        continue
    path = FIXTURES / name
    if not path.is_file():
        continue
    result = run(path)
    check(result.get("action") != "claim", f"{name} never claims")

print()
print("=== reclaimable expired record prefixes expireClaim ===")
expired = FIXTURES / "adversarial-expired-claim.json"
if expired.is_file():
    result = run(expired)
    check(result.get("action") == "claim", "expired claim is reclaimable, not blocked")
    check(
        "expire-claim" in str(result.get("next_action", "")).lower(),
        "next_action reopens the expired claim before claiming",
    )

print()
print("=== a view-filtered subset is normal, not a coverage fault ===")
# Regression pin. `item_count` is recorded when the source is READ
# (main.rs:4357, pre-filter); `items` is what survives `apply_opportunity_query`
# (main.rs:4362). Under view=ready_to_earn the delivered canonical count is
# normally SMALLER than the declared one. An earlier version of this checker
# demanded equality, which would have refused every real production response
# while every fixture still passed -- the fixtures happened to encode
# declared == delivered. This case fails if that equality rule ever returns.
subset = FIXTURES / "view-filtered-subset.json"
if not subset.is_file():
    check(False, "view-filtered-subset.json exists")
else:
    payload = json.loads(subset.read_text())
    declared = next(
        s["item_count"] for s in payload["source_statuses"]
        if s["source_type"] == "canonical_base"
    )
    delivered = sum(
        1 for i in payload["items"] if i.get("source_type") == "canonical_base"
    )
    # Guard against the fixture drifting into a vacuous shape: if declared ever
    # equals delivered, this case would pass under the buggy rule too.
    check(delivered < declared,
          f"fixture actually exercises a filtered subset (declared={declared} > "
          f"delivered={delivered})")
    result = run(subset)
    check(result.get("action") == "claim",
          "a healthy pre-filter item_count above the delivered count still selects work")
    check(result.get("selected") is not None,
          "view-filtered subset selects a bounty")

print()
print("=== an untrustworthy response is never reported as 'no work' ===")
# This is the regression the review caught: `wait` means "the canonical feed is
# healthy and there is genuinely nothing funded". It must be unreachable when
# the projection is degraded, stale, truncated, or backed by a broken source.
for name in (
    "adversarial-empty-with-broken-source.json",
    "adversarial-empty-degraded.json",
    "adversarial-empty-stale.json",
    "adversarial-envelope-incoherent-coverage.json",
):
    path = FIXTURES / name
    if not path.is_file():
        check(False, f"{name} exists")
        continue
    result = run(path)
    check(result.get("action") != "wait", f"{name} does not report a false 'no work'")
    check(result.get("action") == "refresh", f"{name} fails visibly with refresh")
    check(result.get("selected") is None, f"{name} selects nothing")

print()
print("=== every envelope breach fails closed before items are read ===")
for name in EXPECTATIONS:
    if not name.startswith(("adversarial-envelope-", "adversarial-empty-",
                            "adversarial-item-")):
        continue
    path = FIXTURES / name
    if not path.is_file():
        continue
    check(run(path).get("action") != "claim", f"{name} never claims")

print()
print("=== an exact canonical response still selects correctly ===")
# Guard against over-tightening: the strict envelope must not break the happy
# path. multiple.json carries a complete, exact, non-degraded envelope.
good = run(FIXTURES / "multiple.json")
check(good.get("action") == "claim", "exact canonical projection still claims")
selected = good.get("selected") or {}
check(str(selected.get("bounty_contract", "")).startswith("0x"),
      "selection still resolves a canonical bounty contract")
check(float(selected.get("margin_usdc") or 0) > 0,
      "selection still requires a positive margin")

print()
print("=== DIRECT PROBES: mutate one field of a known-good record ===")
# These are the probes the maintainer review ran by hand against the previous
# head, where each of them still returned `claim`. Unlike the fixture table
# these are generated in-process from the exact record that DOES claim, so a
# passing probe cannot be an artefact of some second difference in a hand-
# written file. Each probe changes exactly one thing.
GOOD = json.loads((FIXTURES / "multiple.json").read_text())
GOOD_ITEM = next(i for i in GOOD["items"]
                 if i["opportunity_id"].endswith("0xa2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2"))

PROBE_DIR = Path(tempfile.mkdtemp(prefix="mini-swe-probes-"))


def probe(label: str, expected: set, **changes) -> None:
    """Run the selector over a single-item envelope with `changes` applied."""
    record = json.loads(json.dumps(GOOD_ITEM))
    for key, value in changes.items():
        if value is _DELETE:
            record.pop(key, None)
        else:
            record[key] = value
    payload = json.loads(json.dumps(GOOD))
    payload["items"] = [record]
    payload["source_statuses"][0]["item_count"] = 1
    path = PROBE_DIR / f"probe-{abs(hash(label)):x}.json"
    path.write_text(json.dumps(payload), encoding="utf-8")
    result = run(path)
    action = result.get("action")
    check(action in expected, f"{label} -> {'/'.join(sorted(expected))} (got {action!r})")
    # A refusal must not smuggle a selection out alongside it. When the probe
    # legitimately expects `claim` (the coherent-control cases), the opposite
    # is required: something must actually have been chosen.
    if expected == {"claim"}:
        check(result.get("selected") is not None, f"  {label} selects a record")
    else:
        check(result.get("selected") is None, f"  {label} selects nothing")


_DELETE = object()

# Sanity: the unmutated record really does claim, so every refusal below is
# caused by the mutation and nothing else. Without this the probes could all
# "pass" simply because the baseline never claimed.
baseline = json.loads(json.dumps(GOOD))
baseline["items"] = [GOOD_ITEM]
baseline["source_statuses"][0]["item_count"] = 1
_baseline_path = PROBE_DIR / "baseline.json"
_baseline_path.write_text(json.dumps(baseline), encoding="utf-8")
_baseline = run(_baseline_path)
check(_baseline.get("action") == "claim",
      f"PROBE BASELINE: the unmutated record claims (got {_baseline.get('action')!r})")

REFUSE = {"skip", "refresh"}
# The four the review reproduced on the previous head.
probe("verification_ready=false", REFUSE, verification_ready=False)
probe("escrow fields DELETED", {"refresh"},
      payment_state=_DELETE, payment_committed=_DELETE,
      funded_amount=_DELETE, funding_target=_DELETE)
probe("contract is 0x + 40 NON-HEX chars", {"refresh"}, source_id="0x" + "z" * 40)
probe("bond DELETED", {"refresh"}, bond=_DELETE)
# And the rest of the required contract, one field at a time.
for _field in sorted(("opportunity_id", "source_type", "source_id", "title",
                      "work_state", "payment_state", "payment_committed",
                      "decision_authority", "verification_method",
                      "verification_ready", "reward", "funded_amount",
                      "funding_target", "bond", "discovery_factors",
                      "evidence_boundary")):
    probe(f"required field {_field!r} DELETED", {"refresh"}, **{_field: _DELETE})

probe("payment_state='seeking_funding'", {"skip"}, payment_state="seeking_funding")
probe("payment_committed=false", {"skip"}, payment_committed=False)
probe("payment_committed='true' (string)", {"refresh"}, payment_committed="true")
probe("funded_amount < funding_target", {"skip"},
      funded_amount={"amount": "1", "currency": "USDC", "unit": "base_units",
                     "decimals": 6})
# Upstream builds `bond` and `cash_economics.refundable_claim_bond` from the one
# `item.claim_bond` value (opportunities.rs:796, :848). So zeroing ONLY the
# top-level field is a record upstream cannot have produced -- a data fault
# (`refresh`), not a cheap bounty...
probe("bond zeroed but refundable_claim_bond left at 10000", {"refresh"},
      bond={"amount": "0", "currency": "USDC", "unit": "base_units", "decimals": 6})
# ...whereas a COHERENT zero bond breaches nothing and is refused on its merits.
probe("bond is zero in both places (coherent)", {"skip"},
      bond={"amount": "0", "currency": "USDC", "unit": "base_units", "decimals": 6},
      cash_economics=dict(
          json.loads(json.dumps(GOOD_ITEM["cash_economics"])),
          refundable_claim_bond={"amount": "0", "currency": "USDC",
                                 "unit": "base_units", "decimals": 6}))
probe("bond in a different currency", {"refresh"},
      bond={"amount": "10000", "currency": "DAI", "unit": "base_units", "decimals": 6})
probe("reward amount is a float", {"refresh"},
      reward={"amount": 2.0, "currency": "USDC", "unit": "base_units", "decimals": 6})
probe("work_state='open'", {"skip"}, work_state="open")
probe("work_state='completed'", {"skip"}, work_state="completed")
probe("source_type='github_discovery'", {"skip"}, source_type="github_discovery")
probe("terms_hash is not a 32-byte hash", {"skip"}, terms_hash="0xdeadbeef")

print()
print("=== a PARTIAL cash_economics object is corrupt data, not less data ===")
# `OpportunityCashEconomics` (opportunities.rs:87-95) has six NON-optional
# members. The previous head validated each inner field only `if field in econ`,
# so deleting one simply removed it from scrutiny -- the exact fail-open class
# already removed from the top-level escrow fields. The review reproduced a
# `claim` on an object rewritten down to `gross_cash_margin` alone.
_ECON = GOOD_ITEM["cash_economics"]


def _econ_without(*dropped: str) -> dict:
    return {k: v for k, v in _ECON.items() if k not in dropped}


# The maintainer's two named probes, verbatim.
probe("cash_economics = ONLY gross_cash_margin", {"refresh"},
      cash_economics={"gross_cash_margin": _ECON["gross_cash_margin"]})
probe("cash_economics missing ONLY required_external_spend", {"refresh"},
      cash_economics=_econ_without("required_external_spend"))
# ...and each remaining member, one at a time, so no single omission is safe.
for _member in sorted(_ECON):
    probe(f"cash_economics missing {_member!r}", {"refresh"},
          cash_economics=_econ_without(_member))
probe("cash_economics = {} (empty object)", {"refresh"}, cash_economics={})
probe("cash_economics DELETED from a canonical record", {"refresh"},
      cash_economics=_DELETE)
probe("cash_economics is a list, not an object", {"refresh"}, cash_economics=[])
probe("cash_economics is a string", {"refresh"}, cash_economics="none")

print()
print("=== ...and a COMPLETE one must still be internally coherent ===")


def _econ_with(**changes) -> dict:
    out = json.loads(json.dumps(_ECON))
    out.update(changes)
    return out


_USDC = {"currency": "USDC", "unit": "base_units", "decimals": 6}
# The flag is what a careless consumer reads instead of doing the subtraction,
# so it is the single most valuable field to lie in.
probe("gross_cash_margin_positive=True over a NEGATIVE margin", {"refresh"},
      cash_economics=_econ_with(
          required_external_spend={"amount": "1990000", **_USDC},
          gross_cash_margin={"amount": "-1000000", **_USDC},
          gross_cash_margin_positive=True))
probe("gross_cash_margin_positive=False over a POSITIVE margin", {"refresh"},
      cash_economics=_econ_with(gross_cash_margin_positive=False))
probe("gross_cash_margin_positive is the string 'true'", {"refresh"},
      cash_economics=_econ_with(gross_cash_margin_positive="true"))
# margin must equal solver_reward - required_external_spend (:793-800).
probe("margin contradicts its own components", {"refresh"},
      cash_economics=_econ_with(
          required_external_spend={"amount": "500000", **_USDC}))
probe("required_external_spend is NEGATIVE (a cost as a bonus)", {"refresh"},
      cash_economics=_econ_with(
          required_external_spend={"amount": "-1000000", **_USDC},
          gross_cash_margin={"amount": "3000000", **_USDC}))
# Upstream builds these pairs from ONE value each, so they cannot differ:
#   reward / solver_reward         <- item.solver_reward (:795, :842)
#   bond   / refundable_claim_bond <- item.claim_bond    (:796, :848)
probe("solver_reward disagrees with the top-level reward", {"refresh"},
      cash_economics=_econ_with(solver_reward={"amount": "99000000", **_USDC}))
probe("refundable_claim_bond disagrees with the top-level bond", {"refresh"},
      cash_economics=_econ_with(refundable_claim_bond={"amount": "1", **_USDC}))
# Denomination: an 18dp or foreign-currency amount cannot be subtracted from a
# 6dp USDC reward, and must not be ranked against one.
probe("solver_reward denominated in EUR", {"refresh"},
      cash_economics=_econ_with(
          solver_reward={"amount": "2000000", "currency": "EUR",
                         "unit": "base_units", "decimals": 6}))
probe("required_external_spend carries 18 decimals", {"refresh"},
      cash_economics=_econ_with(
          required_external_spend={"amount": "0", "currency": "USDC",
                                   "unit": "base_units", "decimals": 18}))
probe("scope_disclaimer is empty", {"refresh"},
      cash_economics=_econ_with(scope_disclaimer="   "))
probe("scope_disclaimer is not a string", {"refresh"},
      cash_economics=_econ_with(scope_disclaimer=7))
# Control: a coherent record with a genuinely different (still positive)
# margin must STILL claim, proving the rules above reject incoherence rather
# than simply rejecting anything that is not the baseline object.
probe("coherent economics with a smaller positive margin", {"claim"},
      cash_economics=_econ_with(
          required_external_spend={"amount": "1000000", **_USDC},
          gross_cash_margin={"amount": "1000000", **_USDC},
          gross_cash_margin_positive=True))

print()
print("=== fields OpportunityItem does not define are refused outright ===")
# THE BUG THIS BLOCK EXISTS FOR. `claim_status`, `terms_valid` and
# `verifier_ready` all read keys production has never emitted, and read them
# BEFORE the honest signal. So an attacker did not need to forge a plausible
# record -- appending one unknown key OVERRODE the real one:
#
#   work_state=in_progress                      -> skip   (correct)
#   work_state=in_progress + claim_expires_at   -> claim   (guard defeated)
#   terms_hash="0xdead"                         -> skip   (correct)
#   terms_hash="0xdead" + terms={...}           -> claim   (guard defeated)
#
# This is the third time this selector has read a non-existent field
# (?ready_to_earn=true, then verifier_ready), so it is now closed as a class:
# any unknown key from FORBIDDEN_ITEM_FIELDS is malformed data.
_forbidden = getattr(selector_mod, "FORBIDDEN_ITEM_FIELDS", ())
_required = getattr(selector_mod, "REQUIRED_ITEM_FIELDS", {}) or {}
check(bool(_forbidden), "selector exports FORBIDDEN_ITEM_FIELDS")
for _name in ("claim_expires_at", "claim_expired", "reclaimable",
              "exclusive_claimant", "active_claimant", "terms", "verifier",
              "verifier_ready", "ready_to_earn"):
    check(_name in _forbidden, f"{_name} is refused as a non-schema field")
    # Cross-check the claim against upstream rather than trusting the list:
    # every one of these must be absent from the real REQUIRED_ITEM_FIELDS.
    check(_name not in _required,
          f"  {_name} is not simultaneously required and forbidden")

# Each ghost field injected on a record that otherwise CLAIMS, so the refusal
# is caused by the injection alone (the baseline above proves it claims).
probe("ghost claim_expires_at cannot resurrect an occupied record",
      {"refresh"}, work_state="in_progress", claim_expires_at=1)
probe("ghost claim_expired", {"refresh"}, claim_expired=True)
probe("ghost reclaimable", {"refresh"}, reclaimable=True)
probe("ghost exclusive_claimant", {"refresh"},
      exclusive_claimant="0x8cfb0c37af0c40f96c44fd45fdec30b430bc6a6e")
probe("ghost active_claimant", {"refresh"},
      active_claimant="0x8cfb0c37af0c40f96c44fd45fdec30b430bc6a6e")
probe("ghost terms cannot substitute for a bad terms_hash",
      {"refresh"}, terms_hash="0xdead", terms={"terms_hash": "0xabc"})
probe("ghost verifier block", {"refresh"}, verifier={"ready": True})
probe("ghost verifier_ready (the old misspelling)", {"refresh"},
      verifier_ready=True)
probe("ghost ready_to_earn", {"refresh"}, ready_to_earn=True)

# The exploits exactly as reproduced, asserted end to end: the honest record
# refuses, and the injected one must NOT become claimable. Both halves matter --
# without the honest baseline the second assertion could pass vacuously.
probe("occupied record, honestly", REFUSE, work_state="in_progress")
probe("bad terms_hash, honestly", REFUSE, terms_hash="0xdead")

print()
print("=== deadline_kind, not deadline, decides what a timestamp means ===")
# claim_status must not read a bare `deadline`. Only `deadline_kind ==
# "claim_expires_at"` makes it a claim expiry (web-public/src/lib.rs:464-502);
# a past funding_deadline on an occupied row is not a lapsed claim.
_claim_status = selector_mod.claim_status
_occupied = json.loads(json.dumps(GOOD_ITEM))
_occupied["work_state"] = "in_progress"

_o = json.loads(json.dumps(_occupied))
check(_claim_status(_o)[:2] == (True, False),
      "occupied with no deadline is occupied and not reclaimable")

_o = json.loads(json.dumps(_occupied))
_o["deadline"], _o["deadline_kind"] = "2020-01-01T00:00:00Z", "funding_deadline"
check(_claim_status(_o)[:2] == (True, False),
      "a PAST funding_deadline leaves an occupied row occupied")

_o = json.loads(json.dumps(_occupied))
_o["deadline"], _o["deadline_kind"] = "2020-01-01T00:00:00Z", "verification_expires_at"
check(_claim_status(_o)[:2] == (True, False),
      "a PAST verification_expires_at is not a lapsed CLAIM either")

_o = json.loads(json.dumps(_occupied))
_o["deadline"], _o["deadline_kind"] = "2020-01-01T00:00:00Z", "claim_expires_at"
check(_claim_status(_o)[:2] == (False, True),
      "a PAST claim_expires_at makes the row reclaimable")

_o = json.loads(json.dumps(_occupied))
_o["deadline"], _o["deadline_kind"] = "2099-01-01T00:00:00Z", "claim_expires_at"
check(_claim_status(_o)[:2] == (True, False),
      "a FUTURE claim_expires_at means the claim is still live")

# The fixtures pin 2099 as "future". Assert that is still true rather than
# letting these cases silently invert on some far-future run.
check(selector_mod._parse_rfc3339("2099-01-01T00:00:00Z").timestamp() > time.time(),
      "the fixture FUTURE_DEADLINE is genuinely still in the future")

# An unparseable claim deadline must fail closed, not fall through to
# work_state with expiry unknown.
_o = json.loads(json.dumps(_occupied))
_o["deadline"], _o["deadline_kind"] = "not-a-date", "claim_expires_at"
try:
    _claim_status(_o)
    check(False, "an unparseable claim deadline raises rather than guessing")
except selector_mod.Malformed:
    check(True, "an unparseable claim deadline raises rather than guessing")

print()
print("=== a malformed record poisons the WHOLE response, not just itself ===")
# One item in a genuine response is built by the same constructor as every
# other, so a single contract breach means the payload is not what it claims to
# be. Ranking the plausible-looking remainder would be trusting an attacker to
# have corrupted only the unimportant parts.
poisoned = json.loads(json.dumps(GOOD))
poisoned["items"][0].pop("bond", None)      # break the LOW-margin record only
_poison_path = PROBE_DIR / "poisoned.json"
_poison_path.write_text(json.dumps(poisoned), encoding="utf-8")
_poisoned = run(_poison_path)
check(_poisoned.get("action") == "refresh",
      f"one malformed item makes the whole response refresh (got {_poisoned.get('action')!r})")
check(_poisoned.get("selected") is None,
      "and no 'still fine' sibling record is selected from a poisoned payload")

print()
print("=== the shipped fixtures really are contract-complete ===")
# The previous fixtures had drifted: several omitted verification_ready,
# payment_state, payment_committed, funded_amount, funding_target or bond
# entirely, so they could not have exercised the item contract at all. Every
# fixture is now generated from one canonical shape by fixtures/_build.py, and
# only the deliberately-malformed cases may breach the contract.
selector_required = getattr(selector_mod, "REQUIRED_ITEM_FIELDS", None)
check(isinstance(selector_required, dict) and len(selector_required) >= 16,
      "selector exports the REQUIRED_ITEM_FIELDS contract")
INTENTIONALLY_MALFORMED = {
    name for name, (action, _) in EXPECTATIONS.items()
    if action == "refresh" and name.startswith("adversarial-item-")
} | {"adversarial-malformed-money.json", "adversarial-unit-mismatch.json"}

for name, (expected, _why) in sorted(EXPECTATIONS.items()):
    if name in INTENTIONALLY_MALFORMED:
        continue
    path = FIXTURES / name
    if not path.is_file():
        continue
    for record in json.loads(path.read_text()).get("items") or []:
        absent = [f for f in (selector_required or {}) if f not in record]
        check(not absent,
              f"{name} item {record.get('opportunity_id', '?')} is contract-complete "
              f"(missing={absent})")

builder = FIXTURES / "_build.py"
check(builder.is_file(), "fixtures/_build.py regenerates every fixture deterministically")

# Every shipped fixture must be claimed by an expectation. Without this, adding
# a fixture and forgetting to assert on it leaves a file that looks like
# coverage but tests nothing -- the same "green suite proving nothing" failure
# mode that let the fail-open defects through.
_shipped = {p.name for p in FIXTURES.glob("*.json")}
_orphans = sorted(_shipped - set(EXPECTATIONS) - {"view-filtered-subset.json"})
check(not _orphans, f"every fixture is asserted by EXPECTATIONS (orphans={_orphans})")
_missing = sorted(set(EXPECTATIONS) - _shipped)
check(not _missing, f"every expectation has a fixture on disk (missing={_missing})")

print()
print("=== canonical ready-to-earn feed uses only real query parameters ===")

check(
    "ready_to_earn=true" not in config_src,
    "config does not use the invented ?ready_to_earn=true parameter",
)

# Assert on the actual runtime constant rather than scanning source text: the
# constant is built by string concatenation, so a naive text scan cannot see it.
feed = getattr(selector_mod, "READY_TO_EARN_FEED", None)
check(bool(isinstance(feed, str) and feed), "selector exports a READY_TO_EARN_FEED constant")


def validate_feed_url(url: str, label: str) -> None:
    params = parse_qs(urlparse(url).query)
    unknown = set(params) - SUPPORTED_QUERY_KEYS
    check(not unknown, f"{label} uses only server-supported params (unknown={sorted(unknown)})")
    for key, value in REQUIRED_FILTER.items():
        check(params.get(key) == [value], f"{label} pins {key}={value}")


if isinstance(feed, str) and feed:
    validate_feed_url(feed, "READY_TO_EARN_FEED")

# Every literal opportunities URL in the YAML config must carry the full filter.
for url in sorted(set(re.findall(
    r"https://api\.agentbounties\.app/v1/opportunities[^\s\"',)]*", config_src
))):
    validate_feed_url(url, f"config {url}")

# And the URLs the selector actually emits to the operator must be filtered too.
for name in ("empty.json", "adversarial-missing-freshness.json"):
    path = FIXTURES / name
    if not path.is_file():
        continue
    emitted = str(run(path).get("next_action", ""))
    for url in re.findall(
        r"https://api\.agentbounties\.app/v1/opportunities[^\s\"',)]*", emitted
    ):
        validate_feed_url(url, f"{name} next_action")

print()
if failures:
    print(f"{len(failures)} selector test(s) FAILED:")
    for item in failures:
        print(f"  - {item}")
    raise SystemExit(1)

print(f"all {passes} selector assertions passed")
