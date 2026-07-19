// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";

contract ApiBounty is Ownable {
    IERC20 public usdc;
    address public onchainTermsRegistry;
    uint256 public constant USDC_DECIMALS = 6; // USDC has 6 decimals

    struct Bounty {
        address solver;
        uint256 amount;
        bool claimed;
    }

    mapping(address => bool) public registeredWallets;
    mapping(uint256 => Bounty) public bounties;
    uint256 public nextBountyId;

    event WalletRegistered(address indexed wallet);
    event BountyCreated(uint256 indexed bountyId, address indexed creator, uint256 amount);
    event BountyClaimed(uint256 indexed bountyId, address indexed solver);

    constructor(address _usdc, address _onchainTermsRegistry) {
        usdc = IERC20(_usdc);
        onchainTermsRegistry = _onchainTermsRegistry;
    }

    modifier onlyRegistered() {
        require(registeredWallets[msg.sender], "Wallet not registered");
        _;
    }

    function registerWallet() external {
        require(!registeredWallets[msg.sender], "Wallet already registered");
        registeredWallets[msg.sender] = true;
        emit WalletRegistered(msg.sender);
    }

    function createBounty(uint256 _amount) external onlyRegistered {
        require(usdc.transferFrom(msg.sender, address(this), _amount * (10 ** USDC_DECIMALS)), "Transfer failed");
        bounties[nextBountyId] = Bounty({
            solver: address(0),
            amount: _amount,
            claimed: false
        });
        emit BountyCreated(nextBountyId, msg.sender, _amount);
        nextBountyId++;
    }

    function claimBounty(uint256 _bountyId) external onlyRegistered {
        Bounty storage bounty = bounties[_bountyId];
        require(bounty.solver == address(0), "Bounty already claimed");
        require(!bounty.claimed, "Bounty already claimed");

        // Verifier quorum logic
        require(checkVerifierQuorum(), "Verifier quorum not met");

        // Transfer USDC to the solver
        require(usdc.transfer(msg.sender, bounty.amount * (10 ** USDC_DECIMALS)), "Transfer failed");

        // Update the bounty
        bounty.solver = msg.sender;
        bounty.claimed = true;

        emit BountyClaimed(_bountyId, msg.sender);
    }

    function checkVerifierQuorum() internal view returns (bool) {
        // Implement the verifier quorum logic
        // For simplicity, we assume the quorum is met if at least 2 out of 3 verifiers agree
        // In a real implementation, this would involve more complex logic and possibly off-chain verification
        return true; // Placeholder for actual quorum check
    }

    function publishTermsOnChain(string memory _terms) external onlyOwner {
        // Publish terms to OnchainTermsRegistry
        // This is a placeholder function. The actual implementation would interact with the OnchainTermsRegistry contract
        // For example:
        // (bool success, bytes memory data) = onchainTermsRegistry.call(abi.encodeWithSignature("publishTerms(string)", _terms));
        // require(success, "Publish terms failed");
    }
}