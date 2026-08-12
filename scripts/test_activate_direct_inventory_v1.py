from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

import scripts.activate_direct_growth_v2 as activation


MANIFEST = (
    activation.ROOT
    / "bounties"
    / "autonomous-v1"
    / "direct-inventory-v1-manifest.json"
)


class DirectInventoryActivationTests(unittest.TestCase):
    def test_manifest_pins_five_profitable_tasks_within_balance(self) -> None:
        manifest = activation.load_manifest(MANIFEST)
        self.assertEqual(manifest["task_count"], 5)
        self.assertEqual(manifest["total_funding"], 5_500_000)
        self.assertEqual(manifest["solver_reward"], 1_000_000)
        self.assertEqual(manifest["verifier_reward"], 100_000)
        self.assertEqual(manifest["threshold"], 2)
        self.assertEqual(
            {task["issue"] for task in manifest["tasks"]},
            {869, 870, 871, 872, 873},
        )
        self.assertEqual(
            activation.contract_manifest_path(manifest),
            activation.ROOT / "deployments" / "bounded-agent-wallet-base-mainnet.json",
        )

    def test_manifest_digests_match_every_publishable_benchmark(self) -> None:
        manifest = activation.load_manifest(MANIFEST)
        for task in manifest["tasks"]:
            self.assertEqual(
                task["benchmark_digest"],
                activation.benchmark_digest(task["benchmark_subdirectory"]),
            )

    def test_terms_use_exact_automated_quorum_and_economics(self) -> None:
        manifest = activation.load_manifest(MANIFEST)
        document = activation.terms_document(manifest, manifest["tasks"][0], "d" * 40)
        policy = document["verification_policy"]
        terms = document["contract_terms"]
        self.assertEqual(policy["threshold"], 2)
        self.assertEqual(policy["verifiers"], manifest["verifiers"])
        self.assertTrue(policy["self_verification_forbidden"])
        self.assertEqual(terms["solver_reward"]["amount"], 1_000_000)
        self.assertEqual(terms["verifier_reward"]["amount"], 100_000)
        self.assertEqual(terms["claim_bond"]["amount"], 100_000)
        self.assertEqual(terms["initial_funding"]["amount"], 1_100_000)

    def test_issue_body_uses_manifest_amounts_and_two_signers(self) -> None:
        manifest = activation.load_manifest(MANIFEST)
        task = manifest["tasks"][0]
        body = activation.issue_body(
            manifest,
            task,
            {
                "contract": "0x" + "12" * 20,
                "transaction_hash": "0x" + "34" * 32,
            },
            "e" * 40,
        )
        self.assertIn("1.10 / 1.10 USDC", body)
        self.assertIn("1.00 USDC", body)
        self.assertIn("0.10 USDC", body)
        self.assertIn("2 of 2 precommitted automated signers", body)

    def test_manifest_rejects_indivisible_verifier_reward(self) -> None:
        source = json.loads(MANIFEST.read_text(encoding="utf-8"))
        source["verifier_reward"] = 100_001
        source["initial_funding"] = 1_100_001
        source["total_funding"] = 5_500_005
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "manifest.json"
            path.write_text(json.dumps(source), encoding="utf-8")
            with self.assertRaisesRegex(activation.ActivationError, "divide evenly"):
                activation.load_manifest(path)


if __name__ == "__main__":
    unittest.main()
