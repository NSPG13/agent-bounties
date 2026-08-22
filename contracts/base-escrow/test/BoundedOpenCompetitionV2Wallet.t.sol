// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/BoundedOpenCompetitionV2WalletFactory.sol";
import {ISP1VerifierWithHash} from "../src/ISP1Verifier.sol";

interface VmBoundedCompetitionReserve {
    function etch(address target, bytes calldata code) external;
    function expectRevert(bytes4 selector) external;
    function prank(address sender) external;
    function warp(uint256 timestamp) external;
}

contract BoundedCompetitionReserveToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    mapping(address => mapping(bytes32 => bool)) public authorizationUsed;

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

    function transferWithAuthorization(
        address from,
        address to,
        uint256 amount,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8,
        bytes32,
        bytes32
    ) external {
        require(block.timestamp > validAfter && block.timestamp < validBefore, "authorization timing");
        require(!authorizationUsed[from][nonce], "authorization used");
        require(balanceOf[from] >= amount, "balance");
        authorizationUsed[from][nonce] = true;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
    }
}

abstract contract BoundedCompetitionReserveVerifier is ISP1VerifierWithHash {
    function verifyProof(bytes32, bytes calldata, bytes calldata) external pure {}
}

contract BoundedCompetitionReserveGroth16 is BoundedCompetitionReserveVerifier {
    function VERIFIER_HASH() external pure returns (bytes32) {
        return 0x4388a21c00000000000000000000000000000000000000000000000000000001;
    }
}

contract BoundedCompetitionReservePlonk is BoundedCompetitionReserveVerifier {
    function VERIFIER_HASH() external pure returns (bytes32) {
        return 0x5a093a2f00000000000000000000000000000000000000000000000000000001;
    }
}

