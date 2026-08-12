#!/usr/bin/env python3
"""Export the existing Render operator token only as one RSA-OAEP ciphertext."""

from __future__ import annotations

import argparse
import base64
import binascii
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess  # nosec B404 -- fixed openssl argv, no shell
import tempfile
from typing import Any
import urllib.parse

import render_deploy_recovery as recovery


SCHEMA = "agent-bounties/render-operator-token-encrypted-handoff-v1"
ENV_GROUP_NAME = "agent-bounties-operator"
TOKEN_KEY = "OPERATOR_API_TOKEN"
FINGERPRINT = re.compile(r"^[0-9a-f]{64}$")


def fail(message: str) -> None:
    raise SystemExit(message)


def decode_public_key(encoded: str) -> bytes:
    try:
        public_key = base64.b64decode(encoded, validate=True)
    except (ValueError, binascii.Error):
        fail("recipient public key must be strict base64")
    if not (256 <= len(public_key) <= 8_192):
        fail("recipient public key has an invalid size")
    if not public_key.startswith(b"-----BEGIN PUBLIC KEY-----"):
        fail("recipient key must be an RSA public key in PEM SubjectPublicKeyInfo form")
    return public_key


def run_openssl(openssl: str, arguments: list[str], *, input_bytes: bytes | bytearray | None = None) -> bytes:
    completed = subprocess.run(  # nosec B603
        [openssl, *arguments],
        input=input_bytes,
        capture_output=True,
        check=False,
    )
    if completed.returncode != 0:
        fail("OpenSSL rejected the recipient key or encryption request")
    return completed.stdout


def public_key_der(openssl: str, public_key_path: Path) -> bytes:
    return run_openssl(
        openssl,
        ["pkey", "-pubin", "-in", str(public_key_path), "-outform", "DER"],
    )


def encrypt_secret(
    secret: bytearray,
    encoded_public_key: str,
    expected_fingerprint: str,
    *,
    openssl: str,
) -> tuple[bytes, str]:
    expected = expected_fingerprint.strip().lower()
    if not FINGERPRINT.fullmatch(expected):
        fail("recipient fingerprint must be a lowercase SHA-256 hex digest")
    public_key = decode_public_key(encoded_public_key)
    with tempfile.TemporaryDirectory(prefix="agent-bounties-render-handoff-") as temporary:
        public_key_path = Path(temporary) / "recipient-public.pem"
        public_key_path.write_bytes(public_key)
        fingerprint = hashlib.sha256(public_key_der(openssl, public_key_path)).hexdigest()
        if fingerprint != expected:
            fail("recipient public key fingerprint mismatch")
        ciphertext = run_openssl(
            openssl,
            [
                "pkeyutl",
                "-encrypt",
                "-pubin",
                "-inkey",
                str(public_key_path),
                "-pkeyopt",
                "rsa_padding_mode:oaep",
                "-pkeyopt",
                "rsa_oaep_md:sha256",
                "-pkeyopt",
                "rsa_mgf1_md:sha256",
            ],
            input_bytes=secret,
        )
    if len(ciphertext) < 256 or secret in ciphertext:
        fail("encrypted operator-token handoff failed its confidentiality check")
    return ciphertext, fingerprint


def select_operator_group(payload: object, owner_id: str) -> dict[str, Any]:
    groups = recovery.unwrap_env_group_entries(payload)
    matches = [
        group
        for group in groups
        if group.get("name") == ENV_GROUP_NAME and group.get("ownerId") == owner_id
    ]
    if len(matches) != 1:
        fail("expected exactly one Render operator environment group")
    group = matches[0]
    group_id = group.get("id")
    if not isinstance(group_id, str) or not re.fullmatch(r"evg-[0-9a-z]+", group_id):
        fail("Render operator environment group has an invalid id")
    return group


def read_operator_token(client: recovery.RenderClient) -> bytearray:
    service = client.resolve_service(recovery.SERVICE_SPECS[0])
    owner_id = recovery.validate_owner_id(service.get("ownerId"))
    query = urllib.parse.urlencode(
        {"name": ENV_GROUP_NAME, "ownerId": owner_id, "limit": "20"}
    )
    group = select_operator_group(client._read_with_retry(f"/env-groups?{query}"), owner_id)
    record = recovery.unwrap_env_var(
        client._read_with_retry(f"/env-groups/{group['id']}/env-vars/{TOKEN_KEY}")
    )
    token = record.get("value", "")
    if record.get("key") != TOKEN_KEY or len(token) < 32 or any(character.isspace() for character in token):
        fail("Render operator token is missing or malformed")
    return bytearray(token.encode("utf-8"))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--recipient-public-key-base64", required=True)
    parser.add_argument("--recipient-fingerprint-sha256", required=True)
    parser.add_argument("--openssl", default="openssl")
    parser.add_argument("--ciphertext-output", type=Path, required=True)
    parser.add_argument("--metadata-output", type=Path, required=True)
    args = parser.parse_args()

    api_key = os.environ.get("RENDER_API_KEY", "").strip()
    if not api_key:
        fail("RENDER_API_KEY is required")
    token = read_operator_token(recovery.RenderClient(api_key))
    try:
        ciphertext, fingerprint = encrypt_secret(
            token,
            args.recipient_public_key_base64,
            args.recipient_fingerprint_sha256,
            openssl=args.openssl,
        )
    finally:
        for index in range(len(token)):
            token[index] = 0

    args.ciphertext_output.parent.mkdir(parents=True, exist_ok=True)
    args.metadata_output.parent.mkdir(parents=True, exist_ok=True)
    args.ciphertext_output.write_bytes(ciphertext)
    metadata = {
        "schema_version": SCHEMA,
        "recipient_public_key_sha256": fingerprint,
        "ciphertext_sha256": hashlib.sha256(ciphertext).hexdigest(),
        "ciphertext_bytes": len(ciphertext),
        "encryption": "RSA-OAEP-SHA256-MGF1-SHA256",
        "plaintext_persisted": False,
        "secret_logged": False,
        "evidence_boundary": (
            "This artifact contains only ciphertext encrypted to the declared one-time public key. "
            "It is not an operator authorization, deployment, settlement, or payment record."
        ),
    }
    args.metadata_output.write_text(
        json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(json.dumps(metadata, sort_keys=True))


if __name__ == "__main__":
    main()
