// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./AnonymousStakePoolV1.sol";
import "./VrfSortitionCoordinatorV1.sol";

/// @notice One-time-wired capability router for the immutable fair-earning
/// modules. It has no owner, upgrade, withdrawal, or post-configuration setter.
contract AnonymousProtocolControllerV1 {
    address public immutable configurator;
    AnonymousStakePoolV1 public stakePool;
    VrfSortitionCoordinatorV1 public verifierSortition;
    VrfSortitionCoordinatorV1 public solverSortition;
    address public appealableVerifier;
    address public standingMetaParentFactory;
    bool public configured;

    event ProtocolConfigured(
        address indexed stakePool,
        address indexed verifierSortition,
        address indexed solverSortition,
        address appealableVerifier,
        address standingMetaParentFactory
    );

    constructor(address configurator_) {
        require(configurator_ != address(0), "configurator zero");
        configurator = configurator_;
    }

    function configure(
        address stakePool_,
        address verifierSortition_,
        address solverSortition_,
        address appealableVerifier_,
        address standingMetaParentFactory_
    ) external {
        require(msg.sender == configurator && !configured, "configuration closed");
        require(
            stakePool_.code.length > 0 && verifierSortition_.code.length > 0 && solverSortition_.code.length > 0
                && appealableVerifier_.code.length > 0 && standingMetaParentFactory_.code.length > 0,
            "component missing"
        );
        require(AnonymousStakePoolV1(stakePool_).controller() == address(this), "pool controller mismatch");
        require(
            VrfSortitionCoordinatorV1(verifierSortition_).controller() == address(this)
                && VrfSortitionCoordinatorV1(solverSortition_).controller() == address(this),
            "sortition controller mismatch"
        );
        stakePool = AnonymousStakePoolV1(stakePool_);
        verifierSortition = VrfSortitionCoordinatorV1(verifierSortition_);
        solverSortition = VrfSortitionCoordinatorV1(solverSortition_);
        appealableVerifier = appealableVerifier_;
        standingMetaParentFactory = standingMetaParentFactory_;
        configured = true;
        emit ProtocolConfigured(
            stakePool_, verifierSortition_, solverSortition_, appealableVerifier_, standingMetaParentFactory_
        );
    }

    function eligibleVerifierWallets(address[] calldata exclusions) external view returns (address[] memory) {
        require(msg.sender == appealableVerifier, "appealable verifier only");
        return stakePool.eligibleWallets(AnonymousStakePoolV1.Role.Verifier, exclusions);
    }

    function eligibleSolverWallets(address[] calldata exclusions) external view returns (address[] memory) {
        require(msg.sender == standingMetaParentFactory, "parent factory only");
        return stakePool.eligibleWallets(AnonymousStakePoolV1.Role.Solver, exclusions);
    }

    function requestVerifierSortition(bytes32 commitment, address[] calldata candidates, uint8 selectionCount)
        external
        returns (uint256)
    {
        require(msg.sender == appealableVerifier, "appealable verifier only");
        return verifierSortition.freezeAndRequest(commitment, candidates, selectionCount);
    }

    function requestSolverSortition(bytes32 commitment, address[] calldata candidates, uint8 selectionCount)
        external
        returns (uint256)
    {
        require(msg.sender == standingMetaParentFactory, "parent factory only");
        return solverSortition.freezeAndRequest(commitment, candidates, selectionCount);
    }

    function lockVerifierStake(bytes32 caseId, address wallet, uint256 amount) external {
        require(msg.sender == appealableVerifier, "appealable verifier only");
        require(amount > 0 && amount <= 100_000, "lock amount invalid");
        stakePool.lock(caseId, wallet, AnonymousStakePoolV1.Role.Verifier, amount);
    }

    function releaseVerifierStake(bytes32 caseId, address wallet) external returns (uint256) {
        require(msg.sender == appealableVerifier, "appealable verifier only");
        return stakePool.release(caseId, wallet, AnonymousStakePoolV1.Role.Verifier);
    }

    function slashVerifierStake(bytes32 caseId, address wallet, uint256 amount, address recipient) external {
        require(msg.sender == appealableVerifier, "appealable verifier only");
        require(recipient == appealableVerifier || recipient == address(stakePool), "slash recipient invalid");
        stakePool.slash(caseId, wallet, AnonymousStakePoolV1.Role.Verifier, amount, recipient);
    }
}