contract BoundedOpenCompetitionV2WalletTest {
    VmBoundedCompetitionReserve constant vm =
        VmBoundedCompetitionReserve(address(uint160(uint256(keccak256("hevm cheat code")))));

    bytes32 constant PROGRAM_VKEY = keccak256("public-vector-metric-v1-vkey");
    bytes32 constant SOURCE_HASH = keccak256("public-vector-metric-v1-source");
    bytes32 constant ELF_HASH = keccak256("public-vector-metric-v1-elf");
    bytes32 constant JOURNAL_SCHEMA_HASH = keccak256("open-competition-v2-journal-schema");
    bytes32 constant METRIC_PROGRAM_HASH = keccak256("public-vector-metric-v1");
    bytes32 constant EXECUTION_POLICY_HASH = keccak256("execution-policy");
    bytes32 constant VERIFICATION_POLICY_HASH = keccak256("verification-policy");
    bytes32 constant SETTLEMENT_POLICY_HASH = keccak256("settlement-policy");
    bytes32 constant BETA_RISK_HASH = keccak256("open-competition-v2-beta3-risk");

    uint256 constant SOLVER_REWARD = 3_000_000;
    uint256 constant KEEPER_REWARD = 40_000;
    uint256 constant EXACT_FUNDING = SOLVER_REWARD + KEEPER_REWARD;
    uint256 constant DAILY_CAP = 10 * EXACT_FUNDING;
    uint256 constant RESERVE_FUNDING = 77_668_098;
    uint64 constant PERIOD_SECONDS = 1 days;
    address constant DELEGATE = address(0xD311);
    address constant OUTSIDER = address(0xBAD);

    BoundedCompetitionReserveToken token;
    BoundedCompetitionReserveGroth16 groth16Verifier;
    BoundedCompetitionReservePlonk plonkVerifier;
    OpenCompetitionBountyFactoryV2Beta3 competitionFactory;
    BoundedOpenCompetitionV2WalletFactory reserveFactory;
    BoundedOpenCompetitionV2Wallet reserve;
    OpenCompetitionBountyFactoryV2Beta3.CreateCompetitionParams params;
    bytes32[] approvedCreations;

    function setUp() public {
        token = new BoundedCompetitionReserveToken();
        groth16Verifier = new BoundedCompetitionReserveGroth16();
        plonkVerifier = new BoundedCompetitionReservePlonk();
        competitionFactory = new OpenCompetitionBountyFactoryV2Beta3(
            address(token),
            address(groth16Verifier),
            groth16Verifier.VERIFIER_HASH(),
            address(groth16Verifier).codehash,
            address(plonkVerifier),
            plonkVerifier.VERIFIER_HASH(),
            address(plonkVerifier).codehash
        );
        reserveFactory = new BoundedOpenCompetitionV2WalletFactory(address(competitionFactory));
        params = _params();
        approvedCreations = _commitments(params, 12);

        BoundedOpenCompetitionV2Wallet.Policy memory policy = _policy(DAILY_CAP, RESERVE_FUNDING);
        address predicted = reserveFactory.predictWallet(address(this), policy, approvedCreations, bytes32("primary"));
        token.mint(address(this), RESERVE_FUNDING);
        token.approve(address(reserveFactory), RESERVE_FUNDING);
        reserve = BoundedOpenCompetitionV2Wallet(
            payable(reserveFactory.createWalletAndFund(policy, approvedCreations, bytes32("primary"), RESERVE_FUNDING))
        );

        require(address(reserve) == predicted, "reserve prediction mismatch");
        require(reserve.owner() == address(this), "owner mismatch");
        require(token.balanceOf(address(reserve)) == RESERVE_FUNDING, "reserve not funded");
        require(token.balanceOf(DELEGATE) == 0, "delegate received custody funds");
    }

    function testDelegateCreatesOnlyApprovedCanonicalActiveCompetition() public {
        (address competition,) = _create(1);

        OpenCompetitionBountyV2Beta3 bounty = OpenCompetitionBountyV2Beta3(competition);
        require(competitionFactory.isCanonicalCompetition(competition), "not canonical");
        require(bounty.creator() == address(reserve), "reserve not creator");
        require(bounty.fundedAmount() == EXACT_FUNDING, "funding mismatch");
        require(
            bounty.competitionStatus() == OpenCompetitionBountyV2Beta3.CompetitionStatus.Active,
            "competition not active"
        );
        require(token.balanceOf(address(reserve)) == RESERVE_FUNDING - EXACT_FUNDING, "reserve spend mismatch");
        require(token.allowance(address(reserve), competition) == 0, "allowance retained");
        require(reserve.periodSpent() == EXACT_FUNDING, "period spend mismatch");
        require(reserve.lifetimeSpent() == EXACT_FUNDING, "lifetime spend mismatch");
    }

    function testUnauthorizedAndUnapprovedCreationFailClosed() public {
        vm.expectRevert(BoundedOpenCompetitionV2Wallet.ReserveNotDelegate.selector);
        vm.prank(OUTSIDER);
        reserve.createCompetition(params, bytes32(uint256(1)));

        vm.expectRevert(BoundedOpenCompetitionV2Wallet.ReserveCreationNotApproved.selector);
        vm.prank(DELEGATE);
        reserve.createCompetition(params, bytes32(uint256(999)));
    }

    function testApprovedCreationCannotBeReplayed() public {
        _create(1);

        vm.expectRevert(BoundedOpenCompetitionV2Wallet.ReserveCreationAlreadyUsed.selector);
        vm.prank(DELEGATE);
        reserve.createCompetition(params, bytes32(uint256(1)));
    }

    function testExactEconomicsAndRiskHashAreEnforced() public {
        OpenCompetitionBountyFactoryV2Beta3.CreateCompetitionParams memory changed = params;
        changed.solverReward -= 1;
        vm.expectRevert(BoundedOpenCompetitionV2Wallet.ReserveEconomicsMismatch.selector);
        vm.prank(DELEGATE);
        reserve.createCompetition(changed, bytes32(uint256(1)));

        changed = params;
        changed.betaRiskHash = keccak256("different risk");
        vm.expectRevert(BoundedOpenCompetitionV2Wallet.ReserveRiskHashMismatch.selector);
        vm.prank(DELEGATE);
        reserve.createCompetition(changed, bytes32(uint256(1)));
    }

    function testDailyCapStopsEleventhCreationAndResetsNextPeriod() public {
        for (uint256 index = 1; index <= 10; index++) {
            _create(index);
        }

        vm.expectRevert(BoundedOpenCompetitionV2Wallet.ReservePerPeriodCapExceeded.selector);
        vm.prank(DELEGATE);
        reserve.createCompetition(params, bytes32(uint256(11)));

        vm.warp(block.timestamp + PERIOD_SECONDS);
        _create(11);
        require(reserve.periodSpent() == EXACT_FUNDING, "period did not reset");
        require(reserve.lifetimeSpent() == 11 * EXACT_FUNDING, "lifetime spend reset");
    }

    function testLifetimeCapCannotBeExceededEvenWhenReserveHasMoreFunds() public {
        bytes32[] memory threeApproved = _commitments(params, 3);
        uint256 funding = 3 * EXACT_FUNDING;
        BoundedOpenCompetitionV2Wallet.Policy memory limitedPolicy = _policy(funding, 2 * EXACT_FUNDING);
        token.mint(address(this), funding);
        token.approve(address(reserveFactory), funding);
        BoundedOpenCompetitionV2Wallet limited = BoundedOpenCompetitionV2Wallet(
            payable(reserveFactory.createWalletAndFund(
                    limitedPolicy, threeApproved, bytes32("lifetime-limited"), funding
                ))
        );

        _createWith(limited, 1);
        _createWith(limited, 2);
        vm.expectRevert(BoundedOpenCompetitionV2Wallet.ReserveLifetimeCapExceeded.selector);
        vm.prank(DELEGATE);
        limited.createCompetition(params, bytes32(uint256(3)));

        require(limited.lifetimeSpent() == 2 * EXACT_FUNDING, "lifetime accounting drift");
        require(token.balanceOf(address(limited)) == EXACT_FUNDING, "failed spend moved funds");
    }

    function testPolicyUpdateCannotResetPeriodOrLifetimeSpend() public {
        _create(1);
        bytes32[] memory nextApproved = new bytes32[](1);
        nextApproved[0] = _commitment(params, bytes32(uint256(20)));

        reserve.configurePolicy(_policy(DAILY_CAP, RESERVE_FUNDING), nextApproved);

        require(reserve.periodSpent() == EXACT_FUNDING, "period spend reset");
        require(reserve.lifetimeSpent() == EXACT_FUNDING, "lifetime spend reset");
        vm.expectRevert(BoundedOpenCompetitionV2Wallet.ReserveInvalidPolicy.selector);
        reserve.configurePolicy(_policyWithPeriod(DAILY_CAP, RESERVE_FUNDING, 2 days), nextApproved);
    }

    function testOwnerRecoversUncommittedFundsOnlyAfterRevocation() public {
        _create(1);
        uint256 ownerBefore = token.balanceOf(address(this));

        vm.expectRevert(BoundedOpenCompetitionV2Wallet.ReserveRecoveryRequiresRevocation.selector);
        reserve.recoverUncommitted();

        vm.expectRevert(BoundedOpenCompetitionV2Wallet.ReserveNotOwner.selector);
        vm.prank(DELEGATE);
        reserve.recoverUncommitted();

        reserve.revokePolicy();
        uint256 recovered = reserve.recoverUncommitted();
        require(recovered == RESERVE_FUNDING - EXACT_FUNDING, "wrong recovery amount");
        require(token.balanceOf(address(this)) == ownerBefore + recovered, "owner not repaid");
        require(token.balanceOf(address(reserve)) == 0, "reserve retained uncommitted funds");

        vm.expectRevert(BoundedOpenCompetitionV2Wallet.ReservePolicyRevoked.selector);
        vm.prank(DELEGATE);
        reserve.createCompetition(params, bytes32(uint256(2)));
    }

    function testOwnerRecoversExpiredCompetitionRefundAndKeeperReward() public {
        (address competition,) = _create(1);
        OpenCompetitionBountyV2Beta3 bounty = OpenCompetitionBountyV2Beta3(competition);
        reserve.revokePolicy();
        reserve.recoverUncommitted();

        vm.warp(bounty.proofDeadline() + 1);
        uint256 refund = reserve.expireAndPullRefund(competition);
        require(refund == SOLVER_REWARD, "solver refund mismatch");
        require(token.balanceOf(address(reserve)) == EXACT_FUNDING, "refund and keeper not returned");

        reserve.recoverUncommitted();
        require(token.balanceOf(address(this)) == RESERVE_FUNDING, "owner did not recover full reserve");
        require(token.balanceOf(address(reserve)) == 0, "reserve retained recovered funds");
        require(token.balanceOf(competition) == 0, "competition retained funds");
    }

    function testHealthyActiveEscrowCannotBeClawedBackEarly() public {
        (address competition,) = _create(1);
        reserve.revokePolicy();
        reserve.recoverUncommitted();

        vm.expectRevert(OpenCompetitionBountyV2Beta3.V2FinalizeTooEarly.selector);
        reserve.expireAndPullRefund(competition);
        require(token.balanceOf(competition) == EXACT_FUNDING, "active escrow moved early");
    }

    function testOwnerPullsRefundAfterThirdPartyExpiryWithoutTakingKeeperReward() public {
        (address competition,) = _create(1);
        OpenCompetitionBountyV2Beta3 bounty = OpenCompetitionBountyV2Beta3(competition);
        reserve.revokePolicy();
        reserve.recoverUncommitted();
        vm.warp(bounty.proofDeadline() + 1);

        vm.prank(OUTSIDER);
        bounty.expireCompetition();
        uint256 refund = reserve.pullCancelledRefund(competition);
        require(refund == SOLVER_REWARD, "creator refund mismatch");
        require(token.balanceOf(OUTSIDER) == KEEPER_REWARD, "keeper not paid");

        reserve.recoverUncommitted();
        require(token.balanceOf(address(this)) == RESERVE_FUNDING - KEEPER_REWARD, "owner recovery mismatch");
    }

    function testOwnerRecoversOnVerifierUnavailability() public {
        (address competition,) = _create(1);
        OpenCompetitionBountyV2Beta3 bounty = OpenCompetitionBountyV2Beta3(competition);
        reserve.revokePolicy();
        reserve.recoverUncommitted();
        vm.etch(bounty.verifierAdapter(), hex"");

        uint256 refund = reserve.cancelUnavailableAndPullRefund(competition);
        require(refund == SOLVER_REWARD, "unavailable refund mismatch");
        require(token.balanceOf(address(reserve)) == EXACT_FUNDING, "unavailable funds not returned");
        reserve.recoverUncommitted();
        require(token.balanceOf(address(this)) == RESERVE_FUNDING, "owner not made whole");
    }

    function testTwoStepOwnershipMovesRecoveryAuthority() public {
        address nextOwner = address(0xA11CE);
        reserve.revokePolicy();
        reserve.transferOwnership(nextOwner);

        vm.expectRevert(BoundedOpenCompetitionV2Wallet.ReserveNotPendingOwner.selector);
        vm.prank(OUTSIDER);
        reserve.acceptOwnership();

        vm.prank(nextOwner);
        reserve.acceptOwnership();
        vm.expectRevert(BoundedOpenCompetitionV2Wallet.ReserveNotOwner.selector);
        reserve.recoverUncommitted();

        vm.prank(nextOwner);
        reserve.recoverUncommitted();
        require(token.balanceOf(nextOwner) == RESERVE_FUNDING, "new owner not repaid");
    }

    function testAtomicAuthorizationFundingAndDuplicateAttemptFailClosed() public {
        bytes32 userSalt = bytes32("authorization");
        bytes32 authorizationNonce = keccak256("usdc authorization");
        BoundedOpenCompetitionV2Wallet.Policy memory policy = _policy(DAILY_CAP, RESERVE_FUNDING);
        address predicted = reserveFactory.predictWallet(address(this), policy, approvedCreations, userSalt);
        token.mint(address(this), RESERVE_FUNDING);

        address wallet = reserveFactory.createWalletWithAuthorization(
            address(this),
            policy,
            approvedCreations,
            userSalt,
            RESERVE_FUNDING,
            0,
            block.timestamp + 1 days,
            authorizationNonce,
            0,
            bytes32(0),
            bytes32(0)
        );
        require(wallet == predicted, "authorization prediction mismatch");
        require(token.balanceOf(wallet) == RESERVE_FUNDING, "authorization funding mismatch");

        vm.expectRevert(BoundedOpenCompetitionV2WalletFactory.ReserveFactoryWalletOccupied.selector);
        reserveFactory.createWalletWithAuthorization(
            address(this),
            policy,
            approvedCreations,
            userSalt,
            RESERVE_FUNDING,
            0,
            block.timestamp + 1 days,
            keccak256("second authorization"),
            0,
            bytes32(0),
            bytes32(0)
        );
    }

    function testOutsiderCannotPredeployOwnersPredictedWallet() public {
        bytes32 userSalt = bytes32("owner-only-deployment");
        BoundedOpenCompetitionV2Wallet.Policy memory policy = _policy(DAILY_CAP, RESERVE_FUNDING);

        vm.expectRevert(BoundedOpenCompetitionV2WalletFactory.ReserveFactoryNotOwner.selector);
        vm.prank(OUTSIDER);
        reserveFactory.createWallet(address(this), policy, approvedCreations, userSalt);

        address wallet = reserveFactory.createWallet(address(this), policy, approvedCreations, userSalt);
        require(
            wallet == reserveFactory.predictWallet(address(this), policy, approvedCreations, userSalt), "wrong wallet"
        );
    }

    function _create(uint256 nonce) private returns (address competition, bytes32 bountyId) {
        return _createWith(reserve, nonce);
    }

    function _createWith(BoundedOpenCompetitionV2Wallet target, uint256 nonce)
        private
        returns (address competition, bytes32 bountyId)
    {
        vm.prank(DELEGATE);
        return target.createCompetition(params, bytes32(nonce));
    }

    function _params() private view returns (OpenCompetitionBountyFactoryV2Beta3.CreateCompetitionParams memory) {
        return OpenCompetitionBountyFactoryV2Beta3.CreateCompetitionParams({
            solverReward: SOLVER_REWARD,
            keeperReward: KEEPER_REWARD,
            fundingDeadline: uint64(block.timestamp + 7 days),
            proofWindowSeconds: 3 days,
            winnerMode: OpenCompetitionBountyV2Beta3.WinnerMode.FirstProven,
            scoreDirection: OpenCompetitionBountyV2Beta3.ScoreDirection.HigherIsBetter,
            scoreThreshold: 0,
            proofSystem: competitionFactory.PROOF_SYSTEM_GROTH16(),
            programVKey: PROGRAM_VKEY,
            sourceHash: SOURCE_HASH,
            elfHash: ELF_HASH,
            journalSchemaHash: JOURNAL_SCHEMA_HASH,
            metricProgramHash: METRIC_PROGRAM_HASH,
            executionPolicyHash: EXECUTION_POLICY_HASH,
            verificationPolicyHash: VERIFICATION_POLICY_HASH,
            settlementPolicyHash: SETTLEMENT_POLICY_HASH,
            betaRiskHash: BETA_RISK_HASH
        });
    }

    function _policy(uint256 maxPerPeriod, uint256 maxLifetimeSpend)
        private
        view
        returns (BoundedOpenCompetitionV2Wallet.Policy memory)
    {
        return _policyWithPeriod(maxPerPeriod, maxLifetimeSpend, PERIOD_SECONDS);
    }

    function _policyWithPeriod(uint256 maxPerPeriod, uint256 maxLifetimeSpend, uint64 periodSeconds)
        private
        view
        returns (BoundedOpenCompetitionV2Wallet.Policy memory)
    {
        return BoundedOpenCompetitionV2Wallet.Policy({
            delegate: DELEGATE,
            validAfter: uint64(block.timestamp),
            validUntil: uint64(block.timestamp + 30 days),
            periodSeconds: periodSeconds,
            solverReward: SOLVER_REWARD,
            keeperReward: KEEPER_REWARD,
            exactFundingPerCompetition: EXACT_FUNDING,
            maxPerPeriod: maxPerPeriod,
            maxLifetimeSpend: maxLifetimeSpend,
            betaRiskHash: BETA_RISK_HASH
        });
    }

    function _commitments(OpenCompetitionBountyFactoryV2Beta3.CreateCompetitionParams memory candidate, uint256 count)
        private
        view
        returns (bytes32[] memory commitments)
    {
        commitments = new bytes32[](count);
        for (uint256 index = 0; index < count; index++) {
            commitments[index] = _commitment(candidate, bytes32(index + 1));
        }
    }

    function _commitment(
        OpenCompetitionBountyFactoryV2Beta3.CreateCompetitionParams memory candidate,
        bytes32 creationNonce
    ) private view returns (bytes32) {
        return keccak256(abi.encode(block.chainid, address(competitionFactory), candidate, creationNonce));
    }
}
