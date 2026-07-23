// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";

/// @notice Exact-economics V4 child whose claim authority is restricted to the
/// immutable parent factory. The factory can activate only the wallet selected
/// from the frozen VRF ranking, so generic direct claims cannot reserve or
/// drain the child outside the committed draw.
contract StandingMetaChildV4 {
    using SafeBountyToken for address;

    enum Status {
        Open,
        Claimable,
        Claimed,
        Submitted,
        Settled,
        Cancelled
    }

    uint256 public constant SOLVER_REWARD = 990_000;
    uint256 public constant VERIFIER_REWARD = 10_000;
    uint256 public constant TARGET_AMOUNT = 1_000_000;
    uint64 public constant CLAIM_WINDOW = 7 days;
    uint64 public constant VERIFICATION_WINDOW = 96 hours;
    uint8 public constant DETERMINISTIC_MODULE_MODE = 0;
    bytes32 public constant PROTOCOL_VERSION = keccak256("agent-bounties/standing-meta-child-v4");

    bytes32 public immutable bountyId;
    address public immutable creator;
    address public immutable factory;
    address public immutable settlementToken;
    address public immutable verifierModule;
    address public immutable verifierRewardRecipient;
    bytes32 public immutable termsHash;
    bytes32 public immutable policyHash;
    bytes32 public immutable acceptanceCriteriaHash;
    bytes32 public immutable benchmarkHash;
    bytes32 public immutable evidenceSchemaHash;

    Status private _status;
    uint256 public fundedAmount;
    uint256 public timeoutBondPool;
    uint64 public round;
    address public solver;
    uint256 public activeClaimBond;
    uint64 public claimExpiresAt;
    uint64 public verificationExpiresAt;
    bytes32 public submissionHash;
    bytes32 public evidenceHash;
    bool private _entered;

    event FundingAdded(
        bytes32 indexed bountyId,
        address indexed contributor,
        uint256 amount,
        uint256 fundedAmount,
        uint256 targetAmount
    );
    event BountyBecameClaimable(bytes32 indexed bountyId, uint256 fundedAmount);
    event BountyClaimed(
        bytes32 indexed bountyId,
        uint64 indexed round,
        address indexed solver,
        bytes32 termsHash,
        bytes32 policyHash,
        uint256 claimBond,
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
    event SubmissionRejected(
        bytes32 indexed bountyId,
        uint64 indexed round,
        address indexed solver,
        uint256 verifierReward,
        uint256 forfeitedBond,
        bytes32 verificationHash
    );
    event ClaimExpired(
        bytes32 indexed bountyId,
        uint64 indexed round,
        address indexed solver,
        uint256 forfeitedBond,
        uint256 timeoutBondPool
    );
    event SubmissionExpired(
        bytes32 indexed bountyId, uint64 indexed round, address indexed solver, uint256 refundedBond
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
    event BountyCancelled(bytes32 indexed bountyId, uint256 refundAmount);
    event RefundWithdrawn(bytes32 indexed bountyId, address indexed contributor, uint256 amount);

    modifier onlyFactory() {
        require(msg.sender == factory, "factory only");
        _;
    }

    modifier nonReentrant() {
        require(!_entered, "reentrant");
        _entered = true;
        _;
        _entered = false;
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
            "child config invalid"
        );
        require(
            termsHash_ != bytes32(0) && policyHash_ != bytes32(0) && acceptanceCriteriaHash_ != bytes32(0)
                && benchmarkHash_ != bytes32(0) && evidenceSchemaHash_ != bytes32(0),
            "child commitment invalid"
        );
        bountyId = bountyId_;
        creator = creator_;
        factory = factory_;
        settlementToken = settlementToken_;
        verifierModule = verifierModule_;
        verifierRewardRecipient = verifierModule_;
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
        return CLAIM_WINDOW;
    }

    function verificationWindowSeconds() external pure returns (uint64) {
        return VERIFICATION_WINDOW;
    }

    function verificationMode() external pure returns (uint8) {
        return DETERMINISTIC_MODULE_MODE;
    }

    function threshold() external pure returns (uint8) {
        return 1;
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
        emit FundingAdded(bountyId, creator, TARGET_AMOUNT, TARGET_AMOUNT, TARGET_AMOUNT);
        emit BountyBecameClaimable(bountyId, TARGET_AMOUNT);
    }

    function claimAuthorized(
        address solver_,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 authorizationNonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external onlyFactory nonReentrant {
        require(_status == Status.Claimable && activeClaimBond == 0, "child not claimable");
        require(solver_ != address(0) && solver_ != creator, "child solver invalid");
        activeClaimBond = VERIFIER_REWARD;
        settlementToken.safeTransferWithAuthorization(
            solver_, address(this), VERIFIER_REWARD, validAfter, validBefore, authorizationNonce, v, r, s
        );
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this))
                >= fundedAmount + timeoutBondPool + activeClaimBond,
            "claim bond missing"
        );
        round += 1;
        solver = solver_;
        claimExpiresAt = uint64(block.timestamp) + CLAIM_WINDOW;
        _status = Status.Claimed;
        emit BountyClaimed(bountyId, round, solver_, termsHash, policyHash, activeClaimBond, claimExpiresAt);
    }

    function submit(bytes32 submissionHash_, bytes32 evidenceHash_) external nonReentrant {
        require(_status == Status.Claimed && msg.sender == solver, "submission unauthorized");
        require(block.timestamp <= claimExpiresAt, "claim expired");
        require(submissionHash_ != bytes32(0) && evidenceHash_ != bytes32(0), "submission invalid");
        submissionHash = submissionHash_;
        evidenceHash = evidenceHash_;
        verificationExpiresAt = uint64(block.timestamp) + VERIFICATION_WINDOW;
        _status = Status.Submitted;
        emit SubmissionAdded(bountyId, round, solver, submissionHash_, evidenceHash_, verificationExpiresAt);
    }

    function verifyAndSettle(bytes calldata proof) external nonReentrant {
        require(_status == Status.Submitted && block.timestamp <= verificationExpiresAt, "verification unavailable");
        (bool passed, bytes32 responseHash) = IAgentBountyVerifier(verifierModule)
            .verify(bountyId, round, solver, submissionHash, evidenceHash, policyHash, proof);
        bytes32 verificationHash = keccak256(abi.encode(verifierModule, responseHash, keccak256(proof)));
        if (passed) _settle(verificationHash);
        else _reject(verificationHash);
    }

    function expireClaim() external nonReentrant {
        require(_status == Status.Claimed && block.timestamp > claimExpiresAt, "claim not expired");
        address expiredSolver = solver;
        uint256 forfeitedBond = activeClaimBond;
        activeClaimBond = 0;
        timeoutBondPool += forfeitedBond;
        _resetClaim();
        emit ClaimExpired(bountyId, round, expiredSolver, forfeitedBond, timeoutBondPool);
    }

    function expireSubmission() external nonReentrant {
        require(_status == Status.Submitted && block.timestamp > verificationExpiresAt, "submission not expired");
        address expiredSolver = solver;
        uint256 refundedBond = activeClaimBond;
        activeClaimBond = 0;
        _resetClaim();
        settlementToken.safeTransfer(expiredSolver, refundedBond);
        emit SubmissionExpired(bountyId, round, expiredSolver, refundedBond);
    }

    function cancel() external nonReentrant {
        require(msg.sender == creator && (_status == Status.Open || _status == Status.Claimable), "not cancellable");
        _status = Status.Cancelled;
        emit BountyCancelled(bountyId, fundedAmount + timeoutBondPool);
    }

    function withdrawRefund() external nonReentrant {
        require(msg.sender == creator && _status == Status.Cancelled && fundedAmount > 0, "refund unavailable");
        uint256 amount = fundedAmount + timeoutBondPool;
        fundedAmount = 0;
        timeoutBondPool = 0;
        settlementToken.safeTransfer(creator, amount);
        emit RefundWithdrawn(bountyId, creator, amount);
    }

    function _settle(bytes32 verificationHash) private {
        require(fundedAmount == TARGET_AMOUNT, "not fully funded");
        uint256 returnedBond = activeClaimBond;
        uint256 timeoutBonus = timeoutBondPool;
        activeClaimBond = 0;
        timeoutBondPool = 0;
        fundedAmount = 0;
        _status = Status.Settled;
        settlementToken.safeTransfer(solver, SOLVER_REWARD + returnedBond + timeoutBonus);
        settlementToken.safeTransfer(verifierRewardRecipient, VERIFIER_REWARD);
        emit BountySettled(
            bountyId,
            round,
            solver,
            SOLVER_REWARD,
            returnedBond,
            timeoutBonus,
            VERIFIER_REWARD,
            submissionHash,
            evidenceHash,
            policyHash,
            verificationHash
        );
    }

    function _reject(bytes32 verificationHash) private {
        address rejectedSolver = solver;
        uint256 forfeitedBond = activeClaimBond;
        require(forfeitedBond == VERIFIER_REWARD, "claim bond invariant");
        activeClaimBond = 0;
        _resetClaim();
        settlementToken.safeTransfer(verifierRewardRecipient, VERIFIER_REWARD);
        emit SubmissionRejected(bountyId, round, rejectedSolver, VERIFIER_REWARD, forfeitedBond, verificationHash);
    }

    function _resetClaim() private {
        solver = address(0);
        claimExpiresAt = 0;
        verificationExpiresAt = 0;
        submissionHash = bytes32(0);
        evidenceHash = bytes32(0);
        _status = Status.Claimable;
    }
}
