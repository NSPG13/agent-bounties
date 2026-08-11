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


if __name__ == "__main__":
    unittest.main()
