// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

/// @notice Test-only ERC-1271 account for the isolated posting Sepolia rehearsal.
/// @dev Has no delegate, arbitrary-call executor, policy updates or native-ETH path.
contract PostingSepoliaAccountFixture {
    address public immutable owner;

    constructor(address owner_) {
        require(owner_ != address(0), "missing owner");
        owner = owner_;
    }

    function isValidSignature(bytes32 digest, bytes calldata signature) external view returns (bytes4) {
        // Exercise both the legacy 65-byte adapter and wider signature routes.
        if (signature.length == 66 && signature[0] == 0x7f) signature = signature[1:];
        if (signature.length != 65) return 0xffffffff;
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly {
            r := calldataload(signature.offset)
            s := calldataload(add(signature.offset, 32))
            v := byte(0, calldataload(add(signature.offset, 64)))
        }
        return ecrecover(digest, v, r, s) == owner ? bytes4(0x1626ba7e) : bytes4(0xffffffff);
    }
}
