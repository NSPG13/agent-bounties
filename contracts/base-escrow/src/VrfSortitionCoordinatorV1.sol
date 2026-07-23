// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

/// @dev ABI-compatible subset of Chainlink VRF v2.5 contracts-v1.3.0.
library VrfV2PlusClientV1 {
    bytes4 internal constant EXTRA_ARGS_V1_TAG = bytes4(keccak256("VRF ExtraArgsV1"));

    struct ExtraArgsV1 {
        bool nativePayment;
    }

    struct RandomWordsRequest {
        bytes32 keyHash;
        uint256 subId;
        uint16 requestConfirmations;
        uint32 callbackGasLimit;
        uint32 numWords;
        bytes extraArgs;
    }

    function argsToBytes(ExtraArgsV1 memory extraArgs) internal pure returns (bytes memory) {
        return abi.encodeWithSelector(EXTRA_ARGS_V1_TAG, extraArgs);
    }
}

interface IVrfCoordinatorV2PlusV1 {
    function requestRandomWords(VrfV2PlusClientV1.RandomWordsRequest calldata request)
        external
        returns (uint256 requestId);
}

/// @notice Freezes bounded candidate sets and obtains one native-funded
/// Chainlink VRF v2.5 word per immutable commitment. The callback only stores
/// data; ranking is derived in a separate call so fulfillment cannot be bricked.
contract VrfSortitionCoordinatorV1 {
    using VrfV2PlusClientV1 for VrfV2PlusClientV1.ExtraArgsV1;

    struct Request {
        bytes32 commitment;
        bytes32 candidateHash;
        uint64 requestedAt;
        uint64 fulfilledAt;
        uint8 candidateCount;
        uint8 selectionCount;
        bool fulfilled;
        bool late;
        bool rankingDerived;
        uint256 randomWord;
    }

    uint16 public constant REQUEST_CONFIRMATIONS = 3;
    uint32 public constant NUM_WORDS = 1;
    uint32 public constant CALLBACK_GAS_LIMIT = 150_000;
    uint64 public constant FULFILLMENT_DEADLINE = 2 hours;
    uint256 public constant MAX_CANDIDATES = 64;

    address public immutable vrfCoordinator;
    address public immutable controller;
    uint256 public immutable subscriptionId;
    bytes32 public immutable keyHash;

    mapping(bytes32 => bool) public commitmentUsed;
    mapping(bytes32 => uint256) public requestIdByCommitment;
    mapping(uint256 => Request) public requests;
    mapping(uint256 => address[]) private _candidates;
    mapping(uint256 => address[]) private _ranking;

    event CandidateSetFrozen(
        bytes32 indexed commitment, bytes32 indexed candidateHash, uint256 candidateCount, uint256 selectionCount
    );
    event RandomnessRequested(bytes32 indexed commitment, uint256 indexed requestId, uint64 requestedAt);
    event RandomnessStored(uint256 indexed requestId, uint256 randomWord, bool late);
    event FulfillmentIgnored(uint256 indexed requestId, bytes32 reason);
    event RankingDerived(uint256 indexed requestId, bytes32 indexed commitment, bytes32 rankingHash);

    modifier onlyController() {
        require(msg.sender == controller, "controller only");
        _;
    }

    constructor(
        address vrfCoordinator_,
        address controller_,
        uint256 subscriptionId_,
        bytes32 keyHash_
    ) {
        require(vrfCoordinator_.code.length > 0, "coordinator missing");
        require(controller_ != address(0), "controller zero");
        require(subscriptionId_ != 0 && keyHash_ != bytes32(0), "vrf config invalid");
        vrfCoordinator = vrfCoordinator_;
        controller = controller_;
        subscriptionId = subscriptionId_;
        keyHash = keyHash_;
    }

    function freezeAndRequest(bytes32 commitment, address[] calldata candidateSet, uint8 selectionCount)
        external
        onlyController
        returns (uint256 requestId)
    {
        require(commitment != bytes32(0) && !commitmentUsed[commitment], "commitment already used");
        require(candidateSet.length > 0 && candidateSet.length <= MAX_CANDIDATES, "candidate count invalid");
        require(selectionCount > 0 && selectionCount <= candidateSet.length, "selection count invalid");
        for (uint256 i = 0; i < candidateSet.length; i++) {
            require(candidateSet[i] != address(0), "candidate zero");
            for (uint256 j = 0; j < i; j++) require(candidateSet[j] != candidateSet[i], "candidate duplicate");
        }

        commitmentUsed[commitment] = true;
        bytes32 candidateHash = keccak256(abi.encode(candidateSet));
        emit CandidateSetFrozen(commitment, candidateHash, candidateSet.length, selectionCount);

        requestId = IVrfCoordinatorV2PlusV1(vrfCoordinator).requestRandomWords(
            VrfV2PlusClientV1.RandomWordsRequest({
                keyHash: keyHash,
                subId: subscriptionId,
                requestConfirmations: REQUEST_CONFIRMATIONS,
                callbackGasLimit: CALLBACK_GAS_LIMIT,
                numWords: NUM_WORDS,
                extraArgs: VrfV2PlusClientV1.argsToBytes(VrfV2PlusClientV1.ExtraArgsV1({nativePayment: true}))
            })
        );
        require(requestId != 0 && requests[requestId].requestedAt == 0, "request id invalid");
        requestIdByCommitment[commitment] = requestId;
        requests[requestId] = Request({
            commitment: commitment,
            candidateHash: candidateHash,
            requestedAt: uint64(block.timestamp),
            fulfilledAt: 0,
            candidateCount: uint8(candidateSet.length),
            selectionCount: selectionCount,
            fulfilled: false,
            late: false,
            rankingDerived: false,
            randomWord: 0
        });
        for (uint256 i = 0; i < candidateSet.length; i++) _candidates[requestId].push(candidateSet[i]);
        emit RandomnessRequested(commitment, requestId, uint64(block.timestamp));
    }

    /// @notice Chainlink-compatible entry point. Unknown, duplicate, malformed,
    /// or late fulfillments are recorded or ignored without reverting.
    function rawFulfillRandomWords(uint256 requestId, uint256[] calldata randomWords) external {
        if (msg.sender != vrfCoordinator) {
            emit FulfillmentIgnored(requestId, keccak256("caller-not-coordinator"));
            return;
        }
        Request storage request = requests[requestId];
        if (request.requestedAt == 0) {
            emit FulfillmentIgnored(requestId, keccak256("request-unknown"));
            return;
        }
        if (request.fulfilled) {
            emit FulfillmentIgnored(requestId, keccak256("request-already-fulfilled"));
            return;
        }
        if (randomWords.length != 1) {
            emit FulfillmentIgnored(requestId, keccak256("word-count-invalid"));
            return;
        }
        request.fulfilled = true;
        request.fulfilledAt = uint64(block.timestamp);
        request.randomWord = randomWords[0];
        request.late = block.timestamp > uint256(request.requestedAt) + FULFILLMENT_DEADLINE;
        emit RandomnessStored(requestId, randomWords[0], request.late);
    }

    function deriveRanking(uint256 requestId) external returns (address[] memory selectedWallets) {
        Request storage request = requests[requestId];
        require(request.fulfilled && !request.late, "randomness unavailable");
        require(!request.rankingDerived, "ranking already derived");
        address[] memory ranked = _candidates[requestId];
        for (uint256 remaining = ranked.length; remaining > 1; remaining--) {
            uint256 index = uint256(
                keccak256(abi.encode(request.randomWord, request.commitment, request.candidateHash, remaining))
            ) % remaining;
            address swap = ranked[remaining - 1];
            ranked[remaining - 1] = ranked[index];
            ranked[index] = swap;
        }
        request.rankingDerived = true;
        for (uint256 i = 0; i < ranked.length; i++) _ranking[requestId].push(ranked[i]);
        selectedWallets = new address[](request.selectionCount);
        for (uint256 i = 0; i < request.selectionCount; i++) selectedWallets[i] = ranked[i];
        emit RankingDerived(requestId, request.commitment, keccak256(abi.encode(ranked)));
    }

    function candidates(uint256 requestId) external view returns (address[] memory) {
        return _candidates[requestId];
    }

    function ranking(uint256 requestId) external view returns (address[] memory) {
        return _ranking[requestId];
    }

    function selected(uint256 requestId) external view returns (address[] memory result) {
        Request storage request = requests[requestId];
        require(request.rankingDerived, "ranking unavailable");
        result = new address[](request.selectionCount);
        for (uint256 i = 0; i < request.selectionCount; i++) result[i] = _ranking[requestId][i];
    }

    function requestStatus(uint256 requestId) external view returns (Request memory) {
        return requests[requestId];
    }
}
