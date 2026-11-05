// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;
import "./IAgentBounty.sol";

/// @notice Fixed-cutoff mixed verification. Experimental: not an activated
/// canonical deployment. No new solver bond, platform fee, or admin override.
/// Runner signatures are trusted execution evidence, not computation proofs.
contract OpenCompetitionBountyV3 {
    using SafeBountyToken for address;
    enum State {
        Funding,
        Active,
        Settled,
        Cancelled
    }
    enum Assurance {
        TrustedRunner,
        ComputationProof
    }

    struct Checker {
        address authority;
        Assurance assurance;
    }

    struct Config {
        address creator;
        address token;
        bytes32 bountyId;
        bytes32 termsHash;
        bytes32 planHash;
        uint256 reward;
        uint64 fundingCutoff;
        uint64 submissionCutoff;
        uint64 checksCutoff;
        uint64 reviewCutoff;
        uint64 finalCutoff;
        uint16 maximumEntries;
        Checker[] checks;
        address[] reviewers;
        uint8 reviewThreshold;
        uint64 requiredReviewMask;
    }

    struct Entry {
        address solver;
        bytes32 manifestHash;
        uint64 checkedMask;
        uint64 passedMask;
        uint16 reviewerMask;
        uint8 approvals;
        uint8 rejections;
        bool reviewFailed;
        bool scoreSet;
        int256 score;
    }
    bytes32 public constant PROTOCOL_VERSION = keccak256("agent-bounties/open-competition-v3-experimental");
    bytes32 private constant CHECK_TYPEHASH = keccak256(
        "Check(bytes32 planHash,uint256 entryId,bytes32 manifestHash,uint8 checkIndex,bool passed,bytes32 evidenceHash)"
    );
    bytes32 private constant REVIEW_TYPEHASH = keccak256(
        "Review(bytes32 planHash,uint256 entryId,bytes32 manifestHash,uint64 criteriaMask,bool passed,int256 score,bytes32 reportHash)"
    );
    uint256 private constant HALF_ORDER = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;
    address public immutable creator;
    address public immutable settlementToken;
    bytes32 public immutable bountyId;
    bytes32 public immutable termsHash;
    bytes32 public immutable planHash;
    uint256 public immutable targetAmount;
    uint64 public immutable fundingCutoff;
    uint64 public immutable submissionCutoff;
    uint64 public immutable checksCutoff;
    uint64 public immutable reviewCutoff;
    uint64 public immutable finalCutoff;
    uint16 public immutable maximumEntries;
    uint8 public immutable reviewThreshold;
    uint64 public immutable requiredReviewMask;
    uint64 public immutable requiredCheckMask;
    State public state;
    uint256 public fundedAmount;
    uint256 public totalRefunded;
    uint256 public paidAmount;
    uint256 public winningEntry;
    uint256 public unresolvedEntries;
    Checker[] public checks;
    address[] public reviewers;
    Entry[] public entries;
    mapping(address => uint256) public contributions;
    mapping(address => bool) public entered;
    mapping(uint256 => mapping(int256 => uint8)) public scoreVotes;
    bool private enteredCall;
    event Funded(address indexed funder, uint256 amount);
    event SubmissionCommitted(
        uint256 indexed entryId, address indexed solver, bytes32 manifestHash, uint64 submittedAt
    );
    event CheckRecorded(
        uint256 indexed entryId, uint8 indexed checkIndex, bool passed, bytes32 evidenceHash, Assurance assurance
    );
    event ReviewRecorded(
        uint256 indexed entryId, address indexed reviewer, bool passed, int256 score, bytes32 reportHash, bool recovery
    );
    event CompetitionSettledV3(
        bytes32 indexed bountyId, uint256 indexed entryId, address indexed solver, bytes32 manifestHash, uint256 amount
    );
    event CompetitionClosedWithoutWinner(bytes32 indexed bountyId, bool unresolved);
    event RefundWithdrawn(address indexed funder, uint256 amount);
    modifier nonReentrant() {
        require(!enteredCall, "reentrant");
        enteredCall = true;
        _;
        enteredCall = false;
    }

    constructor(Config memory c) {
        require(c.creator != address(0) && c.token.code.length > 0 && c.reward > 0, "invalid funding");
        require(
            c.bountyId != bytes32(0) && c.termsHash != bytes32(0) && c.planHash != bytes32(0), "missing commitments"
        );
        require(
            c.fundingCutoff > block.timestamp && c.fundingCutoff < c.submissionCutoff, "fund before submissions close"
        );
        require(c.submissionCutoff <= block.timestamp + 366 days, "cutoff too far");
        require(c.maximumEntries > 0 && c.maximumEntries <= 128, "entry capacity");
        require(c.checks.length <= 64 && c.reviewers.length <= 16, "verification capacity");
        require(c.checks.length > 0 || c.reviewers.length > 0, "verification required");
        require(
            c.checks.length == 0
                ? c.checksCutoff == c.submissionCutoff
                : c.checksCutoff >= c.submissionCutoff + 24 hours,
            "check window"
        );
        require(
            c.reviewers.length == 0 ? c.reviewCutoff == c.checksCutoff : c.reviewCutoff >= c.checksCutoff + 48 hours,
            "review window"
        );
        require(
            c.reviewers.length == 0 ? c.finalCutoff == c.reviewCutoff : c.finalCutoff == c.reviewCutoff + 24 hours,
            "recovery window"
        );
        require(c.finalCutoff <= c.submissionCutoff + 30 days, "windows too long");
        if (c.reviewers.length == 0) {
            require(c.reviewThreshold == 0 && c.requiredReviewMask == 0, "unused review");
        } else {
            require(
                c.reviewThreshold > c.reviewers.length / 2 && c.reviewThreshold <= c.reviewers.length
                    && c.requiredReviewMask != 0,
                "review quorum"
            );
        }
        for (uint256 i; i < c.checks.length; i++) {
            require(c.checks[i].authority != address(0), "missing checker");
            if (c.checks[i].assurance == Assurance.ComputationProof) {
                require(c.checks[i].authority.code.length > 0, "proof adapter missing");
            }
            checks.push(c.checks[i]);
        }
        for (uint256 i; i < c.reviewers.length; i++) {
            require(c.reviewers[i] != address(0), "missing reviewer");
            for (uint256 j; j < i; j++) {
                require(c.reviewers[i] != c.reviewers[j], "duplicate reviewer");
            }
            reviewers.push(c.reviewers[i]);
        }
        creator = c.creator;
        settlementToken = c.token;
        bountyId = c.bountyId;
        termsHash = c.termsHash;
        planHash = c.planHash;
        targetAmount = c.reward;
        fundingCutoff = c.fundingCutoff;
        submissionCutoff = c.submissionCutoff;
        checksCutoff = c.checksCutoff;
        reviewCutoff = c.reviewCutoff;
        finalCutoff = c.finalCutoff;
        maximumEntries = c.maximumEntries;
        reviewThreshold = c.reviewThreshold;
        requiredReviewMask = c.requiredReviewMask;
        requiredCheckMask = c.checks.length == 64 ? type(uint64).max : uint64((uint256(1) << c.checks.length) - 1);
    }

    function domainSeparator() public view returns (bytes32) {
        return keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("Agent Bounties Mixed Verification"),
                keccak256("3"),
                block.chainid,
                address(this)
            )
        );
    }

    function _typed(bytes32 h) private view returns (bytes32) {
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator(), h));
    }

    function checkDigest(uint256 id, uint8 index, bool passed, bytes32 evidenceHash) public view returns (bytes32) {
        return _typed(
            keccak256(abi.encode(CHECK_TYPEHASH, planHash, id, entries[id].manifestHash, index, passed, evidenceHash))
        );
    }

    function reviewDigest(uint256 id, uint64 mask, bool passed, int256 score, bytes32 reportHash)
        public
        view
        returns (bytes32)
    {
        return _typed(
            keccak256(
                abi.encode(REVIEW_TYPEHASH, planHash, id, entries[id].manifestHash, mask, passed, score, reportHash)
            )
        );
    }

    function fund(uint256 amount) external nonReentrant {
        require(
            state == State.Funding && block.timestamp < fundingCutoff && amount > 0
                && amount <= targetAmount - fundedAmount,
            "funding closed"
        );
        uint256 beforeBalance = IERC20BountyToken(settlementToken).balanceOf(address(this));
        settlementToken.safeTransferFrom(msg.sender, address(this), amount);
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) == beforeBalance + amount, "token accounting"
        );
        contributions[msg.sender] += amount;
        fundedAmount += amount;
        if (fundedAmount == targetAmount) state = State.Active;
        emit Funded(msg.sender, amount);
    }

    function commitSubmission(bytes32 manifestHash) external {
        require(state == State.Active && block.timestamp <= submissionCutoff, "submission closed");
        require(
            manifestHash != bytes32(0) && !entered[msg.sender] && entries.length < maximumEntries, "entry unavailable"
        );
        entered[msg.sender] = true;
        entries.push(Entry(msg.sender, manifestHash, 0, 0, 0, 0, 0, false, false, 0));
        unresolvedEntries++;
        emit SubmissionCommitted(entries.length - 1, msg.sender, manifestHash, uint64(block.timestamp));
    }

    function recordCheck(uint256 id, uint8 index, bool passed, bytes32 evidenceHash, bytes calldata proof)
        external
        nonReentrant
    {
        require(
            state == State.Active && block.timestamp > submissionCutoff && block.timestamp <= checksCutoff,
            "check window closed"
        );
        require(index < checks.length && evidenceHash != bytes32(0), "invalid check");
        Entry storage e = entries[id];
        uint64 bit = uint64(1) << index;
        require(e.checkedMask & bit == 0, "check already recorded");
        bytes32 digest = checkDigest(id, index, passed, evidenceHash);
        Checker memory checker = checks[index];
        if (checker.assurance == Assurance.TrustedRunner) {
            require(_valid(checker.authority, digest, proof), "invalid runner signature");
        } else {
            (bool ok, bytes memory result) = checker.authority.staticcall{gas: 500000}(
                abi.encodeWithSignature("verifyCheck(bytes32,bytes)", digest, proof)
            );
            require(ok && result.length == 32 && abi.decode(result, (bool)), "invalid computation proof");
        }
        bool resolvedBefore = _resolved(e);
        e.checkedMask |= bit;
        if (passed) e.passedMask |= bit;
        if (!resolvedBefore && _resolved(e)) unresolvedEntries--;
        emit CheckRecorded(id, index, passed, evidenceHash, checker.assurance);
    }

    function recordReview(
        uint256 id,
        uint8 reviewerIndex,
        uint64 mask,
        bool passed,
        int256 score,
        bytes32 reportHash,
        bytes calldata signature
    ) external nonReentrant {
        require(
            state == State.Active && block.timestamp >= checksCutoff && block.timestamp <= finalCutoff,
            "review window closed"
        );
        require(
            reviewerIndex < reviewers.length && mask == requiredReviewMask && reportHash != bytes32(0),
            "incomplete review"
        );
        Entry storage e = entries[id];
        require(e.checkedMask == requiredCheckMask && e.passedMask == requiredCheckMask, "mandatory check not passed");
        uint16 bit = uint16(1) << reviewerIndex;
        require(e.reviewerMask & bit == 0, "review already recorded");
        require(
            _valid(reviewers[reviewerIndex], reviewDigest(id, mask, passed, score, reportHash), signature),
            "invalid reviewer signature"
        );
        bool resolvedBefore = _resolved(e);
        e.reviewerMask |= bit;
        // Quorums agree on the verdict and score. A single reviewer cannot
        // defeat a larger quorum or lock its score with the first response.
        if (!passed) {
            e.rejections++;
            e.reviewFailed = e.rejections >= reviewThreshold;
        } else {
            uint8 votes = ++scoreVotes[id][score];
            if (votes > e.approvals) e.approvals = votes;
            if (votes >= reviewThreshold) {
                e.score = score;
                e.scoreSet = true;
            }
        }
        if (!resolvedBefore && _resolved(e)) unresolvedEntries--;
        emit ReviewRecorded(id, reviewers[reviewerIndex], passed, score, reportHash, block.timestamp > reviewCutoff);
    }

    function _resolved(Entry storage e) private view returns (bool) {
        return (e.checkedMask & ~e.passedMask) != 0 || e.reviewFailed
            || (e.checkedMask == requiredCheckMask && e.approvals >= reviewThreshold);
    }

    function qualified(uint256 id) public view returns (bool) {
        Entry storage e = entries[id];
        return e.checkedMask == requiredCheckMask && e.passedMask == requiredCheckMask && !e.reviewFailed
            && e.approvals >= reviewThreshold;
    }

    function finalize() external nonReentrant {
        require(state == State.Active && block.timestamp > reviewCutoff, "review still open");
        if (unresolvedEntries > 0) {
            require(block.timestamp > finalCutoff, "unresolved reviews");
            state = State.Cancelled;
            emit CompetitionClosedWithoutWinner(bountyId, true);
            return;
        }
        bool found;
        uint256 best;
        for (uint256 i; i < entries.length; i++) {
            if (qualified(i) && (!found || entries[i].score > entries[best].score)) {
                best = i;
                found = true;
            }
        }
        if (!found) {
            state = State.Cancelled;
            emit CompetitionClosedWithoutWinner(bountyId, false);
            return;
        }
        state = State.Settled;
        winningEntry = best;
        paidAmount = fundedAmount;
        settlementToken.safeTransfer(entries[best].solver, paidAmount);
        emit CompetitionSettledV3(bountyId, best, entries[best].solver, entries[best].manifestHash, paidAmount);
    }

    function cancelUnfunded() external {
        require(state == State.Funding && block.timestamp >= fundingCutoff, "funding still open");
        state = State.Cancelled;
        emit CompetitionClosedWithoutWinner(bountyId, false);
    }

    function withdrawRefund() external nonReentrant {
        require(state == State.Cancelled, "refund unavailable");
        uint256 amount = contributions[msg.sender];
        require(amount > 0, "no refund");
        contributions[msg.sender] = 0;
        totalRefunded += amount;
        settlementToken.safeTransfer(msg.sender, amount);
        emit RefundWithdrawn(msg.sender, amount);
    }

    function entryCount() external view returns (uint256) {
        return entries.length;
    }

    function _valid(address signer, bytes32 digest, bytes calldata sig) private view returns (bool) {
        if (signer.code.length > 0) {
            (bool ok, bytes memory result) =
                signer.staticcall{gas: 200000}(abi.encodeCall(IERC1271.isValidSignature, (digest, sig)));
            return ok && result.length == 32 && abi.decode(result, (bytes4)) == 0x1626ba7e;
        }
        if (sig.length != 65) return false;
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly {
            r := calldataload(sig.offset)
            s := calldataload(add(sig.offset, 32))
            v := byte(0, calldataload(add(sig.offset, 64)))
        }
        if (uint256(s) > HALF_ORDER || (v != 27 && v != 28)) return false;
        return ecrecover(digest, v, r, s) == signer;
    }
}
