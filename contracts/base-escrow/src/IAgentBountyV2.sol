// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";

/// @notice Minimal view of `ParticipantEligibilityRegistry` used to gate claims.
interface IParticipantEligibilityRegistryV1 {
    function eligibleAt(address wallet, uint64 cutoff)
        external
        view
        returns (bytes32 participantId, bytes32 sourceHash, bool eligible);
}

/// @notice Autonomous-v2 adds an explicit platform fee and an optional claim-eligibility
/// gate to the v1 bounty surface. The fee rate, amount, and recipient are fixed at creation
/// and included in the funding target. Settlement pays the fee in the same transaction as the
/// solver; only if that transfer fails does the fee stay in the bounty for a later
/// permissionless forward to the same recipient, so the recipient can never block payment.
interface IAgentBountyV2 is IAgentBountyV1 {
    function platformFeeBps() external view returns (uint16);
    function platformFee() external view returns (uint256);
    function platformFeeRecipient() external view returns (address);
    function platformFeeAccrued() external view returns (uint256);
    function withdrawPlatformFee() external returns (uint256 amount);
    function claimEligibilityRegistry() external view returns (address);
    function claimEligibilitySource() external view returns (bytes32);
}
