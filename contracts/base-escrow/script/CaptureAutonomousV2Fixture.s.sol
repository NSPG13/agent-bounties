// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AgentBountyFactoryV2.sol";
import "../src/ParticipantEligibilityRegistry.sol";

interface CaptureVm {
    function startBroadcast(uint256 privateKey) external;
    function stopBroadcast() external;
    function addr(uint256 privateKey) external returns (address);
    function envAddress(string calldata name) external returns (address);
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
}

/// @dev Local-only token with a recipient blocklist so the fixture can exercise fee deferral, and
/// Circle-style EIP-3009 `receiveWithAuthorization` under the Base Sepolia USDC domain ("USDC",
/// "2"). The capture tool copies its code to Base Sepolia's USDC address, so the domain separator
/// is derived from `address(this)` at call time rather than cached at construction.
contract CaptureToken {
    bytes32 private constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant RECEIVE_WITH_AUTHORIZATION_TYPEHASH = keccak256(
        "ReceiveWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)"
    );

    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    mapping(address => bool) public blocked;
    mapping(address => mapping(bytes32 => bool)) public authorizationState;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function DOMAIN_SEPARATOR() public view returns (bytes32) {
        return keccak256(abi.encode(DOMAIN_TYPEHASH, keccak256("USDC"), keccak256("2"), block.chainid, address(this)));
    }

    function receiveWithAuthorization(
        address from,
        address to,
        uint256 value,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external {
        require(to == msg.sender, "caller must be the payee");
        require(block.timestamp > validAfter && block.timestamp < validBefore, "authorization not valid");
        require(!authorizationState[from][nonce], "authorization used");
        bytes32 structHash =
            keccak256(abi.encode(RECEIVE_WITH_AUTHORIZATION_TYPEHASH, from, to, value, validAfter, validBefore, nonce));
        address signer = ecrecover(keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR(), structHash)), v, r, s);
        require(signer != address(0) && signer == from, "invalid authorization signature");
        require(!blocked[to], "blocked recipient");
        authorizationState[from][nonce] = true;
        balanceOf[from] -= value;
        balanceOf[to] += value;
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
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(!blocked[to], "blocked recipient");
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }
}

contract CaptureVerdictModule is IAgentBountyVerifier {
    function verify(bytes32, uint64, address, bytes32, bytes32, bytes32, bytes calldata proof)
        external
        pure
        returns (bool passed, bytes32 responseHash)
    {
        passed = proof.length > 0 && proof[0] == 0x01;
        responseHash = keccak256(proof);
    }
}

