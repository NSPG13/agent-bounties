// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactoryV2.sol";
import "../src/BoundedV2FundingWallet.sol";
import "../src/ParticipantEligibilityRegistry.sol";

interface FundingWalletVm {
    function warp(uint256) external;
    function prank(address sender) external;
    function addr(uint256 privateKey) external returns (address);
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
}

contract FundingWalletToken {
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

/// @dev Deterministic module whose verdict is the first proof byte: 0x01 passes, anything else fails.
contract FundingWalletVerdictModule is IAgentBountyVerifier {
    function verify(bytes32, uint64, address, bytes32, bytes32, bytes32, bytes calldata proof)
        external
        pure
        returns (bool passed, bytes32 responseHash)
    {
        passed = proof.length > 0 && proof[0] == 0x01;
        responseHash = keccak256(proof);
    }
}

contract FundingWalletSolver {
    function claimAndSubmit(AgentBountyV2 bounty, FundingWalletToken token) external {
        token.approve(address(bounty), bounty.verifierReward());
        bounty.claim();
        bounty.submit(keccak256("artifact"), keccak256("evidence"));
    }
}

contract BoundedV2FundingWalletTest {
    FundingWalletVm constant vm = FundingWalletVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    uint16 constant FEE_BPS = 750;
    address constant FEE_RECIPIENT = address(0xFEE);
    address constant OWNER = address(0x0A11CE);
    address constant DELEGATE = address(0xD0D0);
    address constant STRANGER = address(0xBAD);
    address constant VERIFIER_RECIPIENT = address(0xBEEF);
    uint256 constant VERIFIER_KEY_A = 0xA11CE;
    uint256 constant VERIFIER_KEY_B = 0xB0B5;
    uint256 constant WALLET_FLOAT = 20_000;
    uint256 constant SOLVER_REWARD = 1_000;
    uint256 constant VERIFIER_REWARD = 100;
    uint256 constant EXPECTED_TARGET = 1_175;

    FundingWalletToken token;
    AgentBountyFactoryV2 factory;
    FundingWalletVerdictModule module;
    BoundedV2FundingWallet wallet;
    address[] quorum;
    uint256 nonceCounter;

    function setUp() public {
        vm.warp(1_000_000);
        token = new FundingWalletToken();
        factory = new AgentBountyFactoryV2(address(token), FEE_BPS, FEE_RECIPIENT);
        module = new FundingWalletVerdictModule();
        quorum.push(vm.addr(VERIFIER_KEY_A));
        quorum.push(vm.addr(VERIFIER_KEY_B));
        wallet = new BoundedV2FundingWallet(address(factory), OWNER, _policy(), _configs());
        token.mint(address(wallet), WALLET_FLOAT);
    }

    // ---------------------------------------------------------------- creation

    function testDelegateCreatesClaimableBountyFundedWithExactTargetIncludingFee() public {
        (uint256 fee, uint256 quotedTarget) = factory.quote(SOLVER_REWARD, VERIFIER_REWARD);
        require(fee == 75 && quotedTarget == EXPECTED_TARGET, "quote mismatch");

        AgentBountyV2 bounty = _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));

        require(bounty.creator() == address(wallet), "wallet must be creator");
        require(bounty.targetAmount() == EXPECTED_TARGET, "target mismatch");
        require(bounty.platformFee() == 75, "fee mismatch");
        require(bounty.contributions(address(wallet)) == EXPECTED_TARGET, "contribution mismatch");
        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Claimable, "not claimable");
        require(wallet.isWalletBounty(address(bounty)), "bounty not recorded");
        require(token.balanceOf(address(wallet)) == WALLET_FLOAT - EXPECTED_TARGET, "wallet debit mismatch");
        require(token.allowance(address(wallet), address(factory)) == 0, "allowance must be reset");
        require(wallet.lifetimeSpent() == EXPECTED_TARGET && wallet.periodSpent() == EXPECTED_TARGET, "spend mismatch");
    }

    function testOnlyActiveDelegateCanCreate() public {
        bytes memory call = _createCall(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD), new address[](0));
        _reverts(OWNER, address(wallet), call, "not delegate");
        _reverts(STRANGER, address(wallet), call, "not delegate");

        vm.prank(OWNER);
        wallet.revokePolicy();
        _reverts(DELEGATE, address(wallet), call, "policy revoked");

        BoundedV2FundingWallet.Policy memory later = _policy();
        later.validAfter = uint64(block.timestamp + 1 hours);
        vm.prank(OWNER);
        wallet.configurePolicy(later, _configs());
        _reverts(DELEGATE, address(wallet), call, "policy not active");

        vm.warp(later.validUntil + 1);
        _reverts(
            DELEGATE,
            address(wallet),
            _createCall(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD), new address[](0)),
            "policy expired"
        );
    }

    function testPerBountyPeriodAndLifetimeCapsBoundEverySpend() public {
        // 1,900 + 100 + 143 fee = 2,143 > 2,000 per-bounty cap.
        _reverts(
            DELEGATE,
            address(wallet),
            _createCall(_moduleParams(1_900, 100), new address[](0)),
            "per-bounty cap exceeded"
        );

        for (uint256 i = 0; i < 4; i++) {
            _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));
        }
        require(wallet.periodSpent() == 4 * EXPECTED_TARGET, "period spend mismatch");
        _reverts(
            DELEGATE,
            address(wallet),
            _createCall(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD), new address[](0)),
            "period cap exceeded"
        );

        vm.warp(block.timestamp + 1 days);
        for (uint256 i = 0; i < 4; i++) {
            _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));
        }
        require(wallet.lifetimeSpent() == 8 * EXPECTED_TARGET, "lifetime spend mismatch");
        vm.warp(block.timestamp + 1 days);
        _reverts(
            DELEGATE,
            address(wallet),
            _createCall(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD), new address[](0)),
            "lifetime cap exceeded"
        );
    }

    function testRecoveredRefundsDoNotRestoreSpendingAuthority() public {
        AgentBountyV2 bounty = _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));
        vm.prank(DELEGATE);
        wallet.cancelBounty(address(bounty));
        require(token.balanceOf(address(wallet)) == WALLET_FLOAT, "refund not recovered");
        require(wallet.lifetimeSpent() == EXPECTED_TARGET, "recovery must not restore lifetime authority");
        require(wallet.periodSpent() == EXPECTED_TARGET, "recovery must not restore period authority");
    }

    function testVerificationModeModuleRecipientSetAndThresholdArePinned() public {
        AgentBountyFactoryV2.CreateBountyParams memory params = _moduleParams(SOLVER_REWARD, VERIFIER_REWARD);
        params.verifierModule = address(new FundingWalletVerdictModule());
        _reverts(DELEGATE, address(wallet), _createCall(params, new address[](0)), "verification config not allowed");

        params = _moduleParams(SOLVER_REWARD, VERIFIER_REWARD);
        params.verifierRewardRecipient = STRANGER;
        _reverts(DELEGATE, address(wallet), _createCall(params, new address[](0)), "verification config not allowed");

        params = _quorumParams(AgentBountyV2.VerificationMode.AiJudgeQuorum, 2);
        _reverts(DELEGATE, address(wallet), _createCall(params, quorum), "verification config not allowed");

        address[] memory otherSet = new address[](2);
        otherSet[0] = quorum[0];
        otherSet[1] = STRANGER;
        params = _quorumParams(AgentBountyV2.VerificationMode.SignedQuorum, 2);
        _reverts(DELEGATE, address(wallet), _createCall(params, otherSet), "verification config not allowed");

        params = _quorumParams(AgentBountyV2.VerificationMode.SignedQuorum, 1);
        _reverts(DELEGATE, address(wallet), _createCall(params, quorum), "verification config not allowed");

        params = _quorumParams(AgentBountyV2.VerificationMode.SignedQuorum, 2);
        AgentBountyV2 bounty = _createWith(params, quorum);
        require(bounty.verifierSetHash() == keccak256(abi.encode(quorum)), "quorum set mismatch");
        require(bounty.threshold() == 2, "threshold mismatch");
    }

    function testClaimEligibilityIsPinned() public {
        ParticipantEligibilityRegistry registry = new ParticipantEligibilityRegistry(OWNER);
        bytes32 source = keccak256("agent-bounties/invoice-contractor-v1");

        AgentBountyFactoryV2.CreateBountyParams memory gated = _moduleParams(SOLVER_REWARD, VERIFIER_REWARD);
        gated.claimEligibilityRegistry = address(registry);
        gated.claimEligibilitySource = source;
        _reverts(DELEGATE, address(wallet), _createCall(gated, new address[](0)), "claim eligibility not allowed");

        BoundedV2FundingWallet.Policy memory gatedPolicy = _policy();
        gatedPolicy.claimEligibilityRegistry = address(registry);
        gatedPolicy.claimEligibilitySource = source;
        vm.prank(OWNER);
        wallet.configurePolicy(gatedPolicy, _configs());

        _reverts(
            DELEGATE,
            address(wallet),
            _createCall(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD), new address[](0)),
            "claim eligibility not allowed"
        );
        AgentBountyV2 bounty = _createWith(gated, new address[](0));
        require(bounty.claimEligibilityRegistry() == address(registry), "registry mismatch");
    }

    function testFundingClaimAndVerificationWindowsAreBounded() public {
        AgentBountyFactoryV2.CreateBountyParams memory params = _moduleParams(SOLVER_REWARD, VERIFIER_REWARD);
        params.fundingDeadline = uint64(block.timestamp + 30 days + 1);
        _reverts(DELEGATE, address(wallet), _createCall(params, new address[](0)), "funding window too long");

        params = _moduleParams(SOLVER_REWARD, VERIFIER_REWARD);
        params.claimWindowSeconds = 7 days + 1;
        _reverts(DELEGATE, address(wallet), _createCall(params, new address[](0)), "claim window too long");

        params = _moduleParams(SOLVER_REWARD, VERIFIER_REWARD);
        params.verificationWindowSeconds = 7 days + 1;
        _reverts(DELEGATE, address(wallet), _createCall(params, new address[](0)), "verification window too long");
    }

    // ---------------------------------------------------------------- settlement and cancellation

    function testSettlementOfWalletBountyPaysSolverVerifierAndFee() public {
        AgentBountyV2 bounty = _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));
        FundingWalletSolver solver = _solve(bounty);

        bounty.verifyAndSettle(hex"01");

        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Settled, "not settled");
        require(token.balanceOf(address(solver)) == SOLVER_REWARD + VERIFIER_REWARD, "solver reward plus bond");
        require(token.balanceOf(VERIFIER_RECIPIENT) == VERIFIER_REWARD, "verifier reward");
        require(token.balanceOf(FEE_RECIPIENT) == 75, "platform fee");
        require(token.balanceOf(address(wallet)) == WALLET_FLOAT - EXPECTED_TARGET, "wallet must not be charged twice");
    }

    function testCancelIdleBountyReturnsTheFullTargetToTheWallet() public {
        AgentBountyV2 bounty = _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));
        vm.prank(DELEGATE);
        uint256 recovered = wallet.cancelBounty(address(bounty));

        require(recovered == EXPECTED_TARGET, "recovered amount mismatch");
        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Cancelled, "not cancelled");
        require(bounty.contributions(address(wallet)) == 0, "contribution not cleared");
        require(token.balanceOf(address(wallet)) == WALLET_FLOAT, "wallet not made whole");
        _reverts(
            DELEGATE,
            address(wallet),
            abi.encodeCall(wallet.cancelBounty, (address(bounty))),
            "refund already recovered"
        );
    }

    function testOwnerCancelsWhenTheDelegateMayNotOrIsRevoked() public {
        BoundedV2FundingWallet.Policy memory noCancel = _policy();
        noCancel.delegateMayCancel = false;
        vm.prank(OWNER);
        wallet.configurePolicy(noCancel, _configs());
        AgentBountyV2 bounty = _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));

        bytes memory call = abi.encodeCall(wallet.cancelBounty, (address(bounty)));
        _reverts(DELEGATE, address(wallet), call, "delegate may not cancel");
        _reverts(STRANGER, address(wallet), call, "not delegate");

        vm.prank(OWNER);
        wallet.revokePolicy();
        vm.prank(OWNER);
        require(wallet.cancelBounty(address(bounty)) == EXPECTED_TARGET, "owner recovery failed");
    }

    function testCancelDuringActiveRoundLetsAPassingSolverBePaid() public {
        AgentBountyV2 bounty = _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));
        FundingWalletSolver solver = _solve(bounty);

        vm.prank(DELEGATE);
        require(wallet.cancelBounty(address(bounty)) == 0, "active round must only request cancellation");
        require(bounty.cancelRequested(), "cancel not requested");
        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Submitted, "round must continue");

        bounty.verifyAndSettle(hex"01");
        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Settled, "passing solver must settle");
        require(token.balanceOf(address(solver)) == SOLVER_REWARD + VERIFIER_REWARD, "solver not paid");
    }

    function testCancelRequestedRoundRecoversAfterExpiry() public {
        AgentBountyV2 bounty = _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));
        FundingWalletSolver solver = _solve(bounty);
        vm.prank(DELEGATE);
        wallet.cancelBounty(address(bounty));

        vm.warp(block.timestamp + 1 days + 1);
        bounty.expireSubmission();
        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Cancelled, "request must cancel after expiry");
        require(token.balanceOf(address(solver)) == VERIFIER_REWARD, "expired bond must return");

        vm.prank(DELEGATE);
        require(wallet.cancelBounty(address(bounty)) == EXPECTED_TARGET, "wallet must recover the target");
        require(token.balanceOf(address(wallet)) == WALLET_FLOAT, "wallet not made whole");
    }

    function testThirdPartyRefundPushAlwaysPaysTheWallet() public {
        AgentBountyV2 bounty = _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));
        vm.warp(block.timestamp + 1 days + 1);
        vm.prank(STRANGER);
        bounty.cancel();
        vm.prank(STRANGER);
        bounty.withdrawRefundFor(address(wallet));
        require(token.balanceOf(address(wallet)) == WALLET_FLOAT, "refund must return to the wallet");
        require(token.balanceOf(STRANGER) == 0, "stranger must receive nothing");
    }

    function testCannotCancelABountyTheWalletDidNotCreate() public {
        token.mint(address(this), EXPECTED_TARGET);
        token.approve(address(factory), EXPECTED_TARGET);
        (address foreign,) = factory.createBounty(
            _moduleParams(SOLVER_REWARD, VERIFIER_REWARD), new address[](0), EXPECTED_TARGET, bytes32(uint256(999))
        );
        _reverts(DELEGATE, address(wallet), abi.encodeCall(wallet.cancelBounty, (foreign)), "not wallet bounty");
        _reverts(OWNER, address(wallet), abi.encodeCall(wallet.cancelBounty, (foreign)), "not wallet bounty");
    }

    // ---------------------------------------------------------------- authority

    function testStolenDelegateCannotWithdrawRedirectOrReconfigure() public {
        _reverts(
            DELEGATE, address(wallet), abi.encodeCall(wallet.withdrawToken, (address(token), DELEGATE, 1)), "not owner"
        );
        _reverts(
            DELEGATE, address(wallet), abi.encodeCall(wallet.configurePolicy, (_policy(), _configs())), "not owner"
        );
        _reverts(DELEGATE, address(wallet), abi.encodeCall(wallet.transferOwnership, (DELEGATE)), "not owner");
        _reverts(DELEGATE, address(wallet), abi.encodeCall(wallet.revokePolicy, ()), "not owner");
        _reverts(
            DELEGATE, address(wallet), abi.encodeCall(wallet.setCaps, (1e30, 1e30, type(uint256).max)), "not owner"
        );
        require(token.balanceOf(address(wallet)) == WALLET_FLOAT, "delegate moved funds");
    }

    function testOwnerWithdrawsAndTransfersOwnershipInTwoSteps() public {
        vm.prank(OWNER);
        wallet.withdrawToken(address(token), OWNER, 500);
        require(token.balanceOf(OWNER) == 500, "withdrawal failed");

        vm.prank(OWNER);
        wallet.transferOwnership(STRANGER);
        require(wallet.owner() == OWNER, "ownership moved before acceptance");
        _reverts(DELEGATE, address(wallet), abi.encodeCall(wallet.acceptOwnership, ()), "not pending owner");
        vm.prank(STRANGER);
        wallet.acceptOwnership();
        require(wallet.owner() == STRANGER && wallet.pendingOwner() == address(0), "ownership not transferred");
    }

    function testPolicyReplacementBumpsVersionAndHashCommitsEveryField() public {
        bytes32 before = wallet.policyHash();
        require(
            before == keccak256(abi.encode(wallet.policy(), wallet.verificationConfigs())),
            "hash must commit the full policy"
        );
        require(wallet.policyVersion() == 1, "initial version");

        BoundedV2FundingWallet.Policy memory next = _policy();
        next.maxPerBounty = 1_500;
        vm.prank(OWNER);
        wallet.configurePolicy(next, _configs());
        require(wallet.policyVersion() == 2, "version not bumped");
        require(wallet.policyHash() != before, "hash must change with any field");
        require(!wallet.revoked(), "replacement clears revocation");
    }

    function testInvalidPoliciesAreRejected() public {
        BoundedV2FundingWallet.Policy memory p = _policy();
        p.delegate = OWNER;
        _rejectPolicy(p, "delegate is owner");

        p = _policy();
        p.delegate = address(0);
        _rejectPolicy(p, "delegate zero");

        p = _policy();
        p.validUntil = uint64(block.timestamp - 1);
        p.validAfter = 0;
        _rejectPolicy(p, "policy already expired");

        p = _policy();
        p.periodSeconds = 0;
        _rejectPolicy(p, "period zero");

        p = _policy();
        p.maxClaimWindowSeconds = 30 days + 1;
        _rejectPolicy(p, "bad claim window cap");

        p = _policy();
        p.maxFundingWindowSeconds = 0;
        _rejectPolicy(p, "bad funding window cap");

        p = _policy();
        p.maxPerPeriod = p.maxPerBounty - 1;
        _rejectPolicy(p, "period cap below per-bounty cap");

        p = _policy();
        p.maxLifetimeSpend = p.maxPerPeriod - 1;
        _rejectPolicy(p, "lifetime cap below period cap");

        p = _policy();
        p.claimEligibilitySource = keccak256("source-without-registry");
        _rejectPolicy(p, "bad claim eligibility");
    }

    function testInvalidVerificationConfigsAreRejected() public {
        _rejectConfigs(new BoundedV2FundingWallet.VerificationConfig[](0), "bad verification config count");

        BoundedV2FundingWallet.VerificationConfig[] memory tooMany = new BoundedV2FundingWallet.VerificationConfig[](9);
        for (uint256 i = 0; i < 9; i++) {
            address[] memory set = new address[](1);
            set[0] = address(uint160(0x1000 + i));
            tooMany[i] = _quorumConfig(AgentBountyV2.VerificationMode.SignedQuorum, set, 1);
        }
        _rejectConfigs(tooMany, "bad verification config count");

        BoundedV2FundingWallet.VerificationConfig[] memory one = new BoundedV2FundingWallet.VerificationConfig[](1);
        one[0] = _moduleConfig();
        one[0].verifierModule = STRANGER;
        _rejectConfigs(one, "deterministic verifier unavailable");

        one[0] = _moduleConfig();
        one[0].verifierRewardRecipient = address(0);
        _rejectConfigs(one, "verifier recipient zero");

        one[0] = _moduleConfig();
        one[0].threshold = 2;
        _rejectConfigs(one, "bad deterministic config");

        one[0] = _moduleConfig();
        one[0].verifierSetHash = keccak256(abi.encode(quorum));
        _rejectConfigs(one, "bad deterministic config");

        one[0] = _quorumConfig(AgentBountyV2.VerificationMode.SignedQuorum, quorum, 2);
        one[0].verifierRewardRecipient = STRANGER;
        _rejectConfigs(one, "quorum module present");

        one[0] = _quorumConfig(AgentBountyV2.VerificationMode.SignedQuorum, quorum, 2);
        one[0].verifierSetHash = bytes32(0);
        _rejectConfigs(one, "verifier set zero");

        one[0] = _quorumConfig(AgentBountyV2.VerificationMode.SignedQuorum, quorum, 0);
        _rejectConfigs(one, "bad quorum threshold");

        one[0] = _quorumConfig(AgentBountyV2.VerificationMode.SignedQuorum, quorum, 9);
        _rejectConfigs(one, "bad quorum threshold");

        one[0] = _quorumConfig(AgentBountyV2.VerificationMode.AiJudgeQuorum, quorum, 1);
        _rejectConfigs(one, "bad quorum threshold");

        BoundedV2FundingWallet.VerificationConfig[] memory duplicate =
            new BoundedV2FundingWallet.VerificationConfig[](2);
        duplicate[0] = _moduleConfig();
        duplicate[1] = _moduleConfig();
        _rejectConfigs(duplicate, "duplicate verification config");
    }

    function testSeveralExactVerificationSetupsAreAllowedAndNearMissesRejected() public {
        address[] memory customerReview = new address[](1);
        customerReview[0] = vm.addr(0xC057);
        BoundedV2FundingWallet.VerificationConfig[] memory configs = new BoundedV2FundingWallet.VerificationConfig[](4);
        configs[0] = _moduleConfig();
        configs[1] = _quorumConfig(AgentBountyV2.VerificationMode.SignedQuorum, quorum, 2);
        configs[2] = _quorumConfig(AgentBountyV2.VerificationMode.SignedQuorum, customerReview, 1);
        configs[3] = _quorumConfig(AgentBountyV2.VerificationMode.AiJudgeQuorum, quorum, 2);
        vm.prank(OWNER);
        wallet.configurePolicy(_policy(), configs);
        require(wallet.verificationConfigs().length == 4, "configs not stored");
        require(
            wallet.policyHash() == keccak256(abi.encode(wallet.policy(), wallet.verificationConfigs())),
            "hash must commit the configs"
        );

        _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));
        _createWith(_quorumParams(AgentBountyV2.VerificationMode.SignedQuorum, 2), quorum);
        _createWith(_quorumParams(AgentBountyV2.VerificationMode.SignedQuorum, 1), customerReview);
        _createWith(_quorumParams(AgentBountyV2.VerificationMode.AiJudgeQuorum, 2), quorum);

        // The customer-review signer cannot be combined with the 2-of-2 threshold or swapped mode.
        _reverts(
            DELEGATE,
            address(wallet),
            _createCall(_quorumParams(AgentBountyV2.VerificationMode.AiJudgeQuorum, 2), customerReview),
            "verification config not allowed"
        );
        _reverts(
            DELEGATE,
            address(wallet),
            _createCall(_quorumParams(AgentBountyV2.VerificationMode.SignedQuorum, 1), quorum),
            "verification config not allowed"
        );
        address[] memory reordered = new address[](2);
        reordered[0] = quorum[1];
        reordered[1] = quorum[0];
        _reverts(
            DELEGATE,
            address(wallet),
            _createCall(_quorumParams(AgentBountyV2.VerificationMode.SignedQuorum, 2), reordered),
            "verification config not allowed"
        );
    }

    function testLifetimeCapCannotBeLoweredBelowWhatWasSpent() public {
        for (uint256 i = 0; i < 2; i++) {
            _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));
        }
        BoundedV2FundingWallet.Policy memory p = _policy();
        p.maxPerBounty = 1_000;
        p.maxPerPeriod = 1_000;
        p.maxLifetimeSpend = 2 * EXPECTED_TARGET - 1;
        _rejectPolicy(p, "lifetime cap below spent");
    }

    // ---------------------------------------------------------------- cap changes

    function testOwnerChangesOnlyTheCapsWithoutResettingSpend() public {
        _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));
        BoundedV2FundingWallet.Policy memory expected = wallet.policy();
        bytes32 hashBefore = wallet.policyHash();

        vm.prank(OWNER);
        wallet.setCaps(1_500, 2_000, type(uint256).max);
        expected.maxPerBounty = 1_500;
        expected.maxPerPeriod = 2_000;
        expected.maxLifetimeSpend = type(uint256).max;
        require(keccak256(abi.encode(wallet.policy())) == keccak256(abi.encode(expected)), "only caps may change");
        require(wallet.policyVersion() == 2 && wallet.policyHash() != hashBefore, "version or hash not updated");
        require(wallet.periodSpent() == EXPECTED_TARGET && wallet.lifetimeSpent() == EXPECTED_TARGET, "spend was reset");

        // Today's earlier spend counts against the lowered period cap: 1,175 + 1,175 > 2,000.
        _reverts(
            DELEGATE,
            address(wallet),
            _createCall(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD), new address[](0)),
            "period cap exceeded"
        );
        // 1,400 + 100 + 105 fee > 1,500.
        _reverts(
            DELEGATE,
            address(wallet),
            _createCall(_moduleParams(1_400, 100), new address[](0)),
            "per-bounty cap exceeded"
        );
        _create(_moduleParams(500, 100));
        require(wallet.periodSpent() <= 2_000, "period cap breached");

        // Raised caps apply at once; with no lifetime cap only the float bounds total spend.
        vm.warp(block.timestamp + 1 days);
        vm.prank(OWNER);
        wallet.setCaps(2_500, 2_500, type(uint256).max);
        AgentBountyV2 large = _create(_moduleParams(1_900, 100));
        require(large.targetAmount() > 2_000 && large.targetAmount() <= 2_500, "raised cap not applied");
    }

    /// The card-quote API reads caps with these selectors and word positions (managed_bounty_quotes.rs).
    function testCapReadsUseTheLayoutTheQuoteApiDecodes() public {
        require(wallet.policy.selector == bytes4(0x0505c8c9), "policy() selector");
        require(wallet.revoked.selector == bytes4(0x63d256ce), "revoked() selector");
        require(wallet.lifetimeSpent.selector == bytes4(0xb80762dd), "lifetimeSpent() selector");
        vm.prank(OWNER);
        wallet.setCaps(200e6, 200e6, type(uint256).max);
        (bool ok, bytes memory data) = address(wallet).staticcall(abi.encodeWithSelector(0x0505c8c9));
        require(ok && data.length == 13 * 32, "policy() must return 13 static words");
        uint256[13] memory words = abi.decode(data, (uint256[13]));
        require(words[2] == wallet.policy().validUntil, "validUntil is word 2");
        require(words[8] == 200e6 && words[9] == 200e6 && words[10] == type(uint256).max, "caps are words 8 to 10");
    }

    function testCapChangesKeepRevocationAndValidateEveryCap() public {
        _create(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD));
        _reverts(OWNER, address(wallet), abi.encodeCall(wallet.setCaps, (0, 2_000, 5_000)), "per-bounty cap zero");
        _reverts(
            OWNER,
            address(wallet),
            abi.encodeCall(wallet.setCaps, (2_000, 1_999, 5_000)),
            "period cap below per-bounty cap"
        );
        _reverts(
            OWNER,
            address(wallet),
            abi.encodeCall(wallet.setCaps, (1_000, 2_000, 1_999)),
            "lifetime cap below period cap"
        );
        _reverts(
            OWNER,
            address(wallet),
            abi.encodeCall(wallet.setCaps, (500, 500, EXPECTED_TARGET - 1)),
            "lifetime cap below spent"
        );
        _reverts(STRANGER, address(wallet), abi.encodeCall(wallet.setCaps, (1, 1, type(uint256).max)), "not owner");

        vm.prank(OWNER);
        wallet.revokePolicy();
        vm.prank(OWNER);
        wallet.setCaps(2_000, 5_000, type(uint256).max);
        require(wallet.revoked(), "a cap change must not lift revocation");
        _reverts(
            DELEGATE,
            address(wallet),
            _createCall(_moduleParams(SOLVER_REWARD, VERIFIER_REWARD), new address[](0)),
            "policy revoked"
        );
    }

    // ---------------------------------------------------------------- fuzz

    function testFuzzSpendStaysWithinCapsAndConservesFunds(uint256 seed) public {
        BoundedV2FundingWallet.Policy memory p = wallet.policy();
        uint256 escrowed;
        for (uint256 i = 0; i < 24; i++) {
            seed = uint256(keccak256(abi.encode(seed, i)));
            uint256 solverReward = 1 + (seed % 2_200);
            uint256 verifierReward = 1 + ((seed >> 32) % 200);
            if ((seed >> 64) % 3 == 0) vm.warp(block.timestamp + 1 + ((seed >> 72) % 2 days));

            vm.prank(DELEGATE);
            (bool ok, bytes memory ret) =
                address(wallet).call(_createCall(_moduleParams(solverReward, verifierReward), new address[](0)));
            if (ok) {
                (address bounty,, uint256 target) = abi.decode(ret, (address, bytes32, uint256));
                (, uint256 quoted) = factory.quote(solverReward, verifierReward);
                require(target == quoted && target <= p.maxPerBounty, "target outside policy");
                escrowed += AgentBountyV2(bounty).contributions(address(wallet));
            }
            require(wallet.periodSpent() <= p.maxPerPeriod, "period cap breached");
            require(wallet.lifetimeSpent() <= p.maxLifetimeSpend, "lifetime cap breached");
            require(token.balanceOf(address(wallet)) + escrowed == WALLET_FLOAT, "funds not conserved");
            require(wallet.lifetimeSpent() == escrowed, "spend accounting drifted");
        }
    }

    function testFuzzCapChangesNeverAllowSpendAboveTheCapsInForce(uint256 seed) public {
        uint256 escrowed;
        for (uint256 i = 0; i < 24; i++) {
            seed = uint256(keccak256(abi.encode(seed, i)));
            if ((seed >> 80) % 4 == 0) {
                uint256 perBounty = 1 + ((seed >> 96) % 3_000);
                uint256 perPeriod = perBounty + ((seed >> 128) % 3_000);
                uint256 lifetime = (seed >> 160) % 2 == 0
                    ? type(uint256).max
                    : _max(perPeriod, wallet.lifetimeSpent()) + ((seed >> 168) % 5_000);
                vm.prank(OWNER);
                wallet.setCaps(perBounty, perPeriod, lifetime);
            }
            if ((seed >> 64) % 3 == 0) vm.warp(block.timestamp + 1 + ((seed >> 72) % 2 days));
            uint256 periodBefore = wallet.periodBucket() == block.timestamp / 1 days ? wallet.periodSpent() : 0;
            uint256 solverReward = 1 + (seed % 2_200);
            uint256 verifierReward = 1 + ((seed >> 32) % 200);

            vm.prank(DELEGATE);
            (bool ok, bytes memory ret) =
                address(wallet).call(_createCall(_moduleParams(solverReward, verifierReward), new address[](0)));
            if (ok) {
                BoundedV2FundingWallet.Policy memory p = wallet.policy();
                (address bounty,, uint256 target) = abi.decode(ret, (address, bytes32, uint256));
                require(target <= p.maxPerBounty, "per-bounty cap breached");
                require(periodBefore + target <= p.maxPerPeriod, "period cap breached");
                require(wallet.lifetimeSpent() <= p.maxLifetimeSpend, "lifetime cap breached");
                escrowed += AgentBountyV2(bounty).contributions(address(wallet));
            }
            require(token.balanceOf(address(wallet)) + escrowed == WALLET_FLOAT, "funds not conserved");
            require(wallet.lifetimeSpent() == escrowed, "spend accounting drifted");
        }
    }

    // ---------------------------------------------------------------- helpers

    function _max(uint256 a, uint256 b) private pure returns (uint256) {
        return a > b ? a : b;
    }

    function _policy() private view returns (BoundedV2FundingWallet.Policy memory) {
        return BoundedV2FundingWallet.Policy({
            delegate: DELEGATE,
            validAfter: 0,
            validUntil: uint64(block.timestamp + 365 days),
            periodSeconds: 1 days,
            maxFundingWindowSeconds: 30 days,
            maxClaimWindowSeconds: 7 days,
            maxVerificationWindowSeconds: 7 days,
            delegateMayCancel: true,
            maxPerBounty: 2_000,
            maxPerPeriod: 5_000,
            maxLifetimeSpend: 10_000,
            claimEligibilityRegistry: address(0),
            claimEligibilitySource: bytes32(0)
        });
    }

    function _configs() private view returns (BoundedV2FundingWallet.VerificationConfig[] memory configs) {
        configs = new BoundedV2FundingWallet.VerificationConfig[](2);
        configs[0] = _moduleConfig();
        configs[1] = _quorumConfig(AgentBountyV2.VerificationMode.SignedQuorum, quorum, 2);
    }

    function _moduleConfig() private view returns (BoundedV2FundingWallet.VerificationConfig memory) {
        return BoundedV2FundingWallet.VerificationConfig({
            mode: AgentBountyV2.VerificationMode.DeterministicModule,
            verifierModule: address(module),
            verifierRewardRecipient: VERIFIER_RECIPIENT,
            verifierSetHash: bytes32(0),
            threshold: 1
        });
    }

    function _quorumConfig(AgentBountyV2.VerificationMode mode, address[] memory verifiers, uint8 threshold)
        private
        pure
        returns (BoundedV2FundingWallet.VerificationConfig memory)
    {
        return BoundedV2FundingWallet.VerificationConfig({
            mode: mode,
            verifierModule: address(0),
            verifierRewardRecipient: address(0),
            verifierSetHash: keccak256(abi.encode(verifiers)),
            threshold: threshold
        });
    }

    function _rejectConfigs(BoundedV2FundingWallet.VerificationConfig[] memory configs, string memory reason) private {
        _reverts(OWNER, address(wallet), abi.encodeCall(wallet.configurePolicy, (_policy(), configs)), reason);
    }

    function _moduleParams(uint256 solverReward, uint256 verifierReward)
        private
        view
        returns (AgentBountyFactoryV2.CreateBountyParams memory)
    {
        return AgentBountyFactoryV2.CreateBountyParams({
            solverReward: solverReward,
            verifierReward: verifierReward,
            termsHash: keccak256("terms"),
            policyHash: keccak256("policy"),
            acceptanceCriteriaHash: keccak256("criteria"),
            benchmarkHash: keccak256("benchmark"),
            evidenceSchemaHash: keccak256("evidence-schema"),
            fundingDeadline: uint64(block.timestamp + 1 days),
            claimWindowSeconds: 1 days,
            verificationWindowSeconds: 1 days,
            verificationMode: AgentBountyV2.VerificationMode.DeterministicModule,
            verifierModule: address(module),
            verifierRewardRecipient: VERIFIER_RECIPIENT,
            threshold: 1,
            claimEligibilityRegistry: address(0),
            claimEligibilitySource: bytes32(0)
        });
    }

    function _quorumParams(AgentBountyV2.VerificationMode mode, uint8 threshold)
        private
        view
        returns (AgentBountyFactoryV2.CreateBountyParams memory params)
    {
        params = _moduleParams(SOLVER_REWARD, VERIFIER_REWARD);
        params.verificationMode = mode;
        params.verifierModule = address(0);
        params.verifierRewardRecipient = address(0);
        params.threshold = threshold;
    }

    function _createCall(AgentBountyFactoryV2.CreateBountyParams memory params, address[] memory verifiers)
        private
        returns (bytes memory)
    {
        nonceCounter += 1;
        return abi.encodeCall(wallet.createBounty, (params, verifiers, bytes32(nonceCounter)));
    }

    function _create(AgentBountyFactoryV2.CreateBountyParams memory params) private returns (AgentBountyV2) {
        return _createWith(params, new address[](0));
    }

    function _createWith(AgentBountyFactoryV2.CreateBountyParams memory params, address[] memory verifiers)
        private
        returns (AgentBountyV2)
    {
        nonceCounter += 1;
        vm.prank(DELEGATE);
        (address bounty,,) = wallet.createBounty(params, verifiers, bytes32(nonceCounter));
        return AgentBountyV2(bounty);
    }

    function _solve(AgentBountyV2 bounty) private returns (FundingWalletSolver solver) {
        solver = new FundingWalletSolver();
        token.mint(address(solver), bounty.verifierReward());
        solver.claimAndSubmit(bounty, token);
    }

    function _rejectPolicy(BoundedV2FundingWallet.Policy memory p, string memory reason) private {
        _reverts(OWNER, address(wallet), abi.encodeCall(wallet.configurePolicy, (p, _configs())), reason);
    }

    function _reverts(address sender, address target, bytes memory data, string memory reason) private {
        vm.prank(sender);
        (bool ok, bytes memory returned) = target.call(data);
        require(!ok, string.concat("expected revert: ", reason));
        require(
            keccak256(returned) == keccak256(abi.encodeWithSignature("Error(string)", reason)),
            string.concat("wrong revert, expected: ", reason)
        );
    }
}
