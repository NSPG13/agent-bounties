// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/BoundedAgentWalletV2Factory.sol";

interface VmBoundedV2Activation {
    function etch(address target, bytes calldata code) external;
}

/// @notice Pins the deterministic Base-mainnet V2 factory and implementation.
contract BoundedAgentWalletV2ActivationTest {
    VmBoundedV2Activation private constant vm =
        VmBoundedV2Activation(address(uint160(uint256(keccak256("hevm cheat code")))));

    address private constant CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;
    address private constant BOUNTY_FACTORY = 0x082C52131aaF0C56e76b075f895EAB6fcaB6d2F9;
    address private constant USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
    address private constant VERIFIER = 0x380C1af742593DD88B6f20387e9eE693A0536731;
    address private constant EXPECTED_FACTORY = 0xE3D4f7b203c5E8576E0225D3e64a8532429D3876;
    address private constant EXPECTED_IMPLEMENTATION = 0x00c250bdA8Fa3c49d80A11d9B6ebd961736b7202;
    bytes32 private constant EXPECTED_FACTORY_RUNTIME_HASH =
        0x254e247c3df7b38c257cd24b5d47c6ca1bc3ed335d6cb062498695bced540cf9;
    bytes32 private constant EXPECTED_IMPLEMENTATION_RUNTIME_HASH =
        0xade4fb51d0c5c866bb0cc44ca17ad0b254c6bba92ac5b47ddc2ced4963b05d18;
    bytes32 private constant EXPECTED_CLONE_RUNTIME_HASH =
        0xc11ada07afafbcf407387ff2fa7afc52e55301c80a4edd0888520b478fba9209;

    function testPinnedMainnetV2FactoryAndCloneBytecode() public {
        vm.etch(USDC, hex"00");
        AgentBountyFactory seedFactory = new AgentBountyFactory(USDC);
        vm.etch(BOUNTY_FACTORY, address(seedFactory).code);
        vm.etch(VERIFIER, hex"00");
        vm.etch(
            CREATE2_DEPLOYER,
            hex"7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3"
        );

        bytes32 salt = keccak256("agent-bounties/base-mainnet/bounded-agent-wallet-factory/v2");
        bytes memory initCode =
            abi.encodePacked(type(BoundedAgentWalletV2Factory).creationCode, abi.encode(BOUNTY_FACTORY));
        (bool deployed, bytes memory result) = CREATE2_DEPLOYER.call(abi.encodePacked(salt, initCode));
        require(deployed && result.length == 20, "deterministic deployment failed");
        require(address(bytes20(result)) == EXPECTED_FACTORY, "factory address drift");
        require(EXPECTED_FACTORY.codehash == EXPECTED_FACTORY_RUNTIME_HASH, "factory runtime drift");

        BoundedAgentWalletV2Factory factory = BoundedAgentWalletV2Factory(EXPECTED_FACTORY);
        require(factory.bountyFactory() == AgentBountyFactory(BOUNTY_FACTORY), "bounty factory drift");
        require(factory.settlementToken() == USDC, "token drift");
        require(factory.implementation() == EXPECTED_IMPLEMENTATION, "implementation address drift");
        require(EXPECTED_IMPLEMENTATION.codehash == EXPECTED_IMPLEMENTATION_RUNTIME_HASH, "implementation drift");

        BoundedAgentWallet.Policy memory policy = BoundedAgentWallet.Policy({
            delegate: address(0xD311),
            validAfter: uint64(block.timestamp),
            validUntil: uint64(block.timestamp + 30 days),
            periodSeconds: 1 days,
            maxPerAction: 2_010_000,
            maxPerPeriod: 10_050_000,
            maxLifetimeSpend: 10_050_000,
            maxBountyTarget: 2_010_000,
            allowedActions: 15,
            allowedVerificationModes: 1,
            deterministicVerifierModule: VERIFIER,
            signedQuorumVerifierSetHash: bytes32(0),
            aiJudgeVerifierSetHash: bytes32(0)
        });
        bytes32 userSalt = keccak256("v2-activation-test-wallet");
        address predicted = factory.predictWallet(address(this), policy, userSalt);
        address walletAddress = factory.createWallet(address(this), policy, userSalt);
        require(walletAddress == predicted, "wallet prediction drift");
        require(walletAddress.codehash == EXPECTED_CLONE_RUNTIME_HASH, "clone runtime drift");
        require(
            BoundedAgentWalletV2(payable(walletAddress)).WALLET_VERSION()
                == keccak256("agent-bounties/bounded-wallet/v2"),
            "wallet version drift"
        );
    }
}
