// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";

/// @notice Permissionless deterministic work verifier for end-to-end bounty loops.
/// The work hash is bound to the exact bounty round, solver, submitted hashes,
/// and immutable policy. Anyone can mine and relay a proof, but it cannot be
/// reused for another solver, round, or submission.
contract LeadingZeroWorkVerifier is IAgentBountyVerifier {
    uint8 public immutable difficultyBits;

    constructor(uint8 difficultyBits_) {
        require(difficultyBits_ > 0 && difficultyBits_ <= 32, "difficulty out of bounds");
        difficultyBits = difficultyBits_;
    }

    function workHash(
        bytes32 bountyId,
        uint64 round,
        address solver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 policyHash,
        uint256 nonce
    ) public pure returns (bytes32) {
        return keccak256(abi.encode(bountyId, round, solver, submissionHash, evidenceHash, policyHash, nonce));
    }

    function verify(
        bytes32 bountyId,
        uint64 round,
        address solver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 policyHash,
        bytes calldata proof
    ) external view returns (bool passed, bytes32 responseHash) {
        if (proof.length != 32) {
            return (false, keccak256(abi.encode("malformed-leading-zero-work-proof", keccak256(proof))));
        }

        uint256 nonce = abi.decode(proof, (uint256));
        responseHash = workHash(bountyId, round, solver, submissionHash, evidenceHash, policyHash, nonce);
        passed = uint256(responseHash) >> (256 - difficultyBits) == 0;
    }
}
