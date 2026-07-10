#!/usr/bin/env python3
"""Offline tests for base_deployment_attest.py."""

from __future__ import annotations

import json
import subprocess
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from base_deployment_attest import redact_rpc_url, sanitize_provider_message

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = Path(__file__).resolve().parent / "base_deployment_attest.py"
FIXTURES = Path(__file__).resolve().parent / "fixtures" / "base_attest"


def run_attest(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        cwd=str(ROOT),
        capture_output=True,
        text=True,
        check=False,
    )


def parse_report(stdout: str) -> dict:
    return json.loads(stdout)


class RedactRpcUrlTests(unittest.TestCase):
    def test_userinfo_redacted(self) -> None:
        self.assertEqual(
            redact_rpc_url("https://user:pass@rpc.example:8545/path"),
            "https://rpc.example:8545",
        )

    def test_path_token_redacted(self) -> None:
        self.assertEqual(
            redact_rpc_url("https://eth-mainnet.g.alchemy.com/v2/SECRET"),
            "https://eth-mainnet.g.alchemy.com",
        )

    def test_query_token_redacted(self) -> None:
        self.assertEqual(
            redact_rpc_url("https://rpc.example/?apikey=SECRET"),
            "https://rpc.example",
        )

    def test_sanitize_provider_message_redacts_urls(self) -> None:
        message = "failed at https://rpc.example/?apikey=SECRET"
        self.assertEqual(sanitize_provider_message(message), "failed at https://rpc.example")


class BaseDeploymentAttestTests(unittest.TestCase):
    def test_success_fixture_passes(self) -> None:
        proc = run_attest("--mock-fixture", str(FIXTURES / "success.json"))
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        report = parse_report(proc.stdout)
        self.assertEqual(report["overall_result"], "pass")
        self.assertEqual(report["rpc_url_redacted"], "mock://offline")
        self.assertIn("boundary", report)

    def test_missing_code_fails(self) -> None:
        proc = run_attest("--mock-fixture", str(FIXTURES / "missing_code.json"))
        self.assertEqual(proc.returncode, 1)
        report = parse_report(proc.stdout)
        self.assertEqual(report["overall_result"], "fail")

    def test_failed_receipt_fails(self) -> None:
        proc = run_attest("--mock-fixture", str(FIXTURES / "failed_receipt.json"))
        self.assertEqual(proc.returncode, 1)
        failed = [c for c in parse_report(proc.stdout)["checks"] if c["name"] == "deployment_receipt_status"][0]
        self.assertFalse(failed["pass"])

    def test_wrong_owner_fails(self) -> None:
        proc = run_attest("--mock-fixture", str(FIXTURES / "wrong_owner.json"))
        self.assertEqual(proc.returncode, 1)
        failed = [c for c in parse_report(proc.stdout)["checks"] if c["name"] == "owner()"][0]
        self.assertFalse(failed["pass"])

    def test_wrong_signer_fails(self) -> None:
        proc = run_attest("--mock-fixture", str(FIXTURES / "wrong_signer.json"))
        self.assertEqual(proc.returncode, 1)

    def test_paused_contract_fails(self) -> None:
        proc = run_attest("--mock-fixture", str(FIXTURES / "paused_contract.json"))
        self.assertEqual(proc.returncode, 1)
        failed = [c for c in parse_report(proc.stdout)["checks"] if c["name"] == "paused()"][0]
        self.assertFalse(failed["pass"])

    def test_code_hash_mismatch_fails(self) -> None:
        proc = run_attest("--mock-fixture", str(FIXTURES / "code_hash_mismatch.json"))
        self.assertEqual(proc.returncode, 1)

    def test_malformed_response_fails_structured(self) -> None:
        proc = run_attest("--mock-fixture", str(FIXTURES / "malformed_response.json"))
        self.assertEqual(proc.returncode, 1)
        payload = json.loads(proc.stdout)
        self.assertEqual(payload["overall_result"], "fail")
        self.assertEqual(payload["error_type"], "malformed_provider_response")
        self.assertNotIn("Traceback", proc.stdout)

    def test_invalid_hex_fails_structured(self) -> None:
        proc = run_attest("--mock-fixture", str(FIXTURES / "invalid_hex.json"))
        self.assertEqual(proc.returncode, 1)
        payload = json.loads(proc.stdout)
        self.assertEqual(payload["error_type"], "invalid_hex")

    def test_rpc_provider_error_fails_structured(self) -> None:
        proc = run_attest("--mock-fixture", str(FIXTURES / "rpc_provider_error.json"))
        self.assertEqual(proc.returncode, 1)
        payload = json.loads(proc.stdout)
        self.assertEqual(payload["error_type"], "rpc_provider_error")
        self.assertNotIn("SECRET", payload["error"])

    def test_refuses_live_without_flag(self) -> None:
        proc = run_attest()
        self.assertEqual(proc.returncode, 2)
        self.assertIn("Refusing live RPC", proc.stderr)


if __name__ == "__main__":
    unittest.main()
