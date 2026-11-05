// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./AgentBountyFactoryV2.sol";

/// @notice A policy-enforcing USDC treasury whose delegate may only create fully funded
/// canonical autonomous-v2 bounties and cancel bounties this wallet created.
/// @dev The delegate is an automated signer. It cannot transfer tokens, call other contracts,
/// claim, submit, settle or change the policy. Cancellation refunds always return to this wallet.
/// The owner keeps withdrawal, policy, revocation and ownership authority. Recovered refunds never
/// restore spending authority; only an owner policy replacement can raise the lifetime cap.
contract BoundedV2FundingWallet {
    using SafeBountyToken for address;

    bytes32 public constant WALLET_VERSION = keccak256("agent-bounties/bounded-v2-funding-wallet/v1");

    /// @dev Mirrors the bounds AgentBountyV2 enforces, so a policy cannot promise wider windows.
    uint64 public constant MAX_FUNDING_WINDOW = 366 days;
    uint64 public constant MAX_WORK_WINDOW = 30 days;
    uint8 public constant MAX_QUORUM_VERIFIERS = 8;
    uint8 public constant MAX_VERIFICATION_CONFIGS = 8;

    struct Policy {
        address delegate;
        uint64 validAfter;
        uint64 validUntil;
        uint64 periodSeconds;
        uint64 maxFundingWindowSeconds;
        uint64 maxClaimWindowSeconds;
        uint64 maxVerificationWindowSeconds;
        bool delegateMayCancel;
        uint256 maxPerBounty;
        uint256 maxPerPeriod;
        uint256 maxLifetimeSpend;
        address claimEligibilityRegistry;
        bytes32 claimEligibilitySource;
    }

    /// @notice One exact verification setup a created bounty may use.
    /// @dev Deterministic: module and reward recipient set, no verifier set, threshold one.
    /// Quorum: `verifierSetHash == keccak256(abi.encode(verifiers))`, no module or recipient.
    struct VerificationConfig {
        AgentBountyV2.VerificationMode mode;
        address verifierModule;
        address verifierRewardRecipient;
        bytes32 verifierSetHash;
        uint8 threshold;
    }

    AgentBountyFactoryV2 public immutable factory;
    address public immutable settlementToken;
    address public owner;
    address public pendingOwner;
    uint64 public policyVersion;
    bool public revoked;
    uint256 public periodBucket;
    uint256 public periodSpent;
    uint256 public lifetimeSpent;
    mapping(address => bool) public isWalletBounty;
    Policy private _policy;
    VerificationConfig[] private _verificationConfigs;
    uint256 private _reentrancy = 1;

    event PolicyConfigured(
        uint64 indexed version, address indexed delegate, bytes32 policyHash, uint256 verificationConfigCount
    );
    event PolicyRevoked(uint64 indexed version, address indexed delegate);
    event SpendCharged(uint256 amount, uint256 periodSpent, uint256 lifetimeSpent, uint256 periodBucket);
    event WalletBountyCreated(
        address indexed bounty,
        bytes32 indexed bountyId,
        bytes32 creationNonce,
        uint256 targetAmount,
        uint64 policyVersion
    );
    event WalletBountyCancelRequested(address indexed bounty, bytes32 indexed bountyId, address indexed caller);
    event WalletBountyRefundRecovered(
        address indexed bounty, bytes32 indexed bountyId, uint256 principal, uint256 amount, uint256 walletBalance
    );
    event OwnershipTransferStarted(address indexed owner, address indexed pendingOwner);
    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    event TokenWithdrawn(address indexed token, address indexed to, uint256 amount);

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    modifier nonReentrant() {
        require(_reentrancy == 1, "reentrant");
        _reentrancy = 2;
        _;
        _reentrancy = 1;
    }

    constructor(
        address factory_,
        address owner_,
        Policy memory initialPolicy,
        VerificationConfig[] memory initialConfigs
    ) {
        require(factory_.code.length > 0, "factory has no code");
        require(owner_ != address(0), "owner zero");
        factory = AgentBountyFactoryV2(factory_);
        settlementToken = AgentBountyFactoryV2(factory_).settlementToken();
        require(settlementToken.code.length > 0, "token has no code");
        owner = owner_;
        emit OwnershipTransferred(address(0), owner_);
        _configurePolicy(initialPolicy, initialConfigs);
    }

    function policy() external view returns (Policy memory) {
        return _policy;
    }

    function verificationConfigs() external view returns (VerificationConfig[] memory) {
        return _verificationConfigs;
    }

    /// @notice Commits every policy field and every allowed verification setup, in order.
    function policyHash() public view returns (bytes32) {
        return keccak256(abi.encode(_policy, _verificationConfigs));
    }

    function configurePolicy(Policy calldata nextPolicy, VerificationConfig[] calldata nextConfigs) external onlyOwner {
        _configurePolicy(nextPolicy, nextConfigs);
    }

    /// @notice Changes only the three spend caps. Today's and lifetime spend, revocation and every
    /// other policy field stay as they are, so a lower cap applies at once to what was spent today.
    /// "No lifetime cap" is `type(uint256).max`.
    function setCaps(uint256 maxPerBounty, uint256 maxPerPeriod, uint256 maxLifetimeSpend) external onlyOwner {
        _requireValidCaps(maxPerBounty, maxPerPeriod, maxLifetimeSpend);
        _policy.maxPerBounty = maxPerBounty;
        _policy.maxPerPeriod = maxPerPeriod;
        _policy.maxLifetimeSpend = maxLifetimeSpend;
        policyVersion += 1;
        emit PolicyConfigured(policyVersion, _policy.delegate, policyHash(), _verificationConfigs.length);
    }

    function revokePolicy() external onlyOwner {
        require(!revoked, "already revoked");
        revoked = true;
        emit PolicyRevoked(policyVersion, _policy.delegate);
    }

    function transferOwnership(address nextOwner) external onlyOwner {
        require(nextOwner != address(0), "owner zero");
        pendingOwner = nextOwner;
        emit OwnershipTransferStarted(owner, nextOwner);
    }

    function acceptOwnership() external {
        require(msg.sender == pendingOwner, "not pending owner");
        address previousOwner = owner;
        owner = msg.sender;
        pendingOwner = address(0);
        emit OwnershipTransferred(previousOwner, msg.sender);
    }

    function withdrawToken(address token, address to, uint256 amount) external onlyOwner nonReentrant {
        require(token.code.length > 0, "token has no code");
        require(to != address(0), "recipient zero");
        require(amount > 0, "amount zero");
        token.safeTransfer(to, amount);
        emit TokenWithdrawn(token, to, amount);
    }

    /// @notice Create one canonical v2 bounty funded with its exact target, including the platform fee.
    function createBounty(
        AgentBountyFactoryV2.CreateBountyParams calldata params,
        address[] calldata verifiers,
        bytes32 creationNonce
    ) external nonReentrant returns (address bountyAddress, bytes32 bountyId, uint256 targetAmount) {
        _requireActiveDelegate();
        _requireCreationPolicy(params, verifiers);
        (, targetAmount) = factory.quote(params.solverReward, params.verifierReward);
        require(targetAmount <= _policy.maxPerBounty, "per-bounty cap exceeded");
        _chargeSpend(targetAmount);

        _approveExact(address(factory), targetAmount);
        (bountyAddress, bountyId) = factory.createBounty(params, verifiers, targetAmount, creationNonce);
        _approveExact(address(factory), 0);

        AgentBountyV2 bounty = AgentBountyV2(bountyAddress);
        require(
            bounty.creator() == address(this) && bounty.targetAmount() == targetAmount
                && bounty.contributions(address(this)) == targetAmount
                && bounty.bountyStatus() == AgentBountyV2.BountyStatus.Claimable,
            "bounty not fully funded"
        );
        isWalletBounty[bountyAddress] = true;
        emit WalletBountyCreated(bountyAddress, bountyId, creationNonce, targetAmount, policyVersion);
    }

    /// @notice Cancel a bounty this wallet created and pull its refund back here.
    /// @dev During an active round the bounty records a cancel request instead: the solver can still
    /// pass and be paid. Call again after that round ends to recover the refund. Anyone may also push
    /// this wallet's refund with `withdrawRefundFor`; it always pays this wallet.
    function cancelBounty(address bountyAddress) external nonReentrant returns (uint256 recovered) {
        if (msg.sender != owner) {
            _requireActiveDelegate();
            require(_policy.delegateMayCancel, "delegate may not cancel");
        }
        require(isWalletBounty[bountyAddress], "not wallet bounty");
        AgentBountyV2 bounty = AgentBountyV2(bountyAddress);
        if (bounty.bountyStatus() != AgentBountyV2.BountyStatus.Cancelled) {
            bounty.cancel();
            if (bounty.bountyStatus() != AgentBountyV2.BountyStatus.Cancelled) {
                emit WalletBountyCancelRequested(bountyAddress, bounty.bountyId(), msg.sender);
                return 0;
            }
        }
        uint256 principal = bounty.contributions(address(this));
        require(principal > 0, "refund already recovered");
        uint256 balanceBefore = IERC20BountyToken(settlementToken).balanceOf(address(this));
        bounty.withdrawRefund();
        uint256 balanceAfter = IERC20BountyToken(settlementToken).balanceOf(address(this));
        require(balanceAfter >= balanceBefore + principal, "refund not received");
        recovered = balanceAfter - balanceBefore;
        emit WalletBountyRefundRecovered(bountyAddress, bounty.bountyId(), principal, recovered, balanceAfter);
    }

    function _configurePolicy(Policy memory nextPolicy, VerificationConfig[] memory nextConfigs) private {
        require(nextPolicy.delegate != address(0), "delegate zero");
        require(nextPolicy.delegate != owner, "delegate is owner");
        require(nextPolicy.validUntil > nextPolicy.validAfter, "bad validity window");
        require(nextPolicy.validUntil > block.timestamp, "policy already expired");
        require(nextPolicy.periodSeconds > 0, "period zero");
        require(
            nextPolicy.maxFundingWindowSeconds > 0 && nextPolicy.maxFundingWindowSeconds <= MAX_FUNDING_WINDOW,
            "bad funding window cap"
        );
        require(
            nextPolicy.maxClaimWindowSeconds > 0 && nextPolicy.maxClaimWindowSeconds <= MAX_WORK_WINDOW,
            "bad claim window cap"
        );
        require(
            nextPolicy.maxVerificationWindowSeconds > 0 && nextPolicy.maxVerificationWindowSeconds <= MAX_WORK_WINDOW,
            "bad verification window cap"
        );
        _requireValidCaps(nextPolicy.maxPerBounty, nextPolicy.maxPerPeriod, nextPolicy.maxLifetimeSpend);
        require(
            (nextPolicy.claimEligibilityRegistry == address(0)) == (nextPolicy.claimEligibilitySource == bytes32(0)),
            "bad claim eligibility"
        );
        require(
            nextPolicy.claimEligibilityRegistry == address(0) || nextPolicy.claimEligibilityRegistry.code.length > 0,
            "eligibility registry has no code"
        );

        uint256 count = nextConfigs.length;
        require(count > 0 && count <= MAX_VERIFICATION_CONFIGS, "bad verification config count");
        delete _verificationConfigs;
        for (uint256 i = 0; i < count; i++) {
            VerificationConfig memory config = nextConfigs[i];
            _requireValidConfig(config);
            for (uint256 j = 0; j < i; j++) {
                require(!_sameConfig(nextConfigs[j], config), "duplicate verification config");
            }
            _verificationConfigs.push(config);
        }

        _policy = nextPolicy;
        policyVersion += 1;
        revoked = false;
        periodBucket = block.timestamp / nextPolicy.periodSeconds;
        periodSpent = 0;
        emit PolicyConfigured(policyVersion, nextPolicy.delegate, policyHash(), count);
    }

    function _requireValidCaps(uint256 maxPerBounty, uint256 maxPerPeriod, uint256 maxLifetimeSpend) private view {
        require(maxPerBounty > 0, "per-bounty cap zero");
        require(maxPerPeriod >= maxPerBounty, "period cap below per-bounty cap");
        require(maxLifetimeSpend >= maxPerPeriod, "lifetime cap below period cap");
        require(maxLifetimeSpend >= lifetimeSpent, "lifetime cap below spent");
    }

    function _requireValidConfig(VerificationConfig memory config) private view {
        if (config.mode == AgentBountyV2.VerificationMode.DeterministicModule) {
            require(config.verifierModule.code.length > 0, "deterministic verifier unavailable");
            require(config.verifierRewardRecipient != address(0), "verifier recipient zero");
            require(config.verifierSetHash == bytes32(0) && config.threshold == 1, "bad deterministic config");
            return;
        }
        require(
            config.verifierModule == address(0) && config.verifierRewardRecipient == address(0), "quorum module present"
        );
        require(config.verifierSetHash != bytes32(0), "verifier set zero");
        uint8 minimum = config.mode == AgentBountyV2.VerificationMode.AiJudgeQuorum ? 2 : 1;
        require(config.threshold >= minimum && config.threshold <= MAX_QUORUM_VERIFIERS, "bad quorum threshold");
    }

    function _requireActiveDelegate() private view {
        require(msg.sender == _policy.delegate, "not delegate");
        require(!revoked, "policy revoked");
        require(block.timestamp >= _policy.validAfter, "policy not active");
        require(block.timestamp <= _policy.validUntil, "policy expired");
    }

    function _requireCreationPolicy(
        AgentBountyFactoryV2.CreateBountyParams calldata params,
        address[] calldata verifiers
    ) private view {
        VerificationConfig memory requested = VerificationConfig({
            mode: params.verificationMode,
            verifierModule: params.verifierModule,
            verifierRewardRecipient: params.verifierRewardRecipient,
            verifierSetHash: verifiers.length == 0 ? bytes32(0) : keccak256(abi.encode(verifiers)),
            threshold: params.threshold
        });
        bool allowed = false;
        uint256 count = _verificationConfigs.length;
        for (uint256 i = 0; i < count && !allowed; i++) {
            allowed = _sameConfig(_verificationConfigs[i], requested);
        }
        require(allowed, "verification config not allowed");

        Policy storage p = _policy;
        require(
            params.claimEligibilityRegistry == p.claimEligibilityRegistry
                && params.claimEligibilitySource == p.claimEligibilitySource,
            "claim eligibility not allowed"
        );
        require(params.fundingDeadline <= block.timestamp + p.maxFundingWindowSeconds, "funding window too long");
        require(params.claimWindowSeconds <= p.maxClaimWindowSeconds, "claim window too long");
        require(params.verificationWindowSeconds <= p.maxVerificationWindowSeconds, "verification window too long");
    }

    function _sameConfig(VerificationConfig memory a, VerificationConfig memory b) private pure returns (bool) {
        return a.mode == b.mode && a.verifierModule == b.verifierModule
            && a.verifierRewardRecipient == b.verifierRewardRecipient && a.verifierSetHash == b.verifierSetHash
            && a.threshold == b.threshold;
    }

    function _chargeSpend(uint256 amount) private {
        uint256 bucket = block.timestamp / _policy.periodSeconds;
        if (bucket != periodBucket) {
            periodBucket = bucket;
            periodSpent = 0;
        }
        require(periodSpent + amount <= _policy.maxPerPeriod, "period cap exceeded");
        require(lifetimeSpent + amount <= _policy.maxLifetimeSpend, "lifetime cap exceeded");
        periodSpent += amount;
        lifetimeSpent += amount;
        emit SpendCharged(amount, periodSpent, lifetimeSpent, bucket);
    }

    function _approveExact(address spender, uint256 amount) private {
        (bool zeroOk, bytes memory zeroResult) =
            settlementToken.call(abi.encodeWithSignature("approve(address,uint256)", spender, 0));
        require(zeroOk && (zeroResult.length == 0 || abi.decode(zeroResult, (bool))), "approval reset failed");
        if (amount == 0) return;
        (bool ok, bytes memory result) =
            settlementToken.call(abi.encodeWithSignature("approve(address,uint256)", spender, amount));
        require(ok && (result.length == 0 || abi.decode(result, (bool))), "approval failed");
    }
}
