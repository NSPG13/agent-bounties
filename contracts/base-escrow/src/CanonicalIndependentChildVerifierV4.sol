// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./AppealableVerifierV1.sol";
import "./IAgentBounty.sol";
import "./OnchainTermsRegistryV4.sol";

interface IStandingMetaParentFactoryV4View {
    function isCanonicalParent(address parent) external view returns (bool);
    function childFactory() external view returns (address);
    function termsRegistry() external view returns (address);
    function appealableVerifier() external view returns (address);
    function roundChild(address parent, uint64 round) external view returns (address);
    function roundTermsHash(address parent, uint64 round) external view returns (bytes32);
    function roundSelection(address parent, uint64 round)
        external
        view
        returns (bytes32 commitment, uint256 requestId, bytes32 candidateHash);
    function authorizedChildSolver(address parent, uint64 round, address child, address solver)
        external
        view
        returns (bool);
}

interface IStandingMetaParentV4View {
    function bountyId() external view returns (bytes32);
    function creator() external view returns (address);
    function factory() external view returns (address);
    function settlementToken() external view returns (address);
    function verifierModule() external view returns (address);
    function policyHash() external view returns (bytes32);
    function acceptanceCriteriaHash() external view returns (bytes32);
    function solverReward() external view returns (uint256);
    function status() external view returns (uint8);
    function round() external view returns (uint64);
    function solver() external view returns (address);
    function preparedChild() external view returns (address);
    function preparedChildTermsHash() external view returns (bytes32);
    function claimActivatedAt() external view returns (uint64);
    function claimExpiresAt() external view returns (uint64);
    function submissionHash() external view returns (bytes32);
    function evidenceHash() external view returns (bytes32);
}

interface IStandingMetaChildV4View {
    function bountyId() external view returns (bytes32);
    function creator() external view returns (address);
    function factory() external view returns (address);
    function settlementToken() external view returns (address);
    function solverReward() external view returns (uint256);
    function verifierReward() external view returns (uint256);
    function targetAmount() external view returns (uint256);
    function fundedAmount() external view returns (uint256);
    function termsHash() external view returns (bytes32);
    function policyHash() external view returns (bytes32);
    function acceptanceCriteriaHash() external view returns (bytes32);
    function benchmarkHash() external view returns (bytes32);
    function evidenceSchemaHash() external view returns (bytes32);
    function verificationMode() external view returns (uint8);
    function verifierModule() external view returns (address);
    function verifierRewardRecipient() external view returns (address);
    function threshold() external view returns (uint8);
    function claimWindowSeconds() external view returns (uint64);
    function verificationWindowSeconds() external view returns (uint64);
    function status() external view returns (uint8);
    function round() external view returns (uint64);
    function solver() external view returns (address);
    function activeClaimBond() external view returns (uint256);
    function submissionHash() external view returns (bytes32);
    function evidenceHash() external view returns (bytes32);
}

