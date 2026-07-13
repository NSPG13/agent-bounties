// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./AgentBountyFactory.sol";

/// @notice A policy-enforcing USDC wallet for an autonomous agent.
/// @dev The delegate can interact only with canonical Agent Bounties contracts.
/// The owner retains withdrawal, policy, revocation, and ownership authority.
contract BoundedAgentWallet {
    using SafeBountyToken for address;

    enum Action {
        Create,
        Fund,
        Claim,
        Submit
    }

    struct Policy {
        address delegate;
        uint64 validAfter;
        uint64 validUntil;
        uint64 periodSeconds;
        uint256 maxPerAction;
        uint256 maxPerPeriod;
        uint256 maxLifetimeSpend;
        uint8 allowedActions;
        uint8 allowedVerificationModes;
    }

    uint8 public constant ACTION_CREATE = uint8(1) << uint8(Action.Create);
    uint8 public constant ACTION_FUND = uint8(1) << uint8(Action.Fund);
    uint8 public constant ACTION_CLAIM = uint8(1) << uint8(Action.Claim);
    uint8 public constant ACTION_SUBMIT = uint8(1) << uint8(Action.Submit);
    uint8 public constant ALL_ACTIONS = ACTION_CREATE | ACTION_FUND | ACTION_CLAIM | ACTION_SUBMIT;

    uint8 public constant MODE_DETERMINISTIC = uint8(1) << uint8(AgentBounty.VerificationMode.DeterministicModule);
    uint8 public constant MODE_SIGNED_QUORUM = uint8(1) << uint8(AgentBounty.VerificationMode.SignedQuorum);
    uint8 public constant MODE_AI_JUDGE_QUORUM = uint8(1) << uint8(AgentBounty.VerificationMode.AiJudgeQuorum);
    uint8 public constant ALL_VERIFICATION_MODES = MODE_DETERMINISTIC | MODE_SIGNED_QUORUM | MODE_AI_JUDGE_QUORUM;

    bytes4 private constant ERC1271_MAGIC_VALUE = 0x1626ba7e;
    uint256 private constant ERC1271_GAS_LIMIT = 200_000;
    uint256 private constant SECP256K1N_DIV_2 = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;
    bytes32 private constant EIP712_DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant NAME_HASH = keccak256("Agent Bounties Bounded Wallet");
    bytes32 private constant VERSION_HASH = keccak256("1");
    bytes32 private constant ACTION_TYPEHASH = keccak256(
        "AgentAction(address wallet,uint8 action,bytes32 payloadHash,uint256 nonce,uint256 deadline,uint64 policyVersion)"
    );

    AgentBountyFactory public immutable factory;
    address public immutable settlementToken;
    address public owner;
    address public pendingOwner;
    Policy public policy;
    uint64 public policyVersion;
    uint256 public delegateNonce;
    uint256 public periodBucket;
    uint256 public periodSpent;
    uint256 public lifetimeSpent;
    bool public revoked;
    uint256 private _reentrancy = 1;

    event PolicyConfigured(
        uint64 indexed version,
        address indexed delegate,
        uint8 allowedActions,
        uint8 allowedVerificationModes,
        uint64 validAfter,
        uint64 validUntil,
        uint64 periodSeconds,
        uint256 maxPerAction,
        uint256 maxPerPeriod,
        uint256 maxLifetimeSpend
    );
    event PolicyRevoked(uint64 indexed version, address indexed delegate);
    event SpendCharged(
        Action indexed action, uint256 amount, uint256 periodSpent, uint256 lifetimeSpent, uint256 periodBucket
    );
    event AgentActionExecuted(
        Action indexed action, address indexed delegate, address indexed relayer, uint256 nonce, bytes32 payloadHash
    );
    event OwnershipTransferStarted(address indexed owner, address indexed pendingOwner);
    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    event TokenWithdrawn(address indexed token, address indexed to, uint256 amount);
    event EthWithdrawn(address indexed to, uint256 amount);

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

    constructor(address owner_, address factory_, Policy memory initialPolicy) {
        require(owner_ != address(0), "owner zero");
        require(factory_.code.length > 0, "factory has no code");
        owner = owner_;
        factory = AgentBountyFactory(factory_);
        settlementToken = AgentBountyFactory(factory_).settlementToken();
        require(settlementToken.code.length > 0, "token has no code");
        _configurePolicy(initialPolicy);
    }

    receive() external payable {}

    function configurePolicy(Policy calldata nextPolicy) external onlyOwner {
        _configurePolicy(nextPolicy);
    }

    function revokePolicy() external onlyOwner {
        require(!revoked, "already revoked");
        revoked = true;
        emit PolicyRevoked(policyVersion, policy.delegate);
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

    function createBounty(
        AgentBountyFactory.CreateBountyParams calldata params,
        address[] calldata verifiers,
        uint256 initialFunding,
        bytes32 creationNonce
    ) external nonReentrant returns (address bounty, bytes32 bountyId) {
        _requireDirectDelegate(Action.Create);
        bytes memory payload = abi.encode(params, verifiers, initialFunding, creationNonce);
        uint256 nonce = _consumeDirectNonce();
        (bounty, bountyId) = _createBounty(params, verifiers, initialFunding, creationNonce);
        emit AgentActionExecuted(Action.Create, policy.delegate, msg.sender, nonce, keccak256(payload));
    }

    function fundBounty(address bountyAddress, uint256 requestedAmount)
        external
        nonReentrant
        returns (uint256 acceptedAmount)
    {
        _requireDirectDelegate(Action.Fund);
        bytes memory payload = abi.encode(bountyAddress, requestedAmount);
        uint256 nonce = _consumeDirectNonce();
        acceptedAmount = _fundBounty(bountyAddress, requestedAmount);
        emit AgentActionExecuted(Action.Fund, policy.delegate, msg.sender, nonce, keccak256(payload));
    }

    function claimBounty(address bountyAddress) external nonReentrant {
        _requireDirectDelegate(Action.Claim);
        bytes memory payload = abi.encode(bountyAddress);
        uint256 nonce = _consumeDirectNonce();
        _claimBounty(bountyAddress);
        emit AgentActionExecuted(Action.Claim, policy.delegate, msg.sender, nonce, keccak256(payload));
    }

    function submitBounty(address bountyAddress, bytes32 submissionHash, bytes32 evidenceHash) external nonReentrant {
        _requireDirectDelegate(Action.Submit);
        bytes memory payload = abi.encode(bountyAddress, submissionHash, evidenceHash);
        uint256 nonce = _consumeDirectNonce();
        _submitBounty(bountyAddress, submissionHash, evidenceHash);
        emit AgentActionExecuted(Action.Submit, policy.delegate, msg.sender, nonce, keccak256(payload));
    }

    /// @notice Any gas sponsor may relay one exact action signed by the active delegate.
    /// @dev Payload formats are documented by action and cannot encode arbitrary calls.
    function executeWithSignature(
        Action action,
        bytes calldata payload,
        uint256 nonce,
        uint256 deadline,
        bytes calldata signature
    ) external nonReentrant returns (bytes memory result) {
        _requireActivePolicy(action);
        require(block.timestamp <= deadline, "action signature expired");
        require(nonce == delegateNonce, "bad delegate nonce");
        bytes32 payloadHash = keccak256(payload);
        bytes32 digest = actionDigest(action, payloadHash, nonce, deadline);
        require(_isValidSignatureNow(policy.delegate, digest, signature), "invalid delegate signature");
        delegateNonce = nonce + 1;
        result = _dispatch(action, payload);
        emit AgentActionExecuted(action, policy.delegate, msg.sender, nonce, payloadHash);
    }

    function actionDigest(Action action, bytes32 payloadHash, uint256 nonce, uint256 deadline)
        public
        view
        returns (bytes32)
    {
        bytes32 structHash = keccak256(
            abi.encode(ACTION_TYPEHASH, address(this), uint8(action), payloadHash, nonce, deadline, policyVersion)
        );
        bytes32 domainSeparator =
            keccak256(abi.encode(EIP712_DOMAIN_TYPEHASH, NAME_HASH, VERSION_HASH, block.chainid, address(this)));
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator, structHash));
    }

    function withdrawToken(address token, address to, uint256 amount) external onlyOwner nonReentrant {
        require(token.code.length > 0, "token has no code");
        require(to != address(0), "recipient zero");
        require(amount > 0, "amount zero");
        token.safeTransfer(to, amount);
        emit TokenWithdrawn(token, to, amount);
    }

    function withdrawEth(address payable to, uint256 amount) external onlyOwner nonReentrant {
        require(to != address(0), "recipient zero");
        require(amount > 0 && amount <= address(this).balance, "bad amount");
        (bool ok,) = to.call{value: amount}("");
        require(ok, "eth transfer failed");
        emit EthWithdrawn(to, amount);
    }

    function _configurePolicy(Policy memory nextPolicy) private {
        require(nextPolicy.delegate != address(0), "delegate zero");
        require(nextPolicy.validUntil > nextPolicy.validAfter, "bad validity window");
        require(nextPolicy.validUntil > block.timestamp, "policy already expired");
        require(nextPolicy.allowedActions != 0, "no actions allowed");
        require((nextPolicy.allowedActions & ~ALL_ACTIONS) == 0, "unknown action");
        require((nextPolicy.allowedVerificationModes & ~ALL_VERIFICATION_MODES) == 0, "unknown verification mode");
        require(nextPolicy.allowedVerificationModes != 0, "no verification mode allowed");
        uint8 spendingActions = ACTION_CREATE | ACTION_FUND | ACTION_CLAIM;
        if ((nextPolicy.allowedActions & spendingActions) != 0) {
            require(nextPolicy.periodSeconds > 0, "period zero");
            require(nextPolicy.maxPerAction > 0, "action cap zero");
            require(nextPolicy.maxPerPeriod > 0, "period cap zero");
            require(nextPolicy.maxLifetimeSpend >= lifetimeSpent, "lifetime cap below spent");
            require(nextPolicy.maxLifetimeSpend > 0, "lifetime cap zero");
        }
        policy = nextPolicy;
        policyVersion += 1;
        revoked = false;
        if (nextPolicy.periodSeconds > 0) {
            periodBucket = block.timestamp / nextPolicy.periodSeconds;
            periodSpent = 0;
        }
        emit PolicyConfigured(
            policyVersion,
            nextPolicy.delegate,
            nextPolicy.allowedActions,
            nextPolicy.allowedVerificationModes,
            nextPolicy.validAfter,
            nextPolicy.validUntil,
            nextPolicy.periodSeconds,
            nextPolicy.maxPerAction,
            nextPolicy.maxPerPeriod,
            nextPolicy.maxLifetimeSpend
        );
    }

    function _requireDirectDelegate(Action action) private view {
        require(msg.sender == policy.delegate, "not delegate");
        _requireActivePolicy(action);
    }

    function _consumeDirectNonce() private returns (uint256 nonce) {
        nonce = delegateNonce;
        delegateNonce = nonce + 1;
    }

    function _requireActivePolicy(Action action) private view {
        require(!revoked, "policy revoked");
        require(block.timestamp >= policy.validAfter, "policy not active");
        require(block.timestamp <= policy.validUntil, "policy expired");
        require((policy.allowedActions & (uint8(1) << uint8(action))) != 0, "action not allowed");
    }

    function _dispatch(Action action, bytes calldata payload) private returns (bytes memory result) {
        if (action == Action.Create) {
            (
                AgentBountyFactory.CreateBountyParams memory params,
                address[] memory verifiers,
                uint256 initialFunding,
                bytes32 creationNonce
            ) = abi.decode(payload, (AgentBountyFactory.CreateBountyParams, address[], uint256, bytes32));
            (address bounty, bytes32 bountyId) = _createBounty(params, verifiers, initialFunding, creationNonce);
            return abi.encode(bounty, bountyId);
        }
        if (action == Action.Fund) {
            (address fundTarget, uint256 requestedAmount) = abi.decode(payload, (address, uint256));
            return abi.encode(_fundBounty(fundTarget, requestedAmount));
        }
        if (action == Action.Claim) {
            address claimTarget = abi.decode(payload, (address));
            _claimBounty(claimTarget);
            return bytes("");
        }
        (address bountyAddress, bytes32 submissionHash, bytes32 evidenceHash) =
            abi.decode(payload, (address, bytes32, bytes32));
        _submitBounty(bountyAddress, submissionHash, evidenceHash);
        return bytes("");
    }

    function _createBounty(
        AgentBountyFactory.CreateBountyParams memory params,
        address[] memory verifiers,
        uint256 initialFunding,
        bytes32 creationNonce
    ) private returns (address bounty, bytes32 bountyId) {
        _requireVerificationMode(params.verificationMode);
        uint256 target = params.solverReward + params.verifierReward;
        require(target >= params.solverReward && initialFunding <= target, "bad initial funding");
        _chargeSpend(Action.Create, initialFunding);
        if (initialFunding > 0) _approveExact(address(factory), initialFunding);
        (bounty, bountyId) = factory.createBounty(params, verifiers, initialFunding, creationNonce);
        if (initialFunding > 0) _approveExact(address(factory), 0);
    }

    function _fundBounty(address bountyAddress, uint256 requestedAmount) private returns (uint256 acceptedAmount) {
        AgentBounty bounty = _canonicalBounty(bountyAddress);
        _requireVerificationMode(bounty.verificationMode());
        require(requestedAmount > 0, "amount zero");
        uint256 remaining = bounty.targetAmount() - bounty.fundedAmount();
        require(remaining > 0, "bounty fully funded");
        acceptedAmount = requestedAmount < remaining ? requestedAmount : remaining;
        _chargeSpend(Action.Fund, acceptedAmount);
        _approveExact(bountyAddress, acceptedAmount);
        require(bounty.fund(requestedAmount) == acceptedAmount, "accepted amount changed");
        _approveExact(bountyAddress, 0);
    }

    function _claimBounty(address bountyAddress) private {
        AgentBounty bounty = _canonicalBounty(bountyAddress);
        _requireVerificationMode(bounty.verificationMode());
        uint256 bond = bounty.verifierReward();
        _chargeSpend(Action.Claim, bond);
        _approveExact(bountyAddress, bond);
        bounty.claim();
        _approveExact(bountyAddress, 0);
    }

    function _submitBounty(address bountyAddress, bytes32 submissionHash, bytes32 evidenceHash) private {
        AgentBounty bounty = _canonicalBounty(bountyAddress);
        _requireVerificationMode(bounty.verificationMode());
        bounty.submit(submissionHash, evidenceHash);
    }

    function _canonicalBounty(address bountyAddress) private view returns (AgentBounty bounty) {
        require(factory.isCanonicalBounty(bountyAddress), "not canonical bounty");
        bounty = AgentBounty(bountyAddress);
        require(bounty.factory() == address(factory), "wrong bounty factory");
        require(bounty.settlementToken() == settlementToken, "wrong settlement token");
    }

    function _requireVerificationMode(AgentBounty.VerificationMode mode) private view {
        require((policy.allowedVerificationModes & (uint8(1) << uint8(mode))) != 0, "verification mode not allowed");
    }

    function _chargeSpend(Action action, uint256 amount) private {
        if (amount == 0) return;
        require(amount <= policy.maxPerAction, "per-action cap exceeded");
        uint256 bucket = block.timestamp / policy.periodSeconds;
        if (bucket != periodBucket) {
            periodBucket = bucket;
            periodSpent = 0;
        }
        require(periodSpent + amount <= policy.maxPerPeriod, "period cap exceeded");
        require(lifetimeSpent + amount <= policy.maxLifetimeSpend, "lifetime cap exceeded");
        periodSpent += amount;
        lifetimeSpent += amount;
        emit SpendCharged(action, amount, periodSpent, lifetimeSpent, bucket);
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

    function _isValidSignatureNow(address signer, bytes32 digest, bytes memory signature) private view returns (bool) {
        if (signer.code.length > 0) {
            bytes memory callData = abi.encodeCall(IERC1271.isValidSignature, (digest, signature));
            bool ok;
            bytes4 result;
            uint256 gasLimit = ERC1271_GAS_LIMIT;
            assembly ("memory-safe") {
                let output := mload(0x40)
                mstore(output, 0)
                ok := staticcall(gasLimit, signer, add(callData, 0x20), mload(callData), output, 0x20)
                result := mload(output)
            }
            return ok && result == ERC1271_MAGIC_VALUE;
        }
        return _recover(digest, signature) == signer;
    }

    function _recover(bytes32 digest, bytes memory signature) private pure returns (address recovered) {
        if (signature.length != 65) return address(0);
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly {
            r := mload(add(signature, 0x20))
            s := mload(add(signature, 0x40))
            v := byte(0, mload(add(signature, 0x60)))
        }
        if (uint256(s) > SECP256K1N_DIV_2 || (v != 27 && v != 28)) return address(0);
        recovered = ecrecover(digest, v, r, s);
    }
}
