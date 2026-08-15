import importlib.util
from pathlib import Path
import unittest


PATH = Path(__file__).with_name("promote_open_competition_v2_beta2_release.py")
SPEC = importlib.util.spec_from_file_location("promote_open_competition_v2_beta2_release", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class PromotionTests(unittest.TestCase):
    def fixture(self):
        address = lambda byte: "0x" + byte * 40
        digest = lambda byte: "0x" + byte * 64
        bundle = {
            "protocol_version": MODULE.release.PROTOCOL_VERSION,
            "network": "base-mainnet",
            "source_tree_hash": digest("1"),
            "repository_subject": {"hash": digest("2")},
            "risk": {"hash": digest("3")},
            "factory": {"address": address("1"), "runtime_code_hash": digest("4")},
            "implementation": {"address": address("2"), "runtime_code_hash": digest("5")},
            "settlement_token": address("3"),
            "groth16_verifier": {"address": address("4"), "verifier_hash": digest("6"), "runtime_code_hash": digest("7")},
            "groth16_adapter": {"address": address("5"), "runtime_code_hash": digest("8")},
            "plonk_verifier": {"address": address("6"), "verifier_hash": digest("9"), "runtime_code_hash": digest("a")},
            "plonk_adapter": {"address": address("7"), "runtime_code_hash": digest("b")},
            "source_commit": "c" * 40,
            "sp1": {"patched_source_commit": "d" * 40, "circuit_version": "safe"},
            "metric_profile": {
                "profile_id": "public-vector-metric-v1",
                "program_vkey": digest("c"),
                "source_hash": digest("d"),
                "elf_hash": digest("e"),
                "journal_schema_hash": digest("f"),
                "metric_program_hash": digest("0"),
            },
            "activation": {
                "public_creation_enabled": False,
                "broker_canary_enabled": False,
                "sepolia_broker_rehearsal_enabled": False,
            },
        }
        deployment_runtime = MODULE.release.runtime_manifest(bundle, 123)
        gates = {
            "subject_hash": digest("2"),
            "prelaunch_complete": True,
            "broker_canary_ready": True,
            "sepolia_broker_rehearsal_ready": False,
            "public_beta_launch_complete": False,
            "graduation_complete": False,
        }
        return bundle, {"complete": True, "runtime_manifest": deployment_runtime}, gates

    def test_promotion_changes_only_activation_and_gate_evidence(self):
        bundle, deployment, gates = self.fixture()
        promoted, runtime = MODULE.promote(bundle, deployment, gates)
        self.assertTrue(promoted["activation"]["broker_canary_enabled"])
        self.assertFalse(promoted["activation"]["public_creation_enabled"])
        self.assertTrue(runtime["proof_broker_enabled"])
        self.assertEqual(runtime["factory_contract"], bundle["factory"]["address"])
        self.assertEqual(runtime["deployment_block"], 123)

    def test_promotion_rejects_identity_drift(self):
        bundle, deployment, gates = self.fixture()
        deployment["runtime_manifest"]["factory_contract"] = "0x" + "9" * 40
        with self.assertRaises(MODULE.PromotionError):
            MODULE.promote(bundle, deployment, gates)


if __name__ == "__main__":
    unittest.main()
