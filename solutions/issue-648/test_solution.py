"""
Unit Test Suite for Issue #648 Evidence Validator Helper
Tests valid conformance and rigorous negative assertions across all 10 maintainer review controls.
"""

import math
import unittest
from solution import SolutionHandler

class TestMCPChildBountyValidation(unittest.TestCase):
    def setUp(self):
        self.handler = SolutionHandler()
        self.valid_evidence = {
            "parent_solver": "0x1111111111111111111111111111111111111111",
            "child_solver": "0x2222222222222222222222222222222222222222",
            "parent_reward_usdc": 2.00,
            "child_target_usdc": 1.00,
            "verifier_framework": "sandboxed_regression_v1",
            "quorum_threshold": 2,
            "verifier_router": "0x380c1af742593dd88b6f20387e9ee693a0536731",
            "policy_hash": "0x8d8e5cca196fcc8ba123d6ffdbb4659c014d82d27cfe1f587f9d031059e23e58",
            "child_profile_approved": True,
            "settlement_receipt": {
                "tx_hash": "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                "event": "BountySettled",
                "settled": True,
                "recipient": "0x2222222222222222222222222222222222222222",
                "child_contract": "0x4444444444444444444444444444444444444444",
                "amount_usdc": 1.00
            }
        }

    def test_full_valid_evidence_conformance(self):
        """Valid evidence matches all 4 controls deterministically."""
        result = self.handler.execute(self.valid_evidence)
        self.assertEqual(result["status"], "SHAPE_AND_POLICY_CONFORMANT")
        self.assertTrue(result["all_controls_passed"])
        self.assertEqual(result["controls_passed"], 4)
        self.assertEqual(result["controls_failed"], 0)

    # --- CONTROL 1: PARTICIPANT INDEPENDENCE & ADDRESS BOUNDS ---
    def test_reject_identical_participants(self):
        """Parent and child solver cannot be identical addresses."""
        ev = dict(self.valid_evidence, child_solver=self.valid_evidence["parent_solver"])
        res = self.handler.execute(ev)
        self.assertFalse(res["all_controls_passed"])
        c1 = next(c for c in res["controls"] if c["control_id"] == "CONTROL_01_PARTICIPANT_INDEPENDENCE")
        self.assertFalse(c1["passed"])
        self.assertIn("distinct participants", c1["reason"])

    def test_reject_all_zero_solver_address(self):
        """All-zero Ethereum addresses are rejected."""
        ev = dict(self.valid_evidence, parent_solver="0x0000000000000000000000000000000000000000")
        res = self.handler.execute(ev)
        self.assertFalse(res["all_controls_passed"])
        c1 = next(c for c in res["controls"] if c["control_id"] == "CONTROL_01_PARTICIPANT_INDEPENDENCE")
        self.assertFalse(c1["passed"])
        self.assertIn("cannot be all-zero", c1["reason"])

    # --- CONTROL 2: STRICT NUMERIC & BOUNDED USDC AMOUNTS ---
    def test_reject_string_and_boolean_funding_amounts(self):
        """Boolean values (True/False) or raw strings as funding amounts are rejected."""
        ev1 = dict(self.valid_evidence, child_target_usdc=True)
        res1 = self.handler.execute(ev1)
        self.assertFalse(res1["all_controls_passed"])

        ev2 = dict(self.valid_evidence, parent_reward_usdc="2.00")
        res2 = self.handler.execute(ev2)
        self.assertFalse(res2["all_controls_passed"])

    def test_reject_nan_and_infinite_funding_amounts(self):
        """NaN and infinite float values bypass comparisons if not sanitized; assert rejected."""
        ev_nan = dict(self.valid_evidence, child_target_usdc=float("nan"))
        res_nan = self.handler.execute(ev_nan)
        self.assertFalse(res_nan["all_controls_passed"])
        c2_nan = next(c for c in res_nan["controls"] if c["control_id"] == "CONTROL_02_MARGIN_AND_FUNDING")
        self.assertIn("cannot be NaN", c2_nan["reason"])

        ev_inf = dict(self.valid_evidence, child_target_usdc=float("inf"))
        res_inf = self.handler.execute(ev_inf)
        self.assertFalse(res_inf["all_controls_passed"])

    def test_reject_insufficient_margin(self):
        """Retained margin under 1.00 USDC is rejected."""
        ev = dict(self.valid_evidence, parent_reward_usdc=1.50, child_target_usdc=1.00)
        res = self.handler.execute(ev)
        self.assertFalse(res["all_controls_passed"])
        c2 = next(c for c in res["controls"] if c["control_id"] == "CONTROL_02_MARGIN_AND_FUNDING")
        self.assertIn("below required minimum", c2["reason"])

    # --- CONTROL 3: STRICT INTEGER QUORUM & POLICY HASH ---
    def test_reject_fractional_quorum(self):
        """Fractional quorum values like 2.5 are rejected."""
        ev = dict(self.valid_evidence, quorum_threshold=2.5)
        res = self.handler.execute(ev)
        self.assertFalse(res["all_controls_passed"])
        c3 = next(c for c in res["controls"] if c["control_id"] == "CONTROL_03_VERIFIER_QUORUM")
        self.assertIn("strict integer", c3["reason"])

    def test_reject_malformed_quorum_without_crash(self):
        """Strings 'invalid' or null/None must fail gracefully without raising unhandled exceptions."""
        ev_str = dict(self.valid_evidence, quorum_threshold="invalid")
        res_str = self.handler.execute(ev_str)
        self.assertFalse(res_str["all_controls_passed"])

        ev_null = dict(self.valid_evidence, quorum_threshold=None)
        res_null = self.handler.execute(ev_null)
        self.assertFalse(res_null["all_controls_passed"])

    def test_reject_sub_threshold_quorum(self):
        """Quorum threshold of 1 is below required minimum of 2."""
        ev = dict(self.valid_evidence, quorum_threshold=1)
        res = self.handler.execute(ev)
        self.assertFalse(res["all_controls_passed"])

    def test_reject_policy_hash_mismatch(self):
        """Policy hash must match the immutable canonical policy hash."""
        ev = dict(self.valid_evidence, policy_hash="0x0000000000000000000000000000000000000000000000000000000000000000")
        res = self.handler.execute(ev)
        self.assertFalse(res["all_controls_passed"])
        c3 = next(c for c in res["controls"] if c["control_id"] == "CONTROL_03_VERIFIER_QUORUM")
        self.assertIn("policy hash mismatch", c3["reason"])

    # --- CONTROL 4: STRICT BOOLEANS, NON-ZERO HASH, & SETTLEMENT ---
    def test_reject_string_booleans(self):
        """String 'false' or 'true' cannot bypass boolean checks."""
        ev_app = dict(self.valid_evidence, child_profile_approved="false")
        res_app = self.handler.execute(ev_app)
        self.assertFalse(res_app["all_controls_passed"])

        sr = dict(self.valid_evidence["settlement_receipt"], settled="false")
        ev_set = dict(self.valid_evidence, settlement_receipt=sr)
        res_set = self.handler.execute(ev_set)
        self.assertFalse(res_set["all_controls_passed"])

    def test_reject_all_zero_transaction_hash(self):
        """All-zero 64-char transaction hash is rejected as mock placeholder."""
        zero_receipt = dict(
            self.valid_evidence["settlement_receipt"],
            tx_hash="0x0000000000000000000000000000000000000000000000000000000000000000"
        )
        ev = dict(self.valid_evidence, settlement_receipt=zero_receipt)
        res = self.handler.execute(ev)
        self.assertFalse(res["all_controls_passed"])
        c4 = next(c for c in res["controls"] if c["control_id"] == "CONTROL_04_LANE_READINESS_AND_SETTLEMENT")
        self.assertIn("cannot be all-zero", c4["reason"])

    def test_reject_missing_settlement_receipt(self):
        """Absence of settlement receipt fails control 4."""
        ev = dict(self.valid_evidence, settlement_receipt={})
        res = self.handler.execute(ev)
        self.assertFalse(res["all_controls_passed"])

    # --- PAYLOAD ROBUSTNESS ---
    def test_empty_and_non_dict_payload_fails_gracefully(self):
        """Empty dict, None, or invalid payload structure fail safely without crashing."""
        res_empty = self.handler.execute({})
        self.assertEqual(res_empty["status"], "EVIDENCE_VALIDATION_FAILED")
        self.assertEqual(res_empty["controls_failed"], 4)

        res_none = self.handler.execute(None)
        self.assertEqual(res_none["status"], "EVIDENCE_VALIDATION_FAILED")

if __name__ == "__main__":
    unittest.main()
