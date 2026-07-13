// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/BoundedAgentWalletFactory.sol";

contract WalletFactoryToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        _transfer(msg.sender, to, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(allowance[from][msg.sender] >= amount, "allowance");
        allowance[from][msg.sender] -= amount;
        _transfer(from, to, amount);
        return true;
    }

    function transferWithAuthorization(
        address from,
        address to,
        uint256 amount,
        uint256,
        uint256,
        bytes32,
        uint8,
        bytes32,
        bytes32
    ) external {
        _transfer(from, to, amount);
    }

    function _transfer(address from, address to, uint256 amount) private {
        require(balanceOf[from] >= amount, "balance");
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
    }
}

contract WalletFactoryCaller {
    function createAndFund(
        BoundedAgentWalletFactory factory,
        address owner,
        BoundedAgentWallet.Policy calldata policy,
        bytes32 salt,
        uint256 amount
    ) external returns (address) {
        return factory.createWalletAndFund(owner, policy, salt, amount);
    }
}

contract BoundedAgentWalletFactoryTest {
    WalletFactoryToken private token;
    AgentBountyFactory private bountyFactory;
    BoundedAgentWalletFactory private walletFactory;
    WalletFactoryCaller private caller;

    function setUp() public {
        token = new WalletFactoryToken();
        bountyFactory = new AgentBountyFactory(address(token));
        walletFactory = new BoundedAgentWalletFactory(address(bountyFactory));
        caller = new WalletFactoryCaller();
        token.mint(address(this), 10_000);
    }

    function testPredictionMatchesDeploymentAndPinsCanonicalContracts() public {
        BoundedAgentWallet.Policy memory policy = _policy(address(0xD311));
        bytes32 salt = keccak256("predict");
        address predicted = walletFactory.predictWallet(address(this), policy, salt);
        address deployed = walletFactory.createWallet(address(this), policy, salt);

        require(deployed == predicted, "prediction mismatch");
        require(walletFactory.isFactoryWallet(deployed), "wallet not registered");
        require(address(BoundedAgentWallet(payable(deployed)).factory()) == address(bountyFactory), "factory drift");
        require(BoundedAgentWallet(payable(deployed)).settlementToken() == address(token), "token drift");
        require(BoundedAgentWallet(payable(deployed)).owner() == address(this), "owner drift");
    }

    function testOwnerCanDeployAndFundAtomically() public {
        BoundedAgentWallet.Policy memory policy = _policy(address(0xD311));
        token.approve(address(walletFactory), 500);
        address wallet = walletFactory.createWalletAndFund(address(this), policy, keccak256("allowance"), 500);
        require(token.balanceOf(wallet) == 500, "wallet not funded");
    }

    function testAllowanceFundingCannotBeTriggeredByAnotherCaller() public {
        BoundedAgentWallet.Policy memory policy = _policy(address(0xD311));
        token.approve(address(walletFactory), 500);
        (bool ok,) = address(caller)
            .call(
                abi.encodeCall(
                    caller.createAndFund,
                    (walletFactory, address(this), policy, keccak256("unauthorized"), uint256(500))
                )
            );
        require(!ok, "nonowner pulled approved funds");
        require(token.balanceOf(address(this)) == 10_000, "owner funds moved");
    }

    function testAuthorizationRelayerFundsOnlyPredictedWallet() public {
        BoundedAgentWallet.Policy memory policy = _policy(address(0xD311));
        bytes32 salt = keccak256("authorization");
        address predicted = walletFactory.predictWallet(address(this), policy, salt);
        address wallet = walletFactory.createWalletWithAuthorization(
            address(this), policy, salt, 750, 0, type(uint256).max, keccak256("auth-nonce"), 27, bytes32(0), bytes32(0)
        );
        require(wallet == predicted, "authorization destination drift");
        require(token.balanceOf(wallet) == 750, "authorization funding missing");
    }

    function testDuplicateDeploymentRevertsWithoutMovingFunds() public {
        BoundedAgentWallet.Policy memory policy = _policy(address(0xD311));
        bytes32 salt = keccak256("duplicate");
        walletFactory.createWallet(address(this), policy, salt);
        uint256 beforeBalance = token.balanceOf(address(this));
        (bool ok,) = address(walletFactory)
            .call(
                abi.encodeCall(
                    walletFactory.createWalletWithAuthorization,
                    (
                        address(this),
                        policy,
                        salt,
                        uint256(100),
                        uint256(0),
                        type(uint256).max,
                        keccak256("duplicate-auth"),
                        uint8(27),
                        bytes32(0),
                        bytes32(0)
                    )
                )
            );
        require(!ok, "duplicate wallet deployed");
        require(token.balanceOf(address(this)) == beforeBalance, "duplicate moved funds");
    }

    function _policy(address delegate) private view returns (BoundedAgentWallet.Policy memory) {
        return BoundedAgentWallet.Policy({
            delegate: delegate,
            validAfter: uint64(block.timestamp),
            validUntil: uint64(block.timestamp + 30 days),
            periodSeconds: 1 days,
            maxPerAction: 1_000,
            maxPerPeriod: 2_000,
            maxLifetimeSpend: 5_000,
            allowedActions: 15,
            allowedVerificationModes: 1
        });
    }
}
