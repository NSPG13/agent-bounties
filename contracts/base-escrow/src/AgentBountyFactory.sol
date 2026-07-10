// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./AgentBounty.sol";

/// @notice Deploys canonical bounties and exposes an untrusted external-contract discovery lane.
contract AgentBountyFactory {
    using SafeBountyToken for address;

    bytes32 public constant SUPPORTED_PROTOCOL_VERSION = keccak256("agent-bounties/autonomous-v1");

    struct CreateBountyParams {
        uint256 solverReward;
        uint256 verifierReward;
        bytes32 termsHash;
        bytes32 policyHash;
        bytes32 acceptanceCriteriaHash;
        bytes32 benchmarkHash;
        bytes32 evidenceSchemaHash;
        uint64 fundingDeadline;
        uint64 claimWindowSeconds;
        uint64 verificationWindowSeconds;
        AgentBounty.VerificationMode verificationMode;
        address verifierModule;
        address verifierRewardRecipient;
        uint8 threshold;
    }

    struct FundingAuthorization {
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
        uint8 v;
        bytes32 r;
        bytes32 s;
    }

    address public immutable settlementToken;
    address public immutable implementation;
    uint256 private _reentrancy = 1;
    mapping(address => bool) public isCanonicalBounty;
    mapping(address => bool) public isSubmittedExternalBounty;

    event CanonicalBountyCreated(
        bytes32 indexed bountyId,
        address indexed bounty,
        address indexed creator,
        bytes32 termsHash,
        bytes32 policyHash,
        bytes32 creationNonce
    );
    event CanonicalBountyTermsCommitted(
        bytes32 indexed bountyId, bytes32 acceptanceCriteriaHash, bytes32 benchmarkHash, bytes32 evidenceSchemaHash
    );
    event CanonicalBountyEconomicsConfigured(
        bytes32 indexed bountyId,
        uint256 solverReward,
        uint256 verifierReward,
        uint256 targetAmount,
        uint256 initialFunding,
        uint64 fundingDeadline,
        uint64 claimWindowSeconds,
        uint64 verificationWindowSeconds
    );
    event CanonicalBountyVerificationConfigured(
        bytes32 indexed bountyId,
        AgentBounty.VerificationMode verificationMode,
        address verifierModule,
        address verifierRewardRecipient,
        uint8 threshold,
        bytes32 verifierSetHash
    );
    event ExternalBountySubmitted(
        address indexed bounty,
        address indexed submitter,
        bytes32 indexed bountyId,
        bytes32 termsHash,
        bytes32 policyHash
    );

    modifier nonReentrant() {
        require(_reentrancy == 1, "reentrant");
        _reentrancy = 2;
        _;
        _reentrancy = 1;
    }

    constructor(address settlementToken_) {
        require(settlementToken_ != address(0), "token zero");
        settlementToken = settlementToken_;
        implementation = address(new AgentBounty());
    }

    /// @notice One factory transaction deploys and optionally funds the bounty.
    function createBounty(
        CreateBountyParams calldata params,
        address[] calldata verifiers,
        uint256 initialFunding,
        bytes32 creationNonce
    ) external nonReentrant returns (address bountyAddress, bytes32 bountyId) {
        (AgentBounty bounty, bytes32 id) = _deployBounty(msg.sender, params, verifiers, creationNonce);
        bountyAddress = address(bounty);
        bountyId = id;

        if (initialFunding > 0) {
            settlementToken.safeTransferFrom(msg.sender, bountyAddress, initialFunding);
            bounty.recordFactoryFunding(msg.sender, initialFunding);
        }

        _emitCanonicalBountyCreated(
            bountyId, bountyAddress, msg.sender, params, verifiers, initialFunding, creationNonce
        );
    }

    /// @notice A relayer can create and fund a predictable bounty from one signed
    /// Circle USDC EIP-3009 authorization. The destination is bound to the CREATE2 address.
    function createBountyWithAuthorization(
        address creator,
        CreateBountyParams calldata params,
        address[] calldata verifiers,
        uint256 initialFunding,
        bytes32 creationNonce,
        FundingAuthorization calldata authorization
    ) external nonReentrant returns (address bountyAddress, bytes32 bountyId) {
        require(initialFunding > 0, "initial funding zero");
        (AgentBounty bounty, bytes32 id) = _deployBounty(creator, params, verifiers, creationNonce);
        bountyAddress = address(bounty);
        bountyId = id;
        settlementToken.safeTransferWithAuthorization(
            creator,
            bountyAddress,
            initialFunding,
            authorization.validAfter,
            authorization.validBefore,
            authorization.nonce,
            authorization.v,
            authorization.r,
            authorization.s
        );
        bounty.recordFactoryFunding(creator, initialFunding);
        _emitCanonicalBountyCreated(bountyId, bountyAddress, creator, params, verifiers, initialFunding, creationNonce);
    }

    function bountyIdFor(
        address creator,
        CreateBountyParams calldata params,
        address[] calldata verifiers,
        bytes32 creationNonce
    ) public view returns (bytes32) {
        return keccak256(abi.encode(block.chainid, address(this), creator, creationNonce, params, verifiers));
    }

    function predictBountyAddress(
        address creator,
        CreateBountyParams calldata params,
        address[] calldata verifiers,
        bytes32 creationNonce
    ) external view returns (address) {
        return _predictDeterministicAddress(implementation, bountyIdFor(creator, params, verifiers, creationNonce));
    }

    /// @notice Makes a compatible external contract discoverable without endorsing its code or funds.
    function submitExternalBounty(address bountyAddress) external {
        require(bountyAddress.code.length > 0, "external bounty has no code");
        require(!isCanonicalBounty[bountyAddress], "already canonical");
        require(!isSubmittedExternalBounty[bountyAddress], "already submitted");

        IAgentBountyV1 bounty = IAgentBountyV1(bountyAddress);
        require(bounty.supportsInterface(type(IAgentBountyV1).interfaceId), "unsupported interface");
        require(bounty.protocolVersion() == SUPPORTED_PROTOCOL_VERSION, "unsupported version");
        require(bounty.settlementToken() == settlementToken, "unsupported token");
        require(bounty.targetAmount() > 0, "external target zero");

        isSubmittedExternalBounty[bountyAddress] = true;
        emit ExternalBountySubmitted(
            bountyAddress, msg.sender, bounty.bountyId(), bounty.termsHash(), bounty.policyHash()
        );
    }

    function _emitCanonicalBountyCreated(
        bytes32 bountyId,
        address bountyAddress,
        address creator,
        CreateBountyParams calldata params,
        address[] calldata verifiers,
        uint256 initialFunding,
        bytes32 creationNonce
    ) private {
        emit CanonicalBountyCreated(
            bountyId, bountyAddress, creator, params.termsHash, params.policyHash, creationNonce
        );
        emit CanonicalBountyTermsCommitted(
            bountyId, params.acceptanceCriteriaHash, params.benchmarkHash, params.evidenceSchemaHash
        );
        emit CanonicalBountyEconomicsConfigured(
            bountyId,
            params.solverReward,
            params.verifierReward,
            params.solverReward + params.verifierReward,
            initialFunding,
            params.fundingDeadline,
            params.claimWindowSeconds,
            params.verificationWindowSeconds
        );
        emit CanonicalBountyVerificationConfigured(
            bountyId,
            params.verificationMode,
            params.verifierModule,
            params.verifierRewardRecipient,
            params.threshold,
            verifiers.length == 0 ? bytes32(0) : keccak256(abi.encode(verifiers))
        );
    }

    function _deployBounty(
        address creator,
        CreateBountyParams calldata params,
        address[] calldata verifiers,
        bytes32 creationNonce
    ) private returns (AgentBounty bounty, bytes32 bountyId) {
        require(creator != address(0), "creator zero");
        require(creationNonce != bytes32(0), "creation nonce zero");
        uint256 target = params.solverReward + params.verifierReward;
        require(target >= params.solverReward, "target overflow");
        require(target <= type(uint64).max, "target too large");
        bountyId = bountyIdFor(creator, params, verifiers, creationNonce);
        address bountyAddress = _cloneDeterministic(implementation, bountyId);
        bounty = AgentBounty(bountyAddress);
        AgentBounty.Config memory config = AgentBounty.Config({
            bountyId: bountyId,
            creator: creator,
            factory: address(this),
            settlementToken: settlementToken,
            solverReward: params.solverReward,
            verifierReward: params.verifierReward,
            termsHash: params.termsHash,
            policyHash: params.policyHash,
            acceptanceCriteriaHash: params.acceptanceCriteriaHash,
            benchmarkHash: params.benchmarkHash,
            evidenceSchemaHash: params.evidenceSchemaHash,
            fundingDeadline: params.fundingDeadline,
            claimWindowSeconds: params.claimWindowSeconds,
            verificationWindowSeconds: params.verificationWindowSeconds,
            verificationMode: params.verificationMode,
            verifierModule: params.verifierModule,
            verifierRewardRecipient: params.verifierRewardRecipient,
            threshold: params.threshold
        });
        isCanonicalBounty[bountyAddress] = true;
        bounty.initialize(config, verifiers);
    }

    function _cloneDeterministic(address target, bytes32 salt) private returns (address instance) {
        bytes20 targetBytes = bytes20(target);
        bytes memory creationCode = abi.encodePacked(
            hex"3d602d80600a3d3981f3", hex"363d3d373d3d3d363d73", targetBytes, hex"5af43d82803e903d91602b57fd5bf3"
        );
        assembly ("memory-safe") {
            instance := create2(0, add(creationCode, 0x20), mload(creationCode), salt)
        }
        require(instance != address(0), "bounty deployment failed");
    }

    function _predictDeterministicAddress(address target, bytes32 salt) private view returns (address) {
        bytes32 initCodeHash = keccak256(
            abi.encodePacked(
                hex"3d602d80600a3d3981f3",
                hex"363d3d373d3d3d363d73",
                bytes20(target),
                hex"5af43d82803e903d91602b57fd5bf3"
            )
        );
        return address(uint160(uint256(keccak256(abi.encodePacked(bytes1(0xff), address(this), salt, initCodeHash)))));
    }
}
