// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";

interface IStandingMetaPreparedChildV4 {
    function status() external view returns (uint8);
}

/// @notice Exact-economics parent whose only claim path is the factory's atomic
/// child creation flow. Missing child evidence reverts and remains retryable.
contract StandingMetaParentV4 {
    using SafeBountyToken for address;

    enum Status {
        Open,
        Claimable,
        Claimed,
        Submitted,
        Settled,
        Cancelled
    }

    uint256 public constant SOLVER_REWARD = 2_000_000;
    uint256 public constant VERIFIER_REWARD = 10_000;
    uint256 public constant TARGET_AMOUNT = 2_010_000;
    uint64 public constant WORK_WINDOW = 14 days;
    uint64 public constant VERIFICATION_WINDOW = 24 hours;
    uint8 public constant CHILD_SETTLED_STATUS = 4;
    bytes32 public constant PROTOCOL_VERSION = keccak256("agent-bounties/standing-meta-v4");

    bytes32 public immutable bountyId;
    address public immutable creator;
    address public immutable factory;
    address public immutable settlementToken;
    address public immutable verifierModule;
    bytes32 public immutable termsHash;
    bytes32 public immutable policyHash;
    bytes32 public immutable acceptanceCriteriaHash;
    bytes32 public immutable benchmarkHash;
    bytes32 public immutable evidenceSchemaHash;

    Status private _status;
    uint256 public fundedAmount;
    uint64 public round;
    address public solver;
    address public preparedChild;
    bytes32 public preparedChildTermsHash;
    uint256 public activeClaimBond;
    uint64 public claimExpiresAt;
    uint64 public claimActivatedAt;
    uint64 public verificationExpiresAt;
    bytes32 public submissionHash;
    bytes32 public evidenceHash;
    bool private _entered;

    event ParentFunded(bytes32 indexed bountyId, address indexed creator, uint256 amount);
    event ParentClaimed(
        bytes32 indexed bountyId,
        uint64 indexed round,
        address indexed solver,
        address child,
        bytes32 childTermsHash,
        uint64 claimExpiresAt
    );
    event SubmissionAdded(
        bytes32 indexed bountyId,
        uint64 indexed round,
        address indexed solver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        uint64 verificationExpiresAt
    );
    event BountySettled(
        bytes32 indexed bountyId,
        uint64 indexed round,
        address indexed solver,
        uint256 solverReward,
        uint256 returnedBond,
        uint256 timeoutBonus,
        uint256 verifierReward,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 policyHash,
        bytes32 verificationHash
    );
    event ParentExpired(bytes32 indexed bountyId, uint64 indexed round, address indexed solver, uint256 refundedBond);
    event BountyCancelled(bytes32 indexed bountyId, uint256 refundAmount);
    event RefundWithdrawn(bytes32 indexed bountyId, address indexed contributor, uint256 amount);

    modifier nonReentrant() {
        require(!_entered, "reentrant");
        _entered = true;
        _;
        _entered = false;
    }

    modifier onlyFactory() {
        require(msg.sender == factory, "factory only");
        _;
    }

    constructor(
        bytes32 bountyId_,
        address creator_,
        address factory_,
        address settlementToken_,
        address verifierModule_,
        bytes32 termsHash_,
        bytes32 policyHash_,
        bytes32 acceptanceCriteriaHash_,
        bytes32 benchmarkHash_,
        bytes32 evidenceSchemaHash_
    ) {
        require(
            bountyId_ != bytes32(0) && creator_ != address(0) && factory_ != address(0)
                && settlementToken_ != address(0) && verifierModule_.code.length > 0,
            "parent config invalid"
        );
        require(
            termsHash_ != bytes32(0) && policyHash_ != bytes32(0) && acceptanceCriteriaHash_ != bytes32(0)
                && benchmarkHash_ != bytes32(0) && evidenceSchemaHash_ != bytes32(0),
            "parent commitment invalid"
        );
        bountyId = bountyId_;
        creator = creator_;
        factory = factory_;
        settlementToken = settlementToken_;
        verifierModule = verifierModule_;
        termsHash = termsHash_;
        policyHash = policyHash_;
        acceptanceCriteriaHash = acceptanceCriteriaHash_;
        benchmarkHash = benchmarkHash_;
        evidenceSchemaHash = evidenceSchemaHash_;
        _status = Status.Open;
    }

    function protocolVersion() external pure returns (bytes32) {
        return PROTOCOL_VERSION;
    }

    function solverReward() external pure returns (uint256) {
        return SOLVER_REWARD;
    }

    function verifierReward() external pure returns (uint256) {
        return VERIFIER_REWARD;
    }

    function targetAmount() external pure returns (uint256) {
        return TARGET_AMOUNT;
    }

    function claimWindowSeconds() external pure returns (uint64) {
        return WORK_WINDOW;
    }

    function verificationWindowSeconds() external pure returns (uint64) {
        return VERIFICATION_WINDOW;
    }

    function status() external view returns (uint8) {
        return uint8(_status);
    }

    function bountyStatus() external view returns (Status) {
        return _status;
    }

    function recordInitialFunding() external onlyFactory {
        require(_status == Status.Open && fundedAmount == 0, "already funded");
        require(IERC20BountyToken(settlementToken).balanceOf(address(this)) == TARGET_AMOUNT, "funding mismatch");
        fundedAmount = TARGET_AMOUNT;
        _status = Status.Claimable;
        emit ParentFunded(bountyId, creator, TARGET_AMOUNT);
    }

    function activatePreparedClaim(
        address solver_,
        address child,
        bytes32 childTermsHash,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 authorizationNonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external onlyFactory nonReentrant {
        require(_status == Status.Claimable, "parent not claimable");
        require(solver_ != address(0) && solver_ != creator && child != address(0), "claim parties invalid");
        require(childTermsHash != bytes32(0), "child terms missing");
        round += 1;
        solver = solver_;
        preparedChild = child;
        preparedChildTermsHash = childTermsHash;
        activeClaimBond = VERIFIER_REWARD;
        claimActivatedAt = uint64(block.timestamp);
        claimExpiresAt = uint64(block.timestamp) + WORK_WINDOW;
        _status = Status.Claimed;
        settlementToken.safeTransferWithAuthorization(
            solver_, address(this), VERIFIER_REWARD, validAfter, validBefore, authorizationNonce, v, r, s
        );
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) == TARGET_AMOUNT + VERIFIER_REWARD,
            "claim bond missing"
        );
        emit ParentClaimed(bountyId, round, solver_, child, childTermsHash, claimExpiresAt);
    }

    function submitChild(address child) external nonReentrant {
        require(_status == Status.Claimed && msg.sender == solver, "submission unauthorized");
        require(block.timestamp <= claimExpiresAt && child == preparedChild, "submission invalid");
        require(
            child.code.length > 0 && IStandingMetaPreparedChildV4(child).status() == CHILD_SETTLED_STATUS,
            "child not settled"
        );
        submissionHash = keccak256(abi.encode(child));
        evidenceHash = keccak256(abi.encode(preparedChildTermsHash, child));
        verificationExpiresAt = uint64(block.timestamp) + VERIFICATION_WINDOW;
        _status = Status.Submitted;
        emit SubmissionAdded(bountyId, round, solver, submissionHash, evidenceHash, verificationExpiresAt);
    }

    function verifyAndSettle() external nonReentrant {
        require(_status == Status.Submitted && block.timestamp <= verificationExpiresAt, "verification unavailable");
        bytes memory proof = abi.encode(preparedChild);
        (bool passed, bytes32 responseHash) = IAgentBountyVerifier(verifierModule)
            .verify(bountyId, round, solver, submissionHash, evidenceHash, policyHash, proof);
        require(passed, "parent predicate incomplete");
        uint256 returnedBond = activeClaimBond;
        activeClaimBond = 0;
        fundedAmount = 0;
        _status = Status.Settled;
        settlementToken.safeTransfer(solver, SOLVER_REWARD + returnedBond);
        settlementToken.safeTransfer(msg.sender, VERIFIER_REWARD);
        bytes32 verificationHash = keccak256(abi.encode(verifierModule, responseHash, keccak256(proof)));
        emit BountySettled(
            bountyId,
            round,
            solver,
            SOLVER_REWARD,
            returnedBond,
            0,
            VERIFIER_REWARD,
            submissionHash,
            evidenceHash,
            policyHash,
            verificationHash
        );
    }

    function expireWork() external nonReentrant {
        require(
            (_status == Status.Claimed && block.timestamp > claimExpiresAt)
                || (_status == Status.Submitted && block.timestamp > verificationExpiresAt),
            "work not expired"
        );
        address expiredSolver = solver;
        uint256 refundedBond = activeClaimBond;
        activeClaimBond = 0;
        solver = address(0);
        preparedChild = address(0);
        preparedChildTermsHash = bytes32(0);
        claimExpiresAt = 0;
        claimActivatedAt = 0;
        verificationExpiresAt = 0;
        submissionHash = bytes32(0);
        evidenceHash = bytes32(0);
        _status = Status.Claimable;
        settlementToken.safeTransfer(expiredSolver, refundedBond);
        emit ParentExpired(bountyId, round, expiredSolver, refundedBond);
    }

    function cancel() external nonReentrant {
        require(msg.sender == creator && _status == Status.Claimable, "cancellation unavailable");
        _status = Status.Cancelled;
        emit BountyCancelled(bountyId, fundedAmount);
    }

    function withdrawRefund() external nonReentrant {
        require(msg.sender == creator && _status == Status.Cancelled && fundedAmount > 0, "refund unavailable");
        uint256 amount = fundedAmount;
        fundedAmount = 0;
        settlementToken.safeTransfer(creator, amount);
        emit RefundWithdrawn(bountyId, creator, amount);
    }
}
