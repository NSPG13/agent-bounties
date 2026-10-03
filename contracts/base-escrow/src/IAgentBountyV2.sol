// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";

/// @notice Autonomous-v2 adds an explicit platform fee to the v1 bounty surface.
/// The fee rate, amount, and recipient are fixed at creation and included in the
/// funding target. The fee accrues only on settlement and is pulled to the fixed
/// recipient, so recipient liveness can never block solver or verifier payment.
interface IAgentBountyV2 is IAgentBountyV1 {
    function platformFeeBps() external view returns (uint16);
    function platformFee() external view returns (uint256);
    function platformFeeRecipient() external view returns (address);
    function platformFeeAccrued() external view returns (uint256);
    function withdrawPlatformFee() external returns (uint256 amount);
}
