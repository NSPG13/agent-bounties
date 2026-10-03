// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactoryV2.sol";

interface V2ForkVm {
    function createSelectFork(string calldata urlOrAlias, uint256 blockNumber) external returns (uint256 forkId);
    function envOr(string calldata name, bool defaultValue) external returns (bool value);
    function envString(string calldata name) external returns (string memory value);
    function prank(address sender) external;
    function addr(uint256 privateKey) external returns (address keyAddr);
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
    function skip(bool skipTest) external;
}

interface V2ForkUsdc {
    function approve(address spender, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
    function transfer(address recipient, uint256 amount) external returns (bool);
    function DOMAIN_SEPARATOR() external view returns (bytes32);
    function blacklister() external view returns (address);
    function blacklist(address account) external;
    function unBlacklist(address account) external;
    function isBlacklisted(address account) external view returns (bool);
}

contract V2ForkVerdictModule is IAgentBountyVerifier {
    function verify(bytes32, uint64, address, bytes32, bytes32, bytes32, bytes calldata proof)
        external
        pure
        returns (bool passed, bytes32 responseHash)
    {
        passed = proof.length > 0 && proof[0] == 0x01;
        responseHash = keccak256(proof);
    }
}

/// @notice Opt-in integration test of autonomous-v2 against real Base mainnet USDC on a fork.
/// Set RUN_MAINNET_FORK=true and BASE_MAINNET_RPC_URL to execute it. No transaction is broadcast.
contract AgentBountyV2MainnetForkTest {
    V2ForkVm private constant vm = V2ForkVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    uint256 private constant FORK_BLOCK = 52_129_622;
    address private constant USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
    /// @dev Launch fee recipient and funding source; holds 2.900015 USDC at FORK_BLOCK.
    address private constant OPERATOR_WALLET = 0x884834E884d6e93462655A2820140aD03E6747bC;
    address private constant ISOLATED_FEE_RECIPIENT = 0xfEE0000000000000000000000000000000000fee;
    address private constant VERIFIER_RECIPIENT = 0x7000000000000000000000000000000000000007;
    address private constant RELAYER = 0x3000000000000000000000000000000000000003;
    uint256 private constant POSTER_KEY = 0xB0B0B0;
    uint256 private constant SOLVER_KEY = 0x501E501E;

    uint16 private constant FEE_BPS = 750;
    uint256 private constant SOLVER_REWARD = 1_000_000;
    uint256 private constant VERIFIER_REWARD = 100_000;
    uint256 private constant FEE = 75_000;
    uint256 private constant TARGET = 1_175_000;

    bytes32 private constant TRANSFER_WITH_AUTHORIZATION_TYPEHASH = keccak256(
        "TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)"
    );

    V2ForkUsdc private usdc;
    AgentBountyFactoryV2 private factory;
    V2ForkVerdictModule private module;

    function testRealUsdcAuthorizedLoopPaysLaunchFeeRecipientAtPayout() public {
        if (!_selectFork()) return;
        factory = new AgentBountyFactoryV2(USDC, FEE_BPS, OPERATOR_WALLET);
        (uint256 quotedFee, uint256 quotedTarget) = factory.quote(SOLVER_REWARD, VERIFIER_REWARD);
        require(quotedFee == FEE && quotedTarget == TARGET, "quote mismatch");

        address poster = vm.addr(POSTER_KEY);
        address solver = vm.addr(SOLVER_KEY);
        vm.prank(OPERATOR_WALLET);
        require(usdc.transfer(poster, TARGET), "poster funding transfer");
        vm.prank(OPERATOR_WALLET);
        require(usdc.transfer(solver, VERIFIER_REWARD), "solver bond transfer");
        uint256 operatorBefore = usdc.balanceOf(OPERATOR_WALLET);

        AgentBountyV2 bounty = _createWithRealAuthorization(poster);
        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Claimable, "not claimable");
        require(usdc.balanceOf(address(bounty)) == TARGET, "escrow must hold reward, verifier reward and fee");

        _claimWithRealAuthorization(bounty, solver);
        vm.prank(solver);
        bounty.submit(keccak256("v2-fork-artifact"), keccak256("v2-fork-evidence"));
        vm.prank(RELAYER);
        bounty.verifyAndSettle(hex"01");

        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Settled, "not settled");
        require(usdc.balanceOf(solver) == SOLVER_REWARD + VERIFIER_REWARD, "solver reward plus returned bond");
        require(usdc.balanceOf(VERIFIER_RECIPIENT) == VERIFIER_REWARD, "verifier reward");
        require(usdc.balanceOf(OPERATOR_WALLET) == operatorBefore + FEE, "launch recipient paid at payout");
        require(bounty.platformFeeAccrued() == 0, "fee must not be deferred");
        require(usdc.balanceOf(address(bounty)) == 0, "escrow retained USDC");
    }

    function testRealUsdcBlacklistedRecipientDefersFeeWithoutBlockingSolver() public {
        if (!_selectFork()) return;
        factory = new AgentBountyFactoryV2(USDC, FEE_BPS, ISOLATED_FEE_RECIPIENT);
        address solver = vm.addr(SOLVER_KEY);
        vm.prank(OPERATOR_WALLET);
        require(usdc.transfer(solver, VERIFIER_REWARD), "solver bond transfer");

        vm.prank(OPERATOR_WALLET);
        usdc.approve(address(factory), TARGET);
        vm.prank(OPERATOR_WALLET);
        (address bountyAddress,) = factory.createBounty(_params(), new address[](0), TARGET, keccak256("blacklist"));
        AgentBountyV2 bounty = AgentBountyV2(bountyAddress);

        address blacklister = usdc.blacklister();
        vm.prank(blacklister);
        usdc.blacklist(ISOLATED_FEE_RECIPIENT);
        require(usdc.isBlacklisted(ISOLATED_FEE_RECIPIENT), "recipient not blacklisted");

        vm.prank(solver);
        usdc.approve(address(bounty), VERIFIER_REWARD);
        vm.prank(solver);
        bounty.claim();
        vm.prank(solver);
        bounty.submit(keccak256("v2-fork-artifact"), keccak256("v2-fork-evidence"));
        vm.prank(RELAYER);
        bounty.verifyAndSettle(hex"01");

        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Settled, "blacklist blocked settlement");
        require(usdc.balanceOf(solver) == SOLVER_REWARD + VERIFIER_REWARD, "solver unpaid");
        require(usdc.balanceOf(VERIFIER_RECIPIENT) == VERIFIER_REWARD, "verifier unpaid");
        require(bounty.platformFeeAccrued() == FEE, "fee not deferred");
        require(usdc.balanceOf(address(bounty)) == FEE, "deferred fee not escrowed");

        (bool blockedForward,) = address(bountyAddress).call(abi.encodeCall(AgentBountyV2.withdrawPlatformFee, ()));
        require(!blockedForward, "forward to blacklisted recipient succeeded");

        vm.prank(blacklister);
        usdc.unBlacklist(ISOLATED_FEE_RECIPIENT);
        vm.prank(RELAYER);
        require(bounty.withdrawPlatformFee() == FEE, "forward amount");
        require(usdc.balanceOf(ISOLATED_FEE_RECIPIENT) == FEE, "recipient paid after unblacklist");
        require(usdc.balanceOf(address(bounty)) == 0, "escrow retained USDC");
    }

    function _selectFork() private returns (bool) {
        if (!vm.envOr("RUN_MAINNET_FORK", false)) {
            vm.skip(true);
            return false;
        }
        vm.createSelectFork(vm.envString("BASE_MAINNET_RPC_URL"), FORK_BLOCK);
        require(block.chainid == 8453, "wrong chain");
        usdc = V2ForkUsdc(USDC);
        module = new V2ForkVerdictModule();
        require(usdc.balanceOf(OPERATOR_WALLET) >= 2 * TARGET, "fork funding source drift");
        return true;
    }

    function _createWithRealAuthorization(address poster) private returns (AgentBountyV2) {
        AgentBountyFactoryV2.CreateBountyParams memory params = _params();
        bytes32 creationNonce = keccak256("v2-fork-create");
        address predicted = factory.predictBountyAddress(poster, params, new address[](0), creationNonce);
        AgentBountyFactoryV2.FundingAuthorization memory authorization =
            _authorize(POSTER_KEY, poster, predicted, TARGET, keccak256("v2-fork-create-auth"));
        vm.prank(RELAYER);
        (address bountyAddress,) = factory.createBountyWithAuthorization(
            poster, params, new address[](0), TARGET, creationNonce, authorization
        );
        require(bountyAddress == predicted, "predicted address mismatch");
        return AgentBountyV2(bountyAddress);
    }

    function _claimWithRealAuthorization(AgentBountyV2 bounty, address solver) private {
        AgentBountyFactoryV2.FundingAuthorization memory bond =
            _authorize(SOLVER_KEY, solver, address(bounty), VERIFIER_REWARD, keccak256("v2-fork-bond-auth"));
        vm.prank(RELAYER);
        bounty.claimWithAuthorization(solver, bond.validAfter, bond.validBefore, bond.nonce, bond.v, bond.r, bond.s);
        require(bounty.solver() == solver, "claim not recorded");
    }

    function _authorize(uint256 key, address from, address to, uint256 value, bytes32 nonce)
        private
        returns (AgentBountyFactoryV2.FundingAuthorization memory authorization)
    {
        authorization.validAfter = 0;
        authorization.validBefore = block.timestamp + 1 hours;
        authorization.nonce = nonce;
        bytes32 structHash = keccak256(
            abi.encode(
                TRANSFER_WITH_AUTHORIZATION_TYPEHASH,
                from,
                to,
                value,
                authorization.validAfter,
                authorization.validBefore,
                nonce
            )
        );
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", usdc.DOMAIN_SEPARATOR(), structHash));
        (authorization.v, authorization.r, authorization.s) = vm.sign(key, digest);
    }

    function _params() private view returns (AgentBountyFactoryV2.CreateBountyParams memory) {
        return AgentBountyFactoryV2.CreateBountyParams({
            solverReward: SOLVER_REWARD,
            verifierReward: VERIFIER_REWARD,
            termsHash: keccak256("v2-fork-terms"),
            policyHash: keccak256("v2-fork-policy"),
            acceptanceCriteriaHash: keccak256("v2-fork-criteria"),
            benchmarkHash: keccak256("v2-fork-benchmark"),
            evidenceSchemaHash: keccak256("v2-fork-evidence-schema"),
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
}
