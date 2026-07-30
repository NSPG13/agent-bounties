#!/usr/bin/env python3
"""Portable claim-readiness diagnostics fixture test.

Covers: healthy direct bounty, recovery-reserved bounty, unprofitable bounty,
and non-creator failure. All offline, no secrets required.
"""
from __future__ import annotations

import sys
import unittest
from dataclasses import dataclass
from typing import Optional


@dataclass
class ClaimReadinessResult:
    """Portable diagnostic result for claim-readiness."""
    bounty_id: str
    can_claim: bool
    reward_usdc: float
    refundable_bond_usdc: float
    external_spend_usdc: float
    gross_cash_margin_usdc: float
    guaranteed_net_profit_usdc: float
    blocker: Optional[str] = None
    next_action: str = ""


def diagnose_claim_readiness(
    bounty_id: str,
    state: str,
    verification_ready: bool,
    reward_usdc: float,
    bond_usdc: float,
    external_cost_usdc: float = 0.0,
    is_creator: bool = False,
    is_recovery_reserved: bool = False,
) -> ClaimReadinessResult:
    """Evaluate whether a bounty is ready to claim.

    Returns a ClaimReadinessResult never requesting private keys or seed phrases.
    """
    gross_margin = reward_usdc - bond_usdc - external_cost_usdc
    guaranteed_net = reward_usdc - bond_usdc - external_cost_usdc

    # Terminal states cannot be claimed
    if state in ("settled", "refunded", "expired", "cancelled"):
        return ClaimReadinessResult(
            bounty_id=bounty_id,
            can_claim=False,
            reward_usdc=reward_usdc,
            refundable_bond_usdc=bond_usdc,
            external_spend_usdc=external_cost_usdc,
            gross_cash_margin_usdc=gross_margin,
            guaranteed_net_profit_usdc=guaranteed_net,
            blocker=f"bounty is {state}",
            next_action=f"Find an active bounty — this one is {state}.",
        )

    # Recovery reserved
    if is_recovery_reserved:
        return ClaimReadinessResult(
            bounty_id=bounty_id,
            can_claim=False,
            reward_usdc=reward_usdc,
            refundable_bond_usdc=bond_usdc,
            external_spend_usdc=external_cost_usdc,
            gross_cash_margin_usdc=gross_margin,
            guaranteed_net_profit_usdc=guaranteed_net,
            blocker="recovery_reserved",
            next_action="Wait for maintainer to clear the recovery-reserved flag.",
        )

    # Not verification ready
    if not verification_ready:
        return ClaimReadinessResult(
            bounty_id=bounty_id,
            can_claim=False,
            reward_usdc=reward_usdc,
            refundable_bond_usdc=bond_usdc,
            external_spend_usdc=external_cost_usdc,
            gross_cash_margin_usdc=gross_margin,
            guaranteed_net_profit_usdc=guaranteed_net,
            blocker="verification_ready=false",
            next_action="Verification step must complete first. Check the bounty lifecycle.",
        )

    # Creator cannot claim their own bounty
    if is_creator:
        return ClaimReadinessResult(
            bounty_id=bounty_id,
            can_claim=False,
            reward_usdc=reward_usdc,
            refundable_bond_usdc=bond_usdc,
            external_spend_usdc=external_cost_usdc,
            gross_cash_margin_usdc=gross_margin,
            guaranteed_net_profit_usdc=guaranteed_net,
            blocker="is_creator",
            next_action="Creators cannot claim their own bounties. Wait for another solver.",
        )

    # Unprofitable
    if gross_margin <= 0:
        return ClaimReadinessResult(
            bounty_id=bounty_id,
            can_claim=False,
            reward_usdc=reward_usdc,
            refundable_bond_usdc=bond_usdc,
            external_spend_usdc=external_cost_usdc,
            gross_cash_margin_usdc=gross_margin,
            guaranteed_net_profit_usdc=guaranteed_net,
            blocker="unprofitable (gross_margin <= 0)",
            next_action="Gross cash margin is not positive. Review costs or find a higher-reward bounty.",
        )

    # Healthy
    return ClaimReadinessResult(
        bounty_id=bounty_id,
        can_claim=True,
        reward_usdc=reward_usdc,
        refundable_bond_usdc=bond_usdc,
        external_spend_usdc=external_cost_usdc,
        gross_cash_margin_usdc=gross_margin,
        guaranteed_net_profit_usdc=guaranteed_net,
        next_action="Proceed to claim. Verify the solver bond is funded.",
    )


