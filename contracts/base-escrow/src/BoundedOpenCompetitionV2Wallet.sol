// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./OpenCompetitionBountyFactoryV2Beta3.sol";

interface IERC20OpenCompetitionReserve is IERC20BountyToken {
    function allowance(address owner, address spender) external view returns (uint256);
}

/// @notice Owner-recoverable USDC reserve for reviewed GMV meta-competitions.
/// @dev The delegate can create only exact, preapproved competition configurations.
/// The owner can revoke the delegate and recover every uncommitted token without
/// cooperation from the delegate, relayer, hosted API, or wallet provider.
contract BoundedOpenCompetitionV2Wallet {
    using SafeBountyToken for address;

    struct Policy {
        address delegate;
        uint64 validAfter;
        uint64 validUntil;
        uint64 periodSeconds;
        uint256 solverReward;
        uint256 keeperReward;
        uint256 exactFundingPerCompetition;
        uint256 maxPerPeriod;
        uint256 maxLifetimeSpend;
        bytes32 betaRiskHash;
        bytes32 gmvMetricProgramHash;
        bytes32 gmvJournalSchemaHash;
    }

    uint256 public constant MAX_APPROVED_CREATIONS = 64;

    OpenCompetitionBountyFactoryV2Beta3 public immutable competitionFactory;
    address public immutable settlementToken;
    address public immutable deploymentFactory;

    address public owner;
    address public pendingOwner;
    Policy public policy;
    uint64 public policyVersion;
    bytes32 public activePolicyHash;
    bytes32 public initialPolicyHash;
    uint256 public periodBucket;
    uint256 public periodSpent;
    uint256 public lifetimeSpent;
    bool public revoked;

    mapping(uint64 => mapping(bytes32 => bool)) private _approvedCreation;
    mapping(bytes32 => bool) public usedCreation;

    uint256 private _reentrancy = 1;
    bool private _initialized;

    event OwnershipTransferStarted(address indexed owner, address indexed pendingOwner);
    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    event PolicyConfigured(
        uint64 indexed version,
        address indexed delegate,
        bytes32 indexed policyHash,
        uint256 approvedCreationCount,
        uint256 exactFundingPerCompetition,
        uint256 maxPerPeriod,
        uint256 maxLifetimeSpend
    );
    event PolicyRevoked(uint64 indexed version, address indexed delegate);
    event CompetitionCreatedFromReserve(
        bytes32 indexed creationCommitment,
        bytes32 indexed bountyId,
        address indexed competition,
        uint256 amount,
        uint256 periodSpent,
        uint256 lifetimeSpent
    );
    event CompetitionRefundPulled(address indexed competition, uint256 amount);
    event UncommittedReserveRecovered(address indexed owner, uint256 amount);
    event NativeReserveRecovered(address indexed owner, uint256 amount);

    error ReserveAlreadyInitialized();
    error ReserveNotDeploymentFactory();
    error ReserveNotOwner();
    error ReserveNotPendingOwner();
    error ReserveNotDelegate();
    error ReservePolicyInactive();
    error ReservePolicyRevoked();
    error ReserveInvalidPolicy();
    error ReserveInvalidApprovedCreation();
    error ReserveCreationNotApproved();
    error ReserveCreationAlreadyUsed();
    error ReserveEconomicsMismatch();
    error ReserveRiskHashMismatch();
    error ReserveNotGmvMetaCompetition();
    error ReservePerPeriodCapExceeded();
    error ReserveLifetimeCapExceeded();
    error ReserveUnexpectedCompetition();
    error ReserveUnexpectedAllowance();
    error ReserveRecoveryRequiresRevocation();
    error ReserveNothingToRecover();
    error ReserveNativeTransferFailed();
    error ReserveReentrantCall();

    modifier onlyOwner() {
        if (msg.sender != owner) revert ReserveNotOwner();
        _;
    }

    modifier nonReentrant() {
        if (_reentrancy != 1) revert ReserveReentrantCall();
        _reentrancy = 2;
        _;
        _reentrancy = 1;
    }

    constructor(address competitionFactory_) {
        if (competitionFactory_.code.length == 0) revert ReserveInvalidPolicy();
        deploymentFactory = msg.sender;
        competitionFactory = OpenCompetitionBountyFactoryV2Beta3(competitionFactory_);
        settlementToken = OpenCompetitionBountyFactoryV2Beta3(competitionFactory_).settlementToken();
        if (settlementToken.code.length == 0) revert ReserveInvalidPolicy();
        _initialized = true;
    }

    receive() external payable {}

    function initialize(address owner_, Policy calldata initialPolicy, bytes32[] calldata approvedCreations) external {
        if (msg.sender != deploymentFactory) revert ReserveNotDeploymentFactory();
        if (_initialized) revert ReserveAlreadyInitialized();
        if (owner_ == address(0)) revert ReserveInvalidPolicy();
        _initialized = true;
        _reentrancy = 1;
        owner = owner_;
        emit OwnershipTransferred(address(0), owner_);
        _configurePolicy(initialPolicy, approvedCreations);
        initialPolicyHash = activePolicyHash;
    }

    function configurePolicy(Policy calldata nextPolicy, bytes32[] calldata approvedCreations)
        external
        onlyOwner
        nonReentrant
    {
        _configurePolicy(nextPolicy, approvedCreations);
    }

    function revokePolicy() external onlyOwner {
        if (revoked) revert ReservePolicyRevoked();
        revoked = true;
        emit PolicyRevoked(policyVersion, policy.delegate);
    }

    function transferOwnership(address nextOwner) external onlyOwner {
        if (nextOwner == address(0)) revert ReserveInvalidPolicy();
        pendingOwner = nextOwner;
        emit OwnershipTransferStarted(owner, nextOwner);
    }

    function acceptOwnership() external {
        if (msg.sender != pendingOwner) revert ReserveNotPendingOwner();
        address previousOwner = owner;
        owner = msg.sender;
        pendingOwner = address(0);
        emit OwnershipTransferred(previousOwner, msg.sender);
    }

    function policyHashFor(Policy calldata candidate, bytes32[] calldata approvedCreations)
        public
        pure
        returns (bytes32)
    {
        return keccak256(abi.encode(candidate, approvedCreations));
    }

    function creationCommitment(
        OpenCompetitionBountyFactoryV2Beta3.CreateCompetitionParams calldata params,
        bytes32 creationNonce
    ) public view returns (bytes32) {
        return keccak256(abi.encode(block.chainid, address(competitionFactory), params, creationNonce));
    }

    function isApprovedCreation(bytes32 commitment) external view returns (bool) {
        return _approvedCreation[policyVersion][commitment];
    }

    function createCompetition(
        OpenCompetitionBountyFactoryV2Beta3.CreateCompetitionParams calldata params,
        bytes32 creationNonce
    ) external nonReentrant returns (address competition, bytes32 bountyId) {
        _requireActiveDelegate();
        if (
            params.solverReward != policy.solverReward || params.keeperReward != policy.keeperReward
                || params.solverReward + params.keeperReward != policy.exactFundingPerCompetition
        ) revert ReserveEconomicsMismatch();
        if (params.betaRiskHash != policy.betaRiskHash) revert ReserveRiskHashMismatch();
        if (
            params.winnerMode != OpenCompetitionBountyV2Beta3.WinnerMode.BestScore
                || params.scoreDirection != OpenCompetitionBountyV2Beta3.ScoreDirection.HigherIsBetter
                || params.scoreThreshold <= 0 || params.metricProgramHash != policy.gmvMetricProgramHash
                || params.journalSchemaHash != policy.gmvJournalSchemaHash
        ) revert ReserveNotGmvMetaCompetition();

        bytes32 commitment = creationCommitment(params, creationNonce);
        if (!_approvedCreation[policyVersion][commitment]) revert ReserveCreationNotApproved();
        if (usedCreation[commitment]) revert ReserveCreationAlreadyUsed();

        _chargeSpend(policy.exactFundingPerCompetition);
        usedCreation[commitment] = true;

        address predicted = competitionFactory.predictCompetitionAddress(address(this), params, creationNonce);
        if (predicted.code.length != 0) revert ReserveUnexpectedCompetition();
        _approveExact(predicted, policy.exactFundingPerCompetition);
        (competition, bountyId) = competitionFactory.createCompetition(
            params, policy.exactFundingPerCompetition, creationNonce, policy.betaRiskHash
        );
        _approveExact(predicted, 0);

        OpenCompetitionBountyV2Beta3 created = OpenCompetitionBountyV2Beta3(competition);
        if (
            competition != predicted || !competitionFactory.isCanonicalCompetition(competition)
                || created.creator() != address(this) || created.settlementToken() != settlementToken
                || created.targetAmount() != policy.exactFundingPerCompetition
                || created.fundedAmount() != policy.exactFundingPerCompetition
                || created.status() != uint8(OpenCompetitionBountyV2Beta3.CompetitionStatus.Active)
        ) revert ReserveUnexpectedCompetition();
        if (IERC20OpenCompetitionReserve(settlementToken).allowance(address(this), predicted) != 0) {
            revert ReserveUnexpectedAllowance();
        }

        emit CompetitionCreatedFromReserve(
            commitment, bountyId, competition, policy.exactFundingPerCompetition, periodSpent, lifetimeSpent
        );
    }

    /// @notice Stops relying on the delegate and returns every token not held by a competition.
    function recoverUncommitted() external onlyOwner nonReentrant returns (uint256 amount) {
        _requireRecoveryMode();
        amount = IERC20BountyToken(settlementToken).balanceOf(address(this));
        if (amount == 0) revert ReserveNothingToRecover();
        settlementToken.safeTransfer(owner, amount);
        emit UncommittedReserveRecovered(owner, amount);
    }

    function recoverNative() external onlyOwner nonReentrant returns (uint256 amount) {
        _requireRecoveryMode();
        amount = address(this).balance;
        if (amount == 0) revert ReserveNothingToRecover();
        (bool ok,) = payable(owner).call{value: amount}("");
        if (!ok) revert ReserveNativeTransferFailed();
        emit NativeReserveRecovered(owner, amount);
    }

    function cancelUnavailableAndPullRefund(address competition)
        external
        onlyOwner
        nonReentrant
        returns (uint256 amount)
    {
        _requireRecoveryMode();
        OpenCompetitionBountyV2Beta3 bounty = _ownedCanonicalCompetition(competition);
        bounty.cancelForUnavailableVerifier();
        amount = _pullRefund(bounty);
    }

    function expireAndPullRefund(address competition) external onlyOwner nonReentrant returns (uint256 amount) {
        _requireRecoveryMode();
        OpenCompetitionBountyV2Beta3 bounty = _ownedCanonicalCompetition(competition);
        bounty.expireCompetition();
        amount = _pullRefund(bounty);
    }

    function pullCancelledRefund(address competition) external onlyOwner nonReentrant returns (uint256 amount) {
        _requireRecoveryMode();
        amount = _pullRefund(_ownedCanonicalCompetition(competition));
    }

    function _configurePolicy(Policy calldata nextPolicy, bytes32[] calldata approvedCreations) private {
        if (
            nextPolicy.delegate == address(0) || nextPolicy.validUntil <= nextPolicy.validAfter
                || nextPolicy.validUntil <= block.timestamp || nextPolicy.periodSeconds == 0
                || nextPolicy.solverReward == 0 || nextPolicy.keeperReward == 0
                || nextPolicy.solverReward + nextPolicy.keeperReward != nextPolicy.exactFundingPerCompetition
                || nextPolicy.maxPerPeriod < nextPolicy.exactFundingPerCompetition
                || nextPolicy.maxLifetimeSpend < lifetimeSpent || nextPolicy.betaRiskHash == bytes32(0)
                || nextPolicy.gmvMetricProgramHash == bytes32(0) || nextPolicy.gmvJournalSchemaHash == bytes32(0)
                || approvedCreations.length == 0 || approvedCreations.length > MAX_APPROVED_CREATIONS
        ) revert ReserveInvalidPolicy();

        if (policyVersion > 0) {
            _syncPeriod();
            if (nextPolicy.periodSeconds != policy.periodSeconds) revert ReserveInvalidPolicy();
        } else {
            periodBucket = block.timestamp / nextPolicy.periodSeconds;
        }

        uint64 nextVersion = policyVersion + 1;
        for (uint256 index = 0; index < approvedCreations.length; index++) {
            bytes32 commitment = approvedCreations[index];
            if (commitment == bytes32(0) || _approvedCreation[nextVersion][commitment]) {
                revert ReserveInvalidApprovedCreation();
            }
            _approvedCreation[nextVersion][commitment] = true;
        }

        policy = nextPolicy;
        policyVersion = nextVersion;
        activePolicyHash = policyHashFor(nextPolicy, approvedCreations);
        revoked = false;
        emit PolicyConfigured(
            nextVersion,
            nextPolicy.delegate,
            activePolicyHash,
            approvedCreations.length,
            nextPolicy.exactFundingPerCompetition,
            nextPolicy.maxPerPeriod,
            nextPolicy.maxLifetimeSpend
        );
    }

    function _requireActiveDelegate() private view {
        if (msg.sender != policy.delegate) revert ReserveNotDelegate();
        if (revoked) revert ReservePolicyRevoked();
        if (block.timestamp < policy.validAfter || block.timestamp > policy.validUntil) {
            revert ReservePolicyInactive();
        }
    }

    function _chargeSpend(uint256 amount) private {
        _syncPeriod();
        if (periodSpent + amount > policy.maxPerPeriod) revert ReservePerPeriodCapExceeded();
        if (lifetimeSpent + amount > policy.maxLifetimeSpend) revert ReserveLifetimeCapExceeded();
        periodSpent += amount;
        lifetimeSpent += amount;
    }

    function _syncPeriod() private {
        uint256 currentBucket = block.timestamp / policy.periodSeconds;
        if (currentBucket != periodBucket) {
            periodBucket = currentBucket;
            periodSpent = 0;
        }
    }

    function _requireRecoveryMode() private view {
        if (!revoked) revert ReserveRecoveryRequiresRevocation();
    }

    function _ownedCanonicalCompetition(address competition)
        private
        view
        returns (OpenCompetitionBountyV2Beta3 bounty)
    {
        if (!competitionFactory.isCanonicalCompetition(competition)) {
            revert ReserveUnexpectedCompetition();
        }
        bounty = OpenCompetitionBountyV2Beta3(competition);
        if (bounty.creator() != address(this) || bounty.settlementToken() != settlementToken) {
            revert ReserveUnexpectedCompetition();
        }
    }

    function _pullRefund(OpenCompetitionBountyV2Beta3 bounty) private returns (uint256 amount) {
        uint256 beforeBalance = IERC20BountyToken(settlementToken).balanceOf(address(this));
        amount = bounty.withdrawRefundFor(address(this));
        uint256 afterBalance = IERC20BountyToken(settlementToken).balanceOf(address(this));
        if (amount == 0 || afterBalance != beforeBalance + amount) revert ReserveUnexpectedCompetition();
        emit CompetitionRefundPulled(address(bounty), amount);
    }

    function _approveExact(address spender, uint256 amount) private {
        (bool zeroOk, bytes memory zeroResult) =
            settlementToken.call(abi.encodeWithSignature("approve(address,uint256)", spender, 0));
        if (!zeroOk || (zeroResult.length > 0 && !abi.decode(zeroResult, (bool)))) {
            revert ReserveUnexpectedAllowance();
        }
        if (amount == 0) return;
        (bool ok, bytes memory result) =
            settlementToken.call(abi.encodeWithSignature("approve(address,uint256)", spender, amount));
        if (!ok || (result.length > 0 && !abi.decode(result, (bool)))) revert ReserveUnexpectedAllowance();
    }
}
