#!/usr/bin/env python3

from __future__ import annotations

import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPTS = Path(__file__).resolve().parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

import direct_recovery_689 as recovery


class DirectRecovery689Tests(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest = recovery.load_manifest(recovery.DEFAULT_MANIFEST)

    def write_manifest(self, value: object, directory: str) -> Path:
        path = Path(directory) / "manifest.json"
        path.write_text(json.dumps(value), encoding="utf-8")
        return path

    def test_manifest_is_exactly_allowlisted_and_recovery_excluded(self) -> None:
        self.assertEqual(
            {item["contract"] for item in self.manifest["bounties"]},
            recovery.EXACT_CONTRACTS,
        )
        self.assertEqual(
            self.manifest["metrics_classification"],
            "operator_recovery_excluded",
        )
        self.assertEqual(self.manifest["initial_solver_bond_total"], 50_000)
        self.assertEqual(self.manifest["exact_return_amount"], 10_000_000)

    def test_manifest_rejects_an_arbitrary_recovery_target(self) -> None:
        changed = copy.deepcopy(self.manifest)
        changed["bounties"][0]["contract"] = "0x" + "11" * 20
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(recovery.RecoveryError, "allowlist"):
                recovery.load_manifest(self.write_manifest(changed, directory))

    def test_manifest_rejects_redirected_return_recipient(self) -> None:
        changed = copy.deepcopy(self.manifest)
        changed["return_recipient"] = "0x" + "11" * 20
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(recovery.RecoveryError, "return_recipient"):
                recovery.load_manifest(self.write_manifest(changed, directory))

    def test_manifest_rejects_weakened_acceptance_command(self) -> None:
        changed = copy.deepcopy(self.manifest)
        changed["bounties"][0]["check"] = ["python", "-c", "print('pass')"]
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(recovery.RecoveryError, "public criteria"):
                recovery.load_manifest(self.write_manifest(changed, directory))

    def test_candidate_hashes_are_deterministic_and_disclose_no_credit(self) -> None:
        bounty = self.manifest["bounties"][0]
        check = {
            "command": bounty["check"],
            "exit_code": 0,
            "repository_check": {
                "command": self.manifest["required_repository_command"],
                "exit_code": 0,
            },
        }
        args = (
            self.manifest,
            bounty,
            "a" * 40,
            "https://github.com/NSPG13/agent-bounties/pull/690",
            ["https://github.com/NSPG13/agent-bounties/actions/runs/1"],
            check,
        )
        first = recovery.build_candidate(*args)
        second = recovery.build_candidate(*args)
        self.assertEqual(first, second)
        self.assertTrue(recovery.HASH.fullmatch(first["submission_hash"]))
        self.assertTrue(recovery.HASH.fullmatch(first["evidence_hash"]))
        self.assertTrue(recovery.HASH.fullmatch(first["response_hash"]))
        self.assertFalse(first["evidence"]["metrics_credit"])
        self.assertFalse(first["evidence"]["reputation_credit"])
        self.assertFalse(first["evidence"]["leaderboard_credit"])
        self.assertFalse(first["evidence"]["organic_completion"])

    def test_original_submitted_candidate_hashes_remain_pinned(self) -> None:
        for bounty in self.manifest["bounties"]:
            check = {
                "command": bounty["check"],
                "exit_code": 0,
                "repository_check": {
                    "command": self.manifest["required_repository_command"],
                    "exit_code": 0,
                },
            }
            candidate = recovery.build_candidate(
                self.manifest,
                bounty,
                recovery.EXACT_ACCEPTED_REVISION,
                recovery.EXACT_ACCEPTED_PULL_REQUEST_URL,
                [recovery.EXACT_CHECK_RUN_URL],
                check,
            )
            recovery.validate_candidate_hashes(candidate)

    def test_stale_revision_fails_closed(self) -> None:
        with self.assertRaisesRegex(recovery.RecoveryError, "stale"):
            recovery.current_revision("0" * 40)

    def test_pull_request_url_is_pinned_to_the_project(self) -> None:
        self.assertEqual(
            recovery.require_pull_request_url(
                "https://github.com/NSPG13/agent-bounties/pull/690"
            ),
            "https://github.com/NSPG13/agent-bounties/pull/690",
        )
        with self.assertRaisesRegex(recovery.RecoveryError, "reviewed"):
            recovery.require_pull_request_url(
                "https://github.com/attacker/agent-bounties/pull/690"
            )

    def test_signer_set_requires_both_exact_verifiers(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = []
            for index, verifier in enumerate(self.manifest["verifiers"]):
                path = root / str(index)
                path.mkdir()
                entries = []
                for bounty in self.manifest["bounties"]:
                    issue = bounty["issue"]
                    name = f"attestation-{issue}.json"
                    recovery.write_json(
                        path / name,
                        {
                            "schema": recovery.ATTESTATION_SCHEMA,
                            "classification": self.manifest["metrics_classification"],
                            "issue": issue,
                            "verifier": verifier,
                            "revision": "a" * 40,
                            "accepted_work_revision": recovery.EXACT_ACCEPTED_REVISION,
                        },
                    )
                    entries.append({"issue": issue, "file": name})
                recovery.write_json(
                    path / "manifest.json",
                    {
                        "schema": recovery.ATTESTATION_SCHEMA,
                        "classification": self.manifest["metrics_classification"],
                        "revision": "a" * 40,
                        "accepted_work_revision": recovery.EXACT_ACCEPTED_REVISION,
                        "signer": verifier,
                        "attestations": entries,
                    },
                )
                paths.append(path)
            observed = recovery.load_attestations(paths, self.manifest)
            self.assertTrue(all(len(items) == 2 for items in observed.values()))
            with self.assertRaisesRegex(recovery.RecoveryError, "exact verifier set"):
                recovery.load_attestations(paths[:1], self.manifest)

    def test_transaction_jobs_install_full_rust_check_components(self) -> None:
        workflow = (
            SCRIPTS.parent / ".github" / "workflows" / "direct-recovery-689.yml"
        ).read_text(encoding="utf-8")
        self.assertEqual(workflow.count("toolchain: 1.88.0"), 4)
        self.assertEqual(workflow.count("components: rustfmt, clippy"), 4)

    def test_acceptance_environment_excludes_recovery_runtime_values(self) -> None:
        injected = {
            name: f"private-{index}"
            for index, name in enumerate(recovery.ACCEPTANCE_ENV_EXCLUSIONS)
        }
        injected["PATH"] = "public-tool-path"
        with patch.dict(recovery.os.environ, injected, clear=True):
            environment = recovery.acceptance_environment()
        self.assertEqual(environment, {"PATH": "public-tool-path"})

    def test_send_uses_one_explicit_sequential_nonce_cursor(self) -> None:
        cast = recovery.Cast("cast", "https://rpc.example")
        key = "secret"
        cast._next_nonces[key] = 62
        transaction_hash = "0x" + "11" * 32
        with (
            patch.object(cast, "rpc", side_effect=["0x1234", "0x5678"]) as rpc,
            patch.object(cast, "_publish") as publish,
            patch.object(
                cast,
                "_receipt",
                return_value={"blockNumber": "0x1", "status": "0x1"},
            ),
            patch.object(
                recovery,
                "run",
                side_effect=[transaction_hash, transaction_hash],
            ),
        ):
            cast.send(key, "0x" + "22" * 20, "claim()")
            cast.send(key, "0x" + "33" * 20, "claim()")
        self.assertEqual(rpc.call_args_list[0].args[3:5], ("--nonce", "62"))
        self.assertEqual(rpc.call_args_list[1].args[3:5], ("--nonce", "63"))
        self.assertEqual(publish.call_count, 2)

    def test_rpc_rotates_after_retryable_provider_failure(self) -> None:
        cast = recovery.Cast(
            "cast",
            "https://first.example,https://second.example",
        )
        with patch.object(
            recovery,
            "run",
            side_effect=[
                recovery.RecoveryError("HTTP error 429"),
                "8453",
            ],
        ) as runner:
            self.assertEqual(cast.rpc("chain-id"), "8453")
        self.assertIn("https://first.example", runner.call_args_list[0].args[0])
        self.assertIn("https://second.example", runner.call_args_list[1].args[0])

    def test_rpc_does_not_retry_semantic_contract_failure(self) -> None:
        cast = recovery.Cast(
            "cast",
            "https://first.example,https://second.example",
        )
        with (
            patch.object(
                recovery,
                "run",
                side_effect=recovery.RecoveryError("execution reverted: wrong signer"),
            ) as runner,
            self.assertRaisesRegex(recovery.RecoveryError, "wrong signer"),
        ):
            cast.rpc("call", "0x" + "11" * 20, "owner()(address)")
        self.assertEqual(runner.call_count, 1)

    def test_rpc_endpoint_list_requires_credential_free_https(self) -> None:
        self.assertEqual(
            recovery.rpc_urls("https://one.example, https://two.example"),
            ["https://one.example", "https://two.example"],
        )
        for value in (
            "http://rpc.example",
            "https://user:secret@rpc.example",
            "https://rpc.example/#fragment",
        ):
            with self.subTest(value=value):
                with self.assertRaisesRegex(recovery.RecoveryError, "HTTPS"):
                    recovery.rpc_urls(value)

    def test_send_rejects_an_existing_pending_keeper_transaction(self) -> None:
        cast = recovery.Cast("cast", "https://rpc.example")
        with (
            patch.object(cast, "wallet_address", return_value="0x" + "11" * 20),
            patch.object(cast, "rpc", side_effect=["62", "63"]),
            self.assertRaisesRegex(recovery.RecoveryError, "another pending transaction"),
        ):
            cast.next_nonce("secret")


if __name__ == "__main__":
    unittest.main()
