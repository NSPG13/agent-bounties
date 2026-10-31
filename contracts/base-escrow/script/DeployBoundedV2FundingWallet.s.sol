// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/BoundedV2FundingWallet.sol";

interface FundingWalletDeployVm {
    function envAddress(string calldata name) external returns (address);
    function envUint(string calldata name) external returns (uint256);
    function startBroadcast() external;
    function stopBroadcast() external;
    function serializeAddress(string calldata objectKey, string calldata valueKey, address value)
        external
        returns (string memory);
    function serializeBytes32(string calldata objectKey, string calldata valueKey, bytes32 value)
        external
        returns (string memory);
    function serializeUint(string calldata objectKey, string calldata valueKey, uint256 value)
        external
        returns (string memory);
    function writeJson(string calldata json, string calldata path) external;
}

/// @notice Deployment of the card-bounty funding wallet for the recorded v2 factory. The owner
/// is a parameter, so any funded account may broadcast (`--interactive`, `--account`,
/// `--ledger`); the broadcaster gets no role. This script reads no private key. The only
/// verification setup is the customer-review signer.
contract DeployBoundedV2FundingWallet {
    FundingWalletDeployVm private constant vm =
        FundingWalletDeployVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    address public constant BASE_MAINNET_FACTORY = 0xC33e2Ae33BB9580837Ea59df18E57fA1039aE58A;
    address public constant BASE_SEPOLIA_FACTORY = 0x44DE6074440Fc006c20087991364CE65c87E6d88;
    // Match MANAGED_BOUNTY_QUOTES: 30-day funding window, 7-day claims, 7-day verification
    // (3 days to decide, 2 to appeal, 1 to rule, 1 spare).
    uint64 public constant MAX_FUNDING_WINDOW = 30 days;
    uint64 public constant CLAIM_WINDOW = 7 days;
    uint64 public constant VERIFICATION_WINDOW = 7 days;

    struct Params {
        address owner;
        address delegate;
        address reviewer;
        uint64 validUntil;
        uint256 maxPerBounty;
        uint256 maxPerDay;
        uint256 maxLifetimeSpend;
    }

    function run() external returns (BoundedV2FundingWallet wallet) {
        require(block.chainid == 8_453 || block.chainid == 84_532, "unsupported chain");
        address factory = block.chainid == 8_453 ? BASE_MAINNET_FACTORY : BASE_SEPOLIA_FACTORY;
        Params memory p = Params({
            owner: vm.envAddress("FUNDING_WALLET_OWNER"),
            delegate: vm.envAddress("FUNDING_WALLET_DELEGATE"),
            reviewer: vm.envAddress("CUSTOMER_REVIEW_VERIFIER"),
            validUntil: uint64(vm.envUint("FUNDING_WALLET_VALID_UNTIL")),
            maxPerBounty: vm.envUint("FUNDING_WALLET_MAX_PER_BOUNTY"),
            maxPerDay: vm.envUint("FUNDING_WALLET_MAX_PER_DAY"),
            maxLifetimeSpend: vm.envUint("FUNDING_WALLET_MAX_LIFETIME")
        });
        vm.startBroadcast();
        wallet = deploy(factory, p);
        vm.stopBroadcast();

        string memory key = "bounded-v2-funding-wallet";
        vm.serializeUint(key, "chain_id", block.chainid);
        vm.serializeAddress(key, "factory", factory);
        vm.serializeAddress(key, "wallet", address(wallet));
        vm.serializeAddress(key, "owner", p.owner);
        vm.serializeAddress(key, "delegate", p.delegate);
        vm.serializeAddress(key, "customer_review_verifier", p.reviewer);
        vm.serializeBytes32(key, "runtime_code_hash", address(wallet).codehash);
        string memory json = vm.serializeBytes32(key, "policy_hash", wallet.policyHash());
        vm.writeJson(json, "../../target/bounded-v2-funding-wallet-deployment.json");
    }

    /// @notice Deploys and re-reads every committed field; any drift reverts.
    function deploy(address factory, Params memory p) public returns (BoundedV2FundingWallet wallet) {
        require(factory.code.length > 0, "factory has no code");
        require(p.owner != address(0) && p.delegate != address(0) && p.reviewer != address(0), "zero address");
        require(p.owner != p.delegate && p.owner != p.reviewer && p.delegate != p.reviewer, "roles must differ");
        require(p.validUntil > block.timestamp, "policy already expired");
        require(
            p.maxPerBounty > 0 && p.maxPerBounty <= p.maxPerDay && p.maxPerDay <= p.maxLifetimeSpend,
            "caps must be per bounty <= per day <= lifetime"
        );
        BoundedV2FundingWallet.Policy memory policy = BoundedV2FundingWallet.Policy({
            delegate: p.delegate,
            validAfter: uint64(block.timestamp),
            validUntil: p.validUntil,
            periodSeconds: 1 days,
            maxFundingWindowSeconds: MAX_FUNDING_WINDOW,
            maxClaimWindowSeconds: CLAIM_WINDOW,
            maxVerificationWindowSeconds: VERIFICATION_WINDOW,
            delegateMayCancel: false,
            maxPerBounty: p.maxPerBounty,
            maxPerPeriod: p.maxPerDay,
            maxLifetimeSpend: p.maxLifetimeSpend,
            claimEligibilityRegistry: address(0),
            claimEligibilitySource: bytes32(0)
        });
        address[] memory verifiers = new address[](1);
        verifiers[0] = p.reviewer;
        BoundedV2FundingWallet.VerificationConfig[] memory configs = new BoundedV2FundingWallet.VerificationConfig[](1);
        configs[0] = BoundedV2FundingWallet.VerificationConfig({
            mode: AgentBountyV2.VerificationMode.SignedQuorum,
            verifierModule: address(0),
            verifierRewardRecipient: address(0),
            verifierSetHash: keccak256(abi.encode(verifiers)),
            threshold: 1
        });
        wallet = new BoundedV2FundingWallet(factory, p.owner, policy, configs);

        require(address(wallet.factory()) == factory, "factory drift");
        require(wallet.owner() == p.owner, "owner drift");
        require(wallet.policyHash() == keccak256(abi.encode(policy, configs)), "policy drift");
        BoundedV2FundingWallet.VerificationConfig[] memory stored = wallet.verificationConfigs();
        require(stored.length == 1 && stored[0].verifierSetHash == configs[0].verifierSetHash, "setup drift");
    }
}
