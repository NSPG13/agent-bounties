// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./AgentBountyFactory.sol";
import "./AnonymousProtocolControllerV1.sol";
import "./CanonicalIndependentChildVerifierV4.sol";
import "./OnchainTermsRegistryV4.sol";
import "./StandingMetaParentV4.sol";

/// @notice Creates exact-economics parents and owns the only atomic preparation
/// path, immediate active-pool draw, and authorized solver promotion policy.
contract StandingMetaParentFactoryV4 {
    using SafeBountyToken for address;

    struct ParentConfig {
        bytes32 termsHash;
        bytes32 policyHash;
        bytes32 benchmarkHash;
        bytes32 evidenceSchemaHash;
        bytes32 creationNonce;
    }

    struct BondAuthorization {
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
        uint8 v;
        bytes32 r;
        bytes32 s;
    }

    struct ClaimAndCreateChildRequest {
        bytes canonicalTerms;
        OnchainTermsRegistryV4.TermsInput terms;
        AgentBountyFactory.CreateBountyParams childParams;
        bytes32 childCreationNonce;
        AgentBountyFactory.FundingAuthorization childFundingAuthorization;
        BondAuthorization parentBondAuthorization;
    }

    struct RoundData {
        address child;
        address authorizedSolver;
        bytes32 termsHash;
        bytes32 selectionCommitment;
        bytes32 candidateHash;
        uint64 selectionRequestedAt;
        uint64 assignmentDeadline;
        uint256 requestId;
        uint8 currentRank;
        bool rankingActivated;
    }

    struct PreparationContext {
        uint64 parentRound;
        uint64 selectionRequestedAt;
        address predictedChild;
        bytes32 childTermsHash;
        bytes32 selectionCommitment;
    }

    uint256 public constant CHILD_TARGET = 1_000_000;
    uint256 public constant CHILD_SOLVER_REWARD = 990_000;
    uint256 public constant CHILD_VERIFIER_REWARD = 10_000;
    uint64 public constant ASSIGNMENT_WINDOW = 10 minutes;
    uint64 public constant CHILD_WORK_WINDOW = 7 days;
    uint64 public constant CHILD_VERIFICATION_WINDOW = 96 hours;
    uint8 public constant MINIMUM_CHILD_SOLVER_CANDIDATES = 3;
    uint8 public constant DETERMINISTIC_MODE = 0;
    uint8 public constant CLAIMABLE_STATUS = 1;

    AgentBountyFactory public immutable childFactory;
    address public immutable settlementToken;
    AnonymousProtocolControllerV1 public immutable controller;
    AppealableVerifierV1 public immutable appealableVerifier;
    OnchainTermsRegistryV4 public immutable termsRegistry;
    CanonicalIndependentChildVerifierV4 public immutable verifierModule;

    mapping(address => bool) public isCanonicalParent;
    mapping(address => mapping(uint64 => RoundData)) private _rounds;
    mapping(address => mapping(uint64 => address[])) private _ranking;
    mapping(address => mapping(uint64 => mapping(address => bool))) private _authorized;

    event StandingMetaParentCreated(bytes32 indexed bountyId, address indexed parent, address indexed creator);
    event AtomicChildPrepared(
        address indexed parent,
        uint64 indexed round,
        address indexed solver,
        address child,
        bytes32 termsHash,
        bytes32 selectionCommitment,
        uint64 selectionRequestedAt,
        uint256 selectionRequestId,
        bytes32 candidateHash
    );
    event ChildSolverDrawRequested(
        address indexed parent,
        uint64 indexed round,
        uint256 indexed requestId,
        bytes32 candidateHash,
        uint256 candidateCount
    );
    event ChildSolverAssigned(
        address indexed parent, uint64 indexed round, address indexed candidate, uint8 rank, uint64 deadline
    );
    event ChildSolverClaimed(address indexed parent, uint64 indexed round, address indexed solver, address child);
    event ChildSolverPromoted(
        address indexed parent, uint64 indexed round, address indexed candidate, uint8 rank, bytes32 reason
    );

    constructor(address childFactory_, address controller_, address appealableVerifier_) {
        require(
            childFactory_.code.length > 0 && controller_.code.length > 0 && appealableVerifier_.code.length > 0,
            "factory dependency missing"
        );
        childFactory = AgentBountyFactory(childFactory_);
        settlementToken = childFactory.settlementToken();
        controller = AnonymousProtocolControllerV1(controller_);
        appealableVerifier = AppealableVerifierV1(appealableVerifier_);
        termsRegistry = new OnchainTermsRegistryV4(address(this));
        verifierModule = new CanonicalIndependentChildVerifierV4(
            address(this), childFactory_, address(termsRegistry), appealableVerifier_, settlementToken
        );
    }

    function createParent(ParentConfig calldata config) external returns (address parentAddress, bytes32 bountyId) {
        require(
            config.termsHash != bytes32(0) && config.policyHash != bytes32(0) && config.benchmarkHash != bytes32(0)
                && config.evidenceSchemaHash != bytes32(0) && config.creationNonce != bytes32(0),
            "parent config invalid"
        );
        bountyId = keccak256(abi.encode(block.chainid, address(this), msg.sender, config.creationNonce, config));
        StandingMetaParentV4 parent = new StandingMetaParentV4(
            bountyId,
            msg.sender,
            address(this),
            settlementToken,
            address(verifierModule),
            config.termsHash,
            config.policyHash,
            verifierModule.ACCEPTANCE_CRITERIA_HASH(),
            config.benchmarkHash,
            config.evidenceSchemaHash
        );
        parentAddress = address(parent);
        isCanonicalParent[parentAddress] = true;
        settlementToken.safeTransferFrom(msg.sender, parentAddress, parent.TARGET_AMOUNT());
        parent.recordInitialFunding();
        emit StandingMetaParentCreated(bountyId, parentAddress, msg.sender);
    }

    function claimAndCreateChild(address parentAddress, ClaimAndCreateChildRequest calldata request)
        external
        returns (address childAddress, bytes32 childBountyId)
    {
        require(isCanonicalParent[parentAddress], "parent not canonical");
        StandingMetaParentV4 parent = StandingMetaParentV4(parentAddress);
        require(parent.bountyStatus() == StandingMetaParentV4.Status.Claimable, "parent not claimable");
        require(msg.sender != parent.creator(), "parent creator cannot solve");
        address[] memory noVerifiers = new address[](0);
        address[] memory candidates = _eligibleChildSolvers(parent, msg.sender);
        require(candidates.length >= MINIMUM_CHILD_SOLVER_CANDIDATES, "child solver pool too small");
        bytes32 candidateHash = keccak256(abi.encode(candidates));
        PreparationContext memory context =
            _preparationContext(parentAddress, parent, request, noVerifiers, candidateHash);
        _validateTermsInput(request.terms, parent, context, request.childParams);
        bytes32 publishedTermsHash = termsRegistry.publishFor(msg.sender, request.canonicalTerms, request.terms);
        require(publishedTermsHash == context.childTermsHash, "terms hash drift");

        (childAddress, childBountyId) = childFactory.createBountyWithAuthorization(
            msg.sender,
            request.childParams,
            noVerifiers,
            CHILD_TARGET,
            request.childCreationNonce,
            request.childFundingAuthorization
        );
        require(childAddress == context.predictedChild, "child address drift");

        RoundData storage data = _rounds[parentAddress][context.parentRound];
        require(data.child == address(0), "round already prepared");
        data.child = childAddress;
        data.termsHash = context.childTermsHash;
        data.selectionCommitment = context.selectionCommitment;
        data.selectionRequestedAt = context.selectionRequestedAt;
        data.candidateHash = candidateHash;

        uint256 requestId =
            controller.requestSolverSortition(context.selectionCommitment, candidates, uint8(candidates.length));
        data.requestId = requestId;
        termsRegistry.bindSelection(context.childTermsHash, requestId, candidateHash);

        BondAuthorization calldata bond = request.parentBondAuthorization;
        parent.activatePreparedClaim(
            msg.sender,
            childAddress,
            context.childTermsHash,
            bond.validAfter,
            bond.validBefore,
            bond.nonce,
            bond.v,
            bond.r,
            bond.s
        );
        emit AtomicChildPrepared(
            parentAddress,
            context.parentRound,
            msg.sender,
            childAddress,
            context.childTermsHash,
            context.selectionCommitment,
            context.selectionRequestedAt,
            requestId,
            candidateHash
        );
        emit ChildSolverDrawRequested(parentAddress, context.parentRound, requestId, candidateHash, candidates.length);
    }

    function activateChildDraw(address parentAddress) external {
        StandingMetaParentV4 parent = StandingMetaParentV4(parentAddress);
        uint64 parentRound = parent.round();
        RoundData storage data = _rounds[parentAddress][parentRound];
        require(data.requestId != 0 && !data.rankingActivated, "draw activation unavailable");
        address[] memory ranked = controller.solverSortition().ranking(data.requestId);
        require(ranked.length >= MINIMUM_CHILD_SOLVER_CANDIDATES, "ranking unavailable");
        for (uint256 i = 0; i < ranked.length; i++) {
            _ranking[parentAddress][parentRound].push(ranked[i]);
        }
        data.rankingActivated = true;
        data.currentRank = 0;
        data.assignmentDeadline = uint64(block.timestamp) + ASSIGNMENT_WINDOW;
        emit ChildSolverAssigned(parentAddress, parentRound, ranked[0], 0, data.assignmentDeadline);
    }

    function claimChildAssignment(address parentAddress, BondAuthorization calldata bond) external {
        StandingMetaParentV4 parent = StandingMetaParentV4(parentAddress);
        uint64 parentRound = parent.round();
        RoundData storage data = _rounds[parentAddress][parentRound];
        require(data.rankingActivated && data.authorizedSolver == address(0), "assignment unavailable");
        require(block.timestamp <= data.assignmentDeadline, "assignment expired");
        require(_ranking[parentAddress][parentRound][data.currentRank] == msg.sender, "candidate not selected");
        require(AgentBounty(data.child).status() == CLAIMABLE_STATUS, "child not claimable");
        AgentBounty(data.child)
            .claimWithAuthorization(msg.sender, bond.validAfter, bond.validBefore, bond.nonce, bond.v, bond.r, bond.s);
        data.authorizedSolver = msg.sender;
        data.assignmentDeadline = 0;
        _authorized[parentAddress][parentRound][msg.sender] = true;
        emit ChildSolverClaimed(parentAddress, parentRound, msg.sender, data.child);
    }

    function promoteNonresponsiveChildSolver(address parentAddress) external {
        StandingMetaParentV4 parent = StandingMetaParentV4(parentAddress);
        uint64 parentRound = parent.round();
        RoundData storage data = _rounds[parentAddress][parentRound];
        require(
            data.rankingActivated && data.authorizedSolver == address(0) && data.assignmentDeadline != 0
                && block.timestamp > data.assignmentDeadline,
            "nonresponse promotion unavailable"
        );
        _promote(parentAddress, parentRound, data, keccak256("assignment-nonresponse"));
    }

    function promoteRejectedChildSolver(address parentAddress) external {
        StandingMetaParentV4 parent = StandingMetaParentV4(parentAddress);
        uint64 parentRound = parent.round();
        RoundData storage data = _rounds[parentAddress][parentRound];
        AgentBounty child = AgentBounty(data.child);
        require(
            data.authorizedSolver != address(0) && child.status() == CLAIMABLE_STATUS && child.solver() == address(0),
            "rejection promotion unavailable"
        );
        data.authorizedSolver = address(0);
        _promote(parentAddress, parentRound, data, keccak256("child-returned-claimable"));
    }

    function roundChild(address parent, uint64 parentRound) external view returns (address) {
        return _rounds[parent][parentRound].child;
    }

    function roundTermsHash(address parent, uint64 parentRound) external view returns (bytes32) {
        return _rounds[parent][parentRound].termsHash;
    }

    function roundSelection(address parent, uint64 parentRound)
        external
        view
        returns (bytes32 commitment, uint256 requestId, bytes32 candidateHash)
    {
        RoundData storage data = _rounds[parent][parentRound];
        return (data.selectionCommitment, data.requestId, data.candidateHash);
    }

    function roundTiming(address parent, uint64 parentRound)
        external
        view
        returns (uint64 selectionRequestedAt, uint64 assignmentDeadline, uint8 currentRank, bool rankingActivated)
    {
        RoundData storage data = _rounds[parent][parentRound];
        return (data.selectionRequestedAt, data.assignmentDeadline, data.currentRank, data.rankingActivated);
    }

    function authorizedChildSolver(address parent, uint64 parentRound, address child, address solver)
        external
        view
        returns (bool)
    {
        RoundData storage data = _rounds[parent][parentRound];
        return data.child == child && data.authorizedSolver == solver && _authorized[parent][parentRound][solver];
    }

    function solverRanking(address parent, uint64 parentRound) external view returns (address[] memory) {
        return _ranking[parent][parentRound];
    }

    function _validateChildParams(AgentBountyFactory.CreateBountyParams calldata params, bytes32 termsHash)
        private
        view
    {
        require(
            params.solverReward == CHILD_SOLVER_REWARD && params.verifierReward == CHILD_VERIFIER_REWARD
                && params.solverReward + params.verifierReward == CHILD_TARGET && params.termsHash == termsHash,
            "child economics invalid"
        );
        require(
            uint8(params.verificationMode) == DETERMINISTIC_MODE && params.verifierModule == address(appealableVerifier)
                && params.verifierRewardRecipient == address(appealableVerifier) && params.threshold == 1,
            "child appeal policy invalid"
        );
        require(
            params.claimWindowSeconds == CHILD_WORK_WINDOW
                && params.verificationWindowSeconds == CHILD_VERIFICATION_WINDOW
                && params.fundingDeadline > block.timestamp,
            "child timing invalid"
        );
    }

    function _validateTermsInput(
        OnchainTermsRegistryV4.TermsInput calldata terms,
        StandingMetaParentV4 parent,
        PreparationContext memory context,
        AgentBountyFactory.CreateBountyParams calldata params
    ) private view {
        require(
            terms.parent == address(parent) && terms.child == context.predictedChild
                && terms.parentBountyId == parent.bountyId() && terms.parentRound == context.parentRound
                && terms.selectionCommitment == context.selectionCommitment
                && terms.selectionRequestedAt == context.selectionRequestedAt,
            "terms binding invalid"
        );
        require(
            terms.verifierModule == address(appealableVerifier) && terms.policyHash == params.policyHash
                && terms.acceptanceCriteriaHash == params.acceptanceCriteriaHash
                && terms.benchmarkHash == params.benchmarkHash && terms.evidenceSchemaHash == params.evidenceSchemaHash
                && terms.appealPolicyHash == appealableVerifier.appealPolicyHash(),
            "terms content invalid"
        );
        require(
            terms.childClaimWindowSeconds == CHILD_WORK_WINDOW
                && terms.childVerificationWindowSeconds == CHILD_VERIFICATION_WINDOW
                && terms.childFundingTarget == CHILD_TARGET && terms.childSolverReward == CHILD_SOLVER_REWARD
                && terms.childVerifierReward == CHILD_VERIFIER_REWARD,
            "terms economics invalid"
        );
    }

    function _preparationContext(
        address parentAddress,
        StandingMetaParentV4 parent,
        ClaimAndCreateChildRequest calldata request,
        address[] memory noVerifiers,
        bytes32 candidateHash
    ) private view returns (PreparationContext memory context) {
        context.parentRound = parent.round() + 1;
        context.childTermsHash = keccak256(request.canonicalTerms);
        _validateChildParams(request.childParams, context.childTermsHash);
        context.predictedChild =
            childFactory.predictBountyAddress(msg.sender, request.childParams, noVerifiers, request.childCreationNonce);
        context.selectionRequestedAt = uint64(block.timestamp);
        context.selectionCommitment = keccak256(
            abi.encode(
                keccak256("agent-bounties/standing-meta-v4-child-solver-draw"),
                parentAddress,
                parent.bountyId(),
                context.parentRound,
                context.predictedChild,
                context.childTermsHash,
                context.selectionRequestedAt,
                candidateHash
            )
        );
    }

    function _eligibleChildSolvers(StandingMetaParentV4 parent, address parentSolver)
        private
        view
        returns (address[] memory)
    {
        address[] memory exclusions = new address[](2);
        exclusions[0] = parentSolver;
        exclusions[1] = parent.creator();
        return controller.eligibleSolverWallets(exclusions);
    }

    function _promote(address parentAddress, uint64 parentRound, RoundData storage data, bytes32 reason) private {
        uint256 next = uint256(data.currentRank) + 1;
        require(next < _ranking[parentAddress][parentRound].length, "solver ranking exhausted");
        data.currentRank = uint8(next);
        data.assignmentDeadline = uint64(block.timestamp) + ASSIGNMENT_WINDOW;
        address candidate = _ranking[parentAddress][parentRound][next];
        emit ChildSolverPromoted(parentAddress, parentRound, candidate, data.currentRank, reason);
        emit ChildSolverAssigned(parentAddress, parentRound, candidate, data.currentRank, data.assignmentDeadline);
    }
}
