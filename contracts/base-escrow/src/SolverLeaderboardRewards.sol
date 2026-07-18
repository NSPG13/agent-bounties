// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";

/// @notice Permissionlessly funded, quorum-finalized prizes for canonical solver rankings.
contract SolverLeaderboardRewards {
    using SafeBountyToken for address;

    enum PeriodKind {
        Daily,
        Weekly
    }

    uint256 public constant DAILY_REWARD = 3_000_000;
    uint256 public constant WEEKLY_REWARD = 26_000_000;
    uint64 public constant FINALIZATION_DELAY = 1 hours;
    uint64 private constant MONDAY_EPOCH_OFFSET = 4 days;
    bytes4 private constant ERC1271_MAGIC_VALUE = 0x1626ba7e;
    uint256 private constant ERC1271_GAS_LIMIT = 200_000;
    uint256 private constant SECP256K1N_DIV_2 = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;
    bytes32 private constant EIP712_DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant NAME_HASH = keccak256("Agent Bounties Solver Leaderboard");
    bytes32 private constant VERSION_HASH = keccak256("1");
    bytes32 private constant AWARD_TYPEHASH = keccak256(
        "LeaderboardAward(uint8 kind,uint64 startsAt,address winner,uint32 eligibleCompletions,bytes32 evidenceHash)"
    );

    address public immutable settlementToken;
    address public immutable signerA;
    address public immutable signerB;
    uint64 public immutable firstDailyStart;
    uint64 public immutable firstWeeklyStart;
    mapping(bytes32 => bool) public paidAwards;
    mapping(bytes32 => address) public paidAwardWinner;
    uint256 private _reentrancy = 1;

    event RewardFunded(address indexed contributor, uint256 amount, uint256 balance);
    event LeaderboardRewardPaid(
        bytes32 indexed awardId,
        PeriodKind indexed kind,
        uint64 startsAt,
        address indexed winner,
        uint32 eligibleCompletions,
        bytes32 evidenceHash
    );

    modifier nonReentrant() {
        require(_reentrancy == 1, "reentrant");
        _reentrancy = 2;
        _;
        _reentrancy = 1;
    }

    constructor(address settlementToken_, address signerA_, address signerB_) {
        require(settlementToken_.code.length > 0, "token has no code");
        require(signerA_ != address(0) && signerB_ != address(0), "signer zero");
        require(signerA_ != signerB_, "signers not distinct");
        require(block.timestamp >= MONDAY_EPOCH_OFFSET, "timestamp predates calendar");
        settlementToken = settlementToken_;
        signerA = signerA_;
        signerB = signerB_;
        uint64 deployedAt = uint64(block.timestamp);
        firstDailyStart = deployedAt - (deployedAt % 1 days);
        firstWeeklyStart = deployedAt - ((deployedAt - MONDAY_EPOCH_OFFSET) % 7 days);
    }

    function fund(uint256 amount) external nonReentrant {
        require(amount > 0, "amount zero");
        settlementToken.safeTransferFrom(msg.sender, address(this), amount);
        emit RewardFunded(msg.sender, amount, IERC20BountyToken(settlementToken).balanceOf(address(this)));
    }

    function awardId(PeriodKind kind, uint64 startsAt) public pure returns (bytes32) {
        return keccak256(abi.encode(kind, startsAt));
    }

    function awardDigest(
        PeriodKind kind,
        uint64 startsAt,
        address winner,
        uint32 eligibleCompletions,
        bytes32 evidenceHash
    ) public view returns (bytes32) {
        bytes32 structHash = keccak256(
            abi.encode(AWARD_TYPEHASH, kind, startsAt, winner, eligibleCompletions, evidenceHash)
        );
        bytes32 domainSeparator =
            keccak256(abi.encode(EIP712_DOMAIN_TYPEHASH, NAME_HASH, VERSION_HASH, block.chainid, address(this)));
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator, structHash));
    }

    function pay(
        PeriodKind kind,
        uint64 startsAt,
        address winner,
        uint32 eligibleCompletions,
        bytes32 evidenceHash,
        bytes calldata signatureA,
        bytes calldata signatureB
    ) external nonReentrant {
        uint256 amount = _rewardAmountForClosedPeriod(kind, startsAt);
        require(winner != address(0), "winner zero");
        require(eligibleCompletions > 0, "no completions");
        require(evidenceHash != bytes32(0), "evidence zero");

        bytes32 id = awardId(kind, startsAt);
        require(!paidAwards[id], "award paid");
        {
            bytes32 digest = awardDigest(kind, startsAt, winner, eligibleCompletions, evidenceHash);
            bool direct =
                _isValidSignatureNow(signerA, digest, signatureA) && _isValidSignatureNow(signerB, digest, signatureB);
            bool reversed =
                _isValidSignatureNow(signerA, digest, signatureB) && _isValidSignatureNow(signerB, digest, signatureA);
            require(direct || reversed, "invalid quorum");
        }

        paidAwards[id] = true;
        paidAwardWinner[id] = winner;
        settlementToken.safeTransfer(winner, amount);
        emit LeaderboardRewardPaid(id, kind, startsAt, winner, eligibleCompletions, evidenceHash);
    }

    function _rewardAmountForClosedPeriod(PeriodKind kind, uint64 startsAt) private view returns (uint256 amount) {
        uint64 endsAt;
        (amount, endsAt) = _validatePeriod(kind, startsAt);
        require(block.timestamp >= uint256(endsAt) + FINALIZATION_DELAY, "period not final");
    }

    function _validatePeriod(PeriodKind kind, uint64 startsAt) private view returns (uint256 amount, uint64 endsAt) {
        if (kind == PeriodKind.Daily) {
            require(startsAt % 1 days == 0, "invalid daily period");
            require(startsAt >= firstDailyStart, "period predates program");
            return (DAILY_REWARD, startsAt + 1 days);
        }
        require(
            startsAt >= MONDAY_EPOCH_OFFSET && (startsAt - MONDAY_EPOCH_OFFSET) % 7 days == 0, "invalid weekly period"
        );
        require(startsAt >= firstWeeklyStart, "period predates program");
        return (WEEKLY_REWARD, startsAt + 7 days);
    }

    function _isValidSignatureNow(address signer, bytes32 digest, bytes calldata signature)
        private
        view
        returns (bool)
    {
        if (signer.code.length > 0) {
            bytes memory callData = abi.encodeCall(IERC1271.isValidSignature, (digest, signature));
            bool ok;
            bytes4 result;
            uint256 gasLimit = ERC1271_GAS_LIMIT;
            assembly ("memory-safe") {
                let output := mload(0x40)
                mstore(output, 0)
                ok := staticcall(gasLimit, signer, add(callData, 0x20), mload(callData), output, 0x20)
                result := mload(output)
            }
            return ok && result == ERC1271_MAGIC_VALUE;
        }
        return _recover(digest, signature) == signer;
    }

    function _recover(bytes32 digest, bytes calldata signature) private pure returns (address recovered) {
        if (signature.length != 65) return address(0);
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly ("memory-safe") {
            r := calldataload(signature.offset)
            s := calldataload(add(signature.offset, 0x20))
            v := byte(0, calldataload(add(signature.offset, 0x40)))
        }
        if (uint256(s) > SECP256K1N_DIV_2 || (v != 27 && v != 28)) return address(0);
        recovered = ecrecover(digest, v, r, s);
    }
}
