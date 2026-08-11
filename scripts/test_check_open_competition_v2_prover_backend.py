import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("check_open_competition_v2_prover_backend.py")
SPEC = importlib.util.spec_from_file_location(
    "check_open_competition_v2_prover_backend", SCRIPT
)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class OpenCompetitionV2ProverBackendTests(unittest.TestCase):
    def test_cpu_plonk_fails_below_published_minimum(self) -> None:
        report = MODULE.inspect(
            "plonk", "cpu", environ={}, memory_bytes=63 * 1024**3
        )
        self.assertFalse(report["ready"])
        self.assertEqual(report["required_memory_gib"], 64)
        self.assertIn("V2_PROVER_MEMORY_INSUFFICIENT", report["blockers"])

    def test_cpu_plonk_accepts_exact_minimum(self) -> None:
        report = MODULE.inspect(
            "plonk", "cpu", environ={}, memory_bytes=64 * 1024**3
        )
        self.assertTrue(report["ready"])

    def test_network_requires_key_without_disclosing_it(self) -> None:
        report = MODULE.inspect(
            "plonk",
            "network",
            environ={},
            capabilities={"backends": ["cpu", "network"]},
        )
        self.assertFalse(report["ready"])
        self.assertIn("V2_PROVER_NETWORK_KEY_MISSING", report["blockers"])
        self.assertNotIn("private", str(report).lower())

    def test_network_requires_feature_enabled_runner(self) -> None:
        report = MODULE.inspect(
            "plonk",
            "network",
            environ={"NETWORK_PRIVATE_KEY": "secret"},
            capabilities={"backends": ["cpu"]},
        )
        self.assertFalse(report["ready"])
        self.assertIn("V2_PROVER_RUNNER_LACKS_NETWORK", report["blockers"])

    def test_network_is_ready_with_key_and_capability(self) -> None:
        report = MODULE.inspect(
            "plonk",
            "network",
            environ={"NETWORK_PRIVATE_KEY": "secret"},
            capabilities={"backends": ["cpu", "network"]},
        )
        self.assertTrue(report["ready"])
        self.assertEqual(report["blockers"], [])


if __name__ == "__main__":
    unittest.main()
