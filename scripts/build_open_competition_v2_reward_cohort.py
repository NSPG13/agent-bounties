#!/usr/bin/env python3
"""Build the reviewed matched-window 6-USDC GMV reward cohort."""

from __future__ import annotations

import argparse
from copy import deepcopy
from datetime import datetime, timezone
import json
from pathlib import Path
import re
from typing import Any

from eth_utils import keccak

from forward_canonical_gmv import verification_policy_hash


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "agent-bounties/open-competition-v2-forward-gmv-reward-cohort-v1"
BASELINE_SCHEMA = (
    "agent-bounties/open-competition-v2-forward-gmv-meta-candidate-specs-v2"
)
PROFILE_ID = "forward-canonical-gmv-attribution-metric-v2"
SOLVER_REWARD = 6_000_000
KEEPER_REWARD = 40_000
PER_COMPETITION = SOLVER_REWARD + KEEPER_REWARD
BASELINE_SOLVER_REWARD = 3_000_000
MATCHED_CONTROLS = (
    "external-gmv-forward-daily-20260825-v2",
    "external-gmv-forward-daily-20260826-v2",
    "external-gmv-forward-daily-20260827-v2",
    "external-gmv-forward-week-20260831-v2",
    "external-gmv-forward-fortnight-20260907-v2",
)
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")


class CohortError(ValueError):
    pass


def parse_utc_seconds(value: str, field: str) -> datetime:
    if not value.endswith("Z"):
        raise CohortError(f"{field} must end in Z")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise CohortError(f"{field} must be ISO-8601") from error
    if parsed.tzinfo != timezone.utc or parsed.microsecond:
        raise CohortError(f"{field} must use whole UTC seconds")
    return parsed


def timestamp(value: str) -> int:
    return int(parse_utc_seconds(value, "campaign timestamp").timestamp())


def normalized_contracts(values: list[str]) -> list[str]:
    normalized = [str(value).lower() for value in values]
    if len(normalized) != 10 or len(set(normalized)) != 10:
        raise CohortError("exactly ten unique active reward contracts are required")
    if not all(ADDRESS.fullmatch(value) for value in normalized):
        raise CohortError("active reward contracts must be exact EVM addresses")
    return sorted(normalized)


