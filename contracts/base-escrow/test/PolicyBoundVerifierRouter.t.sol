// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/PolicyBoundVerifierRouter.sol";

interface RouterVm {
    function warp(uint256 timestamp) external;
}

contract RouterFactoryMock {
    mapping(address => bool) public isCanonicalBounty;

    function setCanonical(address bounty, bool canonical) external {
        isCanonicalBounty[bounty] = canonical;
    }
}

contract RoutedVerifierMock is IRoutedAgentBountyVerifier {
    address public immutable verifierRouter;
    bytes32 public immutable committedPolicyHash;
    address public immutable canonicalFactory;
    bool public immutable verdict;

    constructor(address router_, bytes32 policyHash_, address factory_, bool verdict_) {
        verifierRouter = router_;
        committedPolicyHash = policyHash_;
        canonicalFactory = factory_;
        verdict = verdict_;
    }

    function verifyRouted(
        address parentBounty,
        bytes32 bountyId,
        uint64 round,
        address solver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 policyHash,
        bytes calldata proof
    ) external view returns (bool passed, bytes32 responseHash) {
        require(msg.sender == verifierRouter, "not router");
        require(policyHash == committedPolicyHash, "wrong policy");
        responseHash = keccak256(
            abi.encode(
                parentBounty,
                bountyId,
                round,
                solver,
                submissionHash,
                evidenceHash,
                policyHash,
                keccak256(proof)
            )
        );
        return (verdict, responseHash);
    }
}

contract GuardianActor {
    function veto(PolicyBoundVerifierRouter router, bytes32 policyHash) external {
        router.vetoPolicy(policyHash);
    }
}

contract CanonicalRouterCaller {
    function verify(
        PolicyBoundVerifierRouter router,
        bytes32 bountyId,
        uint64 round,
        address solver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 policyHash,
        bytes calldata proof
    ) external view returns (bool passed, bytes32 responseHash) {
        return router.verify(bountyId, round, solver, submissionHash, evidenceHash, policyHash, proof);
    }
}

contract PolicyBoundVerifierRouterTest {
    RouterVm private constant vm = RouterVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint64 private constant DELAY = 7 days;

    RouterFactoryMock private factory;
    GuardianActor private guardian;
    PolicyBoundVerifierRouter private router;
    CanonicalRouterCaller private parent;

    function setUp() public {
        factory = new RouterFactoryMock();
        guardian = new GuardianActor();
        router = new PolicyBoundVerifierRouter(address(factory), address(this), address(guardian), DELAY);
        parent = new CanonicalRouterCaller();
        factory.setCanonical(address(parent), true);
    }

    function testBootstrapRoutesOneImmutablePolicy() public {
        bytes32 policyHash = keccak256("bootstrap-policy");
        RoutedVerifierMock verifier = new RoutedVerifierMock(address(router), policyHash, address(factory), true);
        router.bootstrapPolicy(policyHash, address(verifier));

        require(router.isPolicyActive(policyHash), "bootstrap inactive");
        (bool passed, bytes32 responseHash) = parent.verify(
            router,
            keccak256("bounty"),
            3,
            address(0xBEEF),
            keccak256("submission"),
            keccak256("evidence"),
            policyHash,
            hex"1234"
        );
        require(passed, "verdict changed");
        require(responseHash != bytes32(0), "response missing");

        (bool duplicateOk,) = address(router).call(
            abi.encodeCall(PolicyBoundVerifierRouter.bootstrapPolicy, (policyHash, address(verifier)))
        );
        require(!duplicateOk, "active policy replaced");

        bytes32 secondPolicy = keccak256("second-bootstrap");
        RoutedVerifierMock secondVerifier =
            new RoutedVerifierMock(address(router), secondPolicy, address(factory), true);
        (bool secondOk,) = address(router).call(
            abi.encodeCall(PolicyBoundVerifierRouter.bootstrapPolicy, (secondPolicy, address(secondVerifier)))
        );
        require(!secondOk, "bootstrap reused");
    }

    function testProposedPolicyCannotActivateBeforeDelay() public {
        bytes32 policyHash = keccak256("delayed-policy");
        RoutedVerifierMock verifier = new RoutedVerifierMock(address(router), policyHash, address(factory), true);
        router.proposePolicy(policyHash, address(verifier));

        (bool earlyOk,) = address(router).call(
            abi.encodeCall(PolicyBoundVerifierRouter.activatePolicy, (policyHash))
        );
        require(!earlyOk, "delay bypassed");
        require(!router.isPolicyActive(policyHash), "policy active early");

        vm.warp(block.timestamp + DELAY);
        router.activatePolicy(policyHash);
        require(router.isPolicyActive(policyHash), "policy not active after delay");
    }

    function testGuardianCanVetoPendingPolicyButCannotRewriteIt() public {
        bytes32 policyHash = keccak256("veto-policy");
        RoutedVerifierMock verifier = new RoutedVerifierMock(address(router), policyHash, address(factory), true);
        router.proposePolicy(policyHash, address(verifier));
        guardian.veto(router, policyHash);

        vm.warp(block.timestamp + DELAY);
        (bool activateOk,) = address(router).call(
            abi.encodeCall(PolicyBoundVerifierRouter.activatePolicy, (policyHash))
        );
        require(!activateOk, "veto ignored");

        RoutedVerifierMock replacement = new RoutedVerifierMock(address(router), policyHash, address(factory), false);
        (bool replaceOk,) = address(router).call(
            abi.encodeCall(PolicyBoundVerifierRouter.proposePolicy, (policyHash, address(replacement)))
        );
        require(!replaceOk, "vetoed identity reused");
    }

    function testRejectsNonCanonicalCaller() public {
        bytes32 policyHash = keccak256("canonical-policy");
        RoutedVerifierMock verifier = new RoutedVerifierMock(address(router), policyHash, address(factory), true);
        router.bootstrapPolicy(policyHash, address(verifier));

        (bool ok,) = address(router).staticcall(
            abi.encodeCall(
                PolicyBoundVerifierRouter.verify,
                (
                    keccak256("bounty"),
                    1,
                    address(0xBEEF),
                    keccak256("submission"),
                    keccak256("evidence"),
                    policyHash,
                    hex""
                )
            )
        );
        require(!ok, "non-canonical caller routed");
    }

    function testRejectsMismatchedVerifierMetadata() public {
        bytes32 policyHash = keccak256("metadata-policy");
        RoutedVerifierMock wrongRouter = new RoutedVerifierMock(address(0x1234), policyHash, address(factory), true);
        (bool wrongRouterOk,) = address(router).call(
            abi.encodeCall(PolicyBoundVerifierRouter.proposePolicy, (policyHash, address(wrongRouter)))
        );
        require(!wrongRouterOk, "wrong router accepted");

        RoutedVerifierMock wrongPolicy =
            new RoutedVerifierMock(address(router), keccak256("other-policy"), address(factory), true);
        (bool wrongPolicyOk,) = address(router).call(
            abi.encodeCall(PolicyBoundVerifierRouter.proposePolicy, (policyHash, address(wrongPolicy)))
        );
        require(!wrongPolicyOk, "wrong policy accepted");

        RouterFactoryMock otherFactory = new RouterFactoryMock();
        RoutedVerifierMock wrongFactory =
            new RoutedVerifierMock(address(router), policyHash, address(otherFactory), true);
        (bool wrongFactoryOk,) = address(router).call(
            abi.encodeCall(PolicyBoundVerifierRouter.proposePolicy, (policyHash, address(wrongFactory)))
        );
        require(!wrongFactoryOk, "wrong factory accepted");
    }
}
