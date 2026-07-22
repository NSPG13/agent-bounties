// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

interface StandingMetaV4FundingVm {
    function addr(uint256 privateKey) external returns (address);
    function envAddress(string calldata name) external returns (address);
    function envString(string calldata name) external returns (string memory);
    function envUint(string calldata name) external returns (uint256);
    function startBroadcast(uint256 privateKey) external;
    function stopBroadcast() external;
    function serializeAddress(string calldata objectKey, string calldata valueKey, address value)
        external
        returns (string memory);
    function serializeUint(string calldata objectKey, string calldata valueKey, uint256 value)
        external
        returns (string memory);
    function writeJson(string calldata json, string calldata path) external;
}

interface IVrfNativeSubscriptionFundingV2Plus {
    function fundSubscriptionWithNative(uint256 subId) external payable;
    function getSubscription(uint256 subId)
        external
        view
        returns (uint96 balance, uint96 nativeBalance, uint64 reqCount, address subOwner, address[] memory consumers);
}

/// @notice Funds one already-created V4 subscription with the exact native
/// amount produced by a separately evidenced, owner-authorized conversion.
contract FundStandingMetaV4Subscription {
    StandingMetaV4FundingVm private constant vm =
        StandingMetaV4FundingVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    uint256 private constant BASE_MAINNET_CHAIN_ID = 8_453;
    uint256 private constant BASE_SEPOLIA_CHAIN_ID = 84_532;
    address private constant BASE_MAINNET_VRF = 0xd5D517aBE5cF79B7e95eC98dB0f0277788aFF634;
    address private constant BASE_SEPOLIA_VRF = 0x5C210eF41CD1a72de73bF76eC39637bB0d3d7BEE;
    uint256 private constant MAINNET_SOURCE_USDC_CAP = 7_000_000;

    struct FundingContext {
        address vrf;
        address funder;
        address verifierSortition;
        address solverSortition;
        uint256 subscriptionId;
        uint256 fundingWei;
        uint256 sourceUsdc;
        uint96 nativeBefore;
    }

    function run() external {
        require(block.chainid == BASE_MAINNET_CHAIN_ID || block.chainid == BASE_SEPOLIA_CHAIN_ID, "Base only");
        bool mainnet = block.chainid == BASE_MAINNET_CHAIN_ID;
        uint256 deployerKey = vm.envUint("BASE_KEEPER_PRIVATE_KEY");
        FundingContext memory context = _loadContext(deployerKey, mainnet);
        _validateInputs(context, mainnet);
        context.nativeBefore = _validateSubscription(context);

        vm.startBroadcast(deployerKey);
        IVrfNativeSubscriptionFundingV2Plus(context.vrf).fundSubscriptionWithNative{value: context.fundingWei}(
            context.subscriptionId
        );
        vm.stopBroadcast();

        uint96 nativeAfter = _validateFunding(context);
        _writeEvidence(context, nativeAfter);
    }

    function _loadContext(uint256 deployerKey, bool mainnet) private returns (FundingContext memory) {
        return FundingContext({
            vrf: mainnet ? BASE_MAINNET_VRF : BASE_SEPOLIA_VRF,
            funder: vm.addr(deployerKey),
            verifierSortition: vm.envAddress("V4_VERIFIER_SORTITION"),
            solverSortition: vm.envAddress("V4_SOLVER_SORTITION"),
            subscriptionId: vm.envUint("V4_SUBSCRIPTION_ID"),
            fundingWei: vm.envUint("V4_NATIVE_FUNDING_WEI"),
            sourceUsdc: vm.envUint("V4_SOURCE_USDC_BASE_UNITS"),
            nativeBefore: 0
        });
    }

    function _validateInputs(FundingContext memory context, bool mainnet) private pure {
        require(context.subscriptionId != 0 && context.fundingWei != 0, "funding input zero");
        require(
            context.verifierSortition != address(0) && context.solverSortition != address(0)
                && context.verifierSortition != context.solverSortition,
            "consumer input invalid"
        );
        if (mainnet) {
            require(
                context.sourceUsdc != 0 && context.sourceUsdc <= MAINNET_SOURCE_USDC_CAP,
                "source USDC cap exceeded"
            );
        } else {
            require(context.sourceUsdc == 0, "testnet source USDC must be zero");
        }
    }

    function _validateSubscription(FundingContext memory context) private view returns (uint96) {
        uint96 nativeBefore;
        address subscriptionOwner;
        address[] memory consumers;
        (, nativeBefore,, subscriptionOwner, consumers) =
            IVrfNativeSubscriptionFundingV2Plus(context.vrf).getSubscription(context.subscriptionId);
        require(subscriptionOwner == context.funder, "subscription owner is not funder");
        require(
            _exactConsumers(consumers, context.verifierSortition, context.solverSortition), "exact consumers required"
        );
        require(context.funder.balance > context.fundingWei, "funder native balance insufficient");
        return nativeBefore;
    }

    function _validateFunding(FundingContext memory context) private view returns (uint96) {
        (, uint96 nativeAfter,, address ownerAfter, address[] memory consumersAfter) =
            IVrfNativeSubscriptionFundingV2Plus(context.vrf).getSubscription(context.subscriptionId);
        require(
            ownerAfter == context.funder
                && _exactConsumers(consumersAfter, context.verifierSortition, context.solverSortition),
            "subscription drift"
        );
        require(
            uint256(nativeAfter) == uint256(context.nativeBefore) + context.fundingWei, "native funding delta mismatch"
        );
        return nativeAfter;
    }

    function _writeEvidence(FundingContext memory context, uint96 nativeAfter) private {
        string memory key = "standing-meta-v4-subscription-funding";
        vm.serializeUint(key, "chain_id", block.chainid);
        vm.serializeAddress(key, "funder", context.funder);
        vm.serializeAddress(key, "vrf_coordinator", context.vrf);
        vm.serializeAddress(key, "verifier_sortition", context.verifierSortition);
        vm.serializeAddress(key, "solver_sortition", context.solverSortition);
        vm.serializeUint(key, "subscription_id", context.subscriptionId);
        vm.serializeUint(key, "source_usdc_base_units", context.sourceUsdc);
        vm.serializeUint(key, "native_balance_before", context.nativeBefore);
        vm.serializeUint(key, "native_funding_wei", context.fundingWei);
        string memory json = vm.serializeUint(key, "native_balance_after", nativeAfter);
        vm.writeJson(json, vm.envString("FUNDING_EVIDENCE_PATH"));
    }

    function _exactConsumers(address[] memory consumers, address verifierSortition, address solverSortition)
        private
        pure
        returns (bool)
    {
        return consumers.length == 2
            && ((consumers[0] == verifierSortition && consumers[1] == solverSortition)
                || (consumers[1] == verifierSortition && consumers[0] == solverSortition));
    }
}
