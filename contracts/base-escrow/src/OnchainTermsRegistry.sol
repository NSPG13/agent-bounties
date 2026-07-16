// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

/// @notice Publishes exact task terms and typed commitments before a standing-meta claim.
/// The full preimage is emitted once so indexers can always reconstruct it from Base.
contract OnchainTermsRegistry {
    uint256 public constant MAX_TERMS_BYTES = 32_768;

    struct TermsCommitment {
        address publisher;
        uint64 publishedAt;
        uint64 parentRound;
        uint8 verifierThreshold;
        uint32 byteLength;
        bytes32 parentBountyId;
        bytes32 policyHash;
        bytes32 acceptanceCriteriaHash;
        bytes32 benchmarkHash;
        bytes32 evidenceSchemaHash;
        bytes32 verifierSetHash;
    }

    struct TermsInput {
        bytes32 parentBountyId;
        uint64 parentRound;
        bytes32 policyHash;
        bytes32 acceptanceCriteriaHash;
        bytes32 benchmarkHash;
        bytes32 evidenceSchemaHash;
        bytes32 verifierSetHash;
        uint8 verifierThreshold;
    }

    mapping(bytes32 => TermsCommitment) public commitments;

    event TermsPublished(bytes32 indexed termsHash, address indexed publisher, bytes canonicalTerms);

    function commitment(bytes32 termsHash) external view returns (TermsCommitment memory) {
        return commitments[termsHash];
    }

    function publish(bytes calldata canonicalTerms, TermsInput calldata input) external returns (bytes32 termsHash) {
        require(canonicalTerms.length > 0 && canonicalTerms.length <= MAX_TERMS_BYTES, "terms size out of bounds");
        require(input.parentBountyId != bytes32(0) && input.parentRound > 0, "parent binding missing");
        require(
            input.policyHash != bytes32(0) && input.acceptanceCriteriaHash != bytes32(0)
                && input.benchmarkHash != bytes32(0) && input.evidenceSchemaHash != bytes32(0),
            "content commitment missing"
        );
        require(input.verifierSetHash != bytes32(0) && input.verifierThreshold >= 2, "verifier commitment invalid");

        termsHash = keccak256(canonicalTerms);
        require(commitments[termsHash].publishedAt == 0, "terms already published");
        commitments[termsHash] = TermsCommitment({
            publisher: msg.sender,
            publishedAt: uint64(block.timestamp),
            parentRound: input.parentRound,
            verifierThreshold: input.verifierThreshold,
            byteLength: uint32(canonicalTerms.length),
            parentBountyId: input.parentBountyId,
            policyHash: input.policyHash,
            acceptanceCriteriaHash: input.acceptanceCriteriaHash,
            benchmarkHash: input.benchmarkHash,
            evidenceSchemaHash: input.evidenceSchemaHash,
            verifierSetHash: input.verifierSetHash
        });
        emit TermsPublished(termsHash, msg.sender, canonicalTerms);
    }
}
