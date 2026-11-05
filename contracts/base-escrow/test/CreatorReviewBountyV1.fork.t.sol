// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;
import "./CreatorReviewBountyV1.t.sol";

interface CreatorForkVm {
    function envOr(string calldata, bool) external returns (bool);
    function skip(bool) external;
}

interface CircleForkToken {
    function masterMinter() external view returns (address);
    function configureMinter(address, uint256) external returns (bool);
    function mint(address, uint256) external returns (bool);
    function balanceOf(address) external view returns (uint256);
    function DOMAIN_SEPARATOR() external view returns (bytes32);
}

/// Local fork only. Impersonated minting never leaves the in-memory fork; there
/// is no broadcast cheatcode or real signing key in this test.
contract CreatorOpenCircleForkTest {
    CreatorOpenVm private constant vm = CreatorOpenVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint256 private constant CREATOR_KEY = uint256(keccak256("agent-bounties/creator-open/fork-creator/20260929"));
    uint256 private constant SOLVER_KEY = uint256(keccak256("agent-bounties/creator-open/fork-solver/20260929"));
    CircleForkToken private constant token = CircleForkToken(0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913);

    function signature(uint256 key, bytes32 digest) private returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);
        return abi.encodePacked(r, s, v);
    }

    function authorization(uint256 key, address from, address to, uint256 amount, bytes32 nonce, bool contractWallet)
        private
        returns (CreatorReviewBountyV1.TokenAuthorization memory a)
    {
        a.validBefore = block.timestamp + 1 hours;
        a.nonce = nonce;
        bytes32 structHash = keccak256(
            abi.encode(
                keccak256(
                    "ReceiveWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)"
                ),
                from,
                to,
                amount,
                0,
                a.validBefore,
                nonce
            )
        );
        a.signature = signature(key, keccak256(abi.encodePacked("\x19\x01", token.DOMAIN_SEPARATOR(), structHash)));
        if (contractWallet) a.signature = abi.encodePacked(a.signature, hex"01");
    }

    function testForkCircleEOAAndContractWalletHaveSponsoredLifecycle() public {
        if (!CreatorForkVm(address(vm)).envOr("RUN_CREATOR_OPEN_FORK", false)) {
            CreatorForkVm(address(vm)).skip(true);
            return;
        }
        vm.createSelectFork("https://base.drpc.org");
        address master = token.masterMinter();
        vm.prank(master);
        require(token.configureMinter(address(this), 100_000_000), "fork minter");
        address creator = vm.addr(CREATOR_KEY);
        require(
            creator.code.length == 0 && creator.balance == 0 && token.balanceOf(creator) == 0,
            "fresh public fixture identity"
        );
        address solver = address(new CreatorOpen1271(vm.addr(SOLVER_KEY)));
        require(token.mint(creator, 21_300_000) && token.mint(solver, 1_000_000), "fork balances");
        CreatorReviewBountyFactoryV1 factory = new CreatorReviewBountyFactoryV1(address(token));
        CreatorReviewBountyV1.Config memory c = CreatorReviewBountyV1.Config(
            keccak256("fork"),
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
        CreatorReviewBountyV1 bounty = CreatorReviewBountyV1(
            factory.createWithAuthorization(
                c, bytes32(0), authorization(CREATOR_KEY, creator, predicted, 21_300_000, keccak256("fund"), false)
            )
        );
        CreatorReviewBountyV1.Submission memory s = CreatorReviewBountyV1.Submission(
            solver, keccak256("result"), keccak256("evidence"), c.termsHash, 1, block.timestamp + 1 hours
        );
        CreatorReviewBountyV1.TokenAuthorization memory a =
            authorization(SOLVER_KEY, solver, predicted, 1_000_000, keccak256("bond"), true);
        bytes memory signed = abi.encodePacked(signature(SOLVER_KEY, bounty.submissionDigest(s, a.nonce)), hex"01");
        uint64 id = bounty.submitAuthorized(s, a, signed);
        bytes32 response = keccak256("approved");
        uint256 deadline = block.timestamp + 1 hours;
        signed = signature(CREATOR_KEY, bounty.reviewDigest(id, true, response, deadline));
        bounty.reviewAuthorized(id, true, response, deadline, signed);
        require(
            token.balanceOf(solver) == 21_300_000 && token.balanceOf(creator) == 1_000_000, "real Circle exact payout"
        );
        require(creator.balance == 0 && solver.balance == 0, "zero ETH customers");
        require(token.balanceOf(predicted) == 0 && bounty.winner() == id, "fully reconciled fork");
    }
}
