// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./AgentBountyEscrow.sol";

contract AgentBountyEscrowV2 is AgentBountyEscrow {
    struct TermsAcceptance {
        bool accepted;
        bytes32 termsHash;
        address payoutWallet;
    }

    mapping(uint256 => mapping(address => TermsAcceptance)) public termsAcceptances;

    event TermsAccepted(
        uint256 indexed escrowId,
        address indexed participant,
        address indexed payoutWallet,
        bytes32 termsHash
    );

    constructor(address settlementSigner_) AgentBountyEscrow(settlementSigner_) {}

    function acceptTerms(uint256 escrowId, bytes32 termsHash, address payoutWallet) external {
        Escrow storage escrow = escrows[escrowId];
        require(
            escrow.status == EscrowStatus.Funded || escrow.status == EscrowStatus.Disputed,
            "not active"
        );
        require(escrow.termsHash == termsHash, "terms mismatch");
        require(payoutWallet != address(0), "payout wallet zero");
        require(payoutWallet == msg.sender, "wallet must sign");

        termsAcceptances[escrowId][msg.sender] = TermsAcceptance({
            accepted: true,
            termsHash: termsHash,
            payoutWallet: payoutWallet
        });

        emit TermsAccepted(escrowId, msg.sender, payoutWallet, termsHash);
    }

    function hasAcceptedTerms(
        uint256 escrowId,
        address participant,
        bytes32 termsHash
    ) public view returns (bool) {
        TermsAcceptance storage acceptance = termsAcceptances[escrowId][participant];
        return acceptance.accepted && acceptance.termsHash == termsHash;
    }

    function release(
        uint256 escrowId,
        address[] calldata recipients,
        uint256[] calldata amounts,
        bytes32 proofHash
    ) public override onlySettlementSigner {
        Escrow storage escrow = escrows[escrowId];
        require(escrow.status == EscrowStatus.Funded || escrow.status == EscrowStatus.Disputed, "not releasable");
        require(recipients.length == amounts.length && recipients.length > 0, "bad split");

        uint256 total;
        for (uint256 i = 0; i < recipients.length; i++) {
            require(recipients[i] != address(0), "recipient zero");
            require(hasAcceptedTerms(escrowId, recipients[i], escrow.termsHash), "recipient not accepted");
            total += amounts[i];
        }
        require(total == escrow.amount, "split mismatch");

        escrow.status = EscrowStatus.Released;
        for (uint256 i = 0; i < recipients.length; i++) {
            require(IERC20(escrow.token).transfer(recipients[i], amounts[i]), "transfer failed");
        }

        emit EscrowReleased(escrowId, proofHash);
    }
}
