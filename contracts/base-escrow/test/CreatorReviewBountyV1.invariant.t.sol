// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;
import "./CreatorReviewBountyV1.t.sol";

contract CreatorOpenHandler {
    CreatorOpenVm private constant vm = CreatorOpenVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    CreatorOpenStrictToken public immutable token;
    CreatorReviewBountyFactoryV1 public immutable factory;
    CreatorReviewBountyV1 public immutable bounty;
    uint256 private nonce;
    uint256 public winners;
    uint256 public principalDeposited = 21_300_000;
    uint256 public principalReturned;
    uint256 public rejectedBonds;
    uint256 public refundedBonds;
    uint256 public submittedBonds;
    uint256 public paidOnSettlement;
    uint256 private constant CREATOR_KEY = 0xC0FFEE;

    constructor() {
        token = new CreatorOpenStrictToken();
        factory = new CreatorReviewBountyFactoryV1(address(token));
        address creator = vm.addr(CREATOR_KEY);
        token.mint(creator, 1_000_000_000_000);
        CreatorReviewBountyV1.Config memory c = CreatorReviewBountyV1.Config(
            keccak256("invariant"),
            creator,
            20_300_000,
            1_000_000,
            keccak256("terms"),
            keccak256("criteria"),
            keccak256("schema"),
            uint64(block.timestamp + 90 days),
            2 days,
            8
        );
        address predicted = factory.predict(c, bytes32(0));
        bounty = CreatorReviewBountyV1(
            factory.createWithAuthorization(c, bytes32(0), authorization(CREATOR_KEY, predicted, 21_300_000))
        );
    }

    function signature(uint256 key, bytes32 digest) private returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);
        return abi.encodePacked(r, s, v);
    }

    function authorization(uint256 key, address to, uint256 amount)
        private
        returns (CreatorReviewBountyV1.TokenAuthorization memory a)
    {
        a.validBefore = block.timestamp + 1 hours;
        a.nonce = bytes32(++nonce);
        a.signature = signature(key, token.digest(vm.addr(key), to, amount, 0, a.validBefore, a.nonce));
    }

    function submit(uint8 seed) external {
        uint256 key = 100 + uint256(seed % 24);
        address solver = vm.addr(key);
        CreatorReviewBountyV1.Submission memory s = CreatorReviewBountyV1.Submission(
            solver,
            keccak256(abi.encode(nonce, "result")),
            keccak256(abi.encode(nonce, "evidence")),
            bounty.termsHash(),
            bounty.revision(),
            block.timestamp + 1 hours
        );
        token.mint(solver, 1_000_000);
        CreatorReviewBountyV1.TokenAuthorization memory a = authorization(key, address(bounty), 1_000_000);
        bytes memory sig = signature(key, bounty.submissionDigest(s, a.nonce));
        try bounty.submitAuthorized(s, a, sig) returns (uint64) {
            submittedBonds += 1_000_000;
        } catch {}
    }

    function review(uint64 seed, bool passed) external {
        uint64 id = seed % (bounty.entryCount() + 1) + 1;
        uint256 deadline = block.timestamp + 1 hours;
        bytes32 response = keccak256(abi.encode(id, passed));
        bytes memory sig = signature(CREATOR_KEY, bounty.reviewDigest(id, passed, response, deadline));
        try bounty.reviewAuthorized(id, passed, response, deadline, sig) {
            if (passed) {
                winners++;
                paidOnSettlement = bounty.solverReward() + 2_000_000;
            } else {
                rejectedBonds += 1_000_000;
            }
        } catch {}
    }

    function refund(uint64 seed) external {
        uint64 id = seed % (bounty.entryCount() + 1) + 1;
        try bounty.refundBond(id) {
            refundedBonds += 1_000_000;
        } catch {}
    }

    function increase(uint64 seed) external {
        uint128 newReward = uint128(bounty.solverReward() + uint256(seed % 100_000_000) + 1);
        uint256 added = uint256(newReward) - bounty.solverReward();
        bytes32 terms = keccak256(abi.encode(seed, nonce));
        uint256 deadline = block.timestamp + 1 hours;
        CreatorReviewBountyV1.TokenAuthorization memory a = authorization(CREATOR_KEY, address(bounty), added);
        bytes memory sig = signature(CREATOR_KEY, bounty.increaseDigest(newReward, terms, a.nonce, deadline));
        try bounty.increaseRewardAuthorized(newReward, terms, a, deadline, sig) {
            principalDeposited += added;
        } catch {}
    }

    function cancel() external {
        uint256 deadline = block.timestamp + 1 hours;
        bytes memory sig = signature(CREATOR_KEY, bounty.cancelDigest(deadline));
        try bounty.cancelAuthorized(deadline, sig) {} catch {}
    }

    function expire() external {
        try bounty.expire() {} catch {}
    }

    function refundPrincipal() external {
        uint256 amount = bounty.refundablePrincipal();
        try bounty.refundPrincipal() {
            principalReturned += amount;
        } catch {}
    }

    function advance(uint32 seconds_) external {
        vm.warp(block.timestamp + uint256(seconds_ % 10 days));
    }
}

contract CreatorOpenInvariantTest {
    CreatorOpenVm private constant vm = CreatorOpenVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    CreatorOpenHandler private handler;
    address[] private targets;

    function setUp() public {
        vm.chainId(84532);
        vm.warp(1_800_000_000);
        handler = new CreatorOpenHandler();
        targets.push(address(handler));
    }

    function targetContracts() external view returns (address[] memory) {
        return targets;
    }

    function invariantConservationAndSingleWinner() public view {
        CreatorReviewBountyV1 bounty = handler.bounty();
        uint256 balance = handler.token().balanceOf(address(bounty));
        require(balance == bounty.refundablePrincipal() + bounty.outstandingBonds(), "exact solvency");
        require(
            balance
                == handler.principalDeposited() + handler.submittedBonds() - handler.principalReturned()
                    - handler.rejectedBonds() - handler.refundedBonds() - handler.paidOnSettlement(),
            "independent cash accounting"
        );
        require(handler.winners() <= 1, "multiple payouts");
        require(handler.token().balanceOf(address(handler.factory())) == 0, "factory custody");
    }

    function invariantEveryPendingBondHasOneLiveEntry() public view {
        CreatorReviewBountyV1 bounty = handler.bounty();
        uint256 count;
        for (uint64 i = 1; i <= bounty.entryCount(); i++) {
            (address solver,,,, CreatorReviewBountyV1.EntryStatus status) = bounty.entries(i);
            if (status == CreatorReviewBountyV1.EntryStatus.Pending) {
                count++;
                require(bounty.activeEntry(solver) == i, "pending entry map drift");
            }
        }
        require(count == bounty.pendingEntries(), "pending count drift");
        require(count <= 8 && count * 1_000_000 == bounty.outstandingBonds(), "bond obligation drift");
    }
}
