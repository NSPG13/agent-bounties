// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactory.sol";
import "../src/BoundedAgentWalletFactory.sol";
import "../src/CanonicalChildBountyVerifier.sol";
import "../src/LeadingZeroWorkVerifier.sol";

interface VmFork {
    function createSelectFork(string calldata urlOrAlias) external returns (uint256 forkId);
    function createSelectFork(string calldata urlOrAlias, uint256 blockNumber) external returns (uint256 forkId);
    function envOr(string calldata name, bool defaultValue) external returns (bool value);
    function envString(string calldata name) external returns (string memory value);
    function prank(address sender) external;
    function addr(uint256 privateKey) external returns (address keyAddr);
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
    function skip(bool skipTest) external;
}

interface MainnetUsdc {
    function approve(address spender, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
    function transfer(address recipient, uint256 amount) external returns (bool);
}

/// @notice Opt-in integration test against the actual Base mainnet contracts on an Anvil fork.
/// Set RUN_MAINNET_FORK=true and BASE_MAINNET_RPC_URL to execute it. No transaction is broadcast.
contract AgentBountyMainnetForkTest {
    VmFork private constant vm = VmFork(address(uint160(uint256(keccak256("hevm cheat code")))));

    address private constant FACTORY = 0x082C52131aaF0C56e76b075f895EAB6fcaB6d2F9;
    address private constant IMPLEMENTATION = 0x2fa36D2b2327642db3a6Cc8CDD91544ad7484EB9;
    address private constant USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
    address private constant MODULE = 0xcc6059cEedA5bc4ba8a97ecFbFFa7488C8FD579E;
    address private constant FUNDING_SOURCE = 0x884834E884d6e93462655A2820140aD03E6747bC;
    address private constant BOND_SOURCE = 0x884834E884d6e93462655A2820140aD03E6747bC;
    address private constant CREATOR = 0x1000000000000000000000000000000000000001;
    address private constant SOLVER = 0x2000000000000000000000000000000000000002;
    address private constant RELAYER = 0x3000000000000000000000000000000000000003;
    address private constant CHILD_SOLVER = 0x4000000000000000000000000000000000000004;
    address private constant POOL_CONTRIBUTOR = 0x5000000000000000000000000000000000000005;
    uint256 private constant MAINNET_FORK_BLOCK = 48_567_240;

    bytes32 private constant FACTORY_RUNTIME_HASH = 0x06f810de7b46f854ecc29e9c0c28156edab4b0d3e0bbe2bf5be8876687bebfc6;
    bytes32 private constant IMPLEMENTATION_RUNTIME_HASH =
        0xc36fcba5176b2cd8b57a9fd0cbf931177dc8b36cf8367c1568ccebe5f03be3f6;
    bytes32 private constant MODULE_RUNTIME_HASH = 0xbaa3a8305c4b65d0dc20131d0ef207fdaf4763f345393a831370cd04077df9b3;

    bytes32 private constant TERMS_HASH = 0xd970470bb6adaaa98139740db3362dfd40b00110561dda9c1c7a9e1659ab915e;
    bytes32 private constant POLICY_HASH = 0xb9aaf9ae7bc757288fc72119f1230a82a4c592519cddeb69bea4ab7790e40dc7;
    bytes32 private constant CRITERIA_HASH = 0x8788ae8d44013e65172f494c054361906cc4974eda69f0e184211c7032d8bc78;
    bytes32 private constant BENCHMARK_HASH = 0xf561db43183cd283b1ed059eeb64f6524ff92de33001616882d31071e2086d2c;
    bytes32 private constant EVIDENCE_SCHEMA_HASH = 0x21521c3eac6143dc56fd2c8d1aaf9831057d6c40c18b542fea77102a6c6d2244;
    bytes32 private constant SUBMISSION_HASH = keccak256("mainnet-fork-artifact");
    bytes32 private constant EVIDENCE_HASH = keccak256("mainnet-fork-evidence");
    bytes32 private constant EIP712_DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant TRANSFER_WITH_AUTHORIZATION_TYPEHASH = keccak256(
        "TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)"
    );
    uint256 private constant GAS_RELAY_CREATOR_KEY =
        uint256(keccak256("agent-bounties/mainnet-fork/gas-relay/creator-key"));
    uint256 private constant GAS_RELAY_SOLVER_KEY =
        uint256(keccak256("agent-bounties/mainnet-fork/gas-relay/solver-key"));
    uint256 private constant BOUNDED_CREATOR_DELEGATE_KEY =
        uint256(keccak256("agent-bounties/mainnet-fork/bounded-wallet/creator-delegate"));
    uint256 private constant BOUNDED_SOLVER_DELEGATE_KEY =
        uint256(keccak256("agent-bounties/mainnet-fork/bounded-wallet/solver-delegate"));
    uint256 private constant BOUNDED_OWNER_KEY =
        uint256(keccak256("agent-bounties/mainnet-fork/bounded-wallet/owner"));

    struct RelayLoopContext {
        AgentBounty bounty;
        bytes32 bountyId;
        address creator;
        address solver;
        uint256 creatorBefore;
        uint256 solverBefore;
    }

    function testCanonicalMainnetSettledChildLoop() public {
        if (!vm.envOr("RUN_MAINNET_FORK", false)) {
            vm.skip(true);
            return;
        }
        _selectMainnetFork();

        require(block.chainid == 8453, "wrong chain");
        require(keccak256(FACTORY.code) == FACTORY_RUNTIME_HASH, "factory code drift");
        require(keccak256(IMPLEMENTATION.code) == IMPLEMENTATION_RUNTIME_HASH, "implementation code drift");

        AgentBountyFactory factory = AgentBountyFactory(FACTORY);
        MainnetUsdc usdc = MainnetUsdc(USDC);
        CanonicalChildBountyVerifier childModule = new CanonicalChildBountyVerifier(FACTORY);
        LeadingZeroWorkVerifier workModule = LeadingZeroWorkVerifier(MODULE);
        require(childModule.settlementToken() == USDC, "child module token drift");
        require(keccak256(MODULE.code) == MODULE_RUNTIME_HASH, "work module code drift");

        vm.prank(FUNDING_SOURCE);
        require(usdc.transfer(CREATOR, 1_000_000), "root funding transfer failed");
        vm.prank(FUNDING_SOURCE);
        require(usdc.transfer(SOLVER, 500_000), "parent solver funding transfer failed");
        vm.prank(FUNDING_SOURCE);
        require(usdc.transfer(POOL_CONTRIBUTOR, 500_000), "pooled funding transfer failed");
        vm.prank(FUNDING_SOURCE);
        require(usdc.transfer(CHILD_SOLVER, 100_000), "child bond transfer failed");

        AgentBountyFactory.CreateBountyParams memory rootParams = _childLoopParams(
            childModule,
            childModule.ACCEPTANCE_CRITERIA_HASH(),
            keccak256("mainnet-fork-child-loop-root"),
            900_000,
            100_000
        );
        vm.prank(CREATOR);
        require(usdc.approve(FACTORY, 1_000_000), "root approval failed");
        vm.prank(CREATOR);
        (address rootAddress,) = factory.createBounty(
            rootParams, new address[](0), 1_000_000, keccak256("mainnet-fork-child-loop-root-nonce")
        );
        AgentBounty root = AgentBounty(rootAddress);

        vm.prank(SOLVER);
        require(usdc.approve(rootAddress, 100_000), "parent bond approval failed");
        vm.prank(SOLVER);
        root.claim();
        vm.prank(SOLVER);
        root.submit(keccak256("parent-loop-submission"), keccak256("parent-loop-evidence"));

        AgentBountyFactory.CreateBountyParams memory childParams = _childLoopParams(
            workModule,
            keccak256("mainnet-fork-child-work-criteria"),
            childModule.expectedBenchmarkHash(root.bountyId(), root.round()),
            800_000,
            100_000
        );
        vm.prank(SOLVER);
        require(usdc.approve(FACTORY, 400_000), "child initial approval failed");
        vm.prank(SOLVER);
        (address childAddress,) = factory.createBounty(
            childParams, new address[](0), 400_000, keccak256("mainnet-fork-child-loop-child-nonce")
        );
        AgentBounty child = AgentBounty(childAddress);

        vm.prank(POOL_CONTRIBUTOR);
        require(usdc.approve(childAddress, 500_000), "pool approval failed");
        vm.prank(POOL_CONTRIBUTOR);
        child.fund(500_000);
        vm.prank(CHILD_SOLVER);
        require(usdc.approve(childAddress, 100_000), "child bond approval failed");
        vm.prank(CHILD_SOLVER);
        child.claim();
        vm.prank(CHILD_SOLVER);
        child.submit(SUBMISSION_HASH, EVIDENCE_HASH);

        uint256 childNonce = _mineLeadingZeroNonce(child.bountyId(), child.round(), CHILD_SOLVER);
        vm.prank(RELAYER);
        child.verifyAndSettle(abi.encode(childNonce));

        vm.prank(RELAYER);
        root.verifyAndSettle(abi.encode(childAddress));

        require(root.bountyStatus() == AgentBounty.BountyStatus.Settled, "root not settled");
        require(child.bountyStatus() == AgentBounty.BountyStatus.Settled, "child not settled");
        require(usdc.balanceOf(rootAddress) == 0, "root retained funds");
        require(usdc.balanceOf(childAddress) == 0, "child retained funds");
    }

    function testCanonicalMainnetPermissionlessPaidLoop() public {
        if (!vm.envOr("RUN_MAINNET_FORK", false)) {
            vm.skip(true);
            return;
        }
        _selectMainnetFork();

        require(block.chainid == 8453, "wrong chain");
        require(keccak256(FACTORY.code) == FACTORY_RUNTIME_HASH, "factory code drift");
        require(keccak256(IMPLEMENTATION.code) == IMPLEMENTATION_RUNTIME_HASH, "implementation code drift");
        require(keccak256(MODULE.code) == MODULE_RUNTIME_HASH, "module code drift");

        AgentBountyFactory factory = AgentBountyFactory(FACTORY);
        LeadingZeroWorkVerifier module = LeadingZeroWorkVerifier(MODULE);
        MainnetUsdc usdc = MainnetUsdc(USDC);
        require(factory.settlementToken() == USDC, "factory token drift");
        require(factory.implementation() == IMPLEMENTATION, "factory implementation drift");
        require(module.difficultyBits() == 16, "module difficulty drift");

        uint256 creatorBefore = usdc.balanceOf(CREATOR);
        uint256 solverBefore = usdc.balanceOf(SOLVER);
        uint256 recipientBefore = usdc.balanceOf(CREATOR);
        vm.prank(FUNDING_SOURCE);
        require(usdc.transfer(CREATOR, 1_000_000), "fork funding source transfer failed");
        vm.prank(BOND_SOURCE);
        require(usdc.transfer(SOLVER, 100_000), "fork bond source transfer failed");

        AgentBountyFactory.CreateBountyParams memory params = AgentBountyFactory.CreateBountyParams({
            solverReward: 900_000,
            verifierReward: 100_000,
            termsHash: TERMS_HASH,
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: CRITERIA_HASH,
            benchmarkHash: BENCHMARK_HASH,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH,
            fundingDeadline: uint64(block.timestamp + 30 days),
            claimWindowSeconds: 1 days,
            verificationWindowSeconds: 2 hours,
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: MODULE,
            verifierRewardRecipient: CREATOR,
            threshold: 1
        });
        bytes32 creationNonce = keccak256("agent-bounties/mainnet-fork/module-loop/v1");
        vm.prank(CREATOR);
        require(usdc.approve(FACTORY, 1_000_000), "creator approval failed");
        vm.prank(CREATOR);
        (address bountyAddress, bytes32 bountyId) =
            factory.createBounty(params, new address[](0), 1_000_000, creationNonce);
        AgentBounty bounty = AgentBounty(bountyAddress);

        require(factory.isCanonicalBounty(bountyAddress), "bounty not canonical");
        require(bounty.bountyId() == bountyId, "bounty id mismatch");
        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Claimable, "not claimable");
        require(usdc.balanceOf(bountyAddress) == 1_000_000, "initial funding missing");

        vm.prank(SOLVER);
        require(usdc.approve(bountyAddress, 100_000), "solver bond approval failed");
        vm.prank(SOLVER);
        bounty.claim();
        vm.prank(SOLVER);
        bounty.submit(SUBMISSION_HASH, EVIDENCE_HASH);

        uint256 nonce = _mineLeadingZeroNonce(bountyId, bounty.round(), SOLVER);

        vm.prank(RELAYER);
        bounty.verifyAndSettle(abi.encode(nonce));

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Settled, "not settled");
        require(usdc.balanceOf(bountyAddress) == 0, "settled balance remains");
        require(usdc.balanceOf(SOLVER) == solverBefore + 1_000_000, "solver payout mismatch");
        require(usdc.balanceOf(CREATOR) == recipientBefore + 100_000, "verifier reward mismatch");
        require(creatorBefore == recipientBefore, "creator baseline drift");
    }

    function testCanonicalMainnetGasRelayedPaidLoop() public {
        if (!vm.envOr("RUN_MAINNET_FORK", false)) {
            vm.skip(true);
            return;
        }
        _selectMainnetFork();

        require(block.chainid == 8453, "wrong chain");
        require(keccak256(FACTORY.code) == FACTORY_RUNTIME_HASH, "factory code drift");
        require(keccak256(IMPLEMENTATION.code) == IMPLEMENTATION_RUNTIME_HASH, "implementation code drift");
        require(keccak256(MODULE.code) == MODULE_RUNTIME_HASH, "module code drift");

        RelayLoopContext memory context = _createRelayLoopBounty();
        _relayClaim(context.bounty, context.solver);
        _relaySubmission(context.bounty, context.solver);
        _relaySettlement(context.bounty, context.bountyId, context.solver);

        MainnetUsdc usdc = MainnetUsdc(USDC);
        require(context.bounty.bountyStatus() == AgentBounty.BountyStatus.Settled, "relayed loop not settled");
        require(usdc.balanceOf(address(context.bounty)) == 0, "settled balance remains");
        require(usdc.balanceOf(context.solver) == context.solverBefore + 1_000_000, "relayed solver payout mismatch");
        require(usdc.balanceOf(context.creator) == context.creatorBefore + 100_000, "relayed verifier payout mismatch");
    }

    function testBoundedWalletMainnetForkAutonomousPaidLoop() public {
        if (!vm.envOr("RUN_MAINNET_FORK", false)) {
            vm.skip(true);
            return;
        }
        _selectMainnetFork();

        MainnetUsdc usdc = MainnetUsdc(USDC);
        (BoundedAgentWallet creatorWallet, BoundedAgentWallet solverWallet) = _deployBoundedWallets(usdc);
        AgentBounty bounty = _createBoundedBounty(creatorWallet);
        _relayBoundedAction(
            solverWallet, BoundedAgentWallet.Action.Claim, abi.encode(address(bounty)), BOUNDED_SOLVER_DELEGATE_KEY
        );
        _relayBoundedAction(
            solverWallet,
            BoundedAgentWallet.Action.Submit,
            abi.encode(address(bounty), SUBMISSION_HASH, EVIDENCE_HASH),
            BOUNDED_SOLVER_DELEGATE_KEY
        );
        uint256 nonce = _mineLeadingZeroNonce(bounty.bountyId(), bounty.round(), address(solverWallet));
        uint256 relayerBefore = usdc.balanceOf(RELAYER);
        vm.prank(RELAYER);
        bounty.verifyAndSettle(abi.encode(nonce));

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Settled, "bounded bounty not settled");
        require(usdc.balanceOf(address(bounty)) == 0, "bounded bounty retained USDC");
        require(usdc.balanceOf(address(creatorWallet)) == 0, "creator wallet retained funding");
        require(usdc.balanceOf(address(solverWallet)) == 300_000, "solver wallet payout mismatch");
        require(usdc.balanceOf(RELAYER) == relayerBefore + 100_000, "bounded verifier payout mismatch");
        require(creatorWallet.lifetimeSpent() == 300_000, "creator cap accounting mismatch");
        require(solverWallet.lifetimeSpent() == 100_000, "solver cap accounting mismatch");
    }

    function _createRelayLoopBounty() private returns (RelayLoopContext memory context) {
        AgentBountyFactory factory = AgentBountyFactory(FACTORY);
        MainnetUsdc usdc = MainnetUsdc(USDC);
        context.creator = vm.addr(GAS_RELAY_CREATOR_KEY);
        context.solver = vm.addr(GAS_RELAY_SOLVER_KEY);
        context.creatorBefore = usdc.balanceOf(context.creator);
        context.solverBefore = usdc.balanceOf(context.solver);

        vm.prank(FUNDING_SOURCE);
        require(usdc.transfer(context.creator, 1_000_000), "fork creator funding failed");
        vm.prank(BOND_SOURCE);
        require(usdc.transfer(context.solver, 100_000), "fork solver bond funding failed");

        AgentBountyFactory.CreateBountyParams memory params = AgentBountyFactory.CreateBountyParams({
            solverReward: 900_000,
            verifierReward: 100_000,
            termsHash: TERMS_HASH,
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: CRITERIA_HASH,
            benchmarkHash: BENCHMARK_HASH,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH,
            fundingDeadline: uint64(block.timestamp + 30 days),
            claimWindowSeconds: 1 days,
            verificationWindowSeconds: 2 hours,
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: MODULE,
            verifierRewardRecipient: context.creator,
            threshold: 1
        });
        vm.prank(context.creator);
        require(usdc.approve(FACTORY, 1_000_000), "creator approval failed");
        vm.prank(context.creator);
        (address bountyAddress, bytes32 bountyId) = factory.createBounty(
            params, new address[](0), 1_000_000, keccak256("agent-bounties/mainnet-fork/gas-relay/v1")
        );
        context.bounty = AgentBounty(bountyAddress);
        context.bountyId = bountyId;
    }

    function _boundedPolicy(address delegate) private view returns (BoundedAgentWallet.Policy memory) {
        return BoundedAgentWallet.Policy({
            delegate: delegate,
            validAfter: uint64(block.timestamp),
            validUntil: uint64(block.timestamp + 90 days),
            periodSeconds: 1 days,
            maxPerAction: 500_000,
            maxPerPeriod: 1_000_000,
            maxLifetimeSpend: 1_000_000,
            allowedActions: 15,
            allowedVerificationModes: 1
        });
    }

    function _deployBoundedWallets(MainnetUsdc usdc)
        private
        returns (BoundedAgentWallet creatorWallet, BoundedAgentWallet solverWallet)
    {
        BoundedAgentWalletFactory walletFactory = new BoundedAgentWalletFactory(FACTORY);
        address owner = vm.addr(BOUNDED_OWNER_KEY);
        vm.prank(FUNDING_SOURCE);
        require(usdc.transfer(owner, 400_000), "wallet owner funding failed");
        creatorWallet = _createAuthorizedWallet(
            walletFactory,
            owner,
            _boundedPolicy(vm.addr(BOUNDED_CREATOR_DELEGATE_KEY)),
            keccak256("mainnet-fork-bounded-creator"),
            300_000,
            keccak256("mainnet-fork-bounded-creator-authorization")
        );
        solverWallet = _createAuthorizedWallet(
            walletFactory,
            owner,
            _boundedPolicy(vm.addr(BOUNDED_SOLVER_DELEGATE_KEY)),
            keccak256("mainnet-fork-bounded-solver"),
            100_000,
            keccak256("mainnet-fork-bounded-solver-authorization")
        );
        require(usdc.balanceOf(owner) == 0, "wallet owner retained pilot funding");
        require(usdc.balanceOf(address(creatorWallet)) == 300_000, "creator wallet funding mismatch");
        require(usdc.balanceOf(address(solverWallet)) == 100_000, "solver wallet funding mismatch");
        require(usdc.balanceOf(address(walletFactory)) == 0, "wallet factory retained USDC");
    }

    function _createAuthorizedWallet(
        BoundedAgentWalletFactory walletFactory,
        address owner,
        BoundedAgentWallet.Policy memory policy,
        bytes32 salt,
        uint256 amount,
        bytes32 authorizationNonce
    ) private returns (BoundedAgentWallet wallet) {
        address predicted = walletFactory.predictWallet(owner, policy, salt);
        bytes32 digest =
            _usdcAuthorizationDigest(owner, predicted, amount, 0, type(uint256).max, authorizationNonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(BOUNDED_OWNER_KEY, digest);
        vm.prank(RELAYER);
        address deployed = walletFactory.createWalletWithAuthorization(
            owner, policy, salt, amount, 0, type(uint256).max, authorizationNonce, v, r, s
        );
        require(deployed == predicted, "authorized wallet address drift");
        wallet = BoundedAgentWallet(payable(deployed));
    }

    function _createBoundedBounty(BoundedAgentWallet creatorWallet) private returns (AgentBounty bounty) {
        AgentBountyFactory factory = AgentBountyFactory(FACTORY);
        AgentBountyFactory.CreateBountyParams memory params = AgentBountyFactory.CreateBountyParams({
            solverReward: 200_000,
            verifierReward: 100_000,
            termsHash: TERMS_HASH,
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: CRITERIA_HASH,
            benchmarkHash: BENCHMARK_HASH,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH,
            fundingDeadline: uint64(block.timestamp + 30 days),
            claimWindowSeconds: 1 days,
            verificationWindowSeconds: 2 hours,
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: MODULE,
            verifierRewardRecipient: RELAYER,
            threshold: 1
        });
        bytes32 creationNonce = keccak256("mainnet-fork-bounded-wallet-loop/v1");
        address[] memory noVerifiers = new address[](0);
        address predicted = factory.predictBountyAddress(address(creatorWallet), params, noVerifiers, creationNonce);
        _relayBoundedAction(
            creatorWallet,
            BoundedAgentWallet.Action.Create,
            abi.encode(params, noVerifiers, uint256(300_000), creationNonce),
            BOUNDED_CREATOR_DELEGATE_KEY
        );
        bounty = AgentBounty(predicted);
        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Claimable, "bounded bounty not claimable");
    }

    function _relayBoundedAction(
        BoundedAgentWallet wallet,
        BoundedAgentWallet.Action action,
        bytes memory payload,
        uint256 delegateKey
    ) private {
        uint256 nonce = wallet.delegateNonce();
        uint256 deadline = block.timestamp + 30 minutes;
        bytes32 digest = wallet.actionDigest(action, keccak256(payload), nonce, deadline);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(delegateKey, digest);
        vm.prank(RELAYER);
        wallet.executeWithSignature(action, payload, nonce, deadline, abi.encodePacked(r, s, v));
    }

    function _relayClaim(AgentBounty bounty, address solver) private {
        bytes32 authorizationNonce = keccak256("agent-bounties/mainnet-fork/gas-relay/claim/v1");
        uint256 validBefore = block.timestamp + 1 hours;
        bytes32 authorizationDigest =
            _usdcAuthorizationDigest(solver, address(bounty), 100_000, 0, validBefore, authorizationNonce);
        (uint8 claimV, bytes32 claimR, bytes32 claimS) = vm.sign(GAS_RELAY_SOLVER_KEY, authorizationDigest);

        vm.prank(RELAYER);
        bounty.claimWithAuthorization(solver, 0, validBefore, authorizationNonce, claimV, claimR, claimS);
        require(bounty.solver() == solver, "relayed solver mismatch");
        require(bounty.activeClaimBond() == 100_000, "relayed bond missing");
    }

    function _relaySubmission(AgentBounty bounty, address solver) private {
        uint256 deadline = block.timestamp + 30 minutes;
        bytes32 digest = bounty.submitDigest(solver, bounty.round(), SUBMISSION_HASH, EVIDENCE_HASH, deadline);
        (uint8 submitV, bytes32 submitR, bytes32 submitS) = vm.sign(GAS_RELAY_SOLVER_KEY, digest);

        vm.prank(RELAYER);
        bounty.submitWithSignature(
            SUBMISSION_HASH, EVIDENCE_HASH, deadline, abi.encodePacked(submitR, submitS, submitV)
        );
        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Submitted, "relayed submission missing");
    }

    function _relaySettlement(AgentBounty bounty, bytes32 bountyId, address solver) private {
        uint256 nonce = _mineLeadingZeroNonce(bountyId, bounty.round(), solver);

        vm.prank(RELAYER);
        bounty.verifyAndSettle(abi.encode(nonce));
    }

    function _selectMainnetFork() private {
        vm.createSelectFork(vm.envString("BASE_MAINNET_RPC_URL"), MAINNET_FORK_BLOCK);
        require(block.number == MAINNET_FORK_BLOCK, "fork block drift");
    }

    function _mineLeadingZeroNonce(bytes32 bountyId, uint64 currentRound, address solver)
        private
        pure
        returns (uint256 nonce)
    {
        bytes32 submissionHash = SUBMISSION_HASH;
        bytes32 evidenceHash = EVIDENCE_HASH;
        bytes32 policyHash = POLICY_HASH;
        assembly ("memory-safe") {
            let pointer := mload(0x40)
            mstore(pointer, bountyId)
            mstore(add(pointer, 0x20), currentRound)
            mstore(add(pointer, 0x40), solver)
            mstore(add(pointer, 0x60), submissionHash)
            mstore(add(pointer, 0x80), evidenceHash)
            mstore(add(pointer, 0xa0), policyHash)
            for {} lt(nonce, 1000000) { nonce := add(nonce, 1) } {
                mstore(add(pointer, 0xc0), nonce)
                if iszero(shr(240, keccak256(pointer, 0xe0))) { break }
            }
        }
        require(nonce < 1_000_000, "proof search cap reached");
    }

    function _childLoopParams(
        IAgentBountyVerifier verifier,
        bytes32 acceptanceCriteriaHash,
        bytes32 benchmarkHash,
        uint256 solverReward,
        uint256 verifierReward
    ) private view returns (AgentBountyFactory.CreateBountyParams memory) {
        return AgentBountyFactory.CreateBountyParams({
            solverReward: solverReward,
            verifierReward: verifierReward,
            termsHash: TERMS_HASH,
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: acceptanceCriteriaHash,
            benchmarkHash: benchmarkHash,
            evidenceSchemaHash: keccak256("mainnet-fork-child-loop-evidence-schema"),
            fundingDeadline: uint64(block.timestamp + 30 days),
            claimWindowSeconds: 1 days,
            verificationWindowSeconds: 2 hours,
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: address(verifier),
            verifierRewardRecipient: CREATOR,
            threshold: 1
        });
    }

    function _usdcAuthorizationDigest(
        address from,
        address to,
        uint256 value,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce
    ) private view returns (bytes32) {
        bytes32 domainSeparator = keccak256(
            abi.encode(EIP712_DOMAIN_TYPEHASH, keccak256("USD Coin"), keccak256("2"), block.chainid, USDC)
        );
        bytes32 structHash = keccak256(
            abi.encode(TRANSFER_WITH_AUTHORIZATION_TYPEHASH, from, to, value, validAfter, validBefore, nonce)
        );
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator, structHash));
    }
}
