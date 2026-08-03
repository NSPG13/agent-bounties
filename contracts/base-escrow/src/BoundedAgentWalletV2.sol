// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./BoundedAgentWallet.sol";

/// @notice V2 adds an owner escape hatch for unclaimed bounties created by this wallet.
/// @dev V1 remains immutable. This implementation is used only by the versioned V2 factory.
contract BoundedAgentWalletV2 is BoundedAgentWallet {
    bytes32 public constant WALLET_VERSION = keccak256("agent-bounties/bounded-wallet/v2");

    event OwnerBountyCancelledAndRefunded(
        address indexed bounty, bytes32 indexed bountyId, uint256 principal, uint256 amount, uint256 walletBalance
    );
    event OwnerCancelledBountyRefundWithdrawn(
        address indexed bounty, bytes32 indexed bountyId, uint256 principal, uint256 amount, uint256 walletBalance
    );

    constructor(address factory_) BoundedAgentWallet(factory_) {}

    /// @notice Cancel an unclaimed canonical bounty created by this wallet and pull only its refund.
    /// @dev Other contributors retain their independent pull-refund rights on the bounty contract.
    function cancelAndWithdrawUnclaimedBounty(address bountyAddress)
        external
        onlyOwner
        nonReentrant
        returns (uint256 amount)
    {
        AgentBounty bounty = _creatorBounty(bountyAddress);

        uint8 currentStatus = bounty.status();
        require(
            currentStatus == uint8(AgentBounty.BountyStatus.Open)
                || currentStatus == uint8(AgentBounty.BountyStatus.Claimable),
            "bounty not cancellable"
        );
        require(bounty.solver() == address(0), "solver active");
        require(bounty.activeClaimBond() == 0, "claim bond active");
        require(bounty.submissionHash() == bytes32(0) && bounty.evidenceHash() == bytes32(0), "submission active");

        uint256 principal = bounty.contributions(address(this));
        require(principal > 0, "wallet has no contribution");

        bounty.cancel();
        amount = _withdrawRefund(bounty, principal);

        emit OwnerBountyCancelledAndRefunded(
            bountyAddress,
            bounty.bountyId(),
            principal,
            amount,
            IERC20BountyToken(settlementToken).balanceOf(address(this))
        );
    }

    /// @notice Pull this wallet's refund after a third party cancels an expired bounty.
    function withdrawCancelledBountyRefund(address bountyAddress)
        external
        onlyOwner
        nonReentrant
        returns (uint256 amount)
    {
        AgentBounty bounty = _creatorBounty(bountyAddress);
        require(bounty.status() == uint8(AgentBounty.BountyStatus.Cancelled), "bounty not cancelled");
        uint256 principal = bounty.contributions(address(this));
        require(principal > 0, "wallet has no contribution");
        amount = _withdrawRefund(bounty, principal);
        emit OwnerCancelledBountyRefundWithdrawn(
            bountyAddress,
            bounty.bountyId(),
            principal,
            amount,
            IERC20BountyToken(settlementToken).balanceOf(address(this))
        );
    }

    function _creatorBounty(address bountyAddress) private view returns (AgentBounty bounty) {
        require(factory.isCanonicalBounty(bountyAddress), "not canonical bounty");
        bounty = AgentBounty(bountyAddress);
        require(bounty.factory() == address(factory), "wrong bounty factory");
        require(bounty.settlementToken() == settlementToken, "wrong settlement token");
        require(bounty.creator() == address(this), "wallet not creator");
    }

    function _withdrawRefund(AgentBounty bounty, uint256 principal) private returns (uint256 amount) {
        uint256 balanceBefore = IERC20BountyToken(settlementToken).balanceOf(address(this));
        bounty.withdrawRefund();
        uint256 balanceAfter = IERC20BountyToken(settlementToken).balanceOf(address(this));
        require(balanceAfter >= balanceBefore + principal, "refund not received");
        amount = balanceAfter - balanceBefore;
    }
}
