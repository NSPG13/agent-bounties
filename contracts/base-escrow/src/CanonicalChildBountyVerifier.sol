// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";

interface ICanonicalBountyFactoryView {
    function settlementToken() external view returns (address);
    function isCanonicalBounty(address bounty) external view returns (bool);
}

interface ICanonicalBountyView {
    function bountyId() external view returns (bytes32);
    function creator() external view returns (address);
    function factory() external view returns (address);
    function settlementToken() external view returns (address);
    function solverReward() external view returns (uint256);
    function targetAmount() external view returns (uint256);
    function fundedAmount() external view returns (uint256);
    function acceptanceCriteriaHash() external view returns (bytes32);
    function benchmarkHash() external view returns (bytes32);
    function verificationMode() external view returns (uint8);
    function verifierModule() external view returns (address);
    function threshold() external view returns (uint8);
    function status() external view returns (uint8);
    function round() external view returns (uint64);
    function solver() external view returns (address);
    function activeClaimBond() external view returns (uint256);
    function submissionHash() external view returns (bytes32);
    function evidenceHash() external view returns (bytes32);
    function verificationExpiresAt() external view returns (uint64);
}

/// @notice Verifies a distribution loop using only canonical on-chain state.
/// A parent solver passes after posting a parent-bound child bounty, funding it,
/// and attracting a different wallet that completes it and receives settlement.
contract CanonicalChildBountyVerifier is IAgentBountyVerifier {
    struct ChildSnapshot {
        bytes32 bountyId;
        uint256 target;
        uint256 funded;
        uint64 round;
        address solver;
        uint256 bond;
        bytes32 submissionHash;
        bytes32 evidenceHash;
        uint64 verificationExpiresAt;
    }

    bytes32 public constant PROTOCOL_TAG = keccak256("agent-bounties/canonical-child-v1");
    uint8 public constant DETERMINISTIC_MODULE_MODE = 0;
    uint8 public constant SUBMITTED_STATUS = 3;
    uint8 public constant SETTLED_STATUS = 4;

    string public constant ACCEPTANCE_CRITERIA_JSON = '["Post a canonical autonomous-v1 child bounty whose creator is the active solver.",'
        '"Fully fund the child to at least the parent solver reward; pooled contributors are allowed.",'
        '"Bind the child benchmark to the parent bounty ID and round and use an explicit deterministic verifier.",'
        '"Have a different wallet complete the child and receive canonical settlement before the parent verification deadline."]';
    bytes32 public constant ACCEPTANCE_CRITERIA_HASH = keccak256(bytes(ACCEPTANCE_CRITERIA_JSON));

    address public immutable canonicalFactory;
    address public immutable settlementToken;

    constructor(address canonicalFactory_) {
        require(canonicalFactory_ != address(0), "factory zero");
        address token = ICanonicalBountyFactoryView(canonicalFactory_).settlementToken();
        require(token != address(0), "token zero");
        canonicalFactory = canonicalFactory_;
        settlementToken = token;
    }

    /// @notice Canonical JSON committed as the child bounty benchmark.
    function expectedBenchmarkJson(bytes32 parentBountyId, uint64 parentRound) public pure returns (string memory) {
        return string.concat(
            '{"parent_bounty_id":"',
            _hexBytes32(parentBountyId),
            '","parent_round_hex":"',
            _hexUint64(parentRound),
            '","protocol":"agent-bounties/canonical-child-v1"}'
        );
    }

    function expectedBenchmarkHash(bytes32 parentBountyId, uint64 parentRound) public pure returns (bytes32) {
        return keccak256(bytes(expectedBenchmarkJson(parentBountyId, parentRound)));
    }

    /// @notice The proof is exactly abi.encode(childBountyAddress).
    function verify(
        bytes32 parentBountyId,
        uint64 parentRound,
        address parentSolver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 policyHash,
        bytes calldata proof
    ) external view returns (bool passed, bytes32 responseHash) {
        responseHash = keccak256(
            abi.encode(
                PROTOCOL_TAG,
                msg.sender,
                parentBountyId,
                parentRound,
                parentSolver,
                submissionHash,
                evidenceHash,
                policyHash,
                keccak256(proof)
            )
        );

        if (proof.length != 32 || !_validParent(msg.sender, parentBountyId, parentRound, parentSolver)) {
            return (false, responseHash);
        }

        return _verifyChildProof(parentBountyId, parentRound, parentSolver, proof, responseHash);
    }

    function _verifyChildProof(
        bytes32 parentBountyId,
        uint64 parentRound,
        address parentSolver,
        bytes calldata proof,
        bytes32 baseResponseHash
    ) private view returns (bool passed, bytes32 responseHash) {
        uint256 encodedChild;
        assembly ("memory-safe") {
            encodedChild := calldataload(proof.offset)
        }
        if (encodedChild >> 160 != 0) return (false, baseResponseHash);

        // The high-bit check above proves this cast cannot truncate the encoded address.
        // forge-lint: disable-next-line(unsafe-typecast)
        address childAddress = address(uint160(encodedChild));
        (bool validChild, bytes32 childStateHash) =
            _validChild(msg.sender, childAddress, parentBountyId, parentRound, parentSolver);
        responseHash = keccak256(abi.encode(baseResponseHash, childAddress, childStateHash));
        return (validChild, responseHash);
    }

    function _validParent(address parentAddress, bytes32 parentBountyId, uint64 parentRound, address parentSolver)
        private
        view
        returns (bool)
    {
        if (!ICanonicalBountyFactoryView(canonicalFactory).isCanonicalBounty(parentAddress)) return false;

        ICanonicalBountyView parent = ICanonicalBountyView(parentAddress);
        return parent.bountyId() == parentBountyId && parent.factory() == canonicalFactory
            && parent.settlementToken() == settlementToken
            && parent.acceptanceCriteriaHash() == ACCEPTANCE_CRITERIA_HASH
            && parent.verificationMode() == DETERMINISTIC_MODULE_MODE && parent.verifierModule() == address(this)
            && parent.threshold() == 1 && parent.status() == SUBMITTED_STATUS && parent.round() == parentRound
            && parent.solver() == parentSolver;
    }

    function _validChild(
        address parentAddress,
        address childAddress,
        bytes32 parentBountyId,
        uint64 parentRound,
        address parentSolver
    ) private view returns (bool valid, bytes32 childStateHash) {
        if (
            childAddress == address(0) || childAddress == parentAddress
                || !ICanonicalBountyFactoryView(canonicalFactory).isCanonicalBounty(childAddress)
        ) return (false, bytes32(0));

        ICanonicalBountyView parent = ICanonicalBountyView(parentAddress);
        ICanonicalBountyView child = ICanonicalBountyView(childAddress);
        if (child.creator() != parentSolver) return (false, bytes32(0));
        if (child.factory() != canonicalFactory) return (false, bytes32(0));
        if (child.settlementToken() != settlementToken) return (false, bytes32(0));
        if (child.benchmarkHash() != expectedBenchmarkHash(parentBountyId, parentRound)) return (false, bytes32(0));
        if (child.verificationMode() != DETERMINISTIC_MODULE_MODE) return (false, bytes32(0));
        address childVerifier = child.verifierModule();
        if (childVerifier == address(0) || childVerifier.code.length == 0) return (false, bytes32(0));
        if (child.threshold() != 1 || child.status() != SETTLED_STATUS) return (false, bytes32(0));

        ChildSnapshot memory snapshot;
        snapshot.target = child.targetAmount();
        if (snapshot.target < parent.solverReward()) return (false, bytes32(0));
        snapshot.funded = child.fundedAmount();
        if (snapshot.funded != 0) return (false, bytes32(0));
        snapshot.bond = child.activeClaimBond();
        if (snapshot.bond != 0) return (false, bytes32(0));

        snapshot.solver = child.solver();
        if (snapshot.solver == address(0) || snapshot.solver == parentSolver) return (false, bytes32(0));
        snapshot.submissionHash = child.submissionHash();
        snapshot.evidenceHash = child.evidenceHash();
        if (snapshot.submissionHash == bytes32(0) || snapshot.evidenceHash == bytes32(0)) {
            return (false, bytes32(0));
        }
        snapshot.verificationExpiresAt = child.verificationExpiresAt();
        snapshot.bountyId = child.bountyId();
        snapshot.round = child.round();

        childStateHash = keccak256(abi.encode(snapshot));
        return (true, childStateHash);
    }

    function _hexBytes32(bytes32 value) private pure returns (string memory) {
        bytes memory result = new bytes(66);
        result[0] = "0";
        result[1] = "x";
        bytes16 alphabet = "0123456789abcdef";
        for (uint256 i = 0; i < 32; i++) {
            uint8 current = uint8(value[i]);
            result[2 + i * 2] = alphabet[current >> 4];
            result[3 + i * 2] = alphabet[current & 0x0f];
        }
        return string(result);
    }

    function _hexUint64(uint64 value) private pure returns (string memory) {
        bytes memory result = new bytes(18);
        result[0] = "0";
        result[1] = "x";
        bytes16 alphabet = "0123456789abcdef";
        for (uint256 i = 0; i < 16; i++) {
            result[17 - i] = alphabet[value & 0x0f];
            value >>= 4;
        }
        return string(result);
    }
}
