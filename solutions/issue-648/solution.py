"""
Non-Authoritative Evidence Validator Helper for NSPG13/agent-bounties#648
Title: [META] MCP interoperability child-bounty task
Focus: Strict offline payload-shape, exact decimal/micro-USDC math, and policy constraint validator helper.

NOTE: This is a client-side helper for syntactic and local policy sanity checks.
It does NOT constitute canonical on-chain verification or authorize bounty settlement.
"""

import re
import math
import logging
from decimal import Decimal, InvalidOperation
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
    Enforces strict boolean/integer types, exact micro-USDC integer arithmetic,
    numeric overflow boundaries, and checks immutable policy hash and receipt structures.
    """

    EXPECTED_ROUTER = "0x380c1af742593dd88b6f20387e9ee693a0536731"
    EXPECTED_POLICY_HASH = "0x8d8e5cca196fcc8ba123d6ffdbb4659c014d82d27cfe1f587f9d031059e23e58"
    
    # Micro-USDC integer constants (1 USDC = 1,000,000 micro-USDC)
    MICRO_USDC_PER_USDC = 1_000_000
    MIN_CHILD_TARGET_MICRO = 1 * MICRO_USDC_PER_USDC
    MIN_PARENT_MARGIN_MICRO = 1 * MICRO_USDC_PER_USDC
    MAX_BOUNDED_MICRO = 1_000_000 * MICRO_USDC_PER_USDC  # 1M USDC cap
    
    MIN_QUORUM_THRESHOLD = 2

    def __init__(self, config: Optional[Dict[str, Any]] = None):
        self.config = config or {}

    def _parse_to_micro_usdc(self, val: Any, field_name: str) -> Dict[str, Any]:
        """
        Safely parses and validates a monetary field to integer micro-USDC.
        Guards against booleans, non-numerics, large integers (OverflowError),
        NaN, and infinities without crashing.
        """
        if isinstance(val, bool):
            return {"valid": False, "reason": f"Boolean value is strictly invalid for {field_name}"}

        if not isinstance(val, (int, float, Decimal)):
            return {"valid": False, "reason": f"{field_name} must be a strict numeric type, got '{type(val).__name__}'"}

        # Guard against huge integers (e.g. 10**400) before any float or math operation
        if isinstance(val, int):
            if abs(val) > 1_000_000:  # Exceeds max bounded USDC in base units
                return {"valid": False, "reason": f"{field_name} exceeds maximum bounded limit (1,000,000 USDC)"}

        # Guard against NaN and infinities for floats
        if isinstance(val, float):
            try:
                if not math.isfinite(val):
                    return {"valid": False, "reason": f"{field_name} cannot be NaN, infinite, or non-finite"}
            except OverflowError:
                return {"valid": False, "reason": f"{field_name} exceeds numeric bounds"}

        # Exact decimal conversion to integer micro-USDC to avoid float binary subtraction errors
        try:
            d_val = Decimal(str(val))
            micro_val = int(d_val * self.MICRO_USDC_PER_USDC)
        except (InvalidOperation, OverflowError, ValueError):
            return {"valid": False, "reason": f"{field_name} could not be parsed to exact decimal micro-USDC"}

        if micro_val < 0:
            return {"valid": False, "reason": f"{field_name} cannot be negative"}

        if micro_val > self.MAX_BOUNDED_MICRO:
            return {"valid": False, "reason": f"{field_name} exceeds maximum bounded limit (1,000,000 USDC)"}

        return {"valid": True, "micro_usdc": micro_val}

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
                "reason": "Parent solver and child solver must be distinct supplied addresses"
            }

        return {
            "control_id": "CONTROL_01_PARTICIPANT_INDEPENDENCE",
            "passed": True,
            "reason": "Parent and child solvers are distinct supplied addresses"
        }

    def evaluate_control_02_margin_and_funding(self, evidence: Dict[str, Any]) -> Dict[str, Any]:
        """Control 2: Exact micro-USDC integer arithmetic for funding target and retained gross margin."""
        if not isinstance(evidence, dict):
            return {
                "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
                "passed": False,
                "reason": "Evidence payload must be a dictionary"
            }

        parent_parsed = self._parse_to_micro_usdc(evidence.get("parent_reward_usdc"), "parent_reward_usdc")
        if not parent_parsed["valid"]:
            return {
                "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
                "passed": False,
                "reason": parent_parsed["reason"]
            }

        child_parsed = self._parse_to_micro_usdc(evidence.get("child_target_usdc"), "child_target_usdc")
        if not child_parsed["valid"]:
            return {
                "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
                "passed": False,
                "reason": child_parsed["reason"]
            }

        parent_micro = parent_parsed["micro_usdc"]
        child_micro = child_parsed["micro_usdc"]

        if child_micro < self.MIN_CHILD_TARGET_MICRO:
            return {
                "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
                "passed": False,
                "reason": f"Child funding target ({child_micro / self.MICRO_USDC_PER_USDC:.6f} USDC) is below required minimum (1.000000 USDC)"
            }

        retained_margin_micro = parent_micro - child_micro
        if retained_margin_micro < self.MIN_PARENT_MARGIN_MICRO:
            return {
                "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
                "passed": False,
                "reason": f"Parent retained margin ({retained_margin_micro / self.MICRO_USDC_PER_USDC:.6f} USDC) is below required minimum (1.000000 USDC)"
            }

        return {
            "control_id": "CONTROL_02_MARGIN_AND_FUNDING",
            "passed": True,
            "reason": f"Funding parameters verified in exact micro-USDC: {child_micro / self.MICRO_USDC_PER_USDC:.6f} USDC child target, {retained_margin_micro / self.MICRO_USDC_PER_USDC:.6f} USDC parent margin"
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

        if not isinstance(verifier_framework, str) or verifier_framework.strip() != "sandboxed_regression_v1":
            return {
                "control_id": "CONTROL_03_VERIFIER_QUORUM",
                "passed": False,
                "reason": f"Verifier framework must be strict string 'sandboxed_regression_v1', got '{verifier_framework}'"
            }

        # Strict integer check: reject bool, float, str, None, and large integers
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
        amount_raw = settlement_receipt.get("amount_usdc")

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

        if type(settled) is not bool or not settled:
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": f"BountySettled confirmation missing: 'settled' must be strict boolean True, got '{settled}'"
            }

        if event_name != "BountySettled":
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": f"Settlement event mismatch: expected 'BountySettled', got '{event_name}'"
            }

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

        # Safe parsing of receipt amount using micro-USDC parser
        parsed_amount = self._parse_to_micro_usdc(amount_raw, "amount_usdc")
        if not parsed_amount["valid"]:
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": parsed_amount["reason"]
            }

        if parsed_amount["micro_usdc"] < self.MIN_CHILD_TARGET_MICRO:
            return {
                "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
                "passed": False,
                "reason": f"Settlement receipt requires amount_usdc >= 1.000000 USDC, got '{amount_raw}'"
            }

        return {
            "control_id": "CONTROL_04_LANE_READINESS_AND_SETTLEMENT",
            "passed": True,
            "reason": "Child profile approved and settlement receipt shape accepted"
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
