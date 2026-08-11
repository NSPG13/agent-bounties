// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";

interface IOpenCompetitionBountyV1 is IERC165 {
    function protocolVersion() external pure returns (bytes32);
    function bountyId() external view returns (bytes32);
    function creator() external view returns (address);
    function settlementToken() external view returns (address);
    function termsHash() external view returns (bytes32);
    function policyHash() external view returns (bytes32);
    function targetAmount() external view returns (uint256);
    function fundedAmount() external view returns (uint256);
    function status() external view returns (uint8);
    function solutionCommitment(address solver, bytes32 submissionHash, bytes32 evidenceHash, bytes32 salt)
        external
        view
        returns (bytes32);
}

/// @notice Deterministic open competition in which the first valid committed
/// reveal settles atomically. Commit order and verifier response time do not
/// select the winner; the canonical passing reveal sequence does.
contract OpenCompetitionBountyV1 is IOpenCompetitionBountyV1 {
    using SafeBountyToken for address;

    bytes32 public constant PROTOCOL_VERSION = keccak256("agent-bounties/open-competition-v1");
    bytes32 public constant COMMITMENT_DOMAIN = keccak256("agent-bounties/open-competition-v1-solution");
    uint64 public constant VERIFICATION_ROUND = 1;
    uint256 public constant MAX_FUNDING_WINDOW = 366 days;
    uint64 public constant MAX_COMPETITION_WINDOW = 30 days;
    uint64 public constant MAX_REVEAL_WINDOW = 1 days;
    uint8 public constant MAX_ENTRIES = 64;

    enum CompetitionStatus {
        Open,
        Competition,
        Settled,
        Cancelled
    }

    enum EntryState {
        None,
        Committed,
        Rejected,
        Expired,
        Winner,
        Refunded
    }

    struct Config {
        bytes32 bountyId;
        address creator;
        address factory;
        address settlementToken;
        uint256 solverReward;
        uint256 verifierReward;
        bytes32 termsHash;
        bytes32 policyHash;
        bytes32 acceptanceCriteriaHash;
        bytes32 benchmarkHash;
        bytes32 evidenceSchemaHash;
        uint64 fundingDeadline;
        uint64 competitionWindowSeconds;
        uint64 revealWindowSeconds;
        uint8 maxEntries;
        address verifierModule;
        address verifierRewardRecipient;
    }

    struct Entry {
        bytes32 commitment;
        uint64 committedBlock;
        uint64 revealDeadline;
        uint256 bond;
        EntryState state;
    }

    bytes32 public override bountyId;
    address public override creator;
    address public factory;
    address public override settlementToken;
    uint256 public solverReward;
    uint256 public verifierReward;
    uint256 public override targetAmount;
    bytes32 public override termsHash;
    bytes32 public override policyHash;
    bytes32 public acceptanceCriteriaHash;
    bytes32 public benchmarkHash;
    bytes32 public evidenceSchemaHash;
    uint64 public fundingDeadline;
    uint64 public competitionWindowSeconds;
    uint64 public revealWindowSeconds;
    uint8 public maxEntries;
    address public verifierModule;
    address public verifierRewardRecipient;

    uint256 public override fundedAmount;
    uint256 public lockedBondTotal;
    uint256 public timeoutBondPool;
    uint256 public refundBonusPool;
    uint256 public refundBonusRemaining;
    uint256 public refundPrincipalTotal;
    uint64 public competitionEndsAt;
    uint64 public submissionSequence;
    uint8 public entryCount;
    address public winner;
    bytes32 public winningSubmissionHash;
    bytes32 public winningEvidenceHash;
    uint64 public winningSequence;

    CompetitionStatus private _status;
    mapping(address => uint256) public contributions;
    mapping(address => Entry) public entries;
    mapping(address => bool) public hasEntered;
    address[] public entrants;
    uint256 private _reentrancy = 1;
    bool private _initialized;

    event FundingAdded(
        bytes32 indexed bountyId,
        address indexed contributor,
        uint256 amount,
        uint256 fundedAmount,
        uint256 targetAmount
    );
    event CompetitionOpened(bytes32 indexed bountyId, uint64 competitionEndsAt, uint8 maxEntries);
    event SolutionCommitted(
        bytes32 indexed bountyId,
        address indexed solver,
        uint8 indexed entryNumber,
        bytes32 commitment,
        uint64 committedBlock,
        uint64 revealDeadline,
        uint256 bond
    );
    event SolutionRevealed(
        bytes32 indexed bountyId,
        uint64 indexed submissionSequence,
        address indexed solver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bool passed,
        bytes32 verificationHash
    );
    event CompetitionSubmissionRejected(
        bytes32 indexed bountyId,
        uint64 indexed submissionSequence,
        address indexed solver,
        uint256 bondPaidToVerifier,
        bytes32 verificationHash
    );
    event CommitmentExpired(
        bytes32 indexed bountyId, address indexed solver, uint256 bondForfeited, uint256 timeoutBondPool
    );
    event BountySettled(
        bytes32 indexed bountyId,
        uint64 indexed submissionSequence,
        address indexed solver,
        uint256 solverReward,
        uint256 entryBondReturned,
        uint256 timeoutBondBonus,
        uint256 verifierReward,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 policyHash,
        bytes32 verificationHash
    );
    event EntryBondWithdrawn(bytes32 indexed bountyId, address indexed solver, uint256 amount);
    event BountyCancelled(bytes32 indexed bountyId, uint256 principal, uint256 expiredEntryBonus);
    event RefundWithdrawn(
        bytes32 indexed bountyId,
        address indexed contributor,
        uint256 principal,
        uint256 expiredEntryBonus,
        uint256 amount
    );

    modifier nonReentrant() {
        require(_reentrancy == 1, "reentrant");
        _reentrancy = 2;
        _;
        _reentrancy = 1;
    }

    /// @dev Locks the implementation while leaving clone storage uninitialized.
    constructor() {
        _initialized = true;
    }

    function initialize(Config calldata config) external {
        require(!_initialized, "already initialized");
        require(msg.sender == config.factory, "initializer not factory");
        _initialized = true;
        _reentrancy = 1;
        require(config.bountyId != bytes32(0), "bounty id zero");
        require(config.creator != address(0), "creator zero");
        require(config.factory != address(0), "factory zero");
        require(config.settlementToken != address(0), "token zero");
        require(config.solverReward > 0, "solver reward zero");
        require(config.verifierReward > 0, "verifier reward zero");
        require(config.solverReward + config.verifierReward <= type(uint64).max, "target too large");
        require(config.termsHash != bytes32(0), "terms hash zero");
        require(config.policyHash != bytes32(0), "policy hash zero");
        require(config.acceptanceCriteriaHash != bytes32(0), "criteria hash zero");
        require(config.benchmarkHash != bytes32(0), "benchmark hash zero");
        require(config.evidenceSchemaHash != bytes32(0), "evidence schema hash zero");
        require(config.verifierModule.code.length > 0, "verifier module missing");
        require(config.verifierRewardRecipient != address(0), "reward recipient zero");
        require(
            config.fundingDeadline > block.timestamp && config.fundingDeadline <= block.timestamp + MAX_FUNDING_WINDOW,
            "funding deadline out of bounds"
        );
        require(
            config.competitionWindowSeconds > 0 && config.competitionWindowSeconds <= MAX_COMPETITION_WINDOW,
            "competition window out of bounds"
        );
        require(
            config.revealWindowSeconds > 0 && config.revealWindowSeconds <= MAX_REVEAL_WINDOW
                && config.revealWindowSeconds <= config.competitionWindowSeconds,
            "reveal window out of bounds"
        );
        require(config.maxEntries >= 2 && config.maxEntries <= MAX_ENTRIES, "entry bound invalid");

        bountyId = config.bountyId;
        creator = config.creator;
        factory = config.factory;
        settlementToken = config.settlementToken;
        solverReward = config.solverReward;
        verifierReward = config.verifierReward;
        targetAmount = config.solverReward + config.verifierReward;
        termsHash = config.termsHash;
        policyHash = config.policyHash;
        acceptanceCriteriaHash = config.acceptanceCriteriaHash;
        benchmarkHash = config.benchmarkHash;
        evidenceSchemaHash = config.evidenceSchemaHash;
        fundingDeadline = config.fundingDeadline;
        competitionWindowSeconds = config.competitionWindowSeconds;
        revealWindowSeconds = config.revealWindowSeconds;
        maxEntries = config.maxEntries;
        verifierModule = config.verifierModule;
        verifierRewardRecipient = config.verifierRewardRecipient;
        _status = CompetitionStatus.Open;
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
        return interfaceId == type(IOpenCompetitionBountyV1).interfaceId || interfaceId == type(IERC165).interfaceId;
    }

    function solutionCommitment(address solver, bytes32 submissionHash, bytes32 evidenceHash, bytes32 salt)
        public
        view
        override
        returns (bytes32)
    {
        return keccak256(
            abi.encode(COMMITMENT_DOMAIN, block.chainid, address(this), solver, submissionHash, evidenceHash, salt)
        );
    }

    function recordFactoryFunding(address contributor, uint256 amount) external nonReentrant {
        require(msg.sender == factory, "not factory");
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) >= fundedAmount + amount, "funding not received"
        );
        _recordFunding(contributor, amount);
    }

    function fund(uint256 requestedAmount) external nonReentrant returns (uint256 acceptedAmount) {
        require(_status == CompetitionStatus.Open, "not open");
        require(block.timestamp <= fundingDeadline, "funding closed");
        require(requestedAmount > 0, "amount zero");
        uint256 remaining = targetAmount - fundedAmount;
        acceptedAmount = requestedAmount < remaining ? requestedAmount : remaining;
        _recordFunding(msg.sender, acceptedAmount);
        settlementToken.safeTransferFrom(msg.sender, address(this), acceptedAmount);
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) >= _requiredTokenBalance(),
            "funding not received"
        );
    }

    function fundWithAuthorization(
        address contributor,
        uint256 amount,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external nonReentrant {
        require(_status == CompetitionStatus.Open, "not open");
        require(block.timestamp <= fundingDeadline, "funding closed");
        require(amount > 0 && amount <= targetAmount - fundedAmount, "bad funding amount");
        _recordFunding(contributor, amount);
        settlementToken.safeTransferWithAuthorization(
            contributor, address(this), amount, validAfter, validBefore, nonce, v, r, s
        );
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) >= _requiredTokenBalance(),
            "funding not received"
        );
    }

    function commitSolution(bytes32 commitment) external nonReentrant {
        _recordCommitment(msg.sender, commitment);
        settlementToken.safeTransferFrom(msg.sender, address(this), verifierReward);
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) >= _requiredTokenBalance(),
            "entry bond not received"
        );
    }

    function commitSolutionWithAuthorization(
        address solver,
        bytes32 commitment,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external nonReentrant {
        require(nonce == commitment, "authorization not commitment-bound");
        _recordCommitment(solver, commitment);
        settlementToken.safeTransferWithAuthorization(
            solver, address(this), verifierReward, validAfter, validBefore, nonce, v, r, s
        );
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) >= _requiredTokenBalance(),
            "entry bond not received"
        );
    }

    function revealSolution(bytes32 submissionHash, bytes32 evidenceHash, bytes32 salt, bytes calldata proof)
        external
        nonReentrant
    {
        Entry storage entry = entries[msg.sender];
        require(_status == CompetitionStatus.Competition, "competition inactive");
        require(entry.state == EntryState.Committed, "entry not committed");
        require(block.number > entry.committedBlock, "reveal requires later block");
        require(block.timestamp <= entry.revealDeadline && block.timestamp <= competitionEndsAt, "reveal expired");
        require(submissionHash != bytes32(0) && evidenceHash != bytes32(0) && salt != bytes32(0), "reveal zero");
        require(
            entry.commitment == solutionCommitment(msg.sender, submissionHash, evidenceHash, salt),
            "commitment mismatch"
        );
        require(submissionSequence < type(uint64).max, "sequence exhausted");
        uint64 sequence = submissionSequence + 1;
        (bool passed, bytes32 responseHash) = IAgentBountyVerifier(verifierModule)
            .verify(bountyId, VERIFICATION_ROUND, msg.sender, submissionHash, evidenceHash, policyHash, proof);
        bytes32 verificationHash = keccak256(abi.encode(verifierModule, responseHash, keccak256(proof)));
        submissionSequence = sequence;
        emit SolutionRevealed(bountyId, sequence, msg.sender, submissionHash, evidenceHash, passed, verificationHash);
        if (passed) {
            _settle(msg.sender, entry, sequence, submissionHash, evidenceHash, verificationHash);
        } else {
            _reject(msg.sender, entry, sequence, verificationHash);
        }
    }

    function expireCommitment(address solver) external nonReentrant {
        Entry storage entry = entries[solver];
        require(_status == CompetitionStatus.Competition, "competition inactive");
        require(entry.state == EntryState.Committed, "entry not committed");
        require(block.timestamp > entry.revealDeadline, "commitment not expired");
        uint256 bond = entry.bond;
        entry.state = EntryState.Expired;
        entry.bond = 0;
        lockedBondTotal -= bond;
        timeoutBondPool += bond;
        emit CommitmentExpired(bountyId, solver, bond, timeoutBondPool);
    }

    function withdrawEntryBond() external nonReentrant {
        require(_status == CompetitionStatus.Settled, "bond withdrawal unavailable");
        Entry storage entry = entries[msg.sender];
        require(entry.state == EntryState.Committed, "entry not refundable");
        uint256 bond = entry.bond;
        entry.state = EntryState.Refunded;
        entry.bond = 0;
        lockedBondTotal -= bond;
        settlementToken.safeTransfer(msg.sender, bond);
        emit EntryBondWithdrawn(bountyId, msg.sender, bond);
    }

    function cancelFunding() external nonReentrant {
        require(_status == CompetitionStatus.Open, "funding cancellation unavailable");
        require(msg.sender == creator || block.timestamp > fundingDeadline, "not authorized");
        _cancel();
    }

    function cancelExpiredCompetition() external nonReentrant {
        require(_status == CompetitionStatus.Competition, "competition inactive");
        require(block.timestamp > competitionEndsAt, "competition not expired");
        require(lockedBondTotal == 0, "expire commitments first");
        _cancel();
    }

    function withdrawRefund() external nonReentrant {
        require(_status == CompetitionStatus.Cancelled, "not cancelled");
        uint256 principal = contributions[msg.sender];
        require(principal > 0, "no refund");
        uint256 bonus;
        if (refundBonusRemaining > 0) {
            bonus =
                principal == fundedAmount ? refundBonusRemaining : principal * refundBonusPool / refundPrincipalTotal;
            refundBonusRemaining -= bonus;
        }
        contributions[msg.sender] = 0;
        fundedAmount -= principal;
        uint256 amount = principal + bonus;
        settlementToken.safeTransfer(msg.sender, amount);
        emit RefundWithdrawn(bountyId, msg.sender, principal, bonus, amount);
    }

    function _recordFunding(address contributor, uint256 amount) private {
        require(_status == CompetitionStatus.Open, "not open");
        require(contributor != address(0), "contributor zero");
        require(amount > 0 && amount <= targetAmount - fundedAmount, "bad funding amount");
        contributions[contributor] += amount;
        fundedAmount += amount;
        emit FundingAdded(bountyId, contributor, amount, fundedAmount, targetAmount);
        if (fundedAmount == targetAmount) {
            competitionEndsAt = uint64(block.timestamp) + competitionWindowSeconds;
            _status = CompetitionStatus.Competition;
            emit CompetitionOpened(bountyId, competitionEndsAt, maxEntries);
        }
    }

    function _recordCommitment(address solver, bytes32 commitment) private {
        require(_status == CompetitionStatus.Competition, "competition inactive");
        require(block.timestamp < competitionEndsAt, "competition ended");
        require(solver != address(0) && solver != creator, "solver ineligible");
        require(commitment != bytes32(0), "commitment zero");
        require(!hasEntered[solver], "wallet already entered");
        require(entryCount < maxEntries, "entry capacity reached");
        require(block.number <= type(uint64).max, "block number too large");
        uint64 deadline = uint64(block.timestamp) + revealWindowSeconds;
        if (deadline > competitionEndsAt) deadline = competitionEndsAt;
        entryCount += 1;
        hasEntered[solver] = true;
        entrants.push(solver);
        entries[solver] = Entry({
            commitment: commitment,
            committedBlock: uint64(block.number),
            revealDeadline: deadline,
            bond: verifierReward,
            state: EntryState.Committed
        });
        lockedBondTotal += verifierReward;
        emit SolutionCommitted(bountyId, solver, entryCount, commitment, uint64(block.number), deadline, verifierReward);
    }

    function _settle(
        address solver,
        Entry storage entry,
        uint64 sequence,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 verificationHash
    ) private {
        require(fundedAmount == targetAmount, "not fully funded");
        uint256 returnedBond = entry.bond;
        uint256 timeoutBonus = timeoutBondPool;
        entry.state = EntryState.Winner;
        entry.bond = 0;
        lockedBondTotal -= returnedBond;
        timeoutBondPool = 0;
        fundedAmount = 0;
        winner = solver;
        winningSubmissionHash = submissionHash;
        winningEvidenceHash = evidenceHash;
        winningSequence = sequence;
        _status = CompetitionStatus.Settled;
        settlementToken.safeTransfer(solver, solverReward + returnedBond + timeoutBonus);
        settlementToken.safeTransfer(verifierRewardRecipient, verifierReward);
        emit BountySettled(
            bountyId,
            sequence,
            solver,
            solverReward,
            returnedBond,
            timeoutBonus,
            verifierReward,
            submissionHash,
            evidenceHash,
            policyHash,
            verificationHash
        );
    }

    function _reject(address solver, Entry storage entry, uint64 sequence, bytes32 verificationHash) private {
        uint256 bond = entry.bond;
        require(bond == verifierReward, "entry bond invariant");
        entry.state = EntryState.Rejected;
        entry.bond = 0;
        lockedBondTotal -= bond;
        settlementToken.safeTransfer(verifierRewardRecipient, bond);
        emit CompetitionSubmissionRejected(bountyId, sequence, solver, bond, verificationHash);
    }

    function _cancel() private {
        _status = CompetitionStatus.Cancelled;
        refundPrincipalTotal = fundedAmount;
        refundBonusPool = timeoutBondPool;
        refundBonusRemaining = timeoutBondPool;
        timeoutBondPool = 0;
        emit BountyCancelled(bountyId, fundedAmount, refundBonusPool);
    }

    function _requiredTokenBalance() private view returns (uint256) {
        return fundedAmount + lockedBondTotal + timeoutBondPool;
    }
}
