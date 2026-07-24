// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/OpenCompetitionBountyFactoryV1.sol";

interface CompetitionVm {
    function roll(uint256) external;
    function warp(uint256) external;
}

contract CompetitionTestToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    mapping(address => mapping(bytes32 => bool)) public authorizationUsed;

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
        require(block.timestamp > validAfter && block.timestamp < validBefore, "authorization timing");
        require(!authorizationUsed[from][nonce], "authorization used");
        require(balanceOf[from] >= amount, "balance");
        authorizationUsed[from][nonce] = true;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
    }
}

contract CompetitionActor {
    function approve(CompetitionTestToken token, address spender, uint256 amount) external {
        token.approve(spender, amount);
    }

    function commit(OpenCompetitionBountyV1 bounty, bytes32 commitment) external {
        bounty.commitSolution(commitment);
    }

    function reveal(
        OpenCompetitionBountyV1 bounty,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 salt,
        bytes calldata proof
    ) external {
        bounty.revealSolution(submissionHash, evidenceHash, salt, proof);
    }

    function withdrawBond(OpenCompetitionBountyV1 bounty) external {
        bounty.withdrawEntryBond();
    }
}

contract CompetitionProofVerifier is IAgentBountyVerifier {
    bytes32 public immutable passingProofHash;

    constructor(bytes32 passingProofHash_) {
        passingProofHash = passingProofHash_;
    }

    function verify(bytes32, uint64 round, address, bytes32, bytes32, bytes32, bytes calldata proof)
        external
        view
        returns (bool passed, bytes32 responseHash)
    {
        responseHash = keccak256(proof);
        passed = round == 1 && responseHash == passingProofHash;
    }
}

contract RetryableCompetitionVerifier is IAgentBountyVerifier {
    function verify(bytes32, uint64, address, bytes32, bytes32, bytes32, bytes calldata proof)
        external
        pure
        returns (bool passed, bytes32 responseHash)
    {
        require(proof.length > 0, "temporary verifier failure");
        return (true, keccak256(proof));
    }
}

