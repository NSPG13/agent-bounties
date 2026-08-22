"""Deterministic hashing and quorum signing for forward canonical-GMV campaigns."""

from __future__ import annotations

from typing import Any

from eth_keys import keys
from eth_utils import keccak


CHAIN_ID = 8453
GMV_SNAPSHOT_DOMAIN = bytes.fromhex("52abd265a2d2f97ff5791f02f8940cf87dd129db37373e61c16bf1b123b9ec9d")
GMV_POLICY_DOMAIN = bytes.fromhex("2ca0d81d158e559a56f86e206b4e7131939657aa1d3eb7c74efc1b88f92fd833")
GMV_EXCLUSIONS_DOMAIN = bytes.fromhex("c6b6b1da9249908bdb0412604fe8ddc48caa98251c069921a6de76b150af5d43")
ATTESTATION_DOMAIN = bytes.fromhex("7e5926612bbf5815ecf92cc25cc224d2c938651deec0fc6b40310e67dc6fba67")
ATTESTERS_DOMAIN = bytes.fromhex("5d74cd7806a10862ef069bd28f20646c4018c7f8a5c07357c5ca2fbc40e4ca69")
PROTOCOL_TAGS = {"autonomous": 0, "open_competition_v1": 1, "open_competition_v2": 2}


class ForwardGmvError(ValueError):
    pass


def uint(value: int, size: int) -> bytes:
    if not isinstance(value, int) or value < 0 or value >= 1 << (size * 8):
        raise ForwardGmvError(f"value is not uint{size * 8}")
    return value.to_bytes(size, "big")


def hex_bytes(value: object, size: int, field: str) -> bytes:
    if not isinstance(value, str) or not value.startswith("0x") or len(value) != 2 + size * 2:
        raise ForwardGmvError(f"{field} must be {size}-byte hex")
    try:
        decoded = bytes.fromhex(value[2:])
    except ValueError as error:
        raise ForwardGmvError(f"{field} must be hex") from error
    return decoded


def address(value: object, field: str) -> bytes:
    decoded = hex_bytes(str(value).lower(), 20, field)
    if decoded == bytes(20):
        raise ForwardGmvError(f"{field} must be nonzero")
    return decoded


def hash32(value: object, field: str) -> bytes:
    decoded = hex_bytes(str(value).lower(), 32, field)
    if decoded == bytes(32):
        raise ForwardGmvError(f"{field} must be nonzero")
    return decoded


def sorted_addresses(values: object, field: str, minimum: int = 1) -> list[bytes]:
    if not isinstance(values, list):
        raise ForwardGmvError(f"{field} must be a list")
    decoded = [address(value, field) for value in values]
    if len(decoded) < minimum or decoded != sorted(decoded) or len(set(decoded)) != len(decoded):
        raise ForwardGmvError(f"{field} must be sorted, unique, and contain at least {minimum}")
    return decoded


def exclusions_hash(campaign: dict[str, Any]) -> bytes:
    wallets = sorted_addresses(campaign.get("excluded_wallets"), "excluded wallet")
    contracts = sorted_addresses(campaign.get("excluded_bounty_contracts"), "excluded contract")
    payload = bytearray(GMV_EXCLUSIONS_DOMAIN)
    payload.extend(uint(len(wallets), 4))
    for value in wallets:
        payload.extend(value)
    payload.extend(uint(len(contracts), 4))
    for value in contracts:
        payload.extend(value)
    return keccak(bytes(payload))


def attesters_hash(campaign: dict[str, Any]) -> bytes:
    attesters = sorted_addresses(campaign.get("snapshot_attesters"), "snapshot attester", 2)
    threshold = int(campaign.get("snapshot_attestation_threshold", 0))
    if threshold < 2 or threshold > len(attesters):
        raise ForwardGmvError("snapshot attestation threshold is invalid")
    payload = bytearray(ATTESTERS_DOMAIN)
    payload.extend(uint(threshold, 1))
    payload.extend(uint(len(attesters), 4))
    for value in attesters:
        payload.extend(value)
    return keccak(bytes(payload))


