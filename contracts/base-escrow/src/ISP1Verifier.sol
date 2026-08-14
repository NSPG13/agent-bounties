// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.20;

interface ISP1Verifier {
    function verifyProof(bytes32 programVKey, bytes calldata publicValues, bytes calldata proofBytes) external view;
}

interface ISP1VerifierWithHash is ISP1Verifier {
    function VERIFIER_HASH() external pure returns (bytes32);
}
