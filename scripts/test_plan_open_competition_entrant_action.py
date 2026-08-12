from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import plan_open_competition_entrant_action as planner  # noqa: E402


WALLET = "0x1111111111111111111111111111111111111111"
OWNER = "0x2222222222222222222222222222222222222222"
DELEGATE = "0x3333333333333333333333333333333333333333"
CREATOR = "0x4444444444444444444444444444444444444444"
BOUNTY = "0x5555555555555555555555555555555555555555"
COMMITMENT = "0x" + "66" * 32
SUBMISSION = "0x" + "77" * 32
EVIDENCE = "0x" + "88" * 32
SALT = "0x" + "99" * 32


def report(*, entered: bool = False, entry_state: int = 0, wallet_balance: int = 1_000_000) -> dict:
    return {
        "network": "base-sepolia",
        "chain_id": 84_532,
        "safe_block": {"number": 100, "hash": "0x" + "aa" * 32, "timestamp": 1_000_000},
        "wallet": WALLET,
        "bounty": BOUNTY,
        "wallet_state": {
            "owner": OWNER,
            "policy": {
                "delegate": DELEGATE,
                "valid_after": 900_000,
                "valid_until": 2_000_000,
                "period_seconds": 3_600,
                "max_per_action": 100_000,
                "max_per_period": 200_000,
                "max_lifetime_spend": 300_000,
                "max_bounty_target": 2_000_000,
                "allowed_actions": 7,
                "verifier_module": "0x6666666666666666666666666666666666666666",
                "verifier_runtime_code_hash": "0x" + "ab" * 32,
                "verifier_policy_hash": "0x" + "bc" * 32,
                "acceptance_criteria_hash": "0x" + "cd" * 32,
                "benchmark_hash": "0x" + "de" * 32,
                "evidence_schema_hash": "0x" + "ef" * 32,
            },
            "policy_hash": "0x" + "12" * 32,
            "policy_version": 3,
            "delegate_nonce": 5,
            "period_bucket": 277,
            "period_spent": 50_000,
            "lifetime_spent": 100_000,
            "revoked": False,
            "token_balance": wallet_balance,
        },
        "bounty_state": {
            "factory": "0x7777777777777777777777777777777777777777",
            "settlement_token": "0x8888888888888888888888888888888888888888",
            "creator": CREATOR,
            "target_amount": 1_100_000,
            "verifier_reward": 100_000,
            "status": 1,
            "competition_ends_at": 1_500_000,
            "entry_count": 1 if entered else 0,
            "max_entries": 4,
            "verifier_module": "0x6666666666666666666666666666666666666666",
            "policy_hash": "0x" + "bc" * 32,
            "acceptance_criteria_hash": "0x" + "cd" * 32,
            "benchmark_hash": "0x" + "de" * 32,
            "evidence_schema_hash": "0x" + "ef" * 32,
            "has_entered": entered,
            "entry": {
                "commitment": COMMITMENT if entered else planner.ZERO_HASH,
                "committed_block": 99 if entered else 0,
                "reveal_deadline": 1_200_000 if entered else 0,
                "bond": 100_000 if entered else 0,
                "state": entry_state,
            },
        },
    }


def envelope() -> dict:
    return {
        "schema_version": planner.COMMITMENT_SCHEMA,
        "network": "base-sepolia",
        "chain_id": 84_532,
        "bounty": BOUNTY,
        "solver": WALLET,
        "submission_hash": SUBMISSION,
        "evidence_hash": EVIDENCE,
        "salt": SALT,
        "commitment": COMMITMENT,
        "committed_block": 0,
        "reveal_deadline": 0,
        "evidence_boundary": "local recovery material",
    }


class EntrantActionPlannerTests(unittest.TestCase):
    def test_commit_never_serializes_salt_or_reveal_material(self) -> None:
        plan = planner.build_plan(report(), "commit", envelope(), None, 300)
        serialized = json.dumps(plan).lower()
        self.assertNotIn(SALT[2:], serialized)
        self.assertNotIn(SUBMISSION[2:], serialized)
        self.assertNotIn(EVIDENCE[2:], serialized)
        self.assertEqual(plan["action_summary"]["commitment"], COMMITMENT)
        self.assertEqual(plan["nonce"], 5)
        self.assertEqual(plan["deadline"], 1_000_300)

    def test_reveal_requires_exact_live_commitment_and_later_safe_block(self) -> None:
        current = report(entered=True, entry_state=1)
        plan = planner.build_plan(current, "reveal", envelope(), "0xaabb", 300)
        self.assertEqual(plan["action_summary"]["salt"], SALT)
        self.assertEqual(plan["action_summary"]["proof_hash"], planner.run_cast("keccak", "0xaabb"))
        current["bounty_state"]["entry"]["commitment"] = "0x" + "01" * 32
        with self.assertRaisesRegex(SystemExit, "onchain commitment differs"):
            planner.build_plan(current, "reveal", envelope(), "0xaabb", 300)

    def test_reveal_rechecks_creator_control_after_commit(self) -> None:
        current = report(entered=True, entry_state=1)
        current["wallet_state"]["owner"] = CREATOR
        with self.assertRaisesRegex(SystemExit, "creator-controlled"):
            planner.build_plan(current, "reveal", envelope(), "0xaabb", 300)

    def test_commit_fails_closed_when_remaining_budget_is_insufficient(self) -> None:
        with self.assertRaisesRegex(SystemExit, "bounded budget"):
            planner.build_plan(report(wallet_balance=99_999), "commit", envelope(), None, 300)

    def test_relay_revalidation_preserves_the_original_deadline(self) -> None:
        current = report()
        plan = planner.build_plan(current, "commit", envelope(), None, 300)
        current["safe_block"]["timestamp"] += 10
        regenerated = planner.build_plan(
            current,
            "commit",
            envelope(),
            None,
            300,
            exact_deadline=plan["deadline"],
        )
        self.assertEqual(regenerated["deadline"], plan["deadline"])
        current["safe_block"]["timestamp"] = plan["deadline"]
        with self.assertRaisesRegex(SystemExit, "expires too soon"):
            planner.build_plan(
                current,
                "commit",
                envelope(),
                None,
                300,
                exact_deadline=plan["deadline"],
            )


if __name__ == "__main__":
    unittest.main()
