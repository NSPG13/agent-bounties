// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";

/// @notice A single, immutable-policy digital bounty with autonomous settlement.
contract AgentBounty is IAgentBountyV1 {
    using SafeBountyToken for address;

    bytes32 public constant PROTOCOL_VERSION = keccak256("agent-bounties/autonomous-v1");
    bytes4 private constant ERC1271_MAGIC_VALUE = 0x1626ba7e;
    uint256 private constant ERC1271_GAS_LIMIT = 200_000;
    bytes32 private constant EIP712_DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant NAME_HASH = keccak256("Agent Bounties");
    bytes32 private constant VERSION_HASH = keccak256("1");
    bytes32 private constant CLAIM_TYPEHASH = keccak256(
        "Claim(address bounty,bytes32 bountyId,address solver,uint64 round,bytes32 termsHash,bytes32 policyHash,uint256 deadline)"
    );
    bytes32 private constant SUBMIT_TYPEHASH = keccak256(
        "Submit(address bounty,bytes32 bountyId,address solver,uint64 round,bytes32 submissionHash,bytes32 evidenceHash,bytes32 policyHash,uint256 deadline)"
    );
    bytes32 private constant ATTESTATION_TYPEHASH = keccak256(
        "VerificationAttestation(address bounty,bytes32 bountyId,uint64 round,address verifier,bytes32 submissionHash,bytes32 evidenceHash,bytes32 policyHash,bool passed,bytes32 responseHash,uint256 deadline)"
    );
    uint256 private constant SECP256K1N_DIV_2 = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;

    enum VerificationMode {
        DeterministicModule,
        SignedQuorum,
        AiJudgeQuorum
    }

    enum BountyStatus {
        Open,
        Claimable,
        Claimed,
        Submitted,
        Settled,
        Cancelled
    }

    struct Config {
        bytes32 bountyId;
        address creator;
        address factory;
        address settlementToken;
        uint256 solverReward;
        uint256 verifierReward;
        bytes32 termsHash;
        bytes32 policyHash;
        bytes32 acceptanceCriteriaHash;
        bytes32 benchmarkHash;
        bytes32 evidenceSchemaHash;
        uint64 fundingDeadline;
        uint64 claimWindowSeconds;
        uint64 verificationWindowSeconds;
        VerificationMode verificationMode;
        address verifierModule;
        address verifierRewardRecipient;
        uint8 threshold;
    }

    struct Attestation {
        address verifier;
        bool passed;
        bytes32 responseHash;
        uint256 deadline;
        bytes signature;
    }

    bytes32 public override bountyId;
    address public override creator;
    address public factory;
    address public override settlementToken;
    uint256 public solverReward;
    uint256 public verifierReward;
    uint256 public override targetAmount;
    bytes32 public override termsHash;
    bytes32 public override policyHash;
    bytes32 public override acceptanceCriteriaHash;
    bytes32 public override benchmarkHash;
    bytes32 public override evidenceSchemaHash;
    bytes32 public override verifierSetHash;
    uint64 public fundingDeadline;
    uint64 public claimWindowSeconds;
    uint64 public verificationWindowSeconds;
    VerificationMode public verificationMode;
    address public verifierModule;
    address public verifierRewardRecipient;
    uint8 public threshold;

    uint256 public override fundedAmount;
    BountyStatus private _status;
    uint64 public round;
    uint64 public claimExpiresAt;
    uint64 public verificationExpiresAt;
    address public solver;
    uint256 public activeClaimBond;
    uint256 public timeoutBondPool;
    uint256 public refundBonusPool;
    uint256 public refundBonusRemaining;
    uint256 public refundPrincipalTotal;
    bytes32 public submissionHash;
    bytes32 public evidenceHash;

    mapping(address => uint256) public contributions;
    mapping(address => bool) public isVerifier;
    address[] private _verifiers;
    uint256 private _reentrancy = 1;
    bool private _initialized;

    event FundingAdded(
        bytes32 indexed bountyId,
        address indexed contributor,
        uint256 amount,
        uint256 fundedAmount,
        uint256 targetAmount
    );
    event BountyBecameClaimable(bytes32 indexed bountyId, uint256 fundedAmount);
    event BountyClaimed(
        bytes32 indexed bountyId,
        uint64 indexed round,
        address indexed solver,
        bytes32 termsHash,
        bytes32 policyHash,
        uint256 claimBond,
        uint64 claimExpiresAt
    );
    event SubmissionAdded(
        bytes32 indexed bountyId,
        uint64 indexed round,
        address indexed solver,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        uint64 verificationExpiresAt
    );
    event SubmissionRejected(
        bytes32 indexed bountyId,
        uint64 indexed round,
        address indexed solver,
        uint256 verifierReward,
        uint256 claimBondForfeited,
        bytes32 verificationHash
    );
    event BountySettled(
        bytes32 indexed bountyId,
        uint64 indexed round,
        address indexed solver,
        uint256 solverReward,
        uint256 claimBondReturned,
        uint256 timeoutBondBonus,
        uint256 verifierReward,
        bytes32 submissionHash,
        bytes32 evidenceHash,
        bytes32 policyHash,
        bytes32 verificationHash
    );
    event ClaimExpired(
        bytes32 indexed bountyId,
        uint64 indexed round,
        address indexed solver,
        uint256 claimBondForfeited,
        uint256 timeoutBondPool
    );
    event SubmissionExpired(
        bytes32 indexed bountyId, uint64 indexed round, address indexed solver, uint256 claimBondRefunded
    );
    event BountyCancelled(bytes32 indexed bountyId, uint256 timeoutBondRefundPool);
    event RefundWithdrawn(
        bytes32 indexed bountyId,
        address indexed contributor,
        uint256 principal,
        uint256 timeoutBondBonus,
        uint256 amount
    );

    modifier nonReentrant() {
        require(_reentrancy == 1, "reentrant");
        _reentrancy = 2;
        _;
        _reentrancy = 1;
    }

    /// @dev Locks the implementation while leaving each minimal proxy's storage uninitialized.
    constructor() {
        _initialized = true;
    }

    function initialize(Config calldata config, address[] calldata verifiers_) external {
        require(!_initialized, "already initialized");
        require(msg.sender == config.factory, "initializer not factory");
        _initialized = true;
        _reentrancy = 1;
        require(config.bountyId != bytes32(0), "bounty id zero");
        require(config.creator != address(0), "creator zero");
        require(config.factory != address(0), "factory zero");
        require(config.settlementToken != address(0), "token zero");
        require(config.solverReward > 0, "solver reward zero");
        require(config.verifierReward > 0, "verifier reward zero");
        require(config.solverReward + config.verifierReward <= type(uint64).max, "target too large");
        require(config.termsHash != bytes32(0), "terms hash zero");
        require(config.policyHash != bytes32(0), "policy hash zero");
        require(config.acceptanceCriteriaHash != bytes32(0), "criteria hash zero");
        require(config.benchmarkHash != bytes32(0), "benchmark hash zero");
        require(config.evidenceSchemaHash != bytes32(0), "evidence schema hash zero");
        require(config.fundingDeadline > block.timestamp, "funding deadline elapsed");
        require(config.claimWindowSeconds > 0, "claim window zero");
        require(config.verificationWindowSeconds > 0, "verification window zero");

        if (config.verificationMode == VerificationMode.DeterministicModule) {
            require(config.verifierModule != address(0), "verifier module zero");
            require(config.threshold == 1, "module threshold not one");
            require(verifiers_.length == 0, "module verifiers present");
            require(config.verifierRewardRecipient != address(0), "reward recipient zero");
        } else {
            require(config.verifierModule == address(0), "quorum module present");
            require(config.verifierRewardRecipient == address(0), "quorum reward recipient present");
            require(verifiers_.length > 0 && verifiers_.length <= 8, "bad verifier count");
            require(config.threshold > 0 && config.threshold <= verifiers_.length, "bad threshold");
            if (config.verificationMode == VerificationMode.AiJudgeQuorum) {
                require(config.threshold >= 2, "ai quorum too small");
            }
            require(config.verifierReward % config.threshold == 0, "indivisible verifier reward");
        }

        bountyId = config.bountyId;
        creator = config.creator;
        factory = config.factory;
        settlementToken = config.settlementToken;
        solverReward = config.solverReward;
        verifierReward = config.verifierReward;
        targetAmount = config.solverReward + config.verifierReward;
        termsHash = config.termsHash;
        policyHash = config.policyHash;
        acceptanceCriteriaHash = config.acceptanceCriteriaHash;
        benchmarkHash = config.benchmarkHash;
        evidenceSchemaHash = config.evidenceSchemaHash;
        verifierSetHash = verifiers_.length == 0 ? bytes32(0) : keccak256(abi.encode(verifiers_));
        fundingDeadline = config.fundingDeadline;
        claimWindowSeconds = config.claimWindowSeconds;
        verificationWindowSeconds = config.verificationWindowSeconds;
        verificationMode = config.verificationMode;
        verifierModule = config.verifierModule;
        verifierRewardRecipient = config.verifierRewardRecipient;
        threshold = config.threshold;
        _status = BountyStatus.Open;

        for (uint256 i = 0; i < verifiers_.length; i++) {
            address verifier = verifiers_[i];
            require(verifier != address(0), "verifier zero");
            require(!isVerifier[verifier], "duplicate verifier");
            isVerifier[verifier] = true;
            _verifiers.push(verifier);
        }
    }

    function protocolVersion() external pure override returns (bytes32) {
        return PROTOCOL_VERSION;
    }

    function status() external view override returns (uint8) {
        return uint8(_status);
    }

    function bountyStatus() external view returns (BountyStatus) {
        return _status;
    }

    function verifiers() external view returns (address[] memory) {
        return _verifiers;
    }

    function supportsInterface(bytes4 interfaceId) external pure override returns (bool) {
        return interfaceId == type(IAgentBountyV1).interfaceId || interfaceId == type(IERC165).interfaceId;
    }

    /// @notice Factory records funding only after transferring the exact token amount here.
    function recordFactoryFunding(address contributor, uint256 amount) external nonReentrant {
        require(msg.sender == factory, "not factory");
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) >= fundedAmount + amount, "funding not received"
        );
        _recordFunding(contributor, amount);
    }

    function fund(uint256 requestedAmount) external override nonReentrant returns (uint256 acceptedAmount) {
        require(_status == BountyStatus.Open, "not open");
        require(block.timestamp <= fundingDeadline, "funding closed");
        require(requestedAmount > 0, "amount zero");
        uint256 remaining = targetAmount - fundedAmount;
        acceptedAmount = requestedAmount < remaining ? requestedAmount : remaining;
        _recordFunding(msg.sender, acceptedAmount);
        settlementToken.safeTransferFrom(msg.sender, address(this), acceptedAmount);
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) >= _requiredTokenBalance(),
            "funding not received"
        );
    }

    /// @notice Anyone may relay a contributor's Circle USDC EIP-3009 authorization.
    function fundWithAuthorization(
        address contributor,
        uint256 amount,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external override nonReentrant {
        require(_status == BountyStatus.Open, "not open");
        require(block.timestamp <= fundingDeadline, "funding closed");
        require(amount > 0 && amount <= targetAmount - fundedAmount, "bad funding amount");
        _recordFunding(contributor, amount);
        settlementToken.safeTransferWithAuthorization(
            contributor, address(this), amount, validAfter, validBefore, nonce, v, r, s
        );
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) >= _requiredTokenBalance(),
            "funding not received"
        );
    }

    function claim() external override nonReentrant {
        _prepareClaim(msg.sender);
        activeClaimBond = verifierReward;
        _activateClaim(msg.sender);
        _collectClaimBondFrom(msg.sender);
    }

    function claimWithSignature(address solver_, uint256 deadline, bytes calldata signature) external nonReentrant {
        require(block.timestamp <= deadline, "claim signature expired");
        _prepareClaim(solver_);
        uint64 nextRound = round + 1;
        bytes32 digest = claimDigest(solver_, nextRound, deadline);
        require(_isValidSignatureNow(solver_, digest, signature), "invalid claim signature");
        activeClaimBond = verifierReward;
        _activateClaim(solver_);
        _collectClaimBondFrom(solver_);
    }

    /// @notice Anyone may relay a solver's exact USDC EIP-3009 bond authorization.
    /// The verifier reserve is paid for either verdict; a rejected solver's bond
    /// replaces that reserve so the bounty remains fully funded for another attempt.
    function claimWithAuthorization(
        address solver_,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external nonReentrant {
        require(verifierReward > 0, "claim bond zero");
        _prepareClaim(solver_);
        activeClaimBond = verifierReward;
        _activateClaim(solver_);
        settlementToken.safeTransferWithAuthorization(
            solver_, address(this), verifierReward, validAfter, validBefore, nonce, v, r, s
        );
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) >= _requiredTokenBalance(),
            "claim bond not received"
        );
    }

    function submit(bytes32 submissionHash_, bytes32 evidenceHash_) external override nonReentrant {
        require(msg.sender == solver, "not solver");
        _submit(submissionHash_, evidenceHash_);
    }

    function submitWithSignature(
        bytes32 submissionHash_,
        bytes32 evidenceHash_,
        uint256 deadline,
        bytes calldata signature
    ) external nonReentrant {
        require(block.timestamp <= deadline, "submission signature expired");
        bytes32 digest = submitDigest(solver, round, submissionHash_, evidenceHash_, deadline);
        require(_isValidSignatureNow(solver, digest, signature), "invalid submission signature");
        _submit(submissionHash_, evidenceHash_);
    }

    /// @notice Anyone may relay a valid deterministic proof. A passing call settles atomically.
    function verifyAndSettle(bytes calldata proof) external nonReentrant {
        require(_status == BountyStatus.Submitted, "not submitted");
        require(verificationMode == VerificationMode.DeterministicModule, "not module mode");
        require(block.timestamp <= verificationExpiresAt, "verification expired");
        (bool passed, bytes32 responseHash) = IAgentBountyVerifier(verifierModule)
            .verify(bountyId, round, solver, submissionHash, evidenceHash, policyHash, proof);
        bytes32 verificationHash = keccak256(abi.encode(verifierModule, responseHash, keccak256(proof)));
        address[] memory recipients = new address[](verifierReward > 0 ? 1 : 0);
        if (verifierReward > 0) recipients[0] = verifierRewardRecipient;
        if (passed) {
            _settle(recipients, verificationHash);
        } else {
            _reject(recipients, verificationHash);
        }
    }

    /// @notice Anyone may relay the precommitted verifier quorum. No settlement signer is used.
    function settleWithAttestations(Attestation[] calldata attestations) external nonReentrant {
        require(_status == BountyStatus.Submitted, "not submitted");
        require(
            verificationMode == VerificationMode.SignedQuorum || verificationMode == VerificationMode.AiJudgeQuorum,
            "not quorum mode"
        );
        require(block.timestamp <= verificationExpiresAt, "verification expired");
        require(attestations.length == threshold, "threshold signatures required");

        bool verdict = attestations[0].passed;
        address[] memory participatingVerifiers = new address[](threshold);
        for (uint256 i = 0; i < attestations.length; i++) {
            Attestation calldata attestation = attestations[i];
            require(attestation.passed == verdict, "mixed verdicts");
            require(isVerifier[attestation.verifier], "unauthorized verifier");
            require(block.timestamp <= attestation.deadline, "attestation expired");
            for (uint256 j = 0; j < i; j++) {
                require(attestations[j].verifier != attestation.verifier, "duplicate attestation");
            }
            bytes32 digest = attestationDigest(
                attestation.verifier, attestation.passed, attestation.responseHash, attestation.deadline
            );
            require(_isValidSignatureNow(attestation.verifier, digest, attestation.signature), "invalid attestation");
            participatingVerifiers[i] = attestation.verifier;
        }

        bytes32 verificationHash = keccak256(abi.encode(attestations));
        if (verdict) {
            _settle(participatingVerifiers, verificationHash);
        } else {
            _reject(participatingVerifiers, verificationHash);
        }
    }

    function expireClaim() external nonReentrant {
        require(_status == BountyStatus.Claimed, "claim not active");
        require(block.timestamp > claimExpiresAt, "claim not expired");
        address expiredSolver = solver;
        uint256 forfeitedBond = activeClaimBond;
        activeClaimBond = 0;
        require(timeoutBondPool <= type(uint128).max - forfeitedBond, "timeout pool too large");
        timeoutBondPool += forfeitedBond;
        _resetClaim();
        emit ClaimExpired(bountyId, round, expiredSolver, forfeitedBond, timeoutBondPool);
    }

    function expireSubmission() external nonReentrant {
        require(_status == BountyStatus.Submitted, "submission not active");
        require(block.timestamp > verificationExpiresAt, "submission not expired");
        address expiredSolver = solver;
        uint256 refundedBond = activeClaimBond;
        activeClaimBond = 0;
        _resetClaim();
        if (refundedBond > 0) settlementToken.safeTransfer(expiredSolver, refundedBond);
        emit SubmissionExpired(bountyId, round, expiredSolver, refundedBond);
    }

    function cancel() external nonReentrant {
        require(_status == BountyStatus.Open || _status == BountyStatus.Claimable, "not cancellable");
        require(msg.sender == creator || block.timestamp > fundingDeadline, "not authorized");
        _status = BountyStatus.Cancelled;
        refundPrincipalTotal = fundedAmount;
        refundBonusPool = timeoutBondPool;
        refundBonusRemaining = timeoutBondPool;
        timeoutBondPool = 0;
        emit BountyCancelled(bountyId, refundBonusPool);
    }

    function withdrawRefund() external nonReentrant {
        require(_status == BountyStatus.Cancelled, "not cancelled");
        uint256 principal = contributions[msg.sender];
        require(principal > 0, "no refund");
        uint256 bonus = 0;
        if (refundBonusRemaining > 0) {
            bonus =
                principal == fundedAmount ? refundBonusRemaining : principal * refundBonusPool / refundPrincipalTotal;
            refundBonusRemaining -= bonus;
        }
        contributions[msg.sender] = 0;
        fundedAmount -= principal;
        uint256 amount = principal + bonus;
        settlementToken.safeTransfer(msg.sender, amount);
        emit RefundWithdrawn(bountyId, msg.sender, principal, bonus, amount);
    }

    function claimDigest(address solver_, uint64 round_, uint256 deadline) public view returns (bytes32) {
        bytes32 structHash = keccak256(
            abi.encode(CLAIM_TYPEHASH, address(this), bountyId, solver_, round_, termsHash, policyHash, deadline)
        );
        return _hashTypedData(structHash);
    }

    function submitDigest(
        address solver_,
        uint64 round_,
        bytes32 submissionHash_,
        bytes32 evidenceHash_,
        uint256 deadline
    ) public view returns (bytes32) {
        bytes32 structHash = keccak256(
            abi.encode(
                SUBMIT_TYPEHASH,
                address(this),
                bountyId,
                solver_,
                round_,
                submissionHash_,
                evidenceHash_,
                policyHash,
                deadline
            )
        );
        return _hashTypedData(structHash);
    }

    function attestationDigest(address verifier, bool passed, bytes32 responseHash, uint256 deadline)
        public
        view
        returns (bytes32)
    {
        bytes32 structHash = keccak256(
            abi.encode(
                ATTESTATION_TYPEHASH,
                address(this),
                bountyId,
                round,
                verifier,
                submissionHash,
                evidenceHash,
                policyHash,
                passed,
                responseHash,
                deadline
            )
        );
        return _hashTypedData(structHash);
    }

    function _recordFunding(address contributor, uint256 amount) private {
        require(_status == BountyStatus.Open, "not open");
        require(contributor != address(0), "contributor zero");
        require(amount > 0 && amount <= targetAmount - fundedAmount, "bad funding amount");
        contributions[contributor] += amount;
        fundedAmount += amount;
        emit FundingAdded(bountyId, contributor, amount, fundedAmount, targetAmount);
        if (fundedAmount == targetAmount) {
            _status = BountyStatus.Claimable;
            emit BountyBecameClaimable(bountyId, fundedAmount);
        }
    }

    function _prepareClaim(address solver_) private view {
        require(_status == BountyStatus.Claimable, "not claimable");
        require(solver_ != address(0), "solver zero");
        require(activeClaimBond == 0, "claim bond active");
    }

    function _collectClaimBondFrom(address solver_) private {
        settlementToken.safeTransferFrom(solver_, address(this), verifierReward);
        require(
            IERC20BountyToken(settlementToken).balanceOf(address(this)) >= _requiredTokenBalance(),
            "claim bond not received"
        );
    }

    function _requiredTokenBalance() private view returns (uint256) {
        return fundedAmount + timeoutBondPool + activeClaimBond;
    }

    function _activateClaim(address solver_) private {
        round += 1;
        solver = solver_;
        claimExpiresAt = uint64(block.timestamp) + claimWindowSeconds;
        _status = BountyStatus.Claimed;
        emit BountyClaimed(bountyId, round, solver_, termsHash, policyHash, activeClaimBond, claimExpiresAt);
    }

    function _submit(bytes32 submissionHash_, bytes32 evidenceHash_) private {
        require(_status == BountyStatus.Claimed, "not claimed");
        require(block.timestamp <= claimExpiresAt, "claim expired");
        require(submissionHash_ != bytes32(0), "submission hash zero");
        require(evidenceHash_ != bytes32(0), "evidence hash zero");
        submissionHash = submissionHash_;
        evidenceHash = evidenceHash_;
        verificationExpiresAt = uint64(block.timestamp) + verificationWindowSeconds;
        _status = BountyStatus.Submitted;
        emit SubmissionAdded(bountyId, round, solver, submissionHash_, evidenceHash_, verificationExpiresAt);
    }

    function _settle(address[] memory verifierRecipients, bytes32 verificationHash) private {
        require(fundedAmount == targetAmount, "not fully funded");
        uint256 returnedBond = activeClaimBond;
        uint256 timeoutBonus = timeoutBondPool;
        activeClaimBond = 0;
        timeoutBondPool = 0;
        _status = BountyStatus.Settled;
        fundedAmount = 0;

        settlementToken.safeTransfer(solver, solverReward + returnedBond + timeoutBonus);
        _payVerifierReward(verifierRecipients);

        emit BountySettled(
            bountyId,
            round,
            solver,
            solverReward,
            returnedBond,
            timeoutBonus,
            verifierReward,
            submissionHash,
            evidenceHash,
            policyHash,
            verificationHash
        );
    }

    function _reject(address[] memory verifierRecipients, bytes32 verificationHash) private {
        address rejectedSolver = solver;
        uint256 forfeitedBond = activeClaimBond;
        require(forfeitedBond == verifierReward, "claim bond invariant");
        activeClaimBond = 0;
        _resetClaim();
        _payVerifierReward(verifierRecipients);
        emit SubmissionRejected(bountyId, round, rejectedSolver, verifierReward, forfeitedBond, verificationHash);
    }

    function _payVerifierReward(address[] memory verifierRecipients) private {
        if (verifierReward == 0) return;
        if (verificationMode == VerificationMode.DeterministicModule) {
            require(verifierRecipients.length == 1, "bad module recipients");
            settlementToken.safeTransfer(verifierRecipients[0], verifierReward);
        } else {
            require(verifierRecipients.length == threshold, "bad quorum recipients");
            uint256 share = verifierReward / threshold;
            for (uint256 i = 0; i < verifierRecipients.length; i++) {
                settlementToken.safeTransfer(verifierRecipients[i], share);
            }
        }
    }

    function _resetClaim() private {
        solver = address(0);
        submissionHash = bytes32(0);
        evidenceHash = bytes32(0);
        claimExpiresAt = 0;
        verificationExpiresAt = 0;
        _status = BountyStatus.Claimable;
    }

    function _hashTypedData(bytes32 structHash) private view returns (bytes32) {
        bytes32 domainSeparator =
            keccak256(abi.encode(EIP712_DOMAIN_TYPEHASH, NAME_HASH, VERSION_HASH, block.chainid, address(this)));
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator, structHash));
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
        assembly {
            r := mload(add(signature, 0x20))
            s := mload(add(signature, 0x40))
            v := byte(0, mload(add(signature, 0x60)))
        }
        if (uint256(s) > SECP256K1N_DIV_2 || (v != 27 && v != 28)) return address(0);
        recovered = ecrecover(digest, v, r, s);
    }
}
