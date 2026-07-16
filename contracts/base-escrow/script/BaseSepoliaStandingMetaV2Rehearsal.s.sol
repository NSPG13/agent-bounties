// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactory.sol";
import "../src/CanonicalIndependentChildVerifierV2.sol";

interface RehearsalSettlementToken {
    function approve(address spender, uint256 amount) external returns (bool);
    function transfer(address to, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}

interface StandingMetaRehearsalVm {
    function addr(uint256 privateKey) external returns (address);
    function envAddress(string calldata name) external returns (address);
    function envString(string calldata name) external returns (string memory);
    function envUint(string calldata name) external returns (uint256);
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
    function startBroadcast(uint256 privateKey) external;
    function stopBroadcast() external;
    function warp(uint256 timestamp) external;
    function serializeAddress(string calldata objectKey, string calldata valueKey, address value)
        external
        returns (string memory);
    function serializeBytes32(string calldata objectKey, string calldata valueKey, bytes32 value)
        external
        returns (string memory);
    function serializeUint(string calldata objectKey, string calldata valueKey, uint256 value)
        external
        returns (string memory);
    function writeJson(string calldata json, string calldata path) external;
}

/// @notice Executes a real Base Sepolia deployment, funding, claim, submission,
/// verifier quorum, child payout, and parent payout with canonical Base Sepolia
/// USDC. The solver keys are testnet-only and cannot be used on another chain.
contract BaseSepoliaStandingMetaV2Rehearsal {
    StandingMetaRehearsalVm private constant vm =
        StandingMetaRehearsalVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    uint256 private constant BASE_SEPOLIA_CHAIN_ID = 84_532;
    uint256 private constant PARENT_SOLVER_KEY = uint256(keccak256("agent-bounties/base-sepolia-parent-solver-v2"));
    uint256 private constant CHILD_SOLVER_KEY = uint256(keccak256("agent-bounties/base-sepolia-child-solver-v2"));
    bytes32 private constant SOURCE_HASH = keccak256("agent-bounties/base-sepolia-rehearsal-v2");
    bytes32 private constant PARENT_PARTICIPANT = keccak256("base-sepolia-parent-participant-v2");
    bytes32 private constant CHILD_PARTICIPANT = keccak256("base-sepolia-child-participant-v2");
    bytes32 private constant CHILD_POLICY_HASH = keccak256("sandboxed-regression-policy-v1");
    bytes32 private constant CHILD_CRITERIA_HASH = keccak256("base-sepolia-child-criteria-v2");
    bytes32 private constant CHILD_BENCHMARK_HASH = keccak256("base-sepolia-child-benchmark-v2");
    bytes32 private constant CHILD_EVIDENCE_SCHEMA_HASH = keccak256("base-sepolia-child-evidence-v2");
    bytes private constant CHILD_TERMS = '{"schema":"agent-bounties/base-sepolia-child-v2"}';

    struct Deployment {
        RehearsalSettlementToken token;
        AgentBountyFactory factory;
        ParticipantEligibilityRegistry participants;
        OnchainTermsRegistry terms;
        CanonicalIndependentChildVerifierV2 module;
    }

    struct RehearsalActors {
        uint256 deployerKey;
        uint256 attesterKey;
        uint256 verifierOneKey;
        uint256 verifierTwoKey;
        address deployer;
        address attester;
        address parentSolver;
        address childSolver;
        address[] verifiers;
    }

    function run() external {
        require(block.chainid == BASE_SEPOLIA_CHAIN_ID, "Base Sepolia only");
        RehearsalActors memory actors = _loadActors();
        Deployment memory deployment = _loadDeployment(actors.attester, actors.verifiers);
        bytes32 phase = keccak256(bytes(vm.envString("REHEARSAL_PHASE")));
        if (phase == keccak256("prepare")) {
            _fundActors(actors.deployerKey, deployment.token, actors.parentSolver, actors.childSolver);
            _register(
                actors.deployerKey, actors.attesterKey, deployment.participants, actors.parentSolver, PARENT_PARTICIPANT
            );
            _register(
                actors.deployerKey, actors.attesterKey, deployment.participants, actors.childSolver, CHILD_PARTICIPANT
            );
            AgentBounty preparedParent = _createParent(actors.deployerKey, deployment);
            _publishParentTerms(PARENT_SOLVER_KEY, deployment, preparedParent);
            _writePreparationEvidence(
                actors.deployer, deployment, preparedParent, actors.parentSolver, actors.childSolver, actors.verifiers
            );
            return;
        }
        require(phase == keccak256("complete"), "rehearsal phase must be prepare or complete");
        _complete(actors, deployment);
    }

    function _complete(RehearsalActors memory actors, Deployment memory deployment) private {
        AgentBounty parent = AgentBounty(vm.envAddress("REHEARSAL_PARENT_BOUNTY"));
        require(deployment.factory.isCanonicalBounty(address(parent)), "prepared parent is not canonical");
        require(parent.verifierModule() == address(deployment.module), "prepared parent module drift");
        OnchainTermsRegistry.TermsCommitment memory published = deployment.terms.commitment(keccak256(CHILD_TERMS));
        require(published.publisher == actors.parentSolver, "prepared terms publisher drift");
        require(published.publishedAt < block.timestamp, "wait for a later Base timestamp before completion");
        _claimParent(PARENT_SOLVER_KEY, deployment, parent);
        AgentBounty child =
            _createAndClaimChild(PARENT_SOLVER_KEY, CHILD_SOLVER_KEY, deployment, parent, actors.verifiers);
        _submitAndSettleChild(actors, child);
        _submitAndSettleParent(PARENT_SOLVER_KEY, actors.deployerKey, parent, child);

        require(parent.bountyStatus() == AgentBounty.BountyStatus.Settled, "parent not settled");
        require(child.bountyStatus() == AgentBounty.BountyStatus.Settled, "child not settled");
        require(deployment.token.balanceOf(actors.parentSolver) == 1_000_000, "parent net payout mismatch");
        require(deployment.token.balanceOf(actors.childSolver) == 1_000_000, "child net payout mismatch");
        _writeEvidence(
            actors.deployer, deployment, parent, child, actors.parentSolver, actors.childSolver, actors.verifiers
        );
    }

    function _loadActors() private returns (RehearsalActors memory actors) {
        actors.deployerKey = vm.envUint("BASE_KEEPER_PRIVATE_KEY");
        actors.attesterKey = vm.envUint("PARTICIPANT_ATTESTER_PRIVATE_KEY");
        actors.verifierOneKey = vm.envUint("REGRESSION_VERIFIER_ONE_PRIVATE_KEY");
        actors.verifierTwoKey = vm.envUint("REGRESSION_VERIFIER_TWO_PRIVATE_KEY");
        actors.deployer = vm.addr(actors.deployerKey);
        actors.attester = vm.addr(actors.attesterKey);
        address verifierOne = vm.addr(actors.verifierOneKey);
        address verifierTwo = vm.addr(actors.verifierTwoKey);
        require(actors.attester == vm.envAddress("PARTICIPANT_ATTESTER_ADDRESS"), "attester secret mismatch");
        require(verifierOne == vm.envAddress("REGRESSION_VERIFIER_ONE_ADDRESS"), "verifier one secret mismatch");
        require(verifierTwo == vm.envAddress("REGRESSION_VERIFIER_TWO_ADDRESS"), "verifier two secret mismatch");
        actors.parentSolver = vm.addr(PARENT_SOLVER_KEY);
        actors.childSolver = vm.addr(CHILD_SOLVER_KEY);
        actors.verifiers = new address[](2);
        actors.verifiers[0] = verifierOne;
        actors.verifiers[1] = verifierTwo;
    }

    function _loadDeployment(address attester, address[] memory verifiers)
        private
        returns (Deployment memory deployment)
    {
        deployment.token = RehearsalSettlementToken(vm.envAddress("REHEARSAL_TOKEN"));
        deployment.factory = AgentBountyFactory(vm.envAddress("REHEARSAL_FACTORY"));
        deployment.participants = ParticipantEligibilityRegistry(vm.envAddress("REHEARSAL_PARTICIPANT_REGISTRY"));
        deployment.terms = OnchainTermsRegistry(vm.envAddress("REHEARSAL_TERMS_REGISTRY"));
        deployment.module = CanonicalIndependentChildVerifierV2(vm.envAddress("REHEARSAL_VERIFIER_MODULE"));
        require(deployment.factory.settlementToken() == address(deployment.token), "rehearsal factory token drift");
        require(deployment.participants.attester() == attester, "rehearsal attester drift");
        require(deployment.module.canonicalFactory() == address(deployment.factory), "rehearsal module factory drift");
        require(
            address(deployment.module.participantRegistry()) == address(deployment.participants),
            "rehearsal participant registry drift"
        );
        require(
            address(deployment.module.termsRegistry()) == address(deployment.terms), "rehearsal terms registry drift"
        );
        require(
            deployment.module.taskVerifierSetHash() == keccak256(abi.encode(verifiers)), "rehearsal verifier set drift"
        );
        require(deployment.module.taskVerifierThreshold() == 2, "rehearsal verifier threshold drift");
    }

    function _fundActors(uint256 deployerKey, RehearsalSettlementToken token, address parentSolver, address childSolver)
        private
    {
        vm.startBroadcast(deployerKey);
        require(token.transfer(parentSolver, 1_100_000), "parent rehearsal funding failed");
        require(token.transfer(childSolver, 100_000), "child rehearsal funding failed");
        payable(parentSolver).transfer(15_000_000_000_000);
        payable(childSolver).transfer(5_000_000_000_000);
        vm.stopBroadcast();
    }

    function _register(
        uint256 deployerKey,
        uint256 attesterKey,
        ParticipantEligibilityRegistry registry,
        address wallet,
        bytes32 participantId
    ) private {
        uint64 validUntil = uint64(block.timestamp + 14 days);
        bytes32 digest = registry.attestationDigest(wallet, participantId, SOURCE_HASH, validUntil, 0);
        bytes memory signature = _sign(attesterKey, digest);
        vm.startBroadcast(deployerKey);
        registry.register(wallet, participantId, SOURCE_HASH, validUntil, signature);
        vm.stopBroadcast();
    }

    function _createParent(uint256 deployerKey, Deployment memory deployment) private returns (AgentBounty parent) {
        AgentBountyFactory.CreateBountyParams memory params = AgentBountyFactory.CreateBountyParams({
            solverReward: 900_000,
            verifierReward: 100_000,
            termsHash: keccak256("base-sepolia-parent-terms-v2"),
            policyHash: keccak256("base-sepolia-parent-policy-v2"),
            acceptanceCriteriaHash: deployment.module.ACCEPTANCE_CRITERIA_HASH(),
            benchmarkHash: keccak256("base-sepolia-parent-benchmark-v2"),
            evidenceSchemaHash: keccak256("base-sepolia-parent-evidence-v2"),
            fundingDeadline: uint64(block.timestamp + 1 days),
            claimWindowSeconds: 1 hours,
            verificationWindowSeconds: 1 days,
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: address(deployment.module),
            verifierRewardRecipient: vm.addr(deployerKey),
            threshold: 1
        });
        vm.startBroadcast(deployerKey);
        deployment.token.approve(address(deployment.factory), 1_000_000);
        (address parentAddress,) =
            deployment.factory.createBounty(params, new address[](0), 1_000_000, keccak256("base-sepolia-parent-v2"));
        vm.stopBroadcast();
        parent = AgentBounty(parentAddress);
    }

    function _publishParentTerms(uint256 parentKey, Deployment memory deployment, AgentBounty parent) private {
        address[] memory verifiers = new address[](2);
        verifiers[0] = vm.envAddress("REGRESSION_VERIFIER_ONE_ADDRESS");
        verifiers[1] = vm.envAddress("REGRESSION_VERIFIER_TWO_ADDRESS");
        OnchainTermsRegistry.TermsInput memory input = OnchainTermsRegistry.TermsInput({
            parentBountyId: parent.bountyId(),
            parentRound: 1,
            policyHash: CHILD_POLICY_HASH,
            acceptanceCriteriaHash: CHILD_CRITERIA_HASH,
            benchmarkHash: CHILD_BENCHMARK_HASH,
            evidenceSchemaHash: CHILD_EVIDENCE_SCHEMA_HASH,
            verifierSetHash: keccak256(abi.encode(verifiers)),
            verifierThreshold: 2
        });
        vm.startBroadcast(parentKey);
        deployment.terms.publish(CHILD_TERMS, input);
        vm.stopBroadcast();
    }

    function _claimParent(uint256 parentKey, Deployment memory deployment, AgentBounty parent) private {
        vm.startBroadcast(parentKey);
        deployment.token.approve(address(parent), 100_000);
        parent.claim();
        vm.stopBroadcast();
    }

    function _createAndClaimChild(
        uint256 parentKey,
        uint256 childKey,
        Deployment memory deployment,
        AgentBounty parent,
        address[] memory verifiers
    ) private returns (AgentBounty child) {
        AgentBountyFactory.CreateBountyParams memory params = AgentBountyFactory.CreateBountyParams({
            solverReward: 900_000,
            verifierReward: 100_000,
            termsHash: keccak256(CHILD_TERMS),
            policyHash: CHILD_POLICY_HASH,
            acceptanceCriteriaHash: CHILD_CRITERIA_HASH,
            benchmarkHash: CHILD_BENCHMARK_HASH,
            evidenceSchemaHash: CHILD_EVIDENCE_SCHEMA_HASH,
            fundingDeadline: uint64(block.timestamp + 1 days),
            claimWindowSeconds: 1 hours,
            verificationWindowSeconds: 1 days,
            verificationMode: AgentBounty.VerificationMode.SignedQuorum,
            verifierModule: address(0),
            verifierRewardRecipient: address(0),
            threshold: 2
        });
        vm.startBroadcast(parentKey);
        deployment.token.approve(address(deployment.factory), 1_000_000);
        (address childAddress,) = deployment.factory
            .createBounty(params, verifiers, 1_000_000, keccak256(abi.encode("base-sepolia-child-v2", address(parent))));
        vm.stopBroadcast();
        child = AgentBounty(childAddress);
        vm.startBroadcast(childKey);
        deployment.token.approve(address(child), 100_000);
        child.claim();
        vm.stopBroadcast();
    }

    function _submitAndSettleChild(RehearsalActors memory actors, AgentBounty child) private {
        vm.startBroadcast(CHILD_SOLVER_KEY);
        child.submit(keccak256("base-sepolia-child-submission-v2"), keccak256("base-sepolia-child-evidence-v2"));
        vm.stopBroadcast();

        AgentBounty.Attestation[] memory attestations = new AgentBounty.Attestation[](2);
        uint256 deadline = block.timestamp + 1 hours;
        uint256[2] memory keys = [actors.verifierOneKey, actors.verifierTwoKey];
        for (uint256 i = 0; i < 2; i++) {
            bytes32 responseHash = keccak256(abi.encode("base-sepolia-sandbox-pass-v2", i));
            attestations[i] = AgentBounty.Attestation({
                verifier: actors.verifiers[i],
                passed: true,
                responseHash: responseHash,
                deadline: deadline,
                signature: _sign(keys[i], child.attestationDigest(actors.verifiers[i], true, responseHash, deadline))
            });
        }
        vm.startBroadcast(actors.deployerKey);
        child.settleWithAttestations(attestations);
        vm.stopBroadcast();
    }

    function _submitAndSettleParent(uint256 parentKey, uint256 deployerKey, AgentBounty parent, AgentBounty child)
        private
    {
        vm.startBroadcast(parentKey);
        parent.submit(keccak256("base-sepolia-parent-submission-v2"), keccak256("base-sepolia-parent-evidence-v2"));
        vm.stopBroadcast();
        vm.startBroadcast(deployerKey);
        parent.verifyAndSettle(abi.encode(address(child)));
        vm.stopBroadcast();
    }

    function _writeEvidence(
        address deployer,
        Deployment memory deployment,
        AgentBounty parent,
        AgentBounty child,
        address parentSolver,
        address childSolver,
        address[] memory verifiers
    ) private {
        string memory objectKey = "base-sepolia-standing-meta-v2-rehearsal";
        vm.serializeUint(objectKey, "chain_id", block.chainid);
        vm.serializeAddress(objectKey, "deployer", deployer);
        vm.serializeAddress(objectKey, "rehearsal_token", address(deployment.token));
        vm.serializeAddress(objectKey, "canonical_factory", address(deployment.factory));
        vm.serializeAddress(objectKey, "participant_registry", address(deployment.participants));
        vm.serializeAddress(objectKey, "terms_registry", address(deployment.terms));
        vm.serializeAddress(objectKey, "verifier_module", address(deployment.module));
        vm.serializeAddress(objectKey, "parent_bounty", address(parent));
        vm.serializeAddress(objectKey, "child_bounty", address(child));
        vm.serializeAddress(objectKey, "parent_solver", parentSolver);
        vm.serializeAddress(objectKey, "child_solver", childSolver);
        vm.serializeAddress(objectKey, "verifier_one", verifiers[0]);
        vm.serializeAddress(objectKey, "verifier_two", verifiers[1]);
        vm.serializeBytes32(objectKey, "parent_bounty_id", parent.bountyId());
        vm.serializeBytes32(objectKey, "child_bounty_id", child.bountyId());
        vm.serializeBytes32(objectKey, "verifier_set_hash", keccak256(abi.encode(verifiers)));
        vm.serializeBytes32(objectKey, "acceptance_criteria_hash", deployment.module.ACCEPTANCE_CRITERIA_HASH());
        vm.serializeUint(objectKey, "parent_status", uint8(parent.bountyStatus()));
        string memory json = vm.serializeUint(objectKey, "child_status", uint8(child.bountyStatus()));
        vm.writeJson(json, vm.envString("REHEARSAL_EVIDENCE_PATH"));
    }

    function _writePreparationEvidence(
        address deployer,
        Deployment memory deployment,
        AgentBounty parent,
        address parentSolver,
        address childSolver,
        address[] memory verifiers
    ) private {
        OnchainTermsRegistry.TermsCommitment memory published = deployment.terms.commitment(keccak256(CHILD_TERMS));
        string memory objectKey = "base-sepolia-standing-meta-v2-preparation";
        vm.serializeUint(objectKey, "chain_id", block.chainid);
        vm.serializeAddress(objectKey, "deployer", deployer);
        vm.serializeAddress(objectKey, "rehearsal_token", address(deployment.token));
        vm.serializeAddress(objectKey, "canonical_factory", address(deployment.factory));
        vm.serializeAddress(objectKey, "participant_registry", address(deployment.participants));
        vm.serializeAddress(objectKey, "terms_registry", address(deployment.terms));
        vm.serializeAddress(objectKey, "verifier_module", address(deployment.module));
        vm.serializeAddress(objectKey, "parent_bounty", address(parent));
        vm.serializeAddress(objectKey, "parent_solver", parentSolver);
        vm.serializeAddress(objectKey, "child_solver", childSolver);
        vm.serializeAddress(objectKey, "verifier_one", verifiers[0]);
        vm.serializeAddress(objectKey, "verifier_two", verifiers[1]);
        vm.serializeBytes32(objectKey, "parent_bounty_id", parent.bountyId());
        vm.serializeBytes32(objectKey, "verifier_set_hash", keccak256(abi.encode(verifiers)));
        vm.serializeUint(objectKey, "terms_published_at", published.publishedAt);
        string memory json = vm.serializeUint(objectKey, "parent_status", uint8(parent.bountyStatus()));
        vm.writeJson(json, vm.envString("REHEARSAL_EVIDENCE_PATH"));
    }

    function _sign(uint256 privateKey, bytes32 digest) private returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(privateKey, digest);
        return abi.encodePacked(r, s, v);
    }
}
