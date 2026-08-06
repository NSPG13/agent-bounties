from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import scripts.activate_direct_growth_v2 as activation


class DirectGrowthActivationTests(unittest.TestCase):
    def test_manifest_pins_four_unique_tasks_and_exact_budget(self) -> None:
        manifest = activation.load_manifest()
        self.assertEqual(len(manifest["tasks"]), 4)
        self.assertEqual(manifest["total_funding"], 8_040_000)
        self.assertEqual(manifest["initial_funding"], 2_010_000)
        self.assertEqual(manifest["threshold"], 1)
        self.assertEqual(
            manifest["verifier_set_hash"],
            "0x0838846e439ed67544d8a06da2a0f344fb25cd44723ad65839da3f242a72b1f2",
        )
        self.assertEqual(
            {task["issue"] for task in manifest["tasks"]}, {771, 772, 773, 774}
        )

    def test_terms_use_one_pinned_sandbox_verifier(self) -> None:
        manifest = activation.load_manifest()
        commit = "a" * 40
        document = activation.terms_document(manifest, manifest["tasks"][0], commit)
        policy = document["verification_policy"]
        runner = document["benchmark"]["runner_manifest"]
        self.assertEqual(policy["mechanism"], "signed_quorum")
        self.assertEqual(policy["threshold"], 1)
        self.assertTrue(policy["self_verification_forbidden"])
        self.assertEqual(runner["command"], ["python", "/benchmark/check.py"])
        self.assertIn("@sha256:", runner["image"])
        self.assertEqual(document["benchmark"]["source"]["commit"], commit)
        self.assertEqual(
            document["contract_terms"]["initial_funding"]["amount"], 2_010_000
        )

    def test_create_payload_copies_only_published_hashes(self) -> None:
        manifest = activation.load_manifest()
        document = activation.terms_document(manifest, manifest["tasks"][0], "b" * 40)
        published = {
            "terms_hash": "0x" + "11" * 32,
            "policy_hash": "0x" + "22" * 32,
            "acceptance_criteria_hash": "0x" + "33" * 32,
            "benchmark_hash": "0x" + "44" * 32,
            "evidence_schema_hash": "0x" + "55" * 32,
        }
        payload = activation.create_payload(document, published)
        self.assertEqual(payload["creator"], manifest["wallet"])
        self.assertEqual(payload["verification_mode"], "signed_quorum")
        self.assertIsNone(payload["verifier_module"])
        self.assertIsNone(payload["verifier_reward_recipient"])
        self.assertEqual(payload["terms_hash"], published["terms_hash"])

    def test_issue_body_discloses_claim_and_payment_boundaries(self) -> None:
        task = activation.load_manifest()["tasks"][0]
        result = {
            "contract": "0x" + "12" * 20,
            "transaction_hash": "0x" + "34" * 32,
        }
        body = activation.issue_body(task, result, "c" * 40)
        self.assertIn("Funded and claimable on Base mainnet", body)
        self.assertIn(f"/claim #{task['issue']} wallet:", body)
        self.assertIn("BountySettled", body)
        self.assertIn("Post your own bounty", body)

    def test_manifest_rejects_duplicate_benchmark_digest(self) -> None:
        source = json.loads(activation.MANIFEST_PATH.read_text(encoding="utf-8"))
        source["tasks"][1]["benchmark_digest"] = source["tasks"][0]["benchmark_digest"]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "manifest.json"
            path.write_text(json.dumps(source), encoding="utf-8")
            with self.assertRaisesRegex(activation.ActivationError, "benchmark digest"):
                activation.load_manifest(path)

    def test_manifest_digests_match_publishable_benchmark_files(self) -> None:
        manifest = activation.load_manifest()
        for task in manifest["tasks"]:
            self.assertEqual(
                task["benchmark_digest"],
                activation.benchmark_digest(task["benchmark_subdirectory"]),
            )

    def test_rpc_list_selects_first_reachable_base_endpoint(self) -> None:
        observed: list[str] = []

        def chain_id(url: str) -> int:
            observed.append(url)
            if url.endswith("unavailable"):
                raise OSError("offline")
            return 8453

        with mock.patch.object(activation, "rpc_chain_id", side_effect=chain_id):
            selected = activation.select_rpc_url(
                "https://rpc.example/unavailable, https://rpc.example/base"
            )
        self.assertEqual(selected, "https://rpc.example/base")
        self.assertEqual(
            observed,
            ["https://rpc.example/unavailable", "https://rpc.example/base"],
        )

    def test_rpc_list_rejects_non_base_and_non_https_endpoints(self) -> None:
        with mock.patch.object(activation, "rpc_chain_id", return_value=1):
            with self.assertRaisesRegex(
                activation.ActivationError, "Base chain ID 8453"
            ):
                activation.select_rpc_url(
                    "http://rpc.example,https://rpc.example/ethereum"
                )


if __name__ == "__main__":
    unittest.main()
