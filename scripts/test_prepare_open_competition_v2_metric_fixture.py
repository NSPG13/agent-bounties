import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("prepare_open_competition_v2_metric_fixture.py")
SPEC = importlib.util.spec_from_file_location("open_competition_v2_fixture", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class OpenCompetitionV2MetricFixtureTests(unittest.TestCase):
    def scope(self) -> dict:
        return {
            "chain_id": 84532,
            "competition": "0x" + "11" * 20,
            "bounty_id": "0x" + "22" * 32,
            "solver": "0x" + "33" * 20,
            "solver_nonce": 7,
            "proof_system": "groth16",
            "program_vkey": "0x" + "66" * 32,
            "source_hash": "0x" + "77" * 32,
            "elf_hash": "0x" + "88" * 32,
            "execution_policy_hash": "0x" + "bb" * 32,
            "settlement_policy_hash": "0x" + "dd" * 32,
            "beta_risk_hash": "0x" + "ee" * 32,
        }

    def test_policy_hash_excludes_solver_observations(self) -> None:
        template = {
            "mode": "maximize_exact_matches",
            "threshold": 2,
            "vectors": [{"expected": 3, "observed": 1, "weight": 4}],
        }
        first = MODULE.bind(template, self.scope())
        template["vectors"][0]["observed"] = 3
        second = MODULE.bind(template, self.scope())
        self.assertEqual(
            first["expected"]["verification_policy_hash"],
            second["expected"]["verification_policy_hash"],
        )
        self.assertNotEqual(first["vectors"], second["vectors"])

    def test_golden_policy_hash_matches_rust_vector(self) -> None:
        fixture = __import__("json").loads(
            (
                MODULE.Path(__file__).parents[1]
                / "programs/public-vector-metric-v1/fixtures/golden-v1.json"
            ).read_text(encoding="utf-8")
        )
        self.assertEqual(
            MODULE.verification_policy_hash(
                fixture["mode"], int(fixture["threshold"]), fixture["vectors"]
            ),
            fixture["expected"]["verification_policy_hash"],
        )

    def test_invalid_weight_and_proof_system_fail_closed(self) -> None:
        template = {
            "mode": "all_equal",
            "threshold": 0,
            "vectors": [{"expected": 1, "observed": 1, "weight": 0}],
        }
        with self.assertRaisesRegex(ValueError, "positive uint32"):
            MODULE.bind(template, self.scope())
        template["vectors"][0]["weight"] = 1
        scope = self.scope()
        scope["proof_system"] = "other"
        with self.assertRaisesRegex(ValueError, "groth16 or plonk"):
            MODULE.bind(template, scope)


if __name__ == "__main__":
    unittest.main()
