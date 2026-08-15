// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/OpenCompetitionBountyFactoryV2Beta2.sol";
import {ISP1VerifierWithHash} from "../src/ISP1Verifier.sol";

struct VmCompetitionLog {
    bytes32[] topics;
    bytes data;
    address emitter;
}

interface VmCompetitionV2 {
    function addr(uint256 privateKey) external returns (address);
    function etch(address target, bytes calldata code) external;
    function pauseGasMetering() external;
    function prank(address sender) external;
    function resumeGasMetering() external;
    function recordLogs() external;
    function getRecordedLogs() external returns (VmCompetitionLog[] memory);
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
    function warp(uint256 timestamp) external;
}

contract CompetitionV2Token {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    mapping(address => mapping(bytes32 => bool)) public authorizationUsed;
    bool public skipTransferFrom;

    function setSkipTransferFrom(bool value) external {
        skipTransferFrom = value;
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
        require(block.timestamp > validAfter && block.timestamp < validBefore, "authorization timing");
        require(!authorizationUsed[from][nonce], "authorization used");
        require(balanceOf[from] >= amount, "balance");
        authorizationUsed[from][nonce] = true;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
    }
}

abstract contract CompetitionV2Verifier is ISP1VerifierWithHash {
    bytes32 public acceptedProofHash;

    function setAcceptedProof(bytes calldata proof) external {
        acceptedProofHash = keccak256(proof);
    }

    function verifyProof(bytes32 programVKey, bytes calldata publicValues, bytes calldata proofBytes) external view {
        require(programVKey != bytes32(0), "vkey zero");
        require(publicValues.length > 0, "journal empty");
        require(keccak256(proofBytes) == acceptedProofHash, "invalid proof");
    }
}

contract CompetitionV2Groth16Verifier is CompetitionV2Verifier {
    function VERIFIER_HASH() external pure returns (bytes32) {
        return 0x4388a21c00000000000000000000000000000000000000000000000000000001;
    }
}

contract CompetitionV2PlonkVerifier is CompetitionV2Verifier {
    function VERIFIER_HASH() external pure returns (bytes32) {
        return 0x5a093a2f00000000000000000000000000000000000000000000000000000001;
    }
}

contract CompetitionV2Actor {
    function approve(CompetitionV2Token token, address spender, uint256 amount) external {
        token.approve(spender, amount);
    }

    function fund(OpenCompetitionBountyV2Beta2 bounty, uint256 amount, bytes32 riskHash) external {
        bounty.fund(amount, riskHash);
    }

    function submit(OpenCompetitionBountyV2Beta2 bounty, bytes calldata journal, bytes calldata proof) external {
        bounty.submitProof(journal, proof);
    }
}

contract CompetitionV2Signer1271 is IERC1271 {
    bytes32 public acceptedDigest;

    function setAcceptedDigest(bytes32 digest) external {
        acceptedDigest = digest;
    }

    function isValidSignature(bytes32 digest, bytes calldata signature) external view returns (bytes4) {
        return digest == acceptedDigest && signature.length > 0 ? bytes4(0x1626ba7e) : bytes4(0xffffffff);
    }
}

contract CompetitionV2ReturnBombSigner {
    fallback() external {
        assembly ("memory-safe") {
            mstore(0, shl(224, 0x1626ba7e))
            return(0, 65536)
        }
    }
}

