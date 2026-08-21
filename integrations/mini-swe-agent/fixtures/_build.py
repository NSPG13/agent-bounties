#!/usr/bin/env python3
"""Regenerate the selector fixtures from one canonical, contract-complete shape.

WHY THIS EXISTS. Every fixture used to be hand-written, and they had drifted
away from the real `OpportunityItem` contract: several omitted
`verification_ready`, `payment_state`, `payment_committed`, `funded_amount`,
`funding_target` or `bond` entirely. Fixtures that are not shaped like real
responses cannot falsify anything, and they are how a fail-open selector passes
its own tests -- exactly the defect maintainer review caught.

So the base item here mirrors crates/api/src/opportunities.rs::OpportunityItem
field for field, and every adversarial fixture is derived from it by changing
ONE thing. That keeps each case isolated: if it refuses, the reason is the one
mutation, not an accidental second defect.

Run:  python3 -B integrations/mini-swe-agent/fixtures/_build.py
"""

from __future__ import annotations

import copy
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent

SCHEMA = "agent-bounties/opportunity-projection-v1"
FEED = ("https://api.agentbounties.app/v1/opportunities"
        "?network=base-mainnet&view=ready_to_earn&source_type=canonical_base")
ENVELOPE_BOUNDARY = (
    "Projection only. Listing is not funding, claim, verification, or settlement; "
    "only a confirmed canonical BountySettled event proves payment."
)
ITEM_BOUNDARY = (
    "Canonical lifecycle and payment language require confirmed factory/bounty "
    "events. Payment is `paid` only after confirmed BountySettled; a plan, "
    "signature, transaction hash, hosted row, or AI analysis is not payment evidence."
)


def usdc(amount: str) -> dict:
    """An upstream OpportunityAmount. All four fields are non-optional."""
    return {"amount": amount, "currency": "USDC", "unit": "base_units", "decimals": 6}


def item(
    *,
    oid: str,
    contract: str,
    title: str,
    goal: str,
    categories: list,
    reward: str,
    bond: str,
    spend: str = "0",
    funded: str | None = None,
    work_state: str = "claimable",
) -> dict:
    """A complete canonical OpportunityItem, ready to earn."""
    target = funded if funded is not None else str(int(reward) + int(bond))
    margin = str(int(reward) - int(spend))
    return {
        "opportunity_id": oid,
        "source_type": "canonical_base",
        "source_id": contract,
        "source_status": "claimable",
        "title": title,
        "goal": goal,
        "categories": categories,
        "skills": ["python", "rust"],
        "public_url": f"https://agentbounties.app/bounties/{oid}",
        "network": "base-mainnet",
        "work_state": work_state,
        "payment_state": "escrowed",
        "payment_committed": True,
        "competition_mode": "exclusive_claim",
        "standing_meta_bounty": False,
        "decision_authority": (
            f"The immutable canonical verification mode/module configured on {contract} "
            "decides the submission result."
        ),
        "payment_authority": (
            f"The exact canonical bounty contract {contract} controls escrow; only its "
            "confirmed BountySettled event proves payment."
        ),
        "reward": usdc(reward),
        "completion_bonus": usdc("0"),
        "funded_amount": usdc(target),
        "funding_target": usdc(target),
        "bond": usdc(bond),
        "verification_method": "signed_quorum",
        "verification_ready": True,
        "evidence_requirements": {"required": ["repository", "commit", "test_command"]},
        "terms_hash": "0x" + "ab" * 32,
        "proof_urls": [],
        "cash_economics": {
            "solver_reward": usdc(reward),
            "refundable_claim_bond": usdc(bond),
            "required_external_spend": usdc(spend),
            "gross_cash_margin": usdc(margin),
            "gross_cash_margin_positive": int(margin) > 0,
            "scope_disclaimer": "Gross cash margin excludes gas, taxes and failure risk.",
        },
        "discovery_factors": [
            "source_type=canonical_base",
            "work_state=claimable",
            "payment_state=escrowed",
            "view:ready_to_earn;factors=claimable+escrowed+verification_ready"
            "+positive_gross_cash_margin",
        ],
        "created_at": "2026-08-20T00:00:00+00:00",
        "updated_at": "2026-08-21T05:55:00+00:00",
        "evidence_boundary": ITEM_BOUNDARY,
    }


CHECKER = item(
    oid="canonical:base-mainnet:0xa1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
    contract="0xa1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
    title="Add a deterministic MCP discovery checker",
    goal="Implement a deterministic checker for MCP discovery.",
    categories=["coding"],
    reward="990000", bond="10000",
)
FAILOVER = item(
    oid="canonical:base-mainnet:0xa2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2",
    contract="0xa2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2",
    title="Add retry-safe Base RPC failover for CLI tooling",
    goal="Implement deterministic RPC failover for the CLI.",
    categories=["coding"],
    reward="2000000", bond="10000",
)
# A complete, valid canonical record that simply is not coding work. This is a
# legitimate `skip`, not a data fault, so it must be contract-complete too.
MARKETING = item(
    oid="canonical:base-mainnet:0xa3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3",
    contract="0xa3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3",
    title="Write launch marketing copy",
    goal="Draft promotional copy for the launch announcement.",
    categories=["marketing"],
    reward="5000000", bond="10000",
)
MARKETING["skills"] = ["copywriting"]


