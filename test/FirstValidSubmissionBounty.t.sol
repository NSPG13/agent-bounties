// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

import "forge-std/Test.sol";
import "../contracts/FirstValidSubmissionBounty.sol";

contract FirstValidSubmissionBountyTest is Test {
    FirstValidSubmissionBounty public bounty;
    address public creator = address(0x1);
    address public verifier = address(0x2);
    address public submitter1 = address(0x3);
    address public submitter2 = address(0x4);

    function setUp() public {
        bounty = new FirstValidSubmissionBounty();
        vm.deal(creator, 100 ether);
        vm.deal(submitter1, 10 ether);
        vm.deal(submitter2, 10 ether);
    }

    function testCreateBounty() public {
        vm.prank(creator);
        uint256 bountyId = bounty.createBounty{value: 1 ether}(
            verifier,
            0.1 ether,
            10,
            1 days,
            7 days
        );
        assertEq(bountyId, 0);
    }

    function testCommitRevealSettle() public {
        vm.prank(creator);
        uint256 bountyId = bounty.createBounty{value: 1 ether}(
            verifier,
            0.1 ether,
            10,
            1 days,
            7 days
        );

        bytes memory solution = "solution data";
        bytes32 salt = keccak256("salt1");
        bytes32 commitHash = keccak256(abi.encodePacked(solution, salt));

        vm.prank(submitter1);
        bounty.submitCommitment{value: 0.1 ether}(bountyId, commitHash);

        vm.warp(block.timestamp + 1 days + 1);

        vm.prank(submitter1);
        bounty.reveal(bountyId, 0, solution, salt);

        vm.prank(verifier);
        bounty.settleBounty(bountyId, 0);

        (,,,,,,,, bool settled, address winner) = bounty.bounties(bountyId);
        assertEq(settled, true);
        assertEq(winner, submitter1);
    }

    function testEarliestValidWins() public {
        vm.prank(creator);
        uint256 bountyId = bounty.createBounty{value: 1 ether}(
            verifier,
            0.1 ether,
            10,
            1 days,
            7 days
        );

        bytes32 salt1 = keccak256("salt1");
        bytes32 salt2 = keccak256("salt2");
        bytes32 commit1 = keccak256(abi.encodePacked("solution1", salt1));
        bytes32 commit2 = keccak256(abi.encodePacked("solution2", salt2));

        vm.prank(submitter1);
        bounty.submitCommitment{value: 0.1 ether}(bountyId, commit1);

        vm.warp(block.timestamp + 100);

        vm.prank(submitter2);
        bounty.submitCommitment{value: 0.1 ether}(bountyId, commit2);

        vm.warp(block.timestamp + 1 days);

        vm.prank(submitter1);
        bounty.reveal(bountyId, 0, "solution1", salt1);

        vm.prank(submitter2);
        bounty.reveal(bountyId, 1, "solution2", salt2);

        vm.prank(verifier);
        bounty.settleBounty(bountyId, 0);

        (,,,,,,,, bool settled, address winner) = bounty.bounties(bountyId);
        assertEq(winner, submitter1);
    }

    function testRefundAfterExpiry() public {
        vm.prank(creator);
        uint256 bountyId = bounty.createBounty{value: 1 ether}(
            verifier,
            0.1 ether,
            10,
            1 days,
            7 days
        );

        bytes32 commit = keccak256(abi.encodePacked("solution", keccak256("salt")));
        vm.prank(submitter1);
        bounty.submitCommitment{value: 0.1 ether}(bountyId, commit);

        vm.warp(block.timestamp + 1 days + 7 days + 1);

        uint256 balanceBefore = submitter1.balance;
        vm.prank(submitter1);
        bounty.refundBond(bountyId, 0);
        assertEq(submitter1.balance, balanceBefore + 0.1 ether);
    }

    function testCannotCommitAfterDeadline() public {
        vm.prank(creator);
        uint256 bountyId = bounty.createBounty{value: 1 ether}(
            verifier,
            0.1 ether,
            10,
            1 days,
            7 days
        );

        vm.warp(block.timestamp + 1 days + 1);

        vm.prank(submitter1);
        vm.expectRevert("Commit period ended");
        bounty.submitCommitment{value: 0.1 ether}(bountyId, keccak256("test"));
    }
}
