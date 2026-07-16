// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/BoundedAgentWalletFactory.sol";
import "../src/LeadingZeroWorkVerifier.sol";

interface VmBoundedWalletFork {
    function addr(uint256 privateKey) external returns (address keyAddr);
    function createSelectFork(string calldata urlOrAlias, uint256 blockNumber) external returns (uint256 forkId);
    function envOr(string calldata name, bool defaultValue) external returns (bool value);
    function envString(string calldata name) external returns (string memory value);
    function prank(address sender) external;
    function sign(uint256 privateKey, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
    function skip(bool skipTest) external;
}

interface BoundedWalletForkUsdc {
    function balanceOf(address account) external view returns (uint256);
    function name() external view returns (string memory);
    function version() external view returns (string memory);
    function transfer(address recipient, uint256 amount) external returns (bool);
}

/// @notice Opt-in rehearsals against the actual Base USDC and canonical bounty contracts.
/// Fork mutations never broadcast. Set RUN_MAINNET_FORK or RUN_SEPOLIA_FORK with its RPC URL.
contract BoundedAgentWalletForkTest {
    VmBoundedWalletFork private constant vm =
        VmBoundedWalletFork(address(uint160(uint256(keccak256("hevm cheat code")))));

    address private constant MAINNET_FACTORY = 0x082C52131aaF0C56e76b075f895EAB6fcaB6d2F9;
    address private constant MAINNET_USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
    address private constant MAINNET_MODULE = 0xcc6059cEedA5bc4ba8a97ecFbFFa7488C8FD579E;
    address private constant MAINNET_FUNDING_SOURCE = 0x884834E884d6e93462655A2820140aD03E6747bC;
    address private constant SEPOLIA_USDC = 0x036CbD53842c5426634e7929541eC2318f3dCF7e;
    address private constant SEPOLIA_FUNDING_SOURCE = 0x74E1608EC3E5F8B6B3f57D22301a11A5b9Fb736D;
    address private constant VERIFIER_RECIPIENT = 0x000000000000000000000000000000000000bEEF;
    address private constant RELAYER = 0x000000000000000000000000000000000000a11c;
    uint256 private constant MAINNET_FORK_BLOCK = 48_567_240;
    uint256 private constant SEPOLIA_FORK_BLOCK = 44_207_324;
    uint256 private constant OWNER_KEY = uint256(keccak256("bounded-wallet/fork/owner"));
    uint256 private constant DELEGATE_KEY = uint256(keccak256("bounded-wallet/fork/delegate"));
    uint256 private constant ONE_USDC = 1_000_000;

    bytes32 private constant EIP712_DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant TRANSFER_WITH_AUTHORIZATION_TYPEHASH = keccak256(
        "TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)"
    );

    struct AuthorizationInput {
        address from;
        address to;
        uint256 value;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
    }

    function testMainnetOneSignatureFundingAndBoundedCreate() public {
        if (!vm.envOr("RUN_MAINNET_FORK", false)) {
            vm.skip(true);
            return;
        }
        vm.createSelectFork(vm.envString("BASE_MAINNET_RPC_URL"), MAINNET_FORK_BLOCK);
        require(block.chainid == 8453, "wrong chain");
        _runRehearsal(MAINNET_FACTORY, MAINNET_USDC, MAINNET_MODULE, MAINNET_FUNDING_SOURCE);
    }

    function testBaseSepoliaOneSignatureFundingAndBoundedCreate() public {
        if (!vm.envOr("RUN_SEPOLIA_FORK", false)) {
            vm.skip(true);
            return;
        }
        vm.createSelectFork(vm.envString("BASE_SEPOLIA_RPC_URL"), SEPOLIA_FORK_BLOCK);
        require(block.chainid == 84532, "wrong chain");
        AgentBountyFactory factory = new AgentBountyFactory(SEPOLIA_USDC);
        LeadingZeroWorkVerifier module = new LeadingZeroWorkVerifier(16);
        _runRehearsal(address(factory), SEPOLIA_USDC, address(module), SEPOLIA_FUNDING_SOURCE);
    }

    function _runRehearsal(
        address bountyFactory,
        address settlementToken,
        address verifierModule,
        address fundingSource
    ) private {
        require(bountyFactory.code.length > 0, "bounty factory unavailable");
        require(verifierModule.code.length > 0, "verifier unavailable");
        BoundedWalletForkUsdc usdc = BoundedWalletForkUsdc(settlementToken);
        BoundedAgentWalletFactory walletFactory = new BoundedAgentWalletFactory(bountyFactory);
        (address walletAddress, address delegate) =
            _deployFundedWallet(walletFactory, usdc, verifierModule, fundingSource);
        _createFundedBounty(walletAddress, delegate, verifierModule);
    }

    function _deployFundedWallet(
        BoundedAgentWalletFactory walletFactory,
        BoundedWalletForkUsdc usdc,
        address verifierModule,
        address fundingSource
    ) private returns (address walletAddress, address delegate) {
        address owner = vm.addr(OWNER_KEY);
        delegate = vm.addr(DELEGATE_KEY);
        bytes32 salt = keccak256("bounded-wallet/fork/v1");
        BoundedAgentWallet.Policy memory policy = _policy(delegate, verifierModule);
        address predicted = walletFactory.predictWallet(owner, policy, salt);

        uint256 ownerBefore = usdc.balanceOf(owner);
        vm.prank(fundingSource);
        require(usdc.transfer(owner, ONE_USDC), "owner funding failed");

        AuthorizationInput memory authorization = AuthorizationInput({
            from: owner,
            to: predicted,
            value: ONE_USDC,
            validAfter: block.timestamp - 1,
            validBefore: block.timestamp + 1 hours,
            nonce: keccak256("bounded-wallet/fork/funding/v1")
        });
        bytes32 digest = _authorizationDigest(usdc, walletFactory.settlementToken(), authorization);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(OWNER_KEY, digest);
        vm.prank(RELAYER);
        walletAddress = walletFactory.createWalletWithAuthorization(
            owner,
            policy,
            salt,
            ONE_USDC,
            authorization.validAfter,
            authorization.validBefore,
            authorization.nonce,
            v,
            r,
            s
        );

        require(walletAddress == predicted, "wallet prediction drift");
        require(usdc.balanceOf(owner) == ownerBefore, "authorization left owner funds");
        require(usdc.balanceOf(walletAddress) == ONE_USDC, "wallet funding missing");
    }

    function _createFundedBounty(address walletAddress, address delegate, address verifierModule) private {
        BoundedAgentWallet wallet = BoundedAgentWallet(payable(walletAddress));
        AgentBountyFactory.CreateBountyParams memory params = AgentBountyFactory.CreateBountyParams({
            solverReward: 990_000,
            verifierReward: 10_000,
            termsHash: keccak256("bounded-wallet-fork-terms"),
            policyHash: keccak256("bounded-wallet-fork-policy"),
            acceptanceCriteriaHash: keccak256("bounded-wallet-fork-criteria"),
            benchmarkHash: keccak256("bounded-wallet-fork-benchmark"),
            evidenceSchemaHash: keccak256("bounded-wallet-fork-evidence"),
            fundingDeadline: uint64(block.timestamp + 1 days),
            claimWindowSeconds: 1 hours,
            verificationWindowSeconds: 1 hours,
            verificationMode: AgentBounty.VerificationMode.DeterministicModule,
            verifierModule: verifierModule,
            verifierRewardRecipient: VERIFIER_RECIPIENT,
            threshold: 1
        });
        vm.prank(delegate);
        (address bountyAddress,) =
            wallet.createBounty(params, new address[](0), ONE_USDC, keccak256("bounded-wallet-fork-bounty"));
        AgentBounty bounty = AgentBounty(bountyAddress);
        require(bounty.creator() == walletAddress, "wallet not creator");
        require(bounty.fundedAmount() == ONE_USDC, "bounty not funded");
        require(wallet.lifetimeSpent() == ONE_USDC, "gross spend not recorded");
    }

    function _policy(address delegate, address verifierModule) private view returns (BoundedAgentWallet.Policy memory) {
        return BoundedAgentWallet.Policy({
            delegate: delegate,
            validAfter: uint64(block.timestamp),
            validUntil: uint64(block.timestamp + 30 days),
            periodSeconds: 1 days,
            maxPerAction: 5_000_000,
            maxPerPeriod: 10_000_000,
            maxLifetimeSpend: 89_000_000,
            maxBountyTarget: 5_000_000,
            allowedActions: 15,
            allowedVerificationModes: 1,
            deterministicVerifierModule: verifierModule,
            signedQuorumVerifierSetHash: bytes32(0),
            aiJudgeVerifierSetHash: bytes32(0)
        });
    }

    function _authorizationDigest(
        BoundedWalletForkUsdc usdc,
        address settlementToken,
        AuthorizationInput memory authorization
    ) private view returns (bytes32) {
        bytes32 domainSeparator = keccak256(
            abi.encode(
                EIP712_DOMAIN_TYPEHASH,
                keccak256(bytes(usdc.name())),
                keccak256(bytes(usdc.version())),
                block.chainid,
                settlementToken
            )
        );
        bytes32 structHash = keccak256(
            abi.encode(
                TRANSFER_WITH_AUTHORIZATION_TYPEHASH,
                authorization.from,
                authorization.to,
                authorization.value,
                authorization.validAfter,
                authorization.validBefore,
                authorization.nonce
            )
        );
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator, structHash));
    }
}
