// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/SolverLeaderboardRewards.sol";

interface LeaderboardDeployVm {
    function addr(uint256 privateKey) external returns (address);
    function envAddress(string calldata name) external returns (address);
    function envString(string calldata name) external returns (string memory);
    function envUint(string calldata name) external returns (uint256);
    function startBroadcast(uint256 privateKey) external;
    function stopBroadcast() external;
    function serializeAddress(string calldata objectKey, string calldata valueKey, address value)
        external
        returns (string memory);
    function serializeUint(string calldata objectKey, string calldata valueKey, uint256 value)
        external
        returns (string memory);
    function writeJson(string calldata json, string calldata path) external;
}

contract DeploySolverLeaderboardRewards {
    LeaderboardDeployVm private constant vm =
        LeaderboardDeployVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    address private constant BASE_MAINNET_USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
    address private constant BASE_SEPOLIA_USDC = 0x036CbD53842c5426634e7929541eC2318f3dCF7e;

    function run() external {
        require(block.chainid == 8_453 || block.chainid == 84_532, "unsupported chain");
        address token = block.chainid == 8_453 ? BASE_MAINNET_USDC : BASE_SEPOLIA_USDC;
        address signerA = vm.envAddress("LEADERBOARD_SIGNER_A");
        address signerB = vm.envAddress("LEADERBOARD_SIGNER_B");
        require(signerA != address(0) && signerB != address(0) && signerA != signerB, "invalid signers");

        uint256 deployerKey = vm.envUint("BASE_KEEPER_PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        require(deployer.balance >= 100_000_000_000_000, "deployment gas reserve too low");

        vm.startBroadcast(deployerKey);
        SolverLeaderboardRewards rewards = new SolverLeaderboardRewards(token, signerA, signerB);
        vm.stopBroadcast();

        require(rewards.settlementToken() == token, "token drift");
        require(rewards.signerA() == signerA && rewards.signerB() == signerB, "signer drift");
        require(rewards.DAILY_REWARD() == 3_000_000, "daily reward drift");
        require(rewards.WEEKLY_REWARD() == 26_000_000, "weekly reward drift");
        require(rewards.firstDailyStart() % 1 days == 0, "daily start drift");
        require(
            rewards.firstWeeklyStart() >= 4 days && (rewards.firstWeeklyStart() - 4 days) % 7 days == 0,
            "weekly start drift"
        );

        string memory objectKey = "solver-leaderboard-rewards";
        vm.serializeUint(objectKey, "chain_id", block.chainid);
        vm.serializeAddress(objectKey, "deployer", deployer);
        vm.serializeAddress(objectKey, "settlement_token", token);
        vm.serializeAddress(objectKey, "reward_contract", address(rewards));
        vm.serializeAddress(objectKey, "signer_a", signerA);
        vm.serializeAddress(objectKey, "signer_b", signerB);
        vm.serializeUint(objectKey, "first_daily_start", rewards.firstDailyStart());
        string memory json = vm.serializeUint(objectKey, "first_weekly_start", rewards.firstWeeklyStart());
        vm.writeJson(json, vm.envString("LEADERBOARD_DEPLOYMENT_OUTPUT"));
    }
}
