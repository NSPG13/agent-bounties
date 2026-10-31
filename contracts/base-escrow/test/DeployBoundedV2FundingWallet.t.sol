// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../script/DeployBoundedV2FundingWallet.s.sol";
import "../src/AgentBountyFactoryV2.sol";
import "./BoundedV2FundingWallet.t.sol";

interface DeployScriptVm {
    function expectRevert(bytes calldata reason) external;
}

contract DeployBoundedV2FundingWalletTest {
    DeployScriptVm constant vm = DeployScriptVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    DeployBoundedV2FundingWallet script;
    address factory;

    function setUp() public {
        script = new DeployBoundedV2FundingWallet();
        factory = address(new AgentBountyFactoryV2(address(new FundingWalletToken()), 750, address(0xFEE)));
    }

    function params() internal view returns (DeployBoundedV2FundingWallet.Params memory) {
        return DeployBoundedV2FundingWallet.Params({
            owner: address(0x0A11CE),
            delegate: address(0xD0D0),
            reviewer: address(0xA0EE),
            validUntil: uint64(block.timestamp + 365 days),
            maxPerBounty: 100_000_000,
            maxPerDay: 500_000_000,
            maxLifetimeSpend: 5_000_000_000
        });
    }

    function testDeploysOnlyTheCustomerReviewSetupWithTheGivenCaps() public {
        BoundedV2FundingWallet wallet = script.deploy(factory, params());
        BoundedV2FundingWallet.Policy memory policy = wallet.policy();
        require(wallet.owner() == address(0x0A11CE) && policy.delegate == address(0xD0D0), "roles");
        require(!policy.delegateMayCancel && policy.periodSeconds == 1 days, "policy");
        require(policy.maxPerBounty == 100_000_000 && policy.maxPerPeriod == 500_000_000, "caps");
        require(policy.maxClaimWindowSeconds == 7 days && policy.maxVerificationWindowSeconds == 7 days, "windows");
        BoundedV2FundingWallet.VerificationConfig[] memory configs = wallet.verificationConfigs();
        address[] memory verifiers = new address[](1);
        verifiers[0] = address(0xA0EE);
        require(configs.length == 1 && configs[0].threshold == 1, "one setup");
        require(configs[0].mode == AgentBountyV2.VerificationMode.SignedQuorum, "quorum");
        require(configs[0].verifierSetHash == keccak256(abi.encode(verifiers)), "reviewer");
    }

    function testRejectsSharedRolesAndInvertedCaps() public {
        DeployBoundedV2FundingWallet.Params memory p = params();
        p.reviewer = p.delegate;
        vm.expectRevert(bytes("roles must differ"));
        script.deploy(factory, p);
        p = params();
        p.maxPerDay = p.maxPerBounty - 1;
        vm.expectRevert(bytes("caps must be per bounty <= per day <= lifetime"));
        script.deploy(factory, p);
        p = params();
        p.validUntil = uint64(block.timestamp);
        vm.expectRevert(bytes("policy already expired"));
        script.deploy(factory, p);
        vm.expectRevert(bytes("factory has no code"));
        script.deploy(address(0xF00D), params());
    }
}
