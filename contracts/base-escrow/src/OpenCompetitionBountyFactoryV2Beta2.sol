// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./OpenCompetitionBountyV2Beta2.sol";

/// @notice Canonical deterministic factory for isolated V2 Beta2 escrows. It
/// never receives bounty funds and never spends a contributor allowance.
contract OpenCompetitionBountyFactoryV2Beta2 {
    bytes32 public constant SUPPORTED_PROTOCOL_VERSION = keccak256("agent-bounties/open-competition-v2-beta2");
    bytes32 public constant PROOF_SYSTEM_GROTH16 = keccak256("sp1-groth16");
    bytes32 public constant PROOF_SYSTEM_PLONK = keccak256("sp1-plonk");

    address public constant BASE_USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
    address public constant BASE_SEPOLIA_USDC = 0x036CbD53842c5426634e7929541eC2318f3dCF7e;

    struct CreateCompetitionParams {
        uint256 solverReward;
        uint256 keeperReward;
        uint64 fundingDeadline;
        uint64 proofWindowSeconds;
        OpenCompetitionBountyV2Beta2.WinnerMode winnerMode;
        OpenCompetitionBountyV2Beta2.ScoreDirection scoreDirection;
        int256 scoreThreshold;
        bytes32 proofSystem;
        bytes32 programVKey;
        bytes32 sourceHash;
        bytes32 elfHash;
        bytes32 journalSchemaHash;
        bytes32 metricProgramHash;
        bytes32 executionPolicyHash;
        bytes32 verificationPolicyHash;
        bytes32 settlementPolicyHash;
        bytes32 betaRiskHash;
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
    address public immutable groth16Adapter;
    address public immutable plonkAdapter;
    address public immutable implementation;
    mapping(address => bool) public isCanonicalCompetition;
    uint256 private _reentrancy = 1;

    event CanonicalCompetitionCreatedV2(
        bytes32 indexed bountyId,
        address indexed competition,
        address indexed creator,
        bytes32 creationNonce,
        bytes32 betaRiskHash
    );
    event CanonicalCompetitionEconomicsV2(
        bytes32 indexed bountyId,
        uint256 solverReward,
        uint256 keeperReward,
        uint64 fundingDeadline,
        uint64 proofWindowSeconds,
        uint8 winnerMode,
        uint8 scoreDirection,
        int256 scoreThreshold
    );
    event CanonicalCompetitionVerificationV2(
        bytes32 indexed bountyId,
        bytes32 proofSystem,
        address verifierAdapter,
        bytes32 programVKey,
        bytes32 sourceHash,
        bytes32 elfHash,
        bytes32 journalSchemaHash,
        bytes32 metricProgramHash
    );
    event CanonicalCompetitionPoliciesV2(
        bytes32 indexed bountyId,
        bytes32 executionPolicyHash,
        bytes32 verificationPolicyHash,
        bytes32 settlementPolicyHash
    );

    modifier nonReentrant() {
        require(_reentrancy == 1, "reentrant");
        _reentrancy = 2;
        _;
        _reentrancy = 1;
    }

    constructor(
        address settlementToken_,
        address groth16Verifier_,
        bytes32 groth16VerifierHash_,
        bytes32 groth16RuntimeCodeHash_,
        address plonkVerifier_,
        bytes32 plonkVerifierHash_,
        bytes32 plonkRuntimeCodeHash_
    ) {
        _requireCanonicalBaseToken(settlementToken_);
        require(settlementToken_.code.length > 0, "token missing");
        settlementToken = settlementToken_;
        groth16Adapter = address(
            new Sp1VerifierAdapterV2Beta2(
                PROOF_SYSTEM_GROTH16, groth16Verifier_, groth16VerifierHash_, groth16RuntimeCodeHash_
            )
        );
        plonkAdapter = address(
            new Sp1VerifierAdapterV2Beta2(PROOF_SYSTEM_PLONK, plonkVerifier_, plonkVerifierHash_, plonkRuntimeCodeHash_)
        );
        implementation = address(new OpenCompetitionBountyV2Beta2());
    }

    function createCompetition(
        CreateCompetitionParams calldata params,
        uint256 initialFunding,
        bytes32 creationNonce,
        bytes32 acknowledgedRiskHash
    ) external nonReentrant returns (address competitionAddress, bytes32 bountyId) {
        OpenCompetitionBountyV2Beta2 competition;
        (competition, bountyId) = _deploy(msg.sender, params, creationNonce, acknowledgedRiskHash);
        competitionAddress = address(competition);
        _emitConfiguration(bountyId, competitionAddress, msg.sender, params, creationNonce);
        if (initialFunding > 0) {
            competition.fundFromFactory(msg.sender, initialFunding, acknowledgedRiskHash);
        }
    }

    function createCompetitionWithAuthorization(
        address creator,
        CreateCompetitionParams calldata params,
        uint256 initialFunding,
        bytes32 creationNonce,
        bytes32 acknowledgedRiskHash,
        FundingAuthorization calldata authorization
    ) external nonReentrant returns (address competitionAddress, bytes32 bountyId) {
        require(initialFunding > 0, "initial funding zero");
        OpenCompetitionBountyV2Beta2 competition;
        (competition, bountyId) = _deploy(creator, params, creationNonce, acknowledgedRiskHash);
        competitionAddress = address(competition);
        _emitConfiguration(bountyId, competitionAddress, creator, params, creationNonce);
        competition.fundWithAuthorization(
            creator,
            initialFunding,
            acknowledgedRiskHash,
            authorization.validAfter,
            authorization.validBefore,
            authorization.nonce,
            authorization.v,
            authorization.r,
            authorization.s
        );
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

    function _deploy(
        address creator,
        CreateCompetitionParams calldata params,
        bytes32 creationNonce,
        bytes32 acknowledgedRiskHash
    ) private returns (OpenCompetitionBountyV2Beta2 competition, bytes32 bountyId) {
        require(creator != address(0), "creator zero");
        require(creationNonce != bytes32(0), "creation nonce zero");
        require(acknowledgedRiskHash == params.betaRiskHash, "risk hash mismatch");
        bountyId = bountyIdFor(creator, params, creationNonce);
        address competitionAddress = _cloneDeterministic(implementation, bountyId);
        competition = OpenCompetitionBountyV2Beta2(competitionAddress);
        require(
            params.proofSystem == PROOF_SYSTEM_GROTH16 || params.proofSystem == PROOF_SYSTEM_PLONK,
            "unsupported proof system"
        );
        address adapter = params.proofSystem == PROOF_SYSTEM_GROTH16 ? groth16Adapter : plonkAdapter;
        competition.initialize(
            OpenCompetitionBountyV2Beta2.Config({
                bountyId: bountyId,
                creator: creator,
                factory: address(this),
                settlementToken: settlementToken,
                verifierAdapter: adapter,
                proofSystem: params.proofSystem,
                programVKey: params.programVKey,
                sourceHash: params.sourceHash,
                elfHash: params.elfHash,
                journalSchemaHash: params.journalSchemaHash,
                metricProgramHash: params.metricProgramHash,
                executionPolicyHash: params.executionPolicyHash,
                verificationPolicyHash: params.verificationPolicyHash,
                settlementPolicyHash: params.settlementPolicyHash,
                betaRiskHash: params.betaRiskHash,
                solverReward: params.solverReward,
                keeperReward: params.keeperReward,
                fundingDeadline: params.fundingDeadline,
                proofWindowSeconds: params.proofWindowSeconds,
                winnerMode: params.winnerMode,
                scoreDirection: params.scoreDirection,
                scoreThreshold: params.scoreThreshold
            })
        );
        isCanonicalCompetition[competitionAddress] = true;
    }

    function _emitConfiguration(
        bytes32 bountyId,
        address competition,
        address creator,
        CreateCompetitionParams calldata params,
        bytes32 creationNonce
    ) private {
        address adapter = params.proofSystem == PROOF_SYSTEM_GROTH16 ? groth16Adapter : plonkAdapter;
        emit CanonicalCompetitionCreatedV2(bountyId, competition, creator, creationNonce, params.betaRiskHash);
        emit CanonicalCompetitionEconomicsV2(
            bountyId,
            params.solverReward,
            params.keeperReward,
            params.fundingDeadline,
            params.proofWindowSeconds,
            uint8(params.winnerMode),
            uint8(params.scoreDirection),
            params.scoreThreshold
        );
        emit CanonicalCompetitionVerificationV2(
            bountyId,
            params.proofSystem,
            adapter,
            params.programVKey,
            params.sourceHash,
            params.elfHash,
            params.journalSchemaHash,
            params.metricProgramHash
        );
        emit CanonicalCompetitionPoliciesV2(
            bountyId, params.executionPolicyHash, params.verificationPolicyHash, params.settlementPolicyHash
        );
    }

    function _requireCanonicalBaseToken(address token) private view {
        if (block.chainid == 8453) {
            require(token == BASE_USDC, "noncanonical Base USDC");
        } else if (block.chainid == 84532) {
            require(token == BASE_SEPOLIA_USDC, "noncanonical Base Sepolia USDC");
        }
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
