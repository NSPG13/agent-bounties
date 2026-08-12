from __future__ import annotations

import base64
import hashlib
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import export_render_operator_token_encrypted as handoff  # noqa: E402


@unittest.skipUnless(shutil.which("openssl"), "openssl is required")
class EncryptedOperatorTokenHandoffTests(unittest.TestCase):
    def test_rsa_oaep_round_trip_never_returns_plaintext(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            private_key = directory / "private.pem"
            public_key = directory / "public.pem"
            subprocess.run(
                ["openssl", "genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:3072", "-out", str(private_key)],
                check=True,
                capture_output=True,
            )
            subprocess.run(
                ["openssl", "pkey", "-in", str(private_key), "-pubout", "-out", str(public_key)],
                check=True,
                capture_output=True,
            )
            der = subprocess.run(
                ["openssl", "pkey", "-pubin", "-in", str(public_key), "-outform", "DER"],
                check=True,
                capture_output=True,
            ).stdout
            fingerprint = hashlib.sha256(der).hexdigest()
            secret = bytearray(b"one-time-operator-token-with-sufficient-entropy")
            ciphertext, observed = handoff.encrypt_secret(
                secret,
                base64.b64encode(public_key.read_bytes()).decode(),
                fingerprint,
                openssl="openssl",
            )
            self.assertEqual(observed, fingerprint)
            self.assertNotIn(bytes(secret), ciphertext)
            decrypted = subprocess.run(
                [
                    "openssl",
                    "pkeyutl",
                    "-decrypt",
                    "-inkey",
                    str(private_key),
                    "-pkeyopt",
                    "rsa_padding_mode:oaep",
                    "-pkeyopt",
                    "rsa_oaep_md:sha256",
                    "-pkeyopt",
                    "rsa_mgf1_md:sha256",
                ],
                input=ciphertext,
                check=True,
                capture_output=True,
            ).stdout
            self.assertEqual(decrypted, bytes(secret))

    def test_rejects_fingerprint_substitution(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            public_key = Path(temporary) / "public.pem"
            private_key = Path(temporary) / "private.pem"
            subprocess.run(
                ["openssl", "genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", str(private_key)],
                check=True,
                capture_output=True,
            )
            subprocess.run(
                ["openssl", "pkey", "-in", str(private_key), "-pubout", "-out", str(public_key)],
                check=True,
                capture_output=True,
            )
            with self.assertRaisesRegex(SystemExit, "fingerprint mismatch"):
                handoff.encrypt_secret(
                    bytearray(b"x" * 40),
                    base64.b64encode(public_key.read_bytes()).decode(),
                    "00" * 32,
                    openssl="openssl",
                )


if __name__ == "__main__":
    unittest.main()
