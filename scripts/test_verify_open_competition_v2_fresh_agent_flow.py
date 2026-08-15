import importlib.util
from pathlib import Path
import unittest


PATH = Path(__file__).with_name("verify_open_competition_v2_fresh_agent_flow.py")
SPEC = importlib.util.spec_from_file_location("verify_open_competition_v2_fresh_agent_flow", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


def fixtures():
    deployer = "0x" + "11" * 20
    solver = "0x" + "22" * 20
    other = "0x" + "33" * 20
    competition = "0x" + "44" * 20
    source = "55" * 20
    derivation = "0x" + "66" * 32
    rehearsal = {
        "passed": True,
        "network": "base-mainnet",
        "source_commit": source,
        "actor_derivation_id": derivation,
        "actors": {"deployer": deployer, "solver_a": solver, "solver_b": other},
        "x402_canary": {"active": True, "solver": solver, "competition": competition},
        "transactions": {
            "fund_solver_a_gas": {"transaction_hash": "0x" + "77" * 32},
            "fund_solver_a_usdc": {"transaction_hash": "0x" + "88" * 32},
        },
    }
    success = {
        "passed": True,
        "network": "base-mainnet",
        "source_commit": source,
        "actor_derivation_id": derivation,
        "solver": solver,
        "competition": competition,
        "generated_agent_wallet": True,
        "manual_state_corrections": 0,
        "standard_exact": True,
        "eip3009": True,
        "payment_transaction": "0x" + "99" * 32,
        "relay_transaction": "0x" + "aa" * 32,
        "proof_hash": "0x" + "bb" * 32,
        "public_values_hash": "0x" + "cc" * 32,
        "settlement_event_id": "8d9e8b2e-3d97-4f2c-b769-28f2d2589842",
    }
    return rehearsal, success


class FreshAgentFlowTests(unittest.TestCase):
    def test_accepts_distinct_funded_generated_solver(self):
        rehearsal, success = fixtures()
        result = MODULE.verify(rehearsal, success)
        self.assertTrue(result["passed"])
        self.assertEqual(result["manual_state_corrections"], 0)

    def test_rejects_deployer_reused_as_solver(self):
        rehearsal, success = fixtures()
        rehearsal["actors"]["solver_a"] = rehearsal["actors"]["deployer"]
        success["solver"] = rehearsal["actors"]["deployer"]
        with self.assertRaises(MODULE.FreshAgentFlowError):
            MODULE.verify(rehearsal, success)

    def test_rejects_retry_identity_drift(self):
        rehearsal, success = fixtures()
        success["actor_derivation_id"] = "0x" + "dd" * 32
        with self.assertRaises(MODULE.FreshAgentFlowError):
            MODULE.verify(rehearsal, success)

    def test_rejects_missing_canonical_settlement(self):
        rehearsal, success = fixtures()
        success["settlement_event_id"] = None
        with self.assertRaises(MODULE.FreshAgentFlowError):
            MODULE.verify(rehearsal, success)


if __name__ == "__main__":
    unittest.main()
