// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactory.sol";
import "../src/CanonicalChildBountyVerifier.sol";
import "../src/LeadingZeroWorkVerifier.sol";

interface ChildLoopVm {
    function warp(uint256 timestamp) external;
}

contract ChildLoopToken {
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

contract ChildLoopActor {
    function approve(ChildLoopToken token, address spender, uint256 amount) external {
        token.approve(spender, amount);
    }

    function create(
        AgentBountyFactory factory,
        AgentBountyFactory.CreateBountyParams calldata params,
        uint256 initialFunding,
        bytes32 creationNonce
    ) external returns (address bountyAddress) {
        (bountyAddress,) = factory.createBounty(params, new address[](0), initialFunding, creationNonce);
    }

    function fund(AgentBounty bounty, uint256 amount) external {
        bounty.fund(amount);
    }

    function claim(AgentBounty bounty) external {
        bounty.claim();
    }

    function submit(AgentBounty bounty, bytes32 submissionHash, bytes32 evidenceHash) external {
        bounty.submit(submissionHash, evidenceHash);
    }
}

contract CanonicalChildBountyVerifierTest {
    ChildLoopVm private constant vm = ChildLoopVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    bytes32 private constant TERMS_HASH = keccak256("canonical-child-terms-v1");
    bytes32 private constant POLICY_HASH = keccak256("canonical-child-policy-v1");
    bytes32 private constant EVIDENCE_SCHEMA_HASH = keccak256("canonical-child-evidence-v1");
    bytes32 private constant PARENT_SUBMISSION_HASH = keccak256("parent-submission");
    bytes32 private constant PARENT_EVIDENCE_HASH = keccak256("parent-evidence");
    bytes32 private constant CHILD_SUBMISSION_HASH = keccak256("child-submission");
    bytes32 private constant CHILD_EVIDENCE_HASH = keccak256("child-evidence");

    ChildLoopToken private token;
    AgentBountyFactory private factory;
    CanonicalChildBountyVerifier private module;
    ChildLoopActor private parentSolver;
    ChildLoopActor private childSolver;
    ChildLoopActor private grandchildSolver;
    ChildLoopActor private poolContributor;
    ChildLoopActor private verifierRecipient;
    uint256 private nonce;

    function setUp() public {
        token = new ChildLoopToken();
        factory = new AgentBountyFactory(address(token));
        module = new CanonicalChildBountyVerifier(address(factory));
        parentSolver = new ChildLoopActor();
        childSolver = new ChildLoopActor();
        grandchildSolver = new ChildLoopActor();
        poolContributor = new ChildLoopActor();
        verifierRecipient = new ChildLoopActor();
        token.mint(address(this), 100_000);
        token.approve(address(factory), type(uint256).max);
    }

    function testPooledChildSubmissionSettlesParentAndLeavesLiveWork() public {
        AgentBounty parent = _createSubmittedParent(module.ACCEPTANCE_CRITERIA_HASH(), 2 hours);
        AgentBounty child = _createPooledSubmittedChild(parent, module, 800, 100, 2 hours);

        parent.verifyAndSettle(abi.encode(address(child)));

        require(parent.bountyStatus() == AgentBounty.BountyStatus.Settled, "parent not settled");
        require(child.bountyStatus() == AgentBounty.BountyStatus.Submitted, "child work not live");
        require(token.balanceOf(address(parentSolver)) == 1_000, "parent solver payout mismatch");
        require(token.balanceOf(address(verifierRecipient)) == 100, "verifier payout mismatch");
        require(token.balanceOf(address(parent)) == 0, "parent retained funds");
        require(token.balanceOf(address(child)) == 1_000, "child funding or bond missing");
    }

    function testVerifierRecursesAcrossTwoPaidGenerations() public {
        AgentBounty parent = _createSubmittedParent(module.ACCEPTANCE_CRITERIA_HASH(), 2 hours);
        AgentBounty child = _createPooledSubmittedChild(parent, module, 800, 100, 2 hours);
        parent.verifyAndSettle(abi.encode(address(child)));

        bytes32 benchmarkHash = module.expectedBenchmarkHash(child.bountyId(), child.round());
        AgentBounty grandchild = _createChild(factory, childSolver, module, benchmarkHash, 700, 100, 800, 2 hours);
        _submit(grandchild, grandchildSolver, keccak256("grandchild-submission"), keccak256("grandchild-evidence"));

        child.verifyAndSettle(abi.encode(address(grandchild)));

        require(child.bountyStatus() == AgentBounty.BountyStatus.Settled, "child not settled");
        require(grandchild.bountyStatus() == AgentBounty.BountyStatus.Submitted, "grandchild work not live");
        require(token.balanceOf(address(childSolver)) == 900, "child solver payout mismatch");
        require(token.balanceOf(address(verifierRecipient)) == 200, "two verifier rewards missing");
    }

    function testClaimableChildCannotSettleParent() public {
        AgentBounty parent = _createSubmittedParent(module.ACCEPTANCE_CRITERIA_HASH(), 2 hours);
        AgentBounty child = _createLinkedChild(parent, parentSolver, module, 800, 100, 900, 2 hours);

        parent.verifyAndSettle(abi.encode(address(child)));

        _assertRejected(parent);
        require(child.bountyStatus() == AgentBounty.BountyStatus.Claimable, "child status changed");
    }

    function testChildCreatedByDifferentWalletCannotSettleParent() public {
        AgentBounty parent = _createSubmittedParent(module.ACCEPTANCE_CRITERIA_HASH(), 2 hours);
        bytes32 benchmarkHash = module.expectedBenchmarkHash(parent.bountyId(), parent.round());
        AgentBounty child = _createChild(factory, poolContributor, module, benchmarkHash, 800, 100, 900, 2 hours);
        _submit(child, childSolver, CHILD_SUBMISSION_HASH, CHILD_EVIDENCE_HASH);

        parent.verifyAndSettle(abi.encode(address(child)));

        _assertRejected(parent);
    }

    function testChildBelowParentSolverRewardCannotSettleParent() public {
        AgentBounty parent = _createSubmittedParent(module.ACCEPTANCE_CRITERIA_HASH(), 2 hours);
        AgentBounty child = _createLinkedChild(parent, parentSolver, module, 798, 101, 899, 2 hours);
        _submit(child, childSolver, CHILD_SUBMISSION_HASH, CHILD_EVIDENCE_HASH);

        parent.verifyAndSettle(abi.encode(address(child)));

        _assertRejected(parent);
    }

    function testWrongParentRoundBenchmarkCannotSettleParent() public {
        AgentBounty parent = _createSubmittedParent(module.ACCEPTANCE_CRITERIA_HASH(), 2 hours);
        bytes32 wrongBenchmark = module.expectedBenchmarkHash(parent.bountyId(), parent.round() + 1);
        AgentBounty child = _createChild(factory, parentSolver, module, wrongBenchmark, 800, 100, 900, 2 hours);
        _submit(child, childSolver, CHILD_SUBMISSION_HASH, CHILD_EVIDENCE_HASH);

        parent.verifyAndSettle(abi.encode(address(child)));

        _assertRejected(parent);
    }

    function testWrongChildVerifierCannotSettleParent() public {
        AgentBounty parent = _createSubmittedParent(module.ACCEPTANCE_CRITERIA_HASH(), 2 hours);
        LeadingZeroWorkVerifier otherModule = new LeadingZeroWorkVerifier(8);
        bytes32 benchmarkHash = module.expectedBenchmarkHash(parent.bountyId(), parent.round());
        AgentBounty child = _createChild(factory, parentSolver, otherModule, benchmarkHash, 800, 100, 900, 2 hours);
        _submit(child, childSolver, CHILD_SUBMISSION_HASH, CHILD_EVIDENCE_HASH);

        parent.verifyAndSettle(abi.encode(address(child)));

        _assertRejected(parent);
    }

    function testExternalFactoryChildCannotSettleParent() public {
        AgentBounty parent = _createSubmittedParent(module.ACCEPTANCE_CRITERIA_HASH(), 2 hours);
        AgentBountyFactory externalFactory = new AgentBountyFactory(address(token));
        bytes32 benchmarkHash = module.expectedBenchmarkHash(parent.bountyId(), parent.round());
        AgentBounty child = _createChild(externalFactory, parentSolver, module, benchmarkHash, 800, 100, 900, 2 hours);
        _submit(child, childSolver, CHILD_SUBMISSION_HASH, CHILD_EVIDENCE_HASH);

        parent.verifyAndSettle(abi.encode(address(child)));

        _assertRejected(parent);
    }

    function testExpiredChildSubmissionCannotSettleParent() public {
        AgentBounty parent = _createSubmittedParent(module.ACCEPTANCE_CRITERIA_HASH(), 2 hours);
        AgentBounty child = _createLinkedChild(parent, parentSolver, module, 800, 100, 900, 60);
        _submit(child, childSolver, CHILD_SUBMISSION_HASH, CHILD_EVIDENCE_HASH);
        vm.warp(block.timestamp + 61);

        parent.verifyAndSettle(abi.encode(address(child)));

        _assertRejected(parent);
    }

    function testMisleadingParentCriteriaCannotUseModule() public {
        AgentBounty parent = _createSubmittedParent(keccak256("different-criteria"), 2 hours);
        AgentBounty child = _createPooledSubmittedChild(parent, module, 800, 100, 2 hours);

        parent.verifyAndSettle(abi.encode(address(child)));

        _assertRejected(parent);
    }

    function testMalformedProofCannotSettleParent() public {
        AgentBounty parent = _createSubmittedParent(module.ACCEPTANCE_CRITERIA_HASH(), 2 hours);

        parent.verifyAndSettle(hex"01");

        _assertRejected(parent);
    }

    function testBenchmarkEncodingIsStableAndLowercase() public view {
        bytes32 parentId = bytes32(uint256(0xabcdef));
        string memory expected = '{"parent_bounty_id":"0x0000000000000000000000000000000000000000000000000000000000abcdef",'
            '"parent_round_hex":"0x0123456789abcdef","protocol":"agent-bounties/canonical-child-v1"}';
        string memory actual = module.expectedBenchmarkJson(parentId, 0x0123456789abcdef);

        require(keccak256(bytes(actual)) == keccak256(bytes(expected)), "benchmark JSON changed");
        require(module.expectedBenchmarkHash(parentId, 0x0123456789abcdef) == keccak256(bytes(expected)), "bad hash");
    }

    function testAcceptanceCriteriaHashMatchesCanonicalJsonPlanner() public view {
        require(
            module.ACCEPTANCE_CRITERIA_HASH() == 0x005f591a8549549698e7c028b78ddc84076e0996ef07e19dd543ebdb12cb4553,
            "acceptance hash drift"
        );
    }

    function _createSubmittedParent(bytes32 criteriaHash, uint64 verificationWindow)
        private
        returns (AgentBounty parent)
    {
        AgentBountyFactory.CreateBountyParams memory params =
            _params(module, keccak256("root-benchmark"), criteriaHash, 900, 100, verificationWindow);
        (address parentAddress,) = factory.createBounty(params, new address[](0), 1_000, _nextNonce());
        parent = AgentBounty(parentAddress);
        _submit(parent, parentSolver, PARENT_SUBMISSION_HASH, PARENT_EVIDENCE_HASH);
    }

    function _createPooledSubmittedChild(
        AgentBounty parent,
        IAgentBountyVerifier verifier,
        uint256 solverReward,
        uint256 verifierReward,
        uint64 verificationWindow
    ) private returns (AgentBounty child) {
        child = _createLinkedChild(
            parent, parentSolver, verifier, solverReward, verifierReward, 400, verificationWindow
        );
        token.mint(address(poolContributor), child.targetAmount() - 400);
        poolContributor.approve(token, address(child), child.targetAmount() - 400);
        poolContributor.fund(child, child.targetAmount() - 400);
        _submit(child, childSolver, CHILD_SUBMISSION_HASH, CHILD_EVIDENCE_HASH);
    }

    function _createLinkedChild(
        AgentBounty parent,
        ChildLoopActor creator,
        IAgentBountyVerifier verifier,
        uint256 solverReward,
        uint256 verifierReward,
        uint256 initialFunding,
        uint64 verificationWindow
    ) private returns (AgentBounty) {
        return _createChild(
            factory,
            creator,
            verifier,
            module.expectedBenchmarkHash(parent.bountyId(), parent.round()),
            solverReward,
            verifierReward,
            initialFunding,
            verificationWindow
        );
    }

    function _createChild(
        AgentBountyFactory childFactory,
        ChildLoopActor creator,
        IAgentBountyVerifier verifier,
        bytes32 benchmarkHash,
        uint256 solverReward,
        uint256 verifierReward,
        uint256 initialFunding,
        uint64 verificationWindow
    ) private returns (AgentBounty child) {
        AgentBountyFactory.CreateBountyParams memory params = _params(
            verifier, benchmarkHash, module.ACCEPTANCE_CRITERIA_HASH(), solverReward, verifierReward, verificationWindow
        );
        if (initialFunding > 0) {
            token.mint(address(creator), initialFunding);
            creator.approve(token, address(childFactory), initialFunding);
        }
        child = AgentBounty(creator.create(childFactory, params, initialFunding, _nextNonce()));
    }

    function _submit(AgentBounty bounty, ChildLoopActor solver, bytes32 submissionHash, bytes32 evidenceHash) private {
        uint256 bond = bounty.verifierReward();
        token.mint(address(solver), bond);
        solver.approve(token, address(bounty), bond);
        solver.claim(bounty);
        solver.submit(bounty, submissionHash, evidenceHash);
    }

    function _params(
        IAgentBountyVerifier verifier,
        bytes32 benchmarkHash,
        bytes32 criteriaHash,
        uint256 solverReward,
        uint256 verifierReward,
        uint64 verificationWindow
    ) private view returns (AgentBountyFactory.CreateBountyParams memory) {
        return AgentBountyFactory.CreateBountyParams({
            solverReward: solverReward,
            verifierReward: verifierReward,
            termsHash: TERMS_HASH,
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: criteriaHash,
            benchmarkHash: benchmarkHash,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH,
            fundingDeadline: uint64(block.timestamp + 1 days),
            claimWindowSeconds: 1 hours,
            verificationWindowSeconds: verificationWindow,
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: address(verifier),
            verifierRewardRecipient: address(verifierRecipient),
            threshold: 1
        });
    }

    function _assertRejected(AgentBounty parent) private view {
        require(parent.bountyStatus() == AgentBounty.BountyStatus.Claimable, "parent did not reject");
        require(parent.fundedAmount() == parent.targetAmount(), "parent funding changed");
        require(parent.solver() == address(0), "parent solver not reset");
        require(token.balanceOf(address(parent)) == parent.targetAmount(), "parent reserve changed");
        require(token.balanceOf(address(verifierRecipient)) == 100, "verifier not paid equally");
    }

    function _nextNonce() private returns (bytes32) {
        nonce += 1;
        return bytes32(nonce);
    }
}
