// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactory.sol";
import "../src/LeadingZeroWorkVerifier.sol";

interface Vm {
    function warp(uint256) external;
}

contract ProtocolTestToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    mapping(address => mapping(bytes32 => bool)) public authorizationUsed;
    bool public skipTransferFrom;
    bool public skipAuthorizationTransfer;

    function setNoOpTransfers(bool transferFrom_, bool authorization_) external {
        skipTransferFrom = transferFrom_;
        skipAuthorizationTransfer = authorization_;
    }

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
        if (skipTransferFrom) return true;
        require(balanceOf[from] >= amount, "balance");
        require(allowance[from][msg.sender] >= amount, "allowance");
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferWithAuthorization(
        address from,
        address to,
        uint256 amount,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8,
        bytes32,
        bytes32
    ) external {
        if (skipAuthorizationTransfer) return;
        require(block.timestamp > validAfter, "authorization not active");
        require(block.timestamp < validBefore, "authorization expired");
        require(!authorizationUsed[from][nonce], "authorization used");
        require(balanceOf[from] >= amount, "balance");
        authorizationUsed[from][nonce] = true;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
    }
}

contract BountyActor {
    function approve(ProtocolTestToken token, address spender, uint256 amount) external {
        token.approve(spender, amount);
    }

    function fund(AgentBounty bounty, uint256 amount) external returns (uint256) {
        return bounty.fund(amount);
    }

    function claim(AgentBounty bounty) external {
        bounty.claim();
    }

    function submit(AgentBounty bounty, bytes32 submissionHash, bytes32 evidenceHash) external {
        bounty.submit(submissionHash, evidenceHash);
    }

    function withdrawRefund(AgentBounty bounty) external {
        bounty.withdrawRefund();
    }
}

contract Mock1271Signer is IERC1271 {
    bytes32 public expectedDigest;

    function setExpectedDigest(bytes32 digest) external {
        expectedDigest = digest;
    }

    function approveToken(ProtocolTestToken token, address spender, uint256 amount) external {
        token.approve(spender, amount);
    }

    function isValidSignature(bytes32 digest, bytes calldata signature) external view returns (bytes4) {
        if (digest == expectedDigest && signature.length > 0) return 0x1626ba7e;
        return 0xffffffff;
    }
}

contract ProofHashVerifier is IAgentBountyVerifier {
    bytes32 public immutable expectedProofHash;

    constructor(bytes32 expectedProofHash_) {
        expectedProofHash = expectedProofHash_;
    }

    function verify(bytes32, uint64, address, bytes32, bytes32, bytes32, bytes calldata proof)
        external
        view
        returns (bool passed, bytes32 responseHash)
    {
        bytes32 actual = keccak256(proof);
        return (actual == expectedProofHash, actual);
    }
}

