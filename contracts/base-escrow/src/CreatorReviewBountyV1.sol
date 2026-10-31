// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";

interface IReceiveAuthorizedUSDC {
    function receiveWithAuthorization(
        address from,
        address to,
        uint256 value,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        bytes calldata signature
    ) external;
}

/// @notice Candidate, undeployed protocol: the first creator-approved entry wins.
/// @dev Separate from autonomous-v1. All user mutations have a sponsored path.
contract CreatorReviewBountyV1 {
    using SafeBountyToken for address;

    bytes32 public constant PROTOCOL_VERSION = keccak256("agent-bounties/creator-open-v1");
    bytes32 private constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant SUBMIT_TYPEHASH = keccak256(
        "OpenSubmission(address solver,bytes32 submissionHash,bytes32 evidenceHash,bytes32 termsHash,uint64 revision,bytes32 tokenNonce,uint256 deadline)"
    );
    bytes32 private constant REVIEW_TYPEHASH = keccak256(
        "CreatorReview(uint64 entry,address solver,bytes32 submissionHash,bytes32 evidenceHash,bytes32 termsHash,uint64 revision,bool passed,bytes32 responseHash,uint256 deadline)"
    );
    bytes32 private constant INCREASE_TYPEHASH = keccak256(
        "IncreaseReward(uint64 revision,bytes32 termsHash,bytes32 newTermsHash,uint256 newReward,bytes32 tokenNonce,uint256 deadline)"
    );
    bytes32 private constant CANCEL_TYPEHASH =
        keccak256("CancelBounty(uint64 revision,bytes32 termsHash,uint256 deadline)");
    uint256 private constant HALF_ORDER = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;

    struct Config {
        bytes32 bountyId;
        address creator;
        uint128 solverReward;
        uint128 reviewReward;
        bytes32 termsHash;
        bytes32 acceptanceCriteriaHash;
        bytes32 evidenceSchemaHash;
        uint64 submissionDeadline;
        uint64 reviewWindow;
        uint16 maxPendingEntries;
    }

    struct TokenAuthorization {
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
        bytes signature;
    }

    struct Submission {
        address solver;
        bytes32 submissionHash;
        bytes32 evidenceHash;
        bytes32 termsHash;
        uint64 revision;
        uint256 deadline;
    }
    enum Status {
        Uninitialized,
        Open,
        Settled,
        Cancelled
    }
    enum EntryStatus {
        None,
        Pending,
        Rejected,
        Winner,
        Refunded
    }

    struct Entry {
        address solver;
        bytes32 submissionHash;
        bytes32 evidenceHash;
        uint64 reviewDeadline;
        EntryStatus status;
    }

    address public immutable factory;
    address public immutable settlementToken;
    bytes32 public bountyId;
    address public creator;
    uint256 public solverReward;
    uint256 public reviewReward;
    bytes32 public termsHash;
    bytes32 public acceptanceCriteriaHash;
    bytes32 public evidenceSchemaHash;
    uint64 public submissionDeadline;
    uint64 public reviewWindow;
    uint16 public maxPendingEntries;
    uint64 public revision;
    uint64 public entryCount;
    uint64 public winner;
    uint16 public pendingEntries;
    uint256 public refundablePrincipal;
    uint256 public outstandingBonds;
    Status public status;
    mapping(uint64 => Entry) public entries;
    mapping(address => uint64) public activeEntry;
    mapping(address => mapping(bytes32 => bool)) public usedSubmissionNonce;
    bool private _initialized;
    uint256 private _guard;

    event OpenBountyFunded(bytes32 indexed bountyId, address indexed creator, uint256 principal, bytes32 termsHash);
    event OpenSubmissionAdded(
        bytes32 indexed bountyId,
        uint64 indexed entry,
        address indexed solver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 termsHash,
        uint64 revision,
        uint64 reviewDeadline,
        uint256 bond
    );
    event OpenSubmissionRejected(
        bytes32 indexed bountyId,
        uint64 indexed entry,
        address indexed solver,
        bytes32 responseHash,
        uint256 reviewReward
    );
    event OpenBountySettled(
        bytes32 indexed bountyId,
        uint64 indexed entry,
        address indexed solver,
        uint256 solverReward,
        uint256 bondReturned,
        uint256 reviewReward,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 termsHash,
        bytes32 responseHash
    );
    event OpenBondRefunded(bytes32 indexed bountyId, uint64 indexed entry, address indexed solver, uint256 amount);
    event OpenRewardIncreased(
        bytes32 indexed bountyId,
        uint64 revision,
        bytes32 oldTermsHash,
        bytes32 newTermsHash,
        uint256 oldReward,
        uint256 newReward
    );
    event OpenBountyCancelled(bytes32 indexed bountyId);
    event OpenPrincipalRefunded(bytes32 indexed bountyId, address indexed creator, uint256 amount);

    modifier nonReentrant() {
        require(_guard == 1, "reentrant or uninitialized");
        _guard = 2;
        _;
        _guard = 1;
    }

    constructor(address token) {
        require(token.code.length > 0, "token has no code");
        factory = msg.sender;
        settlementToken = token;
        _initialized = true; // The implementation itself cannot be used as a bounty.
    }

    /// @dev Callable only by the immutable factory, atomically with deployment.
    function initializeAndFund(Config calldata config, TokenAuthorization calldata authorization) external {
        require(msg.sender == factory && !_initialized, "initialization forbidden");
        require(config.creator != address(0) && config.bountyId != bytes32(0), "invalid identity");
        require(config.solverReward > 0 && config.reviewReward > 0, "positive rewards required");
        require(
            config.termsHash != bytes32(0) && config.acceptanceCriteriaHash != bytes32(0)
                && config.evidenceSchemaHash != bytes32(0),
            "missing commitments"
        );
        require(
            config.submissionDeadline > block.timestamp && config.submissionDeadline <= block.timestamp + 366 days,
            "invalid work window"
        );
        require(config.reviewWindow >= 1 hours && config.reviewWindow <= 30 days, "invalid review window");
        require(config.maxPendingEntries > 0 && config.maxPendingEntries <= 64, "invalid entry cap");
        _initialized = true;
        _guard = 2;
        bountyId = config.bountyId;
        creator = config.creator;
        solverReward = config.solverReward;
        reviewReward = config.reviewReward;
        termsHash = config.termsHash;
        acceptanceCriteriaHash = config.acceptanceCriteriaHash;
        evidenceSchemaHash = config.evidenceSchemaHash;
        submissionDeadline = config.submissionDeadline;
        reviewWindow = config.reviewWindow;
        maxPendingEntries = config.maxPendingEntries;
        revision = 1;
        refundablePrincipal = solverReward + reviewReward;
        status = Status.Open;
        _receive(creator, refundablePrincipal, authorization, block.timestamp + 2 hours);
        _assertSolvent();
        _guard = 1;
        emit OpenBountyFunded(bountyId, creator, refundablePrincipal, termsHash);
    }

    function submitAuthorized(
        Submission calldata submission,
        TokenAuthorization calldata authorization,
        bytes calldata signature
    ) external nonReentrant returns (uint64 id) {
        require(status == Status.Open && block.timestamp < submissionDeadline, "submissions closed");
        require(pendingEntries < maxPendingEntries, "review queue full");
        require(submission.solver != address(0) && submission.solver != creator, "independent solver required");
        require(activeEntry[submission.solver] == 0, "solver has pending entry");
        require(submission.submissionHash != bytes32(0) && submission.evidenceHash != bytes32(0), "missing result");
        require(submission.termsHash == termsHash && submission.revision == revision, "terms changed");
        _deadline(submission.deadline);
        require(!usedSubmissionNonce[submission.solver][authorization.nonce], "submission replay");
        require(
            _validSignature(submission.solver, submissionDigest(submission, authorization.nonce), signature),
            "invalid solver signature"
        );
        usedSubmissionNonce[submission.solver][authorization.nonce] = true;
        id = ++entryCount;
        uint64 expires = uint64(block.timestamp) + reviewWindow;
        entries[id] =
            Entry(submission.solver, submission.submissionHash, submission.evidenceHash, expires, EntryStatus.Pending);
        activeEntry[submission.solver] = id;
        ++pendingEntries;
        outstandingBonds += reviewReward;
        _receive(submission.solver, reviewReward, authorization, submission.deadline);
        _assertSolvent();
        emit OpenSubmissionAdded(
            bountyId,
            id,
            submission.solver,
            submission.submissionHash,
            submission.evidenceHash,
            termsHash,
            revision,
            expires,
            reviewReward
        );
    }

    function reviewAuthorized(uint64 id, bool passed, bytes32 responseHash, uint256 deadline, bytes calldata signature)
        external
        nonReentrant
    {
        require(status == Status.Open, "bounty closed");
        Entry storage entry = entries[id];
        require(entry.status == EntryStatus.Pending && block.timestamp <= entry.reviewDeadline, "review closed");
        require(responseHash != bytes32(0), "missing response");
        _deadline(deadline);
        require(deadline <= entry.reviewDeadline, "signature exceeds review window");
        require(
            _validSignature(creator, reviewDigest(id, passed, responseHash, deadline), signature),
            "invalid creator signature"
        );
        _closeEntry(entry, passed ? EntryStatus.Winner : EntryStatus.Rejected);
        if (passed) {
            status = Status.Settled;
            winner = id;
            refundablePrincipal = 0;
            settlementToken.safeTransfer(entry.solver, solverReward + reviewReward);
            settlementToken.safeTransfer(creator, reviewReward);
            emit OpenBountySettled(
                bountyId,
                id,
                entry.solver,
                solverReward,
                reviewReward,
                reviewReward,
                entry.submissionHash,
                entry.evidenceHash,
                termsHash,
                responseHash
            );
        } else {
            settlementToken.safeTransfer(creator, reviewReward);
            emit OpenSubmissionRejected(bountyId, id, entry.solver, responseHash, reviewReward);
        }
        _assertSolvent();
    }

    /// @notice Anyone can return an unreviewed bond to its exact original solver.
    function refundBond(uint64 id) external nonReentrant {
        Entry storage entry = entries[id];
        require(entry.status == EntryStatus.Pending, "bond not pending");
        require(status != Status.Open || block.timestamp > entry.reviewDeadline, "review still active");
        _closeEntry(entry, EntryStatus.Refunded);
        settlementToken.safeTransfer(entry.solver, reviewReward);
        _assertSolvent();
        emit OpenBondRefunded(bountyId, id, entry.solver, reviewReward);
    }

    function increaseRewardAuthorized(
        uint128 newReward,
        bytes32 newTermsHash,
        TokenAuthorization calldata authorization,
        uint256 deadline,
        bytes calldata signature
    ) external nonReentrant {
        require(
            status == Status.Open && entryCount == 0 && block.timestamp < submissionDeadline,
            "work already started or closed"
        );
        require(newReward > solverReward && newTermsHash != bytes32(0) && newTermsHash != termsHash, "invalid increase");
        _deadline(deadline);
        require(
            _validSignature(creator, increaseDigest(newReward, newTermsHash, authorization.nonce, deadline), signature),
            "invalid increase signature"
        );
        uint256 previousReward = solverReward;
        bytes32 previousTerms = termsHash;
        uint256 additional = uint256(newReward) - previousReward;
        solverReward = newReward;
        refundablePrincipal += additional;
        termsHash = newTermsHash;
        ++revision;
        _receive(creator, additional, authorization, deadline);
        _assertSolvent();
        emit OpenRewardIncreased(bountyId, revision, previousTerms, newTermsHash, previousReward, newReward);
    }

    function cancelAuthorized(uint256 deadline, bytes calldata signature) external nonReentrant {
        require(status == Status.Open && entryCount == 0, "work already started or closed");
        _deadline(deadline);
        require(_validSignature(creator, cancelDigest(deadline), signature), "invalid cancel signature");
        status = Status.Cancelled;
        emit OpenBountyCancelled(bountyId);
    }

    /// @notice After every possible review expires, anyone may close the bounty.
    function expire() external nonReentrant {
        require(
            status == Status.Open && block.timestamp > uint256(submissionDeadline) + reviewWindow, "bounty not expired"
        );
        status = Status.Cancelled;
        emit OpenBountyCancelled(bountyId);
    }

    function refundPrincipal() external nonReentrant {
        require(status == Status.Cancelled && refundablePrincipal > 0, "principal not refundable");
        uint256 amount = refundablePrincipal;
        refundablePrincipal = 0;
        settlementToken.safeTransfer(creator, amount);
        _assertSolvent();
        emit OpenPrincipalRefunded(bountyId, creator, amount);
    }

    function domainSeparator() public view returns (bytes32) {
        return keccak256(
            abi.encode(
                DOMAIN_TYPEHASH, keccak256("Agent Bounties Creator Open"), keccak256("1"), block.chainid, address(this)
            )
        );
    }

    function submissionDigest(Submission calldata submission, bytes32 tokenNonce) public view returns (bytes32) {
        return _digest(
            keccak256(
                abi.encode(
                    SUBMIT_TYPEHASH,
                    submission.solver,
                    submission.submissionHash,
                    submission.evidenceHash,
                    submission.termsHash,
                    submission.revision,
                    tokenNonce,
                    submission.deadline
                )
            )
        );
    }

    function reviewDigest(uint64 id, bool passed, bytes32 responseHash, uint256 deadline)
        public
        view
        returns (bytes32)
    {
        Entry storage entry = entries[id];
        return _digest(
            keccak256(
                abi.encode(
                    REVIEW_TYPEHASH,
                    id,
                    entry.solver,
                    entry.submissionHash,
                    entry.evidenceHash,
                    termsHash,
                    revision,
                    passed,
                    responseHash,
                    deadline
                )
            )
        );
    }

    function increaseDigest(uint128 newReward, bytes32 newTermsHash, bytes32 tokenNonce, uint256 deadline)
        public
        view
        returns (bytes32)
    {
        return _digest(
            keccak256(
                abi.encode(
                    INCREASE_TYPEHASH, revision, termsHash, newTermsHash, uint256(newReward), tokenNonce, deadline
                )
            )
        );
    }

    function cancelDigest(uint256 deadline) public view returns (bytes32) {
        return _digest(keccak256(abi.encode(CANCEL_TYPEHASH, revision, termsHash, deadline)));
    }

    function _digest(bytes32 payload) private view returns (bytes32) {
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator(), payload));
    }

    function _deadline(uint256 deadline) private view {
        require(deadline >= block.timestamp && deadline <= block.timestamp + 2 hours, "invalid signature expiry");
    }

    function _receive(address from, uint256 amount, TokenAuthorization calldata authorization, uint256 deadline)
        private
    {
        require(
            authorization.validBefore <= deadline && authorization.validBefore > block.timestamp, "invalid token expiry"
        );
        uint256 beforeBalance = IERC20BountyToken(settlementToken).balanceOf(address(this));
        IReceiveAuthorizedUSDC(settlementToken)
            .receiveWithAuthorization(
                from,
                address(this),
                amount,
                authorization.validAfter,
                authorization.validBefore,
                authorization.nonce,
                authorization.signature
            );
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) == beforeBalance + amount,
            "exact funding required"
        );
    }

    function _closeEntry(Entry storage entry, EntryStatus outcome) private {
        entry.status = outcome;
        activeEntry[entry.solver] = 0;
        --pendingEntries;
        outstandingBonds -= reviewReward;
    }

    function _assertSolvent() private view {
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) >= refundablePrincipal + outstandingBonds,
            "insolvent"
        );
    }

    function _validSignature(address signer, bytes32 digest, bytes calldata signature) private view returns (bool) {
        if (signer.code.length > 0) {
            (bool ok, bytes memory result) =
                signer.staticcall{gas: 200_000}(abi.encodeCall(IERC1271.isValidSignature, (digest, signature)));
            return ok && result.length >= 32 && abi.decode(result, (bytes4)) == IERC1271.isValidSignature.selector;
        }
        if (signature.length != 65) return false;
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly {
            r := calldataload(signature.offset)
            s := calldataload(add(signature.offset, 32))
            v := byte(0, calldataload(add(signature.offset, 64)))
        }
        return signer != address(0) && uint256(s) <= HALF_ORDER && (v == 27 || v == 28)
            && ecrecover(digest, v, r, s) == signer;
    }
}
