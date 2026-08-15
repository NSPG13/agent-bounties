import importlib.util
from pathlib import Path
import unittest


PATH = Path(__file__).with_name("run_open_competition_v2_sepolia_rehearsal.py")
SPEC = importlib.util.spec_from_file_location("open_competition_v2_sepolia_rehearsal", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class SepoliaRehearsalTests(unittest.TestCase):
    def test_actor_derivation_is_stable_and_scoped(self):
        key = bytes.fromhex("11" * 32)
        commit = "22" * 20
        first = MODULE.derived_actor(key, commit, "solver-a")
        second = MODULE.derived_actor(key, commit, "solver-a")
        other = MODULE.derived_actor(key, commit, "solver-b")
        self.assertEqual(first.address, second.address)
        self.assertNotEqual(first.address, other.address)

    def test_proof_summary_drops_sensitive_bulk_bytes(self):
        value = {
            "mode": "groth16",
            "proof_hex": "0x010203",
            "journal_hex": "0x0405",
            "elapsed_seconds": 3.5,
        }
        summary = MODULE.proof_summary(value)
        self.assertEqual(summary["proof_bytes"], 3)
        self.assertEqual(summary["journal_bytes"], 2)
        self.assertNotIn("proof_hex", summary)
        self.assertNotIn("journal_hex", summary)

    def test_private_key_validation_fails_closed(self):
        for value in ("", "0x1", "0x" + "00" * 32):
            with self.assertRaises(MODULE.SepoliaRehearsalError):
                MODULE.normalized_key(value)

    def test_x402_canary_spec_binds_artifact_to_the_journal(self):
        fixture = {
            "scope": {
                "chain_id": 84532,
                "competition": [17] * 20,
                "bounty_id": [34] * 32,
                "solver": [51] * 20,
                "solver_nonce": 3,
                "proof_system": [68] * 32,
                "program_vkey": [85] * 32,
                "source_hash": [102] * 32,
                "elf_hash": [119] * 32,
                "execution_policy_hash": [136] * 32,
                "settlement_policy_hash": [153] * 32,
                "beta_risk_hash": [170] * 32,
            },
            "mode": "maximize_exact_matches",
            "threshold": 1,
            "vectors": [{"expected": 2, "observed": 2, "weight": 1}],
        }
        spec = MODULE.x402_canary_spec(
            fixture,
            "0x" + "11" * 20,
            "0x" + "22" * 32,
            "0x" + "33" * 20,
            3,
        )
        journal = MODULE.rehearsal.expected_journal(fixture)
        self.assertEqual(spec["artifact_hash"], "0x" + journal[192:224].hex())
        self.assertEqual(spec["metric"]["threshold"], "1")


if __name__ == "__main__":
    unittest.main()
