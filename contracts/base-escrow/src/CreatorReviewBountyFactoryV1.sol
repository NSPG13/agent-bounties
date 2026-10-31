// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./CreatorReviewBountyV1.sol";

/// @notice Candidate factory, undeployed. No owner, upgrade or arbitrary-call authority.
contract CreatorReviewBountyFactoryV1 {
    address public immutable settlementToken;
    address public immutable implementation;
    mapping(address => bool) public isCanonicalBounty;
    uint256 private _guard = 1;
    event CreatorOpenBountyCreated(
        bytes32 indexed bountyId,
        address indexed bounty,
        address indexed creator,
        bytes32 termsHash,
        bytes32 configurationHash,
        bytes32 userSalt
    );

    constructor(address token) {
        settlementToken = token;
        implementation = address(new CreatorReviewBountyV1(token));
    }

    function createWithAuthorization(
        CreatorReviewBountyV1.Config calldata config,
        bytes32 userSalt,
        CreatorReviewBountyV1.TokenAuthorization calldata authorization
    ) external returns (address bounty) {
        require(_guard == 1, "reentrant");
        _guard = 2;
        bounty = predict(config, userSalt);
        if (!isCanonicalBounty[bounty]) {
            require(bounty.code.length == 0, "address occupied");
            bytes memory code = _initCode();
            bytes32 salt = keccak256(abi.encode(config, userSalt));
            address created;
            assembly { created := create2(0, add(code, 32), mload(code), salt) }
            require(created == bounty && created != address(0), "deployment failed");
            isCanonicalBounty[bounty] = true;
            CreatorReviewBountyV1(bounty).initializeAndFund(config, authorization);
            emit CreatorOpenBountyCreated(
                config.bountyId, bounty, config.creator, config.termsHash, keccak256(abi.encode(config)), userSalt
            );
        }
        _guard = 1;
    }

    function predict(CreatorReviewBountyV1.Config calldata config, bytes32 userSalt) public view returns (address) {
        bytes32 salt = keccak256(abi.encode(config, userSalt));
        return address(
            uint160(uint256(keccak256(abi.encodePacked(bytes1(0xff), address(this), salt, keccak256(_initCode())))))
        );
    }

    function _initCode() private view returns (bytes memory) {
        return abi.encodePacked(
            hex"3d602d80600a3d3981f3",
            hex"363d3d373d3d3d363d73",
            bytes20(implementation),
            hex"5af43d82803e903d91602b57fd5bf3"
        );
    }
}
