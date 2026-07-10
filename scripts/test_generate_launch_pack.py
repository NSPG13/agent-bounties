#!/usr/bin/env python3
"""Tests for generate_launch_pack.py (offline fixtures only)."""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = Path(__file__).resolve().parent / "generate_launch_pack.py"
FIXTURE = Path(__file__).resolve().parent / "fixtures" / "launch_pack_scenarios.json"
OUT_ROOT = ROOT / "target" / "tmp" / "launch-pack-tests"


def run_pack(scenario: str, *extra: str) -> subprocess.CompletedProcess[str]:
    out_dir = OUT_ROOT / scenario
    if out_dir.exists():
        shutil.rmtree(out_dir)
    return subprocess.run(
        [
            sys.executable,
            str(SCRIPT),
            "--scenario-file",
            str(FIXTURE),
            "--scenario",
            scenario,
            "--out-dir",
            str(out_dir),
            *extra,
        ],
        cwd=str(ROOT),
        capture_output=True,
        text=True,
        check=False,
    )


def read_json(scenario: str, name: str) -> dict:
    return json.loads((OUT_ROOT / scenario / name).read_text(encoding="utf-8"))


def read_text(scenario: str, name: str) -> str:
    return (OUT_ROOT / scenario / name).read_text(encoding="utf-8")


class LaunchPackTests(unittest.TestCase):
    def test_only_unfunded_candidates_refuse_funded_and_paid_claims(self) -> None:
        proc = run_pack("only_unfunded")
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)

        summary = read_json("only_unfunded", "summary.json")
        self.assertEqual(summary["truth"]["claimable_funded_count"], 0)
        self.assertEqual(summary["truth"]["reconciled_paid_count"], 0)
        self.assertEqual(summary["truth"]["funding_candidate_count"], 2)
        self.assertIn("No reconciled funding evidence", summary["truth"]["refusals"])

        show_hn = read_text("only_unfunded", "show_hn.md")
        self.assertIn("open funding candidates", show_hn)
        self.assertIn("not funded or claimable", show_hn)
        self.assertNotIn("agents got paid", show_hn.lower())
        self.assertIn("source=launch-pack", show_hn)
        self.assertIn("campaign=show_hn", show_hn)

    def test_outputs_all_platforms_and_quality_rubric(self) -> None:
        proc = run_pack("only_unfunded")
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)

        expected = {
            "show_hn",
            "x_thread",
            "github_discussion",
            "reddit",
            "agent_community",
        }
        for platform in expected:
            self.assertTrue((OUT_ROOT / "only_unfunded" / f"{platform}.md").exists())
            payload = read_json("only_unfunded", f"{platform}.json")
            self.assertEqual(payload["platform"], platform)
            self.assertTrue(payload["requires_human_approval"])
            self.assertFalse(payload["publication_enabled"])

        summary = read_json("only_unfunded", "summary.json")
        criteria = {item["criterion"] for item in summary["evaluation_rubric"]}
        self.assertIn("evidence_truth", criteria)
        self.assertIn("anti_spam_quality", criteria)
        self.assertIn("agent_self_interest", criteria)

    def test_reconciled_paid_proof_allows_paid_language(self) -> None:
        proc = run_pack("reconciled_paid_proof")
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)

        summary = read_json("reconciled_paid_proof", "summary.json")
        self.assertEqual(summary["truth"]["reconciled_paid_count"], 1)
        self.assertEqual(summary["truth"]["verified_unpaid_count"], 0)

        discussion = read_text("reconciled_paid_proof", "github_discussion.md")
        self.assertIn("reconciled payout evidence", discussion)
        self.assertIn("paid proof", discussion)

    def test_malicious_text_is_escaped_and_private_data_excluded(self) -> None:
        proc = run_pack("malicious_injection")
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)

        markdown = read_text("malicious_injection", "reddit.md")
        payload = read_json("malicious_injection", "summary.json")
        self.assertNotIn("<script", markdown.lower())
        self.assertNotIn("wallet_secret", markdown)
        self.assertNotIn("private@example.com", markdown)
        self.assertEqual(payload["truth"]["excluded_private_count"], 1)
        self.assertIn("&lt;script&gt;", markdown)

    def test_stale_endpoint_strict_mode_fails_without_outputs(self) -> None:
        proc = run_pack("stale_endpoints", "--strict")
        self.assertEqual(proc.returncode, 2, proc.stderr + proc.stdout)
        self.assertIn("stale", (proc.stderr + proc.stdout).lower())
        self.assertFalse((OUT_ROOT / "stale_endpoints" / "summary.json").exists())


if __name__ == "__main__":
    unittest.main()
