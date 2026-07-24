// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactory.sol";
import "../src/CanonicalIndependentChildVerifierV3.sol";
import "../src/StandingMetaV3Bundle.sol";

interface ProfitableChildVm {
    function addr(uint256 privateKey) external returns (address);
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
    function warp(uint256 timestamp) external;
}

contract ProfitableChildToken {
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

contract ProfitableChildActor {
    function approve(ProfitableChildToken token, address spender, uint256 amount) external {
        token.approve(spender, amount);
    }

    function create(
        AgentBountyFactory factory,
        AgentBountyFactory.CreateBountyParams calldata params,
        address[] calldata verifiers,
        uint256 initialFunding,
        bytes32 creationNonce
    ) external returns (AgentBounty bounty) {
        (address bountyAddress,) = factory.createBounty(params, verifiers, initialFunding, creationNonce);
        bounty = AgentBounty(bountyAddress);
    }

    function publish(
        OnchainTermsRegistry registry,
        bytes calldata terms,
        OnchainTermsRegistry.TermsInput calldata input
    ) external returns (bytes32) {
        return registry.publish(terms, input);
    }

    function claim(AgentBounty bounty) external {
        bounty.claim();
    }

    function submit(AgentBounty bounty, bytes32 submissionHash, bytes32 evidenceHash) external {
        bounty.submit(submissionHash, evidenceHash);
    }
}

contract CanonicalIndependentChildVerifierV3Test {
    ProfitableChildVm private constant vm =
        ProfitableChildVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint256 private constant ELIGIBILITY_ATTESTER_KEY = 0xA11CE;
    uint256 private constant TASK_VERIFIER_ONE_KEY = 0xBEEF;
    uint256 private constant TASK_VERIFIER_TWO_KEY = 0xCAFE;
    bytes32 private constant POLICY_HASH = keccak256("sandboxed-regression-policy-v1");
    bytes32 private constant CHILD_CRITERIA_HASH = keccak256("child-criteria");
    bytes32 private constant CHILD_BENCHMARK_HASH = keccak256("child-benchmark");
    bytes32 private constant EVIDENCE_SCHEMA_HASH = keccak256("child-evidence-schema");
    bytes32 private constant SOURCE_HASH = keccak256("github-source-v1");
    bytes32 private constant PARENT_PARTICIPANT = keccak256("participant-parent");
    bytes32 private constant CHILD_PARTICIPANT = keccak256("participant-child");
    bytes private constant CHILD_TERMS = '{"schema":"agent-bounties/test-child-v3"}';
    uint256 private constant PARENT_REWARD = 2_000_000;
    uint256 private constant PARENT_VERIFIER_REWARD = 10_000;
    uint256 private constant CHILD_TARGET = 1_000_000;
    uint256 private constant CHILD_VERIFIER_REWARD = 10_000;

    ProfitableChildToken private token;
    AgentBountyFactory private factory;
    ParticipantEligibilityRegistry private participantRegistry;
    OnchainTermsRegistry private termsRegistry;
    CanonicalIndependentChildVerifierV3 private module;
    ProfitableChildActor private parentSolver;
    ProfitableChildActor private childSolver;
    ProfitableChildActor private verifierRewardRecipient;
    address[] private taskVerifiers;
    uint256 private nonce;

    function setUp() public {
        token = new ProfitableChildToken();
        factory = new AgentBountyFactory(address(token));
        participantRegistry = new ParticipantEligibilityRegistry(vm.addr(ELIGIBILITY_ATTESTER_KEY));
        termsRegistry = new OnchainTermsRegistry();
        taskVerifiers.push(vm.addr(TASK_VERIFIER_ONE_KEY));
        taskVerifiers.push(vm.addr(TASK_VERIFIER_TWO_KEY));
        module = new CanonicalIndependentChildVerifierV3(
            address(factory),
            address(participantRegistry),
            address(termsRegistry),
            keccak256(abi.encode(taskVerifiers)),
            2
        );
        parentSolver = new ProfitableChildActor();
        childSolver = new ProfitableChildActor();
        verifierRewardRecipient = new ProfitableChildActor();
        token.mint(address(this), 20_000_000);
        token.approve(address(factory), type(uint256).max);
    }

    function testOneUsdcChildLeavesOneUsdcGrossParentProfit() public {
        (AgentBounty parent, AgentBounty child) =
            _prepareSettledChild(PARENT_REWARD, CHILD_TARGET, CHILD_PARTICIPANT, true);

        parentSolver.submit(parent, keccak256("parent-submission"), keccak256("parent-evidence"));
        parent.verifyAndSettle(abi.encode(address(child)));

        require(parent.bountyStatus() == AgentBounty.BountyStatus.Settled, "parent not settled");
        require(child.bountyStatus() == AgentBounty.BountyStatus.Settled, "child not settled");
        require(token.balanceOf(address(childSolver)) == CHILD_TARGET, "child solver payout mismatch");
        require(
            token.balanceOf(address(parentSolver)) == PARENT_REWARD + PARENT_VERIFIER_REWARD,
            "parent balance does not preserve one-USDC gross profit"
        );
        require(
            token.balanceOf(address(parentSolver)) - (CHILD_TARGET + PARENT_VERIFIER_REWARD) == 1_000_000,
            "parent gross profit mismatch"
        );
        require(
            token.balanceOf(address(verifierRewardRecipient)) == PARENT_VERIFIER_REWARD,
            "parent verifier payout mismatch"
        );
    }

    function testChildBelowOneUsdcFloorCannotSettleParent() public {
        (AgentBounty parent, AgentBounty child) =
            _prepareSettledChild(PARENT_REWARD, CHILD_TARGET - 1, CHILD_PARTICIPANT, true);
        parentSolver.submit(parent, keccak256("parent-submission"), keccak256("parent-evidence"));

        (bool ok,) = address(parent).call(abi.encodeCall(parent.verifyAndSettle, (abi.encode(address(child)))));

        require(!ok, "sub-minimum child settled parent");
        _assertParentAttemptPreserved(parent);
    }

    function testChildThatConsumesRequiredMarginCannotSettleParent() public {
        (AgentBounty parent, AgentBounty child) =
            _prepareSettledChild(PARENT_REWARD, CHILD_TARGET + 1, CHILD_PARTICIPANT, true);
        parentSolver.submit(parent, keccak256("parent-submission"), keccak256("parent-evidence"));

        (bool ok,) = address(parent).call(abi.encodeCall(parent.verifyAndSettle, (abi.encode(address(child)))));

        require(!ok, "unprofitable child settled parent");
        _assertParentAttemptPreserved(parent);
    }

    function testParentBelowTwoUsdcCannotUsePolicy() public {
        (AgentBounty parent, AgentBounty child) =
            _prepareSettledChild(PARENT_REWARD - 1, CHILD_TARGET, CHILD_PARTICIPANT, true);
        parentSolver.submit(parent, keccak256("parent-submission"), keccak256("parent-evidence"));

        (bool ok,) = address(parent).call(abi.encodeCall(parent.verifyAndSettle, (abi.encode(address(child)))));

        require(!ok, "underfunded parent settled");
        _assertParentAttemptPreserved(parent);
    }

    function testSameParticipantCannotSettleParent() public {
        (AgentBounty parent, AgentBounty child) =
            _prepareSettledChild(PARENT_REWARD, CHILD_TARGET, PARENT_PARTICIPANT, true);
        parentSolver.submit(parent, keccak256("parent-submission"), keccak256("parent-evidence"));

        (bool ok,) = address(parent).call(abi.encodeCall(parent.verifyAndSettle, (abi.encode(address(child)))));

        require(!ok, "same participant settled parent");
        _assertParentAttemptPreserved(parent);
    }

    function testTermsPublishedAfterClaimCannotSettleParent() public {
        (AgentBounty parent, AgentBounty child) =
            _prepareSettledChild(PARENT_REWARD, CHILD_TARGET, CHILD_PARTICIPANT, false);
        parentSolver.submit(parent, keccak256("parent-submission"), keccak256("parent-evidence"));

        (bool ok,) = address(parent).call(abi.encodeCall(parent.verifyAndSettle, (abi.encode(address(child)))));

        require(!ok, "late terms settled parent");
        _assertParentAttemptPreserved(parent);
    }

    function testBundleDeploysExactProfitPolicyAtomically() public {
        StandingMetaV3Bundle bundle = new StandingMetaV3Bundle(
            address(factory),
            vm.addr(ELIGIBILITY_ATTESTER_KEY),
            vm.addr(TASK_VERIFIER_ONE_KEY),
            vm.addr(TASK_VERIFIER_TWO_KEY)
        );
        CanonicalIndependentChildVerifierV3 bundledModule = bundle.verifierModule();

        require(bundle.canonicalFactory() == address(factory), "bundle factory mismatch");
        require(bundledModule.canonicalFactory() == address(factory), "module factory mismatch");
        require(
            address(bundledModule.participantRegistry()) == address(bundle.participantRegistry()),
            "bundle participant registry mismatch"
        );
        require(address(bundledModule.termsRegistry()) == address(bundle.termsRegistry()), "bundle terms mismatch");
        require(bundledModule.taskVerifierSetHash() == keccak256(abi.encode(taskVerifiers)), "bundle quorum mismatch");
        require(bundledModule.taskVerifierThreshold() == 2, "bundle threshold mismatch");
        require(bundledModule.MINIMUM_CHILD_TARGET() == CHILD_TARGET, "child floor mismatch");
        require(bundledModule.MINIMUM_PARENT_GROSS_MARGIN() == 1_000_000, "profit floor mismatch");
    }

    function _prepareSettledChild(
        uint256 parentReward,
        uint256 childTarget,
        bytes32 childParticipant,
        bool publishBeforeClaim
    ) private returns (AgentBounty parent, AgentBounty child) {
        parent = _createParent(parentReward);
        _register(address(parentSolver), PARENT_PARTICIPANT);
        _register(address(childSolver), childParticipant);
        if (publishBeforeClaim) {
            _publishChildTerms(parent);
            vm.warp(block.timestamp + 1);
        }

        token.mint(address(parentSolver), childTarget + PARENT_VERIFIER_REWARD);
        parentSolver.approve(token, address(parent), PARENT_VERIFIER_REWARD);
        parentSolver.claim(parent);

        if (!publishBeforeClaim) {
            vm.warp(block.timestamp + 1);
            _publishChildTerms(parent);
        }
        parentSolver.approve(token, address(factory), childTarget);
        child = parentSolver.create(factory, _childParams(childTarget), taskVerifiers, childTarget, _nextNonce());

        token.mint(address(childSolver), CHILD_VERIFIER_REWARD);
        childSolver.approve(token, address(child), CHILD_VERIFIER_REWARD);
        childSolver.claim(child);
        childSolver.submit(child, keccak256("child-submission"), keccak256("child-evidence"));
        _settleChild(child);
    }

    function _createParent(uint256 parentReward) private returns (AgentBounty parent) {
        AgentBountyFactory.CreateBountyParams memory params = AgentBountyFactory.CreateBountyParams({
            solverReward: parentReward,
            verifierReward: PARENT_VERIFIER_REWARD,
            termsHash: keccak256(abi.encode("parent-terms", parentReward)),
            policyHash: keccak256("parent-policy"),
            acceptanceCriteriaHash: module.ACCEPTANCE_CRITERIA_HASH(),
            benchmarkHash: keccak256("parent-benchmark"),
            evidenceSchemaHash: keccak256("parent-evidence-schema"),
            fundingDeadline: uint64(block.timestamp + 1 days),
            claimWindowSeconds: 1 hours,
            verificationWindowSeconds: 1 days,
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: address(module),
            verifierRewardRecipient: address(verifierRewardRecipient),
            threshold: 1
        });
        (address parentAddress,) =
            factory.createBounty(params, new address[](0), parentReward + PARENT_VERIFIER_REWARD, _nextNonce());
        parent = AgentBounty(parentAddress);
    }

    function _publishChildTerms(AgentBounty parent) private {
        OnchainTermsRegistry.TermsInput memory input = OnchainTermsRegistry.TermsInput({
            parentBountyId: parent.bountyId(),
            parentRound: 1,
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: CHILD_CRITERIA_HASH,
            benchmarkHash: CHILD_BENCHMARK_HASH,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH,
            verifierSetHash: keccak256(abi.encode(taskVerifiers)),
            verifierThreshold: 2
        });
        parentSolver.publish(termsRegistry, CHILD_TERMS, input);
    }

    function _childParams(uint256 childTarget)
        private
        view
        returns (AgentBountyFactory.CreateBountyParams memory)
    {
        return AgentBountyFactory.CreateBountyParams({
            solverReward: childTarget - CHILD_VERIFIER_REWARD,
            verifierReward: CHILD_VERIFIER_REWARD,
            termsHash: keccak256(CHILD_TERMS),
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: CHILD_CRITERIA_HASH,
            benchmarkHash: CHILD_BENCHMARK_HASH,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH,
            fundingDeadline: uint64(block.timestamp + 1 days),
            claimWindowSeconds: 1 hours,
            verificationWindowSeconds: 1 days,
            verificationMode: AgentBounty.VerificationMode.SignedQuorum,
            verifierModule: address(0),
            verifierRewardRecipient: address(0),
            threshold: 2
        });
    }

    function _settleChild(AgentBounty child) private {
        AgentBounty.Attestation[] memory attestations = new AgentBounty.Attestation[](2);
        uint256 deadline = block.timestamp + 1 hours;
        for (uint256 i = 0; i < 2; i++) {
            bytes32 responseHash = keccak256(abi.encode("sandbox-pass", i));
            address verifier = taskVerifiers[i];
            bytes32 digest = child.attestationDigest(verifier, true, responseHash, deadline);
            uint256 key = i == 0 ? TASK_VERIFIER_ONE_KEY : TASK_VERIFIER_TWO_KEY;
            attestations[i] = AgentBounty.Attestation({
                verifier: verifier,
                passed: true,
                responseHash: responseHash,
                deadline: deadline,
                signature: _sign(key, digest)
            });
        }
        child.settleWithAttestations(attestations);
    }

    function _register(address wallet, bytes32 participantId) private {
        uint64 validUntil = uint64(block.timestamp + 14 days);
        uint256 registrationNonce = participantRegistry.nonces(wallet);
        bytes32 digest =
            participantRegistry.attestationDigest(wallet, participantId, SOURCE_HASH, validUntil, registrationNonce);
        participantRegistry.register(
            wallet, participantId, SOURCE_HASH, validUntil, _sign(ELIGIBILITY_ATTESTER_KEY, digest)
        );
    }

    function _sign(uint256 key, bytes32 digest) private returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);
        return abi.encodePacked(r, s, v);
    }

    function _assertParentAttemptPreserved(AgentBounty parent) private view {
        require(parent.bountyStatus() == AgentBounty.BountyStatus.Submitted, "parent state changed");
        require(parent.fundedAmount() == parent.targetAmount(), "parent funding changed");
        require(parent.solver() == address(parentSolver), "parent solver cleared");
        require(parent.activeClaimBond() == PARENT_VERIFIER_REWARD, "parent bond changed");
    }

    function _nextNonce() private returns (bytes32) {
        nonce += 1;
        return bytes32(nonce);
    }
}
