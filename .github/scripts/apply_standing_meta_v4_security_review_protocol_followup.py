#!/usr/bin/env python3
"""Protocol follow-up edits found during the PR #536 maintainer review."""

from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    value = file.read_text(encoding="utf-8")
    count = value.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one exact match, found {count}: {old[:120]!r}")
    file.write_text(value.replace(old, new, 1), encoding="utf-8")


def main() -> None:
    # Solver participation must never consume the verifier role's bounded capacity.
    replace_once(
        "contracts/base-escrow/src/AnonymousStakePoolV1.sol",
        '        require(totalActiveTickets < MAX_ACTIVE_TICKETS, "pool full");\n',
        '        require(_activeWallets[role].length < MAX_ACTIVE_TICKETS, "role pool full");\n',
    )

    # A V4 child is funded as part of an active parent claim. Its creator cannot
    # cancel it until the parent round has expired or been superseded.
    replace_once(
        "contracts/base-escrow/src/StandingMetaChildV4.sol",
        """    function cancel() external nonReentrant {
        require(msg.sender == creator && (_status == Status.Open || _status == Status.Claimable), "not cancellable");
        _status = Status.Cancelled;
        emit BountyCancelled(bountyId, fundedAmount + timeoutBondPool);
    }
""",
        """    function cancel() external onlyFactory nonReentrant {
        require(_status == Status.Open || _status == Status.Claimable, "not cancellable");
        _status = Status.Cancelled;
        emit BountyCancelled(bountyId, fundedAmount + timeoutBondPool);
    }
""",
    )
    replace_once(
        "contracts/base-escrow/src/StandingMetaChildFactoryV4.sol",
        """    function predictChildAddress(
""",
        """    function cancelAuthorized(address childAddress) external {
        require(configured && msg.sender == parentFactory, "parent factory only");
        require(isCanonicalChild[childAddress], "child not canonical");
        StandingMetaChildV4(childAddress).cancel();
    }

    function predictChildAddress(
""",
    )
    replace_once(
        "contracts/base-escrow/src/StandingMetaParentFactoryV4.sol",
        """    event ChildSolverPromoted(
        address indexed parent, uint64 indexed round, address indexed candidate, uint8 rank, bytes32 reason
    );
""",
        """    event ChildSolverPromoted(
        address indexed parent, uint64 indexed round, address indexed candidate, uint8 rank, bytes32 reason
    );
    event PreparedChildCancelled(
        address indexed parent, uint64 indexed round, address indexed child, address creator
    );
""",
    )
    replace_once(
        "contracts/base-escrow/src/StandingMetaParentFactoryV4.sol",
        """    function roundChild(address parent, uint64 parentRound) external view returns (address) {
""",
        """    function cancelExpiredChild(address parentAddress, uint64 parentRound) external nonReentrant {
        require(isCanonicalParent[parentAddress], "parent not canonical");
        StandingMetaParentV4 parent = StandingMetaParentV4(parentAddress);
        RoundData storage data = _rounds[parentAddress][parentRound];
        require(data.child != address(0), "prepared child missing");
        StandingMetaChildV4 child = StandingMetaChildV4(data.child);
        require(msg.sender == child.creator(), "child creator only");
        require(child.status() == CLAIMABLE_STATUS, "child not refundable");
        uint64 currentRound = parent.round();
        StandingMetaParentV4.Status parentStatus = parent.bountyStatus();
        bool currentRoundClosed = currentRound == parentRound
            && (parentStatus == StandingMetaParentV4.Status.Claimable
                || parentStatus == StandingMetaParentV4.Status.Cancelled);
        require(currentRound > parentRound || currentRoundClosed, "parent round still active");
        standingMetaChildFactory.cancelAuthorized(data.child);
        emit PreparedChildCancelled(parentAddress, parentRound, data.child, msg.sender);
    }

    function roundChild(address parent, uint64 parentRound) external view returns (address) {
""",
    )

    # Test actor helpers for the cancellation boundary.
    replace_once(
        "contracts/base-escrow/test/StandingMetaV4.t.sol",
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
        "contracts/base-escrow/test/StandingMetaV4.t.sol",
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

    # Capacity is bounded independently for solvers and verifiers.
    replace_once(
        "contracts/base-escrow/test/AnonymousStakeAndVrfSortition.t.sol",
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

    # The main patch first rewrites these rows; append the follow-up risks to the post-patch wording.
    replace_once(
        "docs/security/standing-meta-v4-threat-model.md",
        """| Candidate joins after seeing a target | Child solver candidates are the already-active, available pool snapshotted inside `claimAndCreateChild`; the selected ticket is rechecked immediately before claim | Availability may change after the snapshot; an ineligible selection waits for bounded promotion and never receives claim authority |
""",
        """| Candidate joins after seeing a target | Child solver candidates are the already-active, available pool snapshotted inside `claimAndCreateChild`; the selected ticket is rechecked immediately before claim | Availability may change after the snapshot; an ineligible selection waits for bounded promotion and never receives claim authority |
| One role consumes all bounded pool slots | The 64-ticket capacity is enforced independently for solver and verifier roles | A same-role Sybil can still fill that role at the fixed 5 USDC-per-wallet cost; readiness and monitoring must fail closed when healthy eligible depth is missing |
""",
    )
    replace_once(
        "docs/security/standing-meta-v4-threat-model.md",
        """| Atomic preparation race | Terms publication, child creation/funding, active-pool snapshot, VRF request, round binding, and parent claim occur in one transaction; selection time and commitment are derived onchain from the inclusion block | The transaction can revert for gas, authorization, pool-size, or subscription failures |
""",
        """| Atomic preparation race | Terms publication, child creation/funding, active-pool snapshot, VRF request, round binding, and parent claim occur in one transaction; selection time and commitment are derived onchain from the inclusion block | The transaction can revert for gas, authorization, pool-size, or subscription failures |
| Parent solver cancels the freshly funded child | Child cancellation is factory-only while the parent round is active; the child creator can recover only after that round expires, the parent is cancelled, or a newer round supersedes it | Recovery is permissionless only for the original child creator and still requires separate canonical cancellation/refund evidence |
""",
    )


if __name__ == "__main__":
    main()
