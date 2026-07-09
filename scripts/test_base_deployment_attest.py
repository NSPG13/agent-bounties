#!/usr/bin/env python3
"""Offline tests for base_deployment_attest.py."""

from __future__ import annotations

import json
import subprocess
import sys
import unittest
from pathlib import Path

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


class BaseDeploymentAttestTests(unittest.TestCase):
    def test_success_fixture_passes(self) -> None:
        proc = run_attest("--mock-fixture", str(FIXTURES / "success.json"))
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        report = parse_report(proc.stdout)
        self.assertEqual(report["overall_result"], "pass")
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

    def test_malformed_response_fails(self) -> None:
        proc = run_attest("--mock-fixture", str(FIXTURES / "malformed_response.json"))
        self.assertEqual(proc.returncode, 1)
        payload = json.loads(proc.stdout)
        self.assertIn("error", payload)

    def test_refuses_live_without_flag(self) -> None:
        proc = run_attest()
        self.assertEqual(proc.returncode, 2)
        self.assertIn("Refusing live RPC", proc.stderr)


if __name__ == "__main__":
    unittest.main()
