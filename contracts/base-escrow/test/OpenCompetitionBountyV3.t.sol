// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;
import "../src/OpenCompetitionBountyV3.sol";

interface VmMixed {
    function warp(uint256) external;
    function prank(address) external;
    function addr(uint256) external returns (address);
    function sign(uint256, bytes32) external returns (uint8, bytes32, bytes32);
}

contract MixedToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address a, uint256 n) external {
        balanceOf[a] += n;
    }

    function approve(address a, uint256 n) external returns (bool) {
        allowance[msg.sender][a] = n;
        return true;
    }

    function transfer(address a, uint256 n) external returns (bool) {
        balanceOf[msg.sender] -= n;
        balanceOf[a] += n;
        return true;
    }

    function transferFrom(address a, address b, uint256 n) external returns (bool) {
        allowance[a][msg.sender] -= n;
        balanceOf[a] -= n;
        balanceOf[b] += n;
        return true;
    }
}

contract OpenCompetitionBountyV3Test {
    VmMixed constant vm = VmMixed(address(uint160(uint256(keccak256("hevm cheat code")))));
    MixedToken token;
    OpenCompetitionBountyV3 bounty;
    uint256 constant RUNNER = 21;
    uint256 constant REVIEWER = 22;
    uint64 constant D = 1000000;
    address constant A = address(0xA);
    address constant B = address(0xB);

    function config(uint8 checkerCount, uint8 reviewerCount)
        internal
        returns (OpenCompetitionBountyV3.Config memory c)
    {
        c.creator = address(this);
        c.token = address(token);
        c.bountyId = keccak256("bounty");
        c.termsHash = keccak256("terms");
        c.planHash = keccak256("fixed plan");
        c.reward = 17000000;
        c.fundingCutoff = D - 1 days;
        c.submissionCutoff = D;
        c.checksCutoff = checkerCount == 0 ? D : D + 1 days;
        c.reviewCutoff = reviewerCount == 0 ? c.checksCutoff : c.checksCutoff + 2 days;
        c.finalCutoff = reviewerCount == 0 ? c.reviewCutoff : c.reviewCutoff + 1 days;
        c.maximumEntries = 128;
        c.checks = new OpenCompetitionBountyV3.Checker[](checkerCount);
        for (uint8 i; i < checkerCount; i++) {
            c.checks[i] =
                OpenCompetitionBountyV3.Checker(vm.addr(RUNNER), OpenCompetitionBountyV3.Assurance.TrustedRunner);
        }
        c.reviewers = new address[](reviewerCount);
        for (uint8 i; i < reviewerCount; i++) {
            c.reviewers[i] = vm.addr(REVIEWER + i);
        }
        c.reviewThreshold = reviewerCount == 0 ? 0 : reviewerCount / 2 + 1;
        c.requiredReviewMask = reviewerCount == 0 ? 0 : 3;
    }

    function setUp() public {
        vm.warp(100);
        token = new MixedToken();
        bounty = new OpenCompetitionBountyV3(config(2, 1));
        token.mint(address(this), 34000000);
        token.approve(address(bounty), 17000000);
        bounty.fund(17000000);
    }

    function signature(uint256 key, bytes32 hash) internal returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, hash);
        return abi.encodePacked(r, s, v);
    }

    function entry(address solver) internal {
        vm.prank(solver);
        bounty.commitSubmission(keccak256(abi.encode(solver)));
    }

    function check(uint256 id, uint8 index, bool pass) internal {
        bytes32 evidence = keccak256("actual checked files");
        bounty.recordCheck(id, index, pass, evidence, signature(RUNNER, bounty.checkDigest(id, index, pass, evidence)));
    }

    function review(uint256 id, bool pass, int256 score) internal {
        bytes32 report = keccak256("stored report");
        bytes32 hash = bounty.reviewDigest(id, 3, pass, score, report);
        bytes memory sig = signature(REVIEWER, hash);
        bounty.recordReview(id, 0, 3, pass, score, report, sig);
    }

    function tryReview(uint256 id, bool pass, int256 score, bytes32 h) internal returns (bool) {
        bytes32 hash = bounty.reviewDigest(id, 3, pass, score, h);
        bytes memory sig = signature(REVIEWER, hash);
        bytes memory data = abi.encodeCall(bounty.recordReview, (id, 0, 3, pass, score, h, sig));
        (bool ok,) = address(bounty).call(data);
        return ok;
    }

    function testEveryRequiredCheckAndReviewNeeded() public {
        entry(A);
        vm.warp(D + 1);
        check(0, 0, true);
        vm.warp(D + 1 days);
        bytes32 h = keccak256("report");
        bool ok = tryReview(0, true, 10, h);
        require(!ok, "omitted mandatory check accepted");
        check(0, 1, true);
        review(0, true, 10);
        vm.warp(D + 3 days + 1);
        bounty.finalize();
        require(token.balanceOf(A) == 17000000, "wrong payout");
        require(uint8(bounty.state()) == 2, "not settled");
    }

    function testRejectsOverridingFailedMandatoryCheck() public {
        entry(A);
        vm.warp(D + 1);
        check(0, 0, false);
        check(0, 1, true);
        vm.warp(D + 1 days);
        bytes32 h = keccak256("report");
        bool ok = tryReview(0, true, 10, h);
        require(!ok, "reviewer overrode check");
        vm.warp(D + 3 days + 1);
        bounty.finalize();
        bounty.withdrawRefund();
        require(token.balanceOf(A) == 0 && bounty.totalRefunded() == 17000000, "failed work paid");
    }

    function testUnreviewedEligibleEntryPreventsFavoringAnother() public {
        entry(A);
        entry(B);
        vm.warp(D + 1);
        for (uint256 i; i < 2; i++) {
            check(i, 0, true);
            check(i, 1, true);
        }
        vm.warp(D + 1 days);
        review(0, true, 100);
        vm.warp(D + 3 days + 1);
        (bool ok,) = address(bounty).call(abi.encodeCall(bounty.finalize, ()));
        require(!ok, "ignored pending review");
        vm.warp(D + 4 days + 1);
        bounty.finalize();
        require(uint8(bounty.state()) == 3 && token.balanceOf(A) == 0, "unfair winner");
        bounty.withdrawRefund();
    }

    function testLateReviewCannotRewriteRefundOutcome() public {
        entry(A);
        vm.warp(D + 1);
        check(0, 0, true);
        check(0, 1, true);
        vm.warp(D + 4 days + 1);
        bounty.finalize();
        bytes32 h = keccak256("late report");
        bool ok = tryReview(0, true, 5, h);
        require(!ok, "late outcome rewritten");
    }

    function testSubmissionCutoffBoundaryAndDuplicateWallet() public {
        vm.warp(D);
        entry(A);
        vm.prank(A);
        (bool duplicate,) = address(bounty).call(abi.encodeCall(bounty.commitSubmission, (keccak256("second"))));
        require(!duplicate, "duplicate wallet");
        vm.warp(D + 1);
        vm.prank(B);
        (bool late,) = address(bounty).call(abi.encodeCall(bounty.commitSubmission, (keccak256("late"))));
        require(!late, "late commitment");
    }

    function testWrongSignerAndChangedEvidenceFail() public {
        entry(A);
        vm.warp(D + 1);
        bytes32 h = keccak256("result");
        bytes memory sig = signature(REVIEWER, bounty.checkDigest(0, 0, true, h));
        (bool ok,) = address(bounty).call(abi.encodeCall(bounty.recordCheck, (0, 0, true, h, sig)));
        require(!ok, "forged report");
        sig = signature(RUNNER, bounty.checkDigest(0, 0, true, h));
        (ok,) = address(bounty).call(abi.encodeCall(bounty.recordCheck, (0, 0, true, keccak256("changed"), sig)));
        require(!ok, "changed evidence");
    }

    function testOmittedStagesUseNoArtificialWaiting() public {
        OpenCompetitionBountyV3.Config memory c = config(1, 0);
        OpenCompetitionBountyV3 automatic = new OpenCompetitionBountyV3(c);
        require(
            automatic.reviewCutoff() == automatic.checksCutoff() && automatic.finalCutoff() == automatic.reviewCutoff(),
            "unused stage"
        );
    }

    function testFuzzFundsNeverPayAndRefund(uint64 seed) public {
        entry(A);
        vm.warp(D + 1);
        bool pass = seed % 2 == 0;
        check(0, 0, pass);
        check(0, 1, pass);
        if (pass) {
            vm.warp(D + 1 days);
            review(0, true, int256(uint256(seed)));
        }
        vm.warp(D + 4 days + 1);
        bounty.finalize();
        if (!pass) bounty.withdrawRefund();
        require(bounty.paidAmount() + bounty.totalRefunded() == bounty.fundedAmount(), "fund conservation");
        require(token.balanceOf(address(bounty)) == 0, "funds stuck");
    }

    function testInitialFundsAreAccountedFor() public view {
        require(bounty.paidAmount() + bounty.totalRefunded() <= bounty.fundedAmount(), "fund conservation");
        if (bounty.paidAmount() > 0) require(bounty.unresolvedEntries() == 0, "unresolved winner");
    }
}