contract OpenCompetitionBountyV2Beta2Test {
    VmCompetitionV2 constant vm = VmCompetitionV2(address(uint160(uint256(keccak256("hevm cheat code")))));

    bytes constant GROTH16_PROOF = hex"4388a21c01020304";
    bytes constant PLONK_PROOF = hex"5a093a2f01020304";
    bytes32 constant PROGRAM_VKEY = keccak256("public-vector-metric-v1-vkey");
    bytes32 constant SOURCE_HASH = keccak256("public-vector-metric-v1-source");
    bytes32 constant ELF_HASH = keccak256("public-vector-metric-v1-elf");
    bytes32 constant JOURNAL_SCHEMA_HASH = keccak256("open-competition-v2-journal-schema");
    bytes32 constant METRIC_PROGRAM_HASH = keccak256("public-vector-metric-v1");
    bytes32 constant EXECUTION_POLICY_HASH = keccak256("execution-policy");
    bytes32 constant VERIFICATION_POLICY_HASH = keccak256("verification-policy");
    bytes32 constant SETTLEMENT_POLICY_HASH = keccak256("settlement-policy");
    bytes32 constant BETA_RISK_HASH = keccak256("open-competition-v2-beta2-risk");
    bytes32 constant SUBMISSION_HASH = keccak256("submission");
    bytes32 constant EVIDENCE_HASH = keccak256("evidence");
    uint256 constant SOLVER_REWARD = 1_000_000;
    uint256 constant KEEPER_REWARD = 50_000;

    CompetitionV2Token token;
    CompetitionV2Groth16Verifier groth16Verifier;
    CompetitionV2PlonkVerifier plonkVerifier;
    OpenCompetitionBountyFactoryV2Beta2 factory;
    CompetitionV2Actor solverA;
    CompetitionV2Actor solverB;
    CompetitionV2Actor solverC;
    CompetitionV2Actor funder;
    uint256 creationNonce;

    function setUp() public {
        token = new CompetitionV2Token();
        groth16Verifier = new CompetitionV2Groth16Verifier();
        plonkVerifier = new CompetitionV2PlonkVerifier();
        groth16Verifier.setAcceptedProof(GROTH16_PROOF);
        plonkVerifier.setAcceptedProof(PLONK_PROOF);
        factory = new OpenCompetitionBountyFactoryV2Beta2(
            address(token),
            address(groth16Verifier),
            groth16Verifier.VERIFIER_HASH(),
            address(groth16Verifier).codehash,
            address(plonkVerifier),
            plonkVerifier.VERIFIER_HASH(),
            address(plonkVerifier).codehash
        );
        solverA = new CompetitionV2Actor();
        solverB = new CompetitionV2Actor();
        solverC = new CompetitionV2Actor();
        funder = new CompetitionV2Actor();
        token.mint(address(this), 100_000_000_000);
        token.mint(address(funder), 100_000_000_000);
    }

    function testFirstProvenPaysSolverAndProofSubmitterAtomically() public {
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            10,
            factory.PROOF_SYSTEM_GROTH16()
        );
        bytes memory journal = _journal(bounty, address(solverA), 1, 10, true);

        solverA.submit(bounty, journal, _passingProof(bounty));

        require(bounty.competitionStatus() == OpenCompetitionBountyV2Beta2.CompetitionStatus.Settled, "not settled");
        require(bounty.winner() == address(solverA), "wrong winner");
        require(token.balanceOf(address(solverA)) == SOLVER_REWARD + KEEPER_REWARD, "payout mismatch");
        require(token.balanceOf(address(bounty)) == 0, "escrow retained USDC");
        require(bounty.fundedAmount() == 0, "funded accounting not cleared");
    }

    function testCanonicalCreationEventsPrecedeInitialFunding() public {
        vm.recordLogs();
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        VmCompetitionLog[] memory logs = vm.getRecordedLogs();
        bytes32 createdTopic = keccak256("CanonicalCompetitionCreatedV2(bytes32,address,address,bytes32,bytes32)");
        bytes32 fundingTopic = keccak256("FundingAddedV2(bytes32,address,uint256,uint256,uint256)");
        uint256 createdIndex = type(uint256).max;
        uint256 fundingIndex = type(uint256).max;
        for (uint256 i = 0; i < logs.length; ++i) {
            if (logs[i].topics.length == 0) continue;
            if (logs[i].emitter == address(factory) && logs[i].topics[0] == createdTopic) {
                createdIndex = i;
            }
            if (logs[i].emitter == address(bounty) && logs[i].topics[0] == fundingTopic) {
                fundingIndex = i;
            }
        }
        require(createdIndex != type(uint256).max, "canonical creation event missing");
        require(fundingIndex != type(uint256).max, "initial funding event missing");
        require(createdIndex < fundingIndex, "funding preceded canonical creation");
    }

    function testPooledBestScoreKeepsEarliestTieAndFinalizerReward() public {
        OpenCompetitionBountyV2Beta2 bounty = _createPartiallyFunded(
            400_000,
            OpenCompetitionBountyV2Beta2.WinnerMode.BestScore,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            5,
            factory.PROOF_SYSTEM_PLONK()
        );
        funder.approve(token, address(bounty), 650_000);
        funder.fund(bounty, 650_000, BETA_RISK_HASH);
        require(bounty.contributions(address(this)) == 400_000, "creator pool weight");
        require(bounty.contributions(address(funder)) == 650_000, "funder pool weight");

        solverA.submit(bounty, _journal(bounty, address(solverA), 1, 10, true), _passingProof(bounty));
        solverB.submit(bounty, _journal(bounty, address(solverB), 1, 15, true), _passingProof(bounty));
        solverC.submit(bounty, _journal(bounty, address(solverC), 1, 15, true), _passingProof(bounty));

        require(bounty.leader() == address(solverB), "tie replaced earlier leader");
        require(bounty.leaderSequence() == 2, "leader sequence mismatch");
        vm.warp(uint256(bounty.proofDeadline()) + 1);
        bounty.finalizeBestScore();

        require(token.balanceOf(address(solverB)) == SOLVER_REWARD, "solver reward mismatch");
        require(token.balanceOf(address(this)) == 100_000_000_000 - 400_000 + KEEPER_REWARD, "keeper reward mismatch");
        require(token.balanceOf(address(bounty)) == 0, "settled balance remains");
    }

    function testBestScoreLeaderCannotBeExpiredIntoRefunds() public {
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.BestScore,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        solverA.submit(bounty, _journal(bounty, address(solverA), 1, 1, true), _passingProof(bounty));
        vm.warp(uint256(bounty.proofDeadline()) + 1);

        _requireRevertSelector(
            address(bounty),
            abi.encodeCall(OpenCompetitionBountyV2Beta2.expireCompetition, ()),
            OpenCompetitionBountyV2Beta2.V2LeaderRequiresFinalization.selector
        );
        bounty.finalizeBestScore();
        require(bounty.winner() == address(solverA), "leader not finalized");
    }

    function testLowerScoreModeAndThreshold() public {
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.BestScore,
            OpenCompetitionBountyV2Beta2.ScoreDirection.LowerIsBetter,
            20,
            factory.PROOF_SYSTEM_GROTH16()
        );
        solverA.submit(bounty, _journal(bounty, address(solverA), 1, 20, true), _passingProof(bounty));
        solverB.submit(bounty, _journal(bounty, address(solverB), 1, 7, true), _passingProof(bounty));
        require(bounty.leader() == address(solverB), "lower score did not lead");

        bytes memory rejected = _journal(bounty, address(solverC), 1, 21, true);
        bytes memory callData = abi.encodeCall(CompetitionV2Actor.submit, (bounty, rejected, _passingProof(bounty)));
        _requireRevertSelector(address(solverC), callData, OpenCompetitionBountyV2Beta2.V2ScoreThresholdNotMet.selector);
    }

    function testInvalidProofDoesNotConsumeNonceAndCanRetry() public {
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        bytes memory journal = _journal(bounty, address(solverA), 7, 1, true);
        bytes memory callData = abi.encodeCall(CompetitionV2Actor.submit, (bounty, journal, bytes("bad")));
        _requireRevertSelector(address(solverA), callData, OpenCompetitionBountyV2Beta2.V2Sp1ProofInvalid.selector);
        require(!bounty.usedSolverNonces(address(solverA), 7), "failed proof consumed nonce");

        solverA.submit(bounty, journal, _passingProof(bounty));
        require(bounty.usedSolverNonces(address(solverA), 7), "valid proof did not consume nonce");
    }

    function testNonceReplayRejectedWithoutSecondPayment() public {
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.BestScore,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        bytes memory journal = _journal(bounty, address(solverA), 9, 1, true);
        solverA.submit(bounty, journal, _passingProof(bounty));
        bytes memory callData = abi.encodeCall(CompetitionV2Actor.submit, (bounty, journal, _passingProof(bounty)));
        _requireRevertSelector(address(solverA), callData, OpenCompetitionBountyV2Beta2.V2SolverNonceUsed.selector);
        require(bounty.acceptedSequence() == 1, "replay changed sequence");
    }

    function testJournalCannotReplayAcrossCompetitionOrPolicy() public {
        OpenCompetitionBountyV2Beta2 bountyA = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.BestScore,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        OpenCompetitionBountyV2Beta2 bountyB = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.BestScore,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        bytes memory journalA = _journal(bountyA, address(solverA), 1, 1, true);
        bytes memory callData = abi.encodeCall(CompetitionV2Actor.submit, (bountyB, journalA, _passingProof(bountyB)));
        _requireRevertSelector(address(solverA), callData, OpenCompetitionBountyV2Beta2.V2JournalScopeMismatch.selector);
    }

    function testRelayedEoaAuthorizationBindsJournalAndProof() public {
        uint256 solverKey = 0xA11CE;
        address solver = vm.addr(solverKey);
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        bytes memory journal = _journal(bounty, solver, 11, 3, true);
        uint256 deadline = block.timestamp + 1 hours;
        bytes32 digest =
            bounty.entryAuthorizationDigest(solver, 11, keccak256(journal), keccak256(_passingProof(bounty)), deadline);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(solverKey, digest);
        bytes memory signature = abi.encodePacked(r, s, v);

        bounty.submitProofFor(journal, _passingProof(bounty), deadline, signature);
        require(token.balanceOf(solver) == SOLVER_REWARD, "relayed solver unpaid");
        require(
            token.balanceOf(address(this)) == 100_000_000_000 - KEEPER_REWARD - SOLVER_REWARD + KEEPER_REWARD,
            "relay keeper unpaid"
        );
    }

    function testRelayedErc1271Authorization() public {
        CompetitionV2Signer1271 signer = new CompetitionV2Signer1271();
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_PLONK()
        );
        bytes memory journal = _journal(bounty, address(signer), 1, 3, true);
        uint256 deadline = block.timestamp + 1 hours;
        signer.setAcceptedDigest(
            bounty.entryAuthorizationDigest(
                address(signer), 1, keccak256(journal), keccak256(_passingProof(bounty)), deadline
            )
        );

        bounty.submitProofFor(journal, _passingProof(bounty), deadline, bytes("approve"));
        require(token.balanceOf(address(signer)) == SOLVER_REWARD, "1271 solver unpaid");
    }

    function testErc1271ReturnDataIsCappedAndCannotBombRelay() public {
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        CompetitionV2ReturnBombSigner signer = new CompetitionV2ReturnBombSigner();
        bytes memory journal = _journal(bounty, address(signer), 19, 10, true);
        bytes memory proof = _passingProof(bounty);
        uint256 deadline = block.timestamp + 1 hours;

        (bool ok,) = address(bounty).call(abi.encodeCall(bounty.submitProofFor, (journal, proof, deadline, hex"01")));

        require(ok, "return-bomb authorization failed");
        require(bounty.winner() == address(signer), "return-bomb signer did not settle");
    }

    function testExpiredActiveCompetitionPaysKeeperAndRefundsPoolWithoutContributorAction() public {
        OpenCompetitionBountyV2Beta2 bounty = _createPartiallyFunded(
            300_000,
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        funder.approve(token, address(bounty), 750_000);
        funder.fund(bounty, 750_000, BETA_RISK_HASH);
        vm.warp(uint256(bounty.proofDeadline()) + 1);

        uint256 keeperBefore = token.balanceOf(address(solverC));
        vm.prank(address(solverC));
        bounty.expireCompetition();
        require(token.balanceOf(address(solverC)) == keeperBefore + KEEPER_REWARD, "expiry keeper unpaid");

        uint256 creatorBefore = token.balanceOf(address(this));
        uint256 funderBefore = token.balanceOf(address(funder));
        bounty.withdrawRefundFor(address(this));
        bounty.withdrawRefundFor(address(funder));
        require(token.balanceOf(address(this)) - creatorBefore == 285_714, "creator pro rata refund");
        require(token.balanceOf(address(funder)) - funderBefore == 714_286, "funder final refund");
        require(token.balanceOf(address(bounty)) == 0, "refund dust stranded");
        require(bounty.refundPoolRemaining() == 0 && bounty.refundWeightRemaining() == 0, "refund accounting open");
    }

    function testPartialFundingCancellationRefundsEveryReceivedUnit() public {
        OpenCompetitionBountyV2Beta2 bounty = _createPartiallyFunded(
            123_456,
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        uint256 before = token.balanceOf(address(this));
        bounty.cancelFunding();
        bounty.withdrawRefundFor(address(this));
        require(token.balanceOf(address(this)) == before + 123_456, "partial funding not fully refunded");
        require(token.balanceOf(address(bounty)) == 0, "partial refund stranded");
    }

    function testPartialFundingVerifierLossStopsFundingAndRefundsWithoutKeeperCharge() public {
        OpenCompetitionBountyV2Beta2 bounty = _createPartiallyFunded(
            123_456,
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        vm.etch(address(groth16Verifier), hex"");

        funder.approve(token, address(bounty), 1);
        _requireRevertSelector(
            address(funder),
            abi.encodeCall(CompetitionV2Actor.fund, (bounty, 1, BETA_RISK_HASH)),
            OpenCompetitionBountyV2Beta2.V2VerifierUnavailable.selector
        );

        uint256 keeperBefore = token.balanceOf(address(solverC));
        vm.prank(address(solverC));
        bounty.cancelForUnavailableVerifier();
        require(token.balanceOf(address(solverC)) == keeperBefore, "unfunded keeper charged");

        uint256 creatorBefore = token.balanceOf(address(this));
        bounty.withdrawRefundFor(address(this));
        require(token.balanceOf(address(this)) == creatorBefore + 123_456, "partial verifier refund incomplete");
        require(token.balanceOf(address(bounty)) == 0, "partial verifier refund stranded");
    }

    function testVerifierLossCreatesEarlyPermissionlessRefundPath() public {
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        vm.etch(address(groth16Verifier), hex"");
        vm.prank(address(solverC));
        bounty.cancelForUnavailableVerifier();
        require(
            bounty.competitionStatus() == OpenCompetitionBountyV2Beta2.CompetitionStatus.Cancelled,
            "verifier loss not cancelled"
        );
        require(token.balanceOf(address(solverC)) == KEEPER_REWARD, "verifier keeper unpaid");
        bounty.withdrawRefundFor(address(this));
        require(token.balanceOf(address(bounty)) == 0, "verifier refund stranded");
    }

    function testAdapterRejectsAValidProofForTheOtherPinnedProofSystem() public {
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        bytes memory callData = abi.encodeCall(
            CompetitionV2Actor.submit, (bounty, _journal(bounty, address(solverA), 1, 1, true), PLONK_PROOF)
        );
        _requireRevertSelector(address(solverA), callData, OpenCompetitionBountyV2Beta2.V2Sp1ProofInvalid.selector);
    }

    function testSharedReleaseJournalGoldenVectorMatchesExactly() public pure {
        bytes memory encoded = abi.encode(
            OpenCompetitionBountyV2Beta2.Journal({
                domain: 0x110d7acc5c3397f452c974ba4f7296d7d2a2cede57290113d1fd256e1818804b,
                chainId: 84_532,
                competition: 0x1111111111111111111111111111111111111111,
                bountyId: 0x2222222222222222222222222222222222222222222222222222222222222222,
                solver: 0x3333333333333333333333333333333333333333,
                solverNonce: 7,
                submissionHash: 0x4c57655c451f58ed3b530a6c550a6254e99caaef9f2311b26262205d88fc1744,
                evidenceHash: 0x70efee545f9c7d4bd6964f4fa337f41d492ffeb58ca45a013684c74a28aded92,
                proofSystem: 0x0fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d,
                programVKey: 0x00eef4297c56ab28f02ea278ced640852269cb39f9fd062b3b0def1c11bc3b2b,
                sourceHash: 0x48b61197699dcd2e87f683c1d95f2baca4aeaedf2caaa3e4988dfce10c1152a7,
                elfHash: 0xb2dc353084ecb54a22c4dc637be5d532e59b7c1c24819eee037a410388bb0800,
                journalSchemaHash: 0xd9c492538aa0822e8a1d651886e79a2b8ddfc2c3428b3ed92e19d337eefe77d4,
                metricProgramHash: 0x1c27fc20ab65264c7db2997c8b76f78d7291cdb91243481bcae1e88f77beb88a,
                executionPolicyHash: 0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,
                verificationPolicyHash: 0xc1e5661ee1066b8bf3699a878abf6f42d6ea175a2e80297e859e75b4ded7e2ff,
                settlementPolicyHash: 0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd,
                betaRiskHash: 0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee,
                passed: true,
                score: 4
            })
        );
        require(encoded.length == 640, "journal length drift");
        require(
            keccak256(encoded) == 0x6222a0cd0a9dc3aacc7558151550379bfb8786579bf1f5e9be5cc09a9ddb9e34,
            "shared journal drift"
        );
    }

    function testProofAtExactDeadlineQualifiesAndOneSecondLaterDoesNot() public {
        OpenCompetitionBountyV2Beta2 exact = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        vm.warp(exact.proofDeadline());
        solverA.submit(exact, _journal(exact, address(solverA), 1, 1, true), _passingProof(exact));
        require(exact.winner() == address(solverA), "exact deadline rejected");

        OpenCompetitionBountyV2Beta2 late = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        bytes memory lateJournal = _journal(late, address(solverB), 1, 1, true);
        vm.warp(uint256(late.proofDeadline()) + 1);
        bytes memory callData = abi.encodeCall(CompetitionV2Actor.submit, (late, lateJournal, _passingProof(late)));
        _requireRevertSelector(address(solverB), callData, OpenCompetitionBountyV2Beta2.V2ProofDeadlinePassed.selector);
    }

    function testMalformedJournalAndFailedMetricFailClosed() public {
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.BestScore,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        bytes memory malformedCall =
            abi.encodeCall(CompetitionV2Actor.submit, (bounty, bytes("short"), _passingProof(bounty)));
        _requireRevertSelector(
            address(solverA), malformedCall, OpenCompetitionBountyV2Beta2.V2JournalDecodeInvalid.selector
        );
        bytes memory failedJournal = _journal(bounty, address(solverA), 1, 1, false);
        bytes memory failedCall =
            abi.encodeCall(CompetitionV2Actor.submit, (bounty, failedJournal, _passingProof(bounty)));
        _requireRevertSelector(
            address(solverA), failedCall, OpenCompetitionBountyV2Beta2.V2JournalReportedFailure.selector
        );
    }

    function testFactoryNeverReceivesFundsOrUsesContributorAllowance() public {
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        require(token.balanceOf(address(factory)) == 0, "factory held funds");
        require(token.allowance(address(this), address(factory)) == 0, "factory allowance used");
        require(factory.isCanonicalCompetition(address(bounty)), "competition not canonical");
    }

    function testNoOpTokenFundingFailsAccountingCheck() public {
        OpenCompetitionBountyV2Beta2 bounty = _createPartiallyFunded(
            100_000,
            OpenCompetitionBountyV2Beta2.WinnerMode.FirstProven,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        token.setSkipTransferFrom(true);
        token.approve(address(bounty), 1);
        bytes memory callData = abi.encodeCall(bounty.fund, (1, BETA_RISK_HASH));
        _requireRevertSelector(
            address(bounty), callData, OpenCompetitionBountyV2Beta2.V2TokenAccountingMismatch.selector
        );
        require(bounty.fundedAmount() == 100_000, "failed transfer changed funding");
    }

    function testGasDoesNotGrowAcrossOneHundredOrTenThousandEntries() public {
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.BestScore,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            0,
            factory.PROOF_SYSTEM_GROTH16()
        );
        uint256 gasBefore = gasleft();
        solverA.submit(bounty, _journal(bounty, address(solverA), 1, 1, true), _passingProof(bounty));
        uint256 gasOne = gasBefore - gasleft();

        vm.pauseGasMetering();
        for (uint256 nonce = 2; nonce < 100; nonce++) {
            solverA.submit(bounty, _journal(bounty, address(solverA), nonce, 1, true), _passingProof(bounty));
        }
        vm.resumeGasMetering();
        gasBefore = gasleft();
        solverA.submit(bounty, _journal(bounty, address(solverA), 100, 1, true), _passingProof(bounty));
        uint256 gasHundred = gasBefore - gasleft();

        vm.pauseGasMetering();
        for (uint256 nonce = 101; nonce < 10_000; nonce++) {
            solverA.submit(bounty, _journal(bounty, address(solverA), nonce, 1, true), _passingProof(bounty));
        }
        vm.resumeGasMetering();
        gasBefore = gasleft();
        solverA.submit(bounty, _journal(bounty, address(solverA), 10_000, 1, true), _passingProof(bounty));
        uint256 gasTenThousand = gasBefore - gasleft();

        require(gasHundred <= gasOne * 102 / 100, "100-entry gas grew over 2 percent");
        require(gasTenThousand <= gasOne * 102 / 100, "10000-entry gas grew over 2 percent");
        require(bounty.acceptedSequence() == 10_000, "entry sequence mismatch");
    }

    function testFuzzThresholdIsExact(int128 rawThreshold, int128 rawScore) public {
        int256 threshold = int256(rawThreshold);
        int256 score = int256(rawScore);
        OpenCompetitionBountyV2Beta2 bounty = _createFunded(
            OpenCompetitionBountyV2Beta2.WinnerMode.BestScore,
            OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
            threshold,
            factory.PROOF_SYSTEM_GROTH16()
        );
        require(bounty.scoreMeetsThreshold(score) == (score >= threshold), "threshold predicate drift");
    }

    function _createFunded(
        OpenCompetitionBountyV2Beta2.WinnerMode mode,
        OpenCompetitionBountyV2Beta2.ScoreDirection direction,
        int256 threshold,
        bytes32 proofSystem
    ) private returns (OpenCompetitionBountyV2Beta2) {
        return _createPartiallyFunded(SOLVER_REWARD + KEEPER_REWARD, mode, direction, threshold, proofSystem);
    }

    function _createPartiallyFunded(
        uint256 initialFunding,
        OpenCompetitionBountyV2Beta2.WinnerMode mode,
        OpenCompetitionBountyV2Beta2.ScoreDirection direction,
        int256 threshold,
        bytes32 proofSystem
    ) private returns (OpenCompetitionBountyV2Beta2 bounty) {
        OpenCompetitionBountyFactoryV2Beta2.CreateCompetitionParams memory params =
            _params(mode, direction, threshold, proofSystem);
        bytes32 nonce = bytes32(++creationNonce);
        address predicted = factory.predictCompetitionAddress(address(this), params, nonce);
        token.approve(predicted, initialFunding);
        (address competition,) = factory.createCompetition(params, initialFunding, nonce, BETA_RISK_HASH);
        require(competition == predicted, "prediction mismatch");
        return OpenCompetitionBountyV2Beta2(competition);
    }

    function _params(
        OpenCompetitionBountyV2Beta2.WinnerMode mode,
        OpenCompetitionBountyV2Beta2.ScoreDirection direction,
        int256 threshold,
        bytes32 proofSystem
    ) private view returns (OpenCompetitionBountyFactoryV2Beta2.CreateCompetitionParams memory) {
        return OpenCompetitionBountyFactoryV2Beta2.CreateCompetitionParams({
                solverReward: SOLVER_REWARD,
                keeperReward: KEEPER_REWARD,
                fundingDeadline: uint64(block.timestamp + 7 days),
                proofWindowSeconds: 3 days,
                winnerMode: mode,
                scoreDirection: direction,
                scoreThreshold: threshold,
                proofSystem: proofSystem,
                programVKey: PROGRAM_VKEY,
                sourceHash: SOURCE_HASH,
                elfHash: ELF_HASH,
                journalSchemaHash: JOURNAL_SCHEMA_HASH,
                metricProgramHash: METRIC_PROGRAM_HASH,
                executionPolicyHash: EXECUTION_POLICY_HASH,
                verificationPolicyHash: VERIFICATION_POLICY_HASH,
                settlementPolicyHash: SETTLEMENT_POLICY_HASH,
                betaRiskHash: BETA_RISK_HASH
            });
    }

    function _journal(
        OpenCompetitionBountyV2Beta2 bounty,
        address solver,
        uint256 solverNonce,
        int256 score,
        bool passed
    ) private view returns (bytes memory) {
        return abi.encode(
            OpenCompetitionBountyV2Beta2.Journal({
                domain: bounty.JOURNAL_DOMAIN(),
                chainId: block.chainid,
                competition: address(bounty),
                bountyId: bounty.bountyId(),
                solver: solver,
                solverNonce: solverNonce,
                submissionHash: keccak256(abi.encode(SUBMISSION_HASH, solver, solverNonce)),
                evidenceHash: keccak256(abi.encode(EVIDENCE_HASH, solver, solverNonce)),
                proofSystem: bounty.proofSystem(),
                programVKey: PROGRAM_VKEY,
                sourceHash: SOURCE_HASH,
                elfHash: ELF_HASH,
                journalSchemaHash: JOURNAL_SCHEMA_HASH,
                metricProgramHash: METRIC_PROGRAM_HASH,
                executionPolicyHash: EXECUTION_POLICY_HASH,
                verificationPolicyHash: VERIFICATION_POLICY_HASH,
                settlementPolicyHash: SETTLEMENT_POLICY_HASH,
                betaRiskHash: BETA_RISK_HASH,
                passed: passed,
                score: score
            })
        );
    }

    function _passingProof(OpenCompetitionBountyV2Beta2 bounty) private view returns (bytes memory) {
        return bounty.proofSystem() == factory.PROOF_SYSTEM_GROTH16() ? GROTH16_PROOF : PLONK_PROOF;
    }

    function _requireRevertSelector(address target, bytes memory callData, bytes4 expected) private {
        (bool ok, bytes memory result) = target.call(callData);
        require(!ok, "call unexpectedly succeeded");
        require(result.length >= 4, "missing revert selector");
        bytes4 actual;
        assembly ("memory-safe") {
            actual := mload(add(result, 32))
        }
        require(actual == expected, "wrong revert selector");
    }
}

contract CompetitionV2FundingHandler {
    CompetitionV2Token public immutable token;
    OpenCompetitionBountyV2Beta2 public immutable bounty;
    bytes32 public immutable riskHash;

    constructor(CompetitionV2Token token_, OpenCompetitionBountyV2Beta2 bounty_, bytes32 riskHash_) {
        token = token_;
        bounty = bounty_;
        riskHash = riskHash_;
        token_.approve(address(bounty_), type(uint256).max);
    }

    function fund(uint128 rawAmount) external {
        if (bounty.competitionStatus() != OpenCompetitionBountyV2Beta2.CompetitionStatus.Funding) return;
        uint256 remaining = bounty.targetAmount() - bounty.fundedAmount();
        if (remaining == 0) return;
        uint256 amount = uint256(rawAmount) % remaining + 1;
        bounty.fund(amount, riskHash);
    }
}

contract OpenCompetitionBountyV2Beta2InvariantTest {
    VmCompetitionV2 constant vm = VmCompetitionV2(address(uint160(uint256(keccak256("hevm cheat code")))));
    bytes32 constant RISK_HASH = keccak256("invariant-risk");

    CompetitionV2Token token;
    OpenCompetitionBountyFactoryV2Beta2 factory;
    OpenCompetitionBountyV2Beta2 bounty;
    address[] private invariantTargets;

    function setUp() public {
        token = new CompetitionV2Token();
        CompetitionV2Groth16Verifier groth16 = new CompetitionV2Groth16Verifier();
        CompetitionV2PlonkVerifier plonk = new CompetitionV2PlonkVerifier();
        groth16.setAcceptedProof(hex"4388a21c01020304");
        plonk.setAcceptedProof(hex"5a093a2f01020304");
        factory = new OpenCompetitionBountyFactoryV2Beta2(
            address(token),
            address(groth16),
            groth16.VERIFIER_HASH(),
            address(groth16).codehash,
            address(plonk),
            plonk.VERIFIER_HASH(),
            address(plonk).codehash
        );

        OpenCompetitionBountyFactoryV2Beta2.CreateCompetitionParams memory params =
            OpenCompetitionBountyFactoryV2Beta2.CreateCompetitionParams({
                solverReward: 1_000_000,
                keeperReward: 50_000,
                fundingDeadline: uint64(block.timestamp + 30 days),
                proofWindowSeconds: 3 days,
                winnerMode: OpenCompetitionBountyV2Beta2.WinnerMode.BestScore,
                scoreDirection: OpenCompetitionBountyV2Beta2.ScoreDirection.HigherIsBetter,
                scoreThreshold: 0,
                proofSystem: factory.PROOF_SYSTEM_GROTH16(),
                programVKey: keccak256("vkey"),
                sourceHash: keccak256("source"),
                elfHash: keccak256("elf"),
                journalSchemaHash: keccak256("journal"),
                metricProgramHash: keccak256("metric"),
                executionPolicyHash: keccak256("execution"),
                verificationPolicyHash: keccak256("verification"),
                settlementPolicyHash: keccak256("settlement"),
                betaRiskHash: RISK_HASH
            });
        (address competition,) = factory.createCompetition(params, 0, keccak256("invariant"), RISK_HASH);
        bounty = OpenCompetitionBountyV2Beta2(competition);
        CompetitionV2FundingHandler handler = new CompetitionV2FundingHandler(token, bounty, RISK_HASH);
        token.mint(address(handler), 100_000_000);
        invariantTargets.push(address(handler));
    }

    function targetContracts() external view returns (address[] memory) {
        return invariantTargets;
    }

    function invariantEscrowBalanceCoversRecordedFunding() public view {
        require(token.balanceOf(address(bounty)) == bounty.fundedAmount(), "escrow accounting drift");
        require(bounty.fundedAmount() <= bounty.targetAmount(), "overfunded");
    }

    function invariantActivationRequiresExactCoverage() public view {
        if (bounty.competitionStatus() == OpenCompetitionBountyV2Beta2.CompetitionStatus.Active) {
            require(bounty.fundedAmount() == bounty.targetAmount(), "active underfunded");
            require(bounty.proofDeadline() > block.timestamp, "active deadline missing");
        }
    }

    function invariantFactoryNeverCustodiesCompetitionFunds() public view {
        require(token.balanceOf(address(factory)) == 0, "factory custody detected");
    }
}
