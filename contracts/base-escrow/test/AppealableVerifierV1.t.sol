// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactory.sol";
import "../src/AppealableVerifierV1.sol";

interface AppealVerifierVm {
    function warp(uint256 timestamp) external;
}

contract AppealVerifierToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        require(balanceOf[msg.sender] >= amount, "balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(balanceOf[from] >= amount, "balance");
        require(allowance[from][msg.sender] >= amount, "allowance");
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }
}

contract AppealVerifierVrfMock {
    uint256 public nextRequestId = 1;
    mapping(uint256 => address) public consumer;

    function requestRandomWords(VrfV2PlusClientV1.RandomWordsRequest calldata) external returns (uint256 requestId) {
        requestId = nextRequestId;
        nextRequestId += 1;
        consumer[requestId] = msg.sender;
    }

    function fulfill(uint256 requestId, uint256 randomWord) external {
        uint256[] memory words = new uint256[](1);
        words[0] = randomWord;
        VrfSortitionCoordinatorV1(consumer[requestId]).rawFulfillRandomWords(requestId, words);
    }
}

contract AppealVerifierActor {
    function approve(AppealVerifierToken token, address spender, uint256 amount) external {
        token.approve(spender, amount);
    }

    function register(AnonymousStakePoolV1 pool, AnonymousStakePoolV1.Role role) external {
        pool.register(role);
    }

    function activate(AnonymousStakePoolV1 pool, AnonymousStakePoolV1.Role role) external {
        pool.activate(role);
    }

    function claim(AgentBounty bounty) external {
        bounty.claim();
    }

    function submit(AgentBounty bounty, bytes32 submissionHash, bytes32 evidenceHash) external {
        bounty.submit(submissionHash, evidenceHash);
    }

    function verdict(AppealableVerifierV1 verifier, bytes32 caseId, bool passed, bytes32 responseHash) external {
        verifier.submitPrimaryVerdict(caseId, passed, responseHash);
    }

    function appeal(AppealableVerifierV1 verifier, bytes32 caseId) external {
        verifier.openAppeal(caseId);
    }

    function waive(AppealableVerifierV1 verifier, bytes32 caseId) external {
        verifier.waiveAppeal(caseId);
    }

    function vote(AppealableVerifierV1 verifier, bytes32 caseId, bool passed) external {
        verifier.submitAppealVote(caseId, passed);
    }

    function withdraw(AppealableVerifierV1 verifier) external {
        verifier.withdrawCredit();
    }
}

contract AppealVerifierControllerDummy {}