def build_cohort(
    baseline: dict[str, Any],
    active_reward_contracts: list[str],
    approved_at: str,
) -> dict[str, Any]:
    if (
        baseline.get("schema_version") != BASELINE_SCHEMA
        or baseline.get("network") != "base-mainnet"
        or baseline.get("profile_release", {}).get("profile_id") != PROFILE_ID
        or baseline.get("profile_release", {}).get("status") != "reviewed"
    ):
        raise CohortError(
            "baseline pool is not the reviewed Base mainnet forward-GMV pool"
        )
    economics = baseline.get("economics", {})
    if (
        int(economics.get("solver_reward_base_units", 0)) != BASELINE_SOLVER_REWARD
        or int(economics.get("keeper_reward_base_units", 0)) != KEEPER_REWARD
    ):
        raise CohortError("baseline pool economics are not the exact 3-USDC control")
    approved = parse_utc_seconds(approved_at, "approved_at")
    controls = {
        str(candidate.get("candidate_id")): candidate
        for candidate in baseline.get("candidates", [])
        if isinstance(candidate, dict)
    }
    if any(candidate_id not in controls for candidate_id in MATCHED_CONTROLS):
        raise CohortError("a matched baseline control is missing")
    first_start = min(
        parse_utc_seconds(
            str(controls[candidate_id]["epoch"]["starts_at"]), "control starts_at"
        )
        for candidate_id in MATCHED_CONTROLS
    )
    if approved >= first_start:
        raise CohortError("approved_at must precede every treatment scoring window")

    active_contracts = normalized_contracts(active_reward_contracts)
    base_excluded = [
        str(value).lower()
        for value in baseline.get("eligibility_policy", {}).get(
            "excluded_bounty_contracts", []
        )
    ]
    if not all(ADDRESS.fullmatch(value) for value in base_excluded):
        raise CohortError("baseline excluded contracts are malformed")
    excluded_contracts = sorted(set([*base_excluded, *active_contracts]))
    excluded_wallets = [
        str(value).lower()
        for value in baseline.get("eligibility_policy", {}).get("excluded_wallets", [])
    ]
    attesters = [
        str(value).lower()
        for value in baseline.get("attestation_policy", {}).get("attesters", [])
    ]
    threshold = int(baseline.get("attestation_policy", {}).get("threshold", 0))
    if (
        excluded_wallets != sorted(excluded_wallets)
        or threshold != 2
        or len(attesters) != 2
    ):
        raise CohortError("baseline eligibility or attestation policy is not exact")

    candidates: list[dict[str, Any]] = []
    for control_id in MATCHED_CONTROLS:
        control = controls[control_id]
        suffix = control_id.removeprefix("external-gmv-forward-").removesuffix("-v2")
        candidate_id = f"external-gmv-reward-6usdc-{suffix}-v1"
        epoch = deepcopy(control["epoch"])
        epoch["epoch_id"] = (
            "0x"
            + keccak(
                text=f"agent-bounties/canonical-gmv-epoch-v1\0{candidate_id}"
            ).hex()
        )
        campaign = {
            "epoch_id": epoch["epoch_id"],
            "starts_at": timestamp(epoch["starts_at"]),
            "ends_at": timestamp(epoch["ends_at"]),
            "minimum_score_base_units": int(epoch["minimum_score_base_units"]),
            "excluded_wallets": excluded_wallets,
            "excluded_bounty_contracts": excluded_contracts,
            "snapshot_attesters": attesters,
            "snapshot_attestation_threshold": threshold,
        }
        verification_hash = "0x" + verification_policy_hash(campaign).hex()
        candidates.append(
            {
                "candidate_id": candidate_id,
                "title": f"6 USDC prize — {control['title']}",
                "summary": control["summary"],
                "gmv_lane": "external_supply",
                "epoch": epoch,
                "snapshot": {
                    "status": "scheduled",
                    "verification_policy_hash": verification_hash,
                    "snapshot_attesters": attesters,
                    "snapshot_attestation_threshold": threshold,
                    "canonical_snapshot_due_after": epoch["ends_at"],
                },
                "matched_control": {
                    "candidate_id": control_id,
                    "solver_reward_base_units": BASELINE_SOLVER_REWARD,
                    "starts_at": epoch["starts_at"],
                    "ends_at": epoch["ends_at"],
                },
                "analysis_sources": [
                    {
                        "kind": "canonical_platform_metric",
                        "url": "https://api.agentbounties.app/v1/metrics/platform?period=28d",
                    },
                    {
                        "kind": "privacy_safe_funnel_metric",
                        "url": "https://api.agentbounties.app/v1/analytics/site?window_hours=720",
                    },
                ],
                "feedback_sources": [
                    {
                        "kind": "observable_public_participation_behavior",
                        "url": "https://api.agentbounties.app/v1/opportunities?network=base-mainnet&view=ready_to_earn&source_type=canonical_base&limit=300",
                        "observation": "The reviewed 3-USDC controls had zero accepted entries at cohort review time.",
                    }
                ],
            }
        )

    return {
        "schema_version": SCHEMA,
        "protocol_version": baseline["protocol_version"],
        "network": "base-mainnet",
        "factory_contract": str(baseline["factory_contract"]).lower(),
        "release_hash": str(baseline["release_hash"]).lower(),
        "reserve_wallet": str(baseline["reserve_wallet"]).lower(),
        "profile_release": deepcopy(baseline["profile_release"]),
        "approved_at": approved_at,
        "expires_at": baseline["expires_at"],
        "economics": {
            "solver_reward_base_units": SOLVER_REWARD,
            "keeper_reward_base_units": KEEPER_REWARD,
            "total_per_competition_base_units": PER_COMPETITION,
        },
        "experiment": {
            "hypothesis": "A prize that leaves a positive cash result after a 3-USDC child settlement will increase qualified starts and canonical entries.",
            "treatment": "6-USDC solver prize with the same scoring profile, instructions, child template, and matched UTC window.",
            "control": "The already active 3-USDC solver-prize competition with the matched UTC window.",
            "primary_metric": "competition_entry_confirmed per qualified competition_view session",
            "leading_metrics": [
                "funded_bounty_click",
                "competition_instructions_copied",
                "competition_child_post_started",
                "competition_feedback_submitted",
            ],
            "outcomes": ["externally funded canonical GMV", "CompetitionSettledV2"],
            "minimum_qualified_starts": 10,
            "decision_rule": "Do not promote from traffic alone; require at least ten qualified starts and no reduction in canonical GMV or payment integrity.",
        },
        "eligibility_policy": {
            **deepcopy(baseline["eligibility_policy"]),
            "excluded_bounty_contracts": excluded_contracts,
            "contract_boundary": "All reward contracts active when this cohort was reviewed are excluded, together with the baseline exclusions.",
        },
        "attestation_policy": deepcopy(baseline["attestation_policy"]),
        "scoring_policy": deepcopy(baseline["scoring_policy"]),
        "candidates": candidates,
        "evidence_boundary": "This reviewed cohort specification is not a policy change, activation, GMV event, entry, payout, or settlement.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--baseline-pool",
        type=Path,
        default=ROOT / "ops" / "open-competition-v2-forward-gmv-candidate-pool-v2.json",
    )
    parser.add_argument("--active-reward-contract", action="append", required=True)
    parser.add_argument("--approved-at", required=True)
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "ops" / "open-competition-v2-forward-gmv-reward-cohort-v1.json",
    )
    args = parser.parse_args()
    try:
        baseline = json.loads(args.baseline_pool.read_text(encoding="utf-8-sig"))
        value = build_cohort(baseline, args.active_reward_contract, args.approved_at)
    except (
        OSError,
        json.JSONDecodeError,
        KeyError,
        TypeError,
        ValueError,
        CohortError,
    ) as error:
        print(f"reward cohort build blocked: {error}")
        return 2
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
