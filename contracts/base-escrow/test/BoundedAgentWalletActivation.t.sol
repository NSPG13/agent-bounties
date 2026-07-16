// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/BoundedAgentWalletFactory.sol";

interface VmBoundedActivation {
    function etch(address target, bytes calldata code) external;
}

/// @notice Locks the deterministic mainnet factory address and reviewed runtime hashes.
/// This test deploys only into the local Foundry VM.
contract BoundedAgentWalletActivationTest {
    VmBoundedActivation private constant vm =
        VmBoundedActivation(address(uint160(uint256(keccak256("hevm cheat code")))));

    address private constant CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;
    address private constant BOUNTY_FACTORY = 0x082C52131aaF0C56e76b075f895EAB6fcaB6d2F9;
    address private constant USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
    address private constant VERIFIER = 0xcc6059cEedA5bc4ba8a97ecFbFFa7488C8FD579E;
    address private constant EXPECTED_FACTORY = 0x3840936351049AED639780a16845e6094C1f17F6;
    address private constant EXPECTED_IMPLEMENTATION = 0x40D3e16082CF71ecE0129ca3044E1b8233e29dB8;
    bytes32 private constant EXPECTED_FACTORY_RUNTIME_HASH =
        0x243e248a890daf57cb14cee262bc7bb70b8822c65a014a8bf1c39653bc30aa52;
    bytes32 private constant EXPECTED_IMPLEMENTATION_RUNTIME_HASH =
        0x7fb59d5add3ac348ac3d7e6a5aa6b22ad542a6e6093a1ceb8d535f747ed536df;
    bytes32 private constant EXPECTED_CLONE_RUNTIME_HASH =
        0xc663bed9b4097e22e5a18c0ecb662561bf45df1829e6412cdd0d8568d05ca1b6;

    function testPinnedMainnetFactoryAndCloneBytecode() public {
        vm.etch(USDC, hex"00");
        AgentBountyFactory seedFactory = new AgentBountyFactory(USDC);
        vm.etch(BOUNTY_FACTORY, address(seedFactory).code);
        vm.etch(VERIFIER, hex"00");
        vm.etch(
            CREATE2_DEPLOYER,
            hex"7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3"
        );

        bytes32 salt = keccak256("agent-bounties/base-mainnet/bounded-agent-wallet-factory/v1");
        bytes memory initCode =
            abi.encodePacked(type(BoundedAgentWalletFactory).creationCode, abi.encode(BOUNTY_FACTORY));
        (bool deployed, bytes memory result) = CREATE2_DEPLOYER.call(abi.encodePacked(salt, initCode));
        require(deployed && result.length == 20, "deterministic deployment failed");
        address factoryAddress = address(bytes20(result));
        require(factoryAddress == EXPECTED_FACTORY, "factory address drift");
        require(EXPECTED_FACTORY.codehash == EXPECTED_FACTORY_RUNTIME_HASH, "factory runtime drift");

        BoundedAgentWalletFactory walletFactory = BoundedAgentWalletFactory(EXPECTED_FACTORY);
        require(walletFactory.bountyFactory() == AgentBountyFactory(BOUNTY_FACTORY), "bounty factory drift");
        require(walletFactory.settlementToken() == USDC, "token drift");
        require(walletFactory.implementation() == EXPECTED_IMPLEMENTATION, "implementation address drift");
        require(EXPECTED_IMPLEMENTATION.codehash == EXPECTED_IMPLEMENTATION_RUNTIME_HASH, "implementation drift");

        BoundedAgentWallet.Policy memory policy = BoundedAgentWallet.Policy({
            delegate: address(0xD311),
            validAfter: uint64(block.timestamp),
            validUntil: uint64(block.timestamp + 30 days),
            periodSeconds: 1 days,
            maxPerAction: 5_000_000,
            maxPerPeriod: 10_000_000,
            maxLifetimeSpend: 89_000_000,
            maxBountyTarget: 5_000_000,
            allowedActions: 15,
            allowedVerificationModes: 1,
            deterministicVerifierModule: VERIFIER,
            signedQuorumVerifierSetHash: bytes32(0),
            aiJudgeVerifierSetHash: bytes32(0)
        });
        bytes32 userSalt = keccak256("activation-test-wallet");
        address predicted = walletFactory.predictWallet(address(this), policy, userSalt);
        address wallet = walletFactory.createWallet(address(this), policy, userSalt);
        require(wallet == predicted, "wallet prediction drift");
        require(wallet.codehash == EXPECTED_CLONE_RUNTIME_HASH, "clone runtime drift");
    }
}
