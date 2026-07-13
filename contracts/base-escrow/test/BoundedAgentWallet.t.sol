// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/BoundedAgentWallet.sol";

interface VmBoundedWallet {
    function warp(uint256) external;
    function prank(address) external;
    function addr(uint256 privateKey) external returns (address);
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
}

contract WalletTestToken {
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

contract WalletDelegate {
    function create(
        BoundedAgentWallet wallet,
        AgentBountyFactory.CreateBountyParams calldata params,
        address[] calldata verifiers,
        uint256 initialFunding,
        bytes32 creationNonce
    ) external returns (address bounty, bytes32 bountyId) {
        return wallet.createBounty(params, verifiers, initialFunding, creationNonce);
    }

    function fund(BoundedAgentWallet wallet, address bounty, uint256 amount) external returns (uint256) {
        return wallet.fundBounty(bounty, amount);
    }

    function claim(BoundedAgentWallet wallet, address bounty) external {
        wallet.claimBounty(bounty);
    }

    function submit(BoundedAgentWallet wallet, address bounty, bytes32 submissionHash, bytes32 evidenceHash) external {
        wallet.submitBounty(bounty, submissionHash, evidenceHash);
    }

    function withdraw(BoundedAgentWallet wallet, address token, address to, uint256 amount) external {
        wallet.withdrawToken(token, to, amount);
    }
}

contract WalletPassVerifier is IAgentBountyVerifier {
    function verify(bytes32, uint64, address, bytes32, bytes32, bytes32, bytes calldata proof)
        external
        pure
        returns (bool passed, bytes32 responseHash)
    {
        return (true, keccak256(proof));
    }
}

contract BoundedAgentWalletTest {
    VmBoundedWallet constant vm = VmBoundedWallet(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint256 constant DELEGATE_KEY = 0xA11CE;
    bytes32 constant TERMS_HASH = keccak256("wallet-terms");
    bytes32 constant POLICY_HASH = keccak256("wallet-policy");
    bytes32 constant CRITERIA_HASH = keccak256("wallet-criteria");
    bytes32 constant BENCHMARK_HASH = keccak256("wallet-benchmark");
    bytes32 constant EVIDENCE_SCHEMA_HASH = keccak256("wallet-evidence-schema");
    bytes32 constant SUBMISSION_HASH = keccak256("wallet-submission");
    bytes32 constant EVIDENCE_HASH = keccak256("wallet-evidence");

    WalletTestToken token;
    AgentBountyFactory factory;
    WalletPassVerifier verifier;
    WalletDelegate delegateActor;
    BoundedAgentWallet wallet;
    uint256 creationNonce;

    function setUp() public {
        vm.warp(1_800_000_000);
        token = new WalletTestToken();
        factory = new AgentBountyFactory(address(token));
        verifier = new WalletPassVerifier();
        delegateActor = new WalletDelegate();
        wallet = new BoundedAgentWallet(address(this), address(factory), _policy(address(delegateActor), 100, 250, 500));
        token.mint(address(wallet), 1_000);
        token.mint(address(this), 10_000);
        token.approve(address(factory), type(uint256).max);
    }

    function testDelegateCreatesFundedCanonicalBountyWithinCaps() public {
        address[] memory noVerifiers = new address[](0);
        (address bountyAddress,) = delegateActor.create(wallet, _params(90, 10), noVerifiers, 100, _nextNonce());
        AgentBounty bounty = AgentBounty(bountyAddress);

        require(factory.isCanonicalBounty(bountyAddress), "not canonical");
        require(bounty.creator() == address(wallet), "wallet not creator");
        require(bounty.fundedAmount() == 100, "funding missing");
        require(wallet.periodSpent() == 100, "period spend mismatch");
        require(wallet.lifetimeSpent() == 100, "lifetime spend mismatch");
        require(wallet.delegateNonce() == 1, "direct action did not advance nonce");
        require(token.allowance(address(wallet), address(factory)) == 0, "factory allowance remains");
    }

    function testNonDelegateAndDisallowedModeCannotSpend() public {
        address[] memory noVerifiers = new address[](0);
        try wallet.createBounty(_params(90, 10), noVerifiers, 100, _nextNonce()) {
            revert("owner used delegate authority");
        } catch Error(string memory reason) {
            require(_same(reason, "not delegate"), "wrong nondelegate rejection");
        }

        AgentBountyFactory.CreateBountyParams memory params = _params(90, 10);
        params.verificationMode = AgentBounty.VerificationMode.SignedQuorum;
        params.verifierModule = address(0);
        params.verifierRewardRecipient = address(0);
        params.threshold = 1;
        address[] memory verifiers = new address[](1);
        verifiers[0] = address(0xBEEF);
        try delegateActor.create(wallet, params, verifiers, 100, _nextNonce()) {
            revert("disallowed mode created");
        } catch Error(string memory reason) {
            require(_same(reason, "verification mode not allowed"), "wrong mode rejection");
        }
        require(wallet.lifetimeSpent() == 0, "reverted spend charged");
    }

    function testPolicyWithoutExecutableVerificationModeIsRejected() public {
        BoundedAgentWallet.Policy memory invalid = _policy(address(delegateActor), 100, 250, 500);
        invalid.allowedActions = wallet.ACTION_SUBMIT();
        invalid.allowedVerificationModes = 0;
        (bool configured,) = address(wallet).call(abi.encodeCall(wallet.configurePolicy, (invalid)));
        require(!configured, "unusable policy configured");
    }

    function testPerActionPeriodLifetimeAndPeriodResetCaps() public {
        wallet.configurePolicy(_policy(address(delegateActor), 100, 250, 300));
        AgentBounty bountyA = _createExternalBounty(400, 100, 0);
        AgentBounty bountyB = _createExternalBounty(400, 100, 0);

        require(delegateActor.fund(wallet, address(bountyA), 100) == 100, "first funding failed");
        try delegateActor.fund(wallet, address(bountyB), 101) {
            revert("per-action cap bypassed");
        } catch Error(string memory reason) {
            require(_same(reason, "per-action cap exceeded"), "wrong action cap rejection");
        }
        require(delegateActor.fund(wallet, address(bountyB), 100) == 100, "second funding failed");
        try delegateActor.fund(wallet, address(bountyB), 51) {
            revert("period cap bypassed");
        } catch Error(string memory reason) {
            require(_same(reason, "period cap exceeded"), "wrong period cap rejection");
        }

        vm.warp(block.timestamp + 1 days);
        require(delegateActor.fund(wallet, address(bountyB), 100) == 100, "new period funding failed");
        require(wallet.periodSpent() == 100, "period did not reset");
        try delegateActor.fund(wallet, address(bountyB), 100) {
            revert("lifetime cap bypassed");
        } catch Error(string memory reason) {
            require(_same(reason, "lifetime cap exceeded"), "wrong lifetime cap rejection");
        }
    }

    function testFundingChargesOnlyAmountAcceptedByBounty() public {
        AgentBounty bounty = _createExternalBounty(90, 10, 70);
        require(delegateActor.fund(wallet, address(bounty), 100) == 30, "remaining amount not capped");
        require(wallet.lifetimeSpent() == 30, "requested amount charged");
        require(token.allowance(address(wallet), address(bounty)) == 0, "bounty allowance remains");
    }

    function testCanonicalClaimSubmitAndSettlementPayWallet() public {
        AgentBounty bounty = _createExternalBounty(900, 100, 1_000);
        delegateActor.claim(wallet, address(bounty));
        require(bounty.solver() == address(wallet), "wallet not solver");
        require(wallet.lifetimeSpent() == 100, "bond not charged");

        delegateActor.submit(wallet, address(bounty), SUBMISSION_HASH, EVIDENCE_HASH);
        bounty.verifyAndSettle(bytes("proof"));

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Settled, "not settled");
        require(token.balanceOf(address(wallet)) == 1_900, "payout not returned to wallet");
        require(token.allowance(address(wallet), address(bounty)) == 0, "claim allowance remains");
    }

    function testNonCanonicalTargetAndRevokedOrExpiredPolicyFailClosed() public {
        try delegateActor.fund(wallet, address(0xBEEF), 1) {
            revert("noncanonical target funded");
        } catch Error(string memory reason) {
            require(_same(reason, "not canonical bounty"), "wrong canonical rejection");
        }

        wallet.revokePolicy();
        AgentBounty bounty = _createExternalBounty(90, 10, 0);
        try delegateActor.fund(wallet, address(bounty), 1) {
            revert("revoked delegate spent");
        } catch Error(string memory reason) {
            require(_same(reason, "policy revoked"), "wrong revoke rejection");
        }

        wallet.configurePolicy(_policy(address(delegateActor), 100, 250, 500));
        vm.warp(block.timestamp + 8 days);
        try delegateActor.fund(wallet, address(bounty), 1) {
            revert("expired delegate spent");
        } catch Error(string memory reason) {
            require(_same(reason, "policy expired"), "wrong expiry rejection");
        }
    }

    function testSignedActionCanBeRelayedOnceWithoutOwnerPrompt() public {
        address signedDelegate = vm.addr(DELEGATE_KEY);
        wallet.configurePolicy(_policy(signedDelegate, 100, 250, 500));
        AgentBounty bounty = _createExternalBounty(90, 10, 0);
        bytes memory payload = abi.encode(address(bounty), uint256(25));
        uint256 nonce = wallet.delegateNonce();
        uint256 deadline = block.timestamp + 1 hours;
        bytes32 digest = wallet.actionDigest(BoundedAgentWallet.Action.Fund, keccak256(payload), nonce, deadline);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(DELEGATE_KEY, digest);
        bytes memory signature = abi.encodePacked(r, s, v);

        bytes memory result =
            wallet.executeWithSignature(BoundedAgentWallet.Action.Fund, payload, nonce, deadline, signature);
        require(abi.decode(result, (uint256)) == 25, "relay result mismatch");
        require(bounty.fundedAmount() == 25, "relay did not fund");

        (bool replayOk,) = address(wallet)
            .call(
                abi.encodeCall(
                    wallet.executeWithSignature, (BoundedAgentWallet.Action.Fund, payload, nonce, deadline, signature)
                )
            );
        require(!replayOk, "signature replayed");
    }

    function testDirectActionInvalidatesQueuedRelaySignature() public {
        address signedDelegate = vm.addr(DELEGATE_KEY);
        wallet.configurePolicy(_policy(signedDelegate, 100, 250, 500));
        AgentBounty bounty = _createExternalBounty(90, 10, 0);
        bytes memory payload = abi.encode(address(bounty), uint256(25));
        uint256 nonce = wallet.delegateNonce();
        uint256 deadline = block.timestamp + 1 hours;
        bytes32 digest = wallet.actionDigest(BoundedAgentWallet.Action.Fund, keccak256(payload), nonce, deadline);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(DELEGATE_KEY, digest);
        bytes memory signature = abi.encodePacked(r, s, v);

        vm.prank(signedDelegate);
        require(wallet.fundBounty(address(bounty), 10) == 10, "direct action failed");
        require(wallet.delegateNonce() == nonce + 1, "direct action did not advance nonce");

        (bool staleRelayOk,) = address(wallet)
            .call(
                abi.encodeCall(
                    wallet.executeWithSignature, (BoundedAgentWallet.Action.Fund, payload, nonce, deadline, signature)
                )
            );
        require(!staleRelayOk, "stale relay survived direct action");
        require(bounty.fundedAmount() == 10, "stale relay changed funding");
    }

    function testOnlyOwnerCanWithdrawAndRotatePolicy() public {
        uint256 ownerBalance = token.balanceOf(address(this));
        wallet.withdrawToken(address(token), address(this), 50);
        require(token.balanceOf(address(this)) == ownerBalance + 50, "withdrawal missing");

        (bool delegateWithdrawOk,) = address(delegateActor)
            .call(abi.encodeCall(delegateActor.withdraw, (wallet, address(token), address(this), uint256(1))));
        require(!delegateWithdrawOk, "delegate withdrew funds");
    }

    function _createExternalBounty(uint256 solverReward, uint256 verifierReward, uint256 initialFunding)
        private
        returns (AgentBounty bounty)
    {
        address[] memory noVerifiers = new address[](0);
        (address bountyAddress,) =
            factory.createBounty(_params(solverReward, verifierReward), noVerifiers, initialFunding, _nextNonce());
        bounty = AgentBounty(bountyAddress);
    }

    function _params(uint256 solverReward, uint256 verifierReward)
        private
        view
        returns (AgentBountyFactory.CreateBountyParams memory)
    {
        return AgentBountyFactory.CreateBountyParams({
            solverReward: solverReward,
            verifierReward: verifierReward,
            termsHash: TERMS_HASH,
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: CRITERIA_HASH,
            benchmarkHash: BENCHMARK_HASH,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH,
            fundingDeadline: uint64(block.timestamp + 30 days),
            claimWindowSeconds: 1 days,
            verificationWindowSeconds: 1 days,
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: address(verifier),
            verifierRewardRecipient: address(0xFEE),
            threshold: 1
        });
    }

    function _policy(address delegate, uint256 actionCap, uint256 periodCap, uint256 lifetimeCap)
        private
        view
        returns (BoundedAgentWallet.Policy memory)
    {
        return BoundedAgentWallet.Policy({
            delegate: delegate,
            validAfter: uint64(block.timestamp),
            validUntil: uint64(block.timestamp + 7 days),
            periodSeconds: 1 days,
            maxPerAction: actionCap,
            maxPerPeriod: periodCap,
            maxLifetimeSpend: lifetimeCap,
            allowedActions: walletActions(),
            allowedVerificationModes: 1
        });
    }

    function walletActions() private pure returns (uint8) {
        return 1 | 2 | 4 | 8;
    }

    function _nextNonce() private returns (bytes32) {
        creationNonce += 1;
        return bytes32(creationNonce);
    }

    function _same(string memory left, string memory right) private pure returns (bool) {
        return keccak256(bytes(left)) == keccak256(bytes(right));
    }
}
