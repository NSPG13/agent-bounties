#!/usr/bin/env python3
"""Build the deterministic non-production release fixture for forward GMV."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from forward_canonical_gmv import attestation_digest, sign_digest, snapshot_hash, verification_policy_hash


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "programs/canonical-gmv-attribution-metric-v1/fixtures/golden-v1.json"
OUTPUT = ROOT / "programs/forward-canonical-gmv-attribution-metric-v2/fixtures/golden-v1.json"
FIXTURE_KEYS = ("0x" + "00" * 31 + "01", "0x" + "00" * 31 + "02")


def hex_value(values: list[int]) -> str:
    return "0x" + bytes(values).hex()


def array(value: str) -> list[int]:
    return list(bytes.fromhex(value[2:]))


def settlement_wire(value: dict) -> dict:
    return {
        "protocol": value["protocol"],
        "bounty_contract": hex_value(value["bounty_contract"]),
        "bounty_id": hex_value(value["bounty_id"]),
        "creator": hex_value(value["creator"]),
        "solver": hex_value(value["solver"]),
        "settled_at": value["settled_at"],
        "block_number": value["block_number"],
        "transaction_hash": hex_value(value["transaction_hash"]),
        "log_index": value["log_index"],
        "gmv_base_units": value["gmv_base_units"],
        "funding": [
            {
                "contributor": hex_value(item["contributor"]),
                "amount_base_units": item["amount_base_units"],
            }
            for item in value["funding"]
        ],
    }


def build() -> dict:
    source = json.loads(SOURCE.read_text(encoding="utf-8"))
    old_campaign = source["campaign"]
    signatures = [sign_digest(key, bytes(32)) for key in FIXTURE_KEYS]
    attesters = sorted(item["signer"] for item in signatures)
    campaign_wire = {
        "epoch_id": hex_value(old_campaign["epoch_id"]),
        "starts_at": old_campaign["starts_at"],
        "ends_at": old_campaign["ends_at"],
        "minimum_score_base_units": old_campaign["minimum_score_base_units"],
        "excluded_wallets": sorted(hex_value(value) for value in old_campaign["excluded_wallets"]),
        "excluded_bounty_contracts": sorted(
            hex_value(value) for value in old_campaign["excluded_bounty_contracts"]
        ),
        "snapshot_attesters": attesters,
        "snapshot_attestation_threshold": 2,
    }
    snapshot_wire = {
        "start_block": old_campaign["start_block"],
        "end_safe_block": old_campaign["end_safe_block"],
        "end_block_hash": hex_value(old_campaign["end_block_hash"]),
        "settlements": [settlement_wire(value) for value in source["settlements"]],
    }
    policy = verification_policy_hash(campaign_wire)
    snapshot = snapshot_hash(campaign_wire, snapshot_wire)
    digest = attestation_digest(policy, snapshot, snapshot_wire["end_block_hash"])
    attestations = sorted((sign_digest(key, digest) for key in FIXTURE_KEYS), key=lambda value: value["signer"])
    return {
        "scope": source["scope"],
        "campaign": {
            "epoch_id": old_campaign["epoch_id"],
            "starts_at": old_campaign["starts_at"],
            "ends_at": old_campaign["ends_at"],
            "minimum_score_base_units": old_campaign["minimum_score_base_units"],
            "excluded_wallets": [array(value) for value in campaign_wire["excluded_wallets"]],
            "excluded_bounty_contracts": [
                array(value) for value in campaign_wire["excluded_bounty_contracts"]
            ],
            "snapshot_attesters": [array(value) for value in attesters],
            "snapshot_attestation_threshold": 2,
        },
        "snapshot": {
            "start_block": old_campaign["start_block"],
            "end_safe_block": old_campaign["end_safe_block"],
            "end_block_hash": old_campaign["end_block_hash"],
            "settlements": source["settlements"],
            "attestations": [
                {
                    "signer": array(item["signer"]),
                    "signature": array(item["signature"]),
                }
                for item in attestations
            ],
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(build(), indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
