#!/usr/bin/env python3
"""Apply the bounded maintainer security patch reviewed against PR #536.

Every edit is an exact one-time replacement against the reviewed head. The
workflow checks that head SHA before invoking this script and pushes only after
adversarial tests and size checks pass.
"""

from __future__ import annotations

from pathlib import Path


def read(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


def write(path: str, value: str) -> None:
    Path(path).write_text(value, encoding="utf-8")


def replace_once(path: str, old: str, new: str) -> None:
    value = read(path)
    count = value.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one exact match, found {count}: {old[:120]!r}")
    write(path, value.replace(old, new, 1))


def replace_region(path: str, start_marker: str, end_marker: str, replacement: str) -> None:
    value = read(path)
    start = value.find(start_marker)
    if start < 0:
        raise SystemExit(f"{path}: start marker missing: {start_marker}")
    end = value.find(end_marker, start)
    if end < 0:
        raise SystemExit(f"{path}: end marker missing: {end_marker}")
    write(path, value[:start] + replacement + value[end:])


def patch_controller() -> None:
    replace_once(
        "contracts/base-escrow/src/AnonymousProtocolControllerV1.sol",
        """            stakePool_.code.length > 0 && verifierSortition_.code.length > 0 && solverSortition_.code.length > 0
                && appealableVerifier_.code.length > 0 && standingMetaParentFactory_.code.length > 0,
""",
        """            stakePool_.code.length > 0 && verifierSortition_.code.length > 0 && solverSortition_.code.length > 0
                && verifierSortition_ != solverSortition_ && appealableVerifier_.code.length > 0
                && standingMetaParentFactory_.code.length > 0,
""",
    )


def patch_appealable_verifier() -> None:
    path = "contracts/base-escrow/src/AppealableVerifierV1.sol"
    replace_once(
        path,
        """}

/// @notice Anonymous primary-verifier assignment with symmetric, one-round
""",
        """}

interface ICanonicalAppealableChildRegistryV1 {
    function isCanonicalAppealableChild(address bounty) external view returns (bool);
}

/// @notice Anonymous primary-verifier assignment with symmetric, one-round
""",
    )
    replace_once(
        path,
        """        uint64 voteDeadline;
        uint64 openedAt;
        uint256 primaryRequestId;
""",
        """        uint64 voteDeadline;
        uint64 openedAt;
        uint64 bountyVerificationDeadline;
        uint256 primaryRequestId;
""",
    )
    replace_once(
        path,
        """        IAppealableBountyV1 item = IAppealableBountyV1(bounty);
        _validateBounty(item);
""",
        """        IAppealableBountyV1 item = IAppealableBountyV1(bounty);
        _validateBounty(bounty, item);
""",
    )
    replace_once(
        path,
        """    function _validateBounty(IAppealableBountyV1 item) private view {
        require(item.settlementToken() == settlementToken, "token mismatch");
""",
        """    function _validateBounty(address bounty, IAppealableBountyV1 item) private view {
        address registry = controller.standingMetaParentFactory();
        require(
            controller.configured() && registry.code.length > 0
                && ICanonicalAppealableChildRegistryV1(registry).isCanonicalAppealableChild(bounty),
            "bounty not canonical"
        );
        require(item.settlementToken() == settlementToken, "token mismatch");
""",
    )
    replace_once(
        path,
        """        require(item.state == CaseState.AwaitingPrimaryRandomness, "primary activation unavailable");
        address[] memory ranking = sortition.selected(item.primaryRequestId);
""",
        """        require(item.state == CaseState.AwaitingPrimaryRandomness, "primary activation unavailable");
        if (!_canAssignPrimary(item)) {
            item.state = CaseState.TimedOut;
            emit VerificationTimedOut(caseId, keccak256("primary-activation-too-late"));
            return;
        }
        address[] memory ranking = sortition.selected(item.primaryRequestId);
""",
    )
    replace_once(
        path,
        """        require(item.state == CaseState.AwaitingAppealRandomness, "appeal activation unavailable");
        address[] memory ranking = sortition.ranking(item.appealRequestId);
""",
        """        require(item.state == CaseState.AwaitingAppealRandomness, "appeal activation unavailable");
        if (
            uint256(block.timestamp) + VOTING_WINDOW + CASE_COMPLETION_BUFFER
                > item.bountyVerificationDeadline
        ) {
            _timeoutAppeal(caseId, item, keccak256("appeal-activation-too-late"));
            return;
        }
        address[] memory ranking = sortition.ranking(item.appealRequestId);
""",
    )
    replace_once(
        path,
        """        _closeJuryLocks(caseId);
        item.state = CaseState.Finalized;
""",
        """        _closeJuryLocks(caseId, block.timestamp > item.voteDeadline);
        item.state = CaseState.Finalized;
""",
    )
    replace_once(
        path,
        """        opened.openedAt = uint64(block.timestamp);
        opened.primaryRequestId = requestId;
""",
        """        opened.openedAt = uint64(block.timestamp);
        opened.bountyVerificationDeadline = item.verificationExpiresAt();
        opened.primaryRequestId = requestId;
""",
    )
    replace_once(
        path,
        """    function _assignPrimary(bytes32 caseId, VerificationCase storage item, uint8 rank) private {
        for (uint8 cursor = rank; cursor < PRIMARY_RANKING_SIZE; cursor++) {
""",
        """    function _canAssignPrimary(VerificationCase storage item) private view returns (bool) {
        return uint256(block.timestamp) + RESPONSE_WINDOW + APPEAL_WINDOW + VRF_FULFILLMENT_WINDOW
            + VOTING_WINDOW + CASE_COMPLETION_BUFFER <= item.bountyVerificationDeadline;
    }

    function _assignPrimary(bytes32 caseId, VerificationCase storage item, uint8 rank) private {
        if (!_canAssignPrimary(item)) {
            item.state = CaseState.TimedOut;
            emit VerificationTimedOut(caseId, keccak256("primary-assignment-too-late"));
            return;
        }
        for (uint8 cursor = rank; cursor < PRIMARY_RANKING_SIZE; cursor++) {
""",
    )
    replace_once(
        path,
        """        if (item.state == CaseState.AppealVoting) _closeJuryLocks(caseId);
""",
        """        if (item.state == CaseState.AppealVoting) _closeJuryLocks(caseId, true);
""",
    )
    replace_once(
        path,
        """    function _closeJuryLocks(bytes32 caseId) private {
        address[] storage wallets = _appellateWallets[caseId];
        for (uint256 i = 0; i < wallets.length; i++) {
            address wallet = wallets[i];
            bytes32 lockId = _juryLockId(caseId, wallet);
            if (voted[caseId][wallet]) {
                controller.releaseVerifierStake(lockId, wallet);
            } else {
                controller.slashVerifierStake(lockId, wallet, AVAILABILITY_SLASH, address(controller.stakePool()));
            }
        }
    }
""",
        """    function _closeJuryLocks(bytes32 caseId, bool slashNonVoters) private {
        address[] storage wallets = _appellateWallets[caseId];
        for (uint256 i = 0; i < wallets.length; i++) {
            address wallet = wallets[i];
            bytes32 lockId = _juryLockId(caseId, wallet);
            if (voted[caseId][wallet] || !slashNonVoters) {
                controller.releaseVerifierStake(lockId, wallet);
            } else {
                controller.slashVerifierStake(lockId, wallet, AVAILABILITY_SLASH, address(controller.stakePool()));
            }
        }
    }
""",
    )


def patch_parent_factory() -> None:
    path = "contracts/base-escrow/src/StandingMetaParentFactoryV4.sol"
    replace_once(
        path,
        """    mapping(address => bool) public isCanonicalParent;
""",
        """    mapping(address => bool) public isCanonicalParent;
    mapping(bytes32 => address) public parentByBountyId;
""",
    )
    replace_once(
        path,
        """        bountyId = keccak256(abi.encode(block.chainid, address(this), msg.sender, config.creationNonce, config));
        StandingMetaParentV4 parent = new StandingMetaParentV4(
""",
        """        bountyId = keccak256(abi.encode(block.chainid, address(this), msg.sender, config.creationNonce, config));
        require(parentByBountyId[bountyId] == address(0), "parent bounty id already used");
        StandingMetaParentV4 parent = new StandingMetaParentV4(
""",
    )
    replace_once(
        path,
        """        parentAddress = address(parent);
        isCanonicalParent[parentAddress] = true;
""",
        """        parentAddress = address(parent);
        parentByBountyId[bountyId] = parentAddress;
        isCanonicalParent[parentAddress] = true;
""",
    )
    replace_once(
        path,
        """        PreparationContext memory context = _preparationContext(parentAddress, parent, request, candidateHash);
        _validateTermsInput(request.terms, parent, context, request.childParams);
        bytes32 publishedTermsHash = termsRegistry.publishFor(msg.sender, request.canonicalTerms, request.terms);
""",
        """        PreparationContext memory context = _preparationContext(parentAddress, parent, request, candidateHash);
        OnchainTermsRegistryV4.TermsInput memory resolvedTerms =
            _resolvedTermsInput(request.terms, parent, context, request.childParams);
        bytes32 publishedTermsHash = termsRegistry.publishFor(msg.sender, request.canonicalTerms, resolvedTerms);
""",
    )
    replace_once(
        path,
        """        require(_ranking[parentAddress][parentRound][data.currentRank] == msg.sender, "candidate not selected");
        require(StandingMetaChildV4(data.child).status() == CLAIMABLE_STATUS, "child not claimable");
""",
        """        require(_ranking[parentAddress][parentRound][data.currentRank] == msg.sender, "candidate not selected");
        _requireCurrentlyEligibleSolver(msg.sender);
        require(StandingMetaChildV4(data.child).status() == CLAIMABLE_STATUS, "child not claimable");
""",
    )
    replace_once(
        path,
        """    function authorizedChildSolver(address parent, uint64 parentRound, address child, address solver)
""",
        """    function isCanonicalAppealableChild(address bounty) external view returns (bool) {
        return standingMetaChildFactory.isCanonicalChild(bounty);
    }

    function authorizedChildSolver(address parent, uint64 parentRound, address child, address solver)
""",
    )
    replace_region(
        path,
        "    function _validateTermsInput(",
        "    function _preparationContext(",
        """    function _resolvedTermsInput(
        OnchainTermsRegistryV4.TermsInput calldata terms,
        StandingMetaParentV4 parent,
        PreparationContext memory context,
        AgentBountyFactory.CreateBountyParams calldata params
    ) private view returns (OnchainTermsRegistryV4.TermsInput memory resolved) {
        require(
            terms.selectionCommitment == bytes32(0) && terms.selectionRequestedAt == 0,
            "selection fields must be derived"
        );
        resolved = terms;
        resolved.selectionCommitment = context.selectionCommitment;
        resolved.selectionRequestedAt = context.selectionRequestedAt;
        require(
            resolved.parent == address(parent) && resolved.child == context.predictedChild
                && resolved.parentBountyId == parent.bountyId() && resolved.parentRound == context.parentRound,
            "terms binding invalid"
        );
        require(
            resolved.verifierModule == address(appealableVerifier) && resolved.policyHash == params.policyHash
                && resolved.acceptanceCriteriaHash == params.acceptanceCriteriaHash
                && resolved.benchmarkHash == params.benchmarkHash
                && resolved.evidenceSchemaHash == params.evidenceSchemaHash
                && resolved.appealPolicyHash == appealableVerifier.appealPolicyHash(),
            "terms content invalid"
        );
        require(
            resolved.childClaimWindowSeconds == CHILD_WORK_WINDOW
                && resolved.childVerificationWindowSeconds == CHILD_VERIFICATION_WINDOW
                && resolved.childFundingTarget == CHILD_TARGET && resolved.childSolverReward == CHILD_SOLVER_REWARD
                && resolved.childVerifierReward == CHILD_VERIFIER_REWARD,
            "terms economics invalid"
        );
    }

""",
    )
    replace_once(
        path,
        """    function _eligibleChildSolvers(StandingMetaParentV4 parent, address parentSolver)
""",
        """    function _requireCurrentlyEligibleSolver(address wallet) private view {
        (uint128 stake,,, uint64 unbondAt, bool active, bool available) =
            controller.stakePool().tickets(wallet, AnonymousStakePoolV1.Role.Solver);
        require(
            stake == controller.stakePool().STAKE_AMOUNT() && unbondAt == 0 && active && available,
            "candidate no longer eligible"
        );
    }

    function _eligibleChildSolvers(StandingMetaParentV4 parent, address parentSolver)
""",
    )


def patch_bounty_accounting_and_reward() -> None:
    replace_once(
        "contracts/base-escrow/src/StandingMetaParentV4.sol",
        """        require(IERC20BountyToken(settlementToken).balanceOf(address(this)) == TARGET_AMOUNT, "funding mismatch");
""",
        """        require(IERC20BountyToken(settlementToken).balanceOf(address(this)) >= TARGET_AMOUNT, "funding mismatch");
""",
    )
    replace_once(
        "contracts/base-escrow/src/StandingMetaParentV4.sol",
        """            IERC20BountyToken(settlementToken).balanceOf(address(this)) == TARGET_AMOUNT + VERIFIER_REWARD,
""",
        """            IERC20BountyToken(settlementToken).balanceOf(address(this)) >= TARGET_AMOUNT + VERIFIER_REWARD,
""",
    )
    path = "contracts/base-escrow/src/StandingMetaChildV4.sol"
    replace_once(
        path,
        """import "./IAgentBounty.sol";

/// @notice Exact-economics V4 child whose claim authority is restricted to the
""",
        """import "./IAgentBounty.sol";

interface IAppealableVerifierRewardAllocatorV1 {
    function allocateVerifierReward(bytes32 caseId) external;
}

/// @notice Exact-economics V4 child whose claim authority is restricted to the
""",
    )
    replace_once(
        path,
        """        require(IERC20BountyToken(settlementToken).balanceOf(address(this)) == TARGET_AMOUNT, "funding mismatch");
""",
        """        require(IERC20BountyToken(settlementToken).balanceOf(address(this)) >= TARGET_AMOUNT, "funding mismatch");
""",
    )
    replace_once(
        path,
        """    function verifyAndSettle(bytes calldata proof) external nonReentrant {
        require(_status == Status.Submitted && block.timestamp <= verificationExpiresAt, "verification unavailable");
        (bool passed, bytes32 responseHash) = IAgentBountyVerifier(verifierModule)
            .verify(bountyId, round, solver, submissionHash, evidenceHash, policyHash, proof);
        bytes32 verificationHash = keccak256(abi.encode(verifierModule, responseHash, keccak256(proof)));
        if (passed) _settle(verificationHash);
        else _reject(verificationHash);
    }
""",
        """    function verifyAndSettle(bytes calldata proof) external nonReentrant {
        require(_status == Status.Submitted && block.timestamp <= verificationExpiresAt, "verification unavailable");
        require(proof.length == 32, "case proof invalid");
        bytes32 caseId = abi.decode(proof, (bytes32));
        (bool passed, bytes32 responseHash) = IAgentBountyVerifier(verifierModule)
            .verify(bountyId, round, solver, submissionHash, evidenceHash, policyHash, proof);
        bytes32 verificationHash = keccak256(abi.encode(verifierModule, responseHash, keccak256(proof)));
        if (passed) _settle(verificationHash);
        else _reject(verificationHash);
        IAppealableVerifierRewardAllocatorV1(verifierModule).allocateVerifierReward(caseId);
    }
""",
    )


def patch_subscription_funding() -> None:
    path = "contracts/base-escrow/script/FundStandingMetaV4Subscription.s.sol"
    replace_once(
        path,
        """}

/// @notice Funds one already-created V4 subscription with the exact native
""",
        """}

interface IStandingMetaV4SortitionFundingView {
    function vrfCoordinator() external view returns (address);
    function controller() external view returns (address);
    function subscriptionId() external view returns (uint256);
}

interface IStandingMetaV4ControllerFundingView {
    function configured() external view returns (bool);
    function verifierSortition() external view returns (address);
    function solverSortition() external view returns (address);
}

/// @notice Funds one already-created V4 subscription with the exact native
""",
    )
    replace_once(
        path,
        """    function _validateSubscription(FundingContext memory context) private view returns (uint96) {
        uint96 nativeBefore;
        address subscriptionOwner;
        address[] memory consumers;
""",
        """    function _validateSubscription(FundingContext memory context) private view returns (uint96) {
        require(
            context.verifierSortition.code.length > 0 && context.solverSortition.code.length > 0,
            "consumer code missing"
        );
        IStandingMetaV4SortitionFundingView verifier =
            IStandingMetaV4SortitionFundingView(context.verifierSortition);
        IStandingMetaV4SortitionFundingView solver = IStandingMetaV4SortitionFundingView(context.solverSortition);
        address protocolController = verifier.controller();
        require(
            protocolController != address(0) && protocolController.code.length > 0
                && solver.controller() == protocolController && verifier.vrfCoordinator() == context.vrf
                && solver.vrfCoordinator() == context.vrf && verifier.subscriptionId() == context.subscriptionId
                && solver.subscriptionId() == context.subscriptionId,
            "consumer wiring mismatch"
        );
        IStandingMetaV4ControllerFundingView controller = IStandingMetaV4ControllerFundingView(protocolController);
        require(
            controller.configured() && controller.verifierSortition() == context.verifierSortition
                && controller.solverSortition() == context.solverSortition,
            "controller wiring mismatch"
        );

        uint96 nativeBefore;
        address subscriptionOwner;
        address[] memory consumers;
""",
    )


def patch_release_tool_redaction() -> None:
    path = "scripts/standing_meta_v4_deploy.py"
    replace_once(
        path,
        """def run(command: Sequence[str], *, cwd: Path, timeout: int = 300) -> str:
""",
        """def redacted_output(output: str, command: Sequence[str]) -> str:
    rendered = output
    for secret_flag in ("--private-key", "--rpc-url"):
        for index, item in enumerate(command[:-1]):
            if item == secret_flag and command[index + 1]:
                rendered = rendered.replace(command[index + 1], "[redacted]")
    return rendered


def run(command: Sequence[str], *, cwd: Path, timeout: int = 300) -> str:
""",
    )
    replace_once(
        path,
        """            f"command failed ({completed.returncode}): {redacted_command(command)}\n{completed.stdout[-6000:]}"
""",
        """            f"command failed ({completed.returncode}): {redacted_command(command)}\n"
            f"{redacted_output(completed.stdout, command)[-6000:]}"
""",
    )
    replace_once(
        "scripts/test_standing_meta_v4_deploy.py",
        """    def test_networks_pin_official_vrf_configuration(self) -> None:
""",
        """    def test_release_errors_redact_signer_and_rpc_credentials_from_output(self) -> None:
        command = [
            "cast",
            "send",
            "--private-key",
            "0xsupersecret",
            "--rpc-url",
            "https://rpc.example/private-token",
        ]
        rendered = MODULE.redacted_output(
            "failed with 0xsupersecret at https://rpc.example/private-token", command
        )
        self.assertNotIn("supersecret", rendered)
        self.assertNotIn("private-token", rendered)
        self.assertEqual(rendered.count("[redacted]"), 2)

    def test_networks_pin_official_vrf_configuration(self) -> None:
""",
    )


def patch_appeal_tests() -> None:
    path = "contracts/base-escrow/test/AppealableVerifierV1.t.sol"
    replace_once(
        path,
        """contract AppealVerifierControllerDummy {}
""",
        """contract AppealVerifierControllerDummy {
    mapping(address => bool) public isCanonicalAppealableChild;

    function setCanonicalAppealableChild(address bounty, bool canonical) external {
        isCanonicalAppealableChild[bounty] = canonical;
    }
}
""",
    )
    replace_once(
        path,
        """    AppealableVerifierV1 private verifier;
    AppealVerifierActor private solver;
""",
        """    AppealableVerifierV1 private verifier;
    AppealVerifierControllerDummy private parentFactory;
    AppealVerifierActor private solver;
""",
    )
    replace_once(
        path,
        """        AppealVerifierControllerDummy parentFactory = new AppealVerifierControllerDummy();
""",
        """        parentFactory = new AppealVerifierControllerDummy();
""",
    )
    replace_once(
        path,
        """        require(juryCredits == 110_000, "slash and verifier reward not shared");
    }
""",
        """        require(juryCredits == 110_000, "slash and verifier reward not shared");
        for (uint256 i = 3; i < jury.length; i++) {
            (uint128 stake, uint128 locked,,, bool active,) =
                pool.tickets(jury[i], AnonymousStakePoolV1.Role.Verifier);
            require(stake == 5_000_000 && locked == 0 && active, "early nonvoter was slashed");
        }
    }
""",
    )
    replace_once(
        path,
        """    function _prepareCase(uint256 randomWord) private returns (AgentBounty bounty, bytes32 caseId) {
""",
        """    function testUnregisteredInterfaceShapedBountyCannotConsumeVrfOrStake() public {
        AgentBounty bounty = _createSubmittedBounty();
        parentFactory.setCanonicalAppealableChild(address(bounty), false);
        uint256 requestBefore = vrf.nextRequestId();
        (bool opened,) = address(verifier).call(abi.encodeCall(verifier.openCase, (address(bounty))));
        require(!opened, "unregistered bounty opened a verification case");
        require(vrf.nextRequestId() == requestBefore, "unregistered bounty consumed VRF");
    }

    function testDelayedPrimaryActivationTimesOutWithoutLockingVerifiers() public {
        AgentBounty bounty = _createSubmittedBounty();
        (bytes32 caseId,) = verifier.openCase(address(bounty));
        _fulfillLatestAndDerive(909);
        uint256 futureWindow = uint256(verifier.RESPONSE_WINDOW()) + uint256(verifier.APPEAL_WINDOW())
            + uint256(verifier.VRF_FULFILLMENT_WINDOW()) + uint256(verifier.VOTING_WINDOW())
            + uint256(verifier.CASE_COMPLETION_BUFFER());
        vm.warp(uint256(bounty.verificationExpiresAt()) - futureWindow + 1);
        verifier.activatePrimary(caseId);
        require(verifier.caseState(caseId) == AppealableVerifierV1.CaseState.TimedOut, "late primary stayed live");
        for (uint256 i = 0; i < verifierActors.length; i++) {
            (uint128 stake, uint128 locked,,, bool active,) =
                pool.tickets(address(verifierActors[i]), AnonymousStakePoolV1.Role.Verifier);
            require(stake == 5_000_000 && locked == 0 && active, "late activation touched verifier stake");
        }
    }

    function testDelayedAppealActivationRefundsBondAndReleasesPrimary() public {
        (AgentBounty bounty, bytes32 caseId) = _prepareCase(919);
        (,,, address primary,) = verifier.caseParties(caseId);
        AppealVerifierActor(primary).verdict(verifier, caseId, false, keccak256("late-appeal-primary"));
        solver.approve(token, address(verifier), verifier.APPEAL_BOND());
        solver.appeal(verifier, caseId);
        _fulfillLatestAndDerive(929);
        uint256 latest = uint256(bounty.verificationExpiresAt()) - uint256(verifier.VOTING_WINDOW())
            - uint256(verifier.CASE_COMPLETION_BUFFER());
        vm.warp(latest + 1);
        verifier.activateAppeal(caseId);
        require(verifier.caseState(caseId) == AppealableVerifierV1.CaseState.TimedOut, "late appeal stayed live");
        require(verifier.credits(address(solver)) == verifier.APPEAL_BOND(), "late appeal bond not refunded");
        (, uint128 primaryLocked,,,,) = pool.tickets(primary, AnonymousStakePoolV1.Role.Verifier);
        require(primaryLocked == 0, "late appeal kept primary locked");
    }

    function _prepareCase(uint256 randomWord) private returns (AgentBounty bounty, bytes32 caseId) {
""",
    )
    replace_once(
        path,
        """        bounty = AgentBounty(bountyAddress);
        solver.approve(token, address(bounty), type(uint256).max);
""",
        """        bounty = AgentBounty(bountyAddress);
        parentFactory.setCanonicalAppealableChild(bountyAddress, true);
        solver.approve(token, address(bounty), type(uint256).max);
""",
    )


def patch_v4_tests() -> None:
    path = "contracts/base-escrow/test/StandingMetaV4.t.sol"
    replace_once(
        path,
        """        child.verifyAndSettle(abi.encode(caseId));
        appealableVerifier.allocateVerifierReward(caseId);

        parentSolver.submitParent(parent, address(child));
""",
        """        child.verifyAndSettle(abi.encode(caseId));
        require(appealableVerifier.credits(primary) == 10_000, "atomic verifier reward missing");
        (bool duplicateAllocation,) =
            address(appealableVerifier).call(abi.encodeCall(appealableVerifier.allocateVerifierReward, (caseId)));
        require(!duplicateAllocation, "verifier reward allocated twice");

        parentSolver.submitParent(parent, address(child));
""",
    )
    replace_once(
        path,
        """        child.verifyAndSettle(abi.encode(caseId));
        appealableVerifier.allocateVerifierReward(caseId);

        require(child.bountyStatus() == StandingMetaChildV4.Status.Claimable, "rejected child did not reopen");
""",
        """        child.verifyAndSettle(abi.encode(caseId));
        require(appealableVerifier.credits(primary) == 10_000, "atomic rejection reward missing");

        require(child.bountyStatus() == StandingMetaChildV4.Status.Claimable, "rejected child did not reopen");
""",
    )
    replace_once(
        path,
        """    function _createParent() private returns (StandingMetaParentV4 parent) {
""",
        """    function testSelectionCommitmentIsDerivedAtInclusionTime() public {
        StandingMetaParentV4 parent = _createParent();
        StandingMetaParentFactoryV4.ClaimAndCreateChildRequest memory request = _claimRequest(parent);
        require(request.terms.selectionCommitment == bytes32(0), "caller supplied selection commitment");
        require(request.terms.selectionRequestedAt == 0, "caller supplied selection timestamp");
        vm.warp(block.timestamp + 5 minutes);
        address childAddress = parentSolver.claimAndCreate(parentFactory, address(parent), request);
        bytes32 termsHash = StandingMetaChildV4(childAddress).termsHash();
        (uint64 selectionRequestedAt,,,,,) = parentFactory.termsRegistry().economicsTiming(termsHash);
        (,,,,,, bytes32 commitment) = parentFactory.termsRegistry().bindings(termsHash);
        require(selectionRequestedAt == block.timestamp && commitment != bytes32(0), "selection was not block-derived");
    }

    function testUnsolicitedDustCannotBrickFundingOrParentClaim() public {
        StandingMetaParentV4 parent = _createParent();
        StandingMetaParentFactoryV4.ClaimAndCreateChildRequest memory request = _claimRequest(parent);
        token.mint(address(this), 2);
        token.transfer(address(parent), 1);
        token.transfer(request.terms.child, 1);
        address childAddress = parentSolver.claimAndCreate(parentFactory, address(parent), request);
        StandingMetaChildV4 child = StandingMetaChildV4(childAddress);
        require(child.fundedAmount() == 1_000_000, "child principal accounting drift");
        require(token.balanceOf(childAddress) == 1_000_001, "child dust was not tolerated");
        require(parent.bountyStatus() == StandingMetaParentV4.Status.Claimed, "parent claim was dust-bricked");

        StandingMetaParentV4 directParent = new StandingMetaParentV4(
            keccak256("direct-dust-parent"),
            address(this),
            address(this),
            address(token),
            address(parentFactory.verifierModule()),
            keccak256("direct-terms"),
            keccak256("direct-policy"),
            parentFactory.verifierModule().ACCEPTANCE_CRITERIA_HASH(),
            keccak256("direct-benchmark"),
            keccak256("direct-schema")
        );
        token.mint(address(this), 2_010_001);
        token.transfer(address(directParent), 2_010_001);
        directParent.recordInitialFunding();
        require(directParent.fundedAmount() == 2_010_000, "parent principal accounting drift");
    }

    function testParentBountyIdCannotBeReused() public {
        token.mint(address(this), 2_010_000);
        token.approve(address(parentFactory), type(uint256).max);
        StandingMetaParentFactoryV4.ParentConfig memory config = StandingMetaParentFactoryV4.ParentConfig({
            termsHash: keccak256("duplicate-parent-terms"),
            policyHash: keccak256("duplicate-parent-policy"),
            benchmarkHash: keccak256("duplicate-parent-benchmark"),
            evidenceSchemaHash: keccak256("duplicate-parent-schema"),
            creationNonce: keccak256("duplicate-parent-nonce")
        });
        (address firstParent, bytes32 bountyId) = parentFactory.createParent(config);
        (bool duplicate,) = address(parentFactory).call(abi.encodeCall(parentFactory.createParent, (config)));
        require(!duplicate, "duplicate canonical parent bounty id accepted");
        require(parentFactory.parentByBountyId(bountyId) == firstParent, "canonical parent id mapping drift");
    }

    function testSelectedSolverMustRemainEligibleAtClaimTime() public {
        StandingMetaParentV4 parent = _createParent();
        StandingMetaChildV4 child = _prepareAtomicChildAndDraw(parent, 616);
        child;
        address selected = _selectedChildSolver(parent);
        StandingMetaV4Actor(selected).setAvailable(pool, AnonymousStakePoolV1.Role.Solver, false);
        StandingMetaParentFactoryV4.BondAuthorization memory bond =
            _bondAuthorization(keccak256("ineligible-selected-bond"));
        (bool claimed,) = address(selected)
            .call(abi.encodeCall(StandingMetaV4Actor.claimChild, (parentFactory, address(parent), bond)));
        require(!claimed, "unavailable selected solver claimed child");
        vm.warp(block.timestamp + 10 minutes + 1);
        parentFactory.promoteNonresponsiveChildSolver(address(parent));
        (,, uint8 rank,) = parentFactory.roundTiming(address(parent), parent.round());
        require(rank == 1, "ineligible selected solver was not promotable");
    }

    function _createParent() private returns (StandingMetaParentV4 parent) {
""",
    )
    replace_region(
        path,
        "        address[] memory candidates = new address[](childCandidates.length);",
        "        request.canonicalTerms = canonicalTerms;",
        "",
    )
    replace_once(
        path,
        """            selectionCommitment: selectionCommitment,
""",
        """            selectionCommitment: bytes32(0),
""",
    )
    replace_once(
        path,
        """            selectionRequestedAt: selectionRequestedAt,
""",
        """            selectionRequestedAt: 0,
""",
    )


def patch_docs() -> None:
    replace_once(
        "docs/standing-meta-v4-fair-earning.md",
        """6. posts the parent bond and activates the parent claim.

After VRF fulfillment,
""",
        """6. posts the parent bond and activates the parent claim.

The caller supplies zero for the typed selection timestamp and commitment. The parent factory derives both from the actual inclusion block and frozen candidate set, so a signed transaction never has to predict a miner-selected timestamp.

After VRF fulfillment,
""",
    )
    replace_once(
        "docs/standing-meta-v4-fair-earning.md",
        """This is not guaranteed net profit. It excludes failure risk, labor, compute, taxes, gas outside platform sponsorship, and opportunity cost. A V4 opportunity is not ready to earn if gas sponsorship or the funded and authorized VRF subscription is unavailable.
""",
        """This is not guaranteed net profit. It excludes failure risk, labor, compute, taxes, gas outside platform sponsorship, and opportunity cost. A V4 opportunity is not ready to earn if gas sponsorship or the funded and authorized VRF subscription is unavailable. Unsolicited token transfers are not counted as funding, bonds, rewards, or margin and cannot block initialization; any surplus remains outside the immutable accounting.
""",
    )
    path = "docs/security/standing-meta-v4-threat-model.md"
    replace_once(
        path,
        """| Candidate joins after seeing a target | Child solver candidates are the already-active, available pool snapshotted inside `claimAndCreateChild` | Availability may change after the snapshot; ranking activation and claims still fail closed |
""",
        """| Candidate joins after seeing a target | Child solver candidates are the already-active, available pool snapshotted inside `claimAndCreateChild`; the selected ticket is rechecked immediately before claim | Availability may change after the snapshot; an ineligible selection waits for bounded promotion and never receives claim authority |
""",
    )
    replace_once(
        path,
        """| Jury result is already decisive | Three matching votes can be finalized immediately | A split or missing quorum waits until timeout and then fails closed |
""",
        """| Jury result is already decisive | Three matching votes can be finalized immediately; nonvoters are released while their voting window is still open and are slashed only after the deadline | A split or missing quorum waits until timeout and then fails closed |
""",
    )
    replace_once(
        path,
        """| Atomic preparation race | Terms publication, child creation/funding, active-pool snapshot, VRF request, round binding, and parent claim occur in one transaction | The transaction can revert for gas, authorization, pool-size, or subscription failures |
""",
        """| Atomic preparation race | Terms publication, child creation/funding, active-pool snapshot, VRF request, round binding, and parent claim occur in one transaction; selection time and commitment are derived onchain from the inclusion block | The transaction can revert for gas, authorization, pool-size, or subscription failures |
| Interface-shaped fake bounty consumes VRF or honest stake | `openCase` requires the immutable parent-factory registry to identify the exact canonical V4 child before any candidate query or VRF request | A wrong one-time controller/factory configuration is permanent and remains an R4 deployment-review gate |
| Delayed stage activation outlives the bounty | The case stores the bounty verification deadline; primary or jury assignment occurs only when the full remaining response/appeal/voting buffer still fits | A caller must invoke the permissionless fail-closed transition; no verifier is locked when the stage is already too late |
| Pre-sent token dust blocks exact accounting | Initial funding and bond checks require at least the exact accounted amount; unsolicited surplus is never credited as principal, bond, reward, or margin | Surplus cannot be recovered by the immutable protocol and should be treated as an unsolicited transfer |
| Child state changes before verifier reward attribution | The canonical child transfers and allocates the exact verifier reward in the same transaction; any allocation failure reverts the child transition | Generic non-V4 bounty implementations are outside this canonical V4 guarantee |
""",
    )
    replace_once(
        "docs/security/standing-meta-v4-slither-triage.md",
        """| `incorrect-equality` | 6 | Strict equality is intentional for exact USDC funding, exact commitment binding, unique request initialization, and fixed V4 economics. Accepting surplus funding would break conservation/accounting assumptions. |
""",
        """| `incorrect-equality` | 6 | Equality remains intentional for commitments, request initialization, and fixed V4 economics. Token-balance initialization and bond checks use minimum-balance assertions so an unsolicited transfer cannot denial-of-service a predicted contract address; surplus is never credited into protocol accounting. |
""",
    )


def main() -> None:
    patch_controller()
    patch_appealable_verifier()
    patch_parent_factory()
    patch_bounty_accounting_and_reward()
    patch_subscription_funding()
    patch_release_tool_redaction()
    patch_appeal_tests()
    patch_v4_tests()
    patch_docs()


if __name__ == "__main__":
    main()
