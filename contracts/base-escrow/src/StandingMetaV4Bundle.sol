// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./AgentBountyFactory.sol";
import "./AnonymousProtocolControllerV1.sol";
import "./AnonymousStakePoolV1.sol";
import "./AppealableVerifierV1.sol";
import "./StandingMetaParentFactoryV4.sol";
import "./StandingMetaChildFactoryV4.sol";
import "./VrfSortitionCoordinatorV1.sol";

/// @notice Immutable manifest that validates a staged V4 deployment. Staging
/// keeps every deployment transaction below the EIP-3860 initcode limit.
/// Subscription funding and consumer authorization are external Chainlink
/// readiness requirements and are never inferred from this bundle.
contract StandingMetaV4Bundle {
    AgentBountyFactory public immutable childFactory;
    AnonymousProtocolControllerV1 public immutable controller;
    AnonymousStakePoolV1 public immutable stakePool;
    VrfSortitionCoordinatorV1 public immutable verifierSortition;
    VrfSortitionCoordinatorV1 public immutable solverSortition;
    AppealableVerifierV1 public immutable appealableVerifier;
    StandingMetaChildFactoryV4 public immutable standingMetaChildFactory;
    StandingMetaParentFactoryV4 public immutable parentFactory;

    constructor(
        address childFactory_,
        address controller_,
        address stakePool_,
        address verifierSortition_,
        address solverSortition_,
        address appealableVerifier_,
        address standingMetaChildFactory_,
        address parentFactory_
    ) {
        require(
            childFactory_.code.length > 0 && controller_.code.length > 0 && stakePool_.code.length > 0
                && verifierSortition_.code.length > 0 && solverSortition_.code.length > 0
                && appealableVerifier_.code.length > 0 && standingMetaChildFactory_.code.length > 0
                && parentFactory_.code.length > 0,
            "bundle dependency missing"
        );
        childFactory = AgentBountyFactory(childFactory_);
        controller = AnonymousProtocolControllerV1(controller_);
        stakePool = AnonymousStakePoolV1(stakePool_);
        verifierSortition = VrfSortitionCoordinatorV1(verifierSortition_);
        solverSortition = VrfSortitionCoordinatorV1(solverSortition_);
        appealableVerifier = AppealableVerifierV1(appealableVerifier_);
        standingMetaChildFactory = StandingMetaChildFactoryV4(standingMetaChildFactory_);
        parentFactory = StandingMetaParentFactoryV4(parentFactory_);

        require(controller.configured(), "controller not configured");
        require(
            address(controller.stakePool()) == stakePool_
                && address(controller.verifierSortition()) == verifierSortition_
                && address(controller.solverSortition()) == solverSortition_
                && controller.appealableVerifier() == appealableVerifier_
                && controller.standingMetaParentFactory() == parentFactory_,
            "controller wiring mismatch"
        );
        require(
            stakePool.controller() == controller_ && stakePool.settlementToken() == childFactory.settlementToken(),
            "stake pool wiring mismatch"
        );
        require(
            verifierSortition.controller() == controller_ && solverSortition.controller() == controller_,
            "sortition wiring mismatch"
        );
        require(
            address(appealableVerifier.controller()) == controller_
                && address(appealableVerifier.sortition()) == verifierSortition_
                && appealableVerifier.settlementToken() == childFactory.settlementToken(),
            "appeal wiring mismatch"
        );
        require(
            standingMetaChildFactory.configured() && standingMetaChildFactory.parentFactory() == parentFactory_
                && address(standingMetaChildFactory.baseChildFactory()) == childFactory_
                && address(standingMetaChildFactory.appealableVerifier()) == appealableVerifier_,
            "child factory wiring mismatch"
        );
        require(
            address(parentFactory.childFactory()) == childFactory_ && address(parentFactory.controller()) == controller_
                && address(parentFactory.appealableVerifier()) == appealableVerifier_
                && address(parentFactory.standingMetaChildFactory()) == standingMetaChildFactory_
                && parentFactory.verifierModule().canonicalChildFactory() == standingMetaChildFactory_,
            "parent factory wiring mismatch"
        );
    }
}
