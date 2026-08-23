#!/usr/bin/env python3
"""Network-free policy tests for the local V2 GMV floor guard."""

from __future__ import annotations

import io
import json
import unittest
import sys
import urllib.error
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).resolve().parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

from local_open_competition_v2_gmv_guard import (  # noqa: E402
    DAILY_CAP,
    INITIAL_FUNDING,
    PER_COMPETITION,
    GuardError,
    choose_creations,
)
import local_open_competition_v2_gmv_guard as guard_module  # noqa: E402


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
    def test_rpc_retries_http_rate_limit_with_bounded_backoff(self) -> None:
        rate_limit = urllib.error.HTTPError(
            "https://primary.invalid", 429, "rate limited", {}, None
        )
        success = io.BytesIO(json.dumps({"jsonrpc": "2.0", "id": 1, "result": "0x2105"}).encode())
        guard_module._LAST_RPC_REQUEST.clear()
        with (
            mock.patch.object(guard_module, "RPC_MIN_INTERVAL_SECONDS", 0),
            mock.patch.object(
                guard_module.urllib.request,
                "urlopen",
                side_effect=[rate_limit, success],
            ) as urlopen,
            mock.patch.object(guard_module.time, "sleep") as sleep,
        ):
            result = guard_module.rpc("https://primary.invalid", "eth_chainId", [], 1)
        self.assertEqual(result, "0x2105")
        self.assertEqual(urlopen.call_count, 2)
        sleep.assert_called_once_with(guard_module.RPC_RETRY_DELAYS[0])

    def test_rpc_does_not_retry_contract_error(self) -> None:
        failure = io.BytesIO(
            json.dumps(
                {"jsonrpc": "2.0", "id": 1, "error": {"code": 3, "message": "execution reverted"}}
            ).encode()
        )
        guard_module._LAST_RPC_REQUEST.clear()
        with (
            mock.patch.object(guard_module, "RPC_MIN_INTERVAL_SECONDS", 0),
            mock.patch.object(guard_module.urllib.request, "urlopen", return_value=failure) as urlopen,
            mock.patch.object(guard_module.time, "sleep") as sleep,
        ):
            with self.assertRaisesRegex(GuardError, "returned an error"):
                guard_module.rpc("https://primary.invalid", "eth_call", [], 1)
        urlopen.assert_called_once()
        sleep.assert_not_called()

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
