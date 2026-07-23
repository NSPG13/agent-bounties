// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactory.sol";
import "../src/AtomicClaimSponsor.sol";

interface VmAtomicSponsorFork {
    function addr(uint256 privateKey) external returns (address keyAddr);
    function createSelectFork(string calldata urlOrAlias, uint256 blockNumber) external returns (uint256 forkId);
    function envOr(string calldata name, bool defaultValue) external returns (bool value);
    function envString(string calldata name) external returns (string memory value);
    function prank(address sender) external;
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
    function skip(bool skipTest) external;
}

interface AtomicSponsorMainnetUsdc {
    function approve(address spender, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
    function name() external view returns (string memory);
    function version() external view returns (string memory);
    function transfer(address recipient, uint256 amount) external returns (bool);
}

contract AtomicSponsorForkVerifier is IAgentBountyVerifier {
    function verify(bytes32, uint64, address, bytes32, bytes32, bytes32, bytes calldata proof)
        external
        pure
        returns (bool passed, bytes32 responseHash)
    {
        responseHash = keccak256(proof);
        return (keccak256(proof) == keccak256("pass"), responseHash);
    }
}

/// @notice Opt-in atomic sponsorship rehearsals against real Base USDC deployments.
/// Set the matching RUN_*_FORK flag and RPC URL. Fork mutations never broadcast.
contract AtomicClaimSponsorMainnetForkTest {
    VmAtomicSponsorFork private constant vm =
        VmAtomicSponsorFork(address(uint160(uint256(keccak256("hevm cheat code")))));

    address private constant MAINNET_USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
    address private constant MAINNET_FUNDING_SOURCE = 0x884834E884d6e93462655A2820140aD03E6747bC;
    address private constant SEPOLIA_USDC = 0x036CbD53842c5426634e7929541eC2318f3dCF7e;
    address private constant SEPOLIA_FUNDING_SOURCE = 0x74E1608EC3E5F8B6B3f57D22301a11A5b9Fb736D;
    address private constant VERIFIER_RECIPIENT = 0x000000000000000000000000000000000000bEEF;
    uint256 private constant MAINNET_FORK_BLOCK = 48_567_240;
    uint256 private constant SEPOLIA_FORK_BLOCK = 44_207_324;
    uint256 private constant GRANT_SIGNER_KEY = uint256(keccak256("atomic-sponsor/fork/grant-signer"));
    uint256 private constant SOLVER_KEY = uint256(keccak256("atomic-sponsor/fork/solver"));
    uint256 private constant BOND = 10_000;
    uint256 private constant SOLVER_REWARD = 990_000;

    bytes32 private constant EIP712_DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant TRANSFER_WITH_AUTHORIZATION_TYPEHASH = keccak256(
        "TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)"
    );
    bytes32 private constant TERMS_HASH = keccak256("atomic-sponsor-fork-terms-v1");
    bytes32 private constant POLICY_HASH = keccak256("atomic-sponsor-fork-policy-v1");
    bytes32 private constant SUBMISSION_HASH = keccak256("atomic-sponsor-fork-artifact");
    bytes32 private constant EVIDENCE_HASH = keccak256("atomic-sponsor-fork-evidence");

    struct ForkContext {
        AtomicSponsorMainnetUsdc usdc;
        AgentBounty bounty;
        AtomicClaimSponsor sponsor;
        address solver;
        uint256 verifierBefore;
    }

    function testRealUsdcZeroBalanceSolverCompletesSponsoredLoop() public {
        if (!vm.envOr("RUN_MAINNET_FORK", false)) {
            vm.skip(true);
            return;
        }
        vm.createSelectFork(vm.envString("BASE_MAINNET_RPC_URL"), MAINNET_FORK_BLOCK);
        require(block.chainid == 8453, "wrong chain");
        _runSponsoredLoop(MAINNET_USDC, MAINNET_FUNDING_SOURCE);
    }

    function testRealUsdcZeroBalanceSolverCompletesSponsoredLoopOnBaseSepolia() public {
        if (!vm.envOr("RUN_SEPOLIA_FORK", false)) {
            vm.skip(true);
            return;
        }
        vm.createSelectFork(vm.envString("BASE_SEPOLIA_RPC_URL"), SEPOLIA_FORK_BLOCK);
        require(block.chainid == 84532, "wrong chain");
        _runSponsoredLoop(SEPOLIA_USDC, SEPOLIA_FUNDING_SOURCE);
    }

    function _runSponsoredLoop(address settlementToken, address fundingSource) private {
        require(settlementToken.code.length > 0, "USDC code missing");
        ForkContext memory context = _setUpForkLoop(settlementToken, fundingSource);
        _sponsorClaim(context);
        _submitAndSettle(context);
        _assertSettled(context);
    }

    function _setUpForkLoop(address settlementToken, address fundingSource)
        private
        returns (ForkContext memory context)
    {
        context.usdc = AtomicSponsorMainnetUsdc(settlementToken);
        context.solver = vm.addr(SOLVER_KEY);
        require(context.usdc.balanceOf(context.solver) == 0, "solver must start at zero USDC");

        AgentBountyFactory factory = new AgentBountyFactory(settlementToken);
        AtomicSponsorForkVerifier verifier = new AtomicSponsorForkVerifier();
        context.sponsor = new AtomicClaimSponsor(
            settlementToken, address(factory), vm.addr(GRANT_SIGNER_KEY), BOND, BOND * 3, BOND
        );

        vm.prank(fundingSource);
        require(context.usdc.transfer(address(this), SOLVER_REWARD + BOND), "bounty funding transfer failed");
        vm.prank(fundingSource);
        require(context.usdc.transfer(address(context.sponsor), BOND), "sponsor funding transfer failed");
        require(context.usdc.approve(address(factory), SOLVER_REWARD + BOND), "factory approval failed");

        context.bounty = _createBounty(factory, verifier);
        context.verifierBefore = context.usdc.balanceOf(VERIFIER_RECIPIENT);
    }

    function _sponsorClaim(ForkContext memory context) private {
        AtomicClaimSponsor.Grant memory grant = _grant(context.bounty, context.solver);
        (bytes memory grantSignature, uint8 claimV, bytes32 claimR, bytes32 claimS) = _signClaim(context.sponsor, grant);

        context.sponsor.sponsorAndClaim(grant, grantSignature, claimV, claimR, claimS);
        require(context.bounty.bountyStatus() == AgentBounty.BountyStatus.Claimed, "sponsored claim missing");
        require(context.bounty.solver() == context.solver, "solver mismatch");
        require(context.bounty.activeClaimBond() == BOND, "bond mismatch");
        require(context.usdc.balanceOf(context.solver) == 0, "claim left solver funds outside bounty");
        require(context.usdc.balanceOf(address(context.sponsor)) == 0, "sponsor did not spend exact bond");
    }

    function _submitAndSettle(ForkContext memory context) private {
        uint256 submitDeadline = block.timestamp + 30 minutes;
        bytes32 submitDigest = context.bounty
            .submitDigest(context.solver, context.bounty.round(), SUBMISSION_HASH, EVIDENCE_HASH, submitDeadline);
        (uint8 submitV, bytes32 submitR, bytes32 submitS) = vm.sign(SOLVER_KEY, submitDigest);
        context.bounty
            .submitWithSignature(
                SUBMISSION_HASH, EVIDENCE_HASH, submitDeadline, abi.encodePacked(submitR, submitS, submitV)
            );
        context.bounty.verifyAndSettle(bytes("pass"));
    }

    function _assertSettled(ForkContext memory context) private view {
        require(context.bounty.bountyStatus() == AgentBounty.BountyStatus.Settled, "sponsored loop not settled");
        require(context.usdc.balanceOf(address(context.bounty)) == 0, "settled bounty retained USDC");
        require(context.usdc.balanceOf(context.solver) == SOLVER_REWARD + BOND, "solver payout mismatch");
        require(context.usdc.balanceOf(VERIFIER_RECIPIENT) == context.verifierBefore + BOND, "verifier payout mismatch");
        require(context.sponsor.lifetimeSponsored(context.solver) == BOND, "sponsorship accounting missing");
    }

    function _createBounty(AgentBountyFactory factory, AtomicSponsorForkVerifier verifier)
        private
        returns (AgentBounty)
    {
        AgentBountyFactory.CreateBountyParams memory params = AgentBountyFactory.CreateBountyParams({
            solverReward: SOLVER_REWARD,
            verifierReward: BOND,
            termsHash: TERMS_HASH,
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: keccak256("atomic-sponsor-fork-criteria"),
            benchmarkHash: keccak256("atomic-sponsor-fork-benchmark"),
            evidenceSchemaHash: keccak256("atomic-sponsor-fork-evidence-schema"),
            fundingDeadline: uint64(block.timestamp + 1 days),
            claimWindowSeconds: 1 hours,
            verificationWindowSeconds: 1 hours,
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: address(verifier),
            verifierRewardRecipient: VERIFIER_RECIPIENT,
            threshold: 1
        });
        (address bountyAddress,) = factory.createBounty(
            params, new address[](0), SOLVER_REWARD + BOND, keccak256("atomic-sponsor-fork-bounty-v1")
        );
        return AgentBounty(bountyAddress);
    }

    function _grant(AgentBounty bounty, address solver) private view returns (AtomicClaimSponsor.Grant memory) {
        return AtomicClaimSponsor.Grant({
            bounty: address(bounty),
            solver: solver,
            round: bounty.round() + 1,
            bond: BOND,
            termsHash: bounty.termsHash(),
            policyHash: bounty.policyHash(),
            authorizationNonce: keccak256("atomic-sponsor-fork-authorization-v1"),
            validAfter: block.timestamp - 1,
            validBefore: block.timestamp + 1 hours,
            grantNonce: keccak256("atomic-sponsor-fork-grant-v1"),
            deadline: block.timestamp + 30 minutes
        });
    }

    function _signClaim(AtomicClaimSponsor sponsor, AtomicClaimSponsor.Grant memory grant)
        private
        returns (bytes memory grantSignature, uint8 claimV, bytes32 claimR, bytes32 claimS)
    {
        (uint8 grantV, bytes32 grantR, bytes32 grantS) = vm.sign(GRANT_SIGNER_KEY, sponsor.grantDigest(grant));
        grantSignature = abi.encodePacked(grantR, grantS, grantV);
        AtomicSponsorMainnetUsdc usdc = AtomicSponsorMainnetUsdc(sponsor.settlementToken());
        bytes32 authorizationDigest = _usdcAuthorizationDigest(grant, usdc);
        (claimV, claimR, claimS) = vm.sign(SOLVER_KEY, authorizationDigest);
    }

    function _usdcAuthorizationDigest(AtomicClaimSponsor.Grant memory grant, AtomicSponsorMainnetUsdc usdc)
        private
        view
        returns (bytes32)
    {
        bytes32 domainSeparator =
            keccak256(
                abi.encode(
                    EIP712_DOMAIN_TYPEHASH,
                    keccak256(bytes(usdc.name())),
                    keccak256(bytes(usdc.version())),
                    block.chainid,
                    address(usdc)
                )
            );
        bytes32 structHash = keccak256(
            abi.encode(
                TRANSFER_WITH_AUTHORIZATION_TYPEHASH,
                grant.solver,
                grant.bounty,
                grant.bond,
                grant.validAfter,
                grant.validBefore,
                grant.authorizationNonce
            )
        );
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator, structHash));
    }
}
