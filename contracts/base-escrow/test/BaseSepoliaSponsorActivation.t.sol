// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactory.sol";
import "../src/AtomicClaimSponsor.sol";
import "../src/LeadingZeroWorkVerifier.sol";

interface VmBaseSepoliaActivation {
    function etch(address target, bytes calldata code) external;
    function setNonce(address account, uint64 nonce) external;
    function startPrank(address sender) external;
    function stopPrank() external;
}

/// @notice Locks nonce-derived addresses and runtime bytecode for the unsigned
/// Base Sepolia activation bundle. No transaction is broadcast by this test.
contract BaseSepoliaSponsorActivationTest {
    VmBaseSepoliaActivation private constant vm =
        VmBaseSepoliaActivation(address(uint160(uint256(keccak256("hevm cheat code")))));

    address private constant DEPLOYER = 0x884834E884d6e93462655A2820140aD03E6747bC;
    address private constant USDC = 0x036CbD53842c5426634e7929541eC2318f3dCF7e;
    address private constant GRANT_SIGNER = 0x52bbc33FaCB5bD3d31125C168047543f423eE034;
    address private constant FACTORY = 0x9601a40b35Ad6843846732C6CB73c4C82f9Ba850;
    address private constant IMPLEMENTATION = 0xE70b9d541a176307e50f308Aa370A1661eabFd99;
    address private constant VERIFIER = 0x7231f1312448Fa60078Fb56cDB6e2c392Bd1269b;
    address private constant SPONSOR = 0xa1E2E93530114F7FE64c251556b8De13Dad7d157;

    bytes32 private constant FACTORY_RUNTIME_HASH = 0x7e07f933a77423a9183f6bbf3eb897c4e7b73399c95056b6142bdeb6be95d171;
    bytes32 private constant IMPLEMENTATION_RUNTIME_HASH =
        0xc36fcba5176b2cd8b57a9fd0cbf931177dc8b36cf8367c1568ccebe5f03be3f6;
    bytes32 private constant VERIFIER_RUNTIME_HASH = 0xbaa3a8305c4b65d0dc20131d0ef207fdaf4763f345393a831370cd04077df9b3;
    bytes32 private constant SPONSOR_RUNTIME_HASH = 0x09c5ecb7be48d2235ead4d4c4a9d11a83722f5b52dbdd58096ba09e185259a1b;

    function testPinnedAddressesAndRuntimeBytecode() public {
        vm.etch(USDC, hex"00");
        vm.setNonce(DEPLOYER, 1);
        vm.startPrank(DEPLOYER);

        AgentBountyFactory factory = new AgentBountyFactory(USDC);
        LeadingZeroWorkVerifier verifier = new LeadingZeroWorkVerifier(16);
        AtomicClaimSponsor sponsor =
            new AtomicClaimSponsor(USDC, address(factory), GRANT_SIGNER, 100_000, 1_000_000, 100_000);

        vm.stopPrank();

        require(address(factory) == FACTORY, "factory address drift");
        require(factory.implementation() == IMPLEMENTATION, "implementation address drift");
        require(address(verifier) == VERIFIER, "verifier address drift");
        require(address(sponsor) == SPONSOR, "sponsor address drift");
        require(FACTORY.codehash == FACTORY_RUNTIME_HASH, "factory runtime drift");
        require(IMPLEMENTATION.codehash == IMPLEMENTATION_RUNTIME_HASH, "implementation runtime drift");
        require(VERIFIER.codehash == VERIFIER_RUNTIME_HASH, "verifier runtime drift");
        require(SPONSOR.codehash == SPONSOR_RUNTIME_HASH, "sponsor runtime drift");
        require(sponsor.owner() == DEPLOYER, "sponsor owner drift");
    }
}
