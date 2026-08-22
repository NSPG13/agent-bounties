// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./BoundedOpenCompetitionV2Wallet.sol";

/// @notice Deterministically deploys owner-recoverable Open Competition V2 reserve wallets.
contract BoundedOpenCompetitionV2WalletFactory {
    using SafeBountyToken for address;

    enum FundingMode {
        None,
        Allowance,
        Eip3009
    }

    OpenCompetitionBountyFactoryV2Beta3 public immutable competitionFactory;
    address public immutable settlementToken;
    address public immutable implementation;
    mapping(address => bool) public isFactoryWallet;
    uint256 private _reentrancy = 1;

    event BoundedOpenCompetitionV2WalletCreated(
        address indexed wallet,
        address indexed owner,
        address indexed delegate,
        bytes32 userSalt,
        bytes32 effectiveSalt,
        bytes32 initialPolicyHash,
        FundingMode fundingMode,
        uint256 initialFunding
    );

    error ReserveFactoryInvalidConfiguration();
    error ReserveFactoryNotOwner();
    error ReserveFactoryFundingZero();
    error ReserveFactoryWalletAlreadyFundable();
    error ReserveFactoryWalletOccupied();
    error ReserveFactoryDeploymentFailed();
    error ReserveFactoryFundingMismatch();
    error ReserveFactoryReentrantCall();

    modifier nonReentrant() {
        if (_reentrancy != 1) revert ReserveFactoryReentrantCall();
        _reentrancy = 2;
        _;
        _reentrancy = 1;
    }

    constructor(address competitionFactory_) {
        if (competitionFactory_.code.length == 0) revert ReserveFactoryInvalidConfiguration();
        competitionFactory = OpenCompetitionBountyFactoryV2Beta3(competitionFactory_);
        settlementToken = OpenCompetitionBountyFactoryV2Beta3(competitionFactory_).settlementToken();
        if (settlementToken.code.length == 0) revert ReserveFactoryInvalidConfiguration();
        implementation = address(new BoundedOpenCompetitionV2Wallet(competitionFactory_));
    }

    function createWallet(
        address owner,
        BoundedOpenCompetitionV2Wallet.Policy calldata policy,
        bytes32[] calldata approvedCreations,
        bytes32 userSalt
    ) external nonReentrant returns (address wallet) {
        if (msg.sender != owner) revert ReserveFactoryNotOwner();
        (wallet,) = _deploy(owner, policy, approvedCreations, userSalt, FundingMode.None, 0, true);
    }

    function createWalletAndFund(
        BoundedOpenCompetitionV2Wallet.Policy calldata policy,
        bytes32[] calldata approvedCreations,
        bytes32 userSalt,
        uint256 initialFunding
    ) external nonReentrant returns (address wallet) {
        if (initialFunding == 0) revert ReserveFactoryFundingZero();
        bool created;
        (wallet, created) =
            _deploy(msg.sender, policy, approvedCreations, userSalt, FundingMode.Allowance, initialFunding, false);
        if (!created) revert ReserveFactoryWalletAlreadyFundable();
        uint256 beforeBalance = IERC20BountyToken(settlementToken).balanceOf(wallet);
        settlementToken.safeTransferFrom(msg.sender, wallet, initialFunding);
        if (IERC20BountyToken(settlementToken).balanceOf(wallet) != beforeBalance + initialFunding) {
            revert ReserveFactoryFundingMismatch();
        }
    }

    function createWalletWithAuthorization(
        address owner,
        BoundedOpenCompetitionV2Wallet.Policy calldata policy,
        bytes32[] calldata approvedCreations,
        bytes32 userSalt,
        uint256 initialFunding,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 authorizationNonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external nonReentrant returns (address wallet) {
        if (initialFunding == 0) revert ReserveFactoryFundingZero();
        bool created;
        (wallet, created) =
            _deploy(owner, policy, approvedCreations, userSalt, FundingMode.Eip3009, initialFunding, false);
        if (!created) revert ReserveFactoryWalletAlreadyFundable();
        uint256 beforeBalance = IERC20BountyToken(settlementToken).balanceOf(wallet);
        settlementToken.safeTransferWithAuthorization(
            owner, wallet, initialFunding, validAfter, validBefore, authorizationNonce, v, r, s
        );
        if (IERC20BountyToken(settlementToken).balanceOf(wallet) != beforeBalance + initialFunding) {
            revert ReserveFactoryFundingMismatch();
        }
    }

    function predictWallet(address owner, bytes32 userSalt) external view returns (address) {
        return _predictWallet(owner, userSalt);
    }

    function walletInitCodeHash() public view returns (bytes32) {
        return keccak256(
            abi.encodePacked(
                hex"3d602d80600a3d3981f3",
                hex"363d3d373d3d3d363d73",
                bytes20(implementation),
                hex"5af43d82803e903d91602b57fd5bf3"
            )
        );
    }

    function effectiveSalt(address owner, bytes32 userSalt) public pure returns (bytes32) {
        return keccak256(abi.encode(owner, userSalt));
    }

    function _deploy(
        address owner,
        BoundedOpenCompetitionV2Wallet.Policy calldata policy,
        bytes32[] calldata approvedCreations,
        bytes32 userSalt,
        FundingMode fundingMode,
        uint256 initialFunding,
        bool allowExisting
    ) private returns (address wallet, bool created) {
        if (owner == address(0)) revert ReserveFactoryInvalidConfiguration();
        bytes32 initialPolicyHash = _policyHash(policy, approvedCreations);
        bytes32 salt = effectiveSalt(owner, userSalt);
        wallet = _predictWallet(owner, userSalt);
        if (wallet.code.length > 0) {
            if (
                !allowExisting || !isFactoryWallet[wallet]
                    || BoundedOpenCompetitionV2Wallet(payable(wallet)).owner() != owner
                    || BoundedOpenCompetitionV2Wallet(payable(wallet)).initialPolicyHash() != initialPolicyHash
            ) revert ReserveFactoryWalletOccupied();
            return (wallet, false);
        }
        wallet = _cloneDeterministic(implementation, salt);
        isFactoryWallet[wallet] = true;
        BoundedOpenCompetitionV2Wallet(payable(wallet)).initialize(owner, policy, approvedCreations);
        emit BoundedOpenCompetitionV2WalletCreated(
            wallet, owner, policy.delegate, userSalt, salt, initialPolicyHash, fundingMode, initialFunding
        );
        return (wallet, true);
    }

    function _policyHash(BoundedOpenCompetitionV2Wallet.Policy calldata policy, bytes32[] calldata approvedCreations)
        private
        pure
        returns (bytes32)
    {
        return keccak256(abi.encode(policy, approvedCreations));
    }

    function _predictWallet(address owner, bytes32 userSalt) private view returns (address) {
        return address(
            uint160(
                uint256(
                    keccak256(
                        abi.encodePacked(
                            bytes1(0xff),
                            address(this),
                            effectiveSalt(owner, userSalt),
                            walletInitCodeHash()
                        )
                    )
                )
            )
        );
    }

    function _cloneDeterministic(address target, bytes32 salt) private returns (address instance) {
        bytes20 targetBytes = bytes20(target);
        bytes memory creationCode = abi.encodePacked(
            hex"3d602d80600a3d3981f3", hex"363d3d373d3d3d363d73", targetBytes, hex"5af43d82803e903d91602b57fd5bf3"
        );
        assembly ("memory-safe") {
            instance := create2(0, add(creationCode, 0x20), mload(creationCode), salt)
        }
        if (instance == address(0)) revert ReserveFactoryDeploymentFailed();
    }
}
