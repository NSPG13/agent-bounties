#!/usr/bin/env python3
"""Unit tests and negative controls for direct-bounty evidence checklist (Issue #686)."""

from __future__ import annotations

from copy import deepcopy
from typing import Any
import unittest

from scripts.direct_bounty_evidence_checklist import (
    BOUNTY_SETTLED_TOPIC0,
    IS_CANONICAL_BOUNTY_SELECTOR,
    NATIVE_BASE_USDC,
    evaluate_direct_bounty_evidence,
)


COMMIT_SHA = "d58fa377e9081f963f9a380b11a29f51fa57421a"
ARTIFACT_HEX = "754efb520ba132d1bf3efd4e0710ab8c7431df7e7b1061224807460bdda3a957"
ARTIFACT_DIGEST = f"sha256:{ARTIFACT_HEX}"
BOUNTY_FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
BOUNTY_CONTRACT = "0xc13ccf6c6a03b53f836d433c5e628f06bbc1dbf4"
BOUNTY_ID = "0x" + "77" * 32
SOLVER = "0xc26a630e85134ed30968735c8e7de4576cfa5dbc"
TX_HASH = "0x" + "99" * 32


def _word_uint(val: int) -> str:
    return f"{val:064x}"


def _word_bytes32(b32: str) -> str:
    return b32.lower().removeprefix("0x")


def _encode_settled_log_data(
    *,
    solver_reward: int = 1_000_000,
    claim_bond_returned: int = 100_000,
    timeout_bond_bonus: int = 0,
    verifier_reward: int = 100_000,
    submission_hash: str,
    evidence_hash: str,
    policy_hash: str = "0x" + "aa" * 32,
    verification_hash: str = "0x" + "bb" * 32,
) -> str:
    return "0x" + "".join(
        [
            _word_uint(solver_reward),
            _word_uint(claim_bond_returned),
            _word_uint(timeout_bond_bonus),
            _word_uint(verifier_reward),
            _word_bytes32(submission_hash),
            _word_bytes32(evidence_hash),
            _word_bytes32(policy_hash),
            _word_bytes32(verification_hash),
        ]
    )


def _valid_evidence() -> dict[str, Any]:
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


def _make_settlement_rpc(
    *,
    submission_hash: str,
    evidence_hash: str,
    is_canonical_bounty: bool = True,
    tx_status: str = "0x1",
    block_number: int = 22_100_000,
    safe_block: int = 22_100_010,
) -> Any:
    def _rpc(method: str, params: list[Any]) -> Any:
        if method == "eth_chainId":
            return hex(8453)
        if method == "eth_getBlockByNumber":
            return {"number": hex(safe_block), "hash": "0x" + "44" * 32}
        if method == "eth_call":
            call_obj = params[0]
            if (
                call_obj["to"].lower() == BOUNTY_FACTORY
                and call_obj["data"].lower().startswith(IS_CANONICAL_BOUNTY_SELECTOR)
            ):
                return "0x" + _word_uint(1 if is_canonical_bounty else 0)
            return "0x" + _word_uint(0)
        if method == "eth_getTransactionReceipt":
            if params[0].lower() != TX_HASH:
                return None
            return {
                "status": tx_status,
                "blockNumber": hex(block_number),
                "logs": [
                    {
                        "address": BOUNTY_CONTRACT,
                        "topics": [
                            BOUNTY_SETTLED_TOPIC0,
                            BOUNTY_ID,
                            "0x" + _word_uint(1),
                            "0x" + ("0" * 24) + SOLVER[2:],
                        ],
                        "data": _encode_settled_log_data(
                            submission_hash=submission_hash,
                            evidence_hash=evidence_hash,
                        ),
                        "logIndex": "0x4",
                        "removed": False,
                    }
                ],
            }
        raise AssertionError(f"Unexpected RPC method {method}")

    return _rpc


