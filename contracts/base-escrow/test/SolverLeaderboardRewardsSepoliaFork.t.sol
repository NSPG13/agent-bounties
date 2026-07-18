// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/SolverLeaderboardRewards.sol";

interface VmLeaderboardFork {
    function addr(uint256 privateKey) external returns (address keyAddr);
    function createSelectFork(string calldata urlOrAlias) external returns (uint256 forkId);
    function envAddress(string calldata name) external returns (address value);
    function envOr(string calldata name, bool defaultValue) external returns (bool value);
    function envString(string calldata name) external returns (string memory value);
    function envUint(string calldata name) external returns (uint256 value);
    function load(address target, bytes32 slot) external view returns (bytes32 data);
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
    function skip(bool skipTest) external;
    function store(address target, bytes32 slot, bytes32 value) external;
    function warp(uint256 timestamp) external;
}

interface SepoliaUsdc {
    function approve(address spender, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}

/// @notice Replays both prize amounts against the exact deployed Base Sepolia contract.
/// Token balance setup changes fork storage only. No transaction is broadcast.
contract SolverLeaderboardRewardsSepoliaForkTest {
    VmLeaderboardFork private constant vm = VmLeaderboardFork(address(uint160(uint256(keccak256("hevm cheat code")))));

    address private constant USDC = 0x036CbD53842c5426634e7929541eC2318f3dCF7e;
    address private constant WINNER = 0x000000000000000000000000000000000000bEEF;
    uint256 private constant FUNDING = 29_000_000;

    function testDeployedContractPaysDailyAndWeeklyRewards() public {
        if (!vm.envOr("RUN_LEADERBOARD_SEPOLIA_FORK", false)) {
            vm.skip(true);
            return;
        }
        vm.createSelectFork(vm.envString("BASE_SEPOLIA_RPC_URL"));
        require(block.chainid == 84_532, "wrong chain");

        SolverLeaderboardRewards rewards =
            SolverLeaderboardRewards(vm.envAddress("LEADERBOARD_SEPOLIA_REWARD_CONTRACT"));
        require(address(rewards).code.length > 0, "reward contract missing");
        require(rewards.settlementToken() == USDC, "token drift");

        uint256 signerAKey = vm.envUint("REGRESSION_VERIFIER_ONE_PRIVATE_KEY");
        uint256 signerBKey = vm.envUint("REGRESSION_VERIFIER_TWO_PRIVATE_KEY");
        require(rewards.signerA() == vm.addr(signerAKey), "signer A drift");
        require(rewards.signerB() == vm.addr(signerBKey), "signer B drift");

        SepoliaUsdc usdc = SepoliaUsdc(USDC);
        uint256 winnerBefore = usdc.balanceOf(WINNER);
        _setForkBalance(usdc, FUNDING);
        require(usdc.approve(address(rewards), FUNDING), "approval failed");
        rewards.fund(FUNDING);

        uint64 dailyStart = rewards.firstDailyStart();
        uint64 weeklyStart = rewards.firstWeeklyStart();
        uint256 dailyFinal = uint256(dailyStart) + 1 days + rewards.FINALIZATION_DELAY();
        uint256 weeklyFinal = uint256(weeklyStart) + 7 days + rewards.FINALIZATION_DELAY();
        vm.warp(dailyFinal > weeklyFinal ? dailyFinal : weeklyFinal);

        _pay(rewards, SolverLeaderboardRewards.PeriodKind.Daily, dailyStart, 3, signerAKey, signerBKey);
        _pay(rewards, SolverLeaderboardRewards.PeriodKind.Weekly, weeklyStart, 8, signerAKey, signerBKey);

        require(usdc.balanceOf(WINNER) == winnerBefore + FUNDING, "winner did not receive 29 USDC");
        require(usdc.balanceOf(address(rewards)) == 0, "reward pool did not settle exactly");
        require(
            rewards.paidAwardWinner(rewards.awardId(SolverLeaderboardRewards.PeriodKind.Daily, dailyStart)) == WINNER,
            "daily winner missing"
        );
        require(
            rewards.paidAwardWinner(rewards.awardId(SolverLeaderboardRewards.PeriodKind.Weekly, weeklyStart)) == WINNER,
            "weekly winner missing"
        );
    }

    function _pay(
        SolverLeaderboardRewards rewards,
        SolverLeaderboardRewards.PeriodKind kind,
        uint64 startsAt,
        uint32 completions,
        uint256 signerAKey,
        uint256 signerBKey
    ) private {
        bytes32 evidenceHash = keccak256(abi.encode("base-sepolia-rehearsal", kind, startsAt));
        bytes32 digest = rewards.awardDigest(kind, startsAt, WINNER, completions, evidenceHash);
        (uint8 vA, bytes32 rA, bytes32 sA) = vm.sign(signerAKey, digest);
        (uint8 vB, bytes32 rB, bytes32 sB) = vm.sign(signerBKey, digest);
        require(ecrecover(digest, vA, rA, sA) == rewards.signerA(), "signature A mismatch");
        require(ecrecover(digest, vB, rB, sB) == rewards.signerB(), "signature B mismatch");
        rewards.pay(
            kind,
            startsAt,
            WINNER,
            completions,
            evidenceHash,
            abi.encodePacked(rA, sA, vA),
            abi.encodePacked(rB, sB, vB)
        );
    }

    function _setForkBalance(SepoliaUsdc usdc, uint256 amount) private {
        for (uint256 slot; slot < 128; slot++) {
            bytes32 location = keccak256(abi.encode(address(this), slot));
            bytes32 previous = vm.load(USDC, location);
            vm.store(USDC, location, bytes32(amount));
            if (usdc.balanceOf(address(this)) == amount) return;
            vm.store(USDC, location, previous);
        }
        revert("USDC balance slot not found");
    }
}
