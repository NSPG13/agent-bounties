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

/// @notice Circle USDC EIP-3009 `receiveWithAuthorization`. Unlike `transferWithAuthorization`,
/// only the payee (`to`) may execute it, so a published authorization cannot be sent to the token
/// directly to move funds into a contract without that contract's accounting.
interface IEIP3009ReceiveToken {
    function receiveWithAuthorization(
        address from,
        address to,
        uint256 value,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external;
}

library SafeBountyReceive {
    error ReceiveCallFailed();
    error ReceiveReturnedFalse();

    /// @dev Pulls `amount` from `from` into the calling contract under a receive authorization.
    function safeReceiveWithAuthorization(
        address token,
        address from,
        uint256 amount,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) internal {
        (bool ok, bytes memory result) = token.call(
            abi.encodeCall(
                IEIP3009ReceiveToken.receiveWithAuthorization,
                (from, address(this), amount, validAfter, validBefore, nonce, v, r, s)
            )
        );
        if (!ok) revert ReceiveCallFailed();
        if (result.length > 0 && !abi.decode(result, (bool))) revert ReceiveReturnedFalse();
    }
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
