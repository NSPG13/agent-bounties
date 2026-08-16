// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import {ISP1VerifierWithHash} from "./ISP1Verifier.sol";

interface ISp1VerifierAdapterV2Beta3 {
    function proofSystem() external view returns (bytes32);
    function verifier() external view returns (address);
    function verifierHash() external view returns (bytes32);
    function expectedRuntimeCodeHash() external view returns (bytes32);
    function verifierAvailable() external view returns (bool);
    function verify(bytes32 programVKey, bytes calldata publicValues, bytes calldata proofBytes) external view;
}

/// @notice Immutable proof-system adapter pinned to one project-owned SP1
/// verifier. Both the self-reported circuit hash and deployed runtime bytecode
/// must remain exact; there is no gateway route, owner, proxy or upgrade path.
contract Sp1VerifierAdapterV2Beta3 is ISp1VerifierAdapterV2Beta3 {
    bytes32 public immutable override proofSystem;
    address public immutable override verifier;
    bytes32 public immutable override verifierHash;
    bytes32 public immutable override expectedRuntimeCodeHash;

    constructor(bytes32 proofSystem_, address verifier_, bytes32 verifierHash_, bytes32 expectedRuntimeCodeHash_) {
        require(proofSystem_ != bytes32(0), "proof system zero");
        require(verifier_.code.length > 0, "verifier missing");
        require(verifierHash_ != bytes32(0), "verifier hash zero");
        require(expectedRuntimeCodeHash_ != bytes32(0), "runtime hash zero");
        require(verifier_.codehash == expectedRuntimeCodeHash_, "runtime hash mismatch");
        require(ISP1VerifierWithHash(verifier_).VERIFIER_HASH() == verifierHash_, "verifier hash mismatch");
        proofSystem = proofSystem_;
        verifier = verifier_;
        verifierHash = verifierHash_;
        expectedRuntimeCodeHash = expectedRuntimeCodeHash_;
    }

    function verifierAvailable() public view override returns (bool) {
        if (verifier.code.length == 0 || verifier.codehash != expectedRuntimeCodeHash) return false;
        try ISP1VerifierWithHash(verifier).VERIFIER_HASH() returns (bytes32 observedHash) {
            return observedHash == verifierHash;
        } catch {
            return false;
        }
    }

    function verify(bytes32 programVKey, bytes calldata publicValues, bytes calldata proofBytes)
        external
        view
        override
    {
        require(verifierAvailable(), "verifier unavailable");
        require(proofBytes.length >= 4 && bytes4(proofBytes[:4]) == bytes4(verifierHash), "selector mismatch");
        ISP1VerifierWithHash(verifier).verifyProof(programVKey, publicValues, proofBytes);
    }
}
