// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./BoundedAgentWallet.sol";

/// @notice Deterministically deploys policy-capped wallets for one canonical bounty factory.
/// @dev The factory is immutable and has no owner or arbitrary-call authority.
contract BoundedAgentWalletFactory {
    using SafeBountyToken for address;

    enum FundingMode {
        None,
        Allowance,
        Eip3009
    }

    AgentBountyFactory public immutable bountyFactory;
    address public immutable settlementToken;
    mapping(address => bool) public isFactoryWallet;
    uint256 private _reentrancy = 1;

    event BoundedAgentWalletCreated(
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

    constructor(address bountyFactory_) {
        require(bountyFactory_.code.length > 0, "factory has no code");
        bountyFactory = AgentBountyFactory(bountyFactory_);
        settlementToken = AgentBountyFactory(bountyFactory_).settlementToken();
        require(settlementToken.code.length > 0, "token has no code");
    }

    function createWallet(address owner, BoundedAgentWallet.Policy calldata policy, bytes32 userSalt)
        external
        nonReentrant
        returns (address wallet)
    {
        wallet = _deploy(owner, policy, userSalt, FundingMode.None, 0);
    }

    /// @notice The owner may atomically deploy and fund after approving this factory.
    function createWalletAndFund(
        address owner,
        BoundedAgentWallet.Policy calldata policy,
        bytes32 userSalt,
        uint256 initialFunding
    ) external nonReentrant returns (address wallet) {
        require(msg.sender == owner, "not owner");
        require(initialFunding > 0, "funding zero");
        wallet = _deploy(owner, policy, userSalt, FundingMode.Allowance, initialFunding);
        settlementToken.safeTransferFrom(owner, wallet, initialFunding);
        require(IERC20BountyToken(settlementToken).balanceOf(wallet) >= initialFunding, "funding not received");
    }

    /// @notice Any relayer may deploy and fund the exact wallet named by a Circle USDC authorization.
    function createWalletWithAuthorization(
        address owner,
        BoundedAgentWallet.Policy calldata policy,
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
        settlementToken.safeTransferWithAuthorization(
            owner, wallet, initialFunding, validAfter, validBefore, authorizationNonce, v, r, s
        );
        require(IERC20BountyToken(settlementToken).balanceOf(wallet) >= initialFunding, "funding not received");
    }

    function predictWallet(address owner, BoundedAgentWallet.Policy calldata policy, bytes32 userSalt)
        external
        view
        returns (address)
    {
        bytes32 initCodeHash = walletInitCodeHash(owner, policy);
        return address(
            uint160(
                uint256(
                    keccak256(
                        abi.encodePacked(bytes1(0xff), address(this), effectiveSalt(owner, userSalt), initCodeHash)
                    )
                )
            )
        );
    }

    function walletInitCodeHash(address owner, BoundedAgentWallet.Policy calldata policy)
        public
        view
        returns (bytes32)
    {
        return keccak256(
            abi.encodePacked(type(BoundedAgentWallet).creationCode, abi.encode(owner, address(bountyFactory), policy))
        );
    }

    function effectiveSalt(address owner, bytes32 userSalt) public pure returns (bytes32) {
        return keccak256(abi.encode(owner, userSalt));
    }

    function _deploy(
        address owner,
        BoundedAgentWallet.Policy calldata policy,
        bytes32 userSalt,
        FundingMode fundingMode,
        uint256 initialFunding
    ) private returns (address wallet) {
        require(owner != address(0), "owner zero");
        bytes32 salt = effectiveSalt(owner, userSalt);
        BoundedAgentWallet deployed = new BoundedAgentWallet{salt: salt}(owner, address(bountyFactory), policy);
        wallet = address(deployed);
        isFactoryWallet[wallet] = true;
        emit BoundedAgentWalletCreated(
            wallet, owner, policy.delegate, userSalt, salt, keccak256(abi.encode(policy)), fundingMode, initialFunding
        );
    }
}
