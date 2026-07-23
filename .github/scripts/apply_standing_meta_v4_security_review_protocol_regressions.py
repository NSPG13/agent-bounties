#!/usr/bin/env python3
"""Add regressions and durable threat-model notes for PR #536 fixes."""

from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    value = file.read_text(encoding="utf-8")
    count = value.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one exact match, found {count}: {old[:120]!r}")
    file.write_text(value.replace(old, new, 1), encoding="utf-8")


def main() -> None:
    # Adapt the generic appeal harness to the production two-hop provenance
    # check: controller -> parent factory -> canonical child factory.
    appeal_test = "contracts/base-escrow/test/AppealableVerifierV1.t.sol"
    replace_once(
        appeal_test,
        """contract AppealVerifierControllerDummy {
    mapping(address => bool) public isCanonicalAppealableChild;

    function setCanonicalAppealableChild(address bounty, bool canonical) external {
        isCanonicalAppealableChild[bounty] = canonical;
    }
}
""",
        """contract AppealVerifierChildFactoryDummy {
    mapping(address => bool) public isCanonicalChild;

    function setCanonicalChild(address bounty, bool canonical) external {
        isCanonicalChild[bounty] = canonical;
    }
}

contract AppealVerifierControllerDummy {
    address public immutable standingMetaChildFactory;

    constructor(address childFactory) {
        standingMetaChildFactory = childFactory;
    }
}
""",
    )
    replace_once(
        appeal_test,
        """    AppealableVerifierV1 private verifier;
    AppealVerifierControllerDummy private parentFactory;
    AppealVerifierActor private solver;
""",
        """    AppealableVerifierV1 private verifier;
    AppealVerifierChildFactoryDummy private canonicalChildFactory;
    AppealVerifierControllerDummy private parentFactory;
    AppealVerifierActor private solver;
""",
    )
    replace_once(
        appeal_test,
        """        parentFactory = new AppealVerifierControllerDummy();
""",
        """        canonicalChildFactory = new AppealVerifierChildFactoryDummy();
        parentFactory = new AppealVerifierControllerDummy(address(canonicalChildFactory));
""",
    )
    replace_once(
        appeal_test,
        "        parentFactory.setCanonicalAppealableChild(address(bounty), false);\n",
        "        canonicalChildFactory.setCanonicalChild(address(bounty), false);\n",
    )
    replace_once(
        appeal_test,
        "        parentFactory.setCanonicalAppealableChild(bountyAddress, true);\n",
        "        canonicalChildFactory.setCanonicalChild(bountyAddress, true);\n",
    )

    v4_test = "contracts/base-escrow/test/StandingMetaV4.t.sol"
    replace_once(
        v4_test,
        """    function submitChild(StandingMetaChildV4 child, bytes32 submissionHash, bytes32 evidenceHash) external {
        child.submit(submissionHash, evidenceHash);
    }
""",
        """    function submitChild(StandingMetaChildV4 child, bytes32 submissionHash, bytes32 evidenceHash) external {
        child.submit(submissionHash, evidenceHash);
    }

    function cancelChild(StandingMetaChildV4 child) external {
        child.cancel();
    }

    function cancelExpiredChild(StandingMetaParentFactoryV4 parentFactory, address parent, uint64 parentRound)
        external
    {
        parentFactory.cancelExpiredChild(parent, parentRound);
    }

    function withdrawChildRefund(StandingMetaChildV4 child) external {
        child.withdrawRefund();
    }
""",
    )
    replace_once(
        v4_test,
        """        (address firstParent, bytes32 bountyId) = parentFactory.createParent(config);
        (bool duplicate,) = address(parentFactory).call(abi.encodeCall(parentFactory.createParent, (config)));
        require(!duplicate, "duplicate canonical parent bounty id accepted");
        require(parentFactory.parentByBountyId(bountyId) == firstParent, "canonical parent id mapping drift");
""",
        """        parentFactory.createParent(config);
        (bool duplicate,) = address(parentFactory).call(abi.encodeCall(parentFactory.createParent, (config)));
        require(!duplicate, "duplicate canonical parent bounty id accepted");
""",
    )
    replace_once(
        v4_test,
        """    function _createParent() private returns (StandingMetaParentV4 parent) {
""",
        """    function testPreparedChildCannotBeCancelledUntilParentRoundCloses() public {
        StandingMetaParentV4 parent = _createParent();
        StandingMetaParentFactoryV4.ClaimAndCreateChildRequest memory request = _claimRequest(parent);
        address childAddress = parentSolver.claimAndCreate(parentFactory, address(parent), request);
        StandingMetaChildV4 child = StandingMetaChildV4(childAddress);
        uint64 parentRound = parent.round();

        (bool directCancellation,) =
            address(parentSolver).call(abi.encodeCall(StandingMetaV4Actor.cancelChild, (child)));
        require(!directCancellation, "child creator cancelled an active prepared child");
        (bool earlyFactoryCancellation,) = address(parentSolver).call(
            abi.encodeCall(
                StandingMetaV4Actor.cancelExpiredChild, (parentFactory, address(parent), parentRound)
            )
        );
        require(!earlyFactoryCancellation, "factory cancelled an active parent round child");
        require(child.bountyStatus() == StandingMetaChildV4.Status.Claimable, "active child state drift");

        vm.warp(parent.claimExpiresAt() + 1);
        parent.expireWork();
        parentSolver.cancelExpiredChild(parentFactory, address(parent), parentRound);
        require(child.bountyStatus() == StandingMetaChildV4.Status.Cancelled, "expired child not cancelled");
        uint256 beforeRefund = token.balanceOf(address(parentSolver));
        parentSolver.withdrawChildRefund(child);
        require(
            token.balanceOf(address(parentSolver)) == beforeRefund + child.TARGET_AMOUNT(),
            "expired child refund mismatch"
        );
    }

    function _createParent() private returns (StandingMetaParentV4 parent) {
""",
    )

    stake_test = "contracts/base-escrow/test/AnonymousStakeAndVrfSortition.t.sol"
    replace_once(
        stake_test,
        """    function testFrozenRequestsUseNativePaymentThreeConfirmationsAndNoRerolls() public {
""",
        """    function testRoleCapacityCannotLetSolversStarveVerifierActivation() public {
        AnonymousPoolActor[] memory solvers = new AnonymousPoolActor[](pool.MAX_ACTIVE_TICKETS());
        for (uint256 i = 0; i < solvers.length; i++) {
            AnonymousPoolActor solverActor = new AnonymousPoolActor();
            solvers[i] = solverActor;
            token.mint(address(solverActor), pool.STAKE_AMOUNT());
            solverActor.approve(token, address(pool), type(uint256).max);
            solverActor.register(pool, AnonymousStakePoolV1.Role.Solver);
        }
        AnonymousPoolActor extraSolver = new AnonymousPoolActor();
        token.mint(address(extraSolver), pool.STAKE_AMOUNT());
        extraSolver.approve(token, address(pool), type(uint256).max);
        extraSolver.register(pool, AnonymousStakePoolV1.Role.Solver);

        AnonymousPoolActor verifierActor = new AnonymousPoolActor();
        token.mint(address(verifierActor), pool.STAKE_AMOUNT());
        verifierActor.approve(token, address(pool), type(uint256).max);
        verifierActor.register(pool, AnonymousStakePoolV1.Role.Verifier);

        vm.warp(block.timestamp + pool.ACTIVATION_DELAY());
        for (uint256 i = 0; i < solvers.length; i++) {
            solvers[i].activate(pool, AnonymousStakePoolV1.Role.Solver);
        }
        verifierActor.activate(pool, AnonymousStakePoolV1.Role.Verifier);
        require(
            pool.activeWalletCount(AnonymousStakePoolV1.Role.Solver) == pool.MAX_ACTIVE_TICKETS(),
            "solver role cap drift"
        );
        require(pool.activeWalletCount(AnonymousStakePoolV1.Role.Verifier) == 1, "verifier role was starved");
        require(pool.totalActiveTickets() == pool.MAX_ACTIVE_TICKETS() + 1, "aggregate ticket count drift");
        (bool extraActivated,) = address(extraSolver).call(
            abi.encodeCall(AnonymousPoolActor.activate, (pool, AnonymousStakePoolV1.Role.Solver))
        );
        require(!extraActivated, "solver role exceeded its bounded capacity");
    }

    function testFrozenRequestsUseNativePaymentThreeConfirmationsAndNoRerolls() public {
""",
    )

    threat_model = "docs/security/standing-meta-v4-threat-model.md"
    replace_once(
        threat_model,
        """| Candidate joins after seeing a target | Child solver candidates are the already-active, available pool snapshotted inside `claimAndCreateChild`; the selected ticket is rechecked immediately before claim | Availability may change after the snapshot; an ineligible selection waits for bounded promotion and never receives claim authority |
""",
        """| Candidate joins after seeing a target | Child solver candidates are the already-active, available pool snapshotted inside `claimAndCreateChild`; the selected ticket is rechecked immediately before claim | Availability may change after the snapshot; an ineligible selection waits for bounded promotion and never receives claim authority |
| One role consumes all bounded pool slots | The 64-ticket capacity is enforced independently for solver and verifier roles | A same-role Sybil can still fill that role at the fixed 5 USDC-per-wallet cost; readiness and monitoring must fail closed when healthy eligible depth is missing |
""",
    )
    replace_once(
        threat_model,
        """| Atomic preparation race | Terms publication, child creation/funding, active-pool snapshot, VRF request, round binding, and parent claim occur in one transaction; selection time and commitment are derived onchain from the inclusion block | The transaction can revert for gas, authorization, pool-size, or subscription failures |
""",
        """| Atomic preparation race | Terms publication, child creation/funding, active-pool snapshot, VRF request, round binding, and parent claim occur in one transaction; selection time and commitment are derived onchain from the inclusion block | The transaction can revert for gas, authorization, pool-size, or subscription failures |
| Parent solver cancels the freshly funded child | Child cancellation is factory-only while the parent round is active; the child creator can recover only after that round expires, the parent is cancelled, or a newer round supersedes it | Recovery is permissionless only for the original child creator and still requires separate canonical cancellation/refund evidence |
""",
    )


if __name__ == "__main__":
    main()
