import copy
import importlib.util
import json
from pathlib import Path
import unittest


PATH = Path(__file__).with_name("seed_open_competition_v2_discovery.py")
SPEC = importlib.util.spec_from_file_location("seed_open_competition_v2_discovery", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)
MANIFEST_PATH = PATH.parents[1] / "ops/open-competition-v2-discovery-seed-v1.json"


class DiscoverySeedTests(unittest.TestCase):
    def manifest(self):
        return json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))

    def test_exact_seed_has_five_profitable_fully_funded_competitions(self):
        economics = MODULE.validate_manifest(self.manifest())
        self.assertEqual(economics["funding_per_competition"], 3_050_000)
        self.assertEqual(economics["total_funding"], 15_250_000)
        self.assertEqual(economics["net_prize"], 2_890_000)

    def test_seed_rejects_inventory_or_economic_drift(self):
        manifest = self.manifest()
        manifest["tasks"].pop()
        with self.assertRaisesRegex(MODULE.SeedError, "exactly five"):
            MODULE.validate_manifest(manifest)
        manifest = self.manifest()
        manifest["economics"]["hosted_proof_fee_base_units"] = 1_100_000
        with self.assertRaisesRegex(MODULE.SeedError, "cost assumptions"):
            MODULE.validate_manifest(manifest)

    def test_every_task_binds_its_identity_and_json_schema(self):
        manifest = self.manifest()
        MODULE.validate_manifest(manifest)
        for task in manifest["tasks"]:
            equals = {
                (requirement.get("pointer"), requirement.get("expected"))
                for requirement in task["requirements"]
                if requirement["kind"] == "json_pointer_string_equals"
            }
            self.assertIn(("/task_id", task["seed_id"]), equals)
            self.assertIn(("/schema_version", task["artifact_template"]["schema_version"]), equals)

    def test_creation_identity_is_stable_and_release_bound(self):
        manifest = self.manifest()
        economics = MODULE.validate_manifest(manifest)
        task = manifest["tasks"][0]
        profile = {
            "profile": {
                "program_vkey": "0x" + "11" * 32,
                "source_hash": "0x" + "22" * 32,
                "elf_hash": "0x" + "33" * 32,
                "journal_schema_hash": "0x" + "44" * 32,
                "metric_program_hash": "0x" + "55" * 32,
            },
            "verification_policy_hash": "0x" + "66" * 32,
        }
        release = {"release_hash": "0x" + "77" * 32, "beta_risk_hash": "0x" + "88" * 32}
        first = MODULE.creation_body(
            manifest=manifest,
            economics=economics,
            task=task,
            release=release,
            creator="0x" + "99" * 20,
            funding_deadline=1_800_000_000,
            profile_document=profile,
        )
        second = MODULE.creation_body(
            manifest=manifest,
            economics=economics,
            task=copy.deepcopy(task),
            release=copy.deepcopy(release),
            creator="0x" + "99" * 20,
            funding_deadline=1_800_000_000,
            profile_document=copy.deepcopy(profile),
        )
        self.assertEqual(first, second)
        changed = copy.deepcopy(release)
        changed["release_hash"] = "0x" + "aa" * 32
        third = MODULE.creation_body(
            manifest=manifest,
            economics=economics,
            task=task,
            release=changed,
            creator="0x" + "99" * 20,
            funding_deadline=1_800_000_000,
            profile_document=profile,
        )
        self.assertNotEqual(first["creation_nonce"], third["creation_nonce"])


if __name__ == "__main__":
    unittest.main()