/// @notice Retryable deterministic parent predicate for the atomic,
/// anonymous and appealable standing-meta-v4 policy.
contract CanonicalIndependentChildVerifierV4 is IAgentBountyVerifier {
    struct ParentScope {
        address parent;
        bytes32 bountyId;
        uint64 round;
        address solver;
        bytes32 submissionHash;
        bytes32 evidenceHash;
        bytes32 policyHash;
    }

    bytes32 public constant PROTOCOL_TAG = keccak256("agent-bounties/independent-child-v4");
    uint256 public constant MINIMUM_CHILD_TARGET = 1_000_000;
    uint256 public constant MINIMUM_PARENT_MARGIN = 1_000_000;
    uint256 public constant CHILD_SOLVER_REWARD = 990_000;
    uint256 public constant CHILD_VERIFIER_REWARD = 10_000;
    uint64 public constant CHILD_WORK_WINDOW = 7 days;
    uint64 public constant CHILD_VERIFICATION_WINDOW = 96 hours;
    uint8 public constant DETERMINISTIC_MODE = 0;
    uint8 public constant PARENT_SUBMITTED_STATUS = 3;
    uint8 public constant CHILD_SETTLED_STATUS = 4;

    string public constant ACCEPTANCE_CRITERIA_JSON = '["Use claimAndCreateChild so terms publication, claim-restricted canonical V4 child funding, the active solver-pool snapshot, VRF request, round binding, and the parent bond are atomic.",'
        '"Fund the child with exactly 1.00 USDC: 0.99 USDC solver reward and 0.01 USDC verifier reward.",'
        '"Use the anonymous staked verifier pool, VRF selection, and symmetric one-round appeal policy.",'
        '"Have a VRF-authorized solver wallet other than the parent solver complete the child and receive canonical settlement.",'
        '"Preserve a 1.00 USDC successful-settlement onchain parent margin; this is not guaranteed net profit."]';
    bytes32 public constant ACCEPTANCE_CRITERIA_HASH = keccak256(bytes(ACCEPTANCE_CRITERIA_JSON));

    IStandingMetaParentFactoryV4View public immutable parentFactory;
    address public immutable canonicalChildFactory;
    address public immutable settlementToken;
    OnchainTermsRegistryV4 public immutable termsRegistry;
    AppealableVerifierV1 public immutable appealableVerifier;

    error InvalidProof();
    error InvalidParent();
    error InvalidChild();
    error InvalidTerms();
    error InvalidSelection();

    constructor(
        address parentFactory_,
        address childFactory_,
        address termsRegistry_,
        address appealableVerifier_,
        address settlementToken_
    ) {
        require(parentFactory_ != address(0) && settlementToken_ != address(0), "verifier config invalid");
        parentFactory = IStandingMetaParentFactoryV4View(parentFactory_);
        canonicalChildFactory = childFactory_;
        termsRegistry = OnchainTermsRegistryV4(termsRegistry_);
        appealableVerifier = AppealableVerifierV1(appealableVerifier_);
        require(
            canonicalChildFactory.code.length > 0 && address(termsRegistry).code.length > 0
                && address(appealableVerifier).code.length > 0,
            "verifier dependency missing"
        );
        settlementToken = settlementToken_;
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
        if (proof.length != 32) revert InvalidProof();
        address child = abi.decode(proof, (address));
        ParentScope memory scope = ParentScope({
            parent: msg.sender,
            bountyId: bountyId,
            round: round,
            solver: solver,
            submissionHash: submissionHash,
            evidenceHash: evidenceHash,
            policyHash: policyHash
        });
        responseHash = _verify(scope, child);
        return (true, responseHash);
    }

    function _verify(ParentScope memory scope, address childAddress) private view returns (bytes32) {
        IStandingMetaParentV4View parent = _validParent(scope, childAddress);
        IStandingMetaChildV4View child = _validChild(parent, scope, childAddress);
        _validTerms(parent, child, scope);
        _validSelection(child, scope);
        return keccak256(
            abi.encode(
                PROTOCOL_TAG,
                scope,
                childAddress,
                child.bountyId(),
                child.solver(),
                child.targetAmount(),
                child.submissionHash(),
                child.evidenceHash(),
                child.termsHash()
            )
        );
    }

    function _validParent(ParentScope memory scope, address childAddress)
        private
        view
        returns (IStandingMetaParentV4View parent)
    {
        if (!parentFactory.isCanonicalParent(scope.parent)) revert InvalidParent();
        parent = IStandingMetaParentV4View(scope.parent);
        if (
            parent.bountyId() != scope.bountyId || parent.factory() != address(parentFactory)
                || parent.settlementToken() != settlementToken || parent.verifierModule() != address(this)
                || parent.acceptanceCriteriaHash() != ACCEPTANCE_CRITERIA_HASH || parent.solverReward() != 2_000_000
                || parent.status() != PARENT_SUBMITTED_STATUS || parent.round() != scope.round
                || parent.solver() != scope.solver || parent.preparedChild() != childAddress
                || parent.preparedChildTermsHash() != parentFactory.roundTermsHash(scope.parent, scope.round)
                || parent.submissionHash() != scope.submissionHash || parent.evidenceHash() != scope.evidenceHash
                || parent.policyHash() != scope.policyHash
        ) revert InvalidParent();
    }

    function _validChild(IStandingMetaParentV4View parent, ParentScope memory scope, address childAddress)
        private
        view
        returns (IStandingMetaChildV4View child)
    {
        if (
            childAddress == address(0) || childAddress == scope.parent
                || !IProfitableChildFactoryViewV4(canonicalChildFactory).isCanonicalChild(childAddress)
        ) revert InvalidChild();
        child = IStandingMetaChildV4View(childAddress);
        if (
            child.creator() != scope.solver || child.factory() != canonicalChildFactory
                || child.settlementToken() != settlementToken || child.targetAmount() < MINIMUM_CHILD_TARGET
                || child.targetAmount() > parent.solverReward() - MINIMUM_PARENT_MARGIN
                || child.solverReward() != CHILD_SOLVER_REWARD || child.verifierReward() != CHILD_VERIFIER_REWARD
                || child.verificationMode() != DETERMINISTIC_MODE
                || child.verifierModule() != address(appealableVerifier)
                || child.verifierRewardRecipient() != address(appealableVerifier) || child.threshold() != 1
                || child.claimWindowSeconds() != CHILD_WORK_WINDOW
                || child.verificationWindowSeconds() != CHILD_VERIFICATION_WINDOW
                || child.status() != CHILD_SETTLED_STATUS || child.fundedAmount() != 0 || child.activeClaimBond() != 0
                || child.solver() == address(0) || child.solver() == scope.solver
                || child.submissionHash() == bytes32(0) || child.evidenceHash() == bytes32(0)
        ) revert InvalidChild();
    }

    function _validTerms(IStandingMetaParentV4View parent, IStandingMetaChildV4View child, ParentScope memory scope)
        private
        view
    {
        bytes32 termsHash = child.termsHash();
        _validBinding(parent, child, scope, termsHash);
        _validContent(child, termsHash);
        _validEconomics(parent, child, termsHash);
    }

    function _validBinding(
        IStandingMetaParentV4View parent,
        IStandingMetaChildV4View child,
        ParentScope memory scope,
        bytes32 termsHash
    ) private view {
        (
            address publisher,
            address boundParent,
            address boundChild,
            uint64 publishedAt,
            uint64 parentRound,
            bytes32 parentBountyId,
            bytes32 selectionCommitment
        ) = termsRegistry.bindings(termsHash);
        if (
            publisher != scope.solver || boundParent != scope.parent || boundChild != address(child) || publishedAt == 0
                || publishedAt != parent.claimActivatedAt() || parentRound != scope.round
                || parentBountyId != scope.bountyId || selectionCommitment == bytes32(0)
        ) revert InvalidTerms();
    }

    function _validContent(IStandingMetaChildV4View child, bytes32 termsHash) private view {
        (
            address verifierModule,
            bytes32 taskPolicyHash,
            bytes32 criteriaHash,
            bytes32 benchmarkHash,
            bytes32 evidenceSchemaHash,
            bytes32 appealPolicyHash
        ) = termsRegistry.contents(termsHash);
        if (
            verifierModule != address(appealableVerifier) || taskPolicyHash != child.policyHash()
                || criteriaHash != child.acceptanceCriteriaHash() || benchmarkHash != child.benchmarkHash()
                || evidenceSchemaHash != child.evidenceSchemaHash()
                || appealPolicyHash != appealableVerifier.appealPolicyHash()
        ) revert InvalidTerms();
    }

    function _validEconomics(IStandingMetaParentV4View parent, IStandingMetaChildV4View child, bytes32 termsHash)
        private
        view
    {
        (
            uint64 selectionRequestedAt,
            uint64 childClaimWindow,
            uint64 childVerificationWindow,
            uint64 childTarget,
            uint64 childSolverReward,
            uint64 childVerifierReward
        ) = termsRegistry.economicsTiming(termsHash);
        if (
            selectionRequestedAt != parent.claimActivatedAt() || childClaimWindow != CHILD_WORK_WINDOW
                || childVerificationWindow != CHILD_VERIFICATION_WINDOW || childTarget != child.targetAmount()
                || childSolverReward != child.solverReward() || childVerifierReward != child.verifierReward()
        ) revert InvalidTerms();
    }

    function _validSelection(IStandingMetaChildV4View child, ParentScope memory scope) private view {
        bytes32 termsHash = child.termsHash();
        (bytes32 commitment, uint256 requestId, bytes32 candidateHash) =
            parentFactory.roundSelection(scope.parent, scope.round);
        if (
            parentFactory.roundChild(scope.parent, scope.round) != address(child)
                || termsRegistry.selectionRequestId(termsHash) != requestId
                || termsRegistry.selectionCandidateHash(termsHash) != candidateHash || commitment == bytes32(0)
                || requestId == 0 || candidateHash == bytes32(0)
                || !parentFactory.authorizedChildSolver(scope.parent, scope.round, address(child), child.solver())
        ) revert InvalidSelection();
        (,,,,,, bytes32 boundCommitment) = termsRegistry.bindings(termsHash);
        if (boundCommitment != commitment) revert InvalidSelection();
    }
}

interface IProfitableChildFactoryViewV4 {
    function isCanonicalChild(address bounty) external view returns (bool);
}