def verification_policy_hash(campaign: dict[str, Any], chain_id: int = CHAIN_ID) -> bytes:
    starts_at = int(campaign["starts_at"])
    ends_at = int(campaign["ends_at"])
    minimum_score = int(campaign["minimum_score_base_units"])
    if starts_at >= ends_at or minimum_score <= 0:
        raise ForwardGmvError("campaign timing or minimum score is invalid")
    payload = bytearray(GMV_POLICY_DOMAIN)
    payload.extend(uint(chain_id, 8))
    payload.extend(hash32(campaign["epoch_id"], "epoch id"))
    payload.extend(uint(starts_at, 8))
    payload.extend(uint(ends_at, 8))
    payload.extend(uint(minimum_score, 16))
    payload.extend(exclusions_hash(campaign))
    payload.extend(attesters_hash(campaign))
    return keccak(bytes(payload))


def snapshot_hash(
    campaign: dict[str, Any], snapshot: dict[str, Any], chain_id: int = CHAIN_ID
) -> bytes:
    start_block = int(snapshot["start_block"])
    end_safe_block = int(snapshot["end_safe_block"])
    if start_block <= 0 or start_block > end_safe_block:
        raise ForwardGmvError("snapshot block range is invalid")
    settlements = snapshot.get("settlements")
    if not isinstance(settlements, list) or not settlements:
        raise ForwardGmvError("snapshot settlements are required")
    payload = bytearray(GMV_SNAPSHOT_DOMAIN)
    payload.extend(uint(chain_id, 8))
    payload.extend(hash32(campaign["epoch_id"], "epoch id"))
    payload.extend(uint(int(campaign["starts_at"]), 8))
    payload.extend(uint(int(campaign["ends_at"]), 8))
    payload.extend(uint(start_block, 8))
    payload.extend(uint(end_safe_block, 8))
    payload.extend(hash32(snapshot["end_block_hash"], "end block hash"))
    payload.extend(uint(int(campaign["minimum_score_base_units"]), 16))
    payload.extend(exclusions_hash(campaign))
    payload.extend(uint(len(settlements), 4))
    for settlement in settlements:
        protocol = settlement.get("protocol")
        if protocol not in PROTOCOL_TAGS:
            raise ForwardGmvError("unsupported settlement protocol")
        payload.extend(uint(PROTOCOL_TAGS[protocol], 1))
        payload.extend(address(settlement["bounty_contract"], "bounty contract"))
        payload.extend(hash32(settlement["bounty_id"], "bounty id"))
        payload.extend(address(settlement["creator"], "creator"))
        payload.extend(address(settlement["solver"], "solver"))
        payload.extend(uint(int(settlement["settled_at"]), 8))
        payload.extend(uint(int(settlement["block_number"]), 8))
        payload.extend(hash32(settlement["transaction_hash"], "transaction hash"))
        payload.extend(uint(int(settlement["log_index"]), 4))
        payload.extend(uint(int(settlement["gmv_base_units"]), 16))
        funding = settlement.get("funding")
        if not isinstance(funding, list) or not funding:
            raise ForwardGmvError("settlement funding is required")
        payload.extend(uint(len(funding), 4))
        for item in funding:
            payload.extend(address(item["contributor"], "contributor"))
            payload.extend(uint(int(item["amount_base_units"]), 16))
    return keccak(bytes(payload))


def attestation_digest(
    policy_hash: bytes, snapshot_digest: bytes, end_block_hash: object, chain_id: int = CHAIN_ID
) -> bytes:
    if len(policy_hash) != 32 or len(snapshot_digest) != 32:
        raise ForwardGmvError("policy and snapshot hashes must be bytes32")
    return keccak(
        ATTESTATION_DOMAIN
        + uint(chain_id, 8)
        + policy_hash
        + snapshot_digest
        + hash32(end_block_hash, "end block hash")
    )


def sign_digest(private_key: str, digest: bytes) -> dict[str, str]:
    key = keys.PrivateKey(hex_bytes(private_key, 32, "private key"))
    signature = key.sign_msg_hash(digest)
    return {
        "signer": key.public_key.to_checksum_address().lower(),
        "signature": "0x" + signature.to_bytes().hex(),
    }


def recover_signer(digest: bytes, signature: str) -> str:
    raw = hex_bytes(signature, 65, "signature")
    if raw[64] in (27, 28):
        raw = raw[:64] + bytes([raw[64] - 27])
    try:
        public_key = keys.Signature(raw).recover_public_key_from_msg_hash(digest)
    except Exception as error:
        raise ForwardGmvError("signature recovery failed") from error
    return public_key.to_checksum_address().lower()
