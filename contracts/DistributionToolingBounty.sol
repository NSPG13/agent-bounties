// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";

contract DistributionToolingBounty is Ownable {
    IERC20 public usdc;
    uint256 public totalFunding;
    uint256 public parentSolverReward;
    uint256 public automatedVerifierReward;
    uint256 public claimBond;
    bool public isClaimable;
    address public childSolver;
    address public verifier1;
    address public verifier2;

    event BountyFunded(uint256 amount);
    event ChildSolverRegistered(address indexed solver);
    event BountyClaimed(address indexed solver, uint256 amount);

    constructor(
        address _usdc,
        uint256 _totalFunding,
        uint256 _parentSolverReward,
        uint256 _automatedVerifierReward,
        uint256 _claimBond,
        address _verifier1,
        address _verifier2
    ) {
        usdc = IERC20(_usdc);
        totalFunding = _totalFunding;
        parentSolverReward = _parentSolverReward;
        automatedVerifierReward = _automatedVerifierReward;
        claimBond = _claimBond;
        verifier1 = _verifier1;
        verifier2 = _verifier2;
        isClaimable = false;
    }

    function fundBounty() external onlyOwner {
        require(usdc.transferFrom(msg.sender, address(this), totalFunding), "Transfer failed");
        emit BountyFunded(totalFunding);
    }

    function registerChildSolver(address _childSolver) external onlyOwner {
        require(childSolver == address(0), "Child solver already registered");
        childSolver = _childSolver;
        emit ChildSolverRegistered(_childSolver);
    }

    function claimBounty() external {
        require(isClaimable, "Bounty is not claimable");
        require(msg.sender == childSolver, "Only the registered child solver can claim the bounty");

        // Transfer the parent solver reward
        require(usdc.transfer(msg.sender, parentSolverReward), "Transfer failed");

        // Transfer the automated verifier reward
        require(usdc.transfer(verifier1, automatedVerifierReward), "Transfer failed");
        require(usdc.transfer(verifier2, automatedVerifierReward), "Transfer failed");

        // Refund the claim bond
        require(usdc.transfer(owner(), claimBond), "Refund failed");

        isClaimable = false;
        emit BountyClaimed(msg.sender, parentSolverReward);
    }

    function setClaimable(bool _isClaimable) external onlyOwner {
        isClaimable = _isClaimable;
    }

    function withdrawFunds() external onlyOwner {
        require(!isClaimable, "Bounty is still claimable");
        uint256 balance = usdc.balanceOf(address(this));
        require(usdc.transfer(owner(), balance), "Withdrawal failed");
    }
}