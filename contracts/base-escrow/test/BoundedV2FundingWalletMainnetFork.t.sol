// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactoryV2.sol";
import "../src/BoundedV2FundingWallet.sol";

interface FundingWalletForkVm {
    function createSelectFork(string calldata urlOrAlias, uint256 blockNumber) external returns (uint256 forkId);
    function envOr(string calldata name, bool defaultValue) external returns (bool value);
    function envString(string calldata name) external returns (string memory value);
    function prank(address sender) external;
    function skip(bool skipTest) external;
    function store(address target, bytes32 slot, bytes32 value) external;
}

interface FundingWalletForkUsdc {
    function approve(address spender, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}

contract FundingWalletForkVerdictModule is IAgentBountyVerifier {
    function verify(bytes32, uint64, address, bytes32, bytes32, bytes32, bytes calldata proof)
        external
        pure
        returns (bool passed, bytes32 responseHash)
    {
        passed = proof.length > 0 && proof[0] == 0x01;
        responseHash = keccak256(proof);
    }
}

/// @notice Opt-in check of the funding wallet against the deployed autonomous-v2 factory and real
/// Base USDC. Set RUN_MAINNET_FORK=true and BASE_MAINNET_RPC_URL. No transaction is broadcast.
contract BoundedV2FundingWalletMainnetForkTest {
    FundingWalletForkVm private constant vm =
        FundingWalletForkVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    uint256 private constant FORK_BLOCK = 52_900_000;
    address private constant USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
    address private constant V2_FACTORY = 0xC33e2Ae33BB9580837Ea59df18E57fA1039aE58A;
    address private constant V2_FEE_RECIPIENT = 0x884834E884d6e93462655A2820140aD03E6747bC;
    bytes32 private constant V2_FACTORY_RUNTIME_HASH =
        0x3a706b2753ee4c6c05379cb17a3db21f5a7eb5aca6b207166611fed8baf1dc57;
    /// @dev FiatTokenV2_2 stores balances in `balanceAndBlacklistStates` at slot 9.
    uint256 private constant USDC_BALANCE_SLOT = 9;

    address private constant OWNER = 0x0a11CE0000000000000000000000000000000001;
    address private constant DELEGATE = 0xD0d0000000000000000000000000000000000002;
    address private constant SOLVER = 0x5011000000000000000000000000000000000003;
    address private constant VERIFIER_RECIPIENT = 0x7000000000000000000000000000000000000007;
    uint256 private constant SOLVER_REWARD = 1_000_000;
    uint256 private constant VERIFIER_REWARD = 100_000;
    uint256 private constant TARGET = 1_175_000;
    uint256 private constant FLOAT = 5_000_000;

    function testFundingWalletCreatesSettlesAndRecoversOnTheDeployedFactory() public {
        if (!vm.envOr("RUN_MAINNET_FORK", false)) {
            vm.skip(true);
            return;
        }
        vm.createSelectFork(vm.envString("BASE_MAINNET_RPC_URL"), FORK_BLOCK);
        require(V2_FACTORY.codehash == V2_FACTORY_RUNTIME_HASH, "factory runtime changed");
        AgentBountyFactoryV2 factory = AgentBountyFactoryV2(V2_FACTORY);
        require(factory.settlementToken() == USDC, "factory token mismatch");
        require(factory.platformFeeBps() == 750 && factory.platformFeeRecipient() == V2_FEE_RECIPIENT, "fee drift");

        FundingWalletForkVerdictModule module = new FundingWalletForkVerdictModule();
        BoundedV2FundingWallet wallet =
            new BoundedV2FundingWallet(V2_FACTORY, OWNER, _policy(), _configs(address(module)));
        _setUsdcBalance(address(wallet), FLOAT);
        FundingWalletForkUsdc usdc = FundingWalletForkUsdc(USDC);
        uint256 feeRecipientBefore = usdc.balanceOf(V2_FEE_RECIPIENT);

        vm.prank(DELEGATE);
        (address settledAddress,, uint256 target) =
            wallet.createBounty(_params(address(module)), new address[](0), bytes32(uint256(1)));
        require(target == TARGET, "target mismatch");
        require(factory.isCanonicalBounty(settledAddress), "bounty not canonical");
        AgentBountyV2 settled = AgentBountyV2(settledAddress);
        require(settled.bountyStatus() == AgentBountyV2.BountyStatus.Claimable, "not claimable");
        require(usdc.balanceOf(address(wallet)) == FLOAT - TARGET, "wallet debit mismatch");

        _setUsdcBalance(SOLVER, VERIFIER_REWARD);
        vm.prank(SOLVER);
        usdc.approve(settledAddress, VERIFIER_REWARD);
        vm.prank(SOLVER);
        settled.claim();
        vm.prank(SOLVER);
        settled.submit(keccak256("artifact"), keccak256("evidence"));
        settled.verifyAndSettle(hex"01");

        require(settled.bountyStatus() == AgentBountyV2.BountyStatus.Settled, "not settled");
        require(usdc.balanceOf(SOLVER) == SOLVER_REWARD + VERIFIER_REWARD, "solver payout mismatch");
        require(usdc.balanceOf(VERIFIER_RECIPIENT) == VERIFIER_REWARD, "verifier payout mismatch");
        require(usdc.balanceOf(V2_FEE_RECIPIENT) == feeRecipientBefore + 75_000, "fee payout mismatch");

        vm.prank(DELEGATE);
        (address cancelledAddress,,) =
            wallet.createBounty(_params(address(module)), new address[](0), bytes32(uint256(2)));
        vm.prank(DELEGATE);
        require(wallet.cancelBounty(cancelledAddress) == TARGET, "cancel recovery mismatch");
        require(usdc.balanceOf(address(wallet)) == FLOAT - TARGET, "cancelled target not recovered");
        require(wallet.lifetimeSpent() == 2 * TARGET, "recovery must not restore authority");
    }

    function _setUsdcBalance(address account, uint256 amount) private {
        vm.store(USDC, keccak256(abi.encode(account, USDC_BALANCE_SLOT)), bytes32(amount));
        require(FundingWalletForkUsdc(USDC).balanceOf(account) == amount, "USDC balance slot changed");
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
            maxPerBounty: TARGET,
            maxPerPeriod: 2 * TARGET,
            maxLifetimeSpend: 4 * TARGET,
            claimEligibilityRegistry: address(0),
            claimEligibilitySource: bytes32(0)
        });
    }

    function _configs(address module)
        private
        pure
        returns (BoundedV2FundingWallet.VerificationConfig[] memory configs)
    {
        configs = new BoundedV2FundingWallet.VerificationConfig[](1);
        configs[0] = BoundedV2FundingWallet.VerificationConfig({
            mode: AgentBountyV2.VerificationMode.DeterministicModule,
            verifierModule: module,
            verifierRewardRecipient: VERIFIER_RECIPIENT,
            verifierSetHash: bytes32(0),
            threshold: 1
        });
    }

    function _params(address module) private view returns (AgentBountyFactoryV2.CreateBountyParams memory) {
        return AgentBountyFactoryV2.CreateBountyParams({
            solverReward: SOLVER_REWARD,
            verifierReward: VERIFIER_REWARD,
            termsHash: keccak256("fork-terms"),
            policyHash: keccak256("fork-policy"),
            acceptanceCriteriaHash: keccak256("fork-criteria"),
            benchmarkHash: keccak256("fork-benchmark"),
            evidenceSchemaHash: keccak256("fork-evidence-schema"),
            fundingDeadline: uint64(block.timestamp + 1 days),
            claimWindowSeconds: 1 days,
            verificationWindowSeconds: 1 days,
            verificationMode: AgentBountyV2.VerificationMode.DeterministicModule,
            verifierModule: module,
            verifierRewardRecipient: VERIFIER_RECIPIENT,
            threshold: 1,
            claimEligibilityRegistry: address(0),
            claimEligibilitySource: bytes32(0)
        });
    }
}
