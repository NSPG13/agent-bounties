// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

interface IERC20BountyToken {
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 value) external returns (bool);
    function transferFrom(address from, address to, uint256 value) external returns (bool);
}

/// @notice Circle USDC implements EIP-3009 on Base. The signed authorization
/// binds a contributor, recipient, amount, validity window, and unique nonce.
interface IEIP3009BountyToken {
    function transferWithAuthorization(
        address from,
        address to,
        uint256 value,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external;
}

interface IERC1271 {
    function isValidSignature(bytes32 hash, bytes calldata signature) external view returns (bytes4);
}

interface IERC165 {
    function supportsInterface(bytes4 interfaceId) external view returns (bool);
}

/// @notice Minimum interface an externally deployed bounty must expose to be indexed.
/// Registration is discovery only. It does not make an external contract canonical or trusted.
interface IAgentBountyV1 is IERC165 {
    function protocolVersion() external pure returns (bytes32);
    function bountyId() external view returns (bytes32);
    function creator() external view returns (address);
    function settlementToken() external view returns (address);
    function termsHash() external view returns (bytes32);
    function policyHash() external view returns (bytes32);
    function acceptanceCriteriaHash() external view returns (bytes32);
    function benchmarkHash() external view returns (bytes32);
    function evidenceSchemaHash() external view returns (bytes32);
    function verifierSetHash() external view returns (bytes32);
    function targetAmount() external view returns (uint256);
    function fundedAmount() external view returns (uint256);
    function status() external view returns (uint8);
    function fund(uint256 requestedAmount) external returns (uint256 acceptedAmount);
    function fundWithAuthorization(
        address contributor,
        uint256 amount,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external;
    function claim() external;
    function submit(bytes32 submissionHash, bytes32 evidenceHash) external;
}

/// @notice Deterministic verifier modules are fixed in the bounty policy at creation.
interface IAgentBountyVerifier {
    function verify(
        bytes32 bountyId,
        uint64 round,
        address solver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 policyHash,
        bytes calldata proof
    ) external view returns (bool passed, bytes32 responseHash);
}

library SafeBountyToken {
    error TokenCallFailed();
    error TokenReturnedFalse();

    function safeTransfer(address token, address to, uint256 amount) internal {
        _call(token, abi.encodeCall(IERC20BountyToken.transfer, (to, amount)));
    }

    function safeTransferFrom(address token, address from, address to, uint256 amount) internal {
        _call(token, abi.encodeCall(IERC20BountyToken.transferFrom, (from, to, amount)));
    }

    function safeTransferWithAuthorization(
        address token,
        address from,
        address to,
        uint256 amount,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) internal {
        _call(
            token,
            abi.encodeCall(
                IEIP3009BountyToken.transferWithAuthorization,
                (from, to, amount, validAfter, validBefore, nonce, v, r, s)
            )
        );
    }

    function _call(address token, bytes memory data) private {
        (bool ok, bytes memory result) = token.call(data);
        if (!ok) revert TokenCallFailed();
        if (result.length > 0 && !abi.decode(result, (bool))) revert TokenReturnedFalse();
    }
}
