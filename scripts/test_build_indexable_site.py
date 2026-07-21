from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = REPO_ROOT / "scripts" / "build-indexable-site.py"
FIXTURE_DIR = REPO_ROOT / "scripts" / "fixtures"

SPEC = importlib.util.spec_from_file_location("build_indexable_site", SCRIPT_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT_PATH}")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def read_fixture(name: str) -> object:
    return json.loads((FIXTURE_DIR / name).read_text(encoding="utf-8"))


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


class BuildIndexableSiteTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.opportunities = copy.deepcopy(read_fixture("indexable-opportunities.json"))
        self.claim_feed = copy.deepcopy(read_fixture("indexable-claimable-feed.json"))
        self.claim_funnel = copy.deepcopy(read_fixture("indexable-claim-funnel.json"))

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def evidence_paths(self) -> tuple[Path, Path, Path]:
        opportunities = self.root / "opportunities.json"
        claim_feed = self.root / "claimable-feed.json"
        claim_funnel = self.root / "claim-funnel.json"
        write_json(opportunities, self.opportunities)
        write_json(claim_feed, self.claim_feed)
        write_json(claim_funnel, self.claim_funnel)
        return opportunities, claim_feed, claim_funnel

    def build(self, output_name: str = "site") -> tuple[Path, dict[str, object]]:
        paths = self.evidence_paths()
        output = self.root / output_name
        result = MODULE.build_site(
            REPO_ROOT / "site",
            output,
            *paths,
            now=datetime(2026, 7, 20, 20, 5, tzinfo=timezone.utc),
        )
        return output, result

    def expand_for_caps(self) -> None:
        claimable_template = self.opportunities["items"][0]
        feed_template = self.claim_feed[0]
        paid_template = self.opportunities["items"][2]
        self.opportunities["items"] = []
        self.claim_feed = []
        for index in range(1, 23):
            contract = f"0x{index:040x}"
            opportunity = copy.deepcopy(claimable_template)
            opportunity["opportunity_id"] = f"canonical:base-mainnet:{contract}"
            opportunity["source_id"] = contract
            opportunity["title"] = f"Claimable fixture {index}"
            opportunity["goal"] = f"Complete deterministic claimable fixture {index}."
            opportunity["source_url"] = f"https://github.com/NSPG13/agent-bounties/issues/{500 + index}"
            opportunity["updated_at"] = f"2026-07-20T19:{index:02d}:00Z"
            self.opportunities["items"].append(opportunity)
            feed = copy.deepcopy(feed_template)
            feed["bounty_contract"] = contract
            feed["terms"]["document"]["title"] = opportunity["title"]
            feed["terms"]["document"]["goal"] = opportunity["goal"]
            feed["terms"]["document"]["source_url"] = opportunity["source_url"]
            self.claim_feed.append(feed)
        for index in range(1, 8):
            contract = f"0x{100 + index:040x}"
            opportunity = copy.deepcopy(paid_template)
            opportunity["opportunity_id"] = f"canonical:base-mainnet:{contract}"
            opportunity["source_id"] = contract
            opportunity["title"] = f"Settled fixture {index}"
            opportunity["goal"] = f"Inspect deterministic settled fixture {index}."
            opportunity["source_url"] = f"https://github.com/NSPG13/agent-bounties/issues/{600 + index}"
            opportunity["updated_at"] = f"2026-07-20T18:{index:02d}:00Z"
            self.opportunities["items"].append(opportunity)
        self.opportunities["source_statuses"][0]["item_count"] = len(self.opportunities["items"])

    def test_builds_staging_copy_with_bounded_inert_snapshot(self) -> None:
        self.expand_for_caps()
        self.opportunities["items"][0]["title"] = '<script>alert("snapshot")</script>'
        self.claim_feed[0]["terms"]["document"]["title"] = '<script>alert("snapshot")</script>'
        source_hash = hashlib.sha256((REPO_ROOT / "site" / "index.html").read_bytes()).hexdigest()

        output, result = self.build()

        index = (output / "index.html").read_text(encoding="utf-8")
        earn = (output / "earn.html").read_text(encoding="utf-8")
        self.assertEqual(index.count('data-indexable-kind="claimable"'), 5)
        self.assertEqual(index.count('data-indexable-kind="settled"'), 5)
        self.assertEqual(earn.count('data-indexable-kind="claimable"'), 20)
        self.assertEqual(earn.count("data-static-claim-action"), 20)
        self.assertEqual(earn.count("data-live-revalidated"), 0)
        self.assertNotIn('<script>alert("snapshot")</script>', index)
        self.assertIn('&lt;script&gt;alert(&quot;snapshot&quot;)&lt;/script&gt;', index)
        self.assertNotIn("Â", index)
        self.assertNotIn("Â", earn)
        self.assertNotIn("data-analytics-exposure", index)
        self.assertNotIn("data-analytics-exposure", earn)
        self.assertIn("Snapshot as of", index)
        self.assertIn("Claim controls stay disabled", earn)
        for unsupported_guild_field in [
            "data-adventurer-rank",
            "data-mission-difficulty",
            "data-trust-review",
            "data-guild-affiliation",
            "data-poster-eligibility",
        ]:
            self.assertNotIn(unsupported_guild_field, index)
            self.assertNotIn(unsupported_guild_field, earn)
        self.assertEqual(result["homepage_claimable"], 5)
        self.assertEqual(result["homepage_settled"], 5)
        self.assertEqual(result["earn_claimable"], 20)
        self.assertEqual(
            source_hash,
            hashlib.sha256((REPO_ROOT / "site" / "index.html").read_bytes()).hexdigest(),
        )

    def test_canonical_source_failure_leaves_no_output(self) -> None:
        self.opportunities["source_statuses"][0]["available"] = False
        paths = self.evidence_paths()
        output = self.root / "failed-site"

        with self.assertRaisesRegex(MODULE.SnapshotError, "canonical Base opportunity source"):
            MODULE.build_site(
                REPO_ROOT / "site",
                output,
                *paths,
                now=datetime(2026, 7, 20, 20, 5, tzinfo=timezone.utc),
            )

        self.assertFalse(output.exists())

    def test_truncated_canonical_projection_leaves_no_output(self) -> None:
        self.opportunities["source_statuses"][0]["item_count"] += 1
        paths = self.evidence_paths()
        output = self.root / "truncated-projection-site"

        with self.assertRaisesRegex(MODULE.SnapshotError, "complete canonical Base source set"):
            MODULE.build_site(
                REPO_ROOT / "site",
                output,
                *paths,
                now=datetime(2026, 7, 20, 20, 5, tzinfo=timezone.utc),
            )

        self.assertFalse(output.exists())

    def test_feed_projection_disagreement_leaves_no_output(self) -> None:
        self.claim_feed[0]["terms"]["document"]["title"] = "Different public terms"
        paths = self.evidence_paths()
        output = self.root / "failed-site"

        with self.assertRaisesRegex(MODULE.SnapshotError, "disagrees with the opportunity projection"):
            MODULE.build_site(
                REPO_ROOT / "site",
                output,
                *paths,
                now=datetime(2026, 7, 20, 20, 5, tzinfo=timezone.utc),
            )

        self.assertFalse(output.exists())

    def test_claimable_contract_set_disagreement_leaves_no_output(self) -> None:
        baseline_feed = copy.deepcopy(self.claim_feed)
        cases = {
            "projection-only": baseline_feed[1:],
            "feed-only": [
                {
                    **copy.deepcopy(baseline_feed[0]),
                    "bounty_contract": "0x4444444444444444444444444444444444444444",
                },
                *copy.deepcopy(baseline_feed[1:]),
            ],
        }

        for label, claim_feed in cases.items():
            with self.subTest(label=label):
                self.claim_feed = claim_feed
                paths = self.evidence_paths()
                output = self.root / f"set-disagreement-{label}"

                with self.assertRaisesRegex(MODULE.SnapshotError, "contract sets disagree"):
                    MODULE.build_site(
                        REPO_ROOT / "site",
                        output,
                        *paths,
                        now=datetime(2026, 7, 20, 20, 5, tzinfo=timezone.utc),
                    )

                self.assertFalse(output.exists())

    def test_claimable_economic_disagreement_leaves_no_output(self) -> None:
        baseline_feed = copy.deepcopy(self.claim_feed)
        cases = {
            "solver reward": ("solver_reward", "900001"),
            "claim bond": ("claim_bond", "100001"),
            "timeout bonus": ("timeout_bond_pool", "1"),
            "funded amount": ("funded_amount", "1000001"),
            "funding target": ("target_amount", "999999"),
        }

        for label, (field, value) in cases.items():
            with self.subTest(label=label):
                self.claim_feed = copy.deepcopy(baseline_feed)
                self.claim_feed[0][field] = value
                paths = self.evidence_paths()
                output = self.root / f"economic-disagreement-{field}"

                with self.assertRaisesRegex(MODULE.SnapshotError, label):
                    MODULE.build_site(
                        REPO_ROOT / "site",
                        output,
                        *paths,
                        now=datetime(2026, 7, 20, 20, 5, tzinfo=timezone.utc),
                    )

                self.assertFalse(output.exists())

    def test_claimable_source_url_disagreement_leaves_no_output(self) -> None:
        self.claim_feed[0]["terms"]["document"]["source_url"] = (
            "https://github.com/NSPG13/agent-bounties/issues/999"
        )
        paths = self.evidence_paths()
        output = self.root / "source-disagreement-site"

        with self.assertRaisesRegex(MODULE.SnapshotError, "source URL"):
            MODULE.build_site(
                REPO_ROOT / "site",
                output,
                *paths,
                now=datetime(2026, 7, 20, 20, 5, tzinfo=timezone.utc),
            )

        self.assertFalse(output.exists())

    def test_claimable_state_disagreement_leaves_no_output(self) -> None:
        self.claim_feed[0]["verification_ready"] = False
        paths = self.evidence_paths()
        output = self.root / "state-disagreement-site"

        with self.assertRaisesRegex(MODULE.SnapshotError, "verifier-ready claimable work"):
            MODULE.build_site(
                REPO_ROOT / "site",
                output,
                *paths,
                now=datetime(2026, 7, 20, 20, 5, tzinfo=timezone.utc),
            )

        self.assertFalse(output.exists())

    def test_stale_production_evidence_is_rejected(self) -> None:
        paths = self.evidence_paths()
        output = self.root / "failed-site"

        with self.assertRaisesRegex(MODULE.SnapshotError, "freshness window"):
            MODULE.build_site(
                REPO_ROOT / "site",
                output,
                *paths,
                now=datetime(2026, 7, 21, 20, 5, tzinfo=timezone.utc),
            )

        self.assertFalse(output.exists())

    def test_incomplete_settled_record_is_omitted_without_inventing_copy(self) -> None:
        self.opportunities["items"][2]["goal"] = None

        output, result = self.build("settled-omitted-site")

        index = (output / "index.html").read_text(encoding="utf-8")
        self.assertEqual(result["homepage_settled"], 0)
        self.assertNotIn('data-indexable-kind="settled"', index)
        self.assertIn("No canonical record matched this snapshot.", index)

    def test_existing_output_is_never_overwritten(self) -> None:
        output = self.root / "existing"
        output.mkdir()
        sentinel = output / "sentinel.txt"
        sentinel.write_text("keep", encoding="utf-8")
        paths = self.evidence_paths()

        with self.assertRaisesRegex(MODULE.SnapshotError, "already exists"):
            MODULE.build_site(
                REPO_ROOT / "site",
                output,
                *paths,
                now=datetime(2026, 7, 20, 20, 5, tzinfo=timezone.utc),
            )

        self.assertEqual(sentinel.read_text(encoding="utf-8"), "keep")

    def test_generated_staging_copy_passes_site_contract(self) -> None:
        output, _result = self.build("validated-site")

        completed = subprocess.run(
            [
                sys.executable,
                str(REPO_ROOT / "scripts" / "check-site.py"),
                "--site-dir",
                str(output),
                "--require-indexable-snapshot",
            ],
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)

    def test_fixture_mode_cli_accepts_only_committed_validation_evidence(self) -> None:
        output = self.root / "fixture-cli-site"
        completed = subprocess.run(
            [
                sys.executable,
                str(SCRIPT_PATH),
                "--source",
                str(REPO_ROOT / "site"),
                "--output",
                str(output),
                "--opportunities",
                str(FIXTURE_DIR / "indexable-opportunities.json"),
                "--claim-feed",
                str(FIXTURE_DIR / "indexable-claimable-feed.json"),
                "--claim-funnel",
                str(FIXTURE_DIR / "indexable-claim-funnel.json"),
                "--fixture-mode",
            ],
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertTrue((output / "index.html").exists())


if __name__ == "__main__":
    unittest.main()
