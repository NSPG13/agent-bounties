#!/usr/bin/env python3
"""Unit tests and negative controls for direct-bounty evidence checklist (Issue #686)."""

from __future__ import annotations

from copy import deepcopy
import unittest

from scripts.direct_bounty_evidence_checklist import (
    BOUNTY_SETTLED_TOPIC0,
    NATIVE_BASE_USDC,
    evaluate_direct_bounty_evidence,
)


COMMIT_SHA = "d58fa377e9081f963f9a380b11a29f51fa57421a"
ARTIFACT_HEX = "754efb520ba132d1bf3efd4e0710ab8c7431df7e7b1061224807460bdda3a957"
ARTIFACT_DIGEST = f"sha256:{ARTIFACT_HEX}"


def _valid_evidence() -> dict:
    return {
        "repository": "NSPG13/agent-bounties",
        "commit_sha": COMMIT_SHA,
        "subdirectory": "benchmarks/direct-inventory-v1/replenishment-plan",
        "pull_request_url": "https://github.com/NSPG13/agent-bounties/pull/702",
        "check_run_urls": [
            "https://github.com/NSPG13/agent-bounties/actions/runs/30441776495/job/91234567890"
        ],
        "artifact_url": (
            f"https://objects.githubusercontent.com/github-production-release-asset/sha256/{ARTIFACT_HEX}"
        ),
        "artifact_digest": ARTIFACT_DIGEST,
    }


