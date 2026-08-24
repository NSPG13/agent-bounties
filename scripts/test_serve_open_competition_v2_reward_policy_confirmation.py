from __future__ import annotations

import copy
from datetime import datetime, timezone
import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from requests.exceptions import HTTPError
from web3.exceptions import Web3RPCError


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import serve_open_competition_v2_reward_policy_confirmation as MODULE  # noqa: E402
import build_open_competition_v2_reward_policy as BUILDER  # noqa: E402


class RewardPolicyConfirmationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cohort = json.loads(
            (
                ROOT / "ops" / "open-competition-v2-forward-gmv-reward-cohort-v1.json"
            ).read_text()
        )
        profile = cohort["profile_release"]
        state = {
            "schema_version": BUILDER.STATE_SCHEMA,
            "network": "base-mainnet",
            "chain_id": 8453,
            "block_tag": "safe",
            "safe_block": 50_000_000,
            "reserve_wallet": cohort["reserve_wallet"],
            "owner": BUILDER.OWNER,
            "settlement_token": BUILDER.USDC,
            "competition_factory": cohort["factory_contract"],
            "competition_implementation": "0x" + "ab" * 20,
            "policy_version": 1,
            "active_policy_hash": "0x" + "12" * 32,
            "period_bucket": 20_688,
            "period_spent": 30_400_000,
            "lifetime_spent": 30_400_000,
            "reserve_balance": 47_268_098,
            "revoked": False,
            "policy": {
                "delegate": "0xb358898d34c5e907877a1cd7540b234f6851f61b",
                "valid_after": 1_787_503_420,
                "valid_until": 1_793_491_199,
                "period_seconds": 86_400,
                "solver_reward": 3_000_000,
                "keeper_reward": 40_000,
                "exact_funding_per_competition": 3_040_000,
                "max_per_period": 30_400_000,
                "max_lifetime_spend": 77_668_098,
                "beta_risk_hash": "0x" + "34" * 32,
                "gmv_metric_program_hash": profile["metric_program_hash"],
                "gmv_journal_schema_hash": profile["journal_schema_hash"],
            },
        }
        cls.bundle = BUILDER.build_rotation(
            cohort,
            state,
            datetime(2026, 8, 24, 10, 15, tzinfo=timezone.utc),
        )

    def test_exact_bundle_is_zero_value_and_valid(self) -> None:
        MODULE.validate_bundle(copy.deepcopy(self.bundle))
        self.assertEqual(self.bundle["owner_transactions"]["revoke"]["value_wei"], 0)
        self.assertEqual(self.bundle["owner_transactions"]["configure"]["value_wei"], 0)
        self.assertEqual(
            self.bundle["confirmation_summary"]["usdc_moved_by_confirmation"], "0 USDC"
        )

    def test_changed_owner_destination_value_or_calldata_fails_closed(self) -> None:
        mutations = (
            ("owner", "0x" + "11" * 20),
            ("to", "0x" + "22" * 20),
            ("value_wei", 1),
            ("data", "0x"),
        )
        for field, value in mutations:
            bundle = copy.deepcopy(self.bundle)
            if field == "owner":
                bundle[field] = value
            else:
                bundle["owner_transactions"]["configure"][field] = value
            with self.subTest(field=field), self.assertRaises(MODULE.ConfirmationError):
                MODULE.validate_bundle(bundle)

    def test_receipt_transaction_must_match_every_exact_field(self) -> None:
        for stage in ("revoke", "configure"):
            expected = self.bundle["owner_transactions"][stage]
            transaction = {
                "from": expected["from"],
                "to": expected["to"],
                "value": 0,
                "input": expected["data"],
            }
            MODULE.transaction_matches(self.bundle, stage, transaction)
            transaction["input"] = "0x1234"
            with self.assertRaises(MODULE.ConfirmationError):
                MODULE.transaction_matches(self.bundle, stage, transaction)

    def test_safe_revocation_result_survives_server_restart(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_output = Path(directory) / "final.json"
            revoked = {
                "status": "revoked",
                "revoke_transaction_hash": "0x" + "ab" * 32,
                "policy_version": self.bundle["current_policy"]["version"],
                "policy_hash": self.bundle["current_policy"]["hash"],
                "lifetime_spent": MODULE.EXPECTED_LIFETIME_SPENT,
                "reserve_balance": MODULE.EXPECTED_RESERVE_BALANCE,
            }
            MODULE.store_result(MODULE.revocation_result_path(result_output), revoked)
            self.assertEqual(
                MODULE.load_revocation_result(result_output, self.bundle), revoked
            )

            revoked["reserve_balance"] -= 1
            MODULE.revocation_result_path(result_output).write_text(
                json.dumps(revoked), encoding="utf-8"
            )
            with self.assertRaises(MODULE.ConfirmationError):
                MODULE.load_revocation_result(result_output, self.bundle)

    def test_rpc_failover_skips_blocked_and_wrong_chain_endpoints(self) -> None:
        class FakeWeb3:
            def __init__(self, name: str, chain_id: int = 8453) -> None:
                self.name = name
                self.eth = SimpleNamespace(chain_id=chain_id)

            def is_connected(self) -> bool:
                return True

        endpoints = {
            "https://blocked.example": FakeWeb3("blocked"),
            "https://wrong.example": FakeWeb3("wrong", 1),
            "https://base.example": FakeWeb3("base"),
        }

        def operation(w3: FakeWeb3) -> str:
            if w3.name == "blocked":
                raise HTTPError(response=SimpleNamespace(status_code=403))
            return w3.name

        self.assertEqual(
            MODULE.base_rpc_call(
                tuple(endpoints),
                operation,
                "test evidence",
                web3_factory=endpoints.__getitem__,
            ),
            "base",
        )

    def test_json_rpc_execution_error_does_not_fail_over(self) -> None:
        class FakeWeb3:
            def __init__(self, name: str) -> None:
                self.name = name
                self.eth = SimpleNamespace(chain_id=8453)

            def is_connected(self) -> bool:
                return True

        endpoints = {
            "https://first.example": FakeWeb3("first"),
            "https://second.example": FakeWeb3("second"),
        }
        called: list[str] = []

        def operation(w3: FakeWeb3) -> str:
            called.append(w3.name)
            if w3.name == "first":
                raise Web3RPCError("execution reverted")
            return w3.name

        with self.assertRaises(Web3RPCError):
            MODULE.base_rpc_call(
                tuple(endpoints),
                operation,
                "test execution",
                web3_factory=endpoints.__getitem__,
            )
        self.assertEqual(called, ["first"])

    def test_submitted_hash_is_durable_and_restarts_reconciliation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_output = Path(directory) / "final.json"
            transaction_hash = "0x" + "cd" * 32
            stored = MODULE.store_submission_result(
                result_output, self.bundle, "revoke", transaction_hash
            )
            self.assertEqual(stored["transaction_hash"], transaction_hash)
            self.assertEqual(
                MODULE.load_submission_result(result_output, self.bundle, "revoke"),
                stored,
            )

            with patch.object(
                MODULE.ConfirmationServer, "_launch_reconciliation"
            ) as launch:
                server = MODULE.ConfirmationServer(
                    ("127.0.0.1", 0),
                    self.bundle,
                    ("https://base.example",),
                    result_output,
                )
                try:
                    launch.assert_called_once_with("revoke", transaction_hash)
                finally:
                    server.server_close()

            with self.assertRaises(MODULE.ConfirmationError):
                MODULE.store_submission_result(
                    result_output, self.bundle, "revoke", "0x" + "ef" * 32
                )

    def test_rpc_urls_are_https_credential_free_and_deduplicated(self) -> None:
        self.assertEqual(
            MODULE.normalized_rpc_urls(
                ["https://base.example", "https://base.example"]
            ),
            ("https://base.example",),
        )
        for value in ("http://base.example", "https://user:secret@base.example"):
            with self.subTest(value=value), self.assertRaises(MODULE.ConfirmationError):
                MODULE.normalized_rpc_urls([value])


if __name__ == "__main__":
    unittest.main()
