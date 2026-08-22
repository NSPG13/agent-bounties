#!/usr/bin/env python3
"""Network-free tests for the canonical GMV snapshot builder."""

from __future__ import annotations

import json
import sys
import unittest
from unittest import mock
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

import build_open_competition_v2_gmv_snapshots as MODULE


def hex_array(values: list[int]) -> str:
    return "0x" + bytes(values).hex()


def canonical_fixture() -> tuple[dict, list[dict], str]:
    value = json.loads(
        (
            ROOT
            / "programs/canonical-gmv-attribution-metric-v1/fixtures/golden-v1.json"
        ).read_text(encoding="utf-8")
    )
    campaign = value["campaign"]
    normalized_campaign = {
        "epoch_id": hex_array(campaign["epoch_id"]),
        "starts_at": campaign["starts_at"],
        "ends_at": campaign["ends_at"],
        "start_block": campaign["start_block"],
        "end_safe_block": campaign["end_safe_block"],
        "end_block_hash": hex_array(campaign["end_block_hash"]),
        "minimum_score_base_units": campaign["minimum_score_base_units"],
        "excluded_wallets": [hex_array(item) for item in campaign["excluded_wallets"]],
        "excluded_bounty_contracts": [
            hex_array(item) for item in campaign["excluded_bounty_contracts"]
        ],
    }
    settlements = []
    for settlement in value["settlements"]:
        settlements.append(
            {
                "protocol": settlement["protocol"],
                "bounty_contract": hex_array(settlement["bounty_contract"]),
                "bounty_id": hex_array(settlement["bounty_id"]),
                "creator": hex_array(settlement["creator"]),
                "solver": hex_array(settlement["solver"]),
                "settled_at": settlement["settled_at"],
                "block_number": settlement["block_number"],
                "transaction_hash": hex_array(settlement["transaction_hash"]),
                "log_index": settlement["log_index"],
                "gmv_base_units": settlement["gmv_base_units"],
                "funding": [
                    {
                        "contributor": hex_array(funding["contributor"]),
                        "amount_base_units": funding["amount_base_units"],
                    }
                    for funding in settlement["funding"]
                ],
            }
        )
    return normalized_campaign, settlements, hex_array(value["scope"]["source_hash"])


def address_topic(value: str) -> str:
    return "0x" + bytes.fromhex(value[2:]).rjust(32, b"\0").hex()


def uint_word(value: int) -> str:
    return value.to_bytes(32, "big").hex()