def envelope(items, *, declared=None, age=60, **overrides) -> dict:
    body = {
        "schema_version": SCHEMA,
        "generated_at": "2026-08-21T06:00:00Z",
        "network": "base-mainnet",
        "applied_view": "ready_to_earn",
        "degraded": False,
        "source_statuses": [{
            "source_type": "canonical_base",
            "available": True,
            "authoritative_urls": [FEED],
            "item_count": len(items) if declared is None else declared,
            "error": None,
        }],
        "evidence_boundary": ENVELOPE_BOUNDARY,
        "age_seconds": age,
        "staleness_seconds": 900,
        "items": items,
    }
    body.update(overrides)
    return body


def mutate(base: dict, **changes) -> dict:
    """Copy an item and apply one targeted change. `None` deletes the key."""
    out = copy.deepcopy(base)
    for key, value in changes.items():
        if value is None:
            out.pop(key, None)
        else:
            out[key] = value
    return out


def solo(base: dict, **changes) -> dict:
    """A one-item ready-to-earn envelope around a mutated CHECKER-like record."""
    return envelope([mutate(base, **changes)])


THREE = [CHECKER, FAILOVER, MARKETING]

FIXTURES: dict[str, dict] = {}

# ---------------------------------------------------------------- happy paths
FIXTURES["multiple.json"] = envelope(THREE, declared=3)
FIXTURES["empty.json"] = envelope([], declared=0)
# A view filter can only REMOVE items, so a healthy response normally delivers
# fewer than the pre-filter item_count. Declared 9 > delivered 3.
FIXTURES["view-filtered-subset.json"] = envelope(THREE, declared=9)

# ---------------------------------------------------------------- envelope
FIXTURES["stale.json"] = envelope([CHECKER], age=5400)
FIXTURES["adversarial-future-timestamp.json"] = envelope(
    [CHECKER], age=-3600, generated_at="2099-01-01T00:00:00Z")
_missing_fresh = envelope([CHECKER])
del _missing_fresh["age_seconds"]
# `generated_at` is non-optional upstream, so the envelope check requires a
# non-empty string. The freshness fault this exercises is therefore an
# UNPARSEABLE stamp: the field is there, but its age cannot be computed, and an
# age that cannot be computed must never be treated as fresh.
_missing_fresh["generated_at"] = "recently"
FIXTURES["adversarial-missing-freshness.json"] = _missing_fresh
FIXTURES["adversarial-envelope-bad-schema.json"] = envelope(
    THREE, declared=3, schema_version="agent-bounties/opportunity-projection-v99")
FIXTURES["adversarial-envelope-wrong-network.json"] = envelope(
    THREE, declared=3, network="ethereum-mainnet")
FIXTURES["adversarial-envelope-wrong-view.json"] = envelope(
    THREE, declared=3, applied_view=None)
FIXTURES["adversarial-envelope-degraded.json"] = envelope(THREE, declared=3, degraded=True)
FIXTURES["adversarial-envelope-source-error.json"] = envelope(
    THREE, declared=3, source_statuses=[{
        "source_type": "canonical_base", "available": True,
        "authoritative_urls": [FEED], "item_count": 3,
        "error": "base rpc timeout after 3 attempts"}])
FIXTURES["adversarial-envelope-source-unavailable.json"] = envelope(
    THREE, declared=3, source_statuses=[{
        "source_type": "canonical_base", "available": False,
        "authoritative_urls": [FEED], "item_count": 0, "error": None}])
# Delivering MORE canonical items than were read is incoherent: a filter removes.
FIXTURES["adversarial-envelope-incoherent-coverage.json"] = envelope(THREE, declared=1)
_no_statuses = envelope(THREE, declared=3)
_no_statuses["source_statuses"] = []
FIXTURES["adversarial-envelope-no-source-statuses.json"] = _no_statuses

# An empty list is only "no work" when the response itself is trustworthy.
FIXTURES["adversarial-empty-with-broken-source.json"] = envelope(
    [], source_statuses=[{
        "source_type": "canonical_base", "available": False,
        "authoritative_urls": [FEED], "item_count": 0,
        "error": "indexer unreachable"}])
FIXTURES["adversarial-empty-degraded.json"] = envelope([], degraded=True)
FIXTURES["adversarial-empty-stale.json"] = envelope([], age=7200)

