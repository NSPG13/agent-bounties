// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";

interface ICanonicalBountyFactory {
    function isCanonicalBounty(address bounty) external view returns (bool);
    function settlementToken() external view returns (address);
}

interface ISponsoredAgentBountyV1 {
    function protocolVersion() external view returns (bytes32);
    function creator() external view returns (address);
    function settlementToken() external view returns (address);
    function termsHash() external view returns (bytes32);
    function policyHash() external view returns (bytes32);
    function status() external view returns (uint8);
    function round() external view returns (uint64);
    function solver() external view returns (address);
    function verifierReward() external view returns (uint256);
    function activeClaimBond() external view returns (uint256);
    function claimWithAuthorization(
        address solver,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external;
}

/// @notice Gives a new solver its first bond and consumes it in one transaction.
/// @dev The deployed v1 bounty returns a successful bond to the solver. A lifetime
/// cap makes that deliberate acquisition spend: one successful grant lets the
/// solver self-fund later claims without another subsidy.
contract AtomicClaimSponsor {
    using SafeBountyToken for address;

    bytes32 public constant SUPPORTED_PROTOCOL_VERSION = keccak256("agent-bounties/autonomous-v1");
    uint8 private constant CLAIMABLE_STATUS = 1;
    uint8 private constant CLAIMED_STATUS = 2;
    uint256 private constant MAX_AUTHORIZATION_WINDOW = 1 hours;
    uint256 private constant MAX_GRANT_WINDOW = 30 minutes;
    bytes4 private constant ERC1271_MAGIC_VALUE = 0x1626ba7e;
    uint256 private constant ERC1271_GAS_LIMIT = 200_000;
    uint256 private constant SECP256K1N_DIV_2 = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;
    bytes32 private constant EIP712_DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant NAME_HASH = keccak256("Agent Bounties Atomic Claim Sponsor");
    bytes32 private constant VERSION_HASH = keccak256("1");
    bytes32 private constant GRANT_TYPEHASH = keccak256(
        "SponsoredClaim(address sponsor,address factory,address bounty,address solver,uint64 round,uint256 bond,bytes32 termsHash,bytes32 policyHash,bytes32 authorizationNonce,uint256 validAfter,uint256 validBefore,bytes32 grantNonce,uint256 deadline)"
    );

    struct Grant {
        address bounty;
        address solver;
        uint64 round;
        uint256 bond;
        bytes32 termsHash;
        bytes32 policyHash;
        bytes32 authorizationNonce;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 grantNonce;
        uint256 deadline;
    }

    address public immutable settlementToken;
    address public immutable canonicalFactory;
    uint256 public immutable maxBond;
    uint256 public immutable maxNetworkPerDay;
    uint256 public immutable maxLifetimePerSolver;

    address public owner;
    address public pendingOwner;
    address public grantSigner;
    bool public paused;
    uint256 private _reentrancy = 1;

    mapping(bytes32 => bool) public grantNonceUsed;
    mapping(uint256 => uint256) public sponsoredByDay;
    mapping(address => uint256) public lifetimeSponsored;

    event SponsoredClaim(
        address indexed bounty,
        address indexed solver,
        uint64 indexed round,
        uint256 bond,
        bytes32 grantNonce,
        bytes32 authorizationNonce
    );
    event GrantSignerUpdated(address indexed previousSigner, address indexed newSigner);
    event PauseUpdated(bool paused);
    event OwnershipTransferStarted(address indexed owner, address indexed pendingOwner);
    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    event Deposited(address indexed contributor, uint256 amount);
    event Withdrawn(address indexed recipient, uint256 amount);

    error NotOwner();
    error ReentrantCall();
    error SponsorshipPaused();
    error InvalidConfiguration();
    error InvalidGrant();
    error InvalidGrantSignature();
    error GrantReplay();
    error UnsupportedBounty();
    error BountyNotClaimable();
    error SolverIneligible();
    error SponsorshipCapExceeded();
    error InsufficientSponsorBalance();
    error AtomicClaimInvariant();

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    modifier nonReentrant() {
        if (_reentrancy != 1) revert ReentrantCall();
        _reentrancy = 2;
        _;
        _reentrancy = 1;
    }

    constructor(
        address settlementToken_,
        address canonicalFactory_,
        address grantSigner_,
        uint256 maxBond_,
        uint256 maxNetworkPerDay_,
        uint256 maxLifetimePerSolver_
    ) {
        if (
            settlementToken_ == address(0) || canonicalFactory_ == address(0) || grantSigner_ == address(0)
                || settlementToken_.code.length == 0 || canonicalFactory_.code.length == 0 || maxBond_ == 0
                || maxLifetimePerSolver_ < maxBond_ || maxNetworkPerDay_ < maxLifetimePerSolver_
        ) revert InvalidConfiguration();
        if (ICanonicalBountyFactory(canonicalFactory_).settlementToken() != settlementToken_) {
            revert InvalidConfiguration();
        }
        settlementToken = settlementToken_;
        canonicalFactory = canonicalFactory_;
        grantSigner = grantSigner_;
        maxBond = maxBond_;
        maxNetworkPerDay = maxNetworkPerDay_;
        maxLifetimePerSolver = maxLifetimePerSolver_;
        owner = msg.sender;
    }

    function currentDay() public view returns (uint256) {
        return block.timestamp / 1 days;
    }

    function remainingDailyBudget() external view returns (uint256) {
        return maxNetworkPerDay - sponsoredByDay[currentDay()];
    }

    function grantDigest(Grant calldata grant) public view returns (bytes32) {
        bytes32 structHash = keccak256(
            abi.encode(
                GRANT_TYPEHASH,
                address(this),
                canonicalFactory,
                grant.bounty,
                grant.solver,
                grant.round,
                grant.bond,
                grant.termsHash,
                grant.policyHash,
                grant.authorizationNonce,
                grant.validAfter,
                grant.validBefore,
                grant.grantNonce,
                grant.deadline
            )
        );
        bytes32 domainSeparator =
            keccak256(abi.encode(EIP712_DOMAIN_TYPEHASH, NAME_HASH, VERSION_HASH, block.chainid, address(this)));
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator, structHash));
    }

    /// @notice Funds and consumes the exact solver bond atomically. The caller is
    /// an untrusted relayer; the policy grant and solver's USDC authorization
    /// provide authority.
    function sponsorAndClaim(
        Grant calldata grant,
        bytes calldata grantSignature,
        uint8 authorizationV,
        bytes32 authorizationR,
        bytes32 authorizationS
    ) external nonReentrant {
        if (paused) revert SponsorshipPaused();
        _validateGrant(grant, grantSignature);

        ISponsoredAgentBountyV1 bounty = ISponsoredAgentBountyV1(grant.bounty);
        _validateBounty(bounty, grant);

        uint256 sponsorBalanceBefore = IERC20BountyToken(settlementToken).balanceOf(address(this));
        if (sponsorBalanceBefore < grant.bond) revert InsufficientSponsorBalance();

        _consumeGrant(grant);

        settlementToken.safeTransfer(grant.solver, grant.bond);
        bounty.claimWithAuthorization(
            grant.solver,
            grant.validAfter,
            grant.validBefore,
            grant.authorizationNonce,
            authorizationV,
            authorizationR,
            authorizationS
        );

        _assertClaim(bounty, grant, sponsorBalanceBefore);
        _emitSponsoredClaim(grant);
    }

    function deposit(uint256 amount) external nonReentrant {
        if (amount == 0) revert InvalidGrant();
        settlementToken.safeTransferFrom(msg.sender, address(this), amount);
        emit Deposited(msg.sender, amount);
    }

    function withdraw(address recipient, uint256 amount) external onlyOwner nonReentrant {
        if (!paused || recipient == address(0) || amount == 0) revert InvalidGrant();
        settlementToken.safeTransfer(recipient, amount);
        emit Withdrawn(recipient, amount);
    }

    function setGrantSigner(address newSigner) external onlyOwner {
        if (newSigner == address(0)) revert InvalidConfiguration();
        address previousSigner = grantSigner;
        grantSigner = newSigner;
        emit GrantSignerUpdated(previousSigner, newSigner);
    }

    function setPaused(bool paused_) external onlyOwner {
        paused = paused_;
        emit PauseUpdated(paused_);
    }

    function transferOwnership(address newOwner) external onlyOwner {
        if (newOwner == address(0)) revert InvalidConfiguration();
        pendingOwner = newOwner;
        emit OwnershipTransferStarted(owner, newOwner);
    }

    function acceptOwnership() external {
        if (msg.sender != pendingOwner) revert NotOwner();
        address previousOwner = owner;
        owner = msg.sender;
        pendingOwner = address(0);
        emit OwnershipTransferred(previousOwner, msg.sender);
    }

    function _validateGrant(Grant calldata grant, bytes calldata grantSignature) private view {
        if (
            grant.bounty == address(0) || grant.solver == address(0) || grant.round == 0 || grant.bond == 0
                || grant.bond > maxBond || grant.termsHash == bytes32(0) || grant.policyHash == bytes32(0)
                || grant.authorizationNonce == bytes32(0) || grant.grantNonce == bytes32(0)
                || block.timestamp <= grant.validAfter || block.timestamp >= grant.validBefore
                || block.timestamp > grant.deadline || grant.deadline > grant.validBefore
                || grant.validBefore > block.timestamp + MAX_AUTHORIZATION_WINDOW
                || grant.deadline > block.timestamp + MAX_GRANT_WINDOW
        ) revert InvalidGrant();
        if (grantNonceUsed[grant.grantNonce]) revert GrantReplay();
        if (!_isValidSignatureNow(grantSigner, grantDigest(grant), grantSignature)) {
            revert InvalidGrantSignature();
        }
    }

    function _validateBounty(ISponsoredAgentBountyV1 bounty, Grant calldata grant) private view {
        if (
            !ICanonicalBountyFactory(canonicalFactory).isCanonicalBounty(grant.bounty)
                || bounty.protocolVersion() != SUPPORTED_PROTOCOL_VERSION || bounty.settlementToken() != settlementToken
        ) revert UnsupportedBounty();
        if (bounty.status() != CLAIMABLE_STATUS || bounty.round() + 1 != grant.round) revert BountyNotClaimable();
        if (bounty.creator() == grant.solver) revert SolverIneligible();
        if (
            bounty.verifierReward() != grant.bond || bounty.termsHash() != grant.termsHash
                || bounty.policyHash() != grant.policyHash
        ) revert InvalidGrant();
    }

    function _consumeGrant(Grant calldata grant) private {
        if (lifetimeSponsored[grant.solver] != 0) revert SponsorshipCapExceeded();
        uint256 day = currentDay();
        uint256 dayTotal = sponsoredByDay[day] + grant.bond;
        uint256 solverTotal = lifetimeSponsored[grant.solver] + grant.bond;
        if (dayTotal > maxNetworkPerDay || solverTotal > maxLifetimePerSolver) {
            revert SponsorshipCapExceeded();
        }
        grantNonceUsed[grant.grantNonce] = true;
        sponsoredByDay[day] = dayTotal;
        lifetimeSponsored[grant.solver] = solverTotal;
    }

    function _assertClaim(ISponsoredAgentBountyV1 bounty, Grant calldata grant, uint256 sponsorBalanceBefore)
        private
        view
    {
        uint256 sponsorBalanceAfter = IERC20BountyToken(settlementToken).balanceOf(address(this));
        if (
            sponsorBalanceAfter > sponsorBalanceBefore || sponsorBalanceBefore - sponsorBalanceAfter != grant.bond
                || bounty.status() != CLAIMED_STATUS || bounty.round() != grant.round || bounty.solver() != grant.solver
                || bounty.activeClaimBond() != grant.bond
        ) revert AtomicClaimInvariant();
    }

    function _emitSponsoredClaim(Grant calldata grant) private {
        emit SponsoredClaim(
            grant.bounty, grant.solver, grant.round, grant.bond, grant.grantNonce, grant.authorizationNonce
        );
    }

    function _isValidSignatureNow(address signer, bytes32 digest, bytes memory signature) private view returns (bool) {
        if (signer.code.length > 0) {
            bytes memory callData = abi.encodeCall(IERC1271.isValidSignature, (digest, signature));
            bool ok;
            bytes4 result;
            uint256 gasLimit = ERC1271_GAS_LIMIT;
            assembly ("memory-safe") {
                let output := mload(0x40)
                mstore(output, 0)
                ok := staticcall(gasLimit, signer, add(callData, 0x20), mload(callData), output, 0x20)
                result := mload(output)
            }
            return ok && result == ERC1271_MAGIC_VALUE;
        }
        return _recover(digest, signature) == signer;
    }

    function _recover(bytes32 digest, bytes memory signature) private pure returns (address recovered) {
        if (signature.length != 65) return address(0);
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly ("memory-safe") {
            r := mload(add(signature, 0x20))
            s := mload(add(signature, 0x40))
            v := byte(0, mload(add(signature, 0x60)))
        }
        if (uint256(s) > SECP256K1N_DIV_2 || (v != 27 && v != 28)) return address(0);
        return ecrecover(digest, v, r, s);
    }
}
