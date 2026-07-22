// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/StandingMetaV4Bundle.sol";

interface StandingMetaV4Vm {
    function warp(uint256 timestamp) external;
}

contract StandingMetaV4Token {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    mapping(bytes32 => bool) public authorizationUsed;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        _move(msg.sender, to, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(allowance[from][msg.sender] >= amount, "allowance");
        allowance[from][msg.sender] -= amount;
        _move(from, to, amount);
        return true;
    }

    function transferWithAuthorization(
        address from,
        address to,
        uint256 value,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8,
        bytes32,
        bytes32
    ) external {
        require(block.timestamp > validAfter && block.timestamp < validBefore, "authorization timing");
        require(!authorizationUsed[nonce], "authorization replay");
        authorizationUsed[nonce] = true;
        _move(from, to, value);
    }

    function _move(address from, address to, uint256 amount) private {
        require(balanceOf[from] >= amount, "balance");
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
    }
}

contract StandingMetaV4VrfMock {
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

contract StandingMetaV4Actor {
    function approve(StandingMetaV4Token token, address spender, uint256 amount) external {
        token.approve(spender, amount);
    }

    function register(AnonymousStakePoolV1 pool, AnonymousStakePoolV1.Role role) external {
        pool.register(role);
    }

    function activate(AnonymousStakePoolV1 pool, AnonymousStakePoolV1.Role role) external {
        pool.activate(role);
    }

    function claimAndCreate(
        StandingMetaParentFactoryV4 parentFactory,
        address parent,
        StandingMetaParentFactoryV4.ClaimAndCreateChildRequest calldata request
    ) external returns (address child) {
        (child,) = parentFactory.claimAndCreateChild(parent, request);
    }

    function setAvailable(AnonymousStakePoolV1 pool, AnonymousStakePoolV1.Role role, bool available) external {
        pool.setAvailability(role, available);
    }

    function claimChild(
        StandingMetaParentFactoryV4 parentFactory,
        address parent,
        StandingMetaParentFactoryV4.BondAuthorization calldata bond
    ) external {
        parentFactory.claimChildAssignment(parent, bond);
    }

    function submitChild(AgentBounty child, bytes32 submissionHash, bytes32 evidenceHash) external {
        child.submit(submissionHash, evidenceHash);
    }

    function primaryVerdict(AppealableVerifierV1 verifier, bytes32 caseId, bool passed) external {
        verifier.submitPrimaryVerdict(caseId, passed, keccak256(abi.encode(caseId, passed, address(this))));
    }

    function waiveAppeal(AppealableVerifierV1 verifier, bytes32 caseId) external {
        verifier.waiveAppeal(caseId);
    }

    function submitParent(StandingMetaParentV4 parent, address child) external {
        parent.submitChild(child);
    }
}

contract StandingMetaV4Test {
    StandingMetaV4Vm private constant vm = StandingMetaV4Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    bytes private constant CHILD_TERMS = '{"schema":"agent-bounties/standing-meta-v4-test"}';
    bytes32 private constant CHILD_POLICY_HASH = keccak256("child-policy");
    bytes32 private constant CHILD_CRITERIA_HASH = keccak256("child-criteria");
    bytes32 private constant CHILD_BENCHMARK_HASH = keccak256("child-benchmark");
    bytes32 private constant CHILD_EVIDENCE_SCHEMA_HASH = keccak256("child-evidence-schema");

    StandingMetaV4Token private token;
    AgentBountyFactory private childFactory;
    AnonymousProtocolControllerV1 private controller;
    AnonymousStakePoolV1 private pool;
    StandingMetaV4VrfMock private vrf;
    VrfSortitionCoordinatorV1 private verifierSortition;
    VrfSortitionCoordinatorV1 private solverSortition;
    AppealableVerifierV1 private appealableVerifier;
    StandingMetaParentFactoryV4 private parentFactory;
    StandingMetaV4Actor private parentSolver;
    StandingMetaV4Actor[] private childCandidates;
    StandingMetaV4Actor[] private verifierActors;
    uint256 private nonce;

    function setUp() public {
        token = new StandingMetaV4Token();
        childFactory = new AgentBountyFactory(address(token));
        controller = new AnonymousProtocolControllerV1(address(this));
        pool = new AnonymousStakePoolV1(address(token), address(controller));
        vrf = new StandingMetaV4VrfMock();
        verifierSortition =
            new VrfSortitionCoordinatorV1(address(vrf), address(controller), 77, keccak256("verifier-sortition"));
        solverSortition =
            new VrfSortitionCoordinatorV1(address(vrf), address(controller), 77, keccak256("solver-sortition"));
        appealableVerifier = new AppealableVerifierV1(address(token), address(controller), address(verifierSortition));
        parentFactory =
            new StandingMetaParentFactoryV4(address(childFactory), address(controller), address(appealableVerifier));
        controller.configure(
            address(pool),
            address(verifierSortition),
            address(solverSortition),
            address(appealableVerifier),
            address(parentFactory)
        );
        parentSolver = new StandingMetaV4Actor();
        token.mint(address(parentSolver), 1_010_000);

        for (uint256 i = 0; i < 3; i++) {
            StandingMetaV4Actor actor = new StandingMetaV4Actor();
            childCandidates.push(actor);
            token.mint(address(actor), 5_010_000);
            actor.approve(token, address(pool), type(uint256).max);
            actor.register(pool, AnonymousStakePoolV1.Role.Solver);
        }
        for (uint256 i = 0; i < 8; i++) {
            StandingMetaV4Actor actor = new StandingMetaV4Actor();
            verifierActors.push(actor);
            token.mint(address(actor), 5_000_000);
            actor.approve(token, address(pool), type(uint256).max);
            actor.register(pool, AnonymousStakePoolV1.Role.Verifier);
        }
        vm.warp(block.timestamp + 7 days);
        for (uint256 i = 0; i < childCandidates.length; i++) {
            childCandidates[i].activate(pool, AnonymousStakePoolV1.Role.Solver);
        }
        for (uint256 i = 0; i < verifierActors.length; i++) {
            verifierActors[i].activate(pool, AnonymousStakePoolV1.Role.Verifier);
        }
    }

    function testAtomicProfitableChildLoopSettlesBothBountiesWithOneUsdcMargin() public {
        StandingMetaParentV4 parent = _createParent();
        AgentBounty child = _prepareAtomicChildAndDraw(parent, 111);
        StandingMetaV4Actor childSolver = StandingMetaV4Actor(_selectedChildSolver(parent));
        childSolver.claimChild(parentFactory, address(parent), _bondAuthorization(keccak256("child-bond")));
        childSolver.submitChild(child, keccak256("child-submission"), keccak256("child-evidence"));

        (bytes32 caseId,) = appealableVerifier.openCase(address(child));
        uint256 verifierRequestId = vrf.nextRequestId() - 1;
        vrf.fulfill(verifierRequestId, 222);
        verifierSortition.deriveRanking(verifierRequestId);
        appealableVerifier.activatePrimary(caseId);
        (,,, address primary,) = appealableVerifier.caseParties(caseId);
        StandingMetaV4Actor(primary).primaryVerdict(appealableVerifier, caseId, true);
        parentSolver.waiveAppeal(appealableVerifier, caseId);
        child.verifyAndSettle(abi.encode(caseId));
        appealableVerifier.allocateVerifierReward(caseId);

        parentSolver.submitParent(parent, address(child));
        uint256 verifierBalanceBefore = token.balanceOf(address(this));
        parent.verifyAndSettle();

        require(child.bountyStatus() == AgentBounty.BountyStatus.Settled, "child not settled");
        require(parent.bountyStatus() == StandingMetaParentV4.Status.Settled, "parent not settled");
        require(token.balanceOf(address(childSolver)) == 1_000_000, "child solver payout mismatch");
        require(token.balanceOf(address(parentSolver)) == 2_010_000, "parent payout mismatch");
        require(
            token.balanceOf(address(parentSolver)) - 1_010_000 == 1_000_000, "successful-settlement margin mismatch"
        );
        require(token.balanceOf(address(this)) == verifierBalanceBefore + 10_000, "parent verifier reward missing");
    }

    function testBareParentClaimIsUnavailableAndAtomicPreparationCannotUseBadEconomics() public {
        StandingMetaParentV4 parent = _createParent();
        (bool bareClaim,) = address(parent).call(abi.encodeWithSignature("claim()"));
        require(!bareClaim, "bare parent claim enabled");

        StandingMetaParentFactoryV4.ClaimAndCreateChildRequest memory request = _claimRequest(parent);
        request.childParams.solverReward += 1;
        (bool badEconomics,) = address(parentSolver)
            .call(abi.encodeCall(StandingMetaV4Actor.claimAndCreate, (parentFactory, address(parent), request)));
        require(!badEconomics, "bad child economics accepted");
        require(parent.bountyStatus() == StandingMetaParentV4.Status.Claimable, "failed atomic call changed parent");
    }

    function testDrawRequiresThreeEligibleWalletsAndPromotesWithoutReroll() public {
        StandingMetaParentV4 parent = _createParent();
        StandingMetaParentFactoryV4.ClaimAndCreateChildRequest memory request = _claimRequest(parent);
        childCandidates[1].setAvailable(pool, AnonymousStakePoolV1.Role.Solver, false);
        childCandidates[2].setAvailable(pool, AnonymousStakePoolV1.Role.Solver, false);
        (bool tooSmall,) = address(parentSolver)
            .call(abi.encodeCall(StandingMetaV4Actor.claimAndCreate, (parentFactory, address(parent), request)));
        require(!tooSmall, "undersized active-pool draw accepted");

        childCandidates[1].setAvailable(pool, AnonymousStakePoolV1.Role.Solver, true);
        childCandidates[2].setAvailable(pool, AnonymousStakePoolV1.Role.Solver, true);
        AgentBounty child = _prepareAtomicChildAndDraw(parent, 333);
        child;
        address[] memory ranking = parentFactory.solverRanking(address(parent), parent.round());
        uint256 requestId = vrf.nextRequestId() - 1;
        vm.warp(block.timestamp + 10 minutes + 1);
        parentFactory.promoteNonresponsiveChildSolver(address(parent));
        address[] memory promotedRanking = parentFactory.solverRanking(address(parent), parent.round());
        (,, uint8 rank,) = parentFactory.roundTiming(address(parent), parent.round());
        require(rank == 1 && promotedRanking[1] != ranking[0], "backup not promoted");
        require(vrf.nextRequestId() - 1 == requestId, "promotion requested new randomness");
    }

    function testBundleWiresImmutableModulesAndNoParticipantRegistry() public {
        StandingMetaV4Bundle bundle = new StandingMetaV4Bundle(
            address(childFactory),
            address(controller),
            address(pool),
            address(verifierSortition),
            address(solverSortition),
            address(appealableVerifier),
            address(parentFactory)
        );
        require(bundle.controller().configured(), "bundle controller not configured");
        require(
            address(bundle.parentFactory().appealableVerifier()) == address(bundle.appealableVerifier()),
            "appeal module drift"
        );
        require(address(bundle.parentFactory().termsRegistry()) != address(0), "terms registry missing");
        require(bundle.stakePool().MINIMUM_VERIFIER_TICKETS() == 8, "pool minimum drift");
    }

    function _createParent() private returns (StandingMetaParentV4 parent) {
        token.mint(address(this), 2_010_000);
        token.approve(address(parentFactory), type(uint256).max);
        nonce += 1;
        StandingMetaParentFactoryV4.ParentConfig memory config = StandingMetaParentFactoryV4.ParentConfig({
            termsHash: keccak256(abi.encode("parent-terms", nonce)),
            policyHash: keccak256("parent-policy"),
            benchmarkHash: keccak256("parent-benchmark"),
            evidenceSchemaHash: keccak256("parent-evidence-schema"),
            creationNonce: bytes32(nonce)
        });
        (address parentAddress,) = parentFactory.createParent(config);
        parent = StandingMetaParentV4(parentAddress);
    }

    function _prepareAtomicChildAndDraw(StandingMetaParentV4 parent, uint256 randomWord)
        private
        returns (AgentBounty child)
    {
        StandingMetaParentFactoryV4.ClaimAndCreateChildRequest memory request = _claimRequest(parent);
        address childAddress = parentSolver.claimAndCreate(parentFactory, address(parent), request);
        child = AgentBounty(childAddress);
        uint256 requestId = vrf.nextRequestId() - 1;
        vrf.fulfill(requestId, randomWord);
        solverSortition.deriveRanking(requestId);
        parentFactory.activateChildDraw(address(parent));
    }

    function _claimRequest(StandingMetaParentV4 parent)
        private
        view
        returns (StandingMetaParentFactoryV4.ClaimAndCreateChildRequest memory request)
    {
        bytes memory canonicalTerms = abi.encode(CHILD_TERMS, address(parent));
        bytes32 childTermsHash = keccak256(canonicalTerms);
        bytes32 childCreationNonce = keccak256(abi.encode("child", address(parent)));
        request.childParams = AgentBountyFactory.CreateBountyParams({
            solverReward: 990_000,
            verifierReward: 10_000,
            termsHash: childTermsHash,
            policyHash: CHILD_POLICY_HASH,
            acceptanceCriteriaHash: CHILD_CRITERIA_HASH,
            benchmarkHash: CHILD_BENCHMARK_HASH,
            evidenceSchemaHash: CHILD_EVIDENCE_SCHEMA_HASH,
            fundingDeadline: uint64(block.timestamp + 7 days),
            claimWindowSeconds: 7 days,
            verificationWindowSeconds: 96 hours,
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: address(appealableVerifier),
            verifierRewardRecipient: address(appealableVerifier),
            threshold: 1
        });
        address[] memory noVerifiers = new address[](0);
        address predictedChild = childFactory.predictBountyAddress(
            address(parentSolver), request.childParams, noVerifiers, childCreationNonce
        );
        uint64 parentRound = parent.round() + 1;
        address[] memory candidates = new address[](childCandidates.length);
        for (uint256 i = 0; i < childCandidates.length; i++) {
            candidates[i] = address(childCandidates[i]);
        }
        bytes32 candidateHash = keccak256(abi.encode(candidates));
        uint64 selectionRequestedAt = uint64(block.timestamp);
        bytes32 selectionCommitment = keccak256(
            abi.encode(
                keccak256("agent-bounties/standing-meta-v4-child-solver-draw"),
                address(parent),
                parent.bountyId(),
                parentRound,
                predictedChild,
                childTermsHash,
                selectionRequestedAt,
                candidateHash
            )
        );
        request.canonicalTerms = canonicalTerms;
        request.terms = OnchainTermsRegistryV4.TermsInput({
            parent: address(parent),
            child: predictedChild,
            parentBountyId: parent.bountyId(),
            parentRound: parentRound,
            selectionCommitment: selectionCommitment,
            verifierModule: address(appealableVerifier),
            policyHash: CHILD_POLICY_HASH,
            acceptanceCriteriaHash: CHILD_CRITERIA_HASH,
            benchmarkHash: CHILD_BENCHMARK_HASH,
            evidenceSchemaHash: CHILD_EVIDENCE_SCHEMA_HASH,
            appealPolicyHash: appealableVerifier.appealPolicyHash(),
            selectionRequestedAt: selectionRequestedAt,
            childClaimWindowSeconds: 7 days,
            childVerificationWindowSeconds: 96 hours,
            childFundingTarget: 1_000_000,
            childSolverReward: 990_000,
            childVerifierReward: 10_000
        });
        request.childCreationNonce = childCreationNonce;
        request.childFundingAuthorization = AgentBountyFactory.FundingAuthorization({
            validAfter: block.timestamp - 1,
            validBefore: block.timestamp + 1 days,
            nonce: keccak256(abi.encode("child-funding", address(parent))),
            v: 0,
            r: bytes32(0),
            s: bytes32(0)
        });
        request.parentBondAuthorization = _bondAuthorization(keccak256(abi.encode("parent-bond", address(parent))));
    }

    function _bondAuthorization(bytes32 authorizationNonce)
        private
        view
        returns (StandingMetaParentFactoryV4.BondAuthorization memory)
    {
        return StandingMetaParentFactoryV4.BondAuthorization({
            validAfter: block.timestamp - 1,
            validBefore: block.timestamp + 1 days,
            nonce: authorizationNonce,
            v: 0,
            r: bytes32(0),
            s: bytes32(0)
        });
    }

    function _selectedChildSolver(StandingMetaParentV4 parent) private view returns (address) {
        address[] memory ranking = parentFactory.solverRanking(address(parent), parent.round());
        return ranking[0];
    }
}
