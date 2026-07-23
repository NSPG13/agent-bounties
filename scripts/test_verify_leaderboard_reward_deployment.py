#!/usr/bin/env python3

import argparse
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import verify_leaderboard_reward_deployment as verifier


CONTRACT = "0x1111111111111111111111111111111111111111"
SIGNER_A = "0x2222222222222222222222222222222222222222"
SIGNER_B = "0x3333333333333333333333333333333333333333"
TOKEN = "0x036CbD53842c5426634e7929541eC2318f3dCF7e"
TX = "0x" + "44" * 32


class DeploymentVerificationTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        root = Path(self.temp.name)
        self.manifest = root / "manifest.json"
        self.broadcast = root / "broadcast.json"
        self.manifest.write_text(
            json.dumps(
                {
                    "chain_id": 84532,
                    "reward_contract": CONTRACT,
                    "signer_a": SIGNER_A,
                    "signer_b": SIGNER_B,
                }
            ),
            encoding="utf-8",
        )
        self.broadcast.write_text(
            json.dumps(
                {
                    "transactions": [
                        {"contractAddress": CONTRACT, "hash": TX}
                    ]
                }
            ),
            encoding="utf-8",
        )
        self.args = argparse.Namespace(
            network="base-sepolia",
            rpc_url="https://example.invalid",
            manifest=self.manifest,
            broadcast=self.broadcast,
            cast="cast",
        )

    def tearDown(self):
        self.temp.cleanup()

    @staticmethod
    def cast_result(_cast, _rpc, *arguments):
        key = tuple(arguments)
        values = {
            ("chain-id",): "84532",
            ("code", CONTRACT.lower()): "0x6001",
            ("call", CONTRACT.lower(), "settlementToken()(address)"): TOKEN,
            ("call", CONTRACT.lower(), "signerA()(address)"): SIGNER_A,
            ("call", CONTRACT.lower(), "signerB()(address)"): SIGNER_B,
            ("call", CONTRACT.lower(), "DAILY_REWARD()(uint256)"): "3000000 [3e6]",
            ("call", CONTRACT.lower(), "WEEKLY_REWARD()(uint256)"): "26000000 [2.6e7]",
            ("call", CONTRACT.lower(), "FINALIZATION_DELAY()(uint64)"): "3600",
            ("call", CONTRACT.lower(), "firstDailyStart()(uint64)"): "1728000000",
            ("call", CONTRACT.lower(), "firstWeeklyStart()(uint64)"): "1727654400",
            ("receipt", TX, "--json"): json.dumps(
                {
                    "status": "0x1",
                    "contractAddress": CONTRACT,
                    "blockNumber": "0x123",
                }
            ),
        }
        if key not in values:
            raise AssertionError(f"unexpected cast call: {key}")
        return values[key]

    @staticmethod
    def local_cast_result(_cast, *arguments):
        if arguments != ("keccak", "0x6001"):
            raise AssertionError(f"unexpected local cast call: {arguments}")
        return "0x" + "55" * 32

    @patch.object(verifier, "run_local_cast", side_effect=local_cast_result.__func__)
    @patch.object(verifier, "run_cast", side_effect=cast_result.__func__)
    def test_accepts_exact_deployment(self, _run, _local_run):
        report = verifier.verify(self.args)
        self.assertEqual(report["reward_contract"], CONTRACT)
        self.assertEqual(report["daily_reward_usdc_base_units"], 3_000_000)
        self.assertEqual(report["weekly_reward_usdc_base_units"], 26_000_000)
        self.assertEqual(report["deployment_transaction"], TX)

    @patch.object(verifier, "run_local_cast", side_effect=local_cast_result.__func__)
    @patch.object(verifier, "run_cast", side_effect=cast_result.__func__)
    def test_rejects_signer_drift(self, run, _local_run):
        original = run.side_effect

        def drift(cast, rpc, *arguments):
            if arguments[-1] == "signerA()(address)":
                return "0x9999999999999999999999999999999999999999"
            return original(cast, rpc, *arguments)

        run.side_effect = drift
        with self.assertRaisesRegex(verifier.VerificationError, "signer A"):
            verifier.verify(self.args)

    def test_retries_until_code_is_visible(self):
        code_reads = 0

        def eventual(cast, rpc, *arguments):
            nonlocal code_reads
            if arguments == ("code", CONTRACT.lower()):
                code_reads += 1
                if code_reads == 1:
                    return "0x"
            return self.cast_result(cast, rpc, *arguments)

        with (
            patch.object(verifier, "run_cast", side_effect=eventual),
            patch.object(verifier, "run_local_cast", side_effect=self.local_cast_result),
            patch.object(verifier.time, "sleep") as sleep,
        ):
            report = verifier.verify(self.args)

        self.assertEqual(report["reward_contract"], CONTRACT)
        self.assertEqual(code_reads, 2)
        sleep.assert_called_once_with(verifier.CODE_VISIBILITY_DELAY_SECONDS)

    def test_rejects_code_that_never_becomes_visible(self):
        def missing(cast, rpc, *arguments):
            if arguments == ("code", CONTRACT.lower()):
                return "0x"
            return self.cast_result(cast, rpc, *arguments)

        with (
            patch.object(verifier, "CODE_VISIBILITY_ATTEMPTS", 2),
            patch.object(verifier, "run_cast", side_effect=missing),
            patch.object(verifier.time, "sleep"),
            self.assertRaisesRegex(verifier.VerificationError, "bounded retries"),
        ):
            verifier.verify(self.args)


if __name__ == "__main__":
    unittest.main()
