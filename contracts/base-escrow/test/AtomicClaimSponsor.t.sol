// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactory.sol";
import "../src/AtomicClaimSponsor.sol";

interface VmAtomicSponsor {
    function addr(uint256 privateKey) external returns (address keyAddr);
    function prank(address sender) external;
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
    function warp(uint256 timestamp) external;
}

contract AuthorizationToken {
    bytes32 private constant EIP712_DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant TRANSFER_WITH_AUTHORIZATION_TYPEHASH = keccak256(
        "TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)"
    );
    bytes32 private constant NAME_HASH = keccak256("USD Coin");
    bytes32 private constant VERSION_HASH = keccak256("2");

    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    mapping(address => mapping(bytes32 => bool)) public authorizationUsed;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        require(balanceOf[msg.sender] >= amount, "balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(balanceOf[from] >= amount, "balance");
        require(allowance[from][msg.sender] >= amount, "allowance");
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferWithAuthorization(
        address from,
        address to,
        uint256 amount,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external {
        require(block.timestamp > validAfter, "authorization not active");
        require(block.timestamp < validBefore, "authorization expired");
        require(!authorizationUsed[from][nonce], "authorization used");
        bytes32 digest = authorizationDigest(from, to, amount, validAfter, validBefore, nonce);
        require(ecrecover(digest, v, r, s) == from, "authorization signer");
        require(balanceOf[from] >= amount, "balance");
        authorizationUsed[from][nonce] = true;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
    }

    function authorizationDigest(
        address from,
        address to,
        uint256 amount,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce
    ) public view returns (bytes32) {
        bytes32 domainSeparator = keccak256(
            abi.encode(EIP712_DOMAIN_TYPEHASH, NAME_HASH, VERSION_HASH, block.chainid, address(this))
        );
        bytes32 structHash = keccak256(
            abi.encode(TRANSFER_WITH_AUTHORIZATION_TYPEHASH, from, to, amount, validAfter, validBefore, nonce)
        );
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator, structHash));
    }
}

contract AtomicSponsorVerifier is IAgentBountyVerifier {
    function verify(bytes32, uint64, address, bytes32, bytes32, bytes32, bytes calldata proof)
        external
        pure
        returns (bool passed, bytes32 responseHash)
    {
        responseHash = keccak256(proof);
        return (keccak256(proof) == keccak256("pass"), responseHash);
    }
}

