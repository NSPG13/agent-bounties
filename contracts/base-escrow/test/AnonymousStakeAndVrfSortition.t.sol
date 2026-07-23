// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/AnonymousStakePoolV1.sol";
import "../src/VrfSortitionCoordinatorV1.sol";

interface AnonymousPoolVm {
    function warp(uint256 timestamp) external;
}

contract AnonymousPoolToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

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
}

contract AnonymousPoolActor {
    function approve(AnonymousPoolToken token, address spender, uint256 amount) external {
        token.approve(spender, amount);
    }

    function register(AnonymousStakePoolV1 pool, AnonymousStakePoolV1.Role role) external {
        pool.register(role);
    }

    function activate(AnonymousStakePoolV1 pool, AnonymousStakePoolV1.Role role) external {
        pool.activate(role);
    }

    function availability(AnonymousStakePoolV1 pool, AnonymousStakePoolV1.Role role, bool available) external {
        pool.setAvailability(role, available);
    }

    function beginUnbond(AnonymousStakePoolV1 pool, AnonymousStakePoolV1.Role role) external {
        pool.beginUnbond(role);
    }

    function withdraw(AnonymousStakePoolV1 pool, AnonymousStakePoolV1.Role role) external {
        pool.withdraw(role);
    }

    function restore(AnonymousStakePoolV1 pool, AnonymousStakePoolV1.Role role) external {
        pool.restore(role);
    }
}

contract MockVrfCoordinatorV2Plus {
    uint256 public nextRequestId = 1;
    mapping(uint256 => address) public consumers;
    VrfV2PlusClientV1.RandomWordsRequest public lastRequest;

    function requestRandomWords(VrfV2PlusClientV1.RandomWordsRequest calldata requestParams)
        external
        returns (uint256 requestId)
    {
        lastRequest = requestParams;
        requestId = nextRequestId;
        nextRequestId += 1;
        consumers[requestId] = msg.sender;
    }

    function lastRequestDetails() external view returns (VrfV2PlusClientV1.RandomWordsRequest memory) {
        return lastRequest;
    }

    function fulfill(uint256 requestId, uint256 randomWord) external {
        uint256[] memory words = new uint256[](1);
        words[0] = randomWord;
        VrfSortitionCoordinatorV1(consumers[requestId]).rawFulfillRandomWords(requestId, words);
    }

    function fulfillUnknown(VrfSortitionCoordinatorV1 consumer, uint256 requestId, uint256 randomWord) external {
        uint256[] memory words = new uint256[](1);
        words[0] = randomWord;
        consumer.rawFulfillRandomWords(requestId, words);
    }
}

