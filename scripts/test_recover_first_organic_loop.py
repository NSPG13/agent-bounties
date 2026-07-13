import json
import tempfile
import unittest
from pathlib import Path

import recover_first_organic_loop as recovery


def state(**overrides: object) -> recovery.ContractState:
    values: dict[str, object] = {
        "chain_id": recovery.CHAIN_ID,
        "block_timestamp": recovery.VERIFICATION_EXPIRES_AT - 10,
        "codehash": recovery.CONTRACT_CODEHASH,
        "expire_selector": recovery.EXPIRE_SELECTOR,
        "factory": recovery.FACTORY,
        "settlement_token": recovery.USDC,
        "bounty_id": recovery.BOUNTY_ID,
        "round": recovery.ROUND,
        "status": recovery.SUBMITTED_STATUS,
        "solver": recovery.SOLVER,
        "verification_expires_at": recovery.VERIFICATION_EXPIRES_AT,
        "active_claim_bond": recovery.CLAIM_BOND,
    }
    values.update(overrides)
    return recovery.ContractState(**values)  # type: ignore[arg-type]


class FakeClient:
    def __init__(self, states: list[recovery.ContractState]) -> None:
        self.states = states
        self.sent = False

    def keeper_address(self, private_key: str) -> str:
        self.private_key = private_key
        return "0x1111111111111111111111111111111111111111"

    def keeper_balance(self, keeper: str) -> int:
        self.keeper = keeper
        return 100_000_000_000_000

    def send_expiry(self, private_key: str) -> dict[str, object]:
        self.sent = True
        return {
            "status": "0x1",
            "transactionHash": "0x" + "ab" * 32,
            "blockNumber": "0x2e4c001",
            "logs": [
                {
                    "address": recovery.CONTRACT,
                    "topics": [
                        recovery.EXPIRED_EVENT_TOPIC,
                        recovery.BOUNTY_ID,
                        recovery.padded_uint(recovery.ROUND),
                        recovery.padded_address(recovery.SOLVER),
                    ],
                    "data": recovery.padded_uint(recovery.CLAIM_BOND),
                }
            ],
        }


class RecoveryTests(unittest.TestCase):
    def run_recovery(
        self, client: FakeClient, *, execute: bool = False, private_key: str | None = None
    ) -> dict[str, object]:
        with tempfile.TemporaryDirectory() as directory:
            report_path = Path(directory) / "report.json"
            original = recovery.read_state
            recovery.read_state = lambda ignored: client.states.pop(0)  # type: ignore[assignment]
            try:
                report = recovery.recover(
                    client,  # type: ignore[arg-type]
                    execute=execute,
                    private_key=private_key,
                    report_path=report_path,
                )
                self.assertEqual(json.loads(report_path.read_text()), report)
                return report
            finally:
                recovery.read_state = original

    def test_never_sends_before_deadline(self) -> None:
        client = FakeClient([state()])
        report = self.run_recovery(client, execute=True, private_key="secret")
        self.assertEqual(report["outcome"], "not_due")
        self.assertFalse(client.sent)

    def test_executes_once_and_validates_recovered_state(self) -> None:
        client = FakeClient(
            [
                state(block_timestamp=recovery.VERIFICATION_EXPIRES_AT + 1),
                state(
                    block_timestamp=recovery.VERIFICATION_EXPIRES_AT + 3,
                    status=recovery.CLAIMABLE_STATUS,
                    solver=recovery.ZERO_ADDRESS,
                    verification_expires_at=0,
                    active_claim_bond=0,
                ),
            ]
        )
        report = self.run_recovery(client, execute=True, private_key="secret")
        self.assertEqual(report["outcome"], "recovered")
        self.assertTrue(client.sent)

    def test_after_deadline_without_execute_is_read_only(self) -> None:
        client = FakeClient(
            [state(block_timestamp=recovery.VERIFICATION_EXPIRES_AT + 1)]
        )
        report = self.run_recovery(client)
        self.assertEqual(report["outcome"], "ready")
        self.assertFalse(client.sent)

    def test_already_recovered_is_idempotent(self) -> None:
        client = FakeClient(
            [
                state(
                    status=recovery.CLAIMABLE_STATUS,
                    solver=recovery.ZERO_ADDRESS,
                    verification_expires_at=0,
                    active_claim_bond=0,
                )
            ]
        )
        report = self.run_recovery(client, execute=True, private_key="secret")
        self.assertEqual(report["outcome"], "already_recovered")
        self.assertFalse(client.sent)

    def test_identity_drift_fails_closed(self) -> None:
        client = FakeClient([state(codehash="0x" + "00" * 32)])
        with self.assertRaisesRegex(recovery.RecoveryError, "codehash"):
            self.run_recovery(client)

    def test_receipt_requires_exact_expiry_event(self) -> None:
        with self.assertRaisesRegex(recovery.RecoveryError, "SubmissionExpired"):
            recovery.validate_receipt(
                {
                    "status": "0x1",
                    "transactionHash": "0x" + "ab" * 32,
                    "logs": [],
                }
            )


if __name__ == "__main__":
    unittest.main()
