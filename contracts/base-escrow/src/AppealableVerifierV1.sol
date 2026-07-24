// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./AnonymousProtocolControllerV1.sol";
import "./IAgentBounty.sol";

interface IAppealableBountyV1 {
    function bountyId() external view returns (bytes32);
    function creator() external view returns (address);
    function settlementToken() external view returns (address);
    function verifierReward() external view returns (uint256);
    function verificationMode() external view returns (uint8);
    function verifierModule() external view returns (address);
    function verifierRewardRecipient() external view returns (address);
    function verificationWindowSeconds() external view returns (uint64);
    function verificationExpiresAt() external view returns (uint64);
    function status() external view returns (uint8);
    function round() external view returns (uint64);
    function solver() external view returns (address);
    function submissionHash() external view returns (bytes32);
    function evidenceHash() external view returns (bytes32);
    function policyHash() external view returns (bytes32);
}

/// @notice Anonymous primary-verifier assignment with symmetric, one-round
/// appeals. Wallet selection is random; the selected wallets still judge the
/// submission, and anonymous wallets may share an owner.
contract AppealableVerifierV1 is IAgentBountyVerifier {
    using SafeBountyToken for address;

    enum CaseState {
        None,
        AwaitingPrimaryRandomness,
        AwaitingPrimaryVerdict,
        PrimaryVerdict,
        AwaitingAppealRandomness,
        AppealVoting,
        Finalized,
        TimedOut
    }

    struct VerificationCase {
        address bounty;
        address creator;
        address solver;
        address primary;
        address appellant;
        bytes32 bountyId;
        bytes32 submissionHash;
        bytes32 evidenceHash;
        bytes32 policyHash;
        bytes32 primaryResponseHash;
        uint64 round;
        uint64 primaryDeadline;
        uint64 appealDeadline;
        uint64 voteDeadline;
        uint64 openedAt;
        uint256 primaryRequestId;
        uint256 appealRequestId;
        uint8 primaryCursor;
        uint8 voteCount;
        uint8 passVotes;
        uint8 failVotes;
        bool primaryVerdict;
        bool finalVerdict;
        bool appealOpened;
        bool rewardAllocated;
        CaseState state;
    }

    struct VerifyScope {
        bytes32 bountyId;
        uint64 round;
        address solver;
        bytes32 submissionHash;
        bytes32 evidenceHash;
        bytes32 policyHash;
    }

    struct SignedVerdict {
        bool passed;
        bytes32 responseHash;
        uint256 deadline;
        bytes signature;
    }

    uint256 public constant VERIFIER_REWARD = 10_000;
    uint256 public constant APPEAL_BOND = 100_000;
    uint256 public constant PRIMARY_SLASH_LOCK = 100_000;
    uint256 public constant AVAILABILITY_SLASH = 10_000;
    uint64 public constant RESPONSE_WINDOW = 30 minutes;
    uint64 public constant APPEAL_WINDOW = 4 hours;
    uint64 public constant VOTING_WINDOW = 2 hours;
    uint64 public constant REQUIRED_BOUNTY_VERIFICATION_WINDOW = 24 hours;
    uint64 public constant VRF_FULFILLMENT_WINDOW = 2 hours;
    uint64 public constant CASE_COMPLETION_BUFFER = 10 minutes;
    uint64 public constant MINIMUM_CASE_REMAINING = VRF_FULFILLMENT_WINDOW + uint64(PRIMARY_RANKING_SIZE)
        * RESPONSE_WINDOW + APPEAL_WINDOW + VRF_FULFILLMENT_WINDOW + VOTING_WINDOW + CASE_COMPLETION_BUFFER;
    uint8 public constant PRIMARY_RANKING_SIZE = 4;
    uint8 public constant APPELLATE_SIZE = 5;
    uint8 public constant APPELLATE_THRESHOLD = 3;
    uint8 public constant MINIMUM_VERIFIER_POOL = 8;
    uint8 public constant DETERMINISTIC_MODULE_MODE = 0;
    uint8 public constant CLAIMABLE_STATUS = 1;
    uint8 public constant SUBMITTED_STATUS = 3;
    uint8 public constant SETTLED_STATUS = 4;
    bytes32 public constant VERDICT_TYPEHASH = keccak256(
        "AnonymousVerdict(bytes32 caseId,address verifier,bool passed,bytes32 responseHash,uint256 deadline)"
    );
    bytes32 public constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 public constant NAME_HASH = keccak256("Agent Bounties Anonymous Verifier");
    bytes32 public constant VERSION_HASH = keccak256("1");
    uint256 private constant SECP256K1N_DIV_2 = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;

    address public immutable settlementToken;
    AnonymousProtocolControllerV1 public immutable controller;
    VrfSortitionCoordinatorV1 public immutable sortition;

    mapping(bytes32 => VerificationCase) private _cases;
    mapping(bytes32 => address[]) private _primaryRanking;
    mapping(bytes32 => address[]) private _appellateWallets;
    mapping(bytes32 => mapping(address => bool)) public isAppellate;
    mapping(bytes32 => mapping(address => bool)) public voted;
    mapping(address => uint256) public credits;
    uint256 public reservedBalance;
    bool private _entered;

    event VerificationCaseOpened(bytes32 indexed caseId, address indexed bounty, uint256 requestId);
    event PrimaryAssigned(bytes32 indexed caseId, address indexed primary, uint64 deadline, uint8 rank);
    event PrimaryAvailabilityFailed(bytes32 indexed caseId, address indexed primary, uint8 rank);
    event PrimaryVerdictSubmitted(
        bytes32 indexed caseId, address indexed primary, bool passed, bytes32 responseHash, uint64 appealDeadline
    );
    event AppealOpened(bytes32 indexed caseId, address indexed appellant, uint256 requestId, uint256 bond);
    event AppealWaived(bytes32 indexed caseId, address indexed eligibleAppellant);
    event AppealJuryAssigned(bytes32 indexed caseId, address[] wallets, uint64 voteDeadline);
    event AppealVoteSubmitted(bytes32 indexed caseId, address indexed voter, bool passed);
    event VerificationFinalized(bytes32 indexed caseId, bool passed, bool appealed, bool overturned);
    event VerificationTimedOut(bytes32 indexed caseId, bytes32 reason);
    event VerifierRewardAllocated(bytes32 indexed caseId, uint256 amount);
    event CreditWithdrawn(address indexed recipient, uint256 amount);
    event ContradictorySignaturesProved(bytes32 indexed caseId, address indexed verifier);

    modifier nonReentrant() {
        require(!_entered, "reentrant");
        _entered = true;
        _;
        _entered = false;
    }

    constructor(address settlementToken_, address controller_, address sortition_) {
        require(settlementToken_ != address(0), "token zero");
        require(controller_.code.length > 0 && sortition_.code.length > 0, "component missing");
        settlementToken = settlementToken_;
        controller = AnonymousProtocolControllerV1(controller_);
        sortition = VrfSortitionCoordinatorV1(sortition_);
    }

    function caseIdFor(address bounty) public view returns (bytes32) {
        IAppealableBountyV1 item = IAppealableBountyV1(bounty);
        VerifyScope memory scope = VerifyScope({
            bountyId: item.bountyId(),
            round: item.round(),
            solver: item.solver(),
            submissionHash: item.submissionHash(),
            evidenceHash: item.evidenceHash(),
            policyHash: item.policyHash()
        });
        return keccak256(abi.encode(address(this), block.chainid, bounty, scope));
    }

    function appealPolicyHash() external view returns (bytes32) {
        return keccak256(
            abi.encode(
                keccak256("agent-bounties/appeal-policy-v1"),
                address(this),
                address(controller.stakePool()),
                address(sortition),
                APPEAL_BOND,
                APPEAL_WINDOW,
                VOTING_WINDOW,
                PRIMARY_RANKING_SIZE,
                APPELLATE_SIZE,
                APPELLATE_THRESHOLD
            )
        );
    }

    function openCase(address bounty) external nonReentrant returns (bytes32 caseId, uint256 requestId) {
        IAppealableBountyV1 item = IAppealableBountyV1(bounty);
        _validateBounty(item);
        caseId = caseIdFor(bounty);
        require(_cases[caseId].state == CaseState.None, "case exists");

        address[] memory exclusions = new address[](2);
        exclusions[0] = item.creator();
        exclusions[1] = item.solver();
        address[] memory candidates = controller.eligibleVerifierWallets(exclusions);
        require(candidates.length >= MINIMUM_VERIFIER_POOL, "verifier pool too small");
        bytes32 commitment = keccak256(abi.encode("primary", caseId, keccak256(abi.encode(candidates))));
        requestId = controller.requestVerifierSortition(commitment, candidates, PRIMARY_RANKING_SIZE);
        _recordOpenedCase(caseId, bounty, item, requestId);
        emit VerificationCaseOpened(caseId, bounty, requestId);
    }

    function activatePrimary(bytes32 caseId) external nonReentrant {
        VerificationCase storage item = _cases[caseId];
        require(item.state == CaseState.AwaitingPrimaryRandomness, "primary activation unavailable");
        address[] memory ranking = sortition.selected(item.primaryRequestId);
        require(ranking.length == PRIMARY_RANKING_SIZE, "primary ranking invalid");
        for (uint256 i = 0; i < ranking.length; i++) {
            _primaryRanking[caseId].push(ranking[i]);
        }
        item.state = CaseState.AwaitingPrimaryVerdict;
        _assignPrimary(caseId, item, 0);
    }

    function timeoutPrimaryRandomness(bytes32 caseId) external nonReentrant {
        VerificationCase storage item = _cases[caseId];
        require(item.state == CaseState.AwaitingPrimaryRandomness, "primary timeout unavailable");
        VrfSortitionCoordinatorV1.Request memory request = sortition.requestStatus(item.primaryRequestId);
        require(
            request.requestedAt != 0
                && block.timestamp > uint256(request.requestedAt) + sortition.FULFILLMENT_DEADLINE()
                && (!request.fulfilled || request.late),
            "primary timeout pending"
        );
        item.state = CaseState.TimedOut;
        emit VerificationTimedOut(caseId, keccak256("primary-randomness-timeout"));
    }

    function promotePrimary(bytes32 caseId) external nonReentrant {
        VerificationCase storage item = _cases[caseId];
        require(item.state == CaseState.AwaitingPrimaryVerdict, "primary promotion unavailable");
        require(block.timestamp > item.primaryDeadline, "primary response pending");
        address failed = item.primary;
        controller.slashVerifierStake(caseId, failed, AVAILABILITY_SLASH, address(controller.stakePool()));
        controller.releaseVerifierStake(caseId, failed);
        emit PrimaryAvailabilityFailed(caseId, failed, item.primaryCursor);

        uint8 next = item.primaryCursor + 1;
        if (next >= PRIMARY_RANKING_SIZE) {
            item.state = CaseState.TimedOut;
            emit VerificationTimedOut(caseId, keccak256("primary-ranking-exhausted"));
            return;
        }
        _assignPrimary(caseId, item, next);
    }

    function submitPrimaryVerdict(bytes32 caseId, bool passed, bytes32 responseHash) external nonReentrant {
        VerificationCase storage item = _cases[caseId];
        require(item.state == CaseState.AwaitingPrimaryVerdict, "primary verdict unavailable");
        require(msg.sender == item.primary && block.timestamp <= item.primaryDeadline, "primary unauthorized");
        require(responseHash != bytes32(0), "response hash zero");
        item.primaryVerdict = passed;
        item.primaryResponseHash = responseHash;
        item.appealDeadline = uint64(block.timestamp) + APPEAL_WINDOW;
        item.state = CaseState.PrimaryVerdict;
        emit PrimaryVerdictSubmitted(caseId, msg.sender, passed, responseHash, item.appealDeadline);
    }

    function openAppeal(bytes32 caseId) external nonReentrant {
        VerificationCase storage item = _cases[caseId];
        require(item.state == CaseState.PrimaryVerdict && block.timestamp <= item.appealDeadline, "appeal unavailable");
        address eligibleAppellant = item.primaryVerdict ? item.creator : item.solver;
        require(msg.sender == eligibleAppellant, "appellant ineligible");
        settlementToken.safeTransferFrom(msg.sender, address(this), APPEAL_BOND);
        reservedBalance += APPEAL_BOND;

        address[] memory exclusions = new address[](3);
        exclusions[0] = item.primary;
        exclusions[1] = item.solver;
        exclusions[2] = item.creator;
        address[] memory candidates = controller.eligibleVerifierWallets(exclusions);
        require(candidates.length >= APPELLATE_SIZE, "appellate pool too small");
        bytes32 commitment = keccak256(abi.encode("appeal", caseId, keccak256(abi.encode(candidates))));
        uint256 requestId = controller.requestVerifierSortition(commitment, candidates, uint8(candidates.length));
        item.appellant = msg.sender;
        item.appealOpened = true;
        item.appealRequestId = requestId;
        item.state = CaseState.AwaitingAppealRandomness;
        emit AppealOpened(caseId, msg.sender, requestId, APPEAL_BOND);
    }

    /// @notice Lets the only eligible appellant waive the remaining appeal
    /// window so an undisputed verdict can finalize in the same block.
    function waiveAppeal(bytes32 caseId) external nonReentrant {
        VerificationCase storage item = _cases[caseId];
        require(
            item.state == CaseState.PrimaryVerdict && block.timestamp <= item.appealDeadline,
            "appeal waiver unavailable"
        );
        address eligibleAppellant = item.primaryVerdict ? item.creator : item.solver;
        require(msg.sender == eligibleAppellant, "appeal waiver ineligible");
        emit AppealWaived(caseId, msg.sender);
        _finalizePrimary(caseId, item);
    }

    function activateAppeal(bytes32 caseId) external nonReentrant {
        VerificationCase storage item = _cases[caseId];
        require(item.state == CaseState.AwaitingAppealRandomness, "appeal activation unavailable");
        address[] memory ranking = sortition.ranking(item.appealRequestId);
        require(ranking.length >= APPELLATE_SIZE, "appellate ranking unavailable");

        address[] memory selectedWallets = new address[](APPELLATE_SIZE);
        uint256 selectedCount;
        for (uint256 i = 0; i < ranking.length && selectedCount < APPELLATE_SIZE; i++) {
            address wallet = ranking[i];
            try controller.lockVerifierStake(_juryLockId(caseId, wallet), wallet, AVAILABILITY_SLASH) {
                selectedWallets[selectedCount] = wallet;
                _appellateWallets[caseId].push(wallet);
                isAppellate[caseId][wallet] = true;
                selectedCount += 1;
            } catch {}
        }
        require(selectedCount == APPELLATE_SIZE, "five appellate locks unavailable");
        item.voteDeadline = uint64(block.timestamp) + VOTING_WINDOW;
        item.state = CaseState.AppealVoting;
        emit AppealJuryAssigned(caseId, selectedWallets, item.voteDeadline);
    }

    function submitAppealVote(bytes32 caseId, bool passed) external nonReentrant {
        VerificationCase storage item = _cases[caseId];
        require(item.state == CaseState.AppealVoting && block.timestamp <= item.voteDeadline, "appeal vote unavailable");
        require(isAppellate[caseId][msg.sender] && !voted[caseId][msg.sender], "appeal voter unauthorized");
        voted[caseId][msg.sender] = true;
        item.voteCount += 1;
        if (passed) item.passVotes += 1;
        else item.failVotes += 1;
        emit AppealVoteSubmitted(caseId, msg.sender, passed);
    }

    function finalizeUnappealed(bytes32 caseId) external nonReentrant {
        VerificationCase storage item = _cases[caseId];
        require(item.state == CaseState.PrimaryVerdict && block.timestamp > item.appealDeadline, "appeal window open");
        _finalizePrimary(caseId, item);
    }

    function _finalizePrimary(bytes32 caseId, VerificationCase storage item) private {
        item.finalVerdict = item.primaryVerdict;
        item.state = CaseState.Finalized;
        controller.releaseVerifierStake(caseId, item.primary);
        emit VerificationFinalized(caseId, item.finalVerdict, false, false);
    }

    function finalizeAppeal(bytes32 caseId) external nonReentrant {
        VerificationCase storage item = _cases[caseId];
        require(item.state == CaseState.AppealVoting, "appeal finalization unavailable");
        bool decisive = item.passVotes >= APPELLATE_THRESHOLD || item.failVotes >= APPELLATE_THRESHOLD;
        require(decisive || block.timestamp > item.voteDeadline, "appeal voting open");
        if (item.voteCount < APPELLATE_THRESHOLD || item.passVotes == item.failVotes) {
            _timeoutAppeal(caseId, item, keccak256("appellate-quorum-unavailable"));
            return;
        }

        item.finalVerdict = item.passVotes > item.failVotes;
        bool overturned = item.finalVerdict != item.primaryVerdict;
        if (overturned) {
            _credit(item.appellant, APPEAL_BOND);
            controller.slashVerifierStake(caseId, item.primary, PRIMARY_SLASH_LOCK, address(this));
            reservedBalance += PRIMARY_SLASH_LOCK;
            _shareAmongVoters(caseId, PRIMARY_SLASH_LOCK);
        } else {
            controller.releaseVerifierStake(caseId, item.primary);
            _shareAmongVoters(caseId, APPEAL_BOND);
        }
        _closeJuryLocks(caseId);
        item.state = CaseState.Finalized;
        emit VerificationFinalized(caseId, item.finalVerdict, true, overturned);
    }

    function timeoutAppeal(bytes32 caseId) external nonReentrant {
        VerificationCase storage item = _cases[caseId];
        require(
            item.state == CaseState.AwaitingAppealRandomness || item.state == CaseState.AppealVoting,
            "appeal timeout unavailable"
        );
        VrfSortitionCoordinatorV1.Request memory request = sortition.requestStatus(item.appealRequestId);
        bool randomnessLate = item.state == CaseState.AwaitingAppealRandomness && request.requestedAt != 0
            && block.timestamp > uint256(request.requestedAt) + sortition.FULFILLMENT_DEADLINE()
            && (!request.fulfilled || request.late);
        bool votingLate = item.state == CaseState.AppealVoting && block.timestamp > item.voteDeadline;
        require(randomnessLate || votingLate, "appeal timeout pending");
        _timeoutAppeal(caseId, item, keccak256("appeal-timeout"));
    }

    function allocateVerifierReward(bytes32 caseId) external nonReentrant {
        VerificationCase storage item = _cases[caseId];
        require(item.state == CaseState.Finalized && !item.rewardAllocated, "reward allocation unavailable");
        uint8 expectedStatus = item.finalVerdict ? SETTLED_STATUS : CLAIMABLE_STATUS;
        require(IAppealableBountyV1(item.bounty).status() == expectedStatus, "bounty verdict not executed");
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) >= reservedBalance + VERIFIER_REWARD,
            "reward not received"
        );
        item.rewardAllocated = true;
        reservedBalance += VERIFIER_REWARD;
        if (!item.appealOpened || item.finalVerdict == item.primaryVerdict) {
            _credit(item.primary, VERIFIER_REWARD);
        } else {
            _shareAmongVoters(caseId, VERIFIER_REWARD);
        }
        emit VerifierRewardAllocated(caseId, VERIFIER_REWARD);
    }

    function withdrawCredit() external nonReentrant {
        uint256 amount = credits[msg.sender];
        require(amount > 0, "credit empty");
        credits[msg.sender] = 0;
        reservedBalance -= amount;
        settlementToken.safeTransfer(msg.sender, amount);
        emit CreditWithdrawn(msg.sender, amount);
    }

    function verify(
        bytes32 bountyId,
        uint64 round,
        address solver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 policyHash,
        bytes calldata proof
    ) external view returns (bool passed, bytes32 responseHash) {
        require(proof.length == 32, "case proof invalid");
        bytes32 caseId = abi.decode(proof, (bytes32));
        VerifyScope memory scope = VerifyScope({
            bountyId: bountyId,
            round: round,
            solver: solver,
            submissionHash: submissionHash,
            evidenceHash: evidenceHash,
            policyHash: policyHash
        });
        return _finalizedVerdict(caseId, scope);
    }

    function _finalizedVerdict(bytes32 caseId, VerifyScope memory scope)
        private
        view
        returns (bool passed, bytes32 responseHash)
    {
        VerificationCase storage item = _cases[caseId];
        require(item.state == CaseState.Finalized, "verification not finalized");
        require(_scopeMatches(item, scope), "verification scope mismatch");
        passed = item.finalVerdict;
        responseHash = _responseHash(caseId, item);
    }

    function verdictDigest(bytes32 caseId, address verifier, bool passed, bytes32 responseHash, uint256 deadline)
        public
        view
        returns (bytes32)
    {
        bytes32 domain = keccak256(abi.encode(DOMAIN_TYPEHASH, NAME_HASH, VERSION_HASH, block.chainid, address(this)));
        bytes32 value = keccak256(abi.encode(VERDICT_TYPEHASH, caseId, verifier, passed, responseHash, deadline));
        return keccak256(abi.encodePacked("\x19\x01", domain, value));
    }

    function proveContradictorySignatures(bytes32 caseId, SignedVerdict calldata a, SignedVerdict calldata b)
        external
        nonReentrant
    {
        VerificationCase storage item = _cases[caseId];
        require(item.state == CaseState.PrimaryVerdict && !item.appealOpened, "contradiction proof unavailable");
        require(
            a.passed != b.passed && a.responseHash != bytes32(0) && b.responseHash != bytes32(0),
            "verdicts not contradictory"
        );
        require(
            _signedByPrimary(caseId, item.primary, a) && _signedByPrimary(caseId, item.primary, b),
            "contradiction signatures invalid"
        );
        controller.slashVerifierStake(caseId, item.primary, PRIMARY_SLASH_LOCK, address(controller.stakePool()));
        item.state = CaseState.TimedOut;
        emit ContradictorySignaturesProved(caseId, item.primary);
        emit VerificationTimedOut(caseId, keccak256("contradictory-signatures"));
    }

    function primaryRanking(bytes32 caseId) external view returns (address[] memory) {
        return _primaryRanking[caseId];
    }

    function appellateWallets(bytes32 caseId) external view returns (address[] memory) {
        return _appellateWallets[caseId];
    }

    function caseState(bytes32 caseId) external view returns (CaseState) {
        return _cases[caseId].state;
    }

    function caseParties(bytes32 caseId)
        external
        view
        returns (address bounty, address creator, address solver, address primary, address appellant)
    {
        VerificationCase storage item = _cases[caseId];
        return (item.bounty, item.creator, item.solver, item.primary, item.appellant);
    }

    function caseTiming(bytes32 caseId)
        external
        view
        returns (uint64 openedAt, uint64 primaryDeadline, uint64 appealDeadline, uint64 voteDeadline)
    {
        VerificationCase storage item = _cases[caseId];
        return (item.openedAt, item.primaryDeadline, item.appealDeadline, item.voteDeadline);
    }

    function caseVerdict(bytes32 caseId)
        external
        view
        returns (
            bool primaryVerdict,
            bool finalVerdict,
            bool appealOpened,
            bool rewardAllocated,
            uint8 voteCount,
            uint8 passVotes,
            uint8 failVotes,
            bytes32 primaryResponseHash
        )
    {
        VerificationCase storage item = _cases[caseId];
        return (
            item.primaryVerdict,
            item.finalVerdict,
            item.appealOpened,
            item.rewardAllocated,
            item.voteCount,
            item.passVotes,
            item.failVotes,
            item.primaryResponseHash
        );
    }

    function _validateBounty(IAppealableBountyV1 item) private view {
        require(item.settlementToken() == settlementToken, "token mismatch");
        require(item.verifierReward() == VERIFIER_REWARD, "verifier reward mismatch");
        require(
            item.verificationMode() == DETERMINISTIC_MODULE_MODE && item.verifierModule() == address(this)
                && item.verifierRewardRecipient() == address(this),
            "verifier policy mismatch"
        );
        require(item.verificationWindowSeconds() == REQUIRED_BOUNTY_VERIFICATION_WINDOW, "appeal timing mismatch");
        require(
            item.verificationExpiresAt() >= block.timestamp + MINIMUM_CASE_REMAINING, "insufficient verification time"
        );
        require(item.status() == SUBMITTED_STATUS, "bounty not submitted");
    }

    function _scopeMatches(VerificationCase storage item, VerifyScope memory scope) private view returns (bool) {
        return msg.sender == item.bounty && scope.bountyId == item.bountyId && scope.round == item.round
            && scope.solver == item.solver && scope.submissionHash == item.submissionHash
            && scope.evidenceHash == item.evidenceHash && scope.policyHash == item.policyHash;
    }

    function _responseHash(bytes32 caseId, VerificationCase storage item) private view returns (bytes32) {
        return keccak256(
            abi.encode(
                keccak256("agent-bounties/appealable-verifier-v1"),
                caseId,
                item.primary,
                item.primaryVerdict,
                item.primaryResponseHash,
                item.appellant,
                item.passVotes,
                item.failVotes,
                item.finalVerdict
            )
        );
    }

    function _signedByPrimary(bytes32 caseId, address primary, SignedVerdict calldata verdict)
        private
        view
        returns (bool)
    {
        return _recover(
            verdictDigest(caseId, primary, verdict.passed, verdict.responseHash, verdict.deadline), verdict.signature
        ) == primary;
    }

    function _recordOpenedCase(bytes32 caseId, address bounty, IAppealableBountyV1 item, uint256 requestId) private {
        VerificationCase storage opened = _cases[caseId];
        opened.bounty = bounty;
        opened.creator = item.creator();
        opened.solver = item.solver();
        opened.bountyId = item.bountyId();
        opened.submissionHash = item.submissionHash();
        opened.evidenceHash = item.evidenceHash();
        opened.policyHash = item.policyHash();
        opened.round = item.round();
        opened.openedAt = uint64(block.timestamp);
        opened.primaryRequestId = requestId;
        opened.state = CaseState.AwaitingPrimaryRandomness;
    }

    function _assignPrimary(bytes32 caseId, VerificationCase storage item, uint8 rank) private {
        for (uint8 cursor = rank; cursor < PRIMARY_RANKING_SIZE; cursor++) {
            address primary = _primaryRanking[caseId][cursor];
            try controller.lockVerifierStake(caseId, primary, PRIMARY_SLASH_LOCK) {
                item.primary = primary;
                item.primaryCursor = cursor;
                item.primaryDeadline = uint64(block.timestamp) + RESPONSE_WINDOW;
                emit PrimaryAssigned(caseId, primary, item.primaryDeadline, cursor);
                return;
            } catch {}
        }
        item.state = CaseState.TimedOut;
        emit VerificationTimedOut(caseId, keccak256("primary-ranking-unavailable"));
    }

    function _timeoutAppeal(bytes32 caseId, VerificationCase storage item, bytes32 reason) private {
        _credit(item.appellant, APPEAL_BOND);
        if (item.state == CaseState.AppealVoting) _closeJuryLocks(caseId);
        controller.releaseVerifierStake(caseId, item.primary);
        item.state = CaseState.TimedOut;
        emit VerificationTimedOut(caseId, reason);
    }

    function _closeJuryLocks(bytes32 caseId) private {
        address[] storage wallets = _appellateWallets[caseId];
        for (uint256 i = 0; i < wallets.length; i++) {
            address wallet = wallets[i];
            bytes32 lockId = _juryLockId(caseId, wallet);
            if (voted[caseId][wallet]) {
                controller.releaseVerifierStake(lockId, wallet);
            } else {
                controller.slashVerifierStake(lockId, wallet, AVAILABILITY_SLASH, address(controller.stakePool()));
            }
        }
    }

    function _shareAmongVoters(bytes32 caseId, uint256 amount) private {
        VerificationCase storage item = _cases[caseId];
        require(item.voteCount > 0, "no voters");
        uint256 share = amount / item.voteCount;
        uint256 remainder = amount - share * item.voteCount;
        address[] storage wallets = _appellateWallets[caseId];
        bool remainderAssigned;
        for (uint256 i = 0; i < wallets.length; i++) {
            address wallet = wallets[i];
            if (!voted[caseId][wallet]) continue;
            uint256 credit = share;
            if (!remainderAssigned) {
                credit += remainder;
                remainderAssigned = true;
            }
            _credit(wallet, credit);
        }
    }

    function _credit(address recipient, uint256 amount) private {
        require(recipient != address(0), "credit recipient zero");
        credits[recipient] += amount;
    }

    function _juryLockId(bytes32 caseId, address wallet) private pure returns (bytes32) {
        return keccak256(abi.encode("appellate-lock", caseId, wallet));
    }

    function _recover(bytes32 digest, bytes calldata signature) private pure returns (address recovered) {
        if (signature.length != 65) return address(0);
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly ("memory-safe") {
            r := calldataload(signature.offset)
            s := calldataload(add(signature.offset, 0x20))
            v := byte(0, calldataload(add(signature.offset, 0x40)))
        }
        if (uint256(s) > SECP256K1N_DIV_2 || (v != 27 && v != 28)) return address(0);
        recovered = ecrecover(digest, v, r, s);
    }
}
