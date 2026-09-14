#!/usr/bin/env python3
"""Comprehensive test suite for stalled claimed and submitted bounty diagnostics.

Tests deterministic classification, deadline calculations, single next actions,
and strictly enforces anti-inference integrity guards against untrusted GitHub/AI signals.
"""

from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys
import unittest

from scripts.stalled_bounty_diagnostics import (
    ACTION_ATTEST_AND_SETTLE,
    ACTION_EXPIRE_CLAIM,
    ACTION_EXPIRE_SUBMISSION,
    ACTION_FAILOVER_VERIFIER,
    ACTION_REFRESH_INDEX_STATE,
    ACTION_RESOLVE_MISSING_TERMS,
    ACTION_SUBMIT_WORK,
    ACTION_URGENT_ATTEST_AND_SETTLE,
    CLASS_CLAIM_EXPIRING,
    CLASS_HEALTHY_CLAIMED,
    CLASS_SETTLED,
    CLASS_SUBMITTED,
    CLASS_TERMINAL,
    CLASS_VERIFICATION_EXPIRING,
    CLASS_VERIFIER_UNAVAILABLE,
    CanonicalEvent,
    CanonicalIntegrityError,
    BountyContractConfig,
    BountyItemDiagnosis,
    BountyLifecycleState,
    DiagnosticsReport,
    IndexerSyncStatus,
    VerifierFleetStatus,
    diagnose_backlog,
    diagnose_bounty,
    format_iso_timestamp,
    parse_canonical_events,
    to_discovery_projection,
)


