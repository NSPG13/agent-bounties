"""Small dependency-free EVM encoding helpers shared by deployment tooling."""

from __future__ import annotations

import re
from typing import Any

from Crypto.Hash import keccak


def keccak_bytes(data: bytes) -> bytes:
    digest = keccak.new(digest_bits=256)
    digest.update(data)
    return digest.digest()


def keccak256(data: bytes) -> str:
    return f"0x{keccak_bytes(data).hex()}"


def address_bytes(value: str) -> bytes:
    raw = value.removeprefix("0x")
    if not re.fullmatch(r"[0-9a-fA-F]{40}", raw):
        raise ValueError(f"invalid EVM address: {value}")
    return bytes.fromhex(raw)


def address_word(value: str) -> bytes:
    return address_bytes(value).rjust(32, b"\0")


def uint_word(value: int) -> bytes:
    if value < 0 or value >= 1 << 256:
        raise ValueError("uint256 is out of range")
    return value.to_bytes(32, "big")


def _rlp_bytes(value: bytes) -> bytes:
    if len(value) == 1 and value[0] < 0x80:
        return value
    if len(value) <= 55:
        return bytes([0x80 + len(value)]) + value
    length = len(value).to_bytes((len(value).bit_length() + 7) // 8, "big")
    return bytes([0xB7 + len(length)]) + length + value


def _rlp_list(values: list[bytes]) -> bytes:
    payload = b"".join(values)
    if len(payload) <= 55:
        return bytes([0xC0 + len(payload)]) + payload
    length = len(payload).to_bytes((len(payload).bit_length() + 7) // 8, "big")
    return bytes([0xF7 + len(length)]) + length + payload


def create_address(deployer: str, nonce: int) -> str:
    if nonce < 0:
        raise ValueError("deployer nonce must be nonnegative")
    encoded_nonce = b"" if nonce == 0 else nonce.to_bytes((nonce.bit_length() + 7) // 8, "big")
    payload = _rlp_list([_rlp_bytes(address_bytes(deployer)), _rlp_bytes(encoded_nonce)])
    return f"0x{keccak_bytes(payload).hex()[-40:]}"


def artifact_hex(field: Any, name: str, *, distinct_odd_length_error: bool = False) -> bytes:
    value = field.get("object") if isinstance(field, dict) else None
    if not isinstance(value, str):
        raise ValueError(f"artifact {name} is missing concrete bytecode")
    raw = value.removeprefix("0x")
    if not raw or not re.fullmatch(r"[0-9a-fA-F]+", raw):
        raise ValueError(f"artifact {name} is missing concrete bytecode")
    if len(raw) % 2:
        if distinct_odd_length_error:
            raise ValueError(f"artifact {name} has odd-length bytecode")
        raise ValueError(f"artifact {name} is missing concrete bytecode")
    return bytes.fromhex(raw)
