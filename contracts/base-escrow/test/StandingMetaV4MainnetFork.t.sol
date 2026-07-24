// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactory.sol";
import "../src/AnonymousProtocolControllerV1.sol";
import "../src/AnonymousStakePoolV1.sol";
import "../src/AppealableVerifierV1.sol";
import "../src/StandingMetaChildFactoryV4.sol";
import "../src/StandingMetaParentFactoryV4.sol";
import "../src/StandingMetaV4Bundle.sol";
import "../src/VrfSortitionCoordinatorV1.sol";

interface StandingMetaV4ForkVm {
    function deal(address account, uint256 newBalance) external;
    function skip(bool skipTest) external;
}

interface IVrfSubscriptionCoordinatorV4Fork {
    function createSubscription() external returns (uint256 subId);
    function addConsumer(uint256 subId, address consumer) external;
    function fundSubscriptionWithNative(uint256 subId) external payable;
    function getSubscription(uint256 subId)
        external
        view
        returns (uint96 balance, uint96 nativeBalance, uint64 reqCount, address subOwner, address[] memory consumers);
}

contract StandingMetaV4MainnetForkTest {
    StandingMetaV4ForkVm private constant vm =
        StandingMetaV4ForkVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    uint256 private constant BASE_MAINNET_CHAIN_ID = 8_453;
    address private constant BASE_MAINNET_FACTORY = 0x082C52131aaF0C56e76b075f895EAB6fcaB6d2F9;
    address private constant BASE_MAINNET_USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
    address private constant BASE_MAINNET_VRF = 0xd5D517aBE5cF79B7e95eC98dB0f0277788aFF634;
    bytes32 private constant BASE_MAINNET_KEY_HASH =
        0x00b81b5a830cb0a4009fbd8904de511e28631e62ce5ad231373d3cdad373ccab;

    function testOfficialBaseVrfSupportsExactV4GraphAndTwoConsumers() public {
        vm.skip(block.chainid != BASE_MAINNET_CHAIN_ID);
        vm.deal(address(this), 1 ether);
        require(BASE_MAINNET_FACTORY.code.length > 0 && BASE_MAINNET_VRF.code.length > 0, "Base dependency missing");
        require(AgentBountyFactory(BASE_MAINNET_FACTORY).settlementToken() == BASE_MAINNET_USDC, "factory token drift");

        IVrfSubscriptionCoordinatorV4Fork vrf = IVrfSubscriptionCoordinatorV4Fork(BASE_MAINNET_VRF);
        uint256 subscriptionId = vrf.createSubscription();
        require(subscriptionId != 0, "subscription zero");

        AnonymousProtocolControllerV1 controller = new AnonymousProtocolControllerV1(address(this));
        AnonymousStakePoolV1 pool = new AnonymousStakePoolV1(BASE_MAINNET_USDC, address(controller));
        VrfSortitionCoordinatorV1 verifierSortition =
            new VrfSortitionCoordinatorV1(BASE_MAINNET_VRF, address(controller), subscriptionId, BASE_MAINNET_KEY_HASH);
        VrfSortitionCoordinatorV1 solverSortition =
            new VrfSortitionCoordinatorV1(BASE_MAINNET_VRF, address(controller), subscriptionId, BASE_MAINNET_KEY_HASH);
        AppealableVerifierV1 appeal =
            new AppealableVerifierV1(BASE_MAINNET_USDC, address(controller), address(verifierSortition));
        StandingMetaChildFactoryV4 childFactory =
            new StandingMetaChildFactoryV4(BASE_MAINNET_FACTORY, address(appeal), address(this));
        StandingMetaParentFactoryV4 parentFactory = new StandingMetaParentFactoryV4(
            BASE_MAINNET_FACTORY, address(childFactory), address(controller), address(appeal)
        );
        childFactory.configureParentFactory(address(parentFactory));
        controller.configure(
            address(pool),
            address(verifierSortition),
            address(solverSortition),
            address(appeal),
            address(parentFactory)
        );
        StandingMetaV4Bundle bundle = new StandingMetaV4Bundle(
            BASE_MAINNET_FACTORY,
            address(controller),
            address(pool),
            address(verifierSortition),
            address(solverSortition),
            address(appeal),
            address(childFactory),
            address(parentFactory)
        );
        require(address(bundle.controller()) == address(controller), "bundle controller drift");
        require(parentFactory.ASSIGNMENT_WINDOW() == 2 minutes, "assignment window drift");
        require(parentFactory.CHILD_VERIFICATION_WINDOW() == 24 hours, "verification window drift");
        require(appeal.RESPONSE_WINDOW() == 30 minutes, "response window drift");
        require(appeal.APPEAL_WINDOW() == 4 hours, "appeal window drift");
        require(appeal.VOTING_WINDOW() == 2 hours, "voting window drift");

        vrf.addConsumer(subscriptionId, address(verifierSortition));
        vrf.addConsumer(subscriptionId, address(solverSortition));
        vrf.fundSubscriptionWithNative{value: 1e12}(subscriptionId);
        (, uint96 nativeBalance,, address owner, address[] memory consumers) = vrf.getSubscription(subscriptionId);
        require(nativeBalance == 1e12, "native funding drift");
        require(owner == address(this), "subscription owner drift");
        require(consumers.length == 2, "consumer count drift");
        require(
            (consumers[0] == address(verifierSortition) && consumers[1] == address(solverSortition))
                || (consumers[1] == address(verifierSortition) && consumers[0] == address(solverSortition)),
            "consumer set drift"
        );
    }
}
