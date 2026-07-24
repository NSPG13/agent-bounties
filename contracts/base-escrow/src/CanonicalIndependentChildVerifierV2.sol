// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";
import "./OnchainTermsRegistry.sol";
import "./ParticipantEligibilityRegistry.sol";

interface IIndependentChildFactoryView {
    function settlementToken() external view returns (address);
    function isCanonicalBounty(address bounty) external view returns (bool);
}

interface IIndependentChildBountyView {
    function bountyId() external view returns (bytes32);
    function creator() external view returns (address);
    function factory() external view returns (address);
    function settlementToken() external view returns (address);
    function solverReward() external view returns (uint256);
    function targetAmount() external view returns (uint256);
    function fundedAmount() external view returns (uint256);
    function termsHash() external view returns (bytes32);
    function policyHash() external view returns (bytes32);
    function acceptanceCriteriaHash() external view returns (bytes32);
    function benchmarkHash() external view returns (bytes32);
    function evidenceSchemaHash() external view returns (bytes32);
    function verifierSetHash() external view returns (bytes32);
    function verificationMode() external view returns (uint8);
    function verifierModule() external view returns (address);
    function threshold() external view returns (uint8);
    function status() external view returns (uint8);
    function round() external view returns (uint64);
    function solver() external view returns (address);
    function activeClaimBond() external view returns (uint256);
    function submissionHash() external view returns (bytes32);
    function evidenceHash() external view returns (bytes32);
    function claimExpiresAt() external view returns (uint64);
    function claimWindowSeconds() external view returns (uint64);
}

