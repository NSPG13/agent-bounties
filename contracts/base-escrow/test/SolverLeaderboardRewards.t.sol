// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/SolverLeaderboardRewards.sol";

interface VmLeaderboard {
    function addr(uint256 privateKey) external returns (address keyAddr);
    function expectRevert(bytes calldata revertData) external;
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
    function warp(uint256 timestamp) external;
}

contract LeaderboardToken {
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

contract Leaderboard1271Signer {
    bytes4 private constant MAGIC_VALUE = 0x1626ba7e;
    bytes32 private approvedDigest;
    bytes32 private approvedSignatureHash;

    function approve(bytes32 digest, bytes calldata signature) external {
        approvedDigest = digest;
        approvedSignatureHash = keccak256(signature);
    }

    function isValidSignature(bytes32 digest, bytes calldata signature) external view returns (bytes4) {
        if (digest == approvedDigest && keccak256(signature) == approvedSignatureHash) {
            return MAGIC_VALUE;
        }
        return 0xffffffff;
    }
}

contract SolverLeaderboardRewardsTest {
    VmLeaderboard private constant vm = VmLeaderboard(address(uint160(uint256(keccak256("hevm cheat code")))));

    uint256 private constant SIGNER_A_KEY = 0xA11CE;
    uint256 private constant SIGNER_B_KEY = 0xB0B;
    uint256 private constant WRONG_KEY = 0xBAD;
    uint64 private constant DAILY_START = uint64(20_000 days);
    uint64 private constant WEEKLY_START = uint64(4 days + 2_856 * 7 days);
    address private constant WINNER = address(0xCAFE);
    bytes32 private constant EVIDENCE_HASH = keccak256("canonical leaderboard response");

    LeaderboardToken private token;
    SolverLeaderboardRewards private rewards;

    function setUp() public {
        vm.warp(uint256(DAILY_START) + 12 hours);
        token = new LeaderboardToken();
        rewards = new SolverLeaderboardRewards(address(token), vm.addr(SIGNER_A_KEY), vm.addr(SIGNER_B_KEY));
        token.mint(address(this), 100_000_000);
        token.approve(address(rewards), 100_000_000);
        rewards.fund(100_000_000);
        assert(rewards.firstDailyStart() == DAILY_START);
        assert(rewards.firstWeeklyStart() == WEEKLY_START);
    }

    function testPaysDailyRewardAfterFinalization() public {
        uint64 endsAt = DAILY_START + 1 days;
        (bytes memory signatureA, bytes memory signatureB) =
            _sign(SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START, 4);
        vm.warp(uint256(endsAt) + rewards.FINALIZATION_DELAY());

        rewards.pay(
            SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START, WINNER, 4, EVIDENCE_HASH, signatureA, signatureB
        );

        assert(token.balanceOf(WINNER) == 3_000_000);
        assert(rewards.paidAwards(rewards.awardId(SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START)));
        assert(
            rewards.paidAwardWinner(rewards.awardId(SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START)) == WINNER
        );
    }

    function testPaysWeeklyRewardWithReversedSignatureOrder() public {
        uint64 endsAt = WEEKLY_START + 7 days;
        (bytes memory signatureA, bytes memory signatureB) =
            _sign(SolverLeaderboardRewards.PeriodKind.Weekly, WEEKLY_START, 9);
        vm.warp(uint256(endsAt) + rewards.FINALIZATION_DELAY());

        rewards.pay(
            SolverLeaderboardRewards.PeriodKind.Weekly, WEEKLY_START, WINNER, 9, EVIDENCE_HASH, signatureB, signatureA
        );

        assert(token.balanceOf(WINNER) == 26_000_000);
    }

    function testRejectsReplay() public {
        uint64 endsAt = DAILY_START + 1 days;
        (bytes memory signatureA, bytes memory signatureB) =
            _sign(SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START, 1);
        vm.warp(uint256(endsAt) + rewards.FINALIZATION_DELAY());
        rewards.pay(
            SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START, WINNER, 1, EVIDENCE_HASH, signatureA, signatureB
        );

        vm.expectRevert(bytes("award paid"));
        rewards.pay(
            SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START, WINNER, 1, EVIDENCE_HASH, signatureA, signatureB
        );
    }

    function testRejectsOneSignerAndPrematureFinalization() public {
        uint64 endsAt = DAILY_START + 1 days;
        bytes32 digest =
            rewards.awardDigest(SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START, WINNER, 2, EVIDENCE_HASH);
        bytes memory signatureA = _signature(SIGNER_A_KEY, digest);
        bytes memory wrongSignature = _signature(WRONG_KEY, digest);
        vm.warp(uint256(endsAt) + rewards.FINALIZATION_DELAY());
        vm.expectRevert(bytes("invalid quorum"));
        rewards.pay(
            SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START, WINNER, 2, EVIDENCE_HASH, signatureA, wrongSignature
        );

        (, bytes memory signatureB) = _sign(SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START, 2);
        vm.warp(endsAt);
        vm.expectRevert(bytes("period not final"));
        rewards.pay(
            SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START, WINNER, 2, EVIDENCE_HASH, signatureA, signatureB
        );
    }

    function testRejectsMisalignedPeriods() public {
        vm.warp(uint256(DAILY_START) + 10 days);
        vm.expectRevert(bytes("invalid daily period"));
        rewards.pay(SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START + 1, WINNER, 1, EVIDENCE_HASH, "", "");
    }

    function testRejectsPeriodsBeforeProgramDeploymentWeek() public {
        vm.warp(uint256(DAILY_START) + 10 days);
        vm.expectRevert(bytes("period predates program"));
        rewards.pay(SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START - 1 days, WINNER, 1, EVIDENCE_HASH, "", "");

        vm.expectRevert(bytes("period predates program"));
        rewards.pay(SolverLeaderboardRewards.PeriodKind.Weekly, WEEKLY_START - 7 days, WINNER, 1, EVIDENCE_HASH, "", "");
    }

    function testAcceptsErc1271Signer() public {
        Leaderboard1271Signer contractSigner = new Leaderboard1271Signer();
        SolverLeaderboardRewards smartAccountRewards =
            new SolverLeaderboardRewards(address(token), address(contractSigner), vm.addr(SIGNER_B_KEY));
        token.mint(address(this), 3_000_000);
        token.approve(address(smartAccountRewards), 3_000_000);
        smartAccountRewards.fund(3_000_000);

        bytes memory contractSignature = hex"1234";
        bytes32 digest = smartAccountRewards.awardDigest(
            SolverLeaderboardRewards.PeriodKind.Daily, DAILY_START, WINNER, 1, EVIDENCE_HASH
        );
        contractSigner.approve(digest, contractSignature);
        bytes memory signatureB = _signature(SIGNER_B_KEY, digest);
        vm.warp(uint256(DAILY_START + 1 days) + smartAccountRewards.FINALIZATION_DELAY());

        smartAccountRewards.pay(
            SolverLeaderboardRewards.PeriodKind.Daily,
            DAILY_START,
            WINNER,
            1,
            EVIDENCE_HASH,
            contractSignature,
            signatureB
        );

        assert(token.balanceOf(WINNER) == 3_000_000);
    }

    function _sign(SolverLeaderboardRewards.PeriodKind kind, uint64 startsAt, uint32 completions)
        private
        returns (bytes memory signatureA, bytes memory signatureB)
    {
        bytes32 digest = rewards.awardDigest(kind, startsAt, WINNER, completions, EVIDENCE_HASH);
        return (_signature(SIGNER_A_KEY, digest), _signature(SIGNER_B_KEY, digest));
    }

    function _signature(uint256 key, bytes32 digest) private returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);
        return abi.encodePacked(r, s, v);
    }
}
