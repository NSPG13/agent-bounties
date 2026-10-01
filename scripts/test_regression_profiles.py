"""The exact same admission vectors run against the Rust implementation."""
import copy
import json
import unittest
from pathlib import Path
from scripts import regression_profiles

class RegistryTests(unittest.TestCase):
    def test_shared_cases(self):
        profile = regression_profiles.registry()["profiles"][0]
        cases = json.loads((regression_profiles.REGISTRY_PATH.parent / "regression-profile-cases-v1.json").read_text())
        for case in cases:
            with self.subTest(case=case["name"]):
                benchmark = {"engine": "sandboxed_regression_v1", "source": copy.deepcopy(profile["sources"][0]), "runner_manifest": copy.deepcopy(profile["runner_manifest"])}
                for pointer, value in case["changes"].items():
                    parts = pointer.strip("/").split("/")
                    target = benchmark
                    for part in parts[:-1]: target = target[part]
                    target[parts[-1]] = value
                if case["approved"]:
                    self.assertEqual(regression_profiles.require_approved_profile(benchmark)["id"], profile["id"])
                else:
                    with self.assertRaises(ValueError): regression_profiles.require_approved_profile(benchmark)

    def test_all_fields_required(self):
        for profile in regression_profiles.registry()["profiles"]:
            if profile["status"] != "approved": continue
            benchmark = {"engine":"sandboxed_regression_v1", "source":profile["sources"][0], "runner_manifest":profile["runner_manifest"]}
            for key in profile["runner_manifest"]:
                value = copy.deepcopy(benchmark)
                del value["runner_manifest"][key]
                with self.subTest(profile=profile["id"], missing=key), self.assertRaises(ValueError):
                    regression_profiles.require_approved_profile(value)

if __name__ == "__main__": unittest.main()
