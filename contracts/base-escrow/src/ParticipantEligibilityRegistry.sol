// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

/// @notice Records source-scoped participant identities authorized by one immutable attester.
/// Multiple wallets may share one participant ID, but a wallet can never change identities.
contract ParticipantEligibilityRegistry {
    struct ParticipantRecord {
        bytes32 participantId;
        bytes32 sourceHash;
        uint64 registeredAt;
        uint64 validUntil;
    }

    bytes32 private constant EIP712_DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant NAME_HASH = keccak256("Agent Bounties Participant Registry");
    bytes32 private constant VERSION_HASH = keccak256("1");
    bytes32 private constant ATTESTATION_TYPEHASH = keccak256(
        "ParticipantAttestation(address wallet,bytes32 participantId,bytes32 sourceHash,uint64 validUntil,uint256 nonce)"
    );
    uint256 private constant SECP256K1N_DIV_2 = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;

    address public immutable attester;
    mapping(address => ParticipantRecord) public participants;
    mapping(address => uint256) public nonces;

    event ParticipantAttested(
        address indexed wallet,
        bytes32 indexed participantId,
        bytes32 indexed sourceHash,
        uint64 registeredAt,
        uint64 validUntil,
        uint256 nonce
    );

    constructor(address attester_) {
        require(attester_ != address(0), "attester zero");
        attester = attester_;
    }

    function attestationDigest(
        address wallet,
        bytes32 participantId,
        bytes32 sourceHash,
        uint64 validUntil,
        uint256 nonce
    ) public view returns (bytes32) {
        bytes32 structHash = keccak256(
            abi.encode(ATTESTATION_TYPEHASH, wallet, participantId, sourceHash, validUntil, nonce)
        );
        bytes32 domainSeparator =
            keccak256(abi.encode(EIP712_DOMAIN_TYPEHASH, NAME_HASH, VERSION_HASH, block.chainid, address(this)));
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator, structHash));
    }

    /// @notice Anyone may relay an attester-authorized identity registration or renewal.
    function register(
        address wallet,
        bytes32 participantId,
        bytes32 sourceHash,
        uint64 validUntil,
        bytes calldata signature
    ) external {
        require(wallet != address(0), "wallet zero");
        require(participantId != bytes32(0), "participant zero");
        require(sourceHash != bytes32(0), "source zero");
        require(validUntil > block.timestamp && validUntil <= block.timestamp + 365 days, "validity out of bounds");

        uint256 nonce = nonces[wallet];
        require(
            _recover(attestationDigest(wallet, participantId, sourceHash, validUntil, nonce), signature) == attester,
            "invalid attestation"
        );

        ParticipantRecord storage record = participants[wallet];
        if (record.participantId == bytes32(0)) {
            record.participantId = participantId;
            record.sourceHash = sourceHash;
            record.registeredAt = uint64(block.timestamp);
        } else {
            require(record.participantId == participantId, "participant immutable");
            require(record.sourceHash == sourceHash, "source immutable");
            require(block.timestamp <= record.validUntil, "expired record cannot renew");
            require(validUntil > record.validUntil, "renewal must extend validity");
        }
        record.validUntil = validUntil;
        nonces[wallet] = nonce + 1;
        emit ParticipantAttested(wallet, participantId, sourceHash, record.registeredAt, validUntil, nonce);
    }

    function eligibleAt(address wallet, uint64 cutoff)
        external
        view
        returns (bytes32 participantId, bytes32 sourceHash, bool eligible)
    {
        ParticipantRecord memory record = participants[wallet];
        participantId = record.participantId;
        sourceHash = record.sourceHash;
        eligible = participantId != bytes32(0) && record.registeredAt < cutoff && record.validUntil >= cutoff;
    }

    function _recover(bytes32 digest, bytes calldata signature) private pure returns (address recovered) {
        if (signature.length != 65) return address(0);
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly ("memory-safe") {
            r := calldataload(signature.offset)
            s := calldataload(add(signature.offset, 0x20))
            v := byte(0, calldataload(add(signature.offset, 0x40)))
        }
        if (uint256(s) > SECP256K1N_DIV_2 || (v != 27 && v != 28)) return address(0);
        recovered = ecrecover(digest, v, r, s);
    }
}
