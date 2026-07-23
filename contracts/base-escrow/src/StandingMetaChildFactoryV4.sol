// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./AgentBountyFactory.sol";
import "./AppealableVerifierV1.sol";
import "./StandingMetaChildV4.sol";

interface IStandingMetaParentFactoryV4Configuration {
    function childFactory() external view returns (address);
    function standingMetaChildFactory() external view returns (address);
    function appealableVerifier() external view returns (address);
}

/// @notice Staged immutable factory for claim-restricted V4 children. It is
/// wired once to the parent factory so child creation code does not push the
/// parent factory beyond EIP-170/EIP-3860 deployment limits.
contract StandingMetaChildFactoryV4 {
    using SafeBountyToken for address;

    uint256 public constant CHILD_TARGET = 1_000_000;
    uint256 public constant CHILD_SOLVER_REWARD = 990_000;
    uint256 public constant CHILD_VERIFIER_REWARD = 10_000;
    uint64 public constant CHILD_WORK_WINDOW = 7 days;
    uint64 public constant CHILD_VERIFICATION_WINDOW = 96 hours;
    uint8 public constant DETERMINISTIC_MODE = 0;

    AgentBountyFactory public immutable baseChildFactory;
    address public immutable settlementToken;
    AppealableVerifierV1 public immutable appealableVerifier;
    address public immutable configurator;
    address public parentFactory;
    bool public configured;

    mapping(address => bool) public isCanonicalChild;

    event ParentFactoryConfigured(address indexed parentFactory);
    event StandingMetaChildCreated(
        bytes32 indexed bountyId, address indexed child, address indexed creator, address parent, uint64 parentRound
    );

    constructor(address baseChildFactory_, address appealableVerifier_, address configurator_) {
        require(
            baseChildFactory_.code.length > 0 && appealableVerifier_.code.length > 0 && configurator_ != address(0),
            "child factory dependency missing"
        );
        baseChildFactory = AgentBountyFactory(baseChildFactory_);
        settlementToken = baseChildFactory.settlementToken();
        appealableVerifier = AppealableVerifierV1(appealableVerifier_);
        require(appealableVerifier.settlementToken() == settlementToken, "child factory token mismatch");
        configurator = configurator_;
    }

    function configureParentFactory(address parentFactory_) external {
        require(msg.sender == configurator && !configured, "configuration closed");
        require(parentFactory_.code.length > 0, "parent factory missing");
        IStandingMetaParentFactoryV4Configuration candidate =
            IStandingMetaParentFactoryV4Configuration(parentFactory_);
        require(
            candidate.childFactory() == address(baseChildFactory)
                && candidate.standingMetaChildFactory() == address(this)
                && candidate.appealableVerifier() == address(appealableVerifier),
            "parent factory wiring mismatch"
        );
        parentFactory = parentFactory_;
        configured = true;
        emit ParentFactoryConfigured(parentFactory_);
    }

    function createAndFund(
        address parentAddress,
        uint64 parentRound,
        address creator,
        AgentBountyFactory.CreateBountyParams calldata params,
        bytes32 creationNonce,
        AgentBountyFactory.FundingAuthorization calldata funding
    ) external returns (address childAddress, bytes32 childBountyId) {
        require(configured && msg.sender == parentFactory, "parent factory only");
        _validateParams(params);
        childBountyId = _childBountyId(parentAddress, parentRound, creator, params, creationNonce);
        StandingMetaChildV4 child = new StandingMetaChildV4{salt: childBountyId}(
            childBountyId,
            creator,
            address(this),
            settlementToken,
            address(appealableVerifier),
            params.termsHash,
            params.policyHash,
            params.acceptanceCriteriaHash,
            params.benchmarkHash,
            params.evidenceSchemaHash
        );
        childAddress = address(child);
        require(!isCanonicalChild[childAddress], "child already canonical");
        isCanonicalChild[childAddress] = true;
        settlementToken.safeTransferWithAuthorization(
            creator,
            childAddress,
            CHILD_TARGET,
            funding.validAfter,
            funding.validBefore,
            funding.nonce,
            funding.v,
            funding.r,
            funding.s
        );
        child.recordInitialFunding();
        emit StandingMetaChildCreated(childBountyId, childAddress, creator, parentAddress, parentRound);
    }

    function claimAuthorized(
        address childAddress,
        address solver,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 authorizationNonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external {
        require(configured && msg.sender == parentFactory, "parent factory only");
        require(isCanonicalChild[childAddress], "child not canonical");
        StandingMetaChildV4(childAddress).claimAuthorized(solver, validAfter, validBefore, authorizationNonce, v, r, s);
    }

    function predictChildAddress(
        address parentAddress,
        uint64 parentRound,
        address creator,
        AgentBountyFactory.CreateBountyParams calldata params,
        bytes32 creationNonce
    ) external view returns (address predicted) {
        bytes32 childBountyId = _childBountyId(parentAddress, parentRound, creator, params, creationNonce);
        bytes32 initCodeHash = keccak256(
            abi.encodePacked(
                type(StandingMetaChildV4).creationCode,
                abi.encode(
                    childBountyId,
                    creator,
                    address(this),
                    settlementToken,
                    address(appealableVerifier),
                    params.termsHash,
                    params.policyHash,
                    params.acceptanceCriteriaHash,
                    params.benchmarkHash,
                    params.evidenceSchemaHash
                )
            )
        );
        predicted = address(
            uint160(uint256(keccak256(abi.encodePacked(bytes1(0xff), address(this), childBountyId, initCodeHash))))
        );
    }

    function _childBountyId(
        address parentAddress,
        uint64 parentRound,
        address creator,
        AgentBountyFactory.CreateBountyParams calldata params,
        bytes32 creationNonce
    ) private view returns (bytes32) {
        require(parentAddress != address(0) && parentRound > 0 && creator != address(0), "child identity invalid");
        require(creationNonce != bytes32(0), "child creation nonce zero");
        return keccak256(
            abi.encode(
                keccak256("agent-bounties/standing-meta-child-v4"),
                block.chainid,
                address(this),
                parentAddress,
                parentRound,
                creator,
                creationNonce,
                params
            )
        );
    }

    function _validateParams(AgentBountyFactory.CreateBountyParams calldata params) private view {
        require(
            params.solverReward == CHILD_SOLVER_REWARD && params.verifierReward == CHILD_VERIFIER_REWARD
                && params.solverReward + params.verifierReward == CHILD_TARGET,
            "child economics invalid"
        );
        require(
            params.termsHash != bytes32(0) && params.policyHash != bytes32(0)
                && params.acceptanceCriteriaHash != bytes32(0) && params.benchmarkHash != bytes32(0)
                && params.evidenceSchemaHash != bytes32(0),
            "child commitment invalid"
        );
        require(
            uint8(params.verificationMode) == DETERMINISTIC_MODE && params.verifierModule == address(appealableVerifier)
                && params.verifierRewardRecipient == address(appealableVerifier) && params.threshold == 1,
            "child appeal policy invalid"
        );
        require(
            params.claimWindowSeconds == CHILD_WORK_WINDOW
                && params.verificationWindowSeconds == CHILD_VERIFICATION_WINDOW
                && params.fundingDeadline > block.timestamp,
            "child timing invalid"
        );
    }
}
