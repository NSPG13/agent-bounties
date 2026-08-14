// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";
import "./Sp1VerifierAdapterV2Beta2.sol";

interface IOpenCompetitionBountyV2Beta2 is IERC165 {
    function protocolVersion() external pure returns (bytes32);
    function bountyId() external view returns (bytes32);
    function creator() external view returns (address);
    function settlementToken() external view returns (address);
    function fundedAmount() external view returns (uint256);
    function targetAmount() external view returns (uint256);
    function status() external view returns (uint8);
}

/// @notice Isolated, immutable, SP1-verified competition escrow. The contract
/// stores no participant list and no operation depends on participant count.
contract OpenCompetitionBountyV2Beta2 is IOpenCompetitionBountyV2Beta2 {
    using SafeBountyToken for address;

    bytes32 public constant PROTOCOL_VERSION = keccak256("agent-bounties/open-competition-v2-beta2");
    bytes32 public constant JOURNAL_DOMAIN = keccak256("agent-bounties/open-competition-v2-beta2/journal");
    bytes32 public constant PROOF_SYSTEM_GROTH16 = keccak256("sp1-groth16");
    bytes32 public constant PROOF_SYSTEM_PLONK = keccak256("sp1-plonk");

    bytes4 private constant ERC1271_MAGIC_VALUE = 0x1626ba7e;
    uint256 private constant ERC1271_GAS_LIMIT = 200_000;
    uint256 private constant SECP256K1_HALF_ORDER = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;
    bytes32 private constant EIP712_DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant NAME_HASH = keccak256("Agent Bounties Open Competition V2 Beta2");
    bytes32 private constant VERSION_HASH = keccak256("1");
    bytes32 private constant SUBMIT_PROOF_TYPEHASH = keccak256(
        "SubmitProof(address solver,uint256 solverNonce,bytes32 publicValuesHash,bytes32 proofHash,uint256 authorizationDeadline)"
    );

    uint256 public constant MAX_FUNDING_WINDOW = 366 days;
    uint64 public constant MAX_PROOF_WINDOW = 90 days;
    uint256 public constant JOURNAL_ABI_LENGTH = 20 * 32;

    enum CompetitionStatus {
        Funding,
        Active,
        Settled,
        Cancelled
    }

    enum WinnerMode {
        FirstProven,
        BestScore
    }

    enum ScoreDirection {
        HigherIsBetter,
        LowerIsBetter
    }

    struct Config {
        bytes32 bountyId;
        address creator;
        address factory;
        address settlementToken;
        address verifierAdapter;
        bytes32 proofSystem;
        bytes32 programVKey;
        bytes32 sourceHash;
        bytes32 elfHash;
        bytes32 journalSchemaHash;
        bytes32 metricProgramHash;
        bytes32 executionPolicyHash;
        bytes32 verificationPolicyHash;
        bytes32 settlementPolicyHash;
        bytes32 betaRiskHash;
        uint256 solverReward;
        uint256 keeperReward;
        uint64 fundingDeadline;
        uint64 proofWindowSeconds;
        WinnerMode winnerMode;
        ScoreDirection scoreDirection;
        int256 scoreThreshold;
    }

    struct Journal {
        bytes32 domain;
        uint256 chainId;
        address competition;
        bytes32 bountyId;
        address solver;
        uint256 solverNonce;
        bytes32 submissionHash;
        bytes32 evidenceHash;
        bytes32 proofSystem;
        bytes32 programVKey;
        bytes32 sourceHash;
        bytes32 elfHash;
        bytes32 journalSchemaHash;
        bytes32 metricProgramHash;
        bytes32 executionPolicyHash;
        bytes32 verificationPolicyHash;
        bytes32 settlementPolicyHash;
        bytes32 betaRiskHash;
        bool passed;
        int256 score;
    }

    error V2AlreadyInitialized();
    error V2InvalidConfiguration();
    error V2NotFactory();
    error V2NotFunding();
    error V2FundingClosed();
    error V2NotActive();
    error V2ProofDeadlinePassed();
    error V2RiskHashMismatch();
    error V2FundingAmountInvalid();
    error V2SolverAuthInvalid();
    error V2SolverAuthExpired();
    error V2SolverNonceUsed();
    error V2JournalDecodeInvalid();
    error V2JournalScopeMismatch();
    error V2JournalReportedFailure();
    error V2ScoreThresholdNotMet();
    error V2Sp1ProofInvalid();
    error V2NoLeader();
    error V2LeaderRequiresFinalization();
    error V2FinalizeTooEarly();
    error V2RefundUnavailable();
    error V2NothingToRefund();
    error V2VerifierStillAvailable();
    error V2VerifierUnavailable();
    error V2TokenAccountingMismatch();
    error V2ReentrantCall();

    bytes32 public override bountyId;
    address public override creator;
    address public factory;
    address public override settlementToken;
    address public verifierAdapter;
    bytes32 public proofSystem;
    bytes32 public programVKey;
    bytes32 public sourceHash;
    bytes32 public elfHash;
    bytes32 public journalSchemaHash;
    bytes32 public metricProgramHash;
    bytes32 public executionPolicyHash;
    bytes32 public verificationPolicyHash;
    bytes32 public settlementPolicyHash;
    bytes32 public betaRiskHash;
    uint256 public solverReward;
    uint256 public keeperReward;
    uint256 public override targetAmount;
    uint64 public fundingDeadline;
    uint64 public proofWindowSeconds;
    uint64 public proofDeadline;
    WinnerMode public winnerMode;
    ScoreDirection public scoreDirection;
    int256 public scoreThreshold;

    uint256 public override fundedAmount;
    uint256 public acceptedSequence;
    address public leader;
    int256 public leaderScore;
    uint256 public leaderSequence;
    bytes32 public leaderSubmissionHash;
    bytes32 public leaderEvidenceHash;
    address public winner;
    bytes32 public winningSubmissionHash;
    bytes32 public winningEvidenceHash;
    int256 public winningScore;
    uint256 public winningSequence;
    uint256 public refundPoolRemaining;
    uint256 public refundWeightRemaining;

    CompetitionStatus private _status;
    mapping(address => uint256) public contributions;
    mapping(address => mapping(uint256 => bool)) public usedSolverNonces;
    uint256 private _reentrancy = 1;
    bool private _initialized;

    event FundingAddedV2(
        bytes32 indexed bountyId,
        address indexed contributor,
        uint256 amount,
        uint256 fundedAmount,
        uint256 targetAmount
    );
    event CompetitionActivatedV2(bytes32 indexed bountyId, uint64 proofDeadline);
    event CompetitionEntryQualifiedV2(
        bytes32 indexed bountyId,
        uint256 indexed sequence,
        address indexed solver,
        uint256 solverNonce,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        int256 score,
        address proofSubmitter
    );
    event CompetitionLeaderUpdatedV2(
        bytes32 indexed bountyId, uint256 indexed sequence, address indexed solver, int256 score
    );
    event CompetitionSettledV2(
        bytes32 indexed bountyId,
        uint256 indexed winningSequence,
        address indexed solver,
        uint256 solverReward,
        address keeper,
        uint256 keeperReward,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        int256 score,
        bytes32 settlementPolicyHash
    );
    event CompetitionCancelledV2(
        bytes32 indexed bountyId,
        address indexed transitionCaller,
        uint256 refundPool,
        uint256 contributionWeight,
        uint256 keeperPaid,
        bytes32 reason
    );
    event CompetitionRefundWithdrawnV2(
        bytes32 indexed bountyId, address indexed contributor, address indexed caller, uint256 amount
    );

    modifier nonReentrant() {
        if (_reentrancy != 1) revert V2ReentrantCall();
        _reentrancy = 2;
        _;
        _reentrancy = 1;
    }

    /// @dev Locks the implementation while clone storage remains uninitialized.
    constructor() {
        _initialized = true;
    }

    function initialize(Config calldata config) external {
        if (_initialized) revert V2AlreadyInitialized();
        if (msg.sender != config.factory) revert V2NotFactory();
        _initialized = true;
        _reentrancy = 1;
        _validateConfig(config);

        bountyId = config.bountyId;
        creator = config.creator;
        factory = config.factory;
        settlementToken = config.settlementToken;
        verifierAdapter = config.verifierAdapter;
        proofSystem = config.proofSystem;
        programVKey = config.programVKey;
        sourceHash = config.sourceHash;
        elfHash = config.elfHash;
        journalSchemaHash = config.journalSchemaHash;
        metricProgramHash = config.metricProgramHash;
        executionPolicyHash = config.executionPolicyHash;
        verificationPolicyHash = config.verificationPolicyHash;
        settlementPolicyHash = config.settlementPolicyHash;
        betaRiskHash = config.betaRiskHash;
        solverReward = config.solverReward;
        keeperReward = config.keeperReward;
        targetAmount = config.solverReward + config.keeperReward;
        fundingDeadline = config.fundingDeadline;
        proofWindowSeconds = config.proofWindowSeconds;
        winnerMode = config.winnerMode;
        scoreDirection = config.scoreDirection;
        scoreThreshold = config.scoreThreshold;
        _status = CompetitionStatus.Funding;
    }

    function protocolVersion() external pure override returns (bytes32) {
        return PROTOCOL_VERSION;
    }

    function status() external view override returns (uint8) {
        return uint8(_status);
    }

    function competitionStatus() external view returns (CompetitionStatus) {
        return _status;
    }

    function supportsInterface(bytes4 interfaceId) external pure override returns (bool) {
        return
            interfaceId == type(IOpenCompetitionBountyV2Beta2).interfaceId || interfaceId == type(IERC165).interfaceId;
    }

    function domainSeparator() public view returns (bytes32) {
        return keccak256(abi.encode(EIP712_DOMAIN_TYPEHASH, NAME_HASH, VERSION_HASH, block.chainid, address(this)));
    }

    function entryAuthorizationDigest(
        address solver,
        uint256 solverNonce,
        bytes32 publicValuesHash,
        bytes32 proofHash,
        uint256 authorizationDeadline
    ) public view returns (bytes32) {
        bytes32 structHash = keccak256(
            abi.encode(SUBMIT_PROOF_TYPEHASH, solver, solverNonce, publicValuesHash, proofHash, authorizationDeadline)
        );
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator(), structHash));
    }

    function fund(uint256 requestedAmount, bytes32 acknowledgedRiskHash)
        external
        nonReentrant
        returns (uint256 acceptedAmount)
    {
        acceptedAmount = _prepareFunding(msg.sender, requestedAmount, acknowledgedRiskHash, true);
        uint256 beforeBalance = IERC20BountyToken(settlementToken).balanceOf(address(this));
        settlementToken.safeTransferFrom(msg.sender, address(this), acceptedAmount);
        _confirmFunding(beforeBalance, acceptedAmount);
    }

    /// @dev Enables atomic create-and-fund while keeping the competition, not
    /// the factory, as the ERC-20 allowance spender.
    function fundFromFactory(address contributor, uint256 amount, bytes32 acknowledgedRiskHash) external nonReentrant {
        if (msg.sender != factory) revert V2NotFactory();
        uint256 acceptedAmount = _prepareFunding(contributor, amount, acknowledgedRiskHash, false);
        uint256 beforeBalance = IERC20BountyToken(settlementToken).balanceOf(address(this));
        settlementToken.safeTransferFrom(contributor, address(this), acceptedAmount);
        _confirmFunding(beforeBalance, acceptedAmount);
    }

    function fundWithAuthorization(
        address contributor,
        uint256 amount,
        bytes32 acknowledgedRiskHash,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external nonReentrant {
        uint256 acceptedAmount = _prepareFunding(contributor, amount, acknowledgedRiskHash, false);
        uint256 beforeBalance = IERC20BountyToken(settlementToken).balanceOf(address(this));
        settlementToken.safeTransferWithAuthorization(
            contributor, address(this), acceptedAmount, validAfter, validBefore, nonce, v, r, s
        );
        _confirmFunding(beforeBalance, acceptedAmount);
    }

    function submitProof(bytes calldata publicValues, bytes calldata proofBytes) external nonReentrant {
        Journal memory journal = _validatedJournal(publicValues);
        if (journal.solver != msg.sender) revert V2SolverAuthInvalid();
        _acceptProof(journal, publicValues, proofBytes, msg.sender);
    }

    function submitProofFor(
        bytes calldata publicValues,
        bytes calldata proofBytes,
        uint256 authorizationDeadline,
        bytes calldata solverSignature
    ) external nonReentrant {
        Journal memory journal = _validatedJournal(publicValues);
        if (block.timestamp > authorizationDeadline) revert V2SolverAuthExpired();
        bytes32 digest = entryAuthorizationDigest(
            journal.solver, journal.solverNonce, keccak256(publicValues), keccak256(proofBytes), authorizationDeadline
        );
        if (!_isValidSignatureNow(journal.solver, digest, solverSignature)) revert V2SolverAuthInvalid();
        _acceptProof(journal, publicValues, proofBytes, msg.sender);
    }

    function finalizeBestScore() external nonReentrant {
        if (_status != CompetitionStatus.Active || winnerMode != WinnerMode.BestScore) revert V2NotActive();
        if (block.timestamp <= proofDeadline) revert V2FinalizeTooEarly();
        if (leader == address(0)) revert V2NoLeader();
        _settle(leader, leaderSubmissionHash, leaderEvidenceHash, leaderScore, leaderSequence, msg.sender);
    }

    function cancelFunding() external nonReentrant {
        if (_status != CompetitionStatus.Funding) revert V2NotFunding();
        if (msg.sender != creator && block.timestamp <= fundingDeadline) revert V2FundingClosed();
        _cancel(bytes32("funding_cancelled"), address(0), 0, fundedAmount, fundedAmount);
    }

    function expireCompetition() external nonReentrant {
        if (_status != CompetitionStatus.Active) revert V2NotActive();
        if (block.timestamp <= proofDeadline) revert V2FinalizeTooEarly();
        if (winnerMode == WinnerMode.BestScore && leader != address(0)) {
            revert V2LeaderRequiresFinalization();
        }
        _cancel(bytes32("proof_deadline_expired"), msg.sender, keeperReward, solverReward, targetAmount);
    }

    function cancelForUnavailableVerifier() external nonReentrant {
        if (_status != CompetitionStatus.Funding && _status != CompetitionStatus.Active) {
            revert V2NotActive();
        }
        if (_verifierAvailable()) revert V2VerifierStillAvailable();
        if (_status == CompetitionStatus.Funding) {
            _cancel(bytes32("sp1_verifier_unavailable"), address(0), 0, fundedAmount, fundedAmount);
        } else {
            _cancel(bytes32("sp1_verifier_unavailable"), msg.sender, keeperReward, solverReward, targetAmount);
        }
    }

    function withdrawRefundFor(address contributor) external nonReentrant returns (uint256 amount) {
        if (_status != CompetitionStatus.Cancelled) revert V2RefundUnavailable();
        uint256 weight = contributions[contributor];
        if (weight == 0) revert V2NothingToRefund();

        contributions[contributor] = 0;
        if (weight == refundWeightRemaining) {
            amount = refundPoolRemaining;
        } else {
            amount = weight * refundPoolRemaining / refundWeightRemaining;
        }
        refundWeightRemaining -= weight;
        refundPoolRemaining -= amount;
        settlementToken.safeTransfer(contributor, amount);
        emit CompetitionRefundWithdrawnV2(bountyId, contributor, msg.sender, amount);
    }

    function scoreMeetsThreshold(int256 score) public view returns (bool) {
        return scoreDirection == ScoreDirection.HigherIsBetter ? score >= scoreThreshold : score <= scoreThreshold;
    }

    function _prepareFunding(
        address contributor,
        uint256 requestedAmount,
        bytes32 acknowledgedRiskHash,
        bool allowClamp
    ) private returns (uint256 acceptedAmount) {
        if (_status != CompetitionStatus.Funding) revert V2NotFunding();
        if (block.timestamp > fundingDeadline) revert V2FundingClosed();
        if (!_verifierAvailable()) revert V2VerifierUnavailable();
        if (acknowledgedRiskHash != betaRiskHash) revert V2RiskHashMismatch();
        if (contributor == address(0) || requestedAmount == 0) revert V2FundingAmountInvalid();
        uint256 remaining = targetAmount - fundedAmount;
        acceptedAmount = allowClamp && requestedAmount > remaining ? remaining : requestedAmount;
        if (acceptedAmount == 0 || acceptedAmount > remaining) revert V2FundingAmountInvalid();
        contributions[contributor] += acceptedAmount;
        fundedAmount += acceptedAmount;
        emit FundingAddedV2(bountyId, contributor, acceptedAmount, fundedAmount, targetAmount);
    }

    function _confirmFunding(uint256 beforeBalance, uint256 amount) private {
        uint256 afterBalance = IERC20BountyToken(settlementToken).balanceOf(address(this));
        if (afterBalance != beforeBalance + amount || afterBalance < fundedAmount) {
            revert V2TokenAccountingMismatch();
        }
        if (fundedAmount == targetAmount) {
            _status = CompetitionStatus.Active;
            proofDeadline = uint64(block.timestamp + proofWindowSeconds);
            emit CompetitionActivatedV2(bountyId, proofDeadline);
        }
    }

    function _validatedJournal(bytes calldata publicValues) private view returns (Journal memory journal) {
        if (_status != CompetitionStatus.Active) revert V2NotActive();
        if (block.timestamp > proofDeadline) revert V2ProofDeadlinePassed();
        if (publicValues.length != JOURNAL_ABI_LENGTH) revert V2JournalDecodeInvalid();
        journal = abi.decode(publicValues, (Journal));
        if (
            journal.domain != JOURNAL_DOMAIN || journal.chainId != block.chainid || journal.competition != address(this)
                || journal.bountyId != bountyId || journal.solver == address(0) || journal.submissionHash == bytes32(0)
                || journal.evidenceHash == bytes32(0) || journal.proofSystem != proofSystem
                || journal.programVKey != programVKey || journal.sourceHash != sourceHash || journal.elfHash != elfHash
                || journal.journalSchemaHash != journalSchemaHash || journal.metricProgramHash != metricProgramHash
                || journal.executionPolicyHash != executionPolicyHash
                || journal.verificationPolicyHash != verificationPolicyHash
                || journal.settlementPolicyHash != settlementPolicyHash || journal.betaRiskHash != betaRiskHash
        ) revert V2JournalScopeMismatch();
        if (!journal.passed) revert V2JournalReportedFailure();
        if (!scoreMeetsThreshold(journal.score)) revert V2ScoreThresholdNotMet();
        if (usedSolverNonces[journal.solver][journal.solverNonce]) revert V2SolverNonceUsed();
    }

    function _acceptProof(
        Journal memory journal,
        bytes calldata publicValues,
        bytes calldata proofBytes,
        address proofSubmitter
    ) private {
        (bool verified,) = verifierAdapter.staticcall(
            abi.encodeCall(ISp1VerifierAdapterV2Beta2.verify, (programVKey, publicValues, proofBytes))
        );
        if (!verified) revert V2Sp1ProofInvalid();

        usedSolverNonces[journal.solver][journal.solverNonce] = true;
        uint256 sequence = ++acceptedSequence;
        emit CompetitionEntryQualifiedV2(
            bountyId,
            sequence,
            journal.solver,
            journal.solverNonce,
            journal.submissionHash,
            journal.evidenceHash,
            journal.score,
            proofSubmitter
        );

        if (winnerMode == WinnerMode.FirstProven) {
            _settle(
                journal.solver, journal.submissionHash, journal.evidenceHash, journal.score, sequence, proofSubmitter
            );
            return;
        }

        if (leader == address(0) || _isStrictlyBetter(journal.score, leaderScore)) {
            leader = journal.solver;
            leaderScore = journal.score;
            leaderSequence = sequence;
            leaderSubmissionHash = journal.submissionHash;
            leaderEvidenceHash = journal.evidenceHash;
            emit CompetitionLeaderUpdatedV2(bountyId, sequence, journal.solver, journal.score);
        }
    }

    function _settle(
        address solver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        int256 score,
        uint256 sequence,
        address keeper
    ) private {
        _status = CompetitionStatus.Settled;
        fundedAmount = 0;
        winner = solver;
        winningSubmissionHash = submissionHash;
        winningEvidenceHash = evidenceHash;
        winningScore = score;
        winningSequence = sequence;

        settlementToken.safeTransfer(solver, solverReward);
        settlementToken.safeTransfer(keeper, keeperReward);
        emit CompetitionSettledV2(
            bountyId,
            sequence,
            solver,
            solverReward,
            keeper,
            keeperReward,
            submissionHash,
            evidenceHash,
            score,
            settlementPolicyHash
        );
    }

    function _cancel(
        bytes32 reason,
        address keeper,
        uint256 keeperPayment,
        uint256 refundPool,
        uint256 contributionWeight
    ) private {
        _status = CompetitionStatus.Cancelled;
        fundedAmount = 0;
        refundPoolRemaining = refundPool;
        refundWeightRemaining = contributionWeight;
        if (keeperPayment > 0) settlementToken.safeTransfer(keeper, keeperPayment);
        emit CompetitionCancelledV2(bountyId, msg.sender, refundPool, contributionWeight, keeperPayment, reason);
    }

    function _isStrictlyBetter(int256 candidate, int256 current) private view returns (bool) {
        return scoreDirection == ScoreDirection.HigherIsBetter ? candidate > current : candidate < current;
    }

    function _verifierAvailable() private view returns (bool) {
        (bool ok, bytes memory result) =
            verifierAdapter.staticcall(abi.encodeCall(ISp1VerifierAdapterV2Beta2.verifierAvailable, ()));
        return ok && result.length == 32 && abi.decode(result, (bool));
    }

    function _validateConfig(Config calldata config) private view {
        uint256 target = config.solverReward + config.keeperReward;
        if (
            config.bountyId == bytes32(0) || config.creator == address(0) || config.factory == address(0)
                || config.settlementToken.code.length == 0 || config.verifierAdapter.code.length == 0
                || config.programVKey == bytes32(0) || config.sourceHash == bytes32(0) || config.elfHash == bytes32(0)
                || config.journalSchemaHash == bytes32(0) || config.metricProgramHash == bytes32(0)
                || config.executionPolicyHash == bytes32(0) || config.verificationPolicyHash == bytes32(0)
                || config.settlementPolicyHash == bytes32(0) || config.betaRiskHash == bytes32(0)
                || config.solverReward == 0 || config.keeperReward == 0 || target < config.solverReward
                || target > type(uint128).max || config.keeperReward > config.solverReward / 20
                || config.fundingDeadline <= block.timestamp
                || config.fundingDeadline > block.timestamp + MAX_FUNDING_WINDOW || config.proofWindowSeconds == 0
                || config.proofWindowSeconds > MAX_PROOF_WINDOW
                || (config.proofSystem != PROOF_SYSTEM_GROTH16 && config.proofSystem != PROOF_SYSTEM_PLONK)
                || ISp1VerifierAdapterV2Beta2(config.verifierAdapter).proofSystem() != config.proofSystem
                || !ISp1VerifierAdapterV2Beta2(config.verifierAdapter).verifierAvailable()
        ) revert V2InvalidConfiguration();
    }

    function _isValidSignatureNow(address signer, bytes32 digest, bytes calldata signature)
        private
        view
        returns (bool)
    {
        if (signer.code.length > 0) {
            bytes memory callData = abi.encodeCall(IERC1271.isValidSignature, (digest, signature));
            bool ok;
            bytes32 resultWord;
            assembly ("memory-safe") {
                let output := mload(0x40)
                mstore(output, 0)
                ok := staticcall(ERC1271_GAS_LIMIT, signer, add(callData, 0x20), mload(callData), output, 0x20)
                ok := and(ok, iszero(lt(returndatasize(), 0x20)))
                resultWord := mload(output)
            }
            return ok && bytes4(resultWord) == ERC1271_MAGIC_VALUE;
        }
        if (signature.length != 65) return false;
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly ("memory-safe") {
            r := calldataload(signature.offset)
            s := calldataload(add(signature.offset, 32))
            v := byte(0, calldataload(add(signature.offset, 64)))
        }
        if (uint256(s) > SECP256K1_HALF_ORDER || (v != 27 && v != 28)) return false;
        address recovered = ecrecover(digest, v, r, s);
        return recovered != address(0) && recovered == signer;
    }
}