contract AnonymousStakeAndVrfSortitionTest {
    AnonymousPoolVm private constant vm =
        AnonymousPoolVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    bytes32 private constant KEY_HASH = keccak256("base-vrf-key-hash");

    AnonymousPoolToken private token;
    AnonymousStakePoolV1 private pool;
    AnonymousPoolActor private actor;
    MockVrfCoordinatorV2Plus private vrf;
    VrfSortitionCoordinatorV1 private sortition;

    function setUp() public {
        token = new AnonymousPoolToken();
        pool = new AnonymousStakePoolV1(address(token), address(this));
        actor = new AnonymousPoolActor();
        token.mint(address(actor), 20_000_000);
        actor.approve(token, address(pool), type(uint256).max);
        vrf = new MockVrfCoordinatorV2Plus();
        sortition = new VrfSortitionCoordinatorV1(address(vrf), address(this), 123, KEY_HASH);
    }

    function testFixedTicketActivatesOnlyAfterSevenDaysAndRolesAreSeparate() public {
        actor.register(pool, AnonymousStakePoolV1.Role.Verifier);
        (bool early,) = address(actor).call(
            abi.encodeCall(AnonymousPoolActor.activate, (pool, AnonymousStakePoolV1.Role.Verifier))
        );
        require(!early, "ticket activated early");

        vm.warp(block.timestamp + 7 days);
        actor.activate(pool, AnonymousStakePoolV1.Role.Verifier);
        actor.register(pool, AnonymousStakePoolV1.Role.Solver);

        (uint128 verifierStake,, uint64 activationAt,, bool active, bool available) =
            pool.tickets(address(actor), AnonymousStakePoolV1.Role.Verifier);
        (uint128 solverStake,,,,,) = pool.tickets(address(actor), AnonymousStakePoolV1.Role.Solver);
        require(verifierStake == 5_000_000 && solverStake == 5_000_000, "fixed role stake mismatch");
        require(activationAt <= block.timestamp && active && available, "verifier ticket inactive");
        require(pool.activeWalletCount(AnonymousStakePoolV1.Role.Verifier) == 1, "active count mismatch");
    }

    function testLockSlashRestoreAndUnbondAreConservative() public {
        actor.register(pool, AnonymousStakePoolV1.Role.Verifier);
        vm.warp(block.timestamp + 7 days);
        actor.activate(pool, AnonymousStakePoolV1.Role.Verifier);

        bytes32 caseId = keccak256("case-one");
        pool.lock(caseId, address(actor), AnonymousStakePoolV1.Role.Verifier, 100_000);
        pool.slash(caseId, address(actor), AnonymousStakePoolV1.Role.Verifier, 100_000, address(pool));
        (uint128 stake, uint128 locked,,, bool active, bool available) =
            pool.tickets(address(actor), AnonymousStakePoolV1.Role.Verifier);
        require(stake == 4_900_000 && locked == 0, "slash accounting mismatch");
        require(!active && !available && pool.reserveSlashed() == 100_000, "slashed ticket still active");

        actor.restore(pool, AnonymousStakePoolV1.Role.Verifier);
        vm.warp(block.timestamp + 7 days);
        actor.activate(pool, AnonymousStakePoolV1.Role.Verifier);
        actor.beginUnbond(pool, AnonymousStakePoolV1.Role.Verifier);
        (bool early,) = address(actor).call(
            abi.encodeCall(AnonymousPoolActor.withdraw, (pool, AnonymousStakePoolV1.Role.Verifier))
        );
        require(!early, "stake withdrew early");
        vm.warp(block.timestamp + 7 days);
        actor.withdraw(pool, AnonymousStakePoolV1.Role.Verifier);
        require(token.balanceOf(address(actor)) == 19_900_000, "withdrawal balance mismatch");
    }

    function testFrozenRequestsUseNativePaymentThreeConfirmationsAndNoRerolls() public {
        address[] memory candidates = _candidates(8);
        bytes32 commitment = keccak256("primary-case");
        uint256 requestId = sortition.freezeAndRequest(commitment, candidates, 4);

        VrfV2PlusClientV1.RandomWordsRequest memory request = vrf.lastRequestDetails();

        require(requestId == 1, "request id mismatch");
        require(request.subId == 123, "subscription mismatch");
        require(request.requestConfirmations == 3, "confirmation mismatch");
        require(request.numWords == 1, "word count mismatch");
        require(
            keccak256(request.extraArgs)
                == keccak256(abi.encodeWithSelector(bytes4(keccak256("VRF ExtraArgsV1")), true)),
            "native payment mismatch"
        );

        (bool reroll,) = address(sortition).call(
            abi.encodeCall(VrfSortitionCoordinatorV1.freezeAndRequest, (commitment, candidates, 4))
        );
        require(!reroll, "commitment rerolled");
    }

    function testOutOfOrderCallbacksRemainBoundAndDeriveUniqueRankings() public {
        address[] memory first = _candidates(8);
        address[] memory second = _candidates(10);
        uint256 firstRequest = sortition.freezeAndRequest(keccak256("first"), first, 5);
        uint256 secondRequest = sortition.freezeAndRequest(keccak256("second"), second, 5);

        vrf.fulfill(secondRequest, 222);
        vrf.fulfill(firstRequest, 111);
        address[] memory firstSelected = sortition.deriveRanking(firstRequest);
        address[] memory secondSelected = sortition.deriveRanking(secondRequest);
        _requireUnique(firstSelected);
        _requireUnique(secondSelected);
        require(sortition.requestStatus(firstRequest).randomWord == 111, "first word drifted");
        require(sortition.requestStatus(secondRequest).randomWord == 222, "second word drifted");
    }

    function testLateAndUnknownFulfillmentsFailClosedWithoutCallbackRevert() public {
        address[] memory candidates = _candidates(8);
        uint256 requestId = sortition.freezeAndRequest(keccak256("late"), candidates, 4);
        vrf.fulfillUnknown(sortition, 999, 1);
        vm.warp(block.timestamp + 2 hours + 1);
        vrf.fulfill(requestId, 333);

        require(sortition.requestStatus(requestId).late, "late flag missing");
        (bool derived,) = address(sortition).call(abi.encodeCall(VrfSortitionCoordinatorV1.deriveRanking, (requestId)));
        require(!derived, "late randomness derived");
    }

    function _candidates(uint256 count) private pure returns (address[] memory result) {
        result = new address[](count);
        for (uint256 i = 0; i < count; i++) result[i] = address(uint160(i + 100));
    }

    function _requireUnique(address[] memory wallets) private pure {
        for (uint256 i = 0; i < wallets.length; i++) {
            require(wallets[i] != address(0), "selected zero");
            for (uint256 j = 0; j < i; j++) require(wallets[i] != wallets[j], "selected duplicate");
        }
    }
}
