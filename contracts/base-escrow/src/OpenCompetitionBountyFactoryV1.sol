// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./OpenCompetitionBountyV1.sol";

/// @notice Deploys canonical deterministic first-valid competitions.
contract OpenCompetitionBountyFactoryV1 {
    using SafeBountyToken for address;

    bytes32 public constant SUPPORTED_PROTOCOL_VERSION = keccak256("agent-bounties/open-competition-v1");

    struct CreateCompetitionParams {
        uint256 solverReward;
        uint256 verifierReward;
        bytes32 termsHash;
        bytes32 policyHash;
        bytes32 acceptanceCriteriaHash;
        bytes32 benchmarkHash;
        bytes32 evidenceSchemaHash;
        uint64 fundingDeadline;
        uint64 competitionWindowSeconds;
        uint64 revealWindowSeconds;
        uint8 maxEntries;
        address verifierModule;
        address verifierRewardRecipient;
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
    mapping(address => bool) public isCanonicalCompetition;
    uint256 private _reentrancy = 1;

    event CanonicalCompetitionCreated(
        bytes32 indexed bountyId,
        address indexed bounty,
        address indexed creator,
        bytes32 termsHash,
        bytes32 policyHash,
        bytes32 creationNonce
    );
    event CanonicalCompetitionTermsCommitted(
        bytes32 indexed bountyId, bytes32 acceptanceCriteriaHash, bytes32 benchmarkHash, bytes32 evidenceSchemaHash
    );
    event CanonicalCompetitionEconomicsConfigured(
        bytes32 indexed bountyId,
        uint256 solverReward,
        uint256 verifierReward,
        uint256 entryBond,
        uint256 targetAmount,
        uint256 initialFunding,
        uint64 fundingDeadline,
        uint64 competitionWindowSeconds,
        uint64 revealWindowSeconds,
        uint8 maxEntries
    );
    event CanonicalCompetitionVerificationConfigured(
        bytes32 indexed bountyId, address verifierModule, address verifierRewardRecipient
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
        implementation = address(new OpenCompetitionBountyV1());
    }

    function createCompetition(CreateCompetitionParams calldata params, uint256 initialFunding, bytes32 creationNonce)
        external
        nonReentrant
        returns (address bountyAddress, bytes32 bountyId)
    {
        OpenCompetitionBountyV1 bounty;
        (bounty, bountyId) = _deploy(msg.sender, params, creationNonce);
        bountyAddress = address(bounty);
        if (initialFunding > 0) {
            settlementToken.safeTransferFrom(msg.sender, bountyAddress, initialFunding);
            bounty.recordFactoryFunding(msg.sender, initialFunding);
        }
        _emitConfiguration(bountyId, bountyAddress, msg.sender, params, initialFunding, creationNonce);
    }

    function createCompetitionWithAuthorization(
        address creator,
        CreateCompetitionParams calldata params,
        uint256 initialFunding,
        bytes32 creationNonce,
        FundingAuthorization calldata authorization
    ) external nonReentrant returns (address bountyAddress, bytes32 bountyId) {
        require(initialFunding > 0, "initial funding zero");
        OpenCompetitionBountyV1 bounty;
        (bounty, bountyId) = _deploy(creator, params, creationNonce);
        bountyAddress = address(bounty);
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
        _emitConfiguration(bountyId, bountyAddress, creator, params, initialFunding, creationNonce);
    }

    function bountyIdFor(address creator, CreateCompetitionParams calldata params, bytes32 creationNonce)
        public
        view
        returns (bytes32)
    {
        return keccak256(abi.encode(block.chainid, address(this), creator, creationNonce, params));
    }

    function predictCompetitionAddress(address creator, CreateCompetitionParams calldata params, bytes32 creationNonce)
        external
        view
        returns (address)
    {
        return _predictDeterministicAddress(implementation, bountyIdFor(creator, params, creationNonce));
    }

    function _deploy(address creator, CreateCompetitionParams calldata params, bytes32 creationNonce)
        private
        returns (OpenCompetitionBountyV1 bounty, bytes32 bountyId)
    {
        require(creator != address(0), "creator zero");
        require(creationNonce != bytes32(0), "creation nonce zero");
        uint256 target = params.solverReward + params.verifierReward;
        require(target >= params.solverReward && target <= type(uint64).max, "target invalid");
        bountyId = bountyIdFor(creator, params, creationNonce);
        address bountyAddress = _cloneDeterministic(implementation, bountyId);
        bounty = OpenCompetitionBountyV1(bountyAddress);
        isCanonicalCompetition[bountyAddress] = true;
        bounty.initialize(
            OpenCompetitionBountyV1.Config({
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
                competitionWindowSeconds: params.competitionWindowSeconds,
                revealWindowSeconds: params.revealWindowSeconds,
                maxEntries: params.maxEntries,
                verifierModule: params.verifierModule,
                verifierRewardRecipient: params.verifierRewardRecipient
            })
        );
    }

    function _emitConfiguration(
        bytes32 bountyId,
        address bountyAddress,
        address creator,
        CreateCompetitionParams calldata params,
        uint256 initialFunding,
        bytes32 creationNonce
    ) private {
        emit CanonicalCompetitionCreated(
            bountyId, bountyAddress, creator, params.termsHash, params.policyHash, creationNonce
        );
        emit CanonicalCompetitionTermsCommitted(
            bountyId, params.acceptanceCriteriaHash, params.benchmarkHash, params.evidenceSchemaHash
        );
        emit CanonicalCompetitionEconomicsConfigured(
            bountyId,
            params.solverReward,
            params.verifierReward,
            params.verifierReward,
            params.solverReward + params.verifierReward,
            initialFunding,
            params.fundingDeadline,
            params.competitionWindowSeconds,
            params.revealWindowSeconds,
            params.maxEntries
        );
        emit CanonicalCompetitionVerificationConfigured(bountyId, params.verifierModule, params.verifierRewardRecipient);
    }

    function _cloneDeterministic(address target, bytes32 salt) private returns (address instance) {
        bytes20 targetBytes = bytes20(target);
        bytes memory creationCode = abi.encodePacked(
            hex"3d602d80600a3d3981f3", hex"363d3d373d3d3d363d73", targetBytes, hex"5af43d82803e903d91602b57fd5bf3"
        );
        assembly ("memory-safe") {
            instance := create2(0, add(creationCode, 0x20), mload(creationCode), salt)
        }
        require(instance != address(0), "competition deployment failed");
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
