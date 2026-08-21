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

import json
import subprocess
import sys
from pathlib import Path
from urllib.parse import parse_qs, urlparse

HERE = Path(__file__).resolve().parent
SELECTOR = HERE / "select_bounty.py"
FIXTURES = HERE / "fixtures"
CONFIG = HERE / "config.yaml"

# fixture -> (expected action, why refusing matters)
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
        "expiry is evaluated before occupancy, so this is reclaimable",
    ),
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
    "adversarial-envelope-truncated-coverage.json": (
        "refresh", "item_count above delivered items means the page was truncated"),
    "adversarial-envelope-no-source-statuses.json": (
        "refresh", "without source_statuses, coverage is entirely unknown"),
    "adversarial-item-spoofed-canonicity.json": (
        # skip, not refresh: the ENVELOPE here is internally consistent (it
        # declares 0 canonical items and delivers 0), so this is an item-level
        # rejection, not a coverage fault. The refusal must still be absolute.
        "skip", "network + 0x address is not canonicity; source_type is authoritative"),
    # the headline regression: an empty list must NOT be reported as "no work"
    # when the response itself is untrustworthy.
    "adversarial-empty-with-broken-source.json": (
        "refresh", "an empty list from a broken source is a fault, not 'no work'"),
    "adversarial-empty-degraded.json": (
        "refresh", "an empty list from a degraded projection is a fault, not 'no work'"),
    "adversarial-empty-stale.json": (
        "refresh", "an empty list from a stale snapshot is a fault, not 'no work'"),
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
print("=== an untrustworthy response is never reported as 'no work' ===")
# This is the regression the review caught: `wait` means "the canonical feed is
# healthy and there is genuinely nothing funded". It must be unreachable when
# the projection is degraded, stale, truncated, or backed by a broken source.
for name in (
    "adversarial-empty-with-broken-source.json",
    "adversarial-empty-degraded.json",
    "adversarial-empty-stale.json",
    "adversarial-envelope-truncated-coverage.json",
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
print("=== canonical ready-to-earn feed uses only real query parameters ===")
config_src = CONFIG.read_text(encoding="utf-8")

check(
    "ready_to_earn=true" not in config_src,
    "config does not use the invented ?ready_to_earn=true parameter",
)

# Assert on the actual runtime constant rather than scanning source text: the
# constant is built by string concatenation, so a naive text scan cannot see it.
import importlib.util  # noqa: E402  (local to this focused assertion block)

spec = importlib.util.spec_from_file_location("select_bounty", SELECTOR)
if spec is None or spec.loader is None:
    raise SystemExit(f"cannot import selector module from {SELECTOR}")
selector_mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(selector_mod)
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
import re  # noqa: E402

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