class TestStalledBountyDiagnostics(unittest.TestCase):
    """Test suite verifying deterministic stalled bounty diagnostics and recovery actions."""

    def setUp(self) -> None:
        self.fixtures_path = Path("ops/fixtures/stalled-bounty-cases.json")
        self.assertTrue(self.fixtures_path.is_file(), f"Fixture file not found: {self.fixtures_path}")
        self.fixtures_data = json.loads(self.fixtures_path.read_text(encoding="utf-8"))

    def test_healthy_claimed_bounty_classification(self) -> None:
        """Verify healthy claimed bounty where solver has active claim window remaining."""
        contract = BountyContractConfig(
            bounty_id="0x1111111111111111111111111111111111111111111111111111111111111111",
            contract_address="0x1000000000000000000000000000000000000001",
            title="Healthy Claimed Active Bounty",
            claim_window_seconds=604800,
            terms_hash="0xterms01",
        )
        state = BountyLifecycleState(
            status="claimed",
            round=1,
            solver="0xsolver01",
            claim_expires_at=1785500000,
        )
        now = 1785000000  # 500,000s remaining

        diagnosis = diagnose_bounty(contract, state, reference_timestamp=now)
        self.assertEqual(diagnosis.classification, CLASS_HEALTHY_CLAIMED)
        self.assertEqual(diagnosis.current_status, "claimed")
        self.assertFalse(diagnosis.is_stalled)
        self.assertFalse(diagnosis.is_terminal)
        self.assertEqual(diagnosis.next_action, ACTION_SUBMIT_WORK)
        self.assertEqual(diagnosis.deadline, 1785500000)
        self.assertEqual(diagnosis.seconds_remaining, 500000)

    def test_boundary_claim_expiration_timing(self) -> None:
        """Verify exact boundary second behavior for claim_expiring deadlines."""
        contract = BountyContractConfig(
            bounty_id="0x2222222222222222222222222222222222222222222222222222222222222222",
            contract_address="0x2000000000000000000000000000000000000002",
            title="Claim Boundary Test",
            claim_window_seconds=604800,
            terms_hash="0xterms02",
        )
        claim_deadline = 1785500000
        state = BountyLifecycleState(
            status="claimed",
            round=1,
            solver="0xsolver02",
            claim_expires_at=claim_deadline,
        )

        # 1. Exact second before deadline: solver should urgently submit_work
        diag_before = diagnose_bounty(contract, state, reference_timestamp=claim_deadline - 1)
        self.assertEqual(diag_before.classification, CLASS_CLAIM_EXPIRING)
        self.assertTrue(diag_before.is_stalled)
        self.assertEqual(diag_before.next_action, ACTION_SUBMIT_WORK)
        self.assertEqual(diag_before.seconds_remaining, 1)
        self.assertEqual(diag_before.deadline, claim_deadline)

        # 2. Exact second at deadline (T = deadline): solver still within window
        diag_at = diagnose_bounty(contract, state, reference_timestamp=claim_deadline)
        self.assertEqual(diag_at.classification, CLASS_CLAIM_EXPIRING)
        self.assertTrue(diag_at.is_stalled)
        self.assertEqual(diag_at.next_action, ACTION_SUBMIT_WORK)
        self.assertEqual(diag_at.seconds_remaining, 0)
        self.assertEqual(diag_at.deadline, claim_deadline)

        # 3. Exact second after deadline (T > deadline): claim expired, next action expire_claim
        diag_after = diagnose_bounty(contract, state, reference_timestamp=claim_deadline + 1)
        self.assertEqual(diag_after.classification, CLASS_CLAIM_EXPIRING)
        self.assertTrue(diag_after.is_stalled)
        self.assertEqual(diag_after.next_action, ACTION_EXPIRE_CLAIM)
        self.assertEqual(diag_after.seconds_remaining, -1)
        self.assertEqual(diag_after.deadline, claim_deadline)

    def test_healthy_submitted_bounty_awaits_verification(self) -> None:
        """Verify healthy submitted bounty with active verification window and healthy verifiers."""
        contract = BountyContractConfig(
            bounty_id="0x3333333333333333333333333333333333333333333333333333333333333333",
            contract_address="0x3000000000000000000000000000000000000003",
            title="Healthy Submitted Bounty",
            verification_window_seconds=7200,
            terms_hash="0xterms03",
        )
        v_deadline = 1785007200
        state = BountyLifecycleState(
            status="submitted",
            round=1,
            solver="0xsolver03",
            verification_expires_at=v_deadline,
            submission_hash="0xsub03",
        )
        v_status = VerifierFleetStatus(verifiers_available=True, healthy_count=3, required_threshold=2)
        now = 1785000000  # 7200s remaining

        diagnosis = diagnose_bounty(contract, state, verifier_status=v_status, reference_timestamp=now)
        self.assertEqual(diagnosis.classification, CLASS_SUBMITTED)
        self.assertEqual(diagnosis.current_status, "submitted")
        self.assertFalse(diagnosis.is_stalled)
        self.assertEqual(diagnosis.next_action, ACTION_ATTEST_AND_SETTLE)
        self.assertEqual(diagnosis.deadline, v_deadline)
        self.assertEqual(diagnosis.seconds_remaining, 7200)

    def test_boundary_verification_expiration_timing(self) -> None:
        """Verify boundary seconds for verification_expiring and submission expiration."""
        contract = BountyContractConfig(
            bounty_id="0x3333333333333333333333333333333333333333333333333333333333333333",
            contract_address="0x3000000000000000000000000000000000000003",
            title="Verification Boundary Test",
            verification_window_seconds=7200,
            terms_hash="0xterms03",
        )
        v_deadline = 1785007200
        state = BountyLifecycleState(
            status="submitted",
            round=1,
            solver="0xsolver03",
            verification_expires_at=v_deadline,
            submission_hash="0xsub03",
        )
        v_status = VerifierFleetStatus(verifiers_available=True, healthy_count=2, required_threshold=2)

        # 1. Exact second before deadline: urgent attestation required
        diag_before = diagnose_bounty(
            contract, state, verifier_status=v_status, reference_timestamp=v_deadline - 1
        )
        self.assertEqual(diag_before.classification, CLASS_VERIFICATION_EXPIRING)
        self.assertTrue(diag_before.is_stalled)
        self.assertEqual(diag_before.next_action, ACTION_URGENT_ATTEST_AND_SETTLE)
        self.assertEqual(diag_before.deadline, v_deadline)

        # 2. Exact second after deadline: verification expired, refund solver bond via expireSubmission
        diag_after = diagnose_bounty(
            contract, state, verifier_status=v_status, reference_timestamp=v_deadline + 1
        )
        self.assertEqual(diag_after.classification, CLASS_VERIFICATION_EXPIRING)
        self.assertTrue(diag_after.is_stalled)
        self.assertEqual(diag_after.next_action, ACTION_EXPIRE_SUBMISSION)
        self.assertEqual(diag_after.deadline, v_deadline)

    def test_verifier_outage_classification(self) -> None:
        """Verify detection of verifier quorum outage or unavailable fleet."""
        contract = BountyContractConfig(
            bounty_id="0x4444444444444444444444444444444444444444444444444444444444444444",
            contract_address="0x4000000000000000000000000000000000000004",
            title="Verifier Outage Bounty",
            verification_window_seconds=7200,
            terms_hash="0xterms04",
        )
        state = BountyLifecycleState(
            status="submitted",
            round=1,
            solver="0xsolver04",
            verification_expires_at=1785007200,
            submission_hash="0xsub04",
        )
        outage_status = VerifierFleetStatus(
            verifiers_available=False,
            healthy_count=0,
            required_threshold=2,
            error="HTTP 503 Service Unavailable on verifier nodes",
        )

        diagnosis = diagnose_bounty(
            contract, state, verifier_status=outage_status, reference_timestamp=1785001000
        )
        self.assertEqual(diagnosis.classification, CLASS_VERIFIER_UNAVAILABLE)
        self.assertTrue(diagnosis.is_stalled)
        self.assertEqual(diagnosis.next_action, ACTION_FAILOVER_VERIFIER)
        self.assertEqual(diagnosis.deadline, 1785007200)

    def test_stale_indexer_projection_handling(self) -> None:
        """Verify stale indexer lag detection prevents unsafe automated transitions."""
        contract = BountyContractConfig(
            bounty_id="0x7777777777777777777777777777777777777777777777777777777777777777",
            contract_address="0x7000000000000000000000000000000000000007",
            title="Stale Indexer Bounty",
            claim_window_seconds=604800,
            terms_hash="0xterms07",
        )
        state = BountyLifecycleState(
            status="claimed",
            round=1,
            solver="0xsolver07",
            claim_expires_at=1785500000,
        )
        stale_status = IndexerSyncStatus(is_stale=True, lag_blocks=500, heartbeat_age_seconds=1800)

        diagnosis = diagnose_bounty(
            contract, state, indexer_status=stale_status, reference_timestamp=1785000000
        )
        self.assertTrue(diagnosis.is_stalled)
        self.assertEqual(diagnosis.next_action, ACTION_REFRESH_INDEX_STATE)
        self.assertEqual(diagnosis.deadline, 1785500000)

    def test_missing_terms_stall_handling(self) -> None:
        """Verify bounty with missing or unresolvable terms document is halted for resolution."""
        contract = BountyContractConfig(
            bounty_id="0x6666666666666666666666666666666666666666666666666666666666666666",
            contract_address="0x6000000000000000000000000000000000000006",
            title="Missing Terms Bounty",
            claim_window_seconds=604800,
            terms_hash="",  # Empty terms hash
        )
        state = BountyLifecycleState(
            status="claimed",
            round=1,
            solver="0xsolver06",
            claim_expires_at=1785500000,
        )

        diagnosis = diagnose_bounty(
            contract, state, has_valid_terms_document=False, reference_timestamp=1785000000
        )
        self.assertTrue(diagnosis.is_stalled)
        self.assertEqual(diagnosis.next_action, ACTION_RESOLVE_MISSING_TERMS)
        self.assertEqual(diagnosis.deadline, 1785500000)

    def test_canonical_bountysettled_event_marks_settled(self) -> None:
        """Verify canonical BountySettled lifecycle event marks bounty settled and terminal."""
        contract = BountyContractConfig(
            bounty_id="0x5555555555555555555555555555555555555555555555555555555555555555",
            contract_address="0x5000000000000000000000000000000000000005",
            title="Settled Bounty",
            terms_hash="0xterms05",
        )
        state = BountyLifecycleState(
            status="settled",
            round=1,
            solver="0xsolver05",
        )
        settled_event = CanonicalEvent(
            event_name="BountySettled",
            block_number=12000,
            block_timestamp=1784900000,
            tx_hash="0xsettledtx05",
            payload={"round": 1, "solver": "0xsolver05", "solverPayout": 1000000},
        )

        diagnosis = diagnose_bounty(
            contract, state, canonical_events=[settled_event], reference_timestamp=1785000000
        )
        self.assertEqual(diagnosis.classification, CLASS_SETTLED)
        self.assertEqual(diagnosis.current_status, "settled")
        self.assertFalse(diagnosis.is_stalled)
        self.assertTrue(diagnosis.is_terminal)
        self.assertIsNone(diagnosis.next_action)
        self.assertIsNone(diagnosis.deadline)
        self.assertTrue(diagnosis.canonical_evidence["has_bountysettled_event"])

    def test_anti_inference_rejects_github_and_ai_opinion_tampering(self) -> None:
        """Strictly enforce that untrusted GitHub and AI opinion signals are rejected."""
        contract = BountyContractConfig(
            bounty_id="0x1111111111111111111111111111111111111111111111111111111111111111",
            contract_address="0x1000000000000000000000000000000000000001",
        )
        state = BountyLifecycleState(status="claimed", solver="0xsolver01")

        # Attempting to infer settlement from GitHub PR state must raise CanonicalIntegrityError
        with self.assertRaises(CanonicalIntegrityError):
            diagnose_bounty(
                contract,
                state,
                external_github_state={"pr_merged": True, "simulate_settlement": True},
            )

        # Attempting to infer settlement from AI opinion must raise CanonicalIntegrityError
        with self.assertRaises(CanonicalIntegrityError):
            diagnose_bounty(
                contract,
                state,
                ai_opinion={"verdict": "settled", "confidence": 0.99},
            )

    def test_fixture_suite_backlog_diagnostics_report(self) -> None:
        """Run diagnostic report against the full fixture suite."""
        cases = self.fixtures_data.get("cases", [])
        ref_time = self.fixtures_data.get("reference_timestamp", 1785000000)

        report = diagnose_backlog(cases, reference_timestamp=ref_time)
        self.assertIsInstance(report, DiagnosticsReport)
        self.assertEqual(report.summary["total_bounties"], len(cases))
        self.assertGreater(report.summary["stalled_backlog"], 0)
        self.assertIn("## Executive Summary", report.operations_markdown)
        self.assertIn("## Prioritized Stalled Work Backlog", report.operations_markdown)

        # Verify discovery projection structure
        discovery = to_discovery_projection(report)
        self.assertEqual(discovery["schema"], "agent-bounties/stalled-bounty-discovery-v1")
        self.assertIn("backlog", discovery)

    def test_cli_execution_json_and_markdown(self) -> None:
        """Test invoking the diagnostics CLI tool directly."""
        cmd_json = [
            sys.executable,
            "scripts/stalled_bounty_diagnostics.py",
            "--fixtures",
            str(self.fixtures_path),
            "--format",
            "json",
        ]
        res_json = subprocess.run(cmd_json, capture_output=True, text=True, check=True)
        out_json = json.loads(res_json.stdout)
        self.assertIn("summary", out_json)
        self.assertIn("backlog", out_json)

        cmd_md = [
            sys.executable,
            "scripts/stalled_bounty_diagnostics.py",
            "--fixtures",
            str(self.fixtures_path),
            "--format",
            "markdown",
        ]
        res_md = subprocess.run(cmd_md, capture_output=True, text=True, check=True)
        self.assertIn("# Stalled Bounty Operations & Backlog Diagnostic Report", res_md.stdout)


if __name__ == "__main__":
    unittest.main()
