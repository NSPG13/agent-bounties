"""
Non-Authoritative Evidence Validator Helper for NSPG13/agent-bounties#648
Title: [META] MCP interoperability child-bounty task
Focus: Strict offline payload-shape, strict typing, and policy conformance validator helper.

NOTE: This is a client-side helper for syntactic and local policy sanity checks.
It does NOT constitute canonical on-chain verification or authorize bounty settlement.
"""

import re
import math
import logging
from typing import Dict, Any, List, Optional

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger("issue_648_validator")

ETH_ADDRESS_REGEX = re.compile(r"^0x[a-fA-F0-9]{40}$")
HEX32_REGEX = re.compile(r"^0x[a-fA-F0-9]{64}$")
ZERO_ETH_ADDRESS = "0x0000000000000000000000000000000000000000"
ZERO_TX_HASH = "0x0000000000000000000000000000000000000000000000000000000000000000"

class SolutionHandler:
    """
    Offline payload-shape and policy constraint validator for MCP child-bounties.
    Enforces strict boolean/integer types, finite bounded USDC units, 
    and checks immutable policy hash and settlement receipt structures.
    """

    EXPECTED_ROUTER = "0x380c1af742593dd88b6f20387e9ee693a0536731"
    EXPECTED_POLICY_HASH = "0x8d8e5cca196fcc8ba123d6ffdbb4659c014d82d27cfe1f587f9d031059e23e58"
    MIN_CHILD_TARGET_USDC = 1.00
    MIN_PARENT_MARGIN_USDC = 1.00
    MAX_BOUNDED_USDC = 1_000_000.00
    MIN_QUORUM_THRESHOLD = 2

    def __init__(self, config: Optional[Dict[str, Any]] = None):
        self.config = config or {}

    def evaluate_control_01_participant_independence(self, evidence: Dict[str, Any]) -> Dict[str, Any]:
        """Control 1: Verify independent child solver and parent solver addresses with non-zero check."""
        if not isinstance(evidence, dict):
            return {
                "control_id": "CONTROL_01_PARTICIPANT_INDEPENDENCE",
                "passed": False,
                "reason": "Evidence payload must be a dictionary"
            }

        parent_solver = evidence.get("parent_solver")
        child_solver = evidence.get("child_solver")

        if not isinstance(parent_solver, str) or not ETH_ADDRESS_REGEX.match(parent_solver.strip()):
            return {
                "control_id": "CONTROL_01_PARTICIPANT_INDEPENDENCE",
                "passed": False,
                "reason": f"Invalid parent solver address format: '{parent_solver}'"
            }

        if not isinstance(child_solver, str) or not ETH_ADDRESS_REGEX.match(child_solver.strip()):
            return {
                "control_id": "CONTROL_01_PARTICIPANT_INDEPENDENCE",
                "passed": False,
                "reason": f"Invalid child solver address format: '{child_solver}'"
            }

        parent_norm = parent_solver.strip().lower()
        child_norm = child_solver.strip().lower()

        if parent_norm == ZERO_ETH_ADDRESS.lower() or child_norm == ZERO_ETH_ADDRESS.lower():
            return {
                "control_id": "CONTROL_01_PARTICIPANT_INDEPENDENCE",
                "passed": False,
                "reason": "Solver address cannot be all-zero address"
            }

        if parent_norm == child_norm:
            return {
                "control_id": "CONTROL_01_PARTICIPANT_INDEPENDENCE",
                "passed": False,
                "reason": "Parent solver and child solver must be distinct participants"
            }

        return {
            "control_id": "CONTROL_01_PARTICIPANT_INDEPENDENCE",
            "passed": True,
            "reason": "Parent and child solvers are distinct registered participants"
        }

    def evaluate_control_02_margin_and_funding(self, evidence: Dict[str, Any]) -> Dict[str, Any]:
        """Control 2: Verify strictly typed, finite, bounded USDC amounts and margin."""
        if not isinstance(evidence, dict):
            return {
                "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
                "passed": False,
                "reason": "Evidence payload must be a dictionary"
            }

        parent_val = evidence.get("parent_reward_usdc")
        child_val = evidence.get("child_target_usdc")

        # Reject booleans (in Python, bool is a subclass of int)
        if isinstance(parent_val, bool) or isinstance(child_val, bool):
            return {
                "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
                "passed": False,
                "reason": "Boolean amounts are strictly invalid for funding/reward parameters"
            }

        # Validate numeric type
        if not isinstance(parent_val, (int, float)) or not isinstance(child_val, (int, float)):
            return {
                "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
                "passed": False,
                "reason": "Funding or reward amounts must be strict numeric types (float or int)"
            }

        # Check for NaN / Infinity
        if not math.isfinite(parent_val) or not math.isfinite(child_val):
            return {
                "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
                "passed": False,
                "reason": "Funding amounts cannot be NaN, infinite, or non-finite"
            }

        parent_reward = float(parent_val)
        child_funding = float(child_val)

        if child_funding < self.MIN_CHILD_TARGET_USDC:
            return {
                "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
                "passed": False,
                "reason": f"Child funding target ({child_funding:.2f} USDC) is below required minimum ({self.MIN_CHILD_TARGET_USDC:.2f} USDC)"
            }

        if child_funding > self.MAX_BOUNDED_USDC or parent_reward > self.MAX_BOUNDED_USDC:
            return {
                "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
                "passed": False,
                "reason": f"Amounts exceed maximum bounded limit ({self.MAX_BOUNDED_USDC:.2f} USDC)"
            }

        retained_margin = parent_reward - child_funding
        if retained_margin < self.MIN_PARENT_MARGIN_USDC:
            return {
                "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
                "passed": False,
                "reason": f"Parent retained margin ({retained_margin:.2f} USDC) is below required minimum ({self.MIN_PARENT_MARGIN_USDC:.2f} USDC)"
            }

        return {
            "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
            "passed": True,
            "reason": f"Funding parameters verified: {child_funding:.2f} USDC child target, {retained_margin:.2f} USDC parent margin"
        }

    def evaluate_control_03_verifier_quorum(self, evidence: Dict[str, Any]) -> Dict[str, Any]:
        """Control 3: Verify strict integer threshold-two quorum, router address, and immutable policy hash."""
        if not isinstance(evidence, dict):
            return {
                "control_id": "CONTROL_03_VERIFIER_QUORUM",
                "passed": False,
                "reason": "Evidence payload must be a dictionary"
            }

        verifier_framework = evidence.get("verifier_framework")
        raw_quorum = evidence.get("quorum_threshold")
        verifier_router = evidence.get("verifier_router")
        policy_hash = evidence.get("policy_hash")

        # Safe type checks without crashing
        if not isinstance(verifier_framework, str) or verifier_framework.strip() != "sandboxed_regression_v1":
            return {
                "control_id": "CONTROL_03_VERIFIER_QUORUM",
                "passed": False,
                "reason": f"Verifier framework must be strict string 'sandboxed_regression_v1', got '{verifier_framework}'"
            }

        # Strict integer check: reject bool, float (e.g. 2.5), str (e.g. "invalid"), None
        if isinstance(raw_quorum, bool) or not isinstance(raw_quorum, int):
            return {
                "control_id": "CONTROL_03_VERIFIER_QUORUM",
                "passed": False,
                "reason": f"Quorum threshold must be a strict integer (rejecting floats, strings, booleans, and null), got '{raw_quorum}'"
            }

        if raw_quorum < self.MIN_QUORUM_THRESHOLD:
            return {
                "control_id": "CONTROL_03_VERIFIER_QUORUM",
                "passed": False,
                "reason": f"Quorum threshold ({raw_quorum}) must be at least {self.MIN_QUORUM_THRESHOLD}"
            }

        if not isinstance(verifier_router, str) or verifier_router.strip().lower() != self.EXPECTED_ROUTER.lower():
            return {
                "control_id": "CONTROL_03_VERIFIER_QUORUM",
                "passed": False,
                "reason": f"Verifier router mismatch. Expected '{self.EXPECTED_ROUTER}', got '{verifier_router}'"
            }

        # Validate immutable policy hash constant
        if not isinstance(policy_hash, str) or policy_hash.strip().lower() != self.EXPECTED_POLICY_HASH.lower():
            return {
                "control_id": "CONTROL_03_VERIFIER_QUORUM",
                "passed": False,
                "reason": f"Immutable policy hash mismatch. Expected '{self.EXPECTED_POLICY_HASH}', got '{policy_hash}'"
            }

        return {
            "control_id": "CONTROL_03_VERIFIER_QUORUM",
            "passed": True,
            "reason": "Verifier framework, strict integer quorum threshold, router, and policy hash confirmed"
        }

    def evaluate_control_04_lane_readiness_and_settlement(self, evidence: Dict[str, Any]) -> Dict[str, Any]:
        """Control 4: Strict boolean validation for profile approval, receipt structure, non-zero hash, and settlement."""
        if not isinstance(evidence, dict):
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": "Evidence payload must be a dictionary"
            }

        child_profile_approved = evidence.get("child_profile_approved")
        settlement_receipt = evidence.get("settlement_receipt")

        # Strict boolean check (reject strings "false", "true", or truthy numbers)
        if type(child_profile_approved) is not bool or not child_profile_approved:
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": "Child profile unapproved: lane has no independently reviewed approved child profile (must be strict boolean True)"
            }

        if not settlement_receipt or not isinstance(settlement_receipt, dict):
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": "BountySettled confirmation missing: settlement receipt is absent, non-dict, or empty"
            }

        tx_hash = settlement_receipt.get("tx_hash")
        event_name = settlement_receipt.get("event")
        settled = settlement_receipt.get("settled")
        recipient = settlement_receipt.get("recipient")
        child_contract = settlement_receipt.get("child_contract")
        amount_usdc = settlement_receipt.get("amount_usdc")

        # Reject all-zero transaction hash
        if not isinstance(tx_hash, str) or not HEX32_REGEX.match(tx_hash.strip()):
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": f"Invalid settlement transaction hash format: '{tx_hash}'"
            }

        if tx_hash.strip().lower() == ZERO_TX_HASH.lower():
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": "Settlement transaction hash cannot be all-zero mock placeholder"
            }

        # Strict boolean check for settled
        if type(settled) is not bool or not settled:
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": f"Canonical BountySettled confirmation missing: 'settled' must be strict boolean True, got '{settled}'"
            }

        if event_name != "BountySettled":
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": f"Settlement event mismatch: expected 'BountySettled', got '{event_name}'"
            }

        # Validate recipient and child contract addresses
        if not isinstance(recipient, str) or not ETH_ADDRESS_REGEX.match(recipient.strip()) or recipient.strip().lower() == ZERO_ETH_ADDRESS.lower():
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": f"Settlement receipt requires valid non-zero recipient EVM address: '{recipient}'"
            }

        if not isinstance(child_contract, str) or not ETH_ADDRESS_REGEX.match(child_contract.strip()) or child_contract.strip().lower() == ZERO_ETH_ADDRESS.lower():
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": f"Settlement receipt requires valid non-zero child contract EVM address: '{child_contract}'"
            }

        # Validate settled amount
        if type(amount_usdc) is bool or not isinstance(amount_usdc, (int, float)) or not math.isfinite(amount_usdc) or amount_usdc < self.MIN_CHILD_TARGET_USDC:
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": f"Settlement receipt requires finite numeric amount_usdc >= {self.MIN_CHILD_TARGET_USDC:.2f}, got '{amount_usdc}'"
            }

        return {
            "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
            "passed": True,
            "reason": "Child profile approved and canonical BountySettled receipt fields validated"
        }

    def execute(self, payload: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        """
        Executes offline multi-control evidence shape and policy validation.
        Safe against non-dict payloads, null values, and type anomalies.
        """
        if payload is None:
            evidence = self.config.get("evidence", {})
        elif isinstance(payload, dict):
            evidence = payload
        else:
            evidence = {}
        
        c1 = self.evaluate_control_01_participant_independence(evidence)
        c2 = self.evaluate_control_02_margin_and_funding(evidence)
        c3 = self.evaluate_control_03_verifier_quorum(evidence)
        c4 = self.evaluate_control_04_lane_readiness_and_settlement(evidence)

        controls = [c1, c2, c3, c4]
        all_passed = all(c["passed"] for c in controls)
        passed_count = sum(1 for c in controls if c["passed"])
        failed_count = len(controls) - passed_count

        status = "SHAPE_AND_POLICY_CONFORMANT" if all_passed else "EVIDENCE_VALIDATION_FAILED"

        return {
            "status": status,
            "validator_type": "OFFLINE_SHAPE_AND_POLICY_HELPER",
            "issue": 648,
            "all_controls_passed": all_passed,
            "controls_passed": passed_count,
            "controls_failed": failed_count,
            "controls": controls
        }

if __name__ == "__main__":
    handler = SolutionHandler()
    result = handler.execute()
    print(result)
