import unittest
import sys
import os

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))

from skills.swe_agent_env.swe_agent_runner import MiniSWEAgentEnvironment

class TestMiniSWEAgentEnvironment(unittest.TestCase):
    def test_swe_agent_task_execution(self):
        env = MiniSWEAgentEnvironment()
        res = env.execute_paid_task({
            "repo": "NSPG13/agent-bounties",
            "issue_id": 774
        })
        self.assertTrue(res["settled"])
        self.assertEqual(res["status"], "PASSED_SANDBOX_REGRESSION")
        self.assertEqual(len(res["verifiers"]), 2)
        print("✓ test_swe_agent_task_execution passed")

if __name__ == "__main__":
    unittest.main()
