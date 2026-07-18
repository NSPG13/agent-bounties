#!/usr/bin/env python3

from __future__ import annotations

import tempfile
import unittest
from argparse import Namespace
from datetime import datetime, timezone
from pathlib import Path
from unittest.mock import patch

import leaderboard_reward_pipeline as pipeline


CONTRACT = "0x1111111111111111111111111111111111111111"
WINNER = "0x2222222222222222222222222222222222222222"
SECOND = "0x3333333333333333333333333333333333333333"


def response_fixture() -> dict:
    def period(kind: str, starts_at: str, ends_at: str, reward: str) -> dict:
        return {
            "period_status": "closed",
            "reward_usdc": "3.00" if kind == "daily" else "26.00",
            "reward_funding_status": "funded",
            "reward_payout_status": "awaiting_finalization",
            "reward_contract": CONTRACT,
            "reward_paid_wallet": None,
            "reward_payout_observed_safe_block": 10,
            "reward_payout_observed_safe_block_hash": "0x" + "a" * 64,
            "ranking": {
                "period": {
                    "kind": kind,
                    "starts_at": starts_at,
                    "ends_at": ends_at,
                },
                "minimum_solver_reward_usdc_base_units": "2000000",
                "reward_usdc_base_units": reward,
                "leader_wallet": WINNER,
                "entries": [
                    {
                        "rank": 1,
                        "solver_wallet": WINNER,
                        "completed_bounties": 3,
                        "prize_eligible_bounties": 2,
                        "excluded_bounties": 1,
                        "eligible_solver_rewards_usdc_base_units": "5000000",
                        "last_eligible_settlement_at": "2026-07-19T20:00:00Z",
                        "exclusion_counts": {"standing_meta_bounty": 1},
                    },
                    {
                        "rank": 2,
                        "solver_wallet": SECOND,
                        "completed_bounties": 1,
                        "prize_eligible_bounties": 1,
                        "excluded_bounties": 0,
                        "eligible_solver_rewards_usdc_base_units": "2000000",
                        "last_eligible_settlement_at": "2026-07-19T19:00:00Z",
                        "exclusion_counts": {},
                    },
                ],
                "rules": ["confirmed events", "earliest final settlement wins ties"],
            },
        }

    return {
        "schema_version": "agent-bounties/solver-leaderboard-v1",
        "network": "base-mainnet",
        "reward_pool": {
            "contract": CONTRACT,
            "balance_usdc_base_units": "47000000",
        },
        "daily": period(
            "daily", "2026-07-19T00:00:00Z", "2026-07-20T00:00:00Z", "3000000"
        ),
        "weekly": period(
            "weekly", "2026-07-13T00:00:00Z", "2026-07-20T00:00:00Z", "26000000"
        ),
    }


class LeaderboardRewardPipelineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.now = datetime(2026, 7, 20, 2, tzinfo=timezone.utc)
        self.reference = datetime(2026, 7, 19, 23, 59, 59, tzinfo=timezone.utc)

    def test_period_references_select_previous_day_and_monday_week(self) -> None:
        references = pipeline.period_references(self.now)
        self.assertEqual(references["daily"], self.reference)
        self.assertEqual(references["weekly"], self.reference)

        tuesday = datetime(2026, 7, 21, 2, tzinfo=timezone.utc)
        references = pipeline.period_references(tuesday)
        self.assertEqual(references["daily"].isoformat(), "2026-07-20T23:59:59+00:00")
        self.assertEqual(references["weekly"], self.reference)

    def test_candidate_commits_to_exact_ranked_evidence(self) -> None:
        candidate = pipeline.build_candidate(
            response_fixture(), "daily", CONTRACT, self.reference, self.now
        )
        self.assertEqual(candidate["winner"], WINNER)
        self.assertEqual(candidate["eligible_completions"], 2)
        self.assertEqual(candidate["reward_usdc_base_units"], 3_000_000)
        self.assertRegex(candidate["evidence_hash"], pipeline.HASH)

        repeated = pipeline.build_candidate(
            response_fixture(), "daily", CONTRACT, self.reference, self.now
        )
        self.assertEqual(candidate, repeated)

    def test_no_winner_and_insufficient_pool_are_skipped(self) -> None:
        no_winner = response_fixture()
        no_winner["daily"]["ranking"]["leader_wallet"] = None
        no_winner["daily"]["ranking"]["entries"] = []
        with self.assertRaisesRegex(pipeline.NoCandidate, "no eligible winner"):
            pipeline.build_candidate(no_winner, "daily", CONTRACT, self.reference, self.now)

        unfunded = response_fixture()
        unfunded["reward_pool"]["balance_usdc_base_units"] = "2999999"
        with self.assertRaisesRegex(pipeline.NoCandidate, "insufficient"):
            pipeline.build_candidate(unfunded, "daily", CONTRACT, self.reference, self.now)

    def test_paid_winner_mismatch_fails_closed(self) -> None:
        response = response_fixture()
        response["daily"]["reward_payout_status"] = "paid_to_different_wallet"
        response["daily"]["reward_paid_wallet"] = SECOND
        with self.assertRaisesRegex(pipeline.PipelineError, "conflicts"):
            pipeline.build_candidate(response, "daily", CONTRACT, self.reference, self.now)

    def test_candidate_drift_fails_closed(self) -> None:
        first = pipeline.build_candidate(
            response_fixture(), "daily", CONTRACT, self.reference, self.now
        )
        changed_response = response_fixture()
        changed_response["daily"]["ranking"]["entries"][0][
            "prize_eligible_bounties"
        ] = 3
        changed = pipeline.build_candidate(
            changed_response, "daily", CONTRACT, self.reference, self.now
        )
        with self.assertRaisesRegex(pipeline.PipelineError, "changed"):
            pipeline.assert_candidate_unchanged(first, changed)

    def test_runner_fetches_shared_monday_reference_once(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            args = Namespace(
                api_base="https://example.test",
                network="base-mainnet",
                rpc_url="https://rpc.example.test",
                contract=CONTRACT,
                expected_token=pipeline.BASE_MAINNET_USDC,
                output=Path(temporary),
                cast=Path("cast"),
            )
            with (
                patch.object(pipeline, "fetch_leaderboard", return_value=response_fixture()) as fetch,
                patch.object(pipeline, "validate_contract", return_value={WINNER, SECOND}),
                patch.object(
                    pipeline,
                    "contract_period_starts",
                    return_value={"daily": 0, "weekly": 0},
                ),
                patch.object(pipeline, "datetime") as clock,
            ):
                clock.now.return_value = self.now
                clock.fromisoformat.side_effect = datetime.fromisoformat
                pipeline.command_run(args)
            self.assertEqual(fetch.call_count, 1)
            manifest = pipeline.read_json(Path(temporary) / "manifest.json")
            self.assertEqual(len(manifest["candidates"]), 2)

    def test_runner_skips_period_before_contract_program_start(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            args = Namespace(
                api_base="https://example.test",
                network="base-mainnet",
                rpc_url="https://rpc.example.test",
                contract=CONTRACT,
                expected_token=pipeline.BASE_MAINNET_USDC,
                output=Path(temporary),
                cast=Path("cast"),
            )
            with (
                patch.object(pipeline, "fetch_leaderboard", return_value=response_fixture()),
                patch.object(pipeline, "validate_contract", return_value={WINNER, SECOND}),
                patch.object(
                    pipeline,
                    "contract_period_starts",
                    return_value={"daily": 0, "weekly": 2_000_000_000},
                ),
                patch.object(pipeline, "datetime") as clock,
            ):
                clock.now.return_value = self.now
                clock.fromisoformat.side_effect = datetime.fromisoformat
                pipeline.command_run(args)
            manifest = pipeline.read_json(Path(temporary) / "manifest.json")
            self.assertEqual(len(manifest["candidates"]), 1)
            self.assertEqual(
                manifest["skipped"],
                [{"period": "weekly", "reason": "period predates program"}],
            )


if __name__ == "__main__":
    unittest.main()