/// @notice Emits every autonomous-v2 event from the compiled contracts on a local Anvil chain so
/// `crates/chain-base` can test its decoder, feed, and planner against real ABI-encoded data.
/// `tools/capture_autonomous_v2_fixture.py` deploys the token, module, registry, and factory with
/// `forge create` and passes their addresses in; deploying them here would embed the factory and
/// bounty creation code and push this script past the EIP-170 size gate. Regenerate
/// `crates/chain-base/tests/fixtures/autonomous-v2-loop.json` with
/// `python tools/capture_autonomous_v2_fixture.py` (needs anvil, forge, and cast on PATH).
contract CaptureAutonomousV2Fixture {
    CaptureVm private constant vm = CaptureVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    // Anvil's default development keys. Never use them outside a local chain.
    uint256 private constant CREATOR_KEY = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    uint256 private constant SOLVER_KEY = 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;
    uint256 private constant SECOND_SOLVER_KEY = 0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a;
    uint256 private constant ATTESTER_KEY = 0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6;
    uint256 private constant VERIFIER_A_KEY = 0x4bbbf85ce3377467afe5d46f804f221813b2bb87f24d81f60f1fcdbf7cbf4356;
    uint256 private constant VERIFIER_B_KEY = 0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97;
    address private constant FEE_RECIPIENT = 0xfEE0000000000000000000000000000000000fee;
    address private constant VERIFIER_RECIPIENT = 0x7000000000000000000000000000000000000007;
    bytes32 private constant CONTRACTOR_SOURCE = keccak256("agent-bounties/invoice-contractor-v1");
    uint256 private constant SOLVER_REWARD = 1_000_000;
    uint256 private constant VERIFIER_REWARD = 100_000;
    uint256 private constant TARGET = 1_175_000;

    CaptureToken private token;
    CaptureVerdictModule private module;
    AgentBountyFactoryV2 private factory;
    ParticipantEligibilityRegistry private registry;

    function run() external {
        address creator = vm.addr(CREATOR_KEY);
        address solver = vm.addr(SOLVER_KEY);
        address secondSolver = vm.addr(SECOND_SOLVER_KEY);

        token = CaptureToken(vm.envAddress("CAPTURE_TOKEN"));
        module = CaptureVerdictModule(vm.envAddress("CAPTURE_MODULE"));
        registry = ParticipantEligibilityRegistry(vm.envAddress("CAPTURE_REGISTRY"));
        factory = AgentBountyFactoryV2(vm.envAddress("CAPTURE_FACTORY"));
        require(factory.platformFeeBps() == 750 && factory.platformFeeRecipient() == FEE_RECIPIENT, "factory fee");
        require(registry.attester() == vm.addr(ATTESTER_KEY), "registry attester");

        vm.startBroadcast(CREATOR_KEY);
        token.mint(creator, 10 * TARGET);
        token.mint(solver, 10 * VERIFIER_REWARD);
        token.mint(secondSolver, 10 * VERIFIER_REWARD);
        token.approve(address(factory), type(uint256).max);
        // 1. Signed quorum: paid at payout after one rejected round. A module verdict can only pass.
        AgentBountyV2 paid = _createQuorum(1);
        // 2. Fee deferred by a blocked recipient, then forwarded.
        AgentBountyV2 deferred = _create(2, TARGET, address(0), bytes32(0));
        // 3. Contractor-gated bounty.
        AgentBountyV2 gated = _create(3, TARGET, address(registry), CONTRACTOR_SOURCE);
        // 4. Partially funded, then cancelled by its creator and refunded.
        AgentBountyV2 cancelled = _create(4, 500_000, address(0), bytes32(0));
        // 6. Module submission nobody proves; the driver expires it after the window (nonce 5 is
        //    the gasless loop).
        AgentBountyV2 unproven = _create(6, TARGET, address(0), bytes32(0));
        vm.stopBroadcast();

        _claimAndSubmit(SECOND_SOLVER_KEY, paid);
        AgentBountyV2.Attestation[] memory rejection = _verdict(paid, false);
        vm.startBroadcast(CREATOR_KEY);
        paid.settleWithAttestations(rejection);
        vm.stopBroadcast();
        _claimAndSubmit(SOLVER_KEY, paid);
        AgentBountyV2.Attestation[] memory acceptance = _verdict(paid, true);
        vm.startBroadcast(CREATOR_KEY);
        paid.settleWithAttestations(acceptance);

        token.setBlocked(FEE_RECIPIENT, true);
        vm.stopBroadcast();
        _claimAndSubmit(SECOND_SOLVER_KEY, deferred);
        vm.startBroadcast(CREATOR_KEY);
        deferred.verifyAndSettle(hex"01");
        token.setBlocked(FEE_RECIPIENT, false);
        deferred.withdrawPlatformFee();

        _attest(solver);
        vm.stopBroadcast();
        vm.startBroadcast(SOLVER_KEY);
        token.approve(address(gated), VERIFIER_REWARD);
        gated.claim();
        vm.stopBroadcast();

        vm.startBroadcast(CREATOR_KEY);
        cancelled.cancel();
        cancelled.withdrawRefund();
        vm.stopBroadcast();

        _claimAndSubmit(SOLVER_KEY, unproven);
    }

    function _create(uint256 nonce, uint256 initialFunding, address eligibilityRegistry, bytes32 eligibilitySource)
        private
        returns (AgentBountyV2)
    {
        AgentBountyFactoryV2.CreateBountyParams memory params = _params(nonce, eligibilityRegistry, eligibilitySource);
        (address bountyAddress,) = factory.createBounty(params, new address[](0), initialFunding, bytes32(nonce));
        return AgentBountyV2(bountyAddress);
    }

    function _params(uint256 nonce, address eligibilityRegistry, bytes32 eligibilitySource)
        private
        view
        returns (AgentBountyFactoryV2.CreateBountyParams memory)
    {
        return AgentBountyFactoryV2.CreateBountyParams({
            solverReward: SOLVER_REWARD,
            verifierReward: VERIFIER_REWARD,
            termsHash: keccak256(abi.encode("terms", nonce)),
            policyHash: keccak256(abi.encode("policy", nonce)),
            acceptanceCriteriaHash: keccak256(abi.encode("criteria", nonce)),
            benchmarkHash: keccak256(abi.encode("benchmark", nonce)),
            evidenceSchemaHash: keccak256(abi.encode("evidence-schema", nonce)),
            fundingDeadline: uint64(block.timestamp + 7 days),
            claimWindowSeconds: 1 days,
            verificationWindowSeconds: 1 days,
            verificationMode: AgentBountyV2.VerificationMode.DeterministicModule,
            verifierModule: address(module),
            verifierRewardRecipient: VERIFIER_RECIPIENT,
            threshold: 1,
            claimEligibilityRegistry: eligibilityRegistry,
            claimEligibilitySource: eligibilitySource
        });
    }

    function _createQuorum(uint256 nonce) private returns (AgentBountyV2) {
        AgentBountyFactoryV2.CreateBountyParams memory params = _params(nonce, address(0), bytes32(0));
        params.verificationMode = AgentBountyV2.VerificationMode.SignedQuorum;
        params.verifierModule = address(0);
        params.verifierRewardRecipient = address(0);
        params.threshold = 2;
        address[] memory verifiers = new address[](2);
        verifiers[0] = vm.addr(VERIFIER_A_KEY);
        verifiers[1] = vm.addr(VERIFIER_B_KEY);
        (address bountyAddress,) = factory.createBounty(params, verifiers, TARGET, bytes32(nonce));
        return AgentBountyV2(bountyAddress);
    }

    function _verdict(AgentBountyV2 bounty, bool passed)
        private
        returns (AgentBountyV2.Attestation[] memory attestations)
    {
        attestations = new AgentBountyV2.Attestation[](2);
        uint256[2] memory keys = [VERIFIER_A_KEY, VERIFIER_B_KEY];
        for (uint256 i = 0; i < 2; i++) {
            address verifier = vm.addr(keys[i]);
            bytes32 responseHash = keccak256(abi.encode("verdict", bounty.round(), passed));
            uint256 deadline = block.timestamp + 1 days;
            (uint8 v, bytes32 r, bytes32 s) =
                vm.sign(keys[i], bounty.attestationDigest(verifier, passed, responseHash, deadline));
            attestations[i] = AgentBountyV2.Attestation({
                verifier: verifier,
                passed: passed,
                responseHash: responseHash,
                deadline: deadline,
                signature: abi.encodePacked(r, s, v)
            });
        }
    }

    function _claimAndSubmit(uint256 solverKey, AgentBountyV2 bounty) private {
        vm.startBroadcast(solverKey);
        token.approve(address(bounty), VERIFIER_REWARD);
        bounty.claim();
        bounty.submit(keccak256(abi.encode("artifact", bounty.round())), keccak256("evidence"));
        vm.stopBroadcast();
    }

    function _attest(address wallet) private {
        uint64 validUntil = uint64(block.timestamp + 30 days);
        bytes32 participantId = keccak256(abi.encode(wallet));
        bytes32 digest = registry.attestationDigest(wallet, participantId, CONTRACTOR_SOURCE, validUntil, 0);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ATTESTER_KEY, digest);
        registry.register(wallet, participantId, CONTRACTOR_SOURCE, validUntil, abi.encodePacked(r, s, v));
    }
}