class DirectBountyEvidenceChecklistTests(unittest.TestCase):
    def test_valid_unsettled_and_verified_settled_evidence_pass(self) -> None:
        base = _valid_evidence()
        res = evaluate_direct_bounty_evidence(base)
        self.assertTrue(res["ok"], res["errors"])
        self.assertFalse(res["payment"]["settled"])
        self.assertFalse(res["payment"]["paid"])
        self.assertIn("Submission", res["markdown"])
        self.assertIn("Verification", res["markdown"])

        # Attach payment metadata and verify via chain RPC callback
        sub_hash = res["submission"]["submission_hash"]
        ev_hash = res["verification"]["evidence_hash"]
        settled_payload = deepcopy(base)
        settled_payload["payment"] = {
            "chain_id": 8453,
            "settlement_token": NATIVE_BASE_USDC,
            "event": "BountySettled",
            "bounty_factory": BOUNTY_FACTORY,
            "bounty_contract": BOUNTY_CONTRACT,
            "bounty_id": BOUNTY_ID,
            "solver": SOLVER,
            "transaction_hash": TX_HASH,
        }
        rpc_fn = _make_settlement_rpc(submission_hash=sub_hash, evidence_hash=ev_hash)
        settled_res = evaluate_direct_bounty_evidence(settled_payload, rpc_call=rpc_fn)
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

        # Negative control: digest or commit SHA appearing inside a mutable or arbitrary URL path must be rejected!
        digest_in_mutable_release = _valid_evidence()
        digest_in_mutable_release["artifact_url"] = (
            f"https://github.com/NSPG13/agent-bounties/releases/download/v1.0/{ARTIFACT_HEX}.tar.gz"
        )
        res_rel = evaluate_direct_bounty_evidence(digest_in_mutable_release)
        self.assertFalse(res_rel["ok"])
        self.assertIn("mutable_artifact_url", res_rel["errors"])

        digest_in_arbitrary_path = _valid_evidence()
        digest_in_arbitrary_path["artifact_url"] = (
            f"https://example.com/builds/{ARTIFACT_HEX}/output.tar.gz"
        )
        res_arb = evaluate_direct_bounty_evidence(digest_in_arbitrary_path)
        self.assertFalse(res_arb["ok"])
        self.assertIn("mutable_artifact_url", res_arb["errors"])

        wrong_cas_digest = _valid_evidence()
        wrong_cas_digest["artifact_url"] = (
            "https://objects.githubusercontent.com/github-production-release-asset/sha256/" + ("ab" * 32)
        )
        res_wrong_cas = evaluate_direct_bounty_evidence(wrong_cas_digest)
        self.assertFalse(res_wrong_cas["ok"])
        self.assertIn("mutable_artifact_url", res_wrong_cas["errors"])

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

    def test_fabricated_or_unverified_settlement_receipts_never_become_paid(self) -> None:
        base = _valid_evidence()
        base_eval = evaluate_direct_bounty_evidence(base)
        sub_hash = base_eval["submission"]["submission_hash"]
        ev_hash = base_eval["verification"]["evidence_hash"]

        # 1. Fabricated solver-supplied receipt JSON without chain RPC verification must be rejected (`paid: False`)
        fabricated = deepcopy(base)
        fabricated["payment"] = {
            "chain_id": 8453,
            "settlement_token": NATIVE_BASE_USDC,
            "event": "BountySettled",
            "bounty_contract": BOUNTY_CONTRACT,
            "bounty_id": BOUNTY_ID,
            "solver": SOLVER,
            "transaction_hash": TX_HASH,
            "receipt": {
                "status": 1,
                "block_number": 22_100_000,
                "safe_block": 22_100_010,
                "bounty_settled_log": {
                    "address": BOUNTY_CONTRACT,
                    "topics": [BOUNTY_SETTLED_TOPIC0, BOUNTY_ID],
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
        res_fab = evaluate_direct_bounty_evidence(fabricated)
        self.assertFalse(res_fab["ok"])
        self.assertFalse(res_fab["payment"]["paid"])
        self.assertIn("unverified_receipt_missing_proof", res_fab["errors"])
        self.assertIn("unverified_self_reported_receipt_rejected", res_fab["errors"])

        # 2. Non-canonical bounty contract on chain is rejected
        non_canon_rpc = _make_settlement_rpc(
            submission_hash=sub_hash,
            evidence_hash=ev_hash,
            is_canonical_bounty=False,
        )
        res_non_canon = evaluate_direct_bounty_evidence(fabricated, rpc_call=non_canon_rpc)
        self.assertFalse(res_non_canon["ok"])
        self.assertFalse(res_non_canon["payment"]["paid"])
        self.assertIn("unverified_receipt_noncanonical_bounty_contract", res_non_canon["errors"])

        # 3. Mismatched submission_hash / evidence_hash in on-chain log is rejected
        mismatch_rpc = _make_settlement_rpc(
            submission_hash="0x" + "11" * 32,
            evidence_hash="0x" + "22" * 32,
        )
        res_mis = evaluate_direct_bounty_evidence(fabricated, rpc_call=mismatch_rpc)
        self.assertFalse(res_mis["ok"])
        self.assertFalse(res_mis["payment"]["paid"])
        self.assertIn("unverified_receipt_submission_hash_mismatch", res_mis["errors"])
        self.assertIn("unverified_receipt_evidence_hash_mismatch", res_mis["errors"])


if __name__ == "__main__":
    unittest.main()
