#!/usr/bin/env python3
"""Ed25519 signature verification and canonical binding encoding.

WHY THIS FILE EXISTS
--------------------
Maintainer review, correctly: "a path outside the session directory alone is
insufficient ... use a signed operator binding verified against a pinned
operator key". A same-OS-user OpenHands session can rewrite any file its own
user owns, so provenance-by-location was an authority boundary in name only.

A signature is a boundary the session process genuinely cannot cross: it can
rewrite the binding file all it likes, but it cannot produce a valid signature
over the rewritten body without the operator's private key, which never enters
the sandbox. The verifier only ever needs the PUBLIC key.

WHY NOT JUST IMPORT A CRYPTO LIBRARY
------------------------------------
The Stop hook runs inside whatever sandbox OpenHands gives it. Making the
authority boundary depend on `pip install` succeeding there would mean the guard
silently degrades to "no verification available" in exactly the environments it
is supposed to protect. So verification works with the standard library alone.

`verify()` prefers PyCryptodome when it is importable (it is already pinned by
scripts/requirements-attest.txt) and otherwise falls back to the RFC 8032
reference verification below. Both paths are cross-checked against the official
RFC 8032 section 7.1 test vectors by scripts/check-openhands-integration.py, and
the checker also asserts the two implementations agree, so the fallback cannot
drift away from a real Ed25519 implementation unnoticed.

This module verifies signatures. It NEVER signs, never handles a private key,
and never touches wallet material; `sign_binding.py` is the separate
operator-side tool and is meant to be run outside the sandbox.
"""

from __future__ import annotations

import hashlib
import json

# ---------------------------------------------------------------------------
# RFC 8032 Ed25519, verification only. Structure follows the reference code in
# RFC 8032 section 6 (public domain).
# ---------------------------------------------------------------------------

_P = 2 ** 255 - 19
_L = 2 ** 252 + 27742317777372353535851937790883648493
_D = -121665 * pow(121666, _P - 2, _P) % _P
_I = pow(2, (_P - 1) // 4, _P)


def _sha512(data: bytes) -> bytes:
    return hashlib.sha512(data).digest()


def _x_recover(y: int) -> int:
    xx = (y * y - 1) * pow(_D * y * y + 1, _P - 2, _P)
    x = pow(xx, (_P + 3) // 8, _P)
    if (x * x - xx) % _P != 0:
        x = (x * _I) % _P
    if x % 2 != 0:
        x = _P - x
    return x


_BY = 4 * pow(5, _P - 2, _P) % _P
_BASE = (_x_recover(_BY), _BY, 1, _x_recover(_BY) * _BY % _P)
_IDENTITY = (0, 1, 1, 0)


def _add(p, q):
    """Extended twisted-Edwards point addition (RFC 8032 'edwards_add')."""
    x1, y1, z1, t1 = p
    x2, y2, z2, t2 = q
    a = (y1 - x1) * (y2 - x2) % _P
    b = (y1 + x1) * (y2 + x2) % _P
    c = t1 * 2 * _D * t2 % _P
    d = z1 * 2 * z2 % _P
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % _P, g * h % _P, f * g % _P, e * h % _P)


def _mul(point, scalar: int):
    result = _IDENTITY
    while scalar > 0:
        if scalar & 1:
            result = _add(result, point)
        point = _add(point, point)
        scalar >>= 1
    return result


def _affine(point):
    x, y, z, _ = point
    inv = pow(z, _P - 2, _P)
    return (x * inv % _P, y * inv % _P)


def _decode_point(data: bytes):
    """Decode a 32-byte compressed point, or return None if it is not on the curve."""
    if len(data) != 32:
        return None
    value = int.from_bytes(data, "little")
    sign = value >> 255
    y = value & ((1 << 255) - 1)
    if y >= _P:
        return None
    x = _x_recover(y)
    if x & 1 != sign:
        x = _P - x
    point = (x, y, 1, x * y % _P)
    # Reject anything not actually satisfying the curve equation.
    if (-x * x + y * y - 1 - _D * x * x * y * y) % _P != 0:
        return None
    return point


def _verify_pure(public_key: bytes, message: bytes, signature: bytes) -> bool:
    if len(signature) != 64 or len(public_key) != 32:
        return False
    r_bytes, s_bytes = signature[:32], signature[32:]
    s = int.from_bytes(s_bytes, "little")
    if s >= _L:            # non-canonical S: reject (malleability)
        return False
    point_a = _decode_point(public_key)
    point_r = _decode_point(r_bytes)
    if point_a is None or point_r is None:
        return False
    k = int.from_bytes(_sha512(r_bytes + public_key + message), "little") % _L
    left = _affine(_mul(_BASE, s))
    right = _affine(_add(point_r, _mul(point_a, k)))
    return left == right


def _verify_pycryptodome(public_key: bytes, message: bytes, signature: bytes):
    """Return True/False using PyCryptodome, or None if it is unavailable."""
    try:
        from Crypto.PublicKey import ECC
        from Crypto.Signature import eddsa
    except ImportError:
        return None
    try:
        key = ECC.import_key(
            b"\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00" + public_key)
        eddsa.new(key, "rfc8032").verify(message, signature)
        return True
    except (ValueError, TypeError):
        return False


def verify(public_key: bytes, message: bytes, signature: bytes) -> bool:
    """Verify a detached Ed25519 signature. Never raises; returns a bool."""
    if not isinstance(public_key, (bytes, bytearray)) or len(public_key) != 32:
        return False
    if not isinstance(signature, (bytes, bytearray)) or len(signature) != 64:
        return False
    library = _verify_pycryptodome(bytes(public_key), bytes(message), bytes(signature))
    if library is not None:
        return library
    return _verify_pure(bytes(public_key), bytes(message), bytes(signature))


# ---------------------------------------------------------------------------
# Canonical signing payload.
# ---------------------------------------------------------------------------

def canonical_payload(binding: dict) -> bytes:
    """The exact bytes a signature covers.

    Everything in the binding EXCEPT the signature envelope itself is covered,
    serialized with sorted keys and no insignificant whitespace so that the
    signer and the verifier cannot disagree about byte order. Anything the
    signature does not cover is, by definition, attacker-controlled -- so the
    rule is "sign the whole document minus the signature", not a field list that
    can quietly fall behind the schema.
    """
    body = {k: v for k, v in binding.items() if k != "signature"}
    return json.dumps(body, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False).encode("utf-8")
