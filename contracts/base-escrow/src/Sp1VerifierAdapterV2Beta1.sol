// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

interface ISP1VerifierV2Beta1 {
    function verifyProof(bytes32 programVKey, bytes calldata publicValues, bytes calldata proofBytes) external view;
}

interface ISP1VerifierGatewayV2Beta1 is ISP1VerifierV2Beta1 {
    function routes(bytes4 selector) external view returns (address verifier, bool frozen);
}

interface ISp1VerifierAdapterV2Beta1 {
    function proofSystem() external view returns (bytes32);
    function gateway() external view returns (address);
    function verifierSelector() external view returns (bytes4);
    function expectedVerifier() external view returns (address);
    function gatewayAvailable() external view returns (bool);
    function verify(bytes32 programVKey, bytes calldata publicValues, bytes calldata proofBytes) external view;
}

/// @notice Immutable proof-system adapter. Invalid proofs are expected to
/// revert in the canonical SP1 gateway and are mapped to a stable V2 error by
/// the competition contract.
contract Sp1VerifierAdapterV2Beta1 is ISp1VerifierAdapterV2Beta1 {
    bytes32 public immutable override proofSystem;
    address public immutable override gateway;
    bytes4 public immutable override verifierSelector;
    address public immutable override expectedVerifier;

    constructor(bytes32 proofSystem_, address gateway_, bytes4 verifierSelector_, address expectedVerifier_) {
        require(proofSystem_ != bytes32(0), "proof system zero");
        require(gateway_.code.length > 0, "gateway missing");
        require(verifierSelector_ != bytes4(0), "selector zero");
        require(expectedVerifier_.code.length > 0, "verifier missing");
        (address routedVerifier, bool frozen) = ISP1VerifierGatewayV2Beta1(gateway_).routes(verifierSelector_);
        require(routedVerifier == expectedVerifier_ && !frozen, "route unavailable");
        proofSystem = proofSystem_;
        gateway = gateway_;
        verifierSelector = verifierSelector_;
        expectedVerifier = expectedVerifier_;
    }

    function gatewayAvailable() external view override returns (bool) {
        if (gateway.code.length == 0 || expectedVerifier.code.length == 0) return false;
        try ISP1VerifierGatewayV2Beta1(gateway).routes(verifierSelector) returns (address routedVerifier, bool frozen) {
            return routedVerifier == expectedVerifier && !frozen;
        } catch {
            return false;
        }
    }

    function verify(bytes32 programVKey, bytes calldata publicValues, bytes calldata proofBytes)
        external
        view
        override
    {
        require(proofBytes.length >= 4 && bytes4(proofBytes[:4]) == verifierSelector, "selector mismatch");
        ISP1VerifierGatewayV2Beta1(gateway).verifyProof(programVKey, publicValues, proofBytes);
    }
}
