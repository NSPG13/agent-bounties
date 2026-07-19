pragma solidity ^0.8.0;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/utils/Strings.sol";

contract BountyCreator {
    IERC20 public usdc;
    address public onchainTermsRegistry = 0x35e5d49c12b75c119d33951c2c4f054c5732208c;

    constructor(address _usdc) {
        usdc = IERC20(_usdc);
    }

    function publishTerms(
        string memory title,
        string memory description,
        uint256 reward,
        address[] memory verifiers,
        uint256 quorumThreshold,
        string memory verificationModule
    ) external {
        // Encode the terms data
        bytes memory termsData = abi.encode(
            title,
            description,
            reward,
            verifiers,
            quorumThreshold,
            verificationModule
        );

        // Publish the terms on-chain
        (bool success, ) = onchainTermsRegistry.call(abi.encodeWithSignature("publishTerms(bytes)", termsData));
        require(success, "Failed to publish terms");
    }

    function fundBounty(uint256 amount) external {
        require(usdc.transferFrom(msg.sender, address(this), amount), "Transfer failed");
    }
}
