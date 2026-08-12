from __future__ import annotations

import sys
import unittest
from pathlib import Path

from web3 import Web3


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import run_open_competition_entrant_mainnet_canary as canary  # noqa: E402


class OpenCompetitionEntrantMainnetCanaryTests(unittest.TestCase):
    def test_creator_balance_reconciliation_preserves_surplus(self) -> None:
        after_setup, after_settlement = canary.expected_creator_balances(788_053)
        self.assertEqual(after_setup, 688_053)
        self.assertEqual(after_settlement, 698_053)

    def test_creator_balance_reconciliation_accepts_exact_budget(self) -> None:
        after_setup, after_settlement = canary.expected_creator_balances(100_000)
        self.assertEqual(after_setup, 0)
        self.assertEqual(after_settlement, 10_000)

    def test_creator_balance_reconciliation_rejects_underfunding(self) -> None:
        with self.assertRaisesRegex(SystemExit, "at least 0.10 USDC"):
            canary.expected_creator_balances(99_999)

    def test_commitment_binds_chain_bounty_solver_and_reveal_material(self) -> None:
        bounty = "0x1111111111111111111111111111111111111111"
        solver = "0x2222222222222222222222222222222222222222"
        submission = Web3.keccak(text="submission")
        evidence = Web3.keccak(text="evidence")
        salt = Web3.keccak(text="salt")
        first = canary.commitment_for(bounty, solver, submission, evidence, salt)
        second = canary.commitment_for(
            bounty, solver, submission, evidence, Web3.keccak(text="different-salt")
        )
        self.assertEqual(len(first), 32)
        self.assertNotEqual(first, second)

    def test_leading_zero_proof_matches_requested_difficulty(self) -> None:
        _, proof, work_hash = canary.mine_proof(
            Web3.keccak(text="bounty"),
            "0x2222222222222222222222222222222222222222",
            Web3.keccak(text="submission"),
            Web3.keccak(text="evidence"),
            Web3.keccak(text="policy"),
            difficulty_bits=8,
            cap=10_000,
        )
        self.assertEqual(len(proof), 32)
        self.assertEqual(int.from_bytes(work_hash, "big") >> 248, 0)


if __name__ == "__main__":
    unittest.main()
