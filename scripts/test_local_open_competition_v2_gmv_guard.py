#!/usr/bin/env python3
"""Network-free policy tests for the local V2 GMV floor guard."""

from __future__ import annotations

import unittest

from scripts.local_open_competition_v2_gmv_guard import (
    DAILY_CAP,
    INITIAL_FUNDING,
    PER_COMPETITION,
    GuardError,
    choose_creations,
)


def guard_state(active: int, used: int, *, period_spent: int = 0, lifetime_spent: int = 0):
    return {
        "active": active,
        "period_spent_base_units": period_spent,
        "lifetime_spent_base_units": lifetime_spent,
        "reserve_balance_base_units": INITIAL_FUNDING - lifetime_spent,
        "creations": [
            {
                "candidate_id": f"candidate-{index}-v1",
                "approved": True,
                "used": index < used,
            }
            for index in range(20)
        ],
    }


class LocalGmvGuardTests(unittest.TestCase):
    def test_private_inventory_states_restore_exact_target(self) -> None:
        for active in (10, 9, 5, 4, 0):
            with self.subTest(active=active):
                self.assertEqual(len(choose_creations(guard_state(active, active))), 10 - active)

    def test_other_or_used_candidates_cannot_satisfy_deficit(self) -> None:
        state = guard_state(4, 14)
        state["creations"][14]["approved"] = False
        with self.assertRaisesRegex(GuardError, "bounded capacity"):
            choose_creations(state)

    def test_daily_lifetime_and_balance_caps_fail_closed(self) -> None:
        cases = (
            guard_state(4, 4, period_spent=DAILY_CAP - PER_COMPETITION),
            guard_state(4, 20, lifetime_spent=INITIAL_FUNDING - PER_COMPETITION),
        )
        for state in cases:
            with self.subTest():
                with self.assertRaises(GuardError):
                    choose_creations(state)


if __name__ == "__main__":
    unittest.main()