/// @notice Settles a standing meta-bounty only after a precommitted, task-verified,
/// independently completed child pays out. Incomplete or malformed proofs revert,
/// so an untrusted caller cannot turn missing evidence into a rejected submission.
contract CanonicalIndependentChildVerifierV2 is IAgentBountyVerifier {
    struct ParentScope {
        address parentAddress;
        bytes32 bountyId;
        uint64 round;
        address solver;
        bytes32 submissionHash;
        bytes32 evidenceHash;
        bytes32 policyHash;
    }

    bytes32 public constant PROTOCOL_TAG = keccak256("agent-bounties/independent-child-v2");
    uint8 public constant DETERMINISTIC_MODULE_MODE = 0;
    uint8 public constant SIGNED_QUORUM_MODE = 1;
    uint8 public constant SUBMITTED_STATUS = 3;
    uint8 public constant SETTLED_STATUS = 4;

    string public constant ACCEPTANCE_CRITERIA_JSON = '["Publish the exact child terms on Base from the parent solver wallet before claiming this bounty.",'
        '"Create and fully fund the parent-bound canonical child to at least this bounty solver reward.",'
        '"Use the committed sandboxed-regression signed verifier quorum and immutable task criteria.",'
        '"Have a participant registered before the parent claim, with a different participant ID, complete the child.",'
        '"Receive canonical child settlement before submitting the child address to this verifier."]';
    bytes32 public constant ACCEPTANCE_CRITERIA_HASH = keccak256(bytes(ACCEPTANCE_CRITERIA_JSON));

    address public immutable canonicalFactory;
    address public immutable settlementToken;
    ParticipantEligibilityRegistry public immutable participantRegistry;
    OnchainTermsRegistry public immutable termsRegistry;
    bytes32 public immutable taskVerifierSetHash;
    uint8 public immutable taskVerifierThreshold;

    error InvalidProof();
    error InvalidParent();
    error InvalidChild();
    error TermsUnavailable();
    error ParticipantIneligible();
    error SameParticipant();

    constructor(
        address canonicalFactory_,
        address participantRegistry_,
        address termsRegistry_,
        bytes32 taskVerifierSetHash_,
        uint8 taskVerifierThreshold_
    ) {
        require(canonicalFactory_ != address(0), "factory zero");
        require(participantRegistry_.code.length > 0, "participant registry missing");
        require(termsRegistry_.code.length > 0, "terms registry missing");
        require(taskVerifierSetHash_ != bytes32(0) && taskVerifierThreshold_ >= 2, "verifier policy invalid");
        address token = IIndependentChildFactoryView(canonicalFactory_).settlementToken();
        require(token != address(0), "token zero");
        canonicalFactory = canonicalFactory_;
        settlementToken = token;
        participantRegistry = ParticipantEligibilityRegistry(participantRegistry_);
        termsRegistry = OnchainTermsRegistry(termsRegistry_);
        taskVerifierSetHash = taskVerifierSetHash_;
        taskVerifierThreshold = taskVerifierThreshold_;
    }

    /// @notice The proof is exactly abi.encode(address childBounty).
    function verify(
        bytes32 parentBountyId,
        uint64 parentRound,
        address parentSolver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 policyHash,
        bytes calldata proof
    ) external view returns (bool passed, bytes32 responseHash) {
        if (proof.length != 32) revert InvalidProof();
        uint256 encodedChild;
        assembly ("memory-safe") {
            encodedChild := calldataload(proof.offset)
        }
        if (encodedChild >> 160 != 0) revert InvalidProof();
        // The high-bit check above proves this cast cannot truncate.
        // forge-lint: disable-next-line(unsafe-typecast)
        address childAddress = address(uint160(encodedChild));

        ParentScope memory scope = ParentScope({
            parentAddress: msg.sender,
            bountyId: parentBountyId,
            round: parentRound,
            solver: parentSolver,
            submissionHash: submissionHash,
            evidenceHash: evidenceHash,
            policyHash: policyHash
        });
        responseHash = _verify(scope, childAddress);
        return (true, responseHash);
    }

    function _verify(ParentScope memory scope, address childAddress) private view returns (bytes32 responseHash) {
        IIndependentChildBountyView parent =
            _validParent(scope.parentAddress, scope.bountyId, scope.round, scope.solver);
        uint64 claimedAt = parent.claimExpiresAt() - parent.claimWindowSeconds();
        IIndependentChildBountyView child = _validChild(parent, childAddress, scope.solver);
        _validTerms(child, scope.bountyId, scope.round, scope.solver, claimedAt);
        _validParticipants(scope.solver, child.solver(), claimedAt);

        responseHash = keccak256(
            abi.encode(
                PROTOCOL_TAG,
                scope,
                childAddress,
                child.bountyId(),
                child.solver(),
                child.submissionHash(),
                child.evidenceHash(),
                child.termsHash()
            )
        );
    }

    function _validParent(address parentAddress, bytes32 parentBountyId, uint64 parentRound, address parentSolver)
        private
        view
        returns (IIndependentChildBountyView parent)
    {
        if (!IIndependentChildFactoryView(canonicalFactory).isCanonicalBounty(parentAddress)) revert InvalidParent();
        parent = IIndependentChildBountyView(parentAddress);
        if (
            parent.bountyId() != parentBountyId || parent.factory() != canonicalFactory
                || parent.settlementToken() != settlementToken
                || parent.acceptanceCriteriaHash() != ACCEPTANCE_CRITERIA_HASH
                || parent.verificationMode() != DETERMINISTIC_MODULE_MODE || parent.verifierModule() != address(this)
                || parent.threshold() != 1 || parent.status() != SUBMITTED_STATUS || parent.round() != parentRound
                || parent.solver() != parentSolver || parent.claimExpiresAt() < parent.claimWindowSeconds()
        ) revert InvalidParent();
    }

    function _validChild(IIndependentChildBountyView parent, address childAddress, address parentSolver)
        private
        view
        returns (IIndependentChildBountyView child)
    {
        if (
            childAddress == address(0) || childAddress == address(parent)
                || !IIndependentChildFactoryView(canonicalFactory).isCanonicalBounty(childAddress)
        ) revert InvalidChild();
        child = IIndependentChildBountyView(childAddress);
        if (
            child.creator() != parentSolver || child.factory() != canonicalFactory
                || child.settlementToken() != settlementToken || child.targetAmount() < parent.solverReward()
                || child.verificationMode() != SIGNED_QUORUM_MODE || child.verifierModule() != address(0)
                || child.verifierSetHash() != taskVerifierSetHash || child.threshold() != taskVerifierThreshold
                || child.status() != SETTLED_STATUS || child.fundedAmount() != 0 || child.activeClaimBond() != 0
                || child.solver() == address(0) || child.solver() == parentSolver
                || child.submissionHash() == bytes32(0) || child.evidenceHash() == bytes32(0)
        ) revert InvalidChild();
    }

    function _validTerms(
        IIndependentChildBountyView child,
        bytes32 parentBountyId,
        uint64 parentRound,
        address parentSolver,
        uint64 parentClaimedAt
    ) private view {
        OnchainTermsRegistry.TermsCommitment memory record = termsRegistry.commitment(child.termsHash());
        if (record.publishedAt == 0) revert TermsUnavailable();
        if (
            record.publisher != parentSolver || record.publishedAt >= parentClaimedAt
                || record.parentBountyId != parentBountyId || record.parentRound != parentRound
                || record.policyHash != child.policyHash()
                || record.acceptanceCriteriaHash != child.acceptanceCriteriaHash()
                || record.benchmarkHash != child.benchmarkHash()
                || record.evidenceSchemaHash != child.evidenceSchemaHash()
                || record.verifierSetHash != taskVerifierSetHash || record.verifierThreshold != taskVerifierThreshold
        ) revert TermsUnavailable();
    }

    function _validParticipants(address parentSolver, address childSolver, uint64 parentClaimedAt) private view {
        (bytes32 parentParticipant,, bool parentEligible) =
            participantRegistry.eligibleAt(parentSolver, parentClaimedAt);
        (bytes32 childParticipant,, bool childEligible) = participantRegistry.eligibleAt(childSolver, parentClaimedAt);
        if (!parentEligible || !childEligible) revert ParticipantIneligible();
        if (parentParticipant == childParticipant) revert SameParticipant();
    }
}
