#!/usr/bin/env python3
"""Tests for the read-only QICSWU production shadow."""

from __future__ import annotations

import copy
import json
import tempfile
import unittest
from datetime import date
from pathlib import Path

from scripts import qicswu_metric as metric
from scripts import qicswu_shadow as shadow


ROOT = Path(__file__).resolve().parents[1]
DAY = date(2026, 9, 7)
CAPTURED_AT = "2026-09-08T02:17:00Z"


def event(protocol: str, ordinal: int, *, occurred_at: str = "2026-09-07T12:00:00Z") -> dict:
    source = shadow.SOURCE_DEFINITIONS[protocol]
    reward_field = source["verifier_reward_field"]
    data = {
        "solver": "0x" + f"{ordinal + 100:040x}",
        "solver_reward": 100_000,
        reward_field: 10_000,
        "timeout_bond_bonus": 0,
    }
    if protocol == "autonomous":
        data["round"] = ordinal + 1
    return {
        "block_number": 1_000 + ordinal,
        "bounty_id": "0x" + f"{ordinal + 200:064x}",
        "contract_address": "0x" + f"{ordinal + 300:040x}",
        "data": data,
        "kind": source["settlement_event_kind"],
        "log_index": ordinal,
        "occurred_at": occurred_at,
        "protocol_version": source["protocol_version"],
        "tx_hash": "0x" + f"{ordinal + 400:064x}",
    }


def payloads() -> dict[str, object]:
    values: dict[str, object] = {}
    for ordinal, (protocol, source) in enumerate(shadow.SOURCE_DEFINITIONS.items()):
        rows = [event(protocol, ordinal)]
        if source["envelope_schema"] is None:
            values[source["url"]] = rows
        else:
            values[source["url"]] = {
                "schema_version": source["envelope_schema"],
                "network": shadow.NETWORK,
                "events": rows,
            }
    return values


def fetcher(values):
    def fetch(url):
        value = copy.deepcopy(values[url])
        return value, json.dumps(value).encode()

    return fetch


class QicswuShadowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.policy = metric.load_json(ROOT / "ops/metrics/qicswu-policy-v1.json")
        self.principals = metric.load_json(ROOT / "ops/metrics/qicswu-principal-registry-v1.json")
        self.root_map = metric.load_json(ROOT / "ops/metrics/qicswu-root-work-map-v1.json")

    def capture(self, values=None):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        output = Path(temporary.name) / "shadow"
        run = shadow.capture_day(
            self.policy,
            self.principals,
            self.root_map,
            day=DAY,
            captured_at=CAPTURED_AT,
            output_dir=output,
            fetcher=fetcher(values or payloads()),
        )
        return run, output

    def test_all_protocols_are_captured_and_open_competition_is_primary(self) -> None:
        run, output = self.capture()
        input_document = metric.load_json(output / "qicswu-input.json")
        self.assertEqual(run["primary_participation_path"], "open_competition")
        self.assertEqual(run["primary_protocol"], "open_competition_v2")
        self.assertEqual(
            run["measured_protocol_roles"],
            {"compatibility": 1, "legacy": 1, "primary": 1},
        )
        self.assertEqual(
            run["participation_path_source_counts"],
            {"exclusive_claim": 1, "open_competition": 2},
        )
        self.assertEqual({row["protocol"] for row in input_document["candidates"]}, set(shadow.SOURCE_DEFINITIONS))
        self.assertEqual(run["metric"]["status"], "unavailable")
        self.assertIsNone(run["metric"]["north_star_per_day"])
        self.assertFalse(run["publication_eligible"])

    def test_open_competition_v1_cannot_replace_v2_as_primary(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["product_path_priority"]["primary_protocol"] = "open_competition_v1"
        with self.assertRaisesRegex(shadow.ShadowCaptureError, "V2 must remain"):
            shadow.validate_policy(policy)

    def test_open_competition_v1_uses_the_canonical_solution_commit_event(self) -> None:
        policy = copy.deepcopy(self.policy)
        row = next(
            item
            for item in policy["canonical_settlement_protocols"]
            if item["protocol"] == "open_competition_v1"
        )
        row["participation_event_kind"] = "entry_committed"
        with self.assertRaisesRegex(shadow.ShadowCaptureError, "canonical protocol"):
            shadow.validate_policy(policy)

    def test_open_competition_cannot_be_redefined_as_claim_based(self) -> None:
        policy = copy.deepcopy(self.policy)
        row = next(
            item
            for item in policy["canonical_settlement_protocols"]
            if item["protocol"] == "open_competition_v2"
        )
        row["participation_event_kind"] = "bounty_claimed"
        with self.assertRaisesRegex(shadow.ShadowCaptureError, "canonical protocol"):
            shadow.validate_policy(policy)

    def test_new_policy_protocol_without_a_source_fails_closed(self) -> None:
        policy = copy.deepcopy(self.policy)
        extra = copy.deepcopy(policy["canonical_settlement_protocols"][0])
        extra["protocol"] = "unobserved_protocol"
        policy["canonical_settlement_protocols"].append(extra)
        with self.assertRaisesRegex(shadow.ShadowCaptureError, "every policy protocol"):
            shadow.validate_policy(policy)

    def test_out_of_window_settlements_are_not_selected(self) -> None:
        values = payloads()
        for protocol, source in shadow.SOURCE_DEFINITIONS.items():
            row = event(protocol, 10, occurred_at="2026-09-08T00:00:00Z")
            values[source["url"]] = (
                [row]
                if source["envelope_schema"] is None
                else {"schema_version": source["envelope_schema"], "network": shadow.NETWORK, "events": [row]}
            )
        run, _output = self.capture(values)
        self.assertEqual(sum(run["observed_settlement_candidates_by_protocol"].values()), 0)

    def test_cross_protocol_settlement_log_reuse_fails_closed(self) -> None:
        values = payloads()
        first = event("autonomous", 1)
        reused = event("open_competition_v1", 2)
        reused["tx_hash"] = first["tx_hash"]
        reused["log_index"] = first["log_index"]
        values[shadow.SOURCE_DEFINITIONS["autonomous"]["url"]] = [first]
        source = shadow.SOURCE_DEFINITIONS["open_competition_v1"]
        values[source["url"]] = {
            "schema_version": source["envelope_schema"],
            "network": shadow.NETWORK,
            "events": [reused],
        }
        with self.assertRaisesRegex(shadow.ShadowCaptureError, "reused"):
            self.capture(values)


if __name__ == "__main__":
    unittest.main()
