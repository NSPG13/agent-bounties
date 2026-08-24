#!/usr/bin/env python3
"""Build one exact owner policy-rotation transaction for the 6-USDC cohort."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
from typing import Any

from eth_abi import encode
from eth_utils import keccak, to_checksum_address

from build_open_competition_v2_gmv_activation import (
    PARAM_TYPE,
    POLICY_TYPE,
    PROOF_WINDOW_SECONDS,
    address,
    calldata,
    create2,
    hash32,
    hex_hash,
    parse_time,
    raw_hash,
)
from build_open_competition_v2_reward_cohort import (
    KEEPER_REWARD,
    PER_COMPETITION,
    SCHEMA as COHORT_SCHEMA,
    SOLVER_REWARD,
)


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "agent-bounties/open-competition-v2-reward-policy-rotation-v1"
STATE_SCHEMA = "agent-bounties/open-competition-v2-reserve-safe-state-v1"
CHAIN_ID = 8453
OWNER = "0x884834e884d6e93462655a2820140ad03e6747bc"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
BASELINE_SOLVER_REWARD = 3_000_000
BASELINE_KEEPER_REWARD = 40_000
BASELINE_PER_COMPETITION = 3_040_000
MAX_PER_PERIOD = 30_400_000
MAX_LIFETIME = 77_668_098
EXPECTED_LIFETIME_SPENT = 30_400_000
EXPECTED_RESERVE_BALANCE = 47_268_098
PERIOD_SECONDS = 86_400
FUTURE_FLOOR_RESERVE = 5 * BASELINE_PER_COMPETITION


class RotationError(ValueError):
    pass


def require_int(value: object, field: str) -> int:
    try:
        parsed = int(value)
    except (TypeError, ValueError) as error:
        raise RotationError(f"{field} must be an integer") from error
    if parsed < 0:
        raise RotationError(f"{field} must be nonnegative")
    return parsed


def build_rotation(
    cohort: dict[str, Any],
    state: dict[str, Any],
    activation_time: datetime,
) -> dict[str, Any]:
    activation_time = activation_time.astimezone(timezone.utc)
    if (
        cohort.get("schema_version") != COHORT_SCHEMA
        or cohort.get("network") != "base-mainnet"
    ):
        raise RotationError("reward cohort is not the reviewed Base mainnet schema")
    if (
        state.get("schema_version") != STATE_SCHEMA
        or int(state.get("chain_id", 0)) != CHAIN_ID
    ):
        raise RotationError(
            "reserve evidence is not the Base mainnet safe-state schema"
        )
    if (
        state.get("block_tag") != "safe"
        or require_int(state.get("safe_block"), "safe block") <= 0
    ):
        raise RotationError("reserve evidence is not pinned to a safe block")

    reserve = address(cohort.get("reserve_wallet"), "cohort reserve")
    if address(state.get("reserve_wallet"), "state reserve") != reserve:
        raise RotationError("safe state is for a different reserve")
    if address(state.get("owner"), "reserve owner") != OWNER:
        raise RotationError("reserve owner differs from the exact funding owner")
    if bool(state.get("revoked")):
        raise RotationError("reserve policy is revoked")
    current_version = require_int(state.get("policy_version"), "policy version")
    if current_version != 1:
        raise RotationError("the reviewed rotation requires exact policy version 1")
    current_policy_hash = hash32(state.get("active_policy_hash"), "active policy hash")
    factory = address(state.get("competition_factory"), "competition factory")
    if factory != address(cohort.get("factory_contract"), "cohort factory"):
        raise RotationError("safe-state factory differs from the cohort factory")
    implementation = address(
        state.get("competition_implementation"), "competition implementation"
    )
    settlement_token = address(state.get("settlement_token"), "settlement token")
    if settlement_token != USDC:
        raise RotationError("reserve does not settle in native Base USDC")

    policy = state.get("policy")
    profile = cohort.get("profile_release")
    if not isinstance(policy, dict) or not isinstance(profile, dict):
        raise RotationError("safe policy or reviewed profile is missing")
    delegate = address(policy.get("delegate"), "delegate")
    exact_current = {
        "period_seconds": PERIOD_SECONDS,
        "solver_reward": BASELINE_SOLVER_REWARD,
        "keeper_reward": BASELINE_KEEPER_REWARD,
        "exact_funding_per_competition": BASELINE_PER_COMPETITION,
        "max_per_period": MAX_PER_PERIOD,
        "max_lifetime_spend": MAX_LIFETIME,
    }
    for field, expected in exact_current.items():
        if require_int(policy.get(field), f"current {field}") != expected:
            raise RotationError(f"current {field} differs from the reviewed baseline")
    beta_risk_hash = hash32(policy.get("beta_risk_hash"), "Beta risk hash")
    metric_program_hash = hash32(
        policy.get("gmv_metric_program_hash"), "GMV metric program"
    )
    journal_schema_hash = hash32(
        policy.get("gmv_journal_schema_hash"), "GMV journal schema"
    )
    if metric_program_hash != hash32(
        profile.get("metric_program_hash"), "cohort metric program"
    ):
        raise RotationError("live metric program differs from the reviewed cohort")
    if journal_schema_hash != hash32(
        profile.get("journal_schema_hash"), "cohort journal schema"
    ):
        raise RotationError("live journal schema differs from the reviewed cohort")
    lifetime_spent = require_int(state.get("lifetime_spent"), "lifetime spent")
    reserve_balance = require_int(state.get("reserve_balance"), "reserve balance")
    if (
        lifetime_spent != EXPECTED_LIFETIME_SPENT
        or reserve_balance != EXPECTED_RESERVE_BALANCE
    ):
        raise RotationError("reserve spend or balance changed after cohort review")
    period_bucket = require_int(state.get("period_bucket"), "period bucket")
    period_spent = require_int(state.get("period_spent"), "period spent")
    if period_spent != MAX_PER_PERIOD:
        raise RotationError("current UTC-period spend changed after cohort review")
    candidates = cohort.get("candidates")
    if not isinstance(candidates, list) or len(candidates) != 5:
        raise RotationError(
            "the reviewed treatment must contain exactly five candidates"
        )
    treatment_total = len(candidates) * PER_COMPETITION
    if reserve_balance - treatment_total < FUTURE_FLOOR_RESERVE:
        raise RotationError(
            "treatment would not preserve five later 3.04-USDC floor replacements"
        )

    approved_at = parse_time(str(cohort.get("approved_at")), "cohort approved_at")
    expires_at = parse_time(str(cohort.get("expires_at")), "cohort expires_at")
    if not approved_at <= activation_time < expires_at:
        raise RotationError("cohort review window is not current")
    first_start = min(
        parse_time(str(item["epoch"]["starts_at"]), "candidate starts_at")
        for item in candidates
    )
    confirmation_deadline = first_start.timestamp() - 600
    if activation_time.timestamp() >= confirmation_deadline:
        raise RotationError(
            "the first matched window is too close; rebuild the cohort without it"
        )

    execution_policy_hash = hash32(
        profile.get("execution_policy_hash"), "execution policy"
    )
    settlement_policy_hash = hash32(
        profile.get("settlement_policy_hash"), "settlement policy"
    )
    proof_system = hex_hash(keccak(text="sp1-plonk"))
    funding_deadline = int(expires_at.timestamp())
    commitments: list[bytes] = []
    creations: list[dict[str, Any]] = []
    candidate_ids: set[str] = set()
    competition_init = (
        bytes.fromhex("3d602d80600a3d3981f3")
        + bytes.fromhex("363d3d373d3d3d363d73")
        + bytes.fromhex(implementation[2:])
        + bytes.fromhex("5af43d82803e903d91602b57fd5bf3")
    )
    release_hash = hash32(cohort.get("release_hash"), "release hash")
    for candidate in candidates:
        if (
            not isinstance(candidate, dict)
            or candidate.get("gmv_lane") != "external_supply"
        ):
            raise RotationError("candidate does not target external marketplace demand")
        candidate_id = str(candidate.get("candidate_id") or "")
        if not candidate_id or candidate_id in candidate_ids:
            raise RotationError("candidate IDs must be unique")
        candidate_ids.add(candidate_id)
        epoch = candidate.get("epoch", {})
        snapshot = candidate.get("snapshot", {})
        starts_at = parse_time(str(epoch.get("starts_at")), f"{candidate_id} starts_at")
        ends_at = parse_time(str(epoch.get("ends_at")), f"{candidate_id} ends_at")
        if starts_at >= ends_at or ends_at >= expires_at:
            raise RotationError(f"{candidate_id} scoring window is invalid")
        score_threshold = require_int(
            epoch.get("minimum_score_base_units"), "score threshold"
        )
        if score_threshold <= 0 or snapshot.get("status") != "scheduled":
            raise RotationError(f"{candidate_id} snapshot is not scheduled")
        verification_policy_hash = hash32(
            snapshot.get("verification_policy_hash"),
            f"{candidate_id} verification policy",
        )
        params = (
            SOLVER_REWARD,
            KEEPER_REWARD,
            funding_deadline,
            PROOF_WINDOW_SECONDS,
            1,
            0,
            score_threshold,
            raw_hash(proof_system),
            raw_hash(hash32(profile.get("program_vkey"), "program vkey")),
            raw_hash(hash32(profile.get("source_hash"), "source hash")),
            raw_hash(hash32(profile.get("elf_hash"), "ELF hash")),
            raw_hash(journal_schema_hash),
            raw_hash(metric_program_hash),
            raw_hash(execution_policy_hash),
            raw_hash(verification_policy_hash),
            raw_hash(settlement_policy_hash),
            raw_hash(beta_risk_hash),
        )
        nonce = keccak(
            text=f"agent-bounties/base-mainnet/gmv-reward-cohort/{release_hash}/{candidate_id}/v1"
        )
        commitment = keccak(
            encode(
                ["uint256", "address", PARAM_TYPE, "bytes32"],
                [CHAIN_ID, to_checksum_address(factory), params, nonce],
            )
        )
        bounty_id = keccak(
            encode(
                ["uint256", "address", "address", "bytes32", PARAM_TYPE],
                [
                    CHAIN_ID,
                    to_checksum_address(factory),
                    to_checksum_address(reserve),
                    nonce,
                    params,
                ],
            )
        )
        predicted = create2(factory, bounty_id, keccak(competition_init))
        commitments.append(commitment)
        creations.append(
            {
                "candidate_id": candidate_id,
                "title": candidate.get("title"),
                "creation_nonce": hex_hash(nonce),
                "creation_commitment": hex_hash(commitment),
                "predicted_competition": predicted,
                "bounty_id": hex_hash(bounty_id),
                "scoring_window": {
                    "starts_at": epoch["starts_at"],
                    "ends_at": epoch["ends_at"],
                },
                "delegate_transaction": {
                    "from": delegate,
                    "to": reserve,
                    "value_wei": 0,
                    "data": calldata(
                        f"createCompetition({PARAM_TYPE},bytes32)",
                        [PARAM_TYPE, "bytes32"],
                        [params, nonce],
                    ),
                },
            }
        )

    valid_after = max(0, int(activation_time.timestamp()) - 60)
    valid_until = int(expires_at.timestamp())
    next_policy_tuple = (
        to_checksum_address(delegate),
        valid_after,
        valid_until,
        PERIOD_SECONDS,
        SOLVER_REWARD,
        KEEPER_REWARD,
        PER_COMPETITION,
        MAX_PER_PERIOD,
        MAX_LIFETIME,
        raw_hash(beta_risk_hash),
        raw_hash(metric_program_hash),
        raw_hash(journal_schema_hash),
    )
    next_policy_hash = keccak(
        encode([POLICY_TYPE, "bytes32[]"], [next_policy_tuple, commitments])
    )
    configure_data = calldata(
        f"configurePolicy({POLICY_TYPE},bytes32[])",
        [POLICY_TYPE, "bytes32[]"],
        [next_policy_tuple, commitments],
    )
    activation_bucket = int(activation_time.timestamp()) // PERIOD_SECONDS
    elapsed_period_sync = activation_bucket != period_bucket
    effective_period_spent = 0 if elapsed_period_sync else period_spent
    if effective_period_spent + treatment_total > MAX_PER_PERIOD:
        earliest_treatment_spend_at = (activation_bucket + 1) * PERIOD_SECONDS
    else:
        earliest_treatment_spend_at = int(activation_time.timestamp())
    return {
        "schema_version": SCHEMA,
        "network": "base-mainnet",
        "chain_id": CHAIN_ID,
        "safe_block": state["safe_block"],
        "owner": OWNER,
        "reserve_wallet": reserve,
        "settlement_token": USDC,
        "current_policy": {
            "version": current_version,
            "hash": current_policy_hash,
            "period_spent": period_spent,
            "lifetime_spent": lifetime_spent,
            "reserve_balance": reserve_balance,
        },
        "next_policy": {
            "version": current_version + 1,
            "hash": hex_hash(next_policy_hash),
            "delegate": delegate,
            "valid_after": valid_after,
            "valid_until": valid_until,
            "period_seconds": PERIOD_SECONDS,
            "solver_reward": SOLVER_REWARD,
            "keeper_reward": KEEPER_REWARD,
            "exact_funding_per_competition": PER_COMPETITION,
            "max_per_period": MAX_PER_PERIOD,
            "max_lifetime_spend": MAX_LIFETIME,
            "beta_risk_hash": beta_risk_hash,
            "gmv_metric_program_hash": metric_program_hash,
            "gmv_journal_schema_hash": journal_schema_hash,
        },
        "approved_creation_commitments": [hex_hash(value) for value in commitments],
        "creations": creations,
        "owner_transactions": {
            "revoke": {
                "from": OWNER,
                "to": reserve,
                "value_wei": 0,
                "data": calldata("revokePolicy()", [], []),
                "function": "revokePolicy()",
            },
            "configure": {
                "from": OWNER,
                "to": reserve,
                "value_wei": 0,
                "data": configure_data,
                "function": f"configurePolicy({POLICY_TYPE},bytes32[])",
            },
        },
        "execution_boundary": {
            "policy_change_moves_usdc": False,
            "policy_reconfiguration_does_not_create_a_fresh_period": True,
            "elapsed_period_sync_resets_spend": elapsed_period_sync,
            "observed_period_bucket": period_bucket,
            "effective_period_bucket": activation_bucket,
            "effective_period_spent_after_configuration": effective_period_spent,
            "earliest_treatment_spend_at": earliest_treatment_spend_at,
            "treatment_competition_count": len(candidates),
            "treatment_total_base_units": treatment_total,
            "reserve_after_treatment_base_units": reserve_balance - treatment_total,
            "later_floor_reserve_base_units": FUTURE_FLOOR_RESERVE,
        },
        "confirmation_deadline": int(confirmation_deadline),
        "confirmation_summary": {
            "owner": OWNER,
            "reserve": reserve,
            "action": "Revoke policy version 1, verify unchanged reserve state, then install five preapproved 6-USDC GMV meta-competitions",
            "transaction_count": 2,
            "transaction_value": "0 ETH each",
            "usdc_moved_by_confirmation": "0 USDC",
            "per_competition": "6.04 USDC",
            "maximum_utc_day": "30.40 USDC",
            "maximum_lifetime": "77.668098 USDC",
            "already_spent": "30.40 USDC",
            "uncommitted_reserve": "47.268098 USDC",
            "treatment_total": "30.20 USDC",
            "reserve_after_treatment": "17.068098 USDC",
            "later_floor_reserved": "15.20 USDC",
        },
        "recovery": {
            "owner": OWNER,
            "revoke_policy_call": {
                "to": reserve,
                "data": calldata("revokePolicy()", [], []),
            },
            "recover_uncommitted_call": {
                "to": reserve,
                "data": calldata("recoverUncommitted()", [], []),
            },
            "boundary": "The owner retains revocation and recovery of uncommitted USDC; active competition escrow follows canonical settlement or refund paths.",
        },
        "evidence_boundary": "These unsigned zero-value owner transactions are not a policy change, competition activation, GMV event, entry, payout, or settlement.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--cohort",
        type=Path,
        default=ROOT / "ops" / "open-competition-v2-forward-gmv-reward-cohort-v1.json",
    )
    parser.add_argument("--safe-state", type=Path, required=True)
    parser.add_argument("--activation-time")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        now = (
            parse_time(args.activation_time, "activation time")
            if args.activation_time
            else datetime.now(timezone.utc)
        )
        cohort = json.loads(args.cohort.read_text(encoding="utf-8-sig"))
        state = json.loads(args.safe_state.read_text(encoding="utf-8-sig"))
        value = build_rotation(cohort, state, now)
    except (
        OSError,
        json.JSONDecodeError,
        KeyError,
        TypeError,
        ValueError,
        RotationError,
    ) as error:
        print(f"reward policy build blocked: {error}")
        return 2
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
