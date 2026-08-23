#!/usr/bin/env python3
"""Build twenty reviewed forward GMV meta-competition candidates."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re

from eth_abi import encode
from eth_utils import keccak

from forward_canonical_gmv import verification_policy_hash


ROOT = Path(__file__).resolve().parents[1]
PROFILE_ID = "forward-canonical-gmv-attribution-metric-v2"
SCHEMA = "agent-bounties/open-competition-v2-forward-gmv-meta-candidate-specs-v2"
PROTOCOL = "agent-bounties/open-competition-v2-beta3"
METRIC_PROGRAM_HASH = "0xe1b52ffcfff0675b7dacea84dcabdf3fbcf1cde09b3d2fb55aa389acac5c2ff9"
JOURNAL_SCHEMA_HASH = "0x660ddc720ea9fc13e7bbdd88839a2ac7b19a124e5daf046518350fa6febe8a40"
EXECUTION_POLICY_HASH = "0x0f4a13e4bedc6c4e2445c75059153cca12ee4fade502850b661cc2d8a8b2f30a"
SETTLEMENT_POLICY_HASH = "0xa664183e3688ef42f3c48c0942e5dac1c4108a17b1556c20da4ad05d5e95e8ee"
ATTESTERS = [
    "0x6fe4d6da2a4371d82b4a7ff94810a94091fb4c35",
    "0xfd7be4c69541ab297aece2a674fc1418b898cc0a",
]
OWNER = "0x884834e884d6e93462655a2820140ad03e6747bc"
BASE_EXCLUDED_WALLETS = [
    "0x1eaa1c68772cf76bc5f4e4174766076e33ace662",
    "0x6fe4d6da2a4371d82b4a7ff94810a94091fb4c35",
    OWNER,
    "0xb358898d34c5e907877a1cd7540b234f6851f61b",
    "0xfb58949365e3a30fd62e86edb0daffccf4ef7477",
    "0xfd7be4c69541ab297aece2a674fc1418b898cc0a",
]
EXCLUDED_CONTRACTS = [
    "0x3e052b933628b960d61654a68fca23d869d8989f",
    "0x5f884d4a4cc2727ddbc22382efd776274bc3e7aa",
    "0xaa4a9300bb1c90f93b4048fd83298da6c6145734",
    "0xf8c8897e748e4057d52182c27beb4025f4d49d68",
]
WINDOWS = (
    ("daily-20260824", "2026-08-24T00:00:00Z", "2026-08-25T00:00:00Z", "initial"),
    ("daily-20260825", "2026-08-25T00:00:00Z", "2026-08-26T00:00:00Z", "initial"),
    ("daily-20260826", "2026-08-26T00:00:00Z", "2026-08-27T00:00:00Z", "initial"),
    ("daily-20260827", "2026-08-27T00:00:00Z", "2026-08-28T00:00:00Z", "initial"),
    ("week-20260824", "2026-08-24T00:00:00Z", "2026-08-31T00:00:00Z", "initial"),
    ("week-20260831", "2026-08-31T00:00:00Z", "2026-09-07T00:00:00Z", "initial"),
    ("fortnight-20260824", "2026-08-24T00:00:00Z", "2026-09-07T00:00:00Z", "initial"),
    ("fortnight-20260907", "2026-09-07T00:00:00Z", "2026-09-21T00:00:00Z", "initial"),
    ("month-20260824", "2026-08-24T00:00:00Z", "2026-09-21T00:00:00Z", "initial"),
    ("sprint-20260824", "2026-08-24T00:00:00Z", "2026-08-27T00:00:00Z", "initial"),
    ("daily-20260828", "2026-08-28T00:00:00Z", "2026-08-29T00:00:00Z", "standby"),
    ("daily-20260829", "2026-08-29T00:00:00Z", "2026-08-30T00:00:00Z", "standby"),
    ("daily-20260830", "2026-08-30T00:00:00Z", "2026-08-31T00:00:00Z", "standby"),
    ("week-20260907", "2026-09-07T00:00:00Z", "2026-09-14T00:00:00Z", "standby"),
    ("week-20260914", "2026-09-14T00:00:00Z", "2026-09-21T00:00:00Z", "standby"),
    ("fortnight-20260921", "2026-09-21T00:00:00Z", "2026-10-05T00:00:00Z", "standby"),
    ("month-20260921", "2026-09-21T00:00:00Z", "2026-10-19T00:00:00Z", "standby"),
    ("sprint-20260827", "2026-08-27T00:00:00Z", "2026-08-30T00:00:00Z", "standby"),
    ("sprint-20260830", "2026-08-30T00:00:00Z", "2026-09-02T00:00:00Z", "standby"),
    ("sprint-20260902", "2026-09-02T00:00:00Z", "2026-09-05T00:00:00Z", "standby"),
)
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
HASH = re.compile(r"^0x[0-9a-f]{64}$")


def timestamp(value: str) -> int:
    return int(datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp())


def reviewed_timestamp(value: str) -> str:
    if not value.endswith("Z"):
        raise ValueError("approved_at must be an exact UTC timestamp ending in Z")
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo != timezone.utc or parsed.microsecond != 0:
        raise ValueError("approved_at must use whole UTC seconds")
    if parsed >= datetime.fromisoformat(WINDOWS[0][1].replace("Z", "+00:00")):
        raise ValueError("approved_at must precede the first forward scoring window")
    return value


def predict_reserve_wallet(
    reserve_factory: str,
    reserve_implementation: str,
    release_hash: str,
    owner: str = OWNER,
) -> str:
    for value, field in (
        (reserve_factory, "reserve factory"),
        (reserve_implementation, "reserve implementation"),
        (owner, "reserve owner"),
    ):
        if not ADDRESS.fullmatch(value.lower()):
            raise ValueError(f"{field} must be an exact address")
    if not HASH.fullmatch(release_hash.lower()):
        raise ValueError("release hash must be exact bytes32")
    user_salt = keccak(
        text=f"agent-bounties/base-mainnet/gmv-meta-reserve/{owner.lower()}/{release_hash.lower()}/v1"
    )
    effective_salt = keccak(
        encode(["address", "bytes32"], [owner.lower(), user_salt])
    )
    init_code = (
        bytes.fromhex("3d602d80600a3d3981f3")
        + bytes.fromhex("363d3d373d3d3d363d73")
        + bytes.fromhex(reserve_implementation.lower()[2:])
        + bytes.fromhex("5af43d82803e903d91602b57fd5bf3")
    )
    return "0x" + keccak(
        b"\xff"
        + bytes.fromhex(reserve_factory.lower()[2:])
        + effective_salt
        + keccak(init_code)
    )[12:].hex()


def build(
    factory: str,
    release_hash: str,
    reserve_factory: str,
    reserve_implementation: str,
    identity: dict,
    approved_at: str,
) -> dict:
    factory = factory.lower()
    release_hash = release_hash.lower()
    if not ADDRESS.fullmatch(factory) or not HASH.fullmatch(release_hash):
        raise ValueError("factory and release hash must be exact lowercase values")
    reserve_wallet = predict_reserve_wallet(
        reserve_factory.lower(), reserve_implementation.lower(), release_hash
    )
    if reserve_wallet in BASE_EXCLUDED_WALLETS:
        raise ValueError("the release-bound reserve collides with a reviewed operator wallet")
    excluded_wallets = sorted([*BASE_EXCLUDED_WALLETS, reserve_wallet])
    reproduced = identity.get("status") == "reproduced_beta3"
    profile = {
        "profile_id": PROFILE_ID,
        "status": "reviewed" if reproduced else "awaiting_reproduction",
        "metric_program_hash": METRIC_PROGRAM_HASH,
        "journal_schema_hash": JOURNAL_SCHEMA_HASH,
        "execution_policy_hash": EXECUTION_POLICY_HASH,
        "settlement_policy_hash": SETTLEMENT_POLICY_HASH,
        "program_vkey": identity.get("program_vkey") if reproduced else None,
        "source_hash": identity.get("source_hash") if reproduced else None,
        "elf_hash": identity.get("elf_keccak256") if reproduced else None,
    }
    candidates = []
    for name, starts_at, ends_at, _private_role in WINDOWS:
        candidate_id = f"external-gmv-forward-{name}-v2"
        campaign = {
            "epoch_id": "0x" + keccak(text=f"agent-bounties/canonical-gmv-epoch-v1\0{candidate_id}").hex(),
            "starts_at": timestamp(starts_at),
            "ends_at": timestamp(ends_at),
            "minimum_score_base_units": 1,
            "excluded_wallets": excluded_wallets,
            "excluded_bounty_contracts": EXCLUDED_CONTRACTS,
            "snapshot_attesters": ATTESTERS,
            "snapshot_attestation_threshold": 2,
        }
        policy_hash = "0x" + verification_policy_hash(campaign).hex()
        candidates.append(
            {
                "candidate_id": candidate_id,
                "title": f"Highest externally funded canonical GMV — {name.replace('-', ' ')}",
                "summary": "Compete to create and fund marketplace demand that settles canonically during the announced forward window. Highest pro-rata externally funded GMV wins.",
                "gmv_lane": "external_supply",
                "epoch": {
                    "starts_at": starts_at,
                    "ends_at": ends_at,
                    "minimum_score_base_units": 1,
                    "epoch_id": campaign["epoch_id"],
                },
                "snapshot": {
                    "status": "scheduled",
                    "verification_policy_hash": policy_hash,
                    "snapshot_attesters": ATTESTERS,
                    "snapshot_attestation_threshold": 2,
                    "canonical_snapshot_due_after": ends_at,
                },
                "analysis_sources": [
                    {
                        "kind": "canonical_platform_metric",
                        "url": "https://api.agentbounties.app/v1/metrics/platform?period=28d",
                    }
                ],
                "feedback_sources": [
                    {
                        "kind": "documented_contributor_feedback",
                        "url": "https://github.com/NSPG13/agent-bounties/blob/main/docs/distribution-learning.md",
                    }
                ],
            }
        )
    return {
        "schema_version": SCHEMA,
        "protocol_version": PROTOCOL,
        "network": "base-mainnet",
        "factory_contract": factory,
        "release_hash": release_hash,
        "reserve_wallet": reserve_wallet,
        "profile_release": profile,
        "approved_at": reviewed_timestamp(approved_at),
        "expires_at": "2026-10-31T23:59:59Z",
        "economics": {
            "solver_reward_base_units": 3_000_000,
            "keeper_reward_base_units": 40_000,
            "total_per_competition_base_units": 3_040_000,
        },
        "eligibility_policy": {
            "excluded_wallets": excluded_wallets,
            "excluded_bounty_contracts": EXCLUDED_CONTRACTS,
            "wallet_boundary": "Known owner, reserve, delegate, deployer, and snapshot-attester wallets are ineligible. Wallets are not inferred to be unique people.",
            "contract_boundary": "Declared synthetic canaries and prior GMV reward contracts are ineligible and must be extended before each future snapshot review.",
        },
        "attestation_policy": {
            "attesters": ATTESTERS,
            "threshold": 2,
            "purpose": "Two deterministic signers attest the exact dual-indexer safe-block snapshot; neither signer nor AI selects the winner.",
        },
        "scoring_policy": {
            "score_unit": "usdc_base_units",
            "winner_mode": "best_score",
            "score_direction": "higher_is_better",
            "attribution": "settlement_gmv_times_entrant_funding_divided_by_total_funding",
            "tie_break": "earliest qualifying proof sequence",
        },
        "candidates": candidates,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--factory", required=True)
    parser.add_argument("--release-hash", required=True)
    parser.add_argument(
        "--approved-at",
        required=True,
        help="Actual UTC time when the exact release, reserve, exclusions, and candidates were reviewed",
    )
    parser.add_argument(
        "--reserve-deployment",
        type=Path,
        required=True,
        help="Exact protected-release bounded reserve deployment artifact",
    )
    parser.add_argument(
        "--identity",
        type=Path,
        default=ROOT / "programs/forward-canonical-gmv-attribution-metric-v2/release-identity.json",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "ops/open-competition-v2-forward-gmv-candidate-pool-v2.json",
    )
    args = parser.parse_args()
    reserve = json.loads(args.reserve_deployment.read_text(encoding="utf-8"))["reserve_factory"]
    value = build(
        args.factory,
        args.release_hash,
        reserve["address"],
        reserve["implementation"],
        json.loads(args.identity.read_text(encoding="utf-8")),
        args.approved_at,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
