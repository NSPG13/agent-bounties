// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/OpenCompetitionEntrantWalletFactoryV1.sol";

interface EntrantWalletVm {
    function warp(uint256) external;
    function roll(uint256) external;
    function prank(address) external;
    function addr(uint256 privateKey) external returns (address);
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
    function etch(address target, bytes calldata code) external;
}

contract EntrantWalletToken {
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

contract EntrantWalletNonTransferringToken {
    function balanceOf(address) external pure returns (uint256) {
        return 0;
    }

    function approve(address, uint256) external pure returns (bool) {
        return true;
    }

    function transfer(address, uint256) external pure returns (bool) {
        return true;
    }

    function transferFrom(address, address, uint256) external pure returns (bool) {
        return true;
    }

    function transferWithAuthorization(address, address, uint256, uint256, uint256, bytes32, uint8, bytes32, bytes32)
        external
        pure
    {}
}

contract EntrantWalletVerifier is IAgentBountyVerifier {
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

contract EntrantWalletCreator {
    function approve(EntrantWalletToken token, address spender, uint256 amount) external {
        token.approve(spender, amount);
    }

    function create(
        OpenCompetitionBountyFactoryV1 factory,
        OpenCompetitionBountyFactoryV1.CreateCompetitionParams calldata params,
        uint256 initialFunding,
        bytes32 creationNonce
    ) external returns (address bounty, bytes32 bountyId) {
        return factory.createCompetition(params, initialFunding, creationNonce);
    }
}

contract EntrantWalletOtherSolver {
    function approve(EntrantWalletToken token, address spender, uint256 amount) external {
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
}

contract EntrantWallet1271Delegate is IERC1271 {
    bytes32 private approvedDigest;
    bytes32 private approvedSignatureHash;

    function approve(bytes32 digest, bytes calldata signature) external {
        approvedDigest = digest;
        approvedSignatureHash = keccak256(signature);
    }

    function isValidSignature(bytes32 digest, bytes calldata signature) external view returns (bytes4) {
        return digest == approvedDigest && keccak256(signature) == approvedSignatureHash ? bytes4(0x1626ba7e) : bytes4(0);
    }
}

contract OpenCompetitionEntrantWalletV1Test {
    EntrantWalletVm private constant vm =
        EntrantWalletVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint256 private constant DELEGATE_KEY = 0xA11CE;
    uint256 private constant WRONG_KEY = 0xB0B;
    uint256 private constant SECP256K1_N =
        0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141;
    bytes32 private constant TERMS_HASH = keccak256("entrant-wallet-terms");
    bytes32 private constant VERIFIER_POLICY_HASH = keccak256("entrant-wallet-verifier-policy");
    bytes32 private constant CRITERIA_HASH = keccak256("entrant-wallet-criteria");
    bytes32 private constant BENCHMARK_HASH = keccak256("entrant-wallet-benchmark");
    bytes32 private constant EVIDENCE_SCHEMA_HASH = keccak256("entrant-wallet-evidence-schema");
    bytes32 private constant SUBMISSION_HASH = keccak256("entrant-wallet-submission");
    bytes32 private constant OTHER_SUBMISSION_HASH = keccak256("entrant-wallet-other-submission");
    bytes32 private constant EVIDENCE_HASH = keccak256("entrant-wallet-evidence");
    bytes32 private constant OTHER_EVIDENCE_HASH = keccak256("entrant-wallet-other-evidence");
    bytes32 private constant SALT = keccak256("entrant-wallet-private-salt");
    bytes32 private constant OTHER_SALT = keccak256("entrant-wallet-other-private-salt");
    bytes private constant PASSING_PROOF = bytes("entrant-wallet-passing-proof");

    EntrantWalletToken private token;
    OpenCompetitionBountyFactoryV1 private competitionFactory;
    OpenCompetitionEntrantWalletFactoryV1 private walletFactory;
    EntrantWalletVerifier private verifier;
    EntrantWalletCreator private creator;
    OpenCompetitionEntrantWalletV1 private wallet;
    address private delegate;
    uint256 private creationNonce;

    function setUp() public {
        vm.warp(1_800_000_000);
        token = new EntrantWalletToken();
        competitionFactory = new OpenCompetitionBountyFactoryV1(address(token));
        walletFactory = new OpenCompetitionEntrantWalletFactoryV1(address(competitionFactory));
        verifier = new EntrantWalletVerifier(keccak256(PASSING_PROOF));
        creator = new EntrantWalletCreator();
        delegate = vm.addr(DELEGATE_KEY);
        wallet = OpenCompetitionEntrantWalletV1(
            payable(walletFactory.createWallet(address(this), _policy(delegate, 100, 200, 500), keccak256("wallet")))
        );
        token.mint(address(wallet), 1_000);
        token.mint(address(creator), 10_000);
        creator.approve(token, address(competitionFactory), type(uint256).max);
        token.mint(address(this), 10_000);
        token.approve(address(competitionFactory), type(uint256).max);
    }

    function testKeeperRelaysCommitAndRevealWhileWalletIsCanonicalSolver() public {
        OpenCompetitionBountyV1 bounty = _createCompetition(creator, _params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(wallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);

        _executeSigned(OpenCompetitionEntrantWalletV1.Action.Commit, abi.encode(address(bounty), commitment), DELEGATE_KEY);
        require(bounty.hasEntered(address(wallet)), "wallet did not enter");
        require(token.balanceOf(address(wallet)) == 900, "bond not charged");
        require(wallet.lifetimeSpent() == 100, "lifetime spend mismatch");
        require(token.allowance(address(wallet), address(bounty)) == 0, "bounty allowance remains");
        vm.roll(block.number + 1);

        _executeSigned(
            OpenCompetitionEntrantWalletV1.Action.Reveal,
            abi.encode(address(bounty), SUBMISSION_HASH, EVIDENCE_HASH, SALT, PASSING_PROOF),
            DELEGATE_KEY
        );

        require(bounty.winner() == address(wallet), "wallet is not winner");
        require(token.balanceOf(address(wallet)) == 1_900, "wallet payout mismatch");
        require(token.balanceOf(bounty.verifierRewardRecipient()) == 100, "verifier payout mismatch");
        require(wallet.delegateNonce() == 2, "nonce mismatch");
    }

    function testRelayerCannotMutateSignedPayloadOrReplayIt() public {
        OpenCompetitionBountyV1 bounty = _createCompetition(creator, _params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(wallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        bytes memory payload = abi.encode(address(bounty), commitment);
        uint256 deadline = block.timestamp + 300;
        bytes memory signature = _sign(wallet, OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, DELEGATE_KEY);

        bytes memory mutated = abi.encode(address(bounty), bytes32(uint256(commitment) + 1));
        (bool mutatedOk,) = address(wallet).call(
            abi.encodeCall(
                wallet.executeWithSignature,
                (OpenCompetitionEntrantWalletV1.Action.Commit, mutated, 0, deadline, signature)
            )
        );
        require(!mutatedOk && wallet.delegateNonce() == 0, "mutated payload accepted");

        wallet.executeWithSignature(OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, signature);
        (bool replayOk,) = address(wallet).call(
            abi.encodeCall(
                wallet.executeWithSignature,
                (OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, signature)
            )
        );
        require(!replayOk && wallet.delegateNonce() == 1, "signature replayed");
    }

    function testWrongAndMalleableSignaturesFailClosed() public {
        OpenCompetitionBountyV1 bounty = _createCompetition(creator, _params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(wallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        bytes memory payload = abi.encode(address(bounty), commitment);
        uint256 deadline = block.timestamp + 300;
        bytes memory wrongSignature =
            _sign(wallet, OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, WRONG_KEY);
        (bool wrongOk,) = address(wallet).call(
            abi.encodeCall(
                wallet.executeWithSignature,
                (OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, wrongSignature)
            )
        );
        require(!wrongOk, "wrong signer accepted");

        bytes32 digest = wallet.actionDigest(OpenCompetitionEntrantWalletV1.Action.Commit, keccak256(payload), 0, deadline);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(DELEGATE_KEY, digest);
        bytes memory highS = abi.encodePacked(r, bytes32(SECP256K1_N - uint256(s)), v == 27 ? uint8(28) : uint8(27));
        (bool highSOk,) = address(wallet).call(
            abi.encodeCall(
                wallet.executeWithSignature,
                (OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, highS)
            )
        );
        require(!highSOk && wallet.delegateNonce() == 0, "malleable signature accepted");
    }

    function testPolicyRotationInvalidatesQueuedSignature() public {
        OpenCompetitionBountyV1 bounty = _createCompetition(creator, _params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(wallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        bytes memory payload = abi.encode(address(bounty), commitment);
        uint256 deadline = block.timestamp + 300;
        bytes memory signature = _sign(wallet, OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, DELEGATE_KEY);

        wallet.configurePolicy(_policy(delegate, 100, 200, 500));
        (bool ok,) = address(wallet).call(
            abi.encodeCall(
                wallet.executeWithSignature,
                (OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, signature)
            )
        );
        require(!ok && wallet.policyVersion() == 2 && wallet.delegateNonce() == 0, "old policy signature accepted");
    }

    function testDirectDelegateUsesSameNonceAndCaps() public {
        OpenCompetitionBountyV1 bounty = _createCompetition(creator, _params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(wallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        vm.prank(delegate);
        wallet.commitSolution(address(bounty), commitment);
        require(wallet.delegateNonce() == 1, "direct nonce mismatch");
        require(wallet.periodSpent() == 100 && wallet.lifetimeSpent() == 100, "direct spend not charged");
    }

    function testPerActionCapRejectsBondWithoutConsumingNonce() public {
        OpenCompetitionEntrantWalletV1 cappedWallet = OpenCompetitionEntrantWalletV1(
            payable(walletFactory.createWallet(address(this), _policy(delegate, 99, 200, 500), keccak256("capped")))
        );
        token.mint(address(cappedWallet), 1_000);
        OpenCompetitionBountyV1 bounty = _createCompetition(creator, _params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(cappedWallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        bytes memory payload = abi.encode(address(bounty), commitment);
        uint256 deadline = block.timestamp + 300;
        bytes memory signature =
            _sign(cappedWallet, OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, DELEGATE_KEY);
        (bool ok,) = address(cappedWallet).call(
            abi.encodeCall(
                cappedWallet.executeWithSignature,
                (OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, signature)
            )
        );
        require(!ok && cappedWallet.delegateNonce() == 0, "oversized bond accepted");
        require(token.balanceOf(address(cappedWallet)) == 1_000, "failed action moved funds");
    }

    function testUnknownFactoryAndVerifierProfileAreRejected() public {
        EntrantWalletVerifier otherVerifier = new EntrantWalletVerifier(keccak256(PASSING_PROOF));
        OpenCompetitionBountyV1 wrongVerifier = _createCompetition(creator, _params(address(otherVerifier)), 1_000);
        bytes32 wrongCommitment =
            wrongVerifier.solutionCommitment(address(wallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        _assertSignedCallFails(
            wallet,
            OpenCompetitionEntrantWalletV1.Action.Commit,
            abi.encode(address(wrongVerifier), wrongCommitment),
            DELEGATE_KEY
        );

        OpenCompetitionBountyFactoryV1 otherFactory = new OpenCompetitionBountyFactoryV1(address(token));
        creator.approve(token, address(otherFactory), type(uint256).max);
        (address otherBounty,) = creator.create(otherFactory, _params(address(verifier)), 1_000, keccak256("other-factory"));
        bytes32 otherCommitment = OpenCompetitionBountyV1(otherBounty).solutionCommitment(
            address(wallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT
        );
        _assertSignedCallFails(
            wallet,
            OpenCompetitionEntrantWalletV1.Action.Commit,
            abi.encode(otherBounty, otherCommitment),
            DELEGATE_KEY
        );
        require(wallet.delegateNonce() == 0, "rejected bounty consumed nonce");
    }

    function testVerifierRuntimeMismatchFailsClosed() public {
        OpenCompetitionBountyV1 bounty = _createCompetition(creator, _params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(wallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        vm.etch(address(verifier), hex"00");
        _assertSignedCallFails(
            wallet,
            OpenCompetitionEntrantWalletV1.Action.Commit,
            abi.encode(address(bounty), commitment),
            DELEGATE_KEY
        );
        require(token.balanceOf(address(wallet)) == 1_000, "runtime mismatch moved funds");
    }

    function testCreatorControlledWalletCannotEnter() public {
        OpenCompetitionBountyV1 bounty = _createCompetitionFromOwner(_params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(wallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        _assertSignedCallFails(
            wallet,
            OpenCompetitionEntrantWalletV1.Action.Commit,
            abi.encode(address(bounty), commitment),
            DELEGATE_KEY
        );
        require(!bounty.hasEntered(address(wallet)), "creator-controlled wallet entered");
    }

    function testPostCommitOwnershipTransferToCreatorCannotReveal() public {
        OpenCompetitionBountyV1 bounty = _createCompetition(creator, _params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(wallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        _executeSigned(OpenCompetitionEntrantWalletV1.Action.Commit, abi.encode(address(bounty), commitment), DELEGATE_KEY);
        vm.roll(block.number + 1);

        wallet.transferOwnership(address(creator));
        vm.prank(address(creator));
        wallet.acceptOwnership();
        bytes memory payload = abi.encode(address(bounty), SUBMISSION_HASH, EVIDENCE_HASH, SALT, PASSING_PROOF);
        _assertSignedCallFails(wallet, OpenCompetitionEntrantWalletV1.Action.Reveal, payload, DELEGATE_KEY);
        (,,,, OpenCompetitionBountyV1.EntryState entryState) = bounty.entries(address(wallet));
        require(entryState == OpenCompetitionBountyV1.EntryState.Committed, "creator consumed committed entry");
        require(bounty.winner() == address(0), "creator-controlled wallet won");
    }

    function testOwnerRecoversLosingBondAfterPolicyExpiry() public {
        OpenCompetitionBountyV1 bounty = _createCompetition(creator, _params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(wallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        _executeSigned(OpenCompetitionEntrantWalletV1.Action.Commit, abi.encode(address(bounty), commitment), DELEGATE_KEY);

        EntrantWalletOtherSolver other = new EntrantWalletOtherSolver();
        token.mint(address(other), 100);
        other.approve(token, address(bounty), 100);
        bytes32 otherCommitment =
            bounty.solutionCommitment(address(other), OTHER_SUBMISSION_HASH, OTHER_EVIDENCE_HASH, OTHER_SALT);
        other.commit(bounty, otherCommitment);
        vm.roll(block.number + 1);
        other.reveal(bounty, OTHER_SUBMISSION_HASH, OTHER_EVIDENCE_HASH, OTHER_SALT, PASSING_PROOF);
        require(bounty.winner() == address(other), "other solver did not win");

        OpenCompetitionEntrantWalletV1.Policy memory currentPolicy = wallet.policy();
        vm.warp(uint256(currentPolicy.validUntil) + 1);
        wallet.recoverEntryBond(address(bounty));
        require(token.balanceOf(address(wallet)) == 1_000, "owner recovery did not return bond");
    }

    function testNonOwnerCannotWithdrawOrRecover() public {
        (bool withdrawOk,) = address(wallet).call(
            abi.encodeCall(wallet.withdrawToken, (address(token), address(this), 1))
        );
        require(withdrawOk, "owner withdrawal setup failed");
        token.mint(address(wallet), 1);
        vm.prank(delegate);
        (bool nonOwnerOk,) = address(wallet).call(
            abi.encodeCall(wallet.withdrawToken, (address(token), delegate, 1))
        );
        require(!nonOwnerOk, "delegate withdrew token");
    }

    function testFactoryPredictionIsPolicyBoundAndIdempotent() public {
        OpenCompetitionEntrantWalletV1.Policy memory first = _policy(delegate, 100, 200, 500);
        bytes32 salt = keccak256("prediction");
        address predicted = walletFactory.predictWallet(address(this), first, salt);
        address deployed = walletFactory.createWallet(address(this), first, salt);
        require(predicted == deployed && walletFactory.isFactoryWallet(deployed), "prediction mismatch");
        require(walletFactory.createWallet(address(this), first, salt) == deployed, "deployment not idempotent");

        OpenCompetitionEntrantWalletV1.Policy memory changed = _policy(delegate, 100, 200, 501);
        require(walletFactory.predictWallet(address(this), changed, salt) != predicted, "policy did not bind address");
    }

    function testFactoryRejectsNonTransferringFundingToken() public {
        EntrantWalletNonTransferringToken badToken = new EntrantWalletNonTransferringToken();
        OpenCompetitionBountyFactoryV1 badCompetitionFactory =
            new OpenCompetitionBountyFactoryV1(address(badToken));
        OpenCompetitionEntrantWalletFactoryV1 badWalletFactory =
            new OpenCompetitionEntrantWalletFactoryV1(address(badCompetitionFactory));
        EntrantWalletVerifier badVerifier = new EntrantWalletVerifier(keccak256(PASSING_PROOF));
        OpenCompetitionEntrantWalletV1.Policy memory badPolicy = OpenCompetitionEntrantWalletV1.Policy({
            delegate: delegate,
            validAfter: uint64(block.timestamp),
            validUntil: uint64(block.timestamp + 30 days),
            periodSeconds: 1 days,
            maxPerAction: 100,
            maxPerPeriod: 200,
            maxLifetimeSpend: 500,
            maxBountyTarget: 1_000,
            allowedActions: type(uint8).max >> 5,
            verifierModule: address(badVerifier),
            verifierRuntimeCodeHash: address(badVerifier).codehash,
            verifierPolicyHash: VERIFIER_POLICY_HASH,
            acceptanceCriteriaHash: CRITERIA_HASH,
            benchmarkHash: BENCHMARK_HASH,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH
        });
        (bool ok,) = address(badWalletFactory).call(
            abi.encodeCall(badWalletFactory.createWalletAndFund, (badPolicy, keccak256("bad-token"), 100))
        );
        require(!ok, "non-transferring token funded wallet");
    }

    function testContractDelegateSignatureIsSupported() public {
        EntrantWallet1271Delegate smartDelegate = new EntrantWallet1271Delegate();
        OpenCompetitionEntrantWalletV1 smartWallet = OpenCompetitionEntrantWalletV1(
            payable(
                walletFactory.createWallet(
                    address(this), _policy(address(smartDelegate), 100, 200, 500), keccak256("smart-delegate")
                )
            )
        );
        token.mint(address(smartWallet), 1_000);
        OpenCompetitionBountyV1 bounty = _createCompetition(creator, _params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(smartWallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        bytes memory payload = abi.encode(address(bounty), commitment);
        uint256 deadline = block.timestamp + 300;
        bytes memory signature = hex"1234";
        bytes32 digest =
            smartWallet.actionDigest(OpenCompetitionEntrantWalletV1.Action.Commit, keccak256(payload), 0, deadline);
        smartDelegate.approve(digest, signature);
        smartWallet.executeWithSignature(
            OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, signature
        );
        require(bounty.hasEntered(address(smartWallet)), "contract delegate commit failed");
    }

    function testFuzzSignedCommitmentMutationCannotConsumeNonce(bytes32 mutation) public {
        OpenCompetitionBountyV1 bounty = _createCompetition(creator, _params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(wallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        bytes memory payload = abi.encode(address(bounty), commitment);
        uint256 deadline = block.timestamp + 300;
        bytes memory signature =
            _sign(wallet, OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, DELEGATE_KEY);
        bytes32 mutatedCommitment = bytes32(uint256(commitment) ^ (uint256(mutation) | 1));
        bytes memory mutatedPayload = abi.encode(address(bounty), mutatedCommitment);
        (bool ok,) = address(wallet).call(
            abi.encodeCall(
                wallet.executeWithSignature,
                (OpenCompetitionEntrantWalletV1.Action.Commit, mutatedPayload, 0, deadline, signature)
            )
        );
        require(!ok && wallet.delegateNonce() == 0, "fuzzed commitment mutation accepted");
        require(token.balanceOf(address(wallet)) == 1_000, "fuzzed mutation moved funds");
    }

    function testFuzzSubBondPerActionCapConservesFunds(uint96 rawCap) public {
        uint256 cap = 1 + uint256(rawCap) % 99;
        OpenCompetitionEntrantWalletV1 cappedWallet = OpenCompetitionEntrantWalletV1(
            payable(
                walletFactory.createWallet(
                    address(this), _policy(delegate, cap, 200, 500), keccak256(abi.encode("fuzz-cap", cap))
                )
            )
        );
        token.mint(address(cappedWallet), 1_000);
        OpenCompetitionBountyV1 bounty = _createCompetition(creator, _params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(cappedWallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        bytes memory payload = abi.encode(address(bounty), commitment);
        uint256 deadline = block.timestamp + 300;
        bytes memory signature =
            _sign(cappedWallet, OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, DELEGATE_KEY);
        (bool ok,) = address(cappedWallet).call(
            abi.encodeCall(
                cappedWallet.executeWithSignature,
                (OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, signature)
            )
        );
        require(!ok && cappedWallet.delegateNonce() == 0, "fuzzed sub-bond cap accepted");
        require(token.balanceOf(address(cappedWallet)) == 1_000, "fuzzed cap moved funds");
        require(token.balanceOf(address(bounty)) == 1_000, "fuzzed cap changed escrow");
    }

    function testFuzzExpiredSignatureCannotConsumeNonce(uint32 delay) public {
        OpenCompetitionBountyV1 bounty = _createCompetition(creator, _params(address(verifier)), 1_000);
        bytes32 commitment = bounty.solutionCommitment(address(wallet), SUBMISSION_HASH, EVIDENCE_HASH, SALT);
        bytes memory payload = abi.encode(address(bounty), commitment);
        uint256 deadline = block.timestamp + 60;
        bytes memory signature =
            _sign(wallet, OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, DELEGATE_KEY);
        vm.warp(deadline + 1 + uint256(delay) % 1 days);
        (bool ok,) = address(wallet).call(
            abi.encodeCall(
                wallet.executeWithSignature,
                (OpenCompetitionEntrantWalletV1.Action.Commit, payload, 0, deadline, signature)
            )
        );
        require(!ok && wallet.delegateNonce() == 0, "expired fuzzed signature accepted");
        require(token.balanceOf(address(wallet)) == 1_000, "expired signature moved funds");
    }

    function _policy(address delegate_, uint256 maxPerAction, uint256 maxPerPeriod, uint256 maxLifetime)
        private
        view
        returns (OpenCompetitionEntrantWalletV1.Policy memory)
    {
        return OpenCompetitionEntrantWalletV1.Policy({
            delegate: delegate_,
            validAfter: uint64(block.timestamp),
            validUntil: uint64(block.timestamp + 30 days),
            periodSeconds: 1 days,
            maxPerAction: maxPerAction,
            maxPerPeriod: maxPerPeriod,
            maxLifetimeSpend: maxLifetime,
            maxBountyTarget: 1_000,
            allowedActions: 7,
            verifierModule: address(verifier),
            verifierRuntimeCodeHash: address(verifier).codehash,
            verifierPolicyHash: VERIFIER_POLICY_HASH,
            acceptanceCriteriaHash: CRITERIA_HASH,
            benchmarkHash: BENCHMARK_HASH,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH
        });
    }

    function _params(address verifierAddress)
        private
        view
        returns (OpenCompetitionBountyFactoryV1.CreateCompetitionParams memory)
    {
        return OpenCompetitionBountyFactoryV1.CreateCompetitionParams({
            solverReward: 900,
            verifierReward: 100,
            termsHash: TERMS_HASH,
            policyHash: VERIFIER_POLICY_HASH,
            acceptanceCriteriaHash: CRITERIA_HASH,
            benchmarkHash: BENCHMARK_HASH,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH,
            fundingDeadline: uint64(block.timestamp + 1 days),
            competitionWindowSeconds: 1 days,
            revealWindowSeconds: 1 hours,
            maxEntries: 4,
            verifierModule: verifierAddress,
            verifierRewardRecipient: address(0xBEEF)
        });
    }

    function _createCompetition(
        EntrantWalletCreator creator_,
        OpenCompetitionBountyFactoryV1.CreateCompetitionParams memory params,
        uint256 initialFunding
    ) private returns (OpenCompetitionBountyV1 bounty) {
        creationNonce += 1;
        (address bountyAddress,) =
            creator_.create(competitionFactory, params, initialFunding, bytes32(creationNonce));
        return OpenCompetitionBountyV1(bountyAddress);
    }

    function _createCompetitionFromOwner(
        OpenCompetitionBountyFactoryV1.CreateCompetitionParams memory params,
        uint256 initialFunding
    ) private returns (OpenCompetitionBountyV1 bounty) {
        creationNonce += 1;
        (address bountyAddress,) =
            competitionFactory.createCompetition(params, initialFunding, bytes32(creationNonce));
        return OpenCompetitionBountyV1(bountyAddress);
    }

    function _executeSigned(
        OpenCompetitionEntrantWalletV1.Action action,
        bytes memory payload,
        uint256 signerKey
    ) private {
        uint256 nonce = wallet.delegateNonce();
        uint256 deadline = block.timestamp + 300;
        bytes memory signature = _sign(wallet, action, payload, nonce, deadline, signerKey);
        wallet.executeWithSignature(action, payload, nonce, deadline, signature);
    }

    function _assertSignedCallFails(
        OpenCompetitionEntrantWalletV1 targetWallet,
        OpenCompetitionEntrantWalletV1.Action action,
        bytes memory payload,
        uint256 signerKey
    ) private {
        uint256 nonce = targetWallet.delegateNonce();
        uint256 deadline = block.timestamp + 300;
        bytes memory signature = _sign(targetWallet, action, payload, nonce, deadline, signerKey);
        (bool ok,) = address(targetWallet).call(
            abi.encodeCall(targetWallet.executeWithSignature, (action, payload, nonce, deadline, signature))
        );
        require(!ok, "signed call unexpectedly succeeded");
    }

    function _sign(
        OpenCompetitionEntrantWalletV1 targetWallet,
        OpenCompetitionEntrantWalletV1.Action action,
        bytes memory payload,
        uint256 nonce,
        uint256 deadline,
        uint256 signerKey
    ) private returns (bytes memory) {
        bytes32 digest = targetWallet.actionDigest(action, keccak256(payload), nonce, deadline);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(signerKey, digest);
        return abi.encodePacked(r, s, v);
    }
}
