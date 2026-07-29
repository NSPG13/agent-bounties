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
                        },
                    )
                    entries.append({"issue": issue, "file": name})
                recovery.write_json(
                    path / "manifest.json",
                    {
                        "schema": recovery.ATTESTATION_SCHEMA,
                        "classification": self.manifest["metrics_classification"],
                        "revision": "a" * 40,
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


if __name__ == "__main__":
    unittest.main()
