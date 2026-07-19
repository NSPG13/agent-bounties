import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("evaluate_objective_compiler.py")
SPEC = importlib.util.spec_from_file_location("evaluate_objective_compiler", SCRIPT)
evaluation = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(evaluation)


def valid_case():
    return {
        "name": "fixture",
        "max_tasks": 3,
        "solver_budget_usdc": "2.00",
        "expected_terms": ["api", "test"],
    }


def valid_plan():
    return {
        "schema_version": "agent-bounties/cloud-objective-plan-v1",
        "model": "gpt-5.6",
        "tasks": [
            {
                "task_id": "build_api",
                "title": "Build the API",
                "goal": "Publish a bounded API.",
                "depends_on": [],
                "acceptance_criteria": ["The schema fixture validates."],
                "verifier": {"kind": "schema"},
                "evidence_schema": {
                    "type": "object",
                    "required": ["schema_digest"],
                },
                "suggested_solver_reward_usdc": "1.000000",
            },
            {
                "task_id": "test_api",
                "title": "Test the API",
                "goal": "Run the regression test.",
                "depends_on": ["build_api"],
                "acceptance_criteria": ["The committed command exits zero."],
                "verifier": {"kind": "command", "command": "cargo test"},
                "evidence_schema": {
                    "type": "object",
                    "required": ["test_digest"],
                },
                "suggested_solver_reward_usdc": "1.000000",
            },
        ],
        "verification_policy": {
            "model_authority": "advisory_only",
            "committed_before_claim": True,
        },
        "settlement_policy": {
            "protocol": "autonomous-v1",
            "network": "base-mainnet",
            "asset": "native USDC",
            "payout_evidence": "confirmed canonical BountySettled",
        },
    }


class ObjectiveCompilerEvaluationTests(unittest.TestCase):
    def test_valid_plan_conserves_budget_and_covers_terms(self):
        result = evaluation.validate_plan(valid_plan(), valid_case())

        self.assertEqual(result["task_count"], 2)
        self.assertEqual(result["keyword_coverage"], 1.0)

    def test_cycle_fails_closed(self):
        plan = valid_plan()
        plan["tasks"][0]["depends_on"] = ["test_api"]

        with self.assertRaisesRegex(ValueError, "cycle"):
            evaluation.validate_plan(plan, valid_case())


if __name__ == "__main__":
    unittest.main()
