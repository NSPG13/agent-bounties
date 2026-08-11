// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./OpenCompetitionEntrantWalletV1.sol";

/// @notice Deterministically deploys policy-bound entrant wallets for one frozen competition factory.
/// @dev This contract has no owner, upgrade path, arbitrary-call authority, or gas reimbursement path.
contract OpenCompetitionEntrantWalletFactoryV1 {
    using SafeBountyToken for address;

    enum FundingMode {
        None,
        Allowance,
        Eip3009
    }

    OpenCompetitionBountyFactoryV1 public immutable competitionFactory;
    address public immutable settlementToken;
    address public immutable implementation;
    mapping(address => bool) public isFactoryWallet;
    uint256 private _reentrancy = 1;

    event OpenCompetitionEntrantWalletCreated(
        address indexed wallet,
        address indexed owner,
        address indexed delegate,
        bytes32 userSalt,
        bytes32 effectiveSalt,
        bytes32 policyHash,
        FundingMode fundingMode,
        uint256 initialFunding
    );

    modifier nonReentrant() {
        require(_reentrancy == 1, "reentrant");
        _reentrancy = 2;
        _;
        _reentrancy = 1;
    }

    constructor(address competitionFactory_) {
        require(competitionFactory_.code.length > 0, "factory has no code");
        competitionFactory = OpenCompetitionBountyFactoryV1(competitionFactory_);
        settlementToken = OpenCompetitionBountyFactoryV1(competitionFactory_).settlementToken();
        require(settlementToken.code.length > 0, "token has no code");
        implementation = address(new OpenCompetitionEntrantWalletV1(competitionFactory_));
    }

    function createWallet(
        address owner,
        OpenCompetitionEntrantWalletV1.Policy calldata policy,
        bytes32 userSalt
    ) external nonReentrant returns (address wallet) {
        wallet = _deploy(owner, policy, userSalt, FundingMode.None, 0);
    }

    function createWalletAndFund(
        OpenCompetitionEntrantWalletV1.Policy calldata policy,
        bytes32 userSalt,
        uint256 initialFunding
    ) external nonReentrant returns (address wallet) {
        require(initialFunding > 0, "funding zero");
        wallet = _deploy(msg.sender, policy, userSalt, FundingMode.Allowance, initialFunding);
        uint256 balanceBefore = IERC20BountyToken(settlementToken).balanceOf(wallet);
        settlementToken.safeTransferFrom(msg.sender, wallet, initialFunding);
        require(
            IERC20BountyToken(settlementToken).balanceOf(wallet) == balanceBefore + initialFunding,
            "funding not received"
        );
    }

    /// @notice Any keeper may deploy and fund the exact wallet named by a Circle USDC authorization.
    function createWalletWithAuthorization(
        address owner,
        OpenCompetitionEntrantWalletV1.Policy calldata policy,
        bytes32 userSalt,
        uint256 initialFunding,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 authorizationNonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external nonReentrant returns (address wallet) {
        require(initialFunding > 0, "funding zero");
        wallet = _deploy(owner, policy, userSalt, FundingMode.Eip3009, initialFunding);
        uint256 balanceBefore = IERC20BountyToken(settlementToken).balanceOf(wallet);
        settlementToken.safeTransferWithAuthorization(
            owner, wallet, initialFunding, validAfter, validBefore, authorizationNonce, v, r, s
        );
        require(
            IERC20BountyToken(settlementToken).balanceOf(wallet) == balanceBefore + initialFunding,
            "funding not received"
        );
    }

    function predictWallet(
        address owner,
        OpenCompetitionEntrantWalletV1.Policy calldata policy,
        bytes32 userSalt
    ) external view returns (address) {
        return _predictWallet(owner, userSalt, keccak256(abi.encode(policy)));
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

    function effectiveSalt(address owner, bytes32 userSalt, bytes32 policyHash_) public pure returns (bytes32) {
        return keccak256(abi.encode(owner, userSalt, policyHash_));
    }

    function _deploy(
        address owner,
        OpenCompetitionEntrantWalletV1.Policy calldata policy,
        bytes32 userSalt,
        FundingMode fundingMode,
        uint256 initialFunding
    ) private returns (address wallet) {
        require(owner != address(0), "owner zero");
        require(userSalt != bytes32(0), "user salt zero");
        bytes32 policyHash_ = keccak256(abi.encode(policy));
        bytes32 salt = effectiveSalt(owner, userSalt, policyHash_);
        wallet = _predictWallet(owner, userSalt, policyHash_);
        if (wallet.code.length > 0) {
            require(isFactoryWallet[wallet], "wallet address occupied");
            OpenCompetitionEntrantWalletV1 existing = OpenCompetitionEntrantWalletV1(payable(wallet));
            require(existing.owner() == owner, "wallet owner changed");
            require(existing.policyHash() == policyHash_, "wallet policy changed");
            return wallet;
        }
        wallet = _cloneDeterministic(implementation, salt);
        isFactoryWallet[wallet] = true;
        OpenCompetitionEntrantWalletV1(payable(wallet)).initialize(owner, policy);
        emit OpenCompetitionEntrantWalletCreated(
            wallet, owner, policy.delegate, userSalt, salt, policyHash_, fundingMode, initialFunding
        );
    }

    function _predictWallet(address owner, bytes32 userSalt, bytes32 policyHash_) private view returns (address) {
        return address(
            uint160(
                uint256(
                    keccak256(
                        abi.encodePacked(
                            bytes1(0xff),
                            address(this),
                            effectiveSalt(owner, userSalt, policyHash_),
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
        require(instance != address(0), "wallet deployment failed");
    }
}
