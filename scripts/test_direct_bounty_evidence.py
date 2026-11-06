#!/usr/bin/env python3
"""Tests for direct bounty evidence checklist validation (Issue #686)."""

from __future__ import annotations

import json
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from direct_bounty_evidence import (
    DirectBountyEvidenceError,
    format_compact_evidence_checklist,
    validate_direct_bounty_evidence,
)


VALID_EVIDENCE = {
    "schema_version": "agent-bounties/direct-evidence-v1",
    "submission": {
        "repository": "NSPG13/agent-bounties",
        "source_commit": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
        "subdirectory": "scripts",
        "pull_request_url": "https://github.com/NSPG13/agent-bounties/pull/686",
    },
    "verification": {
        "check_run_urls": [
            "https://github.com/NSPG13/agent-bounties/actions/runs/123456789"
        ],
        "artifact_digest": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "artifact_url": "https://api.agentbounties.app/v1/artifacts/686/digest",
    },
    "payment": {
        "settlement_event": "BountySettled",
        "tx_hash": "0x8c172a34188cc68cf61f7293825e3c93e025a029b878436d34aafd2103c2e70e",
        "amount_usdc": 2.0,
    },
}


class DirectBountyEvidenceTests(unittest.TestCase):
    def test_valid_evidence_passes(self) -> None:
        validated = validate_direct_bounty_evidence(VALID_EVIDENCE)
        self.assertEqual(validated["schema_version"], "agent-bounties/direct-evidence-v1")

        compact = format_compact_evidence_checklist(VALID_EVIDENCE)
        self.assertIn("NSPG13/agent-bounties", compact)
        self.assertIn("BountySettled", compact)
        self.assertLess(
            len(compact), 256, "Compact checklist output must be short enough for API/MCP responses"
        )

    def test_rejects_non_https_artifact_url(self) -> None:
        malformed = json.loads(json.dumps(VALID_EVIDENCE))
        malformed["verification"]["artifact_url"] = "http://api.agentbounties.app/artifact"
        with self.assertRaises(DirectBountyEvidenceError) as ctx:
            validate_direct_bounty_evidence(malformed)
        self.assertIn("HTTPS", str(ctx.exception))

    def test_rejects_invalid_commit_hash(self) -> None:
        malformed = json.loads(json.dumps(VALID_EVIDENCE))
        malformed["submission"]["source_commit"] = "not-a-sha"
        with self.assertRaises(DirectBountyEvidenceError) as ctx:
            validate_direct_bounty_evidence(malformed)
        self.assertIn("source_commit", str(ctx.exception))

    def test_rejects_missing_section(self) -> None:
        malformed = json.loads(json.dumps(VALID_EVIDENCE))
        del malformed["payment"]
        with self.assertRaises(DirectBountyEvidenceError) as ctx:
            validate_direct_bounty_evidence(malformed)
        self.assertIn("payment", str(ctx.exception))

    def test_rejects_non_settled_payment_event(self) -> None:
        malformed = json.loads(json.dumps(VALID_EVIDENCE))
        malformed["payment"]["settlement_event"] = "PRMerged"
        with self.assertRaises(DirectBountyEvidenceError) as ctx:
            validate_direct_bounty_evidence(malformed)
        self.assertIn("BountySettled", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
