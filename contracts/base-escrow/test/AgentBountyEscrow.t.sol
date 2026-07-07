// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyEscrow.sol";

contract TestToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
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

    function transfer(address to, uint256 amount) external returns (bool) {
        require(balanceOf[msg.sender] >= amount, "balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }
}

contract AgentBountyEscrowTest {
    AgentBountyEscrow escrow;
    TestToken token;
    address solver = address(0xBEEF);
    address verifier = address(0xCAFE);
    address platform = address(0xFEE);

    function setUp() public {
        escrow = new AgentBountyEscrow(address(this));
        token = new TestToken();
        token.mint(address(this), 1000);
        token.approve(address(escrow), 1000);
    }

    function testReleaseSplit() public {
        uint256 escrowId = escrow.createEscrow(bytes32("bounty"), address(token), 1000, bytes32("terms"));
        address[] memory recipients = new address[](3);
        recipients[0] = solver;
        recipients[1] = verifier;
        recipients[2] = platform;
        uint256[] memory amounts = new uint256[](3);
        amounts[0] = 900;
        amounts[1] = 50;
        amounts[2] = 50;

        escrow.release(escrowId, recipients, amounts, bytes32("proof"));

        require(token.balanceOf(solver) == 900, "solver payout");
        require(token.balanceOf(verifier) == 50, "verifier payout");
        require(token.balanceOf(platform) == 50, "platform fee");
    }

    function testRefund() public {
        uint256 escrowId = escrow.createEscrow(bytes32("bounty"), address(token), 1000, bytes32("terms"));
        escrow.refund(escrowId, bytes32("reason"));
        require(token.balanceOf(address(this)) == 1000, "refund");
    }

    function testDisputeThenRelease() public {
        uint256 escrowId = escrow.createEscrow(bytes32("bounty"), address(token), 1000, bytes32("terms"));
        escrow.markDisputed(escrowId, bytes32("dispute"));

        address[] memory recipients = new address[](2);
        recipients[0] = solver;
        recipients[1] = platform;
        uint256[] memory amounts = new uint256[](2);
        amounts[0] = 950;
        amounts[1] = 50;

        escrow.release(escrowId, recipients, amounts, bytes32("proof"));
        require(token.balanceOf(solver) == 950, "solver payout after dispute");
    }

    function testPauseBlocksCreation() public {
        escrow.pause(true);

        try escrow.createEscrow(bytes32("bounty"), address(token), 1000, bytes32("terms")) {
            revert("expected pause revert");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256(bytes("paused")), "wrong reason");
        }
    }

    function testBadSplitReverts() public {
        uint256 escrowId = escrow.createEscrow(bytes32("bounty"), address(token), 1000, bytes32("terms"));
        address[] memory recipients = new address[](2);
        recipients[0] = solver;
        recipients[1] = platform;
        uint256[] memory amounts = new uint256[](2);
        amounts[0] = 900;
        amounts[1] = 50;

        try escrow.release(escrowId, recipients, amounts, bytes32("proof")) {
            revert("expected split mismatch");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256(bytes("split mismatch")), "wrong reason");
        }
    }

    function testDoubleReleaseReverts() public {
        uint256 escrowId = escrow.createEscrow(bytes32("bounty"), address(token), 1000, bytes32("terms"));
        address[] memory recipients = new address[](1);
        recipients[0] = solver;
        uint256[] memory amounts = new uint256[](1);
        amounts[0] = 1000;

        escrow.release(escrowId, recipients, amounts, bytes32("proof"));

        try escrow.release(escrowId, recipients, amounts, bytes32("proof2")) {
            revert("expected not releasable");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256(bytes("not releasable")), "wrong reason");
        }
    }

    function testOnlySettlementSignerCanRelease() public {
        uint256 escrowId = escrow.createEscrow(bytes32("bounty"), address(token), 1000, bytes32("terms"));
        escrow.setSettlementSigner(address(0x1234));

        address[] memory recipients = new address[](1);
        recipients[0] = solver;
        uint256[] memory amounts = new uint256[](1);
        amounts[0] = 1000;

        try escrow.release(escrowId, recipients, amounts, bytes32("proof")) {
            revert("expected signer revert");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256(bytes("not settlement signer")), "wrong reason");
        }
    }
}
