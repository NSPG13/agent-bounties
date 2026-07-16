// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactory.sol";
import "../src/CanonicalIndependentChildVerifierV2.sol";

interface StandingMetaDeployVm {
    function addr(uint256 privateKey) external returns (address);
    function envAddress(string calldata name) external returns (address);
    function envString(string calldata name) external returns (string memory);
    function envUint(string calldata name) external returns (uint256);
    function startBroadcast(uint256 privateKey) external;
    function stopBroadcast() external;
    function serializeAddress(string calldata objectKey, string calldata valueKey, address value)
        external
        returns (string memory);
    function serializeBytes32(string calldata objectKey, string calldata valueKey, bytes32 value)
        external
        returns (string memory);
    function serializeUint(string calldata objectKey, string calldata valueKey, uint256 value)
        external
        returns (string memory);
    function writeJson(string calldata json, string calldata path) external;
}

/// @notice Deploys only the standing-meta-v2 policy components on Base mainnet.
/// The canonical factory and native USDC addresses are deliberately immutable here.
contract DeployStandingMetaV2 {
    StandingMetaDeployVm private constant vm =
        StandingMetaDeployVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    uint256 private constant BASE_MAINNET_CHAIN_ID = 8_453;
    address private constant BASE_MAINNET_FACTORY = 0x082C52131aaF0C56e76b075f895EAB6fcaB6d2F9;
    address private constant BASE_MAINNET_USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
    uint256 private constant MIN_KEEPER_BALANCE_AFTER_DEPLOYMENT = 100_000_000_000_000;

    function run() external {
        require(block.chainid == BASE_MAINNET_CHAIN_ID, "Base mainnet only");
        require(
            AgentBountyFactory(BASE_MAINNET_FACTORY).settlementToken() == BASE_MAINNET_USDC,
            "canonical factory token drift"
        );

        uint256 deployerKey = vm.envUint("BASE_KEEPER_PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        require(deployer.balance >= 500_000_000_000_000, "keeper reserve too low to deploy safely");

        address attester = vm.envAddress("PARTICIPANT_ATTESTER_ADDRESS");
        address verifierOne = vm.envAddress("REGRESSION_VERIFIER_ONE_ADDRESS");
        address verifierTwo = vm.envAddress("REGRESSION_VERIFIER_TWO_ADDRESS");
        require(attester != address(0), "attester zero");
        require(
            verifierOne != address(0) && verifierTwo != address(0) && verifierOne != verifierTwo, "verifier set invalid"
        );
        address[] memory verifiers = new address[](2);
        verifiers[0] = verifierOne;
        verifiers[1] = verifierTwo;
        bytes32 verifierSetHash = keccak256(abi.encode(verifiers));

        vm.startBroadcast(deployerKey);
        ParticipantEligibilityRegistry participants = new ParticipantEligibilityRegistry(attester);
        OnchainTermsRegistry terms = new OnchainTermsRegistry();
        CanonicalIndependentChildVerifierV2 module = new CanonicalIndependentChildVerifierV2(
            BASE_MAINNET_FACTORY, address(participants), address(terms), verifierSetHash, 2
        );
        vm.stopBroadcast();

        require(address(module.canonicalFactory()) == BASE_MAINNET_FACTORY, "module factory drift");
        require(module.settlementToken() == BASE_MAINNET_USDC, "module token drift");
        require(address(module.participantRegistry()) == address(participants), "participant registry drift");
        require(address(module.termsRegistry()) == address(terms), "terms registry drift");
        require(module.taskVerifierSetHash() == verifierSetHash, "verifier set drift");
        require(module.taskVerifierThreshold() == 2, "verifier threshold drift");
        require(deployer.balance >= MIN_KEEPER_BALANCE_AFTER_DEPLOYMENT, "keeper reserve depleted below minimum");

        string memory objectKey = "standing-meta-v2-deployment";
        vm.serializeUint(objectKey, "chain_id", block.chainid);
        vm.serializeAddress(objectKey, "deployer", deployer);
        vm.serializeAddress(objectKey, "canonical_factory", BASE_MAINNET_FACTORY);
        vm.serializeAddress(objectKey, "settlement_token", BASE_MAINNET_USDC);
        vm.serializeAddress(objectKey, "participant_attester", attester);
        vm.serializeAddress(objectKey, "participant_registry", address(participants));
        vm.serializeAddress(objectKey, "terms_registry", address(terms));
        vm.serializeAddress(objectKey, "verifier_one", verifierOne);
        vm.serializeAddress(objectKey, "verifier_two", verifierTwo);
        vm.serializeBytes32(objectKey, "verifier_set_hash", verifierSetHash);
        vm.serializeBytes32(objectKey, "acceptance_criteria_hash", module.ACCEPTANCE_CRITERIA_HASH());
        vm.serializeUint(objectKey, "keeper_balance_after_wei", deployer.balance);
        string memory json = vm.serializeAddress(objectKey, "verifier_module", address(module));
        vm.writeJson(json, vm.envString("DEPLOYMENT_EVIDENCE_PATH"));
    }
}