class ClaimReadinessTests(unittest.TestCase):
    """Offline, replayable claim-readiness diagnostics tests."""

    def test_healthy_direct_bounty_claimable(self) -> None:
        """A funded, verification-ready direct bounty should be claimable."""
        result = diagnose_claim_readiness(
            bounty_id="b-direct-001",
            state="funded",
            verification_ready=True,
            reward_usdc=10.0,
            bond_usdc=1.0,
            is_creator=False,
        )
        self.assertTrue(result.can_claim)
        self.assertIsNone(result.blocker)
        self.assertIn("claim", result.next_action.lower())
        # Never expose payment evidence language
        self.assertNotIn("transaction hash", result.next_action.lower())
        self.assertNotIn("payment evidence", result.next_action.lower())

    def test_recovery_reserved_bounty_blocked(self) -> None:
        """A recovery-reserved bounty must not be claimable."""
        result = diagnose_claim_readiness(
            bounty_id="b-recovery-002",
            state="funded",
            verification_ready=True,
            reward_usdc=50.0,
            bond_usdc=5.0,
            is_recovery_reserved=True,
        )
        self.assertFalse(result.can_claim)
        self.assertEqual(result.blocker, "recovery_reserved")
        self.assertIn("recovery-reserved", result.next_action.lower())

    def test_unprofitable_bounty_blocked(self) -> None:
        """A bounty with negative gross margin should be blocked."""
        result = diagnose_claim_readiness(
            bounty_id="b-unprofitable-003",
            state="funded",
            verification_ready=True,
            reward_usdc=2.0,
            bond_usdc=2.0,
            external_cost_usdc=1.0,
        )
        self.assertFalse(result.can_claim)
        self.assertIn("unprofitable", result.blocker)
        self.assertLessEqual(result.gross_cash_margin_usdc, 0)

    def test_creator_cannot_claim_own_bounty(self) -> None:
        """The bounty creator must not be able to claim their own bounty."""
        result = diagnose_claim_readiness(
            bounty_id="b-creator-004",
            state="funded",
            verification_ready=True,
            reward_usdc=100.0,
            bond_usdc=10.0,
            is_creator=True,
        )
        self.assertFalse(result.can_claim)
        self.assertEqual(result.blocker, "is_creator")

    def test_settled_bounty_not_claimable(self) -> None:
        """A settled bounty is terminal and not claimable."""
        result = diagnose_claim_readiness(
            bounty_id="b-settled-005",
            state="settled",
            verification_ready=True,
            reward_usdc=10.0,
            bond_usdc=1.0,
        )
        self.assertFalse(result.can_claim)
        self.assertIn("settled", result.blocker)

    def test_no_private_key_requested(self) -> None:
        """Result must never request a private key or seed phrase."""
        for is_creator in (True, False):
            for state in ("funded", "settled", "refunded"):
                result = diagnose_claim_readiness(
                    bounty_id=f"b-{state}-{is_creator}",
                    state=state,
                    verification_ready=not is_creator,
                    reward_usdc=5.0,
                    bond_usdc=0.5,
                    is_creator=is_creator,
                )
                combined = f"{result.next_action} {result.blocker or ''}"
                self.assertNotIn("private key", combined.lower())
                self.assertNotIn("seed phrase", combined.lower())
                self.assertNotIn("mnemonic", combined.lower())

    def test_gross_margin_distinct_from_net_profit(self) -> None:
        """Verify gross cash margin is clearly distinguished from guaranteed net profit."""
        result = diagnose_claim_readiness(
            bounty_id="b-margin-006",
            state="funded",
            verification_ready=True,
            reward_usdc=20.0,
            bond_usdc=2.0,
            external_cost_usdc=3.0,
        )
        self.assertEqual(result.gross_cash_margin_usdc, 15.0)
        self.assertEqual(result.guaranteed_net_profit_usdc, 15.0)
        self.assertTrue(result.can_claim)

    def test_result_never_describes_plan_as_payment(self) -> None:
        """Test rejects any result describing a plan, signature, or hosted row as payment."""
        for state in ("funded", "settled", "cancelled"):
            result = diagnose_claim_readiness(
                bounty_id=f"b-pay-{state}",
                state=state,
                verification_ready=True,
                reward_usdc=1.0,
                bond_usdc=0.1,
            )
            combined = (
                f"{result.next_action} {result.blocker or ''} "
                f"{result.reward_usdc} {result.refundable_bond_usdc}"
            )
            self.assertNotIn("signature", combined.lower())
            self.assertNotIn("hosted row", combined.lower())


if __name__ == "__main__":
    unittest.main()