contract AppealableVerifierV1Test {
    AppealVerifierVm private constant vm = AppealVerifierVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint256 private constant SOLVER_REWARD = 990_000;
    uint256 private constant VERIFIER_REWARD = 10_000;
    bytes32 private constant SUBMISSION_HASH = keccak256("appealable-submission");
    bytes32 private constant EVIDENCE_HASH = keccak256("appealable-evidence");
    bytes32 private constant POLICY_HASH = keccak256("appealable-policy-v1");

    AppealVerifierToken private token;
    AgentBountyFactory private factory;
    AnonymousProtocolControllerV1 private controller;
    AnonymousStakePoolV1 private pool;
    AppealVerifierVrfMock private vrf;
    VrfSortitionCoordinatorV1 private verifierSortition;
    VrfSortitionCoordinatorV1 private solverSortition;
    AppealableVerifierV1 private verifier;
    AppealVerifierActor private solver;
    AppealVerifierActor[] private verifierActors;
    uint256 private nonce;

    function setUp() public {
        token = new AppealVerifierToken();
        factory = new AgentBountyFactory(address(token));
        controller = new AnonymousProtocolControllerV1(address(this));
        pool = new AnonymousStakePoolV1(address(token), address(controller));
        vrf = new AppealVerifierVrfMock();
        verifierSortition =
            new VrfSortitionCoordinatorV1(address(vrf), address(controller), 111, keccak256("verifier-key-hash"));
        solverSortition =
            new VrfSortitionCoordinatorV1(address(vrf), address(controller), 111, keccak256("solver-key-hash"));
        verifier = new AppealableVerifierV1(address(token), address(controller), address(verifierSortition));
        AppealVerifierControllerDummy parentFactory = new AppealVerifierControllerDummy();
        controller.configure(
            address(pool),
            address(verifierSortition),
            address(solverSortition),
            address(verifier),
            address(parentFactory)
        );

        solver = new AppealVerifierActor();
        token.mint(address(solver), 1_000_000);
        for (uint256 i = 0; i < 8; i++) {
            AppealVerifierActor actor = new AppealVerifierActor();
            verifierActors.push(actor);
            token.mint(address(actor), 5_000_000);
            actor.approve(token, address(pool), type(uint256).max);
            actor.register(pool, AnonymousStakePoolV1.Role.Verifier);
        }
        vm.warp(block.timestamp + 7 days);
        for (uint256 i = 0; i < verifierActors.length; i++) {
            verifierActors[i].activate(pool, AnonymousStakePoolV1.Role.Verifier);
        }
    }

    function testVerifyRevertsBeforeFinalizationAndUnappealedPrimaryIsPaid() public {
        (AgentBounty bounty, bytes32 caseId) = _prepareCase(101);
        AppealVerifierActor primary = AppealVerifierActor(_primary(caseId));
        primary.verdict(verifier, caseId, true, keccak256("primary-pass"));

        (bool early,) = address(bounty).call(abi.encodeCall(bounty.verifyAndSettle, (abi.encode(caseId))));
        require(!early, "bounty settled before appeal window");

        vm.warp(block.timestamp + verifier.APPEAL_WINDOW() + 1);
        verifier.finalizeUnappealed(caseId);
        bounty.verifyAndSettle(abi.encode(caseId));
        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Settled, "bounty not settled");
        verifier.allocateVerifierReward(caseId);
        require(verifier.credits(address(primary)) == VERIFIER_REWARD, "primary reward missing");
        primary.withdraw(verifier);
        require(token.balanceOf(address(primary)) == VERIFIER_REWARD, "primary withdrawal mismatch");
    }

    function testSolverCanOverturnRejectionWithFiveWalletJury() public {
        (AgentBounty bounty, bytes32 caseId) = _prepareCase(202);
        address primary = _primary(caseId);
        AppealVerifierActor(primary).verdict(verifier, caseId, false, keccak256("primary-reject"));
        _openSolverAppeal(caseId, 303);
        address[] memory jury = verifier.appellateWallets(caseId);
        for (uint256 i = 0; i < 3; i++) {
            AppealVerifierActor(jury[i]).vote(verifier, caseId, true);
        }

        verifier.finalizeAppeal(caseId);
        bounty.verifyAndSettle(abi.encode(caseId));
        verifier.allocateVerifierReward(caseId);

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Settled, "overturned rejection did not settle");
        require(verifier.credits(address(solver)) == 100_000, "appeal bond not refunded");
        (uint128 primaryStake,,,, bool primaryActive,) = pool.tickets(primary, AnonymousStakePoolV1.Role.Verifier);
        require(primaryStake == 4_900_000 && !primaryActive, "overturned primary not slashed");
        uint256 juryCredits;
        for (uint256 i = 0; i < 3; i++) {
            juryCredits += verifier.credits(jury[i]);
        }
        require(juryCredits == 110_000, "slash and verifier reward not shared");
    }

    function testEitherEligiblePartyCanWaiveAndFinalizeWithoutWaiting() public {
        (AgentBounty acceptedBounty, bytes32 acceptedCaseId) = _prepareCase(151);
        AppealVerifierActor(_primary(acceptedCaseId))
            .verdict(verifier, acceptedCaseId, true, keccak256("accepted-fast-path"));
        verifier.waiveAppeal(acceptedCaseId);
        acceptedBounty.verifyAndSettle(abi.encode(acceptedCaseId));
        require(
            acceptedBounty.bountyStatus() == AgentBounty.BountyStatus.Settled,
            "creator waiver did not finalize acceptance"
        );

        (AgentBounty rejectedBounty, bytes32 rejectedCaseId) = _prepareCase(152);
        AppealVerifierActor(_primary(rejectedCaseId))
            .verdict(verifier, rejectedCaseId, false, keccak256("rejected-fast-path"));
        solver.waive(verifier, rejectedCaseId);
        rejectedBounty.verifyAndSettle(abi.encode(rejectedCaseId));
        require(
            rejectedBounty.bountyStatus() == AgentBounty.BountyStatus.Claimable,
            "solver waiver did not finalize rejection"
        );
    }

    function testCreatorCanOverturnAcceptance() public {
        (AgentBounty bounty, bytes32 caseId) = _prepareCase(404);
        AppealVerifierActor(_primary(caseId)).verdict(verifier, caseId, true, keccak256("primary-accept"));
        token.mint(address(this), 100_000);
        token.approve(address(verifier), 100_000);
        verifier.openAppeal(caseId);
        _activateAppeal(caseId, 505);
        address[] memory jury = verifier.appellateWallets(caseId);
        for (uint256 i = 0; i < 3; i++) {
            AppealVerifierActor(jury[i]).vote(verifier, caseId, false);
        }

        verifier.finalizeAppeal(caseId);
        bounty.verifyAndSettle(abi.encode(caseId));
        verifier.allocateVerifierReward(caseId);

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Claimable, "overturned acceptance did not reject");
        require(verifier.credits(address(this)) == 100_000, "creator bond not refunded");
    }

    function testUpheldAppealPaysPrimaryAndSharesBond() public {
        (AgentBounty bounty, bytes32 caseId) = _prepareCase(606);
        address primary = _primary(caseId);
        AppealVerifierActor(primary).verdict(verifier, caseId, false, keccak256("primary-reject"));
        _openSolverAppeal(caseId, 707);
        address[] memory jury = verifier.appellateWallets(caseId);
        for (uint256 i = 0; i < 3; i++) {
            AppealVerifierActor(jury[i]).vote(verifier, caseId, false);
        }

        verifier.finalizeAppeal(caseId);
        bounty.verifyAndSettle(abi.encode(caseId));
        verifier.allocateVerifierReward(caseId);

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Claimable, "upheld rejection did not reject");
        require(verifier.credits(primary) == VERIFIER_REWARD, "upheld primary reward missing");
        uint256 juryCredits;
        for (uint256 i = 0; i < 3; i++) {
            juryCredits += verifier.credits(jury[i]);
        }
        require(juryCredits == 100_000, "appeal bond not shared");
    }

    function testPrimaryAndAppealTimeoutsFailClosedAndUnlockRemainingStake() public {
        (AgentBounty bounty, bytes32 caseId) = _prepareCase(808);
        address firstPrimary = _primary(caseId);
        vm.warp(block.timestamp + verifier.RESPONSE_WINDOW() + 1);
        verifier.promotePrimary(caseId);
        address backup = _primary(caseId);
        require(backup != firstPrimary, "backup not promoted");
        (uint128 firstStake,,,, bool firstActive,) = pool.tickets(firstPrimary, AnonymousStakePoolV1.Role.Verifier);
        require(firstStake == 4_990_000 && !firstActive, "availability failure not slashed");

        AppealVerifierActor(backup).verdict(verifier, caseId, false, keccak256("backup-reject"));
        _openSolverAppeal(caseId, 909);
        vm.warp(block.timestamp + verifier.VOTING_WINDOW() + 1);
        verifier.finalizeAppeal(caseId);
        require(verifier.caseState(caseId) == AppealableVerifierV1.CaseState.TimedOut, "case did not time out");
        require(verifier.credits(address(solver)) == 100_000, "timed-out appeal bond not refunded");
        (bool settled,) = address(bounty).call(abi.encodeCall(bounty.verifyAndSettle, (abi.encode(caseId))));
        require(!settled, "timed-out case executed verdict");
    }

    function testPrimaryRandomnessTimeoutAndLateCaseOpeningFailClosed() public {
        AgentBounty timedOutBounty = _createSubmittedBounty();
        (bytes32 timedOutCaseId,) = verifier.openCase(address(timedOutBounty));
        vm.warp(block.timestamp + 2 hours + 1);
        verifier.timeoutPrimaryRandomness(timedOutCaseId);
        require(
            verifier.caseState(timedOutCaseId) == AppealableVerifierV1.CaseState.TimedOut,
            "primary randomness did not time out"
        );

        AgentBounty lateBounty = _createSubmittedBounty();
        vm.warp(uint256(lateBounty.verificationExpiresAt()) - uint256(verifier.MINIMUM_CASE_REMAINING()) + 1);
        (bool opened,) = address(verifier).call(abi.encodeCall(verifier.openCase, (address(lateBounty))));
        require(!opened, "late case opening accepted");

        AgentBounty timelyBounty = _createSubmittedBounty();
        (bytes32 timelyCaseId,) = verifier.openCase(address(timelyBounty));
        _fulfillLatestAndDerive(1_010);
        vm.warp(block.timestamp + 2 hours + 1);
        (bool incorrectlyTimedOut,) =
            address(verifier).call(abi.encodeCall(verifier.timeoutPrimaryRandomness, (timelyCaseId)));
        require(!incorrectlyTimedOut, "timely fulfillment was timed out");
        verifier.activatePrimary(timelyCaseId);
    }

    function _prepareCase(uint256 randomWord) private returns (AgentBounty bounty, bytes32 caseId) {
        bounty = _createSubmittedBounty();
        (caseId,) = verifier.openCase(address(bounty));
        _fulfillLatestAndDerive(randomWord);
        verifier.activatePrimary(caseId);
    }

    function _createSubmittedBounty() private returns (AgentBounty bounty) {
        AgentBountyFactory.CreateBountyParams memory params = AgentBountyFactory.CreateBountyParams({
            solverReward: SOLVER_REWARD,
            verifierReward: VERIFIER_REWARD,
            termsHash: keccak256(abi.encode("appealable-terms", nonce)),
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: keccak256("appealable-criteria"),
            benchmarkHash: keccak256("appealable-benchmark"),
            evidenceSchemaHash: keccak256("appealable-evidence-schema"),
            fundingDeadline: uint64(block.timestamp + 7 days),
            claimWindowSeconds: 14 days,
            verificationWindowSeconds: verifier.REQUIRED_BOUNTY_VERIFICATION_WINDOW(),
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: address(verifier),
            verifierRewardRecipient: address(verifier),
            threshold: 1
        });
        address[] memory noVerifiers = new address[](0);
        token.mint(address(this), SOLVER_REWARD + VERIFIER_REWARD);
        token.approve(address(factory), type(uint256).max);
        nonce += 1;
        (address bountyAddress,) =
            factory.createBounty(params, noVerifiers, SOLVER_REWARD + VERIFIER_REWARD, bytes32(nonce));
        bounty = AgentBounty(bountyAddress);
        solver.approve(token, address(bounty), type(uint256).max);
        solver.claim(bounty);
        solver.submit(bounty, SUBMISSION_HASH, EVIDENCE_HASH);
    }

    function _openSolverAppeal(bytes32 caseId, uint256 randomWord) private {
        solver.approve(token, address(verifier), 100_000);
        solver.appeal(verifier, caseId);
        _activateAppeal(caseId, randomWord);
    }

    function _activateAppeal(bytes32 caseId, uint256 randomWord) private {
        _fulfillLatestAndDerive(randomWord);
        verifier.activateAppeal(caseId);
    }

    function _fulfillLatestAndDerive(uint256 randomWord) private {
        uint256 requestId = vrf.nextRequestId() - 1;
        vrf.fulfill(requestId, randomWord);
        verifierSortition.deriveRanking(requestId);
    }

    function _primary(bytes32 caseId) private view returns (address primary) {
        (,,, primary,) = verifier.caseParties(caseId);
    }
}