class GmvSnapshotBuilderTests(unittest.TestCase):
    def test_python_hashes_match_the_rust_release_vector(self) -> None:
        campaign, settlements, source_hash = canonical_fixture()
        snapshot = MODULE.snapshot_hash(campaign, settlements)
        policy = MODULE.verification_policy_hash(campaign, source_hash, snapshot)
        self.assertEqual(
            snapshot.hex(),
            "108f20c52064147687060a3c40ecf8558784f2aa0c848da4c1e9b23e0b36a053",
        )
        self.assertEqual(
            policy.hex(),
            "60a114110529d22982ce9794b288b6a2407bc2c0b13a9d86e2f4d23136de1a7b",
        )

    def test_eligible_scores_exclude_operator_self_dealing_and_reward_contracts(self) -> None:
        _, settlements, _ = canonical_fixture()
        settlement = settlements[0]
        operator = settlement["funding"][0]["contributor"]
        external = settlement["funding"][1]["contributor"]
        scores = MODULE.eligible_scores([settlement], {operator}, set())
        self.assertEqual(scores, {external: 1_000_000})

        settlement["solver"] = external
        self.assertEqual(MODULE.eligible_scores([settlement], {operator}, set()), {})
        settlement["solver"] = "0x" + "06" * 20
        self.assertEqual(
            MODULE.eligible_scores([settlement], {operator}, {settlement["bounty_contract"]}),
            {},
        )

    def test_creator_as_solver_never_generates_an_eligible_score(self) -> None:
        _, settlements, _ = canonical_fixture()
        settlements[0]["creator"] = settlements[0]["solver"]
        self.assertEqual(MODULE.eligible_scores(settlements, set(), set()), {})

    def test_raw_v2_record_is_derived_from_logs_not_api_amounts(self) -> None:
        protocol = MODULE.PROTOCOLS[2]
        bounty = "0x" + "11" * 32
        contract = "0x" + "22" * 20
        creator = "0x" + "33" * 20
        solver = "0x" + "44" * 20
        funder = "0x" + "55" * 20
        common = {
            "address": contract,
            "block_hash": "0x" + "66" * 32,
            "block_number": 100,
            "transaction_hash": "0x" + "77" * 32,
            "log_index": 1,
        }
        creation = {
            **common,
            "address": protocol.factory,
            "topics": [MODULE.topic(protocol.creation_signature), bounty, address_topic(contract), address_topic(creator)],
            "data": "0x" + "00" * 64,
        }
        settlement = {
            **common,
            "topics": [MODULE.topic(protocol.settlement_signature), bounty, "0x" + uint_word(1), address_topic(solver)],
            "data": "0x"
            + uint_word(3_000_000)
            + address_topic("0x" + "88" * 20)[2:]
            + uint_word(40_000)
            + "00" * (32 * 4),
        }
        funding = {
            **common,
            "topics": [MODULE.topic(protocol.funding_signature), bounty, address_topic(funder)],
            "data": "0x" + uint_word(3_040_000) * 3,
        }
        record = MODULE.raw_snapshot_record(
            protocol,
            {"bounty_id": bounty},
            settlement,
            creation,
            [funding],
            1_000,
        )
        self.assertEqual(record["gmv_base_units"], 3_040_000)
        self.assertEqual(record["creator"], creator)
        self.assertEqual(record["solver"], solver)
        self.assertEqual(record["funding"], [{"contributor": funder, "amount_base_units": 3_040_000}])

    def test_duplicate_funding_events_are_aggregated_by_contributor(self) -> None:
        protocol = MODULE.PROTOCOLS[0]
        bounty = "0x" + "11" * 32
        contract = "0x" + "22" * 20
        creator = "0x" + "33" * 20
        solver = "0x" + "44" * 20
        funder = "0x" + "55" * 20
        creation = {
            "address": protocol.factory,
            "block_hash": "0x" + "66" * 32,
            "block_number": 100,
            "transaction_hash": "0x" + "77" * 32,
            "log_index": 1,
            "topics": [MODULE.topic(protocol.creation_signature), bounty, address_topic(contract), address_topic(creator)],
            "data": "0x" + "00" * 96,
        }
        settlement = {
            **creation,
            "address": contract,
            "topics": [MODULE.topic(protocol.settlement_signature), bounty, "0x" + uint_word(1), address_topic(solver)],
            "data": "0x" + uint_word(1_000_000) + uint_word(100_000) + uint_word(0) + uint_word(100_000) + "00" * (32 * 4),
        }
        funding = {
            **creation,
            "address": contract,
            "topics": [MODULE.topic(protocol.funding_signature), bounty, address_topic(funder)],
            "data": "0x" + uint_word(600_000) + uint_word(600_000) + uint_word(1_200_000),
        }
        second = {**funding, "log_index": 2, "data": "0x" + uint_word(600_000) + uint_word(1_200_000) + uint_word(1_200_000)}
        record = MODULE.raw_snapshot_record(
            protocol, {"bounty_id": bounty}, settlement, creation, [funding, second], 1_000
        )
        self.assertEqual(record["funding"][0]["amount_base_units"], 1_200_000)

    def test_rpc_pair_rejects_same_endpoint_and_invalid_span(self) -> None:
        with self.assertRaisesRegex(MODULE.SnapshotError, "independent"):
            MODULE.RpcPair("https://rpc.example", "https://rpc.example", 1)
        with self.assertRaisesRegex(MODULE.SnapshotError, "span"):
            MODULE.RpcPair("https://a.example", "https://b.example", 0)
        with self.assertRaisesRegex(MODULE.SnapshotError, "maximum runtime"):
            MODULE.RpcPair("https://a.example", "https://b.example", 1, 59)
        with self.assertRaisesRegex(MODULE.SnapshotError, "address batch size"):
            MODULE.RpcPair(
                "https://a.example",
                "https://b.example",
                1,
                address_batch_size=0,
            )

    def test_log_query_accepts_a_factory_derived_contract_set(self) -> None:
        pair = MODULE.RpcPair("https://a.example", "https://b.example", 100)
        responses = []

        def fake_call(endpoint: str, method: str, params: list) -> list:
            self.assertEqual(method, "eth_getLogs")
            self.assertEqual(
                params[0]["address"],
                [
                    "0x1111111111111111111111111111111111111111",
                    "0x2222222222222222222222222222222222222222",
                ],
            )
            responses.append(endpoint)
            return []

        with mock.patch.object(pair, "call", side_effect=fake_call):
            self.assertEqual(
                pair.logs(
                    1,
                    2,
                    "0x" + "33" * 32,
                    [
                        "0x1111111111111111111111111111111111111111",
                        "0x2222222222222222222222222222222222222222",
                    ],
                ),
                [],
            )
        self.assertEqual(responses, [pair.primary, pair.shadow])

    def test_log_query_batches_factory_derived_contracts(self) -> None:
        pair = MODULE.RpcPair(
            "https://a.example",
            "https://b.example",
            100,
            address_batch_size=2,
        )
        addresses = [
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
            "0x3333333333333333333333333333333333333333",
        ]
        observed: list[tuple[str, list[str]]] = []

        def fake_call(endpoint: str, method: str, params: list) -> list:
            self.assertEqual(method, "eth_getLogs")
            observed.append((endpoint, params[0]["address"]))
            return []

        with mock.patch.object(pair, "call", side_effect=fake_call):
            self.assertEqual(pair.logs(1, 2, "0x" + "44" * 32, addresses), [])
        self.assertEqual(
            observed,
            [
                (pair.primary, addresses[:2]),
                (pair.shadow, addresses[:2]),
                (pair.primary, addresses[2:]),
                (pair.shadow, addresses[2:]),
            ],
        )

    def test_checked_in_pool_has_twenty_unique_closed_evidence_candidates(self) -> None:
        pool = json.loads(
            (ROOT / "ops/open-competition-v2-gmv-candidate-pool-v1.json").read_text(
                encoding="utf-8"
            )
        )
        identifiers = [candidate["candidate_id"] for candidate in pool["candidates"]]
        self.assertEqual(len(identifiers), 20)
        self.assertEqual(len(set(identifiers)), 20)
        self.assertTrue(pool["eligibility_policy"]["excluded_wallets"])
        self.assertTrue(pool["eligibility_policy"]["excluded_bounty_contracts"])
        for candidate in pool["candidates"]:
            self.assertLess(
                MODULE.parse_time(candidate["epoch"]["starts_at"], "start"),
                MODULE.parse_time(candidate["epoch"]["ends_at"], "end"),
            )
            snapshot = candidate["snapshot"]
            self.assertEqual(snapshot["status"], "ready")
            self.assertEqual(
                snapshot["primary_projection_hash"],
                snapshot["shadow_projection_hash"],
            )
            self.assertEqual(snapshot["snapshot_hash"], snapshot["primary_projection_hash"])
            document = json.loads(
                (
                    ROOT
                    / "site/generated/gmv-snapshots"
                    / f"{candidate['candidate_id']}.json"
                ).read_text(encoding="utf-8")
            )
            self.assertEqual(document["candidate_id"], candidate["candidate_id"])
            self.assertEqual(document["snapshot_hash"], snapshot["snapshot_hash"])
            self.assertEqual(
                document["snapshot_hash"],
                "0x"
                + MODULE.snapshot_hash(
                    document["campaign"], document["settlements"]
                ).hex(),
            )
            self.assertEqual(
                document["verification_policy_hash"],
                "0x"
                + MODULE.verification_policy_hash(
                    document["campaign"],
                    pool["profile_release"]["source_hash"],
                    bytes.fromhex(document["snapshot_hash"].removeprefix("0x")),
                ).hex(),
            )
            self.assertGreater(document["eligible_wallet_count"], 0)
            self.assertGreater(int(document["eligible_gmv_base_units"]), 0)
            self.assertEqual(
                document["reconciliation"]["status"], "primary_shadow_agree"
            )


if __name__ == "__main__":
    unittest.main()
