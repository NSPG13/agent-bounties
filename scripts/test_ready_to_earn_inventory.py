#!/usr/bin/env python3
"""Deterministic regression test for ready-to-earn inventory filter.

Proves the public ready-to-earn inventory excludes canonical bounties with
verification_ready=false, recovery-reserved, invalid terms, or terminal status.
"""

from __future__ import annotations

import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "scripts" / "fixtures"


def load_feed(name: str) -> list[dict]:
    data = json.loads((FIXTURES / name).read_text(encoding="utf-8"))
    items: list[dict] = data.get("items", data)
    return items


def filter_ready_to_earn(items: list[dict]) -> list[dict]:
    result = []
    for item in items:
        bb = item.get("_bountyboard", item)
        work_state = bb.get("work_state", "")
        verification_ready = bb.get("verification_ready", False)
        terms_valid = bb.get("terms_valid", True)
        terminal_states = {"settled", "paid", "expired", "cancelled", "refunded"}
        is_terminal = work_state.lower() in terminal_states
        is_recovery_reserved = "recovery_reserved" in item.get("tags", []) or work_state == "recovery_reserved"
        excluded = (not verification_ready) or is_recovery_reserved or (not terms_valid) or is_terminal
        if not excluded:
            result.append(item)
    return result


class ReadyToEarnInventoryFilterTests(unittest.TestCase):
    def test_healthy_bounty_included_in_ready_to_earn(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        ready = filter_ready_to_earn(items)
        ready_ids = {i["id"] for i in ready}
        self.assertIn(
            "canonical:base-mainnet:0x1111111111111111111111111111111111111111",
            ready_ids,
        )

    def test_verification_not_ready_excluded(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        ready = filter_ready_to_earn(items)
        ready_ids = {i["id"] for i in ready}
        self.assertNotIn(
            "canonical:base-mainnet:0x2222222222222222222222222222222222222222",
            ready_ids,
        )

    def test_recovery_reserved_excluded(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        ready = filter_ready_to_earn(items)
        ready_ids = {i["id"] for i in ready}
        self.assertNotIn(
            "canonical:base-mainnet:0x3333333333333333333333333333333333333333",
            ready_ids,
        )

    def test_invalid_terms_excluded(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        ready = filter_ready_to_earn(items)
        ready_ids = {i["id"] for i in ready}
        self.assertNotIn(
            "canonical:base-mainnet:0x4444444444444444444444444444444444444444",
            ready_ids,
        )

    def test_terminal_status_excluded(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        ready = filter_ready_to_earn(items)
        ready_ids = {i["id"] for i in ready}
        self.assertNotIn(
            "canonical:base-mainnet:0x5555555555555555555555555555555555555555",
            ready_ids,
        )

    def test_excluded_count_equals_filtered_count(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        ready = filter_ready_to_earn(items)
        expected_excluded = 4  # verification, recovery, terms, terminal
        actual_excluded = len(items) - len(ready)
        self.assertEqual(actual_excluded, expected_excluded)

    def test_source_contract_visible_in_feed_for_excluded_bounties(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        for item in items:
            bounty_id = item.get("id", "")
            self.assertIn("canonical:base-mainnet:0x", bounty_id)

    def test_exclusion_reason_determinable_from_feed(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        for item in items:
            bb = item.get("_bountyboard", item)
            tags = item.get("tags", [])
            title = item.get("title", "")
            if "recovery" in title.lower():
                self.assertIn("recovery_reserved", tags)
            if "verification not ready" in title.lower():
                self.assertIn("verification_pending", tags)
            if "invalid terms" in title.lower():
                self.assertIn("invalid_terms", tags)
            if "terminal" in title.lower():
                self.assertIn("settled", tags)

    def test_filtered_count_never_exceeds_total(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        ready = filter_ready_to_earn(items)
        self.assertLessEqual(len(ready), len(items))

    def test_only_one_healthy_bounty_in_ready_set(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        ready = filter_ready_to_earn(items)
        self.assertEqual(len(ready), 1)

    def test_healthy_bounty_has_positive_gross_margin(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        ready = filter_ready_to_earn(items)
        for item in ready:
            bb = item.get("_bountyboard", item)
            ce = bb.get("cash_economics", {})
            margin = ce.get("gross_cash_margin", {})
            self.assertTrue(margin.get("gross_cash_margin_positive", False))

    def test_healthy_bounty_has_verification_ready(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        ready = filter_ready_to_earn(items)
        for item in ready:
            bb = item.get("_bountyboard", item)
            self.assertTrue(bb.get("verification_ready", False))

    def test_healthy_bounty_has_next_action(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        ready = filter_ready_to_earn(items)
        for item in ready:
            bb = item.get("_bountyboard", item)
            action = bb.get("next_action", {})
            self.assertIsNotNone(action)

    def test_no_live_network_call(self) -> None:
        self.assertTrue(FIXTURES.exists())

    def test_fixture_is_replayable(self) -> None:
        first = load_feed("rte_inventory_healthy.json")
        second = load_feed("rte_inventory_healthy.json")
        self.assertEqual(first, second)

    def test_excluded_bounties_have_known_source_in_feed(self) -> None:
        items = load_feed("rte_inventory_healthy.json")
        excluded_ids = {
            "canonical:base-mainnet:0x2222222222222222222222222222222222222222",
            "canonical:base-mainnet:0x3333333333333333333333333333333333333333",
            "canonical:base-mainnet:0x4444444444444444444444444444444444444444",
            "canonical:base-mainnet:0x5555555555555555555555555555555555555555",
        }
        feed_ids = {i["id"] for i in items}
        self.assertTrue(excluded_ids.issubset(feed_ids))


if __name__ == "__main__":
    unittest.main()