contract OpenCompetitionBountyV1Test {
    CompetitionVm constant vm = CompetitionVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    bytes32 constant TERMS_HASH = keccak256("competition-terms-v1");
    bytes32 constant POLICY_HASH = keccak256("competition-policy-v1");
    bytes32 constant CRITERIA_HASH = keccak256("competition-criteria-v1");
    bytes32 constant BENCHMARK_HASH = keccak256("competition-benchmark-v1");
    bytes32 constant EVIDENCE_SCHEMA_HASH = keccak256("competition-evidence-v1");
    bytes32 constant SUBMISSION_A = keccak256("submission-a");
    bytes32 constant SUBMISSION_B = keccak256("submission-b");
    bytes32 constant EVIDENCE_A = keccak256("evidence-a");
    bytes32 constant EVIDENCE_B = keccak256("evidence-b");
    bytes32 constant SALT_A = keccak256("private-salt-a");
    bytes32 constant SALT_B = keccak256("private-salt-b");
    bytes constant PASSING_PROOF = bytes("passing-proof");

    CompetitionTestToken token;
    OpenCompetitionBountyFactoryV1 factory;
    CompetitionActor solverA;
    CompetitionActor solverB;
    CompetitionActor solverC;
    CompetitionActor verifierRecipient;
    uint256 creationNonce;

    function setUp() public {
        token = new CompetitionTestToken();
        factory = new OpenCompetitionBountyFactoryV1(address(token));
        solverA = new CompetitionActor();
        solverB = new CompetitionActor();
        solverC = new CompetitionActor();
        verifierRecipient = new CompetitionActor();
        token.mint(address(this), 1_000_000_000_000);
        token.approve(address(factory), type(uint256).max);
    }

    function testSecondCommitterWinsByFirstPassingRevealAndLoserPullsBond() public {
        CompetitionProofVerifier verifier = new CompetitionProofVerifier(keccak256(PASSING_PROOF));
        OpenCompetitionBountyV1 bounty = _create(verifier, 900, 100, 4, 1_000);
        _commit(solverA, bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, 100);
        _commit(solverB, bounty, SUBMISSION_B, EVIDENCE_B, SALT_B, 100);
        require(bounty.entrants(0) == address(solverA), "first entrant not discoverable");
        require(bounty.entrants(1) == address(solverB), "second entrant not discoverable");
        vm.roll(block.number + 1);

        solverB.reveal(bounty, SUBMISSION_B, EVIDENCE_B, SALT_B, PASSING_PROOF);

        require(bounty.competitionStatus() == OpenCompetitionBountyV1.CompetitionStatus.Settled, "not settled");
        require(bounty.winner() == address(solverB), "wrong winner");
        require(bounty.winningSequence() == 1, "wrong sequence");
        require(token.balanceOf(address(solverB)) == 1_000, "winner payout mismatch");
        require(token.balanceOf(address(verifierRecipient)) == 100, "verifier payout mismatch");
        require(token.balanceOf(address(bounty)) == 100, "loser bond not retained");

        solverA.withdrawBond(bounty);
        require(token.balanceOf(address(solverA)) == 100, "loser bond not returned");
        require(token.balanceOf(address(bounty)) == 0, "contract retained funds");
    }

    function testInvalidRevealConsumesOnlyEntryBondAndLaterValidRevealWins() public {
        CompetitionProofVerifier verifier = new CompetitionProofVerifier(keccak256(PASSING_PROOF));
        OpenCompetitionBountyV1 bounty = _create(verifier, 900, 100, 4, 1_000);
        _commit(solverA, bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, 100);
        _commit(solverB, bounty, SUBMISSION_B, EVIDENCE_B, SALT_B, 100);
        vm.roll(block.number + 1);

        solverA.reveal(bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, bytes("invalid"));
        require(bounty.fundedAmount() == 1_000, "funded target changed");
        require(token.balanceOf(address(bounty)) == 1_100, "failed entry conservation mismatch");
        require(token.balanceOf(address(verifierRecipient)) == 100, "failed verification unpaid");

        solverB.reveal(bounty, SUBMISSION_B, EVIDENCE_B, SALT_B, PASSING_PROOF);
        require(bounty.winner() == address(solverB), "valid solver did not win");
        require(bounty.winningSequence() == 2, "reveal sequence mismatch");
        require(token.balanceOf(address(verifierRecipient)) == 200, "verifier total mismatch");
        require(token.balanceOf(address(bounty)) == 0, "funds remain after settlement");
    }

    function testRevealRequiresLaterBlock() public {
        CompetitionProofVerifier verifier = new CompetitionProofVerifier(keccak256(PASSING_PROOF));
        OpenCompetitionBountyV1 bounty = _create(verifier, 900, 100, 4, 1_000);
        _commit(solverA, bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, 100);

        try solverA.reveal(bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, PASSING_PROOF) {
            revert("same-block reveal succeeded");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256("reveal requires later block"), "wrong rejection");
        }

        vm.roll(block.number + 1);
        solverA.reveal(bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, PASSING_PROOF);
        require(bounty.winner() == address(solverA), "later reveal failed");
    }

    function testCopiedRevealCannotUseAnotherWalletCommitment() public {
        CompetitionProofVerifier verifier = new CompetitionProofVerifier(keccak256(PASSING_PROOF));
        OpenCompetitionBountyV1 bounty = _create(verifier, 900, 100, 4, 1_000);
        _commit(solverA, bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, 100);
        _commit(solverB, bounty, SUBMISSION_B, EVIDENCE_B, SALT_B, 100);
        vm.roll(block.number + 1);

        try solverB.reveal(bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, PASSING_PROOF) {
            revert("copied reveal succeeded");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256("commitment mismatch"), "wrong copy rejection");
        }

        solverA.reveal(bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, PASSING_PROOF);
        require(bounty.winner() == address(solverA), "original solver did not win");
    }

    function testExpiredCommitmentBondBecomesWinnerBonus() public {
        CompetitionProofVerifier verifier = new CompetitionProofVerifier(keccak256(PASSING_PROOF));
        OpenCompetitionBountyV1 bounty = _create(verifier, 900, 100, 4, 1_000);
        _commit(solverA, bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, 100);
        (,, uint64 revealDeadline,,) = bounty.entries(address(solverA));
        vm.warp(uint256(revealDeadline) + 1);
        bounty.expireCommitment(address(solverA));
        _commit(solverB, bounty, SUBMISSION_B, EVIDENCE_B, SALT_B, 100);
        vm.roll(block.number + 1);

        solverB.reveal(bounty, SUBMISSION_B, EVIDENCE_B, SALT_B, PASSING_PROOF);
        require(token.balanceOf(address(solverB)) == 1_100, "timeout bonus missing");
        require(token.balanceOf(address(verifierRecipient)) == 100, "verifier payout mismatch");
        require(token.balanceOf(address(bounty)) == 0, "contract retained bonus funds");
    }

    function testVerifierRevertLeavesCommitmentRetryable() public {
        RetryableCompetitionVerifier verifier = new RetryableCompetitionVerifier();
        OpenCompetitionBountyV1 bounty = _create(verifier, 900, 100, 4, 1_000);
        _commit(solverA, bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, 100);
        vm.roll(block.number + 1);

        try solverA.reveal(bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, bytes("")) {
            revert("reverting verifier succeeded");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256("temporary verifier failure"), "wrong verifier error");
        }
        (,,,, OpenCompetitionBountyV1.EntryState state) = bounty.entries(address(solverA));
        require(state == OpenCompetitionBountyV1.EntryState.Committed, "commitment consumed");
        require(bounty.submissionSequence() == 0, "sequence consumed");

        solverA.reveal(bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, bytes("retry"));
        require(bounty.winner() == address(solverA), "retry did not settle");
    }

    function testEntryCapacityAndOneWalletRule() public {
        CompetitionProofVerifier verifier = new CompetitionProofVerifier(keccak256(PASSING_PROOF));
        OpenCompetitionBountyV1 bounty = _create(verifier, 900, 100, 2, 1_000);
        _commit(solverA, bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, 100);
        _commit(solverB, bounty, SUBMISSION_B, EVIDENCE_B, SALT_B, 100);

        token.mint(address(solverC), 100);
        solverC.approve(token, address(bounty), 100);
        bytes32 commitment = bounty.solutionCommitment(address(solverC), SUBMISSION_A, EVIDENCE_A, SALT_B);
        try solverC.commit(bounty, commitment) {
            revert("capacity exceeded");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256("entry capacity reached"), "wrong capacity error");
        }

        vm.roll(block.number + 1);
        solverA.reveal(bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, bytes("invalid"));
        try solverA.commit(bounty, commitment) {
            revert("wallet reentered");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256("wallet already entered"), "wrong wallet error");
        }
    }

    function testCreatorCannotEnter() public {
        CompetitionProofVerifier verifier = new CompetitionProofVerifier(keccak256(PASSING_PROOF));
        OpenCompetitionBountyV1 bounty = _create(verifier, 900, 100, 4, 1_000);
        token.approve(address(bounty), 100);
        bytes32 commitment = bounty.solutionCommitment(address(this), SUBMISSION_A, EVIDENCE_A, SALT_A);
        try bounty.commitSolution(commitment) {
            revert("creator entered");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256("solver ineligible"), "wrong creator error");
        }
    }

    function testRelayedAuthorizationPostsExactEntryBond() public {
        CompetitionProofVerifier verifier = new CompetitionProofVerifier(keccak256(PASSING_PROOF));
        OpenCompetitionBountyV1 bounty = _create(verifier, 900, 100, 4, 1_000);
        token.mint(address(solverA), 100);
        bytes32 commitment = bounty.solutionCommitment(address(solverA), SUBMISSION_A, EVIDENCE_A, SALT_A);

        bounty.commitSolutionWithAuthorization(
            address(solverA), commitment, 0, type(uint256).max, commitment, 27, bytes32(uint256(1)), bytes32(uint256(2))
        );

        require(token.balanceOf(address(solverA)) == 0, "authorization not consumed");
        require(bounty.lockedBondTotal() == 100, "bond not locked");
    }

    function testRelayerCannotSubstituteCommitmentForAuthorization() public {
        CompetitionProofVerifier verifier = new CompetitionProofVerifier(keccak256(PASSING_PROOF));
        OpenCompetitionBountyV1 bounty = _create(verifier, 900, 100, 4, 1_000);
        token.mint(address(solverA), 100);
        bytes32 commitment = bounty.solutionCommitment(address(solverA), SUBMISSION_A, EVIDENCE_A, SALT_A);

        try bounty.commitSolutionWithAuthorization(
            address(solverA),
            commitment,
            0,
            type(uint256).max,
            keccak256("different-authorization-nonce"),
            27,
            bytes32(uint256(1)),
            bytes32(uint256(2))
        ) {
            revert("relayer substituted commitment");
        } catch Error(string memory reason) {
            require(
                keccak256(bytes(reason)) == keccak256("authorization not commitment-bound"),
                "wrong authorization rejection"
            );
        }
        (,,,, OpenCompetitionBountyV1.EntryState state) = bounty.entries(address(solverA));
        require(state == OpenCompetitionBountyV1.EntryState.None, "entry was recorded");
        require(token.balanceOf(address(solverA)) == 100, "bond was consumed");
    }

    function testExpiredCompetitionRequiresCommitmentExpiryThenRefundsPrincipalAndBonus() public {
        CompetitionProofVerifier verifier = new CompetitionProofVerifier(keccak256(PASSING_PROOF));
        uint256 creatorBefore = token.balanceOf(address(this));
        OpenCompetitionBountyV1 bounty = _create(verifier, 900, 100, 4, 1_000);
        _commit(solverA, bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, 100);
        vm.warp(uint256(bounty.competitionEndsAt()) + 1);

        try bounty.cancelExpiredCompetition() {
            revert("cancelled with locked bond");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256("expire commitments first"), "wrong cancellation error");
        }
        bounty.expireCommitment(address(solverA));
        bounty.cancelExpiredCompetition();
        bounty.withdrawRefund();

        require(token.balanceOf(address(this)) == creatorBefore + 100, "principal and bonus refund mismatch");
        require(token.balanceOf(address(bounty)) == 0, "refund retained funds");
    }

    function testFactoryPredictionAndCanonicalRegistration() public {
        CompetitionProofVerifier verifier = new CompetitionProofVerifier(keccak256(PASSING_PROOF));
        OpenCompetitionBountyFactoryV1.CreateCompetitionParams memory params = _params(verifier, 900, 100, 4);
        bytes32 nonce = keccak256("predicted-competition");
        address predicted = factory.predictCompetitionAddress(address(this), params, nonce);
        (address actual,) = factory.createCompetition(params, 1_000, nonce);

        require(actual == predicted, "prediction mismatch");
        require(factory.isCanonicalCompetition(actual), "canonical registration missing");
        require(_codeSize(actual) < 100, "not a minimal proxy");
    }

    function testFuzzFailedRevealConservesFundedTarget(uint64 rawSolverReward, uint32 rawVerifierReward) public {
        uint256 solverReward = uint256(rawSolverReward % 1_000_000_000) + 1;
        uint256 verifierReward = uint256(rawVerifierReward % 1_000_000) + 1;
        uint256 target = solverReward + verifierReward;
        CompetitionProofVerifier verifier = new CompetitionProofVerifier(keccak256(PASSING_PROOF));
        OpenCompetitionBountyV1 bounty = _create(verifier, solverReward, verifierReward, 4, target);
        _commit(solverA, bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, verifierReward);
        vm.roll(block.number + 1);

        solverA.reveal(bounty, SUBMISSION_A, EVIDENCE_A, SALT_A, bytes("invalid"));

        require(bounty.fundedAmount() == target, "funded accounting changed");
        require(token.balanceOf(address(bounty)) == target, "token target not conserved");
        require(token.balanceOf(address(verifierRecipient)) == verifierReward, "verifier not paid from bond");
    }

    function _create(
        IAgentBountyVerifier verifier,
        uint256 solverReward,
        uint256 verifierReward,
        uint8 maxEntries,
        uint256 initialFunding
    ) private returns (OpenCompetitionBountyV1 bounty) {
        creationNonce += 1;
        OpenCompetitionBountyFactoryV1.CreateCompetitionParams memory params =
            _params(verifier, solverReward, verifierReward, maxEntries);
        (address bountyAddress,) = factory.createCompetition(params, initialFunding, bytes32(creationNonce));
        bounty = OpenCompetitionBountyV1(bountyAddress);
    }

    function _params(IAgentBountyVerifier verifier, uint256 solverReward, uint256 verifierReward, uint8 maxEntries)
        private
        view
        returns (OpenCompetitionBountyFactoryV1.CreateCompetitionParams memory)
    {
        return OpenCompetitionBountyFactoryV1.CreateCompetitionParams({
            solverReward: solverReward,
            verifierReward: verifierReward,
            termsHash: TERMS_HASH,
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: CRITERIA_HASH,
            benchmarkHash: BENCHMARK_HASH,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH,
            fundingDeadline: uint64(block.timestamp + 1 days),
            competitionWindowSeconds: 1 days,
            revealWindowSeconds: 1 hours,
            maxEntries: maxEntries,
            verifierModule: address(verifier),
            verifierRewardRecipient: address(verifierRecipient)
        });
    }

    function _commit(
        CompetitionActor actor,
        OpenCompetitionBountyV1 bounty,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 salt,
        uint256 bond
    ) private {
        token.mint(address(actor), bond);
        actor.approve(token, address(bounty), bond);
        actor.commit(bounty, bounty.solutionCommitment(address(actor), submissionHash, evidenceHash, salt));
    }

    function _codeSize(address account) private view returns (uint256 size) {
        assembly ("memory-safe") {
            size := extcodesize(account)
        }
    }
}
