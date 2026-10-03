// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactory.sol";
import "../src/AgentBountyFactoryV2.sol";
import "../src/ParticipantEligibilityRegistry.sol";

interface V2Vm {
    function warp(uint256) external;
    function addr(uint256 privateKey) external returns (address);
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
    function prank(address sender) external;
}

contract V2TestToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    mapping(address => bool) public blocked;
    mapping(address => mapping(bytes32 => bool)) public authorizationUsed;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function setBlocked(address account, bool value) external {
        blocked[account] = value;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        require(!blocked[to], "blocked recipient");
        require(balanceOf[msg.sender] >= amount, "balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(!blocked[to], "blocked recipient");
        require(balanceOf[from] >= amount, "balance");
        require(allowance[from][msg.sender] >= amount, "allowance");
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    /// @dev Signature checks are covered by the real-USDC fork test; this mock enforces
    /// only the validity window, nonce reuse, and balance, like ProtocolTestToken.
    function transferWithAuthorization(
        address from,
        address to,
        uint256 amount,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8,
        bytes32,
        bytes32
    ) external {
        require(block.timestamp > validAfter && block.timestamp < validBefore, "authorization window");
        require(!authorizationUsed[from][nonce], "authorization used");
        require(balanceOf[from] >= amount, "balance");
        authorizationUsed[from][nonce] = true;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
    }
}

contract V2Actor {
    function approve(V2TestToken token, address spender, uint256 amount) external {
        token.approve(spender, amount);
    }

    function fund(AgentBountyV2 bounty, uint256 amount) external returns (uint256) {
        return bounty.fund(amount);
    }

    function claim(AgentBountyV2 bounty) external {
        bounty.claim();
    }

    function submit(AgentBountyV2 bounty, bytes32 submissionHash, bytes32 evidenceHash) external {
        bounty.submit(submissionHash, evidenceHash);
    }

    function withdrawRefund(AgentBountyV2 bounty) external {
        bounty.withdrawRefund();
    }
}

/// @dev Deterministic module whose verdict is the first proof byte: 0x01 passes, anything else fails.
contract V2VerdictModule is IAgentBountyVerifier {
    function verify(bytes32, uint64, address, bytes32, bytes32, bytes32, bytes calldata proof)
        external
        pure
        returns (bool passed, bytes32 responseHash)
    {
        passed = proof.length > 0 && proof[0] == 0x01;
        responseHash = keccak256(proof);
    }
}

contract AgentBountyV2Test {
    V2Vm constant vm = V2Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint16 constant FEE_BPS = 750;
    address constant FEE_RECIPIENT = address(0xFEE);
    uint256 constant SOLVER_REWARD = 1_000;
    uint256 constant VERIFIER_REWARD = 100;
    uint256 constant EXPECTED_FEE = 75;
    uint256 constant EXPECTED_TARGET = 1_175;

    bytes32 constant TERMS_HASH = keccak256("terms-v2");
    bytes32 constant POLICY_HASH = keccak256("policy-v2");
    bytes32 constant CRITERIA_HASH = keccak256("acceptance-criteria-v2");
    bytes32 constant BENCHMARK_HASH = keccak256("benchmark-v2");
    bytes32 constant EVIDENCE_SCHEMA_HASH = keccak256("evidence-schema-v2");
    bytes32 constant SUBMISSION_HASH = keccak256("artifact");
    bytes32 constant EVIDENCE_HASH = keccak256("evidence-package");

    V2TestToken token;
    AgentBountyFactoryV2 factory;
    V2VerdictModule module;
    V2Actor verifierRecipient;
    uint256 creationNonceCounter;

    function setUp() public {
        token = new V2TestToken();
        factory = new AgentBountyFactoryV2(address(token), FEE_BPS, FEE_RECIPIENT);
        module = new V2VerdictModule();
        verifierRecipient = new V2Actor();
        token.mint(address(this), 10_000_000_000_000);
        token.approve(address(factory), type(uint256).max);
    }

    function testQuoteAndTargetIncludeFeeOnSolverRewardOnly() public {
        (uint256 fee, uint256 target) = factory.quote(SOLVER_REWARD, VERIFIER_REWARD);
        require(fee == EXPECTED_FEE, "fee mismatch");
        require(target == EXPECTED_TARGET, "target mismatch");
        (uint256 feeWithLargerVerifier,) = factory.quote(SOLVER_REWARD, VERIFIER_REWARD * 50);
        require(feeWithLargerVerifier == EXPECTED_FEE, "verifier reward must not carry fee");

        AgentBountyV2 bounty = _create(SOLVER_REWARD, VERIFIER_REWARD, EXPECTED_TARGET);
        require(bounty.platformFeeBps() == FEE_BPS, "bps mismatch");
        require(bounty.platformFee() == EXPECTED_FEE, "bounty fee mismatch");
        require(bounty.platformFeeRecipient() == FEE_RECIPIENT, "recipient mismatch");
        require(bounty.targetAmount() == EXPECTED_TARGET, "bounty target mismatch");
        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Claimable, "not claimable");
        require(bounty.protocolVersion() == keccak256("agent-bounties/autonomous-v2"), "version mismatch");
    }

    function testFeeRoundsUpToNextBaseUnit() public view {
        (uint256 fee,) = factory.quote(1_001, 1);
        require(fee == 76, "1001 * 7.5% must round up to 76");
        (uint256 tinyFee,) = factory.quote(1, 1);
        require(tinyFee == 1, "nonzero reward carries a nonzero fee");
        (uint256 exactFee,) = factory.quote(10_000, 1);
        require(exactFee == 750, "exact multiple must not round");
    }

    function testUnderfundedBountyIsNotClaimable() public {
        AgentBountyV2 bounty = _create(SOLVER_REWARD, VERIFIER_REWARD, SOLVER_REWARD + VERIFIER_REWARD);
        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Open, "fee shortfall must stay open");
        token.approve(address(bounty), EXPECTED_FEE);
        bounty.fund(EXPECTED_FEE);
        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Claimable, "full target must be claimable");
    }

    function testSettlementPaysSolverVerifierAndFeeInOneTransaction() public {
        AgentBountyV2 bounty = _create(SOLVER_REWARD, VERIFIER_REWARD, EXPECTED_TARGET);
        V2Actor solver = _claimAndSubmit(bounty);

        bounty.verifyAndSettle(hex"01");

        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Settled, "not settled");
        require(token.balanceOf(address(solver)) == SOLVER_REWARD + VERIFIER_REWARD, "solver reward plus bond");
        require(token.balanceOf(address(verifierRecipient)) == VERIFIER_REWARD, "verifier reward");
        require(token.balanceOf(FEE_RECIPIENT) == EXPECTED_FEE, "fee must be paid at payout");
        require(bounty.platformFeeAccrued() == 0, "nothing left to forward");
        require(token.balanceOf(address(bounty)) == 0, "bounty retained funds");
    }

    function testNoFeeIsWithdrawableBeforeOrAfterNormalSettlement() public {
        AgentBountyV2 bounty = _create(SOLVER_REWARD, VERIFIER_REWARD, EXPECTED_TARGET);
        (bool earlyOk,) = address(bounty).call(abi.encodeCall(AgentBountyV2.withdrawPlatformFee, ()));
        require(!earlyOk, "fee withdrawn before settlement");

        _claimAndSubmit(bounty);
        (bool submittedOk,) = address(bounty).call(abi.encodeCall(AgentBountyV2.withdrawPlatformFee, ()));
        require(!submittedOk, "fee withdrawn before verdict");

        bounty.verifyAndSettle(hex"01");
        (bool afterOk,) = address(bounty).call(abi.encodeCall(AgentBountyV2.withdrawPlatformFee, ()));
        require(!afterOk, "fee already paid at settlement");
        require(token.balanceOf(FEE_RECIPIENT) == EXPECTED_FEE, "fee paid exactly once");
    }

    function testBlockedFeeRecipientCannotBlockSolverPayment() public {
        AgentBountyV2 bounty = _create(SOLVER_REWARD, VERIFIER_REWARD, EXPECTED_TARGET);
        V2Actor solver = _claimAndSubmit(bounty);
        token.setBlocked(FEE_RECIPIENT, true);

        bounty.verifyAndSettle(hex"01");

        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Settled, "settlement blocked by recipient");
        require(token.balanceOf(address(solver)) == SOLVER_REWARD + VERIFIER_REWARD, "solver unpaid");
        require(token.balanceOf(address(verifierRecipient)) == VERIFIER_REWARD, "verifier unpaid");
        require(bounty.platformFeeAccrued() == EXPECTED_FEE, "failed transfer must defer the fee");
        require(token.balanceOf(address(bounty)) == EXPECTED_FEE, "deferred fee stays escrowed");
        (bool ok,) = address(bounty).call(abi.encodeCall(AgentBountyV2.withdrawPlatformFee, ()));
        require(!ok, "blocked recipient transfer should revert");
        require(bounty.platformFeeAccrued() == EXPECTED_FEE, "failed retry must keep the fee");

        token.setBlocked(FEE_RECIPIENT, false);
        require(bounty.withdrawPlatformFee() == EXPECTED_FEE, "retry amount");
        require(token.balanceOf(FEE_RECIPIENT) == EXPECTED_FEE, "recipient paid after unblock");
        require(token.balanceOf(address(bounty)) == 0, "bounty retained funds");
    }

    function testRejectKeepsFeeEscrowedAndBountyFullyFunded() public {
        AgentBountyV2 bounty = _create(SOLVER_REWARD, VERIFIER_REWARD, EXPECTED_TARGET);
        V2Actor rejected = _claimAndSubmit(bounty);

        bounty.verifyAndSettle(hex"00");

        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Claimable, "reject must reopen");
        require(bounty.fundedAmount() == EXPECTED_TARGET, "target must stay funded");
        require(token.balanceOf(address(bounty)) == EXPECTED_TARGET, "bond must replace verifier reserve");
        require(token.balanceOf(address(rejected)) == 0, "rejected bond forfeited");
        require(token.balanceOf(address(verifierRecipient)) == VERIFIER_REWARD, "verifier paid on fail");
        require(token.balanceOf(FEE_RECIPIENT) == 0, "fee must not be paid on reject");

        V2Actor accepted = _claimAndSubmit(bounty);
        bounty.verifyAndSettle(hex"01");
        require(token.balanceOf(address(accepted)) == SOLVER_REWARD + VERIFIER_REWARD, "second solver paid");
        require(token.balanceOf(address(verifierRecipient)) == VERIFIER_REWARD * 2, "verifier paid per verdict");
        require(token.balanceOf(FEE_RECIPIENT) == EXPECTED_FEE, "fee paid once at accepted payout");
        require(token.balanceOf(address(bounty)) == 0, "bounty retained funds");
    }

    function testClaimTimeoutBonusGoesToSolverAndFeeIsUnchanged() public {
        AgentBountyV2 bounty = _create(SOLVER_REWARD, VERIFIER_REWARD, EXPECTED_TARGET);
        V2Actor expired = _newSolver(bounty);
        expired.claim(bounty);
        vm.warp(block.timestamp + 1 days + 1);
        bounty.expireClaim();
        require(bounty.timeoutBondPool() == VERIFIER_REWARD, "bond to timeout pool");

        V2Actor solver = _claimAndSubmit(bounty);
        bounty.verifyAndSettle(hex"01");

        require(
            token.balanceOf(address(solver)) == SOLVER_REWARD + VERIFIER_REWARD + VERIFIER_REWARD,
            "reward plus bond plus timeout bonus"
        );
        require(token.balanceOf(FEE_RECIPIENT) == EXPECTED_FEE, "fee must not absorb timeout bonus");
        require(token.balanceOf(address(bounty)) == 0, "bounty retained funds");
    }

    function testCancellationRefundsFeeWithPrincipal() public {
        uint64 fundingDeadline = uint64(block.timestamp + 1 days);
        AgentBountyV2 bounty = _createWithDeadline(SOLVER_REWARD, VERIFIER_REWARD, 0, fundingDeadline);
        V2Actor first = _newContributor(bounty, 400);
        V2Actor second = _newContributor(bounty, 375);
        first.fund(bounty, 400);
        second.fund(bounty, 375);
        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Open, "partial pool stays open");

        vm.warp(uint256(fundingDeadline) + 1);
        bounty.cancel();
        first.withdrawRefund(bounty);
        second.withdrawRefund(bounty);

        require(token.balanceOf(address(first)) == 400, "first contributor refund");
        require(token.balanceOf(address(second)) == 375, "second contributor refund");
        require(token.balanceOf(FEE_RECIPIENT) == 0, "no fee on cancellation");
        require(bounty.platformFeeAccrued() == 0, "no accrual on cancellation");
        require(token.balanceOf(address(bounty)) == 0, "bounty retained funds");
    }

    function testZeroFeeFactoryMatchesV1Economics() public {
        AgentBountyFactoryV2 zeroFee = new AgentBountyFactoryV2(address(token), 0, address(0));
        (uint256 fee, uint256 target) = zeroFee.quote(SOLVER_REWARD, VERIFIER_REWARD);
        require(fee == 0 && target == SOLVER_REWARD + VERIFIER_REWARD, "zero fee quote");
    }

    function testFactoryRejectsFeeAboveCapAndInconsistentRecipient() public {
        try new AgentBountyFactoryV2(address(token), 1_001, FEE_RECIPIENT) {
            revert("fee above cap accepted");
        } catch {}
        try new AgentBountyFactoryV2(address(token), FEE_BPS, address(0)) {
            revert("fee without recipient accepted");
        } catch {}
        try new AgentBountyFactoryV2(address(token), 0, FEE_RECIPIENT) {
            revert("recipient without fee accepted");
        } catch {}
        AgentBountyFactoryV2 capped = new AgentBountyFactoryV2(address(token), 1_000, FEE_RECIPIENT);
        require(capped.platformFeeBps() == 1_000, "cap itself must be allowed");
    }

    function testInterfacesAndVersionSeparationFromV1() public {
        AgentBountyV2 bounty = _create(SOLVER_REWARD, VERIFIER_REWARD, EXPECTED_TARGET);
        require(bounty.supportsInterface(type(IAgentBountyV2).interfaceId), "v2 interface");
        require(bounty.supportsInterface(type(IAgentBountyV1).interfaceId), "v1 surface");
        require(bounty.supportsInterface(type(IERC165).interfaceId), "erc165");

        AgentBountyFactory v1Factory = new AgentBountyFactory(address(token));
        (bool listedAsV1,) =
            address(v1Factory).call(abi.encodeCall(AgentBountyFactory.submitExternalBounty, (address(bounty))));
        require(!listedAsV1, "v1 factory must not list a v2 bounty");
    }

    function testSignatureDomainUsesVersionTwo() public {
        AgentBountyV2 bounty = _create(SOLVER_REWARD, VERIFIER_REWARD, EXPECTED_TARGET);
        address solver = address(0xBEEF);
        bytes32 domain = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("Agent Bounties"),
                keccak256("2"),
                block.chainid,
                address(bounty)
            )
        );
        bytes32 structHash = keccak256(
            abi.encode(
                keccak256(
                    "Claim(address bounty,bytes32 bountyId,address solver,uint64 round,bytes32 termsHash,bytes32 policyHash,uint256 deadline)"
                ),
                address(bounty),
                bounty.bountyId(),
                solver,
                uint64(1),
                TERMS_HASH,
                POLICY_HASH,
                uint256(123)
            )
        );
        bytes32 expected = keccak256(abi.encodePacked("\x19\x01", domain, structHash));
        require(bounty.claimDigest(solver, 1, 123) == expected, "claim digest must use domain version 2");
    }

    function testEligibilityGateAdmitsOnlyCurrentContractorsFromTheCommittedSource() public {
        uint256 attesterKey = 0xA77E57;
        ParticipantEligibilityRegistry registry = new ParticipantEligibilityRegistry(vm.addr(attesterKey));
        bytes32 contractorSource = keccak256("agent-bounties/invoice-contractor-v1");
        AgentBountyV2 bounty = _createGated(
            SOLVER_REWARD,
            VERIFIER_REWARD,
            EXPECTED_TARGET,
            uint64(block.timestamp + 1 days),
            address(registry),
            contractorSource
        );
        require(bounty.claimEligibilityRegistry() == address(registry), "registry not stored");
        require(bounty.claimEligibilitySource() == contractorSource, "source not stored");

        V2Actor unregistered = _newSolver(bounty);
        (bool unregisteredOk,) = address(unregistered).call(abi.encodeCall(V2Actor.claim, (bounty)));
        require(!unregisteredOk, "unregistered wallet claimed");

        V2Actor otherSource = _newSolver(bounty);
        _attest(registry, attesterKey, address(otherSource), keccak256("some-other-program"), 30 days);
        (bool otherSourceOk,) = address(otherSource).call(abi.encodeCall(V2Actor.claim, (bounty)));
        require(!otherSourceOk, "wallet from another source claimed");

        V2Actor lapsed = _newSolver(bounty);
        _attest(registry, attesterKey, address(lapsed), contractorSource, 1 hours);
        vm.warp(block.timestamp + 1 hours + 1);
        (bool lapsedOk,) = address(lapsed).call(abi.encodeCall(V2Actor.claim, (bounty)));
        require(!lapsedOk, "expired contractor claimed");

        V2Actor contractor = _newSolver(bounty);
        _attest(registry, attesterKey, address(contractor), contractorSource, 30 days);
        contractor.claim(bounty);
        contractor.submit(bounty, SUBMISSION_HASH, EVIDENCE_HASH);
        bounty.verifyAndSettle(hex"01");
        require(token.balanceOf(address(contractor)) == SOLVER_REWARD + VERIFIER_REWARD, "contractor paid");
        require(token.balanceOf(FEE_RECIPIENT) == EXPECTED_FEE, "fee paid at payout");
    }

    function testUngatedBountyStaysPermissionless() public {
        AgentBountyV2 bounty = _create(SOLVER_REWARD, VERIFIER_REWARD, EXPECTED_TARGET);
        require(bounty.claimEligibilityRegistry() == address(0), "unexpected registry");
        require(bounty.claimEligibilitySource() == bytes32(0), "unexpected source");
        _claimAndSubmit(bounty);
        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Submitted, "any wallet may claim");
    }

    function testFactoryRejectsInconsistentEligibilityConfig() public {
        ParticipantEligibilityRegistry registry = new ParticipantEligibilityRegistry(address(0xA77E57));
        uint64 deadline = uint64(block.timestamp + 1 days);
        try this.createGatedExternal(address(registry), bytes32(0), deadline) {
            revert("registry without source accepted");
        } catch {}
        try this.createGatedExternal(address(0), keccak256("source"), deadline) {
            revert("source without registry accepted");
        } catch {}
        try this.createGatedExternal(address(0xC0DE), keccak256("source"), deadline) {
            revert("registry without code accepted");
        } catch {}
    }

    function createGatedExternal(address registry, bytes32 source, uint64 deadline) external returns (address) {
        return address(_createGated(SOLVER_REWARD, VERIFIER_REWARD, 0, deadline, registry, source));
    }

    function testRelayedAuthorizationCreatesFundedBountyWithFeeInTarget() public {
        address poster = address(0xB0B);
        token.mint(poster, EXPECTED_TARGET);
        creationNonceCounter += 1;
        AgentBountyFactoryV2.CreateBountyParams memory params = _params(
            SOLVER_REWARD, VERIFIER_REWARD, uint64(block.timestamp + 1 days), address(0), bytes32(0)
        );
        address predicted =
            factory.predictBountyAddress(poster, params, new address[](0), bytes32(creationNonceCounter));
        AgentBountyFactoryV2.FundingAuthorization memory authorization = AgentBountyFactoryV2.FundingAuthorization({
            validAfter: 0, validBefore: block.timestamp + 1 hours, nonce: keccak256("create"), v: 27, r: 0, s: 0
        });

        (address bountyAddress,) = factory.createBountyWithAuthorization(
            poster, params, new address[](0), EXPECTED_TARGET, bytes32(creationNonceCounter), authorization
        );

        AgentBountyV2 bounty = AgentBountyV2(bountyAddress);
        require(bountyAddress == predicted, "predicted address mismatch");
        require(bounty.creator() == poster, "creator must be the authorizing poster");
        require(bounty.contributions(poster) == EXPECTED_TARGET, "poster contribution includes fee");
        require(bounty.bountyStatus() == AgentBountyV2.BountyStatus.Claimable, "fully funded with fee");
        require(token.balanceOf(poster) == 0, "exact authorization amount");
    }

    uint256 constant ATTESTER_KEY = 0xA77E57;
    uint256 constant CONTRACTOR_KEY = 0x501E;
    bytes32 constant CONTRACTOR_SOURCE = keccak256("agent-bounties/invoice-contractor-v1");

    function testEligibilityGateAppliesToSignedClaims() public {
        ParticipantEligibilityRegistry registry = new ParticipantEligibilityRegistry(vm.addr(ATTESTER_KEY));
        AgentBountyV2 bounty = _createContractorOnly(registry);
        address contractor = vm.addr(CONTRACTOR_KEY);
        token.mint(contractor, VERIFIER_REWARD);
        vm.prank(contractor);
        token.approve(address(bounty), VERIFIER_REWARD);
        uint256 deadline = block.timestamp + 1 hours;
        bytes memory signature = _claimSignature(bounty, contractor, deadline);

        (bool unregistered,) = address(bounty).call(
            abi.encodeCall(AgentBountyV2.claimWithSignature, (contractor, deadline, signature))
        );
        require(!unregistered, "unregistered signed claim accepted");

        _attest(registry, ATTESTER_KEY, contractor, CONTRACTOR_SOURCE, 30 days);
        bounty.claimWithSignature(contractor, deadline, signature);
        require(bounty.solver() == contractor, "registered signed claim rejected");
        require(token.balanceOf(contractor) == 0, "bond collected");
    }

    function testEligibilityGateAppliesToAuthorizedClaims() public {
        ParticipantEligibilityRegistry registry = new ParticipantEligibilityRegistry(vm.addr(ATTESTER_KEY));
        AgentBountyV2 bounty = _createContractorOnly(registry);
        address contractor = vm.addr(CONTRACTOR_KEY);
        token.mint(contractor, VERIFIER_REWARD);

        (bool unregistered,) = address(bounty).call(
            abi.encodeCall(
                AgentBountyV2.claimWithAuthorization,
                (contractor, 0, block.timestamp + 1 hours, keccak256("bond-a"), 27, bytes32(0), bytes32(0))
            )
        );
        require(!unregistered, "unregistered authorized claim accepted");

        _attest(registry, ATTESTER_KEY, contractor, CONTRACTOR_SOURCE, 30 days);
        bounty.claimWithAuthorization(
            contractor, 0, block.timestamp + 1 hours, keccak256("bond-b"), 27, bytes32(0), bytes32(0)
        );
        require(bounty.solver() == contractor, "registered authorized claim rejected");
        require(token.balanceOf(contractor) == 0, "bond collected");
    }

    function _createContractorOnly(ParticipantEligibilityRegistry registry) private returns (AgentBountyV2) {
        return _createGated(
            SOLVER_REWARD,
            VERIFIER_REWARD,
            EXPECTED_TARGET,
            uint64(block.timestamp + 1 days),
            address(registry),
            CONTRACTOR_SOURCE
        );
    }

    function _claimSignature(AgentBountyV2 bounty, address solver, uint256 deadline)
        private
        returns (bytes memory)
    {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(CONTRACTOR_KEY, bounty.claimDigest(solver, bounty.round() + 1, deadline));
        return abi.encodePacked(r, s, v);
    }

    function testFuzzSettlementConservesEveryBaseUnit(uint64 solverSeed, uint64 verifierSeed) public {
        uint256 solverReward = 1 + uint256(solverSeed) % 1_000_000_000_000;
        uint256 verifierReward = 1 + uint256(verifierSeed) % 1_000_000_000;
        (uint256 fee, uint256 target) = factory.quote(solverReward, verifierReward);
        require(fee * 10_000 >= solverReward * FEE_BPS, "fee must not round down");
        require(fee * 10_000 < solverReward * FEE_BPS + 10_000, "fee rounds up by less than one unit");

        AgentBountyV2 bounty = _create(solverReward, verifierReward, target);
        V2Actor solver = _newSolver(bounty);
        solver.claim(bounty);
        solver.submit(bounty, SUBMISSION_HASH, EVIDENCE_HASH);
        bounty.verifyAndSettle(hex"01");

        require(token.balanceOf(address(solver)) == solverReward + verifierReward, "solver total");
        require(token.balanceOf(address(verifierRecipient)) == verifierReward, "verifier total");
        require(token.balanceOf(FEE_RECIPIENT) == fee, "fee total");
        require(token.balanceOf(address(bounty)) == 0, "dust left in bounty");
    }

    function testFuzzCancellationRefundsEveryContributorExactly(uint64 solverSeed, uint64 splitSeed) public {
        uint256 solverReward = 1 + uint256(solverSeed) % 1_000_000_000_000;
        uint256 verifierReward = 1 + uint256(solverSeed >> 7) % 1_000_000;
        (, uint256 target) = factory.quote(solverReward, verifierReward);
        // Two contributors leave the pool one base unit short of target, so the fee share is still escrowed.
        uint256 firstAmount = 1 + uint256(splitSeed) % (target - 2);
        uint256 secondAmount = target - 1 - firstAmount;

        uint64 fundingDeadline = uint64(block.timestamp + 1 days);
        AgentBountyV2 bounty = _createWithDeadline(solverReward, verifierReward, 0, fundingDeadline);
        V2Actor first = _newContributor(bounty, firstAmount);
        V2Actor second = _newContributor(bounty, secondAmount);
        first.fund(bounty, firstAmount);
        second.fund(bounty, secondAmount);

        vm.warp(uint256(fundingDeadline) + 1);
        bounty.cancel();
        first.withdrawRefund(bounty);
        second.withdrawRefund(bounty);

        require(token.balanceOf(address(first)) == firstAmount, "first refund");
        require(token.balanceOf(address(second)) == secondAmount, "second refund");
        require(token.balanceOf(address(bounty)) == 0, "dust left in bounty");
        require(token.balanceOf(FEE_RECIPIENT) == 0, "fee taken on cancellation");
    }

    function _create(uint256 solverReward, uint256 verifierReward, uint256 initialFunding)
        private
        returns (AgentBountyV2)
    {
        return _createWithDeadline(solverReward, verifierReward, initialFunding, uint64(block.timestamp + 1 days));
    }

    function _createWithDeadline(
        uint256 solverReward,
        uint256 verifierReward,
        uint256 initialFunding,
        uint64 fundingDeadline
    ) private returns (AgentBountyV2) {
        return _createGated(solverReward, verifierReward, initialFunding, fundingDeadline, address(0), bytes32(0));
    }

    function _createGated(
        uint256 solverReward,
        uint256 verifierReward,
        uint256 initialFunding,
        uint64 fundingDeadline,
        address eligibilityRegistry,
        bytes32 eligibilitySource
    ) private returns (AgentBountyV2) {
        creationNonceCounter += 1;
        AgentBountyFactoryV2.CreateBountyParams memory params =
            _params(solverReward, verifierReward, fundingDeadline, eligibilityRegistry, eligibilitySource);
        (address bountyAddress,) =
            factory.createBounty(params, new address[](0), initialFunding, bytes32(creationNonceCounter));
        return AgentBountyV2(bountyAddress);
    }

    function _params(
        uint256 solverReward,
        uint256 verifierReward,
        uint64 fundingDeadline,
        address eligibilityRegistry,
        bytes32 eligibilitySource
    ) private view returns (AgentBountyFactoryV2.CreateBountyParams memory) {
        return AgentBountyFactoryV2.CreateBountyParams({
            solverReward: solverReward,
            verifierReward: verifierReward,
            termsHash: TERMS_HASH,
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: CRITERIA_HASH,
            benchmarkHash: BENCHMARK_HASH,
            evidenceSchemaHash: EVIDENCE_SCHEMA_HASH,
            fundingDeadline: fundingDeadline,
            claimWindowSeconds: 1 days,
            verificationWindowSeconds: 1 days,
            verificationMode: AgentBountyV2.VerificationMode.DeterministicModule,
            verifierModule: address(module),
            verifierRewardRecipient: address(verifierRecipient),
            threshold: 1,
            claimEligibilityRegistry: eligibilityRegistry,
            claimEligibilitySource: eligibilitySource
        });
    }

    function _newSolver(AgentBountyV2 bounty) private returns (V2Actor solver) {
        solver = new V2Actor();
        uint256 bond = bounty.verifierReward();
        token.mint(address(solver), bond);
        solver.approve(token, address(bounty), bond);
    }

    function _newContributor(AgentBountyV2 bounty, uint256 amount) private returns (V2Actor contributor) {
        contributor = new V2Actor();
        token.mint(address(contributor), amount);
        contributor.approve(token, address(bounty), amount);
    }

    function _attest(
        ParticipantEligibilityRegistry registry,
        uint256 attesterKey,
        address wallet,
        bytes32 source,
        uint64 validFor
    ) private {
        bytes32 participantId = keccak256(abi.encode(wallet));
        uint64 validUntil = uint64(block.timestamp) + validFor;
        bytes32 digest =
            registry.attestationDigest(wallet, participantId, source, validUntil, registry.nonces(wallet));
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(attesterKey, digest);
        registry.register(wallet, participantId, source, validUntil, abi.encodePacked(r, s, v));
    }

    function _claimAndSubmit(AgentBountyV2 bounty) private returns (V2Actor solver) {
        solver = _newSolver(bounty);
        solver.claim(bounty);
        solver.submit(bounty, SUBMISSION_HASH, EVIDENCE_HASH);
    }
}