/// The invariant runner drives valid signed operations, instead of spending
/// its campaign on random signatures that only revert.
contract MixedVerificationHandler {
    VmMixed constant vm = VmMixed(address(uint160(uint256(keccak256("hevm cheat code")))));
    MixedToken public immutable token;
    OpenCompetitionBountyV3 public immutable bounty;

    constructor(MixedToken token_, OpenCompetitionBountyV3 bounty_) {
        token = token_;
        bounty = bounty_;
        token_.mint(address(this), bounty_.targetAmount());
        token_.approve(address(bounty_), bounty_.targetAmount());
        bounty_.fund(bounty_.targetAmount());
        for (uint256 i; i < 4; i++) {
            vm.prank(address(uint160(1000 + i)));
            bounty_.commitSubmission(keccak256(abi.encode("stored version", i)));
        }
    }

    function signature(uint256 key, bytes32 hash) private returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, hash);
        return abi.encodePacked(r, s, v);
    }

    function item(uint256 id) private view returns (OpenCompetitionBountyV3.Entry memory) {
        (bool ok, bytes memory data) = address(bounty).staticcall(abi.encodeWithSignature("entries(uint256)", id));
        require(ok, "entry read failed");
        return abi.decode(data, (OpenCompetitionBountyV3.Entry));
    }

    function advance(uint8 stage) public {
        vm.warp(block.timestamp + (uint256(stage) % 6 + 1) * 1 hours);
    }

    function record(uint8 rawId, uint8 rawCheck, bool passed) public {
        if (bounty.state() != OpenCompetitionBountyV3.State.Active) return;
        if (block.timestamp <= bounty.submissionCutoff()) vm.warp(bounty.submissionCutoff() + 1);
        if (block.timestamp > bounty.checksCutoff()) return;
        uint256 id = rawId % 4;
        uint8 index = rawCheck % 2;
        if (item(id).checkedMask & (uint64(1) << index) != 0) return;
        bytes32 evidence = keccak256(abi.encode("runner evidence", id, index, passed));
        bounty.recordCheck(id, index, passed, evidence, signature(21, bounty.checkDigest(id, index, passed, evidence)));
    }

    function review(uint8 rawId, uint8 rawReviewer, bool passed, uint8 rawScore) public {
        if (bounty.state() != OpenCompetitionBountyV3.State.Active) return;
        if (block.timestamp < bounty.checksCutoff()) return;
        if (block.timestamp > bounty.finalCutoff()) return;
        uint256 id = rawId % 4;
        uint8 reviewer = rawReviewer % 3;
        OpenCompetitionBountyV3.Entry memory e = item(id);
        if (e.checkedMask != 3 || e.passedMask != 3 || e.reviewerMask & (uint16(1) << reviewer) != 0) return;
        int256 score = int256(uint256(rawScore % 11)) - 5;
        bytes32 report = keccak256(abi.encode("stored criterion report", id, reviewer, passed, score));
        bytes32 hash = bounty.reviewDigest(id, 3, passed, score, report);
        bytes memory signed = signature(22 + reviewer, hash);
        bounty.recordReview(id, reviewer, 3, passed, score, report, signed);
    }

    function finalize() public {
        if (bounty.state() != OpenCompetitionBountyV3.State.Active || block.timestamp <= bounty.reviewCutoff()) return;
        if (bounty.unresolvedEntries() > 0 && block.timestamp <= bounty.finalCutoff()) return;
        bounty.finalize();
    }

    function refund() public {
        if (bounty.state() == OpenCompetitionBountyV3.State.Cancelled && bounty.contributions(address(this)) > 0) {
            bounty.withdrawRefund();
        }
    }

    function finish() external {
        if (block.timestamp <= bounty.finalCutoff()) vm.warp(bounty.finalCutoff() + 1);
        finalize();
        refund();
    }
}

