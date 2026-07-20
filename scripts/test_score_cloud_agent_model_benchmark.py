import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("score_cloud_agent_model_benchmark.py")
SPEC = importlib.util.spec_from_file_location("score_cloud_agent_model_benchmark", SCRIPT)
scoring = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(scoring)


class CloudAgentModelBenchmarkTests(unittest.TestCase):
    def test_selects_cheapest_candidate_inside_quality_band(self):
        candidates = [
            {
                "model": "gpt-5.6-sol",
                "reasoning_effort": "low",
                "pass_rate": 1.0,
                "keyword_coverage": 1.0,
                "average_cost_per_case_usd": 0.2,
                "p95_latency_ms": 100,
            },
            {
                "model": "gpt-5.6-terra",
                "reasoning_effort": "medium",
                "pass_rate": 1.0,
                "keyword_coverage": 0.96,
                "average_cost_per_case_usd": 0.08,
                "p95_latency_ms": 120,
            },
            {
                "model": "gpt-5.6-luna",
                "reasoning_effort": "low",
                "pass_rate": 1.0,
                "keyword_coverage": 0.90,
                "average_cost_per_case_usd": 0.02,
                "p95_latency_ms": 80,
            },
        ]

        selected = scoring.select_candidate(candidates, 0.75)

        self.assertEqual(selected["model"], "gpt-5.6-terra")

    def test_rejects_partial_passes(self):
        selected = scoring.select_candidate(
            [
                {
                    "model": "gpt-5.6-luna",
                    "reasoning_effort": "low",
                    "pass_rate": 0.99,
                    "keyword_coverage": 1.0,
                    "average_cost_per_case_usd": 0.01,
                    "p95_latency_ms": 50,
                }
            ],
            0.75,
        )

        self.assertIsNone(selected)


if __name__ == "__main__":
    unittest.main()
