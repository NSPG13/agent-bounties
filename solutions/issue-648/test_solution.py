"""
Unit Test Suite for Issue #648 Verification Engine
Tests both passing and failing controls across all 4 canonical criteria.
"""

import unittest
from solution import SolutionHandler

class TestMCPChildBountyVerification(unittest.TestCase):
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
            "child_profile_approved": True,
            "settlement_receipt": {
                "tx_hash": "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                "event": "BountySettled",
                "settled": True
            }
        }

    def test_full_evidence_conformance_passes_all_controls(self):
        """Asserts all 4 controls pass and status is VERIFIED_CONFORMANT when valid evidence is provided."""
        result = self.handler.execute(self.valid_evidence)
        self.assertEqual(result["status"], "VERIFIED_CONFORMANT")
        self.assertTrue(result["all_controls_passed"])
        self.assertEqual(result["controls_passed"], 4)
        self.assertEqual(result["controls_failed"], 0)

    def test_failing_control_identical_participants(self):
        """Asserts Control 1 fails when parent and child solvers are identical."""
        evidence = dict(self.valid_evidence)
        evidence["child_solver"] = evidence["parent_solver"]
        result = self.handler.execute(evidence)

        self.assertEqual(result["status"], "EVIDENCE_VALIDATION_FAILED")
        self.assertFalse(result["all_controls_passed"])
        c1 = next(c for c in result["controls"] if c["control_id"] == "CONTROL_01_PARTICIPANT_INDEPENDENCE")
        self.assertFalse(c1["passed"])
        self.assertIn("distinct participants", c1["reason"])

    def test_failing_control_invalid_address_format(self):
        """Asserts Control 1 fails on malformed address strings."""
        evidence = dict(self.valid_evidence)
        evidence["parent_solver"] = "not-an-eth-address"
        result = self.handler.execute(evidence)

        self.assertFalse(result["all_controls_passed"])
        c1 = next(c for c in result["controls"] if c["control_id"] == "CONTROL_01_PARTICIPANT_INDEPENDENCE")
        self.assertFalse(c1["passed"])
        self.assertIn("Invalid parent solver", c1["reason"])

    def test_failing_control_insufficient_margin(self):
        """Asserts Control 2 fails when retained gross margin is under 1.00 USDC."""
        evidence = dict(self.valid_evidence)
        evidence["parent_reward_usdc"] = 1.50
        evidence["child_target_usdc"] = 1.00  # Retained margin = 0.50 USDC (< 1.00)
        result = self.handler.execute(evidence)

        self.assertFalse(result["all_controls_passed"])
        c2 = next(c for c in result["controls"] if c["control_id"] == "CONTROL_02_MARGIN_AND_FUNDING")
        self.assertFalse(c2["passed"])
        self.assertIn("Parent retained margin (0.50 USDC) is below required minimum", c2["reason"])

    def test_failing_control_insufficient_child_funding(self):
        """Asserts Control 2 fails when child target is below 1.00 USDC."""
        evidence = dict(self.valid_evidence)
        evidence["child_target_usdc"] = 0.50
        result = self.handler.execute(evidence)

        self.assertFalse(result["all_controls_passed"])
        c2 = next(c for c in result["controls"] if c["control_id"] == "CONTROL_02_MARGIN_AND_FUNDING")
        self.assertFalse(c2["passed"])
        self.assertIn("Child funding target (0.50 USDC) is below required minimum", c2["reason"])

    def test_failing_control_sub_quorum_threshold(self):
        """Asserts Control 3 fails when quorum threshold is below 2."""
        evidence = dict(self.valid_evidence)
        evidence["quorum_threshold"] = 1
        result = self.handler.execute(evidence)

        self.assertFalse(result["all_controls_passed"])
        c3 = next(c for c in result["controls"] if c["control_id"] == "CONTROL_03_VERIFIER_QUORUM")
        self.assertFalse(c3["passed"])
        self.assertIn("must be at least 2", c3["reason"])

    def test_failing_control_unapproved_child_profile(self):
        """Asserts Control 4 fails when child profile is unapproved (current canonical blocker)."""
        evidence = dict(self.valid_evidence)
        evidence["child_profile_approved"] = False
        result = self.handler.execute(evidence)

        self.assertFalse(result["all_controls_passed"])
        c4 = next(c for c in result["controls"] if c["control_id"] == "CONTROL_04_LANE_READINESS_AND_SETTLEMENT")
        self.assertFalse(c4["passed"])
        self.assertIn("Child profile unapproved", c4["reason"])

    def test_failing_control_missing_settlement_receipt(self):
        """Asserts Control 4 fails when canonical BountySettled receipt is absent."""
        evidence = dict(self.valid_evidence)
        evidence["settlement_receipt"] = {}
        result = self.handler.execute(evidence)

        self.assertFalse(result["all_controls_passed"])
        c4 = next(c for c in result["controls"] if c["control_id"] == "CONTROL_04_LANE_READINESS_AND_SETTLEMENT")
        self.assertFalse(c4["passed"])
        self.assertIn("BountySettled confirmation missing", c4["reason"])

    def test_empty_payload_fails_all_controls(self):
        """Asserts empty payload produces 4 failing controls and never grants blanket success."""
        result = self.handler.execute({})
        self.assertEqual(result["status"], "EVIDENCE_VALIDATION_FAILED")
        self.assertFalse(result["all_controls_passed"])
        self.assertEqual(result["controls_passed"], 0)
        self.assertEqual(result["controls_failed"], 4)

if __name__ == "__main__":
    unittest.main()
