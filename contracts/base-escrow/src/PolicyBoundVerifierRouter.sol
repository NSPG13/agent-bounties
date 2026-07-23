// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";

interface ICanonicalBountyFactoryRouterView {
    function isCanonicalBounty(address bounty) external view returns (bool);
}

interface IRoutedAgentBountyVerifier {
    function verifierRouter() external view returns (address);
    function committedPolicyHash() external view returns (bytes32);
    function canonicalFactory() external view returns (address);

    function verifyRouted(
        address parentBounty,
        bytes32 bountyId,
        uint64 round,
        address solver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 policyHash,
        bytes calldata proof
    ) external view returns (bool passed, bytes32 responseHash);
}

/// @notice Stable deterministic verifier entrypoint for a bounded autonomous wallet.
/// @dev Policy records are append-only and bind one immutable policy hash to one
/// implementation address and runtime code hash. Active records cannot be replaced.
contract PolicyBoundVerifierRouter is IAgentBountyVerifier {
    struct PolicyRecord {
        address verifier;
        bytes32 runtimeCodeHash;
        uint64 proposedAt;
        uint64 activateAfter;
        uint64 activatedAt;
        bool vetoed;
    }

    uint64 public constant MIN_ACTIVATION_DELAY = 1 days;
    uint64 public constant MAX_ACTIVATION_DELAY = 30 days;
    uint64 public constant BOOTSTRAP_WINDOW = 1 days;

    address public immutable canonicalFactory;
    address public immutable registrar;
    address public immutable guardian;
    uint64 public immutable activationDelay;
    uint64 public immutable bootstrapDeadline;

    bool public bootstrapUsed;
    mapping(bytes32 => PolicyRecord) public policies;

    event PolicyBootstrapped(bytes32 indexed policyHash, address indexed verifier, bytes32 runtimeCodeHash);
    event PolicyProposed(
        bytes32 indexed policyHash,
        address indexed verifier,
        bytes32 runtimeCodeHash,
        uint64 activateAfter
    );
    event PolicyVetoed(bytes32 indexed policyHash, address indexed actor);
    event PolicyActivated(bytes32 indexed policyHash, address indexed verifier, bytes32 runtimeCodeHash);

    error NotRegistrar();
    error NotGuardianOrRegistrar();
    error InvalidConfiguration();
    error InvalidPolicy();
    error PolicyAlreadyExists();
    error BootstrapUnavailable();
    error PolicyNotPending();
    error ActivationTooEarly();
    error PolicyNotActive();
    error VerifierCodeChanged();
    error NonCanonicalCaller();

    constructor(address canonicalFactory_, address registrar_, address guardian_, uint64 activationDelay_) {
        if (
            canonicalFactory_.code.length == 0 || registrar_ == address(0) || guardian_ == address(0)
                || activationDelay_ < MIN_ACTIVATION_DELAY || activationDelay_ > MAX_ACTIVATION_DELAY
        ) revert InvalidConfiguration();
        canonicalFactory = canonicalFactory_;
        registrar = registrar_;
        guardian = guardian_;
        activationDelay = activationDelay_;
        bootstrapDeadline = uint64(block.timestamp) + BOOTSTRAP_WINDOW;
    }

    function bootstrapPolicy(bytes32 policyHash, address verifier) external {
        if (msg.sender != registrar) revert NotRegistrar();
        if (bootstrapUsed || block.timestamp > bootstrapDeadline) revert BootstrapUnavailable();
        if (_recordExists(policyHash)) revert PolicyAlreadyExists();

        bytes32 runtimeCodeHash = _validateVerifier(policyHash, verifier);
        bootstrapUsed = true;
        policies[policyHash] = PolicyRecord({
            verifier: verifier,
            runtimeCodeHash: runtimeCodeHash,
            proposedAt: uint64(block.timestamp),
            activateAfter: uint64(block.timestamp),
            activatedAt: uint64(block.timestamp),
            vetoed: false
        });
        emit PolicyBootstrapped(policyHash, verifier, runtimeCodeHash);
    }

    function proposePolicy(bytes32 policyHash, address verifier) external {
        if (msg.sender != registrar) revert NotRegistrar();
        if (_recordExists(policyHash)) revert PolicyAlreadyExists();

        bytes32 runtimeCodeHash = _validateVerifier(policyHash, verifier);
        uint64 activateAfter = uint64(block.timestamp) + activationDelay;
        policies[policyHash] = PolicyRecord({
            verifier: verifier,
            runtimeCodeHash: runtimeCodeHash,
            proposedAt: uint64(block.timestamp),
            activateAfter: activateAfter,
            activatedAt: 0,
            vetoed: false
        });
        emit PolicyProposed(policyHash, verifier, runtimeCodeHash, activateAfter);
    }

    function vetoPolicy(bytes32 policyHash) external {
        if (msg.sender != guardian && msg.sender != registrar) revert NotGuardianOrRegistrar();
        PolicyRecord storage record = policies[policyHash];
        if (record.verifier == address(0) || record.activatedAt != 0 || record.vetoed) revert PolicyNotPending();
        record.vetoed = true;
        emit PolicyVetoed(policyHash, msg.sender);
    }

    function activatePolicy(bytes32 policyHash) external {
        PolicyRecord storage record = policies[policyHash];
        if (record.verifier == address(0) || record.activatedAt != 0 || record.vetoed) revert PolicyNotPending();
        if (block.timestamp < record.activateAfter) revert ActivationTooEarly();
        _assertVerifierStillMatches(policyHash, record);
        record.activatedAt = uint64(block.timestamp);
        emit PolicyActivated(policyHash, record.verifier, record.runtimeCodeHash);
    }

    function isPolicyActive(bytes32 policyHash) external view returns (bool) {
        PolicyRecord storage record = policies[policyHash];
        return record.activatedAt != 0 && !record.vetoed && record.verifier != address(0)
            && record.verifier.codehash == record.runtimeCodeHash;
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
        if (!ICanonicalBountyFactoryRouterView(canonicalFactory).isCanonicalBounty(msg.sender)) {
            revert NonCanonicalCaller();
        }
        PolicyRecord storage record = policies[policyHash];
        if (record.activatedAt == 0 || record.vetoed || record.verifier == address(0)) revert PolicyNotActive();
        _assertVerifierStillMatches(policyHash, record);
        return IRoutedAgentBountyVerifier(record.verifier).verifyRouted(
            msg.sender,
            bountyId,
            round,
            solver,
            submissionHash,
            evidenceHash,
            policyHash,
            proof
        );
    }

    function _recordExists(bytes32 policyHash) private view returns (bool) {
        if (policyHash == bytes32(0)) revert InvalidPolicy();
        return policies[policyHash].verifier != address(0);
    }

    function _validateVerifier(bytes32 policyHash, address verifier) private view returns (bytes32 runtimeCodeHash) {
        if (policyHash == bytes32(0) || verifier.code.length == 0) revert InvalidPolicy();
        if (
            IRoutedAgentBountyVerifier(verifier).verifierRouter() != address(this)
                || IRoutedAgentBountyVerifier(verifier).committedPolicyHash() != policyHash
                || IRoutedAgentBountyVerifier(verifier).canonicalFactory() != canonicalFactory
        ) revert InvalidPolicy();
        runtimeCodeHash = verifier.codehash;
        if (runtimeCodeHash == bytes32(0)) revert InvalidPolicy();
    }

    function _assertVerifierStillMatches(bytes32 policyHash, PolicyRecord storage record) private view {
        if (record.verifier.codehash != record.runtimeCodeHash) revert VerifierCodeChanged();
        if (
            IRoutedAgentBountyVerifier(record.verifier).verifierRouter() != address(this)
                || IRoutedAgentBountyVerifier(record.verifier).committedPolicyHash() != policyHash
                || IRoutedAgentBountyVerifier(record.verifier).canonicalFactory() != canonicalFactory
        ) revert VerifierCodeChanged();
    }
}