contract OpenCompetitionBountyV3InvariantTest {
    struct FuzzSelector {
        address addr;
        bytes4[] selectors;
    }
    VmMixed constant vm = VmMixed(address(uint160(uint256(keccak256("hevm cheat code")))));
    MixedToken token;
    OpenCompetitionBountyV3 bounty;
    MixedVerificationHandler handler;
    address[] targets;

    function setUp() public {
        vm.warp(100);
        token = new MixedToken();
        OpenCompetitionBountyV3.Config memory c;
        c.creator = address(this);
        c.token = address(token);
        c.bountyId = keccak256("invariant bounty");
        c.termsHash = keccak256("terms");
        c.planHash = keccak256("immutable plan");
        c.reward = 17000000;
        c.fundingCutoff = 100000;
        c.submissionCutoff = 200000;
        c.checksCutoff = c.submissionCutoff + 1 days;
        c.reviewCutoff = c.checksCutoff + 2 days;
        c.finalCutoff = c.reviewCutoff + 1 days;
        c.maximumEntries = 8;
        c.checks = new OpenCompetitionBountyV3.Checker[](2);
        for (uint256 i; i < 2; i++) {
            c.checks[i] = OpenCompetitionBountyV3.Checker(vm.addr(21), OpenCompetitionBountyV3.Assurance.TrustedRunner);
        }
        c.reviewers = new address[](3);
        for (uint256 i; i < 3; i++) {
            c.reviewers[i] = vm.addr(22 + i);
        }
        c.reviewThreshold = 2;
        c.requiredReviewMask = 3;
        bounty = new OpenCompetitionBountyV3(c);
        handler = new MixedVerificationHandler(token, bounty);
        targets.push(address(handler));
    }

    function targetContracts() external view returns (address[] memory) {
        return targets;
    }

    function targetSelectors() external view returns (FuzzSelector[] memory selected) {
        bytes4[] memory selectors = new bytes4[](5);
        selectors[0] = handler.advance.selector;
        selectors[1] = handler.record.selector;
        selectors[2] = handler.review.selector;
        selectors[3] = handler.finalize.selector;
        selectors[4] = handler.refund.selector;
        selected = new FuzzSelector[](1);
        selected[0] = FuzzSelector(address(handler), selectors);
    }

    function invariantFundsAreConservedAndOnlyQualifiedWorkIsPaid() public view {
        require(
            token.balanceOf(address(bounty)) + bounty.paidAmount() + bounty.totalRefunded() == bounty.fundedAmount(),
            "fund conservation"
        );
        if (bounty.paidAmount() > 0) {
            require(bounty.totalRefunded() == 0, "paid and refunded");
            require(bounty.unresolvedEntries() == 0, "ignored timely entry");
            require(bounty.qualified(bounty.winningEntry()), "unqualified winner");
            require(bounty.state() == OpenCompetitionBountyV3.State.Settled, "payment state");
        }
    }

    function afterInvariant() public {
        handler.finish();
        require(token.balanceOf(address(bounty)) == 0, "funds stuck after final cutoff");
        invariantFundsAreConservedAndOnlyQualifiedWorkIsPaid();
    }
}
