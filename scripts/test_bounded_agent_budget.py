from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "plan_bounded_agent_budget.py"
MANIFEST = ROOT / "deployments" / "bounded-agent-wallet-base-mainnet.json"


def load_planner():
    spec = importlib.util.spec_from_file_location("bounded_agent_budget", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load bounded-agent budget planner")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_action_planner():
    scripts = str(ROOT / "scripts")
    if scripts not in sys.path:
        sys.path.insert(0, scripts)
    path = ROOT / "scripts" / "plan_bounded_agent_action.py"
    spec = importlib.util.spec_from_file_location("bounded_agent_action", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load bounded-agent action planner")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class BoundedAgentBudgetPlannerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.planner = load_planner()
        cls.action_planner = load_action_planner()
        cls.manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))

    def test_usdc_amounts_are_exact(self) -> None:
        self.assertEqual(self.planner.usdc_units("89", "amount"), 89_000_000)
        self.assertEqual(self.planner.usdc_units("0.000001", "amount"), 1)
        with self.assertRaises(SystemExit):
            self.planner.usdc_units("0.0000001", "amount")

    def test_policy_changes_authorization_destination(self) -> None:
        base = {
            "delegate": "0x1111111111111111111111111111111111111111",
            "valid_after": 1_800_000_000,
            "valid_until": 1_802_592_000,
            "period_seconds": 86_400,
            "max_per_action": 5_000_000,
            "max_per_period": 10_000_000,
            "max_lifetime_spend": 89_000_000,
            "max_bounty_target": 5_000_000,
            "allowed_actions": 15,
            "allowed_verification_modes": 1,
            "deterministic_verifier_module": self.manifest["canonical"]["deterministic_verifier"],
            "signed_quorum_verifier_set_hash": self.planner.ZERO_HASH,
            "ai_judge_verifier_set_hash": self.planner.ZERO_HASH,
        }
        changed = {**base, "delegate": "0x2222222222222222222222222222222222222222"}
        owner = "0x884834e884d6e93462655a2820140ad03e6747bc"
        salt = "0x" + "11" * 32

        def predict(policy: dict) -> str:
            encoded = self.planner.encode(
                f"f({self.planner.POLICY_TYPE})", self.planner.policy_tuple(policy)
            )
            policy_hash = self.planner.keccak_hex(encoded)
            return self.planner.predicted_wallet(
                self.manifest["wallet_factory"]["address"],
                self.manifest["wallet_factory"]["implementation"],
                owner,
                salt,
                policy_hash,
            )[0]

        self.assertNotEqual(predict(base), predict(changed))

    def test_cli_emits_one_signature_plan_and_owner_escape_hatch(self) -> None:
        now = int(time.time())
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "plan.json"
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--owner",
                    "0x884834e884d6e93462655a2820140ad03e6747bc",
                    "--delegate",
                    "0x1111111111111111111111111111111111111111",
                    "--valid-after",
                    str(now),
                    "--valid-until",
                    str(now + 30 * 86_400),
                    "--user-salt",
                    "0x" + "11" * 32,
                    "--authorization-nonce",
                    "0x" + "22" * 32,
                    "--output",
                    str(output),
                ],
                cwd=ROOT,
                check=True,
                capture_output=True,
                text=True,
            )
            self.assertIn(str(output), result.stdout)
            plan = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(plan["initial_funding"], "89000000")
        self.assertEqual(plan["authorization_typed_data"]["message"]["to"], plan["predicted_wallet"])
        self.assertEqual(plan["relay_call"]["signature_tail"], ["v", "r", "s"])
        self.assertTrue(plan["direct_owner_fallback"]["approval"]["data"].startswith("0x095ea7b3"))
        self.assertTrue(plan["direct_owner_fallback"]["create_and_fund"]["data"].startswith("0x86f357d0"))
        self.assertEqual(plan["owner_controls"]["revoke"]["to"], plan["predicted_wallet"])
        self.assertNotEqual(plan["owner_controls"]["revoke"]["data"], "0x")

    def test_cli_rejects_incoherent_caps(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--owner",
                "0x884834e884d6e93462655a2820140ad03e6747bc",
                "--delegate",
                "0x1111111111111111111111111111111111111111",
                "--max-per-action-usdc",
                "11",
                "--max-per-period-usdc",
                "10",
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("per-action <= per-period <= lifetime", result.stderr)

    def test_manifest_validation_rejects_canonical_drift(self) -> None:
        self.planner.validate_manifest(self.manifest)
        changed = json.loads(json.dumps(self.manifest))
        changed["canonical"]["settlement_token"] = "0x1111111111111111111111111111111111111111"
        with self.assertRaises(SystemExit):
            self.planner.validate_manifest(changed)

    def test_manifest_validation_rejects_dirty_source(self) -> None:
        changed = {**self.manifest, "contract_source_dirty": True}
        with self.assertRaises(SystemExit):
            self.planner.validate_manifest(changed)

    def test_action_planner_fails_closed_on_remaining_caps(self) -> None:
        report = {
            "safe_block": {"timestamp": 1_800_000_000},
            "state": {
                "period_bucket": str(1_800_000_000 // 86_400),
                "period_spent": "9000000",
                "lifetime_spent": "88000000",
                "wallet_usdc_balance": "89000000",
                "policy": {
                    "period_seconds": 86_400,
                    "max_per_action": 5_000_000,
                    "max_per_period": 10_000_000,
                    "max_lifetime_spend": 89_000_000,
                },
            },
        }
        self.action_planner.validate_spend(report, 1_000_000)
        with self.assertRaises(SystemExit):
            self.action_planner.validate_spend(report, 1_000_001)

    def test_action_planner_pins_exact_verifier(self) -> None:
        factory = "0x1111111111111111111111111111111111111111"
        token = "0x2222222222222222222222222222222222222222"
        module = "0x3333333333333333333333333333333333333333"
        state = {
            "factory": factory,
            "settlement_token": token,
            "target_amount": 5_000_000,
            "verification_mode": 0,
            "verifier_module": module,
            "verifier_set_hash": self.planner.ZERO_HASH,
        }
        policy = {
            "max_bounty_target": 5_000_000,
            "allowed_verification_modes": 1,
            "deterministic_verifier_module": module,
            "signed_quorum_verifier_set_hash": self.planner.ZERO_HASH,
            "ai_judge_verifier_set_hash": self.planner.ZERO_HASH,
        }
        self.action_planner.validate_bounty_policy(state, policy, factory, token)
        state["verifier_module"] = "0x4444444444444444444444444444444444444444"
        with self.assertRaises(SystemExit):
            self.action_planner.validate_bounty_policy(state, policy, factory, token)


if __name__ == "__main__":
    unittest.main()
