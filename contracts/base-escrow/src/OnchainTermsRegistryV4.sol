// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

/// @notice Typed terms for the atomic, anonymous standing-meta-v4 successor.
/// The trusted publisher is the immutable parent factory, which may publish only
/// on behalf of the solver currently calling its atomic preparation function.
contract OnchainTermsRegistryV4 {
    uint256 public constant MAX_TERMS_BYTES = 32_768;

    struct Binding {
        address publisher;
        address parent;
        address child;
        uint64 publishedAt;
        uint64 parentRound;
        bytes32 parentBountyId;
        bytes32 selectionCommitment;
    }

    struct Content {
        address verifierModule;
        bytes32 policyHash;
        bytes32 acceptanceCriteriaHash;
        bytes32 benchmarkHash;
        bytes32 evidenceSchemaHash;
        bytes32 appealPolicyHash;
    }

    struct EconomicsTiming {
        uint64 selectionRequestedAt;
        uint64 childClaimWindowSeconds;
        uint64 childVerificationWindowSeconds;
        uint64 childFundingTarget;
        uint64 childSolverReward;
        uint64 childVerifierReward;
    }

    struct TermsInput {
        address parent;
        address child;
        bytes32 parentBountyId;
        uint64 parentRound;
        bytes32 selectionCommitment;
        address verifierModule;
        bytes32 policyHash;
        bytes32 acceptanceCriteriaHash;
        bytes32 benchmarkHash;
        bytes32 evidenceSchemaHash;
        bytes32 appealPolicyHash;
        uint64 selectionRequestedAt;
        uint64 childClaimWindowSeconds;
        uint64 childVerificationWindowSeconds;
        uint64 childFundingTarget;
        uint64 childSolverReward;
        uint64 childVerifierReward;
    }

    address public immutable publisherAuthority;
    mapping(bytes32 => Binding) public bindings;
    mapping(bytes32 => Content) public contents;
    mapping(bytes32 => EconomicsTiming) public economicsTiming;
    mapping(bytes32 => uint256) public selectionRequestId;
    mapping(bytes32 => bytes32) public selectionCandidateHash;

    event TermsPublished(bytes32 indexed termsHash, address indexed publisher, address indexed parent, bytes terms);
    event SelectionBound(
        bytes32 indexed termsHash, bytes32 indexed selectionCommitment, uint256 requestId, bytes32 candidateHash
    );

    constructor(address publisherAuthority_) {
        require(publisherAuthority_ != address(0), "publisher authority zero");
        publisherAuthority = publisherAuthority_;
    }

    function publishFor(address publisher, bytes calldata canonicalTerms, TermsInput calldata input)
        external
        returns (bytes32 termsHash)
    {
        require(msg.sender == publisherAuthority, "publisher authority only");
        require(publisher != address(0), "publisher zero");
        require(canonicalTerms.length > 0 && canonicalTerms.length <= MAX_TERMS_BYTES, "terms size invalid");
        require(
            input.parent != address(0) && input.child != address(0) && input.parentBountyId != bytes32(0)
                && input.parentRound > 0 && input.selectionCommitment != bytes32(0),
            "binding invalid"
        );
        require(
            input.verifierModule != address(0) && input.policyHash != bytes32(0)
                && input.acceptanceCriteriaHash != bytes32(0) && input.benchmarkHash != bytes32(0)
                && input.evidenceSchemaHash != bytes32(0) && input.appealPolicyHash != bytes32(0),
            "content invalid"
        );
        require(
            input.selectionRequestedAt == block.timestamp && input.childClaimWindowSeconds > 0
                && input.childVerificationWindowSeconds > 0 && input.childFundingTarget > 0
                && input.childSolverReward > 0 && input.childVerifierReward > 0,
            "economics or timing invalid"
        );

        termsHash = keccak256(canonicalTerms);
        require(bindings[termsHash].publishedAt == 0, "terms already published");
        bindings[termsHash] = Binding({
            publisher: publisher,
            parent: input.parent,
            child: input.child,
            publishedAt: uint64(block.timestamp),
            parentRound: input.parentRound,
            parentBountyId: input.parentBountyId,
            selectionCommitment: input.selectionCommitment
        });
        contents[termsHash] = Content({
            verifierModule: input.verifierModule,
            policyHash: input.policyHash,
            acceptanceCriteriaHash: input.acceptanceCriteriaHash,
            benchmarkHash: input.benchmarkHash,
            evidenceSchemaHash: input.evidenceSchemaHash,
            appealPolicyHash: input.appealPolicyHash
        });
        economicsTiming[termsHash] = EconomicsTiming({
            selectionRequestedAt: input.selectionRequestedAt,
            childClaimWindowSeconds: input.childClaimWindowSeconds,
            childVerificationWindowSeconds: input.childVerificationWindowSeconds,
            childFundingTarget: input.childFundingTarget,
            childSolverReward: input.childSolverReward,
            childVerifierReward: input.childVerifierReward
        });
        emit TermsPublished(termsHash, publisher, input.parent, canonicalTerms);
    }

    function bindSelection(bytes32 termsHash, uint256 requestId, bytes32 candidateHash) external {
        require(msg.sender == publisherAuthority, "publisher authority only");
        require(bindings[termsHash].publishedAt != 0, "terms unavailable");
        require(requestId != 0 && candidateHash != bytes32(0), "selection invalid");
        require(selectionRequestId[termsHash] == 0, "selection already bound");
        selectionRequestId[termsHash] = requestId;
        selectionCandidateHash[termsHash] = candidateHash;
        emit SelectionBound(termsHash, bindings[termsHash].selectionCommitment, requestId, candidateHash);
    }
}