contract AtomicClaimSponsorTest {
    VmAtomicSponsor private constant vm = VmAtomicSponsor(address(uint160(uint256(keccak256("hevm cheat code")))));

    uint256 private constant GRANT_SIGNER_KEY = 0xA11CE;
    uint256 private constant SOLVER_KEY = 0xB0B;
    uint256 private constant OTHER_SOLVER_KEY = 0xCAFE;
    uint256 private constant BOND = 100;
    uint256 private constant MAX_DAILY = 300;

    bytes32 private constant TERMS_HASH = keccak256("sponsored-terms-v1");
    bytes32 private constant POLICY_HASH = keccak256("sponsored-policy-v1");
    bytes32 private constant SUBMISSION_HASH = keccak256("artifact");
    bytes32 private constant EVIDENCE_HASH = keccak256("evidence");

    AuthorizationToken private token;
    AgentBountyFactory private factory;
    AtomicClaimSponsor private sponsor;
    AtomicSponsorVerifier private verifier;
    address private solver;
    address private otherSolver;
    uint256 private creationNonce;

    function setUp() public {
        vm.warp(2 days);
        token = new AuthorizationToken();
        factory = new AgentBountyFactory(address(token));
        verifier = new AtomicSponsorVerifier();
        solver = vm.addr(SOLVER_KEY);
        otherSolver = vm.addr(OTHER_SOLVER_KEY);
        sponsor =
            new AtomicClaimSponsor(address(token), address(factory), vm.addr(GRANT_SIGNER_KEY), BOND, MAX_DAILY, BOND);
        token.mint(address(this), 100_000);
        token.approve(address(factory), type(uint256).max);
        token.mint(address(sponsor), 1_000);
    }

    function testZeroBalanceSolverClaimsAtomically() public {
        AgentBounty bounty = _createBounty();
        AtomicClaimSponsor.Grant memory grant = _grant(bounty, solver, "zero-balance");
        (bytes memory grantSignature, uint8 v, bytes32 r, bytes32 s) = _sign(grant, SOLVER_KEY);

        require(token.balanceOf(solver) == 0, "solver started funded");
        uint256 trackedTotalBefore = _trackedTotal(bounty, solver);
        uint256 sponsorBalanceBefore = token.balanceOf(address(sponsor));
        sponsor.sponsorAndClaim(grant, grantSignature, v, r, s);

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Claimed, "not claimed");
        require(bounty.solver() == solver, "solver mismatch");
        require(bounty.activeClaimBond() == BOND, "bond mismatch");
        require(token.balanceOf(solver) == 0, "bond not consumed");
        require(token.balanceOf(address(sponsor)) == sponsorBalanceBefore - BOND, "sponsor spend mismatch");
        require(sponsor.lifetimeSponsored(solver) == BOND, "lifetime quota missing");
        require(sponsor.sponsoredByDay(sponsor.currentDay()) == BOND, "daily quota missing");
        require(sponsor.grantNonceUsed(grant.grantNonce), "grant nonce unused");
        require(_trackedTotal(bounty, solver) == trackedTotalBefore, "claim token conservation failed");
    }

    function testPassingSettlementReturnsAcquisitionBondToSolver() public {
        AgentBounty bounty = _createBounty();
        AtomicClaimSponsor.Grant memory grant = _grant(bounty, solver, "settle");
        (bytes memory grantSignature, uint8 v, bytes32 r, bytes32 s) = _sign(grant, SOLVER_KEY);
        uint256 trackedTotalBefore = _trackedTotal(bounty, solver);
        sponsor.sponsorAndClaim(grant, grantSignature, v, r, s);

        vm.warp(block.timestamp + 1);
        (uint8 submitV, bytes32 submitR, bytes32 submitS) = vm.sign(
            SOLVER_KEY,
            bounty.submitDigest(solver, bounty.round(), SUBMISSION_HASH, EVIDENCE_HASH, block.timestamp + 1 hours)
        );
        bounty.submitWithSignature(
            SUBMISSION_HASH, EVIDENCE_HASH, block.timestamp + 1 hours, abi.encodePacked(submitR, submitS, submitV)
        );
        bounty.verifyAndSettle(bytes("pass"));

        require(bounty.bountyStatus() == AgentBounty.BountyStatus.Settled, "not settled");
        require(token.balanceOf(solver) == 1_000, "solver did not receive reward and bond");
        require(token.balanceOf(address(sponsor)) == 900, "grant unexpectedly refunded");
        require(_trackedTotal(bounty, solver) == trackedTotalBefore, "settlement token conservation failed");
    }

    function testInvalidGrantSignerRollsBackGrant() public {
        AgentBounty bounty = _createBounty();
        AtomicClaimSponsor.Grant memory grant = _grant(bounty, solver, "forged-grant");
        (uint8 grantV, bytes32 grantR, bytes32 grantS) = vm.sign(OTHER_SOLVER_KEY, sponsor.grantDigest(grant));
        bytes memory forgedGrantSignature = abi.encodePacked(grantR, grantS, grantV);
        (, uint8 v, bytes32 r, bytes32 s) = _sign(grant, SOLVER_KEY);
        uint256 sponsorBalanceBefore = token.balanceOf(address(sponsor));

        (bool success,) = address(sponsor)
            .call(abi.encodeCall(AtomicClaimSponsor.sponsorAndClaim, (grant, forgedGrantSignature, v, r, s)));

        require(!success, "forged grant accepted");
        require(token.balanceOf(address(sponsor)) == sponsorBalanceBefore, "forged grant moved funds");
        require(token.balanceOf(solver) == 0, "solver retained forged grant");
        require(!sponsor.grantNonceUsed(grant.grantNonce), "forged nonce consumed");
    }

    function testExpiredAndOverlongWindowsAreRejected() public {
        AgentBounty bounty = _createBounty();
        AtomicClaimSponsor.Grant memory expired = _grant(bounty, solver, "expired");
        expired.deadline = block.timestamp - 1;
        bytes memory expiredSignature = _grantSignature(expired);
        (uint8 expiredV, bytes32 expiredR, bytes32 expiredS) = vm.sign(SOLVER_KEY, _authorizationDigest(expired));
        (bool expiredSuccess,) = address(sponsor)
            .call(
                abi.encodeCall(
                    AtomicClaimSponsor.sponsorAndClaim, (expired, expiredSignature, expiredV, expiredR, expiredS)
                )
            );
        require(!expiredSuccess, "expired grant accepted");

        AtomicClaimSponsor.Grant memory overlong = _grant(bounty, solver, "overlong");
        overlong.validBefore = block.timestamp + 1 hours + 1;
        bytes memory overlongSignature = _grantSignature(overlong);
        (uint8 overlongV, bytes32 overlongR, bytes32 overlongS) = vm.sign(SOLVER_KEY, _authorizationDigest(overlong));
        (bool overlongSuccess,) = address(sponsor)
            .call(
                abi.encodeCall(
                    AtomicClaimSponsor.sponsorAndClaim, (overlong, overlongSignature, overlongV, overlongR, overlongS)
                )
            );
        require(!overlongSuccess, "overlong authorization accepted");
        require(token.balanceOf(solver) == 0, "invalid windows moved grant");
        require(sponsor.lifetimeSponsored(solver) == 0, "invalid windows consumed quota");
    }

    function testNoncanonicalBountyIsRejected() public {
        AgentBountyFactory externalFactory = new AgentBountyFactory(address(token));
        token.approve(address(externalFactory), type(uint256).max);
        AgentBounty externalBounty = _createBountyWithFactory(externalFactory);
        AtomicClaimSponsor.Grant memory grant = _grant(externalBounty, solver, "external-factory");
        (bytes memory grantSignature, uint8 v, bytes32 r, bytes32 s) = _sign(grant, SOLVER_KEY);
        uint256 sponsorBalanceBefore = token.balanceOf(address(sponsor));

        (bool success,) =
            address(sponsor).call(abi.encodeCall(AtomicClaimSponsor.sponsorAndClaim, (grant, grantSignature, v, r, s)));

        require(!success, "noncanonical bounty accepted");
        require(token.balanceOf(address(sponsor)) == sponsorBalanceBefore, "noncanonical grant moved funds");
        require(!sponsor.grantNonceUsed(grant.grantNonce), "noncanonical nonce consumed");
    }

    function testFactoryTokenMismatchIsRejectedAtDeployment() public {
        AuthorizationToken otherToken = new AuthorizationToken();
        bool deployed;
        try new AtomicClaimSponsor(
            address(otherToken), address(factory), vm.addr(GRANT_SIGNER_KEY), BOND, MAX_DAILY, BOND
        ) returns (
            AtomicClaimSponsor
        ) {
            deployed = true;
        } catch {}
        require(!deployed, "mismatched factory token accepted");
    }

    function testOwnershipTransferRequiresPendingOwnerAcceptance() public {
        address newOwner = vm.addr(0x0A11CE);
        sponsor.transferOwnership(newOwner);
        require(sponsor.owner() == address(this), "ownership changed before acceptance");
        require(sponsor.pendingOwner() == newOwner, "pending owner missing");

        (bool oldOwnerAccepts,) = address(sponsor).call(abi.encodeCall(AtomicClaimSponsor.acceptOwnership, ()));
        require(!oldOwnerAccepts, "non-pending owner accepted ownership");

        vm.prank(newOwner);
        sponsor.acceptOwnership();
        require(sponsor.owner() == newOwner, "ownership acceptance failed");
        require(sponsor.pendingOwner() == address(0), "pending owner not cleared");

        (bool previousOwnerCanPause,) = address(sponsor).call(abi.encodeCall(AtomicClaimSponsor.setPaused, (true)));
        require(!previousOwnerCanPause, "previous owner retained control");
        vm.prank(newOwner);
        sponsor.setPaused(true);
        require(sponsor.paused(), "new owner cannot control sponsor");
    }

    function testLostRaceRollsBackGrantAndQuota() public {
        AgentBounty bounty = _createBounty();
        token.mint(otherSolver, BOND);
        AtomicClaimSponsor.Grant memory otherGrant = _grant(bounty, otherSolver, "race-winner");
        (, uint8 otherV, bytes32 otherR, bytes32 otherS) = _sign(otherGrant, OTHER_SOLVER_KEY);
        bounty.claimWithAuthorization(
            otherSolver,
            otherGrant.validAfter,
            otherGrant.validBefore,
            otherGrant.authorizationNonce,
            otherV,
            otherR,
            otherS
        );

        AtomicClaimSponsor.Grant memory grant = _grantForRound(bounty, solver, "race-loser", 1);
        (bytes memory grantSignature, uint8 v, bytes32 r, bytes32 s) = _sign(grant, SOLVER_KEY);
        uint256 sponsorBalanceBefore = token.balanceOf(address(sponsor));
        (bool success,) =
            address(sponsor).call(abi.encodeCall(AtomicClaimSponsor.sponsorAndClaim, (grant, grantSignature, v, r, s)));

        require(!success, "raced grant succeeded");
        require(token.balanceOf(address(sponsor)) == sponsorBalanceBefore, "raced grant moved funds");
        require(token.balanceOf(solver) == 0, "solver retained raced grant");
        require(!sponsor.grantNonceUsed(grant.grantNonce), "raced nonce consumed");
        require(sponsor.lifetimeSponsored(solver) == 0, "raced lifetime quota consumed");
        require(sponsor.sponsoredByDay(sponsor.currentDay()) == 0, "raced daily quota consumed");
    }

    function testInvalidSolverAuthorizationRollsBackGrant() public {
        AgentBounty bounty = _createBounty();
        AtomicClaimSponsor.Grant memory grant = _grant(bounty, solver, "bad-auth");
        bytes memory grantSignature = _grantSignature(grant);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(OTHER_SOLVER_KEY, _authorizationDigest(grant));
        uint256 sponsorBalanceBefore = token.balanceOf(address(sponsor));
        (bool success,) =
            address(sponsor).call(abi.encodeCall(AtomicClaimSponsor.sponsorAndClaim, (grant, grantSignature, v, r, s)));

        require(!success, "bad solver authorization accepted");
        require(token.balanceOf(address(sponsor)) == sponsorBalanceBefore, "bad authorization moved grant");
        require(token.balanceOf(solver) == 0, "solver retained failed grant");
        require(!sponsor.grantNonceUsed(grant.grantNonce), "failed nonce consumed");
    }

    function testGrantReplayAndLifetimeCapAreRejected() public {
        AgentBounty first = _createBounty();
        AtomicClaimSponsor.Grant memory grant = _grant(first, solver, "first");
        (bytes memory grantSignature, uint8 v, bytes32 r, bytes32 s) = _sign(grant, SOLVER_KEY);
        sponsor.sponsorAndClaim(grant, grantSignature, v, r, s);

        (bool replaySuccess,) =
            address(sponsor).call(abi.encodeCall(AtomicClaimSponsor.sponsorAndClaim, (grant, grantSignature, v, r, s)));
        require(!replaySuccess, "grant replay accepted");

        AgentBounty second = _createBounty();
        AtomicClaimSponsor.Grant memory secondGrant = _grant(second, solver, "second");
        (bytes memory secondSignature, uint8 v2, bytes32 r2, bytes32 s2) = _sign(secondGrant, SOLVER_KEY);
        (bool secondSuccess,) = address(sponsor)
            .call(abi.encodeCall(AtomicClaimSponsor.sponsorAndClaim, (secondGrant, secondSignature, v2, r2, s2)));
        require(!secondSuccess, "second lifetime grant accepted");
    }

    function testDailyCapRejectsFourthDistinctSolverWithoutMovingFunds() public {
        for (uint256 i = 0; i < 3; i++) {
            uint256 key = 10_000 + i;
            address candidate = vm.addr(key);
            AgentBounty bounty = _createBounty();
            AtomicClaimSponsor.Grant memory grant = _grant(bounty, candidate, bytes32(i + 1));
            (bytes memory grantSignature, uint8 v, bytes32 r, bytes32 s) = _sign(grant, key);
            sponsor.sponsorAndClaim(grant, grantSignature, v, r, s);
        }

        uint256 fourthKey = 20_000;
        address fourth = vm.addr(fourthKey);
        AgentBounty fourthBounty = _createBounty();
        AtomicClaimSponsor.Grant memory fourthGrant = _grant(fourthBounty, fourth, "fourth");
        (bytes memory fourthSignature, uint8 v4, bytes32 r4, bytes32 s4) = _sign(fourthGrant, fourthKey);
        uint256 balanceBefore = token.balanceOf(address(sponsor));
        (bool success,) = address(sponsor)
            .call(abi.encodeCall(AtomicClaimSponsor.sponsorAndClaim, (fourthGrant, fourthSignature, v4, r4, s4)));
        require(!success, "daily cap bypassed");
        require(token.balanceOf(address(sponsor)) == balanceBefore, "daily cap moved funds");
    }

    function testPauseAndOwnerWithdrawal() public {
        sponsor.setPaused(true);
        uint256 ownerBalanceBefore = token.balanceOf(address(this));
        sponsor.withdraw(address(this), BOND);
        require(token.balanceOf(address(this)) == ownerBalanceBefore + BOND, "withdrawal missing");

        AgentBounty bounty = _createBounty();
        AtomicClaimSponsor.Grant memory grant = _grant(bounty, solver, "paused");
        (bytes memory grantSignature, uint8 v, bytes32 r, bytes32 s) = _sign(grant, SOLVER_KEY);
        (bool success,) =
            address(sponsor).call(abi.encodeCall(AtomicClaimSponsor.sponsorAndClaim, (grant, grantSignature, v, r, s)));
        require(!success, "paused sponsor claimed");
    }

    function _createBounty() private returns (AgentBounty bounty) {
        return _createBountyWithFactory(factory);
    }

    function _createBountyWithFactory(AgentBountyFactory targetFactory) private returns (AgentBounty bounty) {
        creationNonce += 1;
        AgentBountyFactory.CreateBountyParams memory params = AgentBountyFactory.CreateBountyParams({
            solverReward: 900,
            verifierReward: BOND,
            termsHash: TERMS_HASH,
            policyHash: POLICY_HASH,
            acceptanceCriteriaHash: keccak256("criteria"),
            benchmarkHash: keccak256("benchmark"),
            evidenceSchemaHash: keccak256("evidence-schema"),
            fundingDeadline: uint64(block.timestamp + 1 days),
            claimWindowSeconds: 1 hours,
            verificationWindowSeconds: 1 hours,
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: address(verifier),
            verifierRewardRecipient: address(0xBEEF),
            threshold: 1
        });
        (address bountyAddress,) = targetFactory.createBounty(params, new address[](0), 1_000, bytes32(creationNonce));
        return AgentBounty(bountyAddress);
    }

    function _trackedTotal(AgentBounty bounty, address candidate) private view returns (uint256) {
        return token.balanceOf(address(this)) + token.balanceOf(address(sponsor)) + token.balanceOf(address(bounty))
            + token.balanceOf(candidate) + token.balanceOf(address(0xBEEF));
    }

    function _grant(AgentBounty bounty, address candidate, bytes32 salt)
        private
        view
        returns (AtomicClaimSponsor.Grant memory)
    {
        return _grantForRound(bounty, candidate, salt, bounty.round() + 1);
    }

    function _grantForRound(AgentBounty bounty, address candidate, bytes32 salt, uint64 round)
        private
        view
        returns (AtomicClaimSponsor.Grant memory)
    {
        return AtomicClaimSponsor.Grant({
            bounty: address(bounty),
            solver: candidate,
            round: round,
            bond: BOND,
            termsHash: bounty.termsHash(),
            policyHash: bounty.policyHash(),
            authorizationNonce: keccak256(abi.encode("authorization", salt, candidate)),
            validAfter: block.timestamp - 1,
            validBefore: block.timestamp + 1 hours,
            grantNonce: keccak256(abi.encode("grant", salt, candidate)),
            deadline: block.timestamp + 30 minutes
        });
    }

    function _sign(AtomicClaimSponsor.Grant memory grant, uint256 solverKey)
        private
        returns (bytes memory grantSignature, uint8 v, bytes32 r, bytes32 s)
    {
        grantSignature = _grantSignature(grant);
        return _withAuthorizationSignature(grant, solverKey, grantSignature);
    }

    function _grantSignature(AtomicClaimSponsor.Grant memory grant) private returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(GRANT_SIGNER_KEY, sponsor.grantDigest(grant));
        return abi.encodePacked(r, s, v);
    }

    function _withAuthorizationSignature(
        AtomicClaimSponsor.Grant memory grant,
        uint256 solverKey,
        bytes memory grantSignature
    ) private returns (bytes memory, uint8, bytes32, bytes32) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(solverKey, _authorizationDigest(grant));
        return (grantSignature, v, r, s);
    }

    function _authorizationDigest(AtomicClaimSponsor.Grant memory grant) private view returns (bytes32) {
        return token.authorizationDigest(
            grant.solver, grant.bounty, grant.bond, grant.validAfter, grant.validBefore, grant.authorizationNonce
        );
    }
}