# ------------------------------------------------- item: well-formed refusals
# These records satisfy the OpportunityItem contract completely. They are
# refused on their MERITS, so the correct action is `skip`, not `refresh`.
FIXTURES["adversarial-non-canonical-source.json"] = solo(
    CHECKER, source_type="third_party_registry", network="ethereum-mainnet",
    discovery_factors=["source_type=third_party_registry"])
# The spoof: everything screams canonical -- Base network, a real 20-byte
# address, discovery_factors ASSERTING canonical_base -- but source_type, the
# one authoritative marker, says otherwise. source_type wins.
FIXTURES["adversarial-item-spoofed-canonicity.json"] = envelope(
    [mutate(CHECKER, source_type="github_discovery")], declared=0)
FIXTURES["adversarial-verifier-unready.json"] = solo(CHECKER, verification_ready=False)
FIXTURES["adversarial-underfunded.json"] = solo(
    CHECKER, funded_amount=usdc("500000"), funding_target=usdc("2010000"))
FIXTURES["adversarial-item-not-escrowed.json"] = solo(
    CHECKER, payment_state="seeking_funding")
FIXTURES["adversarial-item-payment-uncommitted.json"] = solo(
    CHECKER, payment_committed=False)
FIXTURES["adversarial-item-zero-bond.json"] = solo(CHECKER, bond=usdc("0"))
FIXTURES["adversarial-item-work-state-open.json"] = solo(CHECKER, work_state="open")
FIXTURES["no-margin.json"] = envelope([item(
    oid="canonical:base-mainnet:0xa1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
    contract="0xa1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
    title="Seed a paid API child bounty",
    goal="Create and fully fund a coding bounty another participant completes.",
    categories=["coding"], reward="900000", bond="100000", spend="900000")])

# ---------------------------------------------------------- item: data faults
# These BREACH the contract, so nothing can be concluded and the action is
# `refresh`. Each deletes or corrupts exactly one required thing.
FIXTURES["adversarial-item-no-verification-ready.json"] = solo(
    CHECKER, verification_ready=None)
FIXTURES["adversarial-item-verification-ready-not-boolean.json"] = solo(
    CHECKER, verification_ready="true")
FIXTURES["adversarial-item-no-escrow-fields.json"] = solo(
    CHECKER, payment_state=None, payment_committed=None,
    funded_amount=None, funding_target=None)
FIXTURES["adversarial-item-no-bond.json"] = solo(CHECKER, bond=None)
FIXTURES["adversarial-item-bad-contract-address.json"] = solo(
    CHECKER, source_id="0x" + "z" * 40)
FIXTURES["adversarial-item-short-contract-address.json"] = solo(
    CHECKER, source_id="0xa1a1a1")
FIXTURES["adversarial-malformed-money.json"] = solo(
    CHECKER, reward={"amount": "lots"})
FIXTURES["adversarial-item-amount-no-unit.json"] = solo(
    CHECKER, reward={"amount": "990000", "currency": "USDC", "decimals": 6})
FIXTURES["adversarial-unit-mismatch.json"] = solo(CHECKER, cash_economics={
    "solver_reward": usdc("990000"),
    "refundable_claim_bond": usdc("10000"),
    "required_external_spend": {"amount": "1000000", "currency": "DAI",
                                "unit": "base_units", "decimals": 18},
    "gross_cash_margin_positive": True,
    "scope_disclaimer": "x",
})
# The server's precomputed margin contradicts its own components.
FIXTURES["adversarial-item-contradictory-economics.json"] = solo(
    CHECKER, cash_economics={
        "solver_reward": usdc("990000"),
        "refundable_claim_bond": usdc("10000"),
        "required_external_spend": usdc("980000"),
        "gross_cash_margin": usdc("990000"),
        "gross_cash_margin_positive": True,
        "scope_disclaimer": "x",
    })
FIXTURES["adversarial-item-funding-decimal-mismatch.json"] = solo(
    CHECKER, funding_target={"amount": "1000000", "currency": "USDC",
                             "unit": "base_units", "decimals": 18})

# --------------------------------------------------------------- claim status
_live = mutate(CHECKER, work_state="in_progress",
               exclusive_claimant="0x8cfb0c37af0c40f96c44fd45fdec30b430bc6a6e")
FIXTURES["exclusive-claimant.json"] = envelope([_live])
# Expiry is evaluated BEFORE occupancy, so a lapsed claim is reclaimable even
# though a claimant address is still recorded on the row.
_expired = mutate(CHECKER, work_state="in_progress",
                  active_claimant="0x8cfb0c37af0c40f96c44fd45fdec30b430bc6a6e",
                  claim_expires_at=1700000000)
FIXTURES["adversarial-expired-claim.json"] = envelope([_expired])


def main() -> int:
    for name, body in sorted(FIXTURES.items()):
        (HERE / name).write_text(json.dumps(body, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {len(FIXTURES)} fixtures to {HERE}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
