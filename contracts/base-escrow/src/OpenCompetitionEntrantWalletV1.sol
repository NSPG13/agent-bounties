// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./OpenCompetitionBountyFactoryV1.sol";

/// @notice A policy-capped entrant account for canonical Open Competition V1 bounties.
/// @dev A keeper may pay gas for exact EIP-712-authorized actions, but cannot choose
/// a bounty, proof, commitment, recipient, or token transfer on the wallet's behalf.
contract OpenCompetitionEntrantWalletV1 {
    using SafeBountyToken for address;

    enum Action {
        Commit,
        Reveal,
        WithdrawBond
    }

    enum ErrorCode {
        Unauthorized,
        Reentrant,
        InvalidInitialization,
        InvalidAddress,
        AlreadyRevoked,
        InvalidOwnershipTransfer,
        SignatureExpired,
        NonceMismatch,
        InvalidSignature,
        InvalidAmount,
        TransferFailed,
        InvalidPolicy,
        InactivePolicy,
        ActionNotAllowed,
        InvalidCompetition,
        CreatorControlled,
        VerifierProfileMismatch,
        SpendLimitExceeded,
        TokenOperationFailed
    }

    error WalletError(ErrorCode code);

    struct Policy {
        address delegate;
        uint64 validAfter;
        uint64 validUntil;
        uint64 periodSeconds;
        uint256 maxPerAction;
        uint256 maxPerPeriod;
        uint256 maxLifetimeSpend;
        uint256 maxBountyTarget;
        uint8 allowedActions;
        address verifierModule;
        bytes32 verifierRuntimeCodeHash;
        bytes32 verifierPolicyHash;
        bytes32 acceptanceCriteriaHash;
        bytes32 benchmarkHash;
        bytes32 evidenceSchemaHash;
    }

    uint8 public constant ACTION_COMMIT = uint8(1) << uint8(Action.Commit);
    uint8 public constant ACTION_REVEAL = uint8(1) << uint8(Action.Reveal);
    uint8 public constant ACTION_WITHDRAW_BOND = uint8(1) << uint8(Action.WithdrawBond);
    uint8 public constant ALL_ACTIONS = ACTION_COMMIT | ACTION_REVEAL | ACTION_WITHDRAW_BOND;

    bytes4 private constant ERC1271_MAGIC_VALUE = 0x1626ba7e;
    uint256 private constant ERC1271_GAS_LIMIT = 200_000;
    uint256 private constant SECP256K1N_DIV_2 = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;
    bytes32 private constant EIP712_DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant NAME_HASH = keccak256("Agent Bounties Open Competition Entrant Wallet");
    bytes32 private constant VERSION_HASH = keccak256("1");
    bytes32 private constant ACTION_TYPEHASH = keccak256(
        "OpenCompetitionEntrantAction(address wallet,uint8 action,bytes32 payloadHash,uint256 nonce,uint256 deadline,uint64 policyVersion)"
    );

    OpenCompetitionBountyFactoryV1 public immutable factory;
    address public immutable settlementToken;
    address public immutable deploymentFactory;
    address public owner;
    address public pendingOwner;
    Policy private _policy;
    uint64 public policyVersion;
    uint256 public delegateNonce;
    uint256 public periodBucket;
    uint256 public periodSpent;
    uint256 public lifetimeSpent;
    bool public revoked;
    uint256 private _reentrancy = 1;
    bool private _initialized;

    event PolicyConfigured(
        uint64 indexed version, address indexed delegate, uint8 allowedActions, bytes32 indexed policyHash
    );
    event PolicyRevoked(uint64 indexed version, address indexed delegate);
    event SpendCharged(uint256 amount, uint256 periodSpent, uint256 lifetimeSpent, uint256 periodBucket);
    event EntrantActionExecuted(
        Action indexed action, address indexed delegate, address indexed relayer, uint256 nonce, bytes32 payloadHash
    );
    event OwnershipTransferStarted(address indexed owner, address indexed pendingOwner);
    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    event TokenWithdrawn(address indexed token, address indexed to, uint256 amount);
    event EthWithdrawn(address indexed to, uint256 amount);
    event BondRecovered(address indexed bounty, address indexed owner);

    modifier onlyOwner() {
        if (msg.sender != owner) revert WalletError(ErrorCode.Unauthorized);
        _;
    }

    modifier nonReentrant() {
        if (_reentrancy != 1) revert WalletError(ErrorCode.Reentrant);
        _reentrancy = 2;
        _;
        _reentrancy = 1;
    }

    /// @dev Locks the implementation while clones retain independent storage.
    constructor(address factory_) {
        if (factory_.code.length == 0) revert WalletError(ErrorCode.InvalidAddress);
        deploymentFactory = msg.sender;
        factory = OpenCompetitionBountyFactoryV1(factory_);
        settlementToken = OpenCompetitionBountyFactoryV1(factory_).settlementToken();
        if (settlementToken.code.length == 0) revert WalletError(ErrorCode.InvalidAddress);
        _initialized = true;
    }

    function initialize(address owner_, Policy calldata initialPolicy) external {
        if (msg.sender != deploymentFactory) revert WalletError(ErrorCode.Unauthorized);
        if (_initialized) revert WalletError(ErrorCode.InvalidInitialization);
        if (owner_ == address(0)) revert WalletError(ErrorCode.InvalidAddress);
        _initialized = true;
        _reentrancy = 1;
        owner = owner_;
        emit OwnershipTransferred(address(0), owner_);
        _configurePolicy(initialPolicy);
    }

    receive() external payable {}

    function configurePolicy(Policy calldata nextPolicy) external onlyOwner {
        _configurePolicy(nextPolicy);
    }

    function revokePolicy() external onlyOwner {
        if (revoked) revert WalletError(ErrorCode.AlreadyRevoked);
        revoked = true;
        emit PolicyRevoked(policyVersion, _policy.delegate);
    }

    function transferOwnership(address nextOwner) external onlyOwner {
        if (nextOwner == address(0)) revert WalletError(ErrorCode.InvalidAddress);
        pendingOwner = nextOwner;
        emit OwnershipTransferStarted(owner, nextOwner);
    }

    function acceptOwnership() external {
        if (msg.sender != pendingOwner) revert WalletError(ErrorCode.InvalidOwnershipTransfer);
        address previousOwner = owner;
        owner = msg.sender;
        pendingOwner = address(0);
        emit OwnershipTransferred(previousOwner, msg.sender);
    }

    function commitSolution(address bountyAddress, bytes32 commitment) external nonReentrant {
        _requireDirectDelegate(Action.Commit);
        bytes memory payload = abi.encode(bountyAddress, commitment);
        uint256 nonce = _consumeDirectNonce();
        _commitSolution(bountyAddress, commitment);
        emit EntrantActionExecuted(Action.Commit, _policy.delegate, msg.sender, nonce, keccak256(payload));
    }

    function revealSolution(
        address bountyAddress,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 salt,
        bytes calldata proof
    ) external nonReentrant {
        _requireDirectDelegate(Action.Reveal);
        bytes memory payload = abi.encode(bountyAddress, submissionHash, evidenceHash, salt, proof);
        uint256 nonce = _consumeDirectNonce();
        _revealSolution(bountyAddress, submissionHash, evidenceHash, salt, proof);
        emit EntrantActionExecuted(Action.Reveal, _policy.delegate, msg.sender, nonce, keccak256(payload));
    }

    function withdrawEntryBond(address bountyAddress) external nonReentrant {
        _requireDirectDelegate(Action.WithdrawBond);
        bytes memory payload = abi.encode(bountyAddress);
        uint256 nonce = _consumeDirectNonce();
        _withdrawEntryBond(bountyAddress);
        emit EntrantActionExecuted(Action.WithdrawBond, _policy.delegate, msg.sender, nonce, keccak256(payload));
    }

    /// @notice Any keeper may relay one exact action signed by the active delegate.
    function executeWithSignature(
        Action action,
        bytes calldata payload,
        uint256 nonce,
        uint256 deadline,
        bytes calldata signature
    ) external nonReentrant returns (bytes memory result) {
        _requireActivePolicy(action);
        if (block.timestamp > deadline) revert WalletError(ErrorCode.SignatureExpired);
        if (nonce != delegateNonce) revert WalletError(ErrorCode.NonceMismatch);
        bytes32 payloadHash = keccak256(payload);
        bytes32 digest = actionDigest(action, payloadHash, nonce, deadline);
        if (!_isValidSignatureNow(_policy.delegate, digest, signature)) {
            revert WalletError(ErrorCode.InvalidSignature);
        }
        delegateNonce = nonce + 1;
        result = _dispatch(action, payload);
        emit EntrantActionExecuted(action, _policy.delegate, msg.sender, nonce, payloadHash);
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

    function policy() external view returns (Policy memory) {
        return _policy;
    }

    function policyHash() public view returns (bytes32) {
        return keccak256(abi.encode(_policy));
    }

    /// @notice Owner recovery does not depend on an active delegate policy.
    function recoverEntryBond(address bountyAddress) external onlyOwner nonReentrant {
        _withdrawEntryBond(bountyAddress);
        emit BondRecovered(bountyAddress, msg.sender);
    }

    function withdrawToken(address token, address to, uint256 amount) external onlyOwner nonReentrant {
        if (token.code.length == 0 || to == address(0)) revert WalletError(ErrorCode.InvalidAddress);
        if (amount == 0) revert WalletError(ErrorCode.InvalidAmount);
        token.safeTransfer(to, amount);
        emit TokenWithdrawn(token, to, amount);
    }

    function withdrawEth(address payable to, uint256 amount) external onlyOwner nonReentrant {
        if (to == address(0)) revert WalletError(ErrorCode.InvalidAddress);
        if (amount == 0 || amount > address(this).balance) revert WalletError(ErrorCode.InvalidAmount);
        (bool ok,) = to.call{value: amount}("");
        if (!ok) revert WalletError(ErrorCode.TransferFailed);
        emit EthWithdrawn(to, amount);
    }

    function _configurePolicy(Policy memory nextPolicy) private {
        if (
            nextPolicy.delegate == address(0) || nextPolicy.validUntil <= nextPolicy.validAfter
                || nextPolicy.validUntil <= block.timestamp || nextPolicy.allowedActions == 0
                || (nextPolicy.allowedActions & ~ALL_ACTIONS) != 0 || nextPolicy.maxBountyTarget == 0
                || nextPolicy.verifierModule.code.length == 0 || nextPolicy.verifierRuntimeCodeHash == bytes32(0)
                || nextPolicy.verifierModule.codehash != nextPolicy.verifierRuntimeCodeHash
                || nextPolicy.verifierPolicyHash == bytes32(0) || nextPolicy.acceptanceCriteriaHash == bytes32(0)
                || nextPolicy.benchmarkHash == bytes32(0) || nextPolicy.evidenceSchemaHash == bytes32(0)
        ) revert WalletError(ErrorCode.InvalidPolicy);
        if ((nextPolicy.allowedActions & ACTION_COMMIT) != 0) {
            if (
                nextPolicy.periodSeconds == 0 || nextPolicy.maxPerAction == 0 || nextPolicy.maxPerPeriod == 0
                    || nextPolicy.maxLifetimeSpend < lifetimeSpent || nextPolicy.maxLifetimeSpend == 0
            ) revert WalletError(ErrorCode.InvalidPolicy);
        }
        _policy = nextPolicy;
        policyVersion += 1;
        revoked = false;
        if (nextPolicy.periodSeconds > 0) {
            periodBucket = block.timestamp / nextPolicy.periodSeconds;
            periodSpent = 0;
        }
        emit PolicyConfigured(policyVersion, nextPolicy.delegate, nextPolicy.allowedActions, keccak256(abi.encode(nextPolicy)));
    }

    function _requireDirectDelegate(Action action) private view {
        if (msg.sender != _policy.delegate) revert WalletError(ErrorCode.Unauthorized);
        _requireActivePolicy(action);
    }

    function _consumeDirectNonce() private returns (uint256 nonce) {
        nonce = delegateNonce;
        delegateNonce = nonce + 1;
    }

    function _requireActivePolicy(Action action) private view {
        if (revoked || block.timestamp < _policy.validAfter || block.timestamp > _policy.validUntil) {
            revert WalletError(ErrorCode.InactivePolicy);
        }
        if ((_policy.allowedActions & (uint8(1) << uint8(action))) == 0) {
            revert WalletError(ErrorCode.ActionNotAllowed);
        }
    }

    function _dispatch(Action action, bytes calldata payload) private returns (bytes memory) {
        if (action == Action.Commit) {
            (address commitBountyAddress, bytes32 commitment) = abi.decode(payload, (address, bytes32));
            _commitSolution(commitBountyAddress, commitment);
            return bytes("");
        }
        if (action == Action.Reveal) {
            (
                address revealBountyAddress,
                bytes32 submissionHash,
                bytes32 evidenceHash,
                bytes32 salt,
                bytes memory proof
            ) = abi.decode(payload, (address, bytes32, bytes32, bytes32, bytes));
            _revealSolution(revealBountyAddress, submissionHash, evidenceHash, salt, proof);
            return bytes("");
        }
        address withdrawBountyAddress = abi.decode(payload, (address));
        _withdrawEntryBond(withdrawBountyAddress);
        return bytes("");
    }

    function _commitSolution(address bountyAddress, bytes32 commitment) private {
        OpenCompetitionBountyV1 bounty = _canonicalCompetition(bountyAddress);
        _requireBountyPolicy(bounty);
        if (bounty.creator() == owner || bounty.creator() == _policy.delegate) {
            revert WalletError(ErrorCode.CreatorControlled);
        }
        uint256 bond = bounty.verifierReward();
        _chargeSpend(bond);
        _approveExact(bountyAddress, bond);
        bounty.commitSolution(commitment);
        _approveExact(bountyAddress, 0);
    }

    function _revealSolution(
        address bountyAddress,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 salt,
        bytes memory proof
    ) private {
        OpenCompetitionBountyV1 bounty = _canonicalCompetition(bountyAddress);
        _requireBountyPolicy(bounty);
        if (bounty.creator() == owner || bounty.creator() == _policy.delegate) {
            revert WalletError(ErrorCode.CreatorControlled);
        }
        bounty.revealSolution(submissionHash, evidenceHash, salt, proof);
    }

    function _withdrawEntryBond(address bountyAddress) private {
        OpenCompetitionBountyV1 bounty = _canonicalCompetition(bountyAddress);
        bounty.withdrawEntryBond();
    }

    function _canonicalCompetition(address bountyAddress) private view returns (OpenCompetitionBountyV1 bounty) {
        if (!factory.isCanonicalCompetition(bountyAddress)) revert WalletError(ErrorCode.InvalidCompetition);
        bounty = OpenCompetitionBountyV1(bountyAddress);
        if (
            bounty.factory() != address(factory) || bounty.settlementToken() != settlementToken
                || bounty.protocolVersion() != factory.SUPPORTED_PROTOCOL_VERSION()
        ) revert WalletError(ErrorCode.InvalidCompetition);
    }

    function _requireBountyPolicy(OpenCompetitionBountyV1 bounty) private view {
        if (bounty.targetAmount() > _policy.maxBountyTarget) revert WalletError(ErrorCode.SpendLimitExceeded);
        if (
            bounty.verifierModule() != _policy.verifierModule
                || _policy.verifierModule.codehash != _policy.verifierRuntimeCodeHash
                || bounty.policyHash() != _policy.verifierPolicyHash
                || bounty.acceptanceCriteriaHash() != _policy.acceptanceCriteriaHash
                || bounty.benchmarkHash() != _policy.benchmarkHash
                || bounty.evidenceSchemaHash() != _policy.evidenceSchemaHash
        ) revert WalletError(ErrorCode.VerifierProfileMismatch);
    }

    function _chargeSpend(uint256 amount) private {
        if (amount == 0) revert WalletError(ErrorCode.InvalidAmount);
        if (amount > _policy.maxPerAction) revert WalletError(ErrorCode.SpendLimitExceeded);
        uint256 bucket = block.timestamp / _policy.periodSeconds;
        if (bucket != periodBucket) {
            periodBucket = bucket;
            periodSpent = 0;
        }
        if (
            periodSpent + amount > _policy.maxPerPeriod || lifetimeSpent + amount > _policy.maxLifetimeSpend
        ) revert WalletError(ErrorCode.SpendLimitExceeded);
        periodSpent += amount;
        lifetimeSpent += amount;
        emit SpendCharged(amount, periodSpent, lifetimeSpent, bucket);
    }

    function _approveExact(address spender, uint256 amount) private {
        (bool zeroOk, bytes memory zeroResult) =
            settlementToken.call(abi.encodeWithSignature("approve(address,uint256)", spender, 0));
        if (!zeroOk || (zeroResult.length != 0 && !abi.decode(zeroResult, (bool)))) {
            revert WalletError(ErrorCode.TokenOperationFailed);
        }
        if (amount == 0) return;
        (bool ok, bytes memory result) =
            settlementToken.call(abi.encodeWithSignature("approve(address,uint256)", spender, amount));
        if (!ok || (result.length != 0 && !abi.decode(result, (bool)))) {
            revert WalletError(ErrorCode.TokenOperationFailed);
        }
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
