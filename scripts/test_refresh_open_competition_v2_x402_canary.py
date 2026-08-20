import importlib.util
from pathlib import Path
import sys
import unittest


PATH = Path(__file__).with_name("refresh_open_competition_v2_x402_canary.py")
sys.path.insert(0, str(PATH.parent))
SPEC = importlib.util.spec_from_file_location("refresh_open_competition_v2_x402_canary", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class RefreshX402CanaryTests(unittest.TestCase):
    def test_runtime_bundle_selects_only_reviewed_public_vector_profile(self):
        profile = {
            "profile_id": "public-vector-metric-v1",
            "classification": "reviewed",
            "program_vkey": "0x" + "11" * 32,
        }
        runtime = {
            "source_commit": "22" * 20,
            "settlement_token": "0x" + "33" * 20,
            "factory_contract": "0x" + "44" * 20,
            "beta_risk_hash": "0x" + "55" * 32,
            "metric_programs": [{"profile_id": "other"}, profile],
        }
        bundle = MODULE.runtime_bundle(runtime)
        self.assertIs(bundle["metric_profile"], profile)
        self.assertEqual(bundle["factory"]["address"], runtime["factory_contract"])
        self.assertEqual(bundle["chain_id"], 84532)
        self.assertEqual(MODULE.runtime_bundle(runtime, 8453)["chain_id"], 8453)

    def test_runtime_bundle_rejects_disabled_profile(self):
        runtime = {
            "source_commit": "22" * 20,
            "settlement_token": "0x" + "33" * 20,
            "factory_contract": "0x" + "44" * 20,
            "beta_risk_hash": "0x" + "55" * 32,
            "metric_programs": [
                {"profile_id": "public-vector-metric-v1", "classification": "disabled"}
            ],
        }
        with self.assertRaises(MODULE.CanaryRefreshError):
            MODULE.runtime_bundle(runtime)

    def test_replacement_deadline_and_identity_are_deterministic(self):
        self.assertEqual(
            MODULE.replacement_funding_deadline(1_787_143_048, 1),
            1_787_747_848,
        )

    def test_empty_bytecode_responses_are_not_deployments(self):
        self.assertFalse(MODULE.has_runtime_code("0x"))
        self.assertFalse(MODULE.has_runtime_code("0x0"))
        self.assertTrue(MODULE.has_runtime_code("0x6000"))

    def test_rehearsal_preserves_superseded_evidence_and_rebinds_first_proven(self):
        old = {
            "competition": "0x" + "11" * 20,
            "bounty_id": "0x" + "22" * 32,
            "proof_deadline": 10,
            "active": True,
        }
        new = {
            "competition": "0x" + "33" * 20,
            "bounty_id": "0x" + "44" * 32,
            "active": True,
        }
        evidence = {
            "replacement_id": 1,
            "superseded_recovery": {"recovered": True},
        }
        document = MODULE.replace_rehearsal_canary({"x402_canary": old}, new, evidence)
        self.assertEqual(document["x402_canary"], new)
        self.assertFalse(document["superseded_x402_canaries"][0]["active"])
        self.assertTrue(document["superseded_x402_canaries"][0]["recovery"]["recovered"])
        self.assertEqual(document["groth16_first_proven"]["competition"], new["competition"])
        self.assertTrue(document["groth16_first_proven"]["settlement_deferred_to_x402"])


if __name__ == "__main__":
    unittest.main()