contract AgentBountyProtocolTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    ProtocolTestToken token;
    AgentBountyFactory factory;
    BountyActor solver;
    BountyActor verifierRecipient;
    uint256 creationNonceCounter;

    bytes32 constant TERMS_HASH = keccak256("terms-v1");
    bytes32 constant POLICY_HASH = keccak256("policy-v1");
    bytes32 constant CRITERIA_HASH = keccak256("acceptance-criteria-v1");
    bytes32 constant BENCHMARK_HASH = keccak256("benchmark-v1");
    bytes32 constant EVIDENCE_SCHEMA_HASH = keccak256("evidence-schema-v1");
    bytes32 constant SUBMISSION_HASH = keccak256("artifact");
    bytes32 constant EVIDENCE_HASH = keccak256("evidence-package");

    function setUp() public {
        token = new ProtocolTestToken();
        factory = new AgentBountyFactory(address(token));
        solver = new BountyActor();
        verifierRecipient = new BountyActor();
        token.mint(address(this), 100_000);
        token.approve(address(factory), type(uint256).max);
    }

    function testCreateFullyFundedBountyIsImmediatelyClaimable() public {
        ProofHashVerifier module = new ProofHashVerifier(keccak256("proof"));
        AgentBounty bounty = _createDeterministic(module, 1_000);

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Claimable, "not claimable");
        require(bounty.fundedAmount() == 1_000, "funding mismatch");
        require(bounty.contributions(address(this)) == 1_000, "creator contribution missing");
        require(factory.isCanonicalBounty(address(bounty)), "not canonical");
        require(_codeSize(address(bounty)) < 100, "bounty is not a minimal proxy");
    }

    function testLeadingZeroWorkProofCompletesFullPaidLoop() public {
        LeadingZeroWorkVerifier module = new LeadingZeroWorkVerifier(8);
        AgentBounty bounty = _createDeterministic(module, 1_000);
        _prepareSolverBond(bounty, 100);

        solver.claim(bounty);
        solver.submit(bounty, SUBMISSION_HASH, EVIDENCE_HASH);

        uint256 nonce;
        bytes32 responseHash;
        do {
            responseHash = module.workHash(
                bounty.bountyId(),
                bounty.round(),
                address(solver),
                SUBMISSION_HASH,
                EVIDENCE_HASH,
                POLICY_HASH,
                nonce
            );
            nonce += 1;
        } while (uint256(responseHash) >> 248 != 0);

        bounty.verifyAndSettle(abi.encode(nonce - 1));

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Settled, "not settled");
        require(token.balanceOf(address(solver)) == 1_000, "solver payout mismatch");
        require(token.balanceOf(address(verifierRecipient)) == 100, "verifier payout mismatch");
        require(token.balanceOf(address(bounty)) == 0, "bounty retained funds");
    }

    function testLeadingZeroWorkVerifierRejectsMalformedProof() public {
        LeadingZeroWorkVerifier module = new LeadingZeroWorkVerifier(8);
        (bool passed,) = module.verify(
            bytes32(uint256(1)),
            1,
            address(solver),
            SUBMISSION_HASH,
            EVIDENCE_HASH,
            POLICY_HASH,
            hex"01"
        );
        require(!passed, "malformed proof passed");
    }

    function testCreatorCannotClaimOwnBounty() public {
        ProofHashVerifier module = new ProofHashVerifier(keccak256("proof"));
        AgentBounty bounty = _createDeterministic(module, 1_000);

        try bounty.claim() {
            revert("creator claimed own bounty");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256("creator cannot solve"), "wrong claim rejection");
        }
        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Claimable, "status changed");
        require(bounty.solver() == address(0), "solver recorded");
    }

    function testPredictedAddressAndRelayedUsdcAuthorizationCreateFundedBounty() public {
        address creator = address(0xA11CE);
        token.mint(creator, 1_000);
        bytes memory proof = bytes("proof");
        ProofHashVerifier module = new ProofHashVerifier(keccak256(proof));
        AgentBountyFactory.CreateBountyParams memory params = _deterministicParams(module);
        address[] memory noVerifiers = new address[](0);
        bytes32 creationNonce = keccak256("relayed-create");
        address predicted = factory.predictBountyAddress(creator, params, noVerifiers, creationNonce);
        AgentBountyFactory.FundingAuthorization memory authorization = AgentBountyFactory.FundingAuthorization({
            validAfter: 0,
            validBefore: type(uint256).max,
            nonce: keccak256("usdc-authorization"),
            v: 27,
            r: bytes32(uint256(1)),
            s: bytes32(uint256(2))
        });

        (address bountyAddress,) =
            factory.createBountyWithAuthorization(creator, params, noVerifiers, 1_000, creationNonce, authorization);

        require(bountyAddress == predicted, "prediction mismatch");
        require(AgentBounty(bountyAddress).creator() == creator, "creator mismatch");
        require(AgentBounty(bountyAddress).fundedAmount() == 1_000, "funding mismatch");
        require(token.balanceOf(creator) == 0, "creator balance remains");
    }

    function testZeroFundingAndPermissionlessPoolingReachTarget() public {
        ProofHashVerifier module = new ProofHashVerifier(keccak256("proof"));
        AgentBounty bounty = _createDeterministic(module, 0);
        BountyActor funderA = new BountyActor();
        BountyActor funderB = new BountyActor();
        token.mint(address(funderA), 400);
        token.mint(address(funderB), 1_000);
        funderA.approve(token, address(bounty), 400);
        funderB.approve(token, address(bounty), 1_000);

        require(funderA.fund(bounty, 400) == 400, "first contribution");
        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Open, "premature claimable");
        require(funderB.fund(bounty, 1_000) == 600, "remaining cap");

        require(bounty.fundedAmount() == 1_000, "pooled total");
        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Claimable, "not claimable");
        require(token.balanceOf(address(funderB)) == 400, "excess should remain with funder");
    }

    function testDeterministicProofSettlesWithoutOperator() public {
        bytes memory proof = bytes("deterministic-proof");
        ProofHashVerifier module = new ProofHashVerifier(keccak256(proof));
        AgentBounty bounty = _createDeterministic(module, 1_000);

        _prepareSolverBond(bounty, 100);
        solver.claim(bounty);
        solver.submit(bounty, SUBMISSION_HASH, EVIDENCE_HASH);
        bounty.verifyAndSettle(proof);

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Settled, "not settled");
        require(token.balanceOf(address(solver)) == 1_000, "solver reward and bond return missing");
        require(token.balanceOf(address(verifierRecipient)) == 100, "verifier not paid");
        require(token.balanceOf(address(bounty)) == 0, "bounty balance remains");
    }

    function testAiJudgeQuorumSettlesAndPaysIndependentVerifiers() public {
        Mock1271Signer judgeA = new Mock1271Signer();
        Mock1271Signer judgeB = new Mock1271Signer();
        address[] memory judges = new address[](2);
        judges[0] = address(judgeA);
        judges[1] = address(judgeB);
        AgentBounty bounty = _createAiQuorum(judges, 2, 1_000);

        _prepareSolverBond(bounty, 200);
        solver.claim(bounty);
        solver.submit(bounty, SUBMISSION_HASH, EVIDENCE_HASH);
        uint256 deadline = type(uint256).max;
        bytes32 responseA = keccak256("judge-a-response");
        bytes32 responseB = keccak256("judge-b-response");
        judgeA.setExpectedDigest(bounty.attestationDigest(address(judgeA), true, responseA, deadline));
        judgeB.setExpectedDigest(bounty.attestationDigest(address(judgeB), true, responseB, deadline));

        AgentBounty.Attestation[] memory attestations = new AgentBounty.Attestation[](2);
        attestations[0] = AgentBounty.Attestation({
            verifier: address(judgeA), passed: true, responseHash: responseA, deadline: deadline, signature: hex"01"
        });
        attestations[1] = AgentBounty.Attestation({
            verifier: address(judgeB), passed: true, responseHash: responseB, deadline: deadline, signature: hex"02"
        });
        bounty.settleWithAttestations(attestations);

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Settled, "not settled");
        require(token.balanceOf(address(solver)) == 1_000, "solver reward and bond return");
        require(token.balanceOf(address(judgeA)) == 100, "judge a reward");
        require(token.balanceOf(address(judgeB)) == 100, "judge b reward");
    }

    function testAiJudgeSignatureIsBoundToExactResponseAndPolicy() public {
        Mock1271Signer judgeA = new Mock1271Signer();
        Mock1271Signer judgeB = new Mock1271Signer();
        address[] memory judges = new address[](2);
        judges[0] = address(judgeA);
        judges[1] = address(judgeB);
        AgentBounty bounty = _createAiQuorum(judges, 2, 1_000);
        _prepareSolverBond(bounty, 200);
        solver.claim(bounty);
        solver.submit(bounty, SUBMISSION_HASH, EVIDENCE_HASH);

        uint256 deadline = type(uint256).max;
        bytes32 claimedResponse = keccak256("claimed-response");
        bytes32 differentResponse = keccak256("different-response");
        judgeA.setExpectedDigest(bounty.attestationDigest(address(judgeA), true, differentResponse, deadline));
        judgeB.setExpectedDigest(bounty.attestationDigest(address(judgeB), true, claimedResponse, deadline));
        AgentBounty.Attestation[] memory attestations = new AgentBounty.Attestation[](2);
        attestations[0] = AgentBounty.Attestation({
            verifier: address(judgeA),
            passed: true,
            responseHash: claimedResponse,
            deadline: deadline,
            signature: hex"01"
        });
        attestations[1] = AgentBounty.Attestation({
            verifier: address(judgeB),
            passed: true,
            responseHash: claimedResponse,
            deadline: deadline,
            signature: hex"02"
        });

        try bounty.settleWithAttestations(attestations) {
            revert("expected invalid attestation");
        } catch Error(string memory reason) {
            require(_same(reason, "invalid attestation"), "wrong reason");
        }
        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Submitted, "state changed");
    }

    function testRejectQuorumReopensFundedBountyForAnotherSolver() public {
        Mock1271Signer judgeA = new Mock1271Signer();
        Mock1271Signer judgeB = new Mock1271Signer();
        address[] memory judges = new address[](2);
        judges[0] = address(judgeA);
        judges[1] = address(judgeB);
        AgentBounty bounty = _createAiQuorum(judges, 2, 1_000);
        _prepareSolverBond(bounty, 200);
        solver.claim(bounty);
        solver.submit(bounty, SUBMISSION_HASH, EVIDENCE_HASH);

        uint256 deadline = type(uint256).max;
        bytes32 responseA = keccak256("reject-a");
        bytes32 responseB = keccak256("reject-b");
        judgeA.setExpectedDigest(bounty.attestationDigest(address(judgeA), false, responseA, deadline));
        judgeB.setExpectedDigest(bounty.attestationDigest(address(judgeB), false, responseB, deadline));
        AgentBounty.Attestation[] memory attestations = new AgentBounty.Attestation[](2);
        attestations[0] = AgentBounty.Attestation({
            verifier: address(judgeA), passed: false, responseHash: responseA, deadline: deadline, signature: hex"01"
        });
        attestations[1] = AgentBounty.Attestation({
            verifier: address(judgeB), passed: false, responseHash: responseB, deadline: deadline, signature: hex"02"
        });
        bounty.settleWithAttestations(attestations);

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Claimable, "not reopened");
        require(bounty.fundedAmount() == 1_000, "funding changed");
        require(bounty.solver() == address(0), "solver not cleared");
        require(token.balanceOf(address(judgeA)) == 100, "judge a not paid for reject verdict");
        require(token.balanceOf(address(judgeB)) == 100, "judge b not paid for reject verdict");
        require(token.balanceOf(address(bounty)) == 1_000, "rejected bond did not replenish reserve");
    }

    function testRelayedClaimAndSubmissionSupportSmartAccountSignatures() public {
        Mock1271Signer smartSolver = new Mock1271Signer();
        bytes memory proof = bytes("deterministic-proof");
        ProofHashVerifier module = new ProofHashVerifier(keccak256(proof));
        AgentBounty bounty = _createDeterministic(module, 1_000);
        uint256 deadline = type(uint256).max;

        token.mint(address(smartSolver), 100);
        smartSolver.approveToken(token, address(bounty), 100);
        smartSolver.setExpectedDigest(bounty.claimDigest(address(smartSolver), 1, deadline));
        bounty.claimWithSignature(address(smartSolver), deadline, hex"01");
        smartSolver.setExpectedDigest(
            bounty.submitDigest(address(smartSolver), 1, SUBMISSION_HASH, EVIDENCE_HASH, deadline)
        );
        bounty.submitWithSignature(SUBMISSION_HASH, EVIDENCE_HASH, deadline, hex"02");
        bounty.verifyAndSettle(proof);

        require(token.balanceOf(address(smartSolver)) == 1_000, "smart account not paid");
    }

    function testDeterministicRejectPaysVerifierAndPreservesFundedBounty() public {
        ProofHashVerifier module = new ProofHashVerifier(keccak256("valid-proof"));
        AgentBounty bounty = _createDeterministic(module, 1_000);
        _prepareSolverBond(bounty, 100);
        solver.claim(bounty);
        solver.submit(bounty, SUBMISSION_HASH, EVIDENCE_HASH);

        bounty.verifyAndSettle(bytes("invalid-proof"));

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Claimable, "not reopened");
        require(token.balanceOf(address(verifierRecipient)) == 100, "reject verifier not paid");
        require(token.balanceOf(address(bounty)) == 1_000, "reserve not replenished");
        require(bounty.activeClaimBond() == 0, "bond remains active");
    }

    function testExpiredSubmissionReturnsSolverBond() public {
        ProofHashVerifier module = new ProofHashVerifier(keccak256("proof"));
        AgentBounty bounty = _createDeterministic(module, 1_000);
        _prepareSolverBond(bounty, 100);
        solver.claim(bounty);
        solver.submit(bounty, SUBMISSION_HASH, EVIDENCE_HASH);
        vm.warp(uint256(bounty.verificationExpiresAt()) + 1);

        bounty.expireSubmission();

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Claimable, "not reopened");
        require(token.balanceOf(address(solver)) == 100, "bond not refunded");
        require(token.balanceOf(address(bounty)) == 1_000, "bounty principal changed");
    }

    function testExpiredClaimForfeitsBondToNextAcceptedSolver() public {
        bytes memory proof = bytes("proof");
        ProofHashVerifier module = new ProofHashVerifier(keccak256(proof));
        AgentBounty bounty = _createDeterministic(module, 1_000);
        _prepareSolverBond(bounty, 100);
        solver.claim(bounty);
        vm.warp(uint256(bounty.claimExpiresAt()) + 1);

        bounty.expireClaim();

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Claimable, "not reopened");
        require(token.balanceOf(address(solver)) == 0, "inactive solver recovered bond");
        require(bounty.timeoutBondPool() == 100, "timeout bonus missing");
        require(token.balanceOf(address(bounty)) == 1_100, "timeout bond not retained");

        _prepareSolverBond(bounty, 100);
        solver.claim(bounty);
        solver.submit(bounty, SUBMISSION_HASH, EVIDENCE_HASH);
        bounty.verifyAndSettle(proof);

        require(token.balanceOf(address(solver)) == 1_100, "completion bonus not paid");
        require(token.balanceOf(address(verifierRecipient)) == 100, "verifier not paid");
        require(token.balanceOf(address(bounty)) == 0, "settled balance remains");
    }

    function testCancelledBountyRefundsTimeoutBondPoolProRata() public {
        ProofHashVerifier module = new ProofHashVerifier(keccak256("proof"));
        AgentBounty bounty = _createDeterministic(module, 400);
        BountyActor funder = new BountyActor();
        token.mint(address(funder), 600);
        funder.approve(token, address(bounty), 600);
        funder.fund(bounty, 600);
        _prepareSolverBond(bounty, 100);
        solver.claim(bounty);
        vm.warp(uint256(bounty.claimExpiresAt()) + 1);
        bounty.expireClaim();

        bounty.cancel();
        bounty.withdrawRefund();
        funder.withdrawRefund(bounty);

        require(token.balanceOf(address(funder)) == 660, "funder bonus refund mismatch");
        require(token.balanceOf(address(this)) == 100_040, "creator bonus refund mismatch");
        require(bounty.refundBonusRemaining() == 0, "refund bonus dust remains");
        require(token.balanceOf(address(bounty)) == 0, "cancelled balance remains");
    }

    function testRelayedAuthorizationPostsClaimBond() public {
        address authorizedSolver = address(0xB0B);
        ProofHashVerifier module = new ProofHashVerifier(keccak256("proof"));
        AgentBounty bounty = _createDeterministic(module, 1_000);
        token.mint(authorizedSolver, 100);

        bounty.claimWithAuthorization(
            authorizedSolver,
            0,
            type(uint256).max,
            keccak256("claim-bond-authorization"),
            27,
            bytes32(uint256(1)),
            bytes32(uint256(2))
        );

        require(bounty.solver() == authorizedSolver, "authorized solver mismatch");
        require(bounty.activeClaimBond() == 100, "authorized bond missing");
        require(token.balanceOf(address(bounty)) == 1_100, "bond not transferred");
    }

    function testCancelledPooledBountyUsesPullRefunds() public {
        ProofHashVerifier module = new ProofHashVerifier(keccak256("proof"));
        AgentBounty bounty = _createDeterministic(module, 400);
        BountyActor funder = new BountyActor();
        token.mint(address(funder), 300);
        funder.approve(token, address(bounty), 300);
        funder.fund(bounty, 300);

        bounty.cancel();
        bounty.withdrawRefund();
        funder.withdrawRefund(bounty);

        require(token.balanceOf(address(funder)) == 300, "funder refund");
        require(bounty.fundedAmount() == 0, "refund balance remains");
        require(token.balanceOf(address(bounty)) == 0, "token remains");
    }

    function testAiJudgeModeRequiresAtLeastTwoSigners() public {
        Mock1271Signer judge = new Mock1271Signer();
        address[] memory judges = new address[](1);
        judges[0] = address(judge);
        AgentBountyFactory.CreateBountyParams memory params = _aiParams(1);

        try factory.createBounty(params, judges, 0, _nextNonce()) {
            revert("expected quorum rejection");
        } catch Error(string memory reason) {
            require(_same(reason, "ai quorum too small"), "wrong reason");
        }
    }

    function testVerifierRewardAndClaimBondMustBePositive() public {
        ProofHashVerifier module = new ProofHashVerifier(keccak256("proof"));
        AgentBountyFactory.CreateBountyParams memory params = _deterministicParams(module);
        params.verifierReward = 0;
        address[] memory noVerifiers = new address[](0);

        (bool success,) = address(factory)
            .call(abi.encodeCall(AgentBountyFactory.createBounty, (params, noVerifiers, 0, _nextNonce())));

        require(!success, "zero verifier reward accepted");
    }

    function testCanonicalDeadlinesAndWorkWindowsAreBounded() public {
        ProofHashVerifier module = new ProofHashVerifier(keccak256("proof"));
        AgentBountyFactory.CreateBountyParams memory params = _deterministicParams(module);
        address[] memory noVerifiers = new address[](0);

        params.fundingDeadline = uint64(block.timestamp + 366 days + 1);
        (bool fundingSuccess,) = address(factory)
            .call(abi.encodeCall(AgentBountyFactory.createBounty, (params, noVerifiers, 0, _nextNonce())));
        require(!fundingSuccess, "unbounded funding deadline accepted");

        params = _deterministicParams(module);
        params.claimWindowSeconds = 30 days + 1;
        (bool claimSuccess,) = address(factory)
            .call(abi.encodeCall(AgentBountyFactory.createBounty, (params, noVerifiers, 0, _nextNonce())));
        require(!claimSuccess, "unbounded claim window accepted");

        params = _deterministicParams(module);
        params.verificationWindowSeconds = 30 days + 1;
        (bool verificationSuccess,) = address(factory)
            .call(abi.encodeCall(AgentBountyFactory.createBounty, (params, noVerifiers, 0, _nextNonce())));
        require(!verificationSuccess, "unbounded verification window accepted");
    }

    function testCanonicalTargetMustFitIndexerAmountRange() public {
        ProofHashVerifier module = new ProofHashVerifier(keccak256("proof"));
        AgentBountyFactory.CreateBountyParams memory params = _deterministicParams(module);
        params.solverReward = type(uint64).max;
        params.verifierReward = 1;
        address[] memory noVerifiers = new address[](0);

        (bool success,) = address(factory)
            .call(abi.encodeCall(AgentBountyFactory.createBounty, (params, noVerifiers, 0, _nextNonce())));

        require(!success, "oversized target accepted");
    }

    function testFundingRevertsWhenTokenReturnsSuccessWithoutTransfer() public {
        ProofHashVerifier module = new ProofHashVerifier(keccak256("proof"));
        AgentBounty bounty = _createDeterministic(module, 0);
        BountyActor funder = new BountyActor();
        token.mint(address(funder), 1_000);
        funder.approve(token, address(bounty), 1_000);
        token.setNoOpTransfers(true, false);

        (bool success,) = address(funder).call(abi.encodeCall(BountyActor.fund, (bounty, 1_000)));

        require(!success, "phantom funding accepted");
        require(bounty.fundedAmount() == 0, "funding state not rolled back");
        require(bounty.contributions(address(funder)) == 0, "phantom contribution recorded");
        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Open, "phantom claimability");
    }

    function testAuthorizedFundingRevertsWhenTokenDoesNotTransfer() public {
        ProofHashVerifier module = new ProofHashVerifier(keccak256("proof"));
        AgentBounty bounty = _createDeterministic(module, 0);
        token.setNoOpTransfers(false, true);

        (bool success,) = address(bounty)
            .call(
                abi.encodeCall(
                    AgentBounty.fundWithAuthorization,
                    (
                        address(this),
                        1_000,
                        0,
                        type(uint256).max,
                        keccak256("phantom-funding"),
                        27,
                        bytes32(uint256(1)),
                        bytes32(uint256(2))
                    )
                )
            );

        require(!success, "phantom authorized funding accepted");
        require(bounty.fundedAmount() == 0, "authorized funding state not rolled back");
    }

    function testClaimRevertsWhenTokenReturnsSuccessWithoutBond() public {
        ProofHashVerifier module = new ProofHashVerifier(keccak256("proof"));
        AgentBounty bounty = _createDeterministic(module, 1_000);
        _prepareSolverBond(bounty, 100);
        token.setNoOpTransfers(true, false);

        (bool success,) = address(solver).call(abi.encodeCall(BountyActor.claim, (bounty)));

        require(!success, "phantom claim bond accepted");
        require(bounty.activeClaimBond() == 0, "claim bond state not rolled back");
        require(bounty.round() == 0, "claim round not rolled back");
        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Claimable, "claim state not rolled back");
    }

    function testCompatibleExternalBountyCanBeSubmittedButIsNotCanonical() public {
        bytes memory proof = bytes("proof");
        ProofHashVerifier module = new ProofHashVerifier(keccak256(proof));
        AgentBountyFactory externalFactory = new AgentBountyFactory(address(token));
        AgentBountyFactory.CreateBountyParams memory params = _deterministicParams(module);
        (address externalBountyAddress,) =
            externalFactory.createBounty(params, new address[](0), 0, keccak256("external-bounty"));
        AgentBounty externalBounty = AgentBounty(externalBountyAddress);

        factory.submitExternalBounty(address(externalBounty));

        require(factory.isSubmittedExternalBounty(address(externalBounty)), "not submitted");
        require(!factory.isCanonicalBounty(address(externalBounty)), "external marked canonical");
    }

    function _createDeterministic(IAgentBountyVerifier module, uint256 initialFunding)
        private
        returns (AgentBounty bounty)
    {
        AgentBountyFactory.CreateBountyParams memory params = _deterministicParams(module);
        (address bountyAddress,) = factory.createBounty(params, new address[](0), initialFunding, _nextNonce());
        bounty = AgentBounty(bountyAddress);
    }

    function _prepareSolverBond(AgentBounty bounty, uint256 amount) private {
        token.mint(address(solver), amount);
        solver.approve(token, address(bounty), amount);
    }

    function _deterministicParams(IAgentBountyVerifier module)
        private
        view
        returns (AgentBountyFactory.CreateBountyParams memory)
    {
        return AgentBountyFactory.CreateBountyParams({
            solverReward: 900,
            verifierReward: 100,
            termsHash: TERMS_HASH,
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: CRITERIA_HASH,
            benchmarkHash: BENCHMARK_HASH,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH,
            fundingDeadline: uint64(block.timestamp + 1 days),
            claimWindowSeconds: 1 hours,
            verificationWindowSeconds: 1 hours,
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: address(module),
            verifierRewardRecipient: address(verifierRecipient),
            threshold: 1
        });
    }

    function _createAiQuorum(address[] memory judges, uint8 threshold, uint256 initialFunding)
        private
        returns (AgentBounty bounty)
    {
        AgentBountyFactory.CreateBountyParams memory params = _aiParams(threshold);
        (address bountyAddress,) = factory.createBounty(params, judges, initialFunding, _nextNonce());
        bounty = AgentBounty(bountyAddress);
    }

    function _aiParams(uint8 threshold) private view returns (AgentBountyFactory.CreateBountyParams memory) {
        return AgentBountyFactory.CreateBountyParams({
            solverReward: 800,
            verifierReward: 200,
            termsHash: TERMS_HASH,
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: CRITERIA_HASH,
            benchmarkHash: BENCHMARK_HASH,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH,
            fundingDeadline: uint64(block.timestamp + 1 days),
            claimWindowSeconds: 1 hours,
            verificationWindowSeconds: 1 hours,
            verificationMode: AgentBounty.VerificationMode.AiJudgeQuorum,
            verifierModule: address(0),
            verifierRewardRecipient: address(0),
            threshold: threshold
        });
    }

    function _same(string memory left, string memory right) private pure returns (bool) {
        return keccak256(bytes(left)) == keccak256(bytes(right));
    }

    function _nextNonce() private returns (bytes32) {
        creationNonceCounter += 1;
        return bytes32(creationNonceCounter);
    }

    function _codeSize(address account) private view returns (uint256 size) {
        assembly {
            size := extcodesize(account)
        }
    }
}