class DirectBountyEvidenceChecklistTests(unittest.TestCase):
    def test_valid_unsettled_and_verified_settled_evidence_pass(self) -> None:
        base = _valid_evidence()
        res = evaluate_direct_bounty_evidence(base)
        self.assertTrue(res["ok"], res["errors"])
        self.assertFalse(res["payment"]["settled"])
        self.assertFalse(res["payment"]["paid"])
        self.assertIn("Submission", res["markdown"])
        self.assertIn("Verification", res["markdown"])

        # Attach a verified canonical BountySettled receipt bound to the exact hashes
        sub_hash = res["submission"]["submission_hash"]
        ev_hash = res["verification"]["evidence_hash"]
        bounty_contract = "0xc13ccf6c6a03b53f836d433c5e628f06bbc1dbf4"
        bounty_id = "0x" + "77" * 32
        settled_payload = deepcopy(base)
        settled_payload["payment"] = {
            "chain_id": 8453,
            "settlement_token": NATIVE_BASE_USDC,
            "event": "BountySettled",
            "bounty_contract": bounty_contract,
            "bounty_id": bounty_id,
            "solver": "0xc26a630e85134ed30968735c8e7de4576cfa5dbc",
            "transaction_hash": "0x" + "99" * 32,
            "receipt": {
                "status": 1,
                "block_number": 22_100_000,
                "safe_block": 22_100_010,
                "bounty_settled_log": {
                    "address": bounty_contract,
                    "topics": [BOUNTY_SETTLED_TOPIC0, bounty_id],
                    "log_index": 4,
                    "submission_hash": sub_hash,
                    "evidence_hash": ev_hash,
                    "solver_reward": 1_000_000,
                    "claim_bond_returned": 100_000,
                    "timeout_bond_bonus": 0,
                    "solver_payout": 1_100_000,
                },
            },
        }
        settled_res = evaluate_direct_bounty_evidence(settled_payload)
        self.assertTrue(settled_res["ok"], settled_res["errors"])
        self.assertTrue(settled_res["payment"]["settled"])
        self.assertTrue(settled_res["payment"]["paid"])
        self.assertEqual(settled_res["payment"]["solver_payout"], 1_100_000)

    def test_missing_subdirectory_or_artifact_is_rejected(self) -> None:
        no_subdir = _valid_evidence()
        no_subdir["subdirectory"] = ""
        res_subdir = evaluate_direct_bounty_evidence(no_subdir)
        self.assertFalse(res_subdir["ok"])
        self.assertIn("invalid_or_missing_subdirectory", res_subdir["errors"])

        traversal_subdir = _valid_evidence()
        traversal_subdir["subdirectory"] = "benchmarks/../secrets"
        res_trav = evaluate_direct_bounty_evidence(traversal_subdir)
        self.assertFalse(res_trav["ok"])
        self.assertIn("invalid_or_missing_subdirectory", res_trav["errors"])

        no_artifact = _valid_evidence()
        del no_artifact["artifact_url"]
        res_art = evaluate_direct_bounty_evidence(no_artifact)
        self.assertFalse(res_art["ok"])
        self.assertIn("missing_artifact_url", res_art["errors"])

    def test_mutable_non_https_and_credential_bearing_urls_are_rejected(self) -> None:
        mutable = _valid_evidence()
        mutable["artifact_url"] = "https://github.com/NSPG13/agent-bounties/raw/main/artifact.tar.gz"
        res_mut = evaluate_direct_bounty_evidence(mutable)
        self.assertFalse(res_mut["ok"])
        self.assertIn("mutable_artifact_url", res_mut["errors"])

        http_art = _valid_evidence()
        http_art["artifact_url"] = f"http://example.com/sha256/{ARTIFACT_HEX}"
        res_http = evaluate_direct_bounty_evidence(http_art)
        self.assertFalse(res_http["ok"])
        self.assertIn("non_https_artifact_url", res_http["errors"])

        cred_art = _valid_evidence()
        cred_art["artifact_url"] = (
            f"https://user:secret@objects.githubusercontent.com/sha256/{ARTIFACT_HEX}"
        )
        res_cred = evaluate_direct_bounty_evidence(cred_art)
        self.assertFalse(res_cred["ok"])
        self.assertIn("credentials_in_artifact_url", res_cred["errors"])

        query_token = _valid_evidence()
        query_token["artifact_url"] = (
            f"https://objects.githubusercontent.com/sha256/{ARTIFACT_HEX}?token=ghp_secret"
        )
        res_q = evaluate_direct_bounty_evidence(query_token)
        self.assertFalse(res_q["ok"])
        self.assertIn("credentials_in_artifact_url", res_q["errors"])

    def test_cross_repository_pr_and_check_run_urls_are_rejected(self) -> None:
        cross_pr = _valid_evidence()
        cross_pr["pull_request_url"] = "https://github.com/other-org/agent-bounties/pull/702"
        res_pr = evaluate_direct_bounty_evidence(cross_pr)
        self.assertFalse(res_pr["ok"])
        self.assertIn("cross_repository_pull_request_url", res_pr["errors"])

        cross_check = _valid_evidence()
        cross_check["check_run_urls"] = [
            "https://github.com/other-org/agent-bounties/actions/runs/30441776495"
        ]
        res_check = evaluate_direct_bounty_evidence(cross_check)
        self.assertFalse(res_check["ok"])
        self.assertIn("cross_repository_check_run_url", res_check["errors"])

    def test_unverified_settlement_receipts_are_rejected(self) -> None:
        # Formatting-only payment without verified receipt proof
        unverified = _valid_evidence()
        unverified["payment"] = {
            "chain_id": 8453,
            "settlement_token": NATIVE_BASE_USDC,
            "event": "BountySettled",
            "bounty_contract": "0xc13ccf6c6a03b53f836d433c5e628f06bbc1dbf4",
            "bounty_id": "0x" + "77" * 32,
            "solver": "0xc26a630e85134ed30968735c8e7de4576cfa5dbc",
            "transaction_hash": "0x" + "99" * 32,
        }
        res_unv = evaluate_direct_bounty_evidence(unverified)
        self.assertFalse(res_unv["ok"])
        self.assertIn("unverified_receipt_missing_proof", res_unv["errors"])

        # Mismatched evidence_hash in receipt log
        mismatched = _valid_evidence()
        mismatched["payment"] = {
            "chain_id": 8453,
            "settlement_token": NATIVE_BASE_USDC,
            "event": "BountySettled",
            "bounty_contract": "0xc13ccf6c6a03b53f836d433c5e628f06bbc1dbf4",
            "bounty_id": "0x" + "77" * 32,
            "solver": "0xc26a630e85134ed30968735c8e7de4576cfa5dbc",
            "transaction_hash": "0x" + "99" * 32,
            "receipt": {
                "status": 1,
                "block_number": 22_100_000,
                "safe_block": 22_100_010,
                "bounty_settled_log": {
                    "address": "0xc13ccf6c6a03b53f836d433c5e628f06bbc1dbf4",
                    "topics": [BOUNTY_SETTLED_TOPIC0, "0x" + "77" * 32],
                    "log_index": 4,
                    "submission_hash": "0x" + "11" * 32,
                    "evidence_hash": "0x" + "22" * 32,
                    "solver_reward": 1_000_000,
                    "claim_bond_returned": 100_000,
                    "timeout_bond_bonus": 0,
                    "solver_payout": 1_100_000,
                },
            },
        }
        res_mis = evaluate_direct_bounty_evidence(mismatched)
        self.assertFalse(res_mis["ok"])
        self.assertIn("unverified_receipt_submission_hash_mismatch", res_mis["errors"])
        self.assertIn("unverified_receipt_evidence_hash_mismatch", res_mis["errors"])


if __name__ == "__main__":
    unittest.main()
