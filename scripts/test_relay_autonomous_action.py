import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import relay_autonomous_action as relay


SOLVER = "0x2000000000000000000000000000000000000002"
CREATOR = "0x1000000000000000000000000000000000000001"
CONTRACT = "0x4000000000000000000000000000000000000004"
NOW = 1_800_000_000
HASH_A = "0x" + "11" * 32
HASH_B = "0x" + "22" * 32
PRIVATE_KEY = "0x" + "01" * 32


def bounty_state(**overrides: object) -> relay.BountyState:
    values: dict[str, object] = {
        "chain_id": relay.CHAIN_ID,
        "block_timestamp": NOW,
        "codehash": relay.CLONE_CODEHASH,
        "canonical": True,
        "factory_implementation": relay.IMPLEMENTATION,
        "factory": relay.FACTORY,
        "settlement_token": relay.USDC,
        "bounty_id": "0x" + "33" * 32,
        "creator": CREATOR,
        "solver_reward": 900_000,
        "verifier_reward": 100_000,
        "target_amount": 1_000_000,
        "funded_amount": 1_000_000,
        "status": relay.STATUS_CLAIMABLE,
        "round": 0,
        "solver": relay.ZERO_ADDRESS,
        "claim_expires_at": 0,
        "verification_expires_at": 0,
        "active_claim_bond": 0,
        "verification_mode": 0,
        "verifier_module": relay.VERIFIER_MODULE,
        "threshold": 1,
        "policy_hash": "0x" + "44" * 32,
        "submission_hash": "0x" + "00" * 32,
        "evidence_hash": "0x" + "00" * 32,
    }
    values.update(overrides)
    return relay.BountyState(**values)  # type: ignore[arg-type]


def claim_envelope(**overrides: object) -> dict[str, object]:
    value: dict[str, object] = {
        "schema": relay.SCHEMA,
        "action": "claim",
        "network": "base-mainnet",
        "bounty_contract": CONTRACT,
        "solver": SOLVER,
        "authorization": {
            "valid_after": 0,
            "valid_before": NOW + 600,
            "nonce": "0x" + "55" * 32,
            "v": 27,
            "r": "0x" + "66" * 32,
            "s": "0x" + "77" * 32,
        },
    }
    value.update(overrides)
    return value


def submit_envelope(**overrides: object) -> dict[str, object]:
    value: dict[str, object] = {
        "schema": relay.SCHEMA,
        "action": "submit",
        "network": "base-mainnet",
        "bounty_contract": CONTRACT,
        "solver": SOLVER,
        "round": 1,
        "submission_hash": HASH_A,
        "evidence_hash": HASH_B,
        "deadline": NOW + 600,
        "signature": "0x" + "88" * 65,
    }
    value.update(overrides)
    return value


def settle_envelope(**overrides: object) -> dict[str, object]:
    value: dict[str, object] = {
        "schema": relay.SCHEMA,
        "action": "settle",
        "network": "base-mainnet",
        "bounty_contract": CONTRACT,
        "round": 1,
        "proof": "0x" + "99" * 32,
    }
    value.update(overrides)
    return value


def event(envelope: dict[str, object]) -> relay.RelayEvent:
    return relay.RelayEvent(
        relay.REPOSITORY,
        212,
        999,
        "example-agent",
        ("bounty", "funded-live", "claimable-live"),
        envelope,
    )


class FakeClient:
    def __init__(self, *, verifier_passed: bool = True) -> None:
        self.verifier_passed = verifier_passed
        self.sent = False
        self.estimated = False

    def call(self, contract: str, signature: str, *args: str, block: str | None = None) -> str:
        self.last_call = (contract, signature, args, block)
        if signature.startswith("verify("):
            return ("true" if self.verifier_passed else "false") + "\n0x" + "aa" * 32
        raise AssertionError(f"unexpected fake call: {signature}")

    def keeper_address(self, private_key: str) -> str:
        self.private_key = private_key
        return "0x3000000000000000000000000000000000000003"

    def chain_id(self) -> int:
        return relay.CHAIN_ID

    def estimate(self, keeper: str, contract: str, signature: str, *args: str) -> int:
        self.estimated = True
        self.estimate_args = (keeper, contract, signature, args)
        return 100_000

    def gas_price(self) -> int:
        return 1_000_000

    def balance(self, account: str) -> int:
        return 1_000_000_000_000_000

    def send(self, private_key: str, gas_limit: int, contract: str, signature: str, *args: str):
        self.sent = True
        self.send_args = (private_key, gas_limit, contract, signature, args)
        return {
            "status": "0x1",
            "to": contract,
            "transactionHash": "0x" + "ab" * 32,
            "blockNumber": "0x1234",
        }


class RelayTests(unittest.TestCase):
    def test_comment_parser_requires_exact_versioned_json(self) -> None:
        body = relay.COMMAND + "\n```json\n" + json.dumps(claim_envelope()) + "\n```"
        parsed = relay.parse_comment(body)
        self.assertEqual(parsed["action"], "claim")
        with self.assertRaisesRegex(relay.RelayError, "first line"):
            relay.parse_comment("please relay\n{}")
        malformed = claim_envelope(extra=True)
        with self.assertRaisesRegex(relay.RelayError, "keys mismatch"):
            relay.parse_comment(relay.COMMAND + "\n" + json.dumps(malformed))

    def test_event_rejects_prs_and_unavailable_verification(self) -> None:
        payload = {
            "repository": {"full_name": relay.REPOSITORY},
            "issue": {
                "number": 1,
                "pull_request": {"url": "https://example.test"},
                "labels": [{"name": "funded-live"}],
            },
            "comment": {"id": 1, "body": relay.COMMAND, "user": {"login": "agent"}},
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "event.json"
            path.write_text(json.dumps(payload), encoding="utf-8")
            with self.assertRaisesRegex(relay.RelayError, "not pull requests"):
                relay.parse_event(path)
            payload["issue"].pop("pull_request")
            payload["issue"]["labels"].append({"name": "verification-unavailable"})
            payload["comment"]["body"] = relay.COMMAND + "\n" + json.dumps(claim_envelope())
            path.write_text(json.dumps(payload), encoding="utf-8")
            with self.assertRaisesRegex(relay.RelayError, "disabled"):
                relay.parse_event(path)

    def test_invalid_network_replay_retains_source_and_gives_exact_correction(self) -> None:
        fixture = Path(__file__).with_name("fixtures") / "autonomous_relay_invalid_network.json"
        source, body = relay.parse_event_source(fixture)
        self.assertEqual(source.issue_number, 244)
        self.assertEqual(source.comment_id, 424424)
        with self.assertRaises(relay.RelayError) as raised:
            relay.parse_event_request(source, body)
        self.assertIn('received \'Base\'', str(raised.exception))
        self.assertEqual(
            raised.exception.details["correction"],
            'Set "network" to "base-mainnet" and post a new `/agent-bounty relay` command.',
        )

    def test_published_invalid_request_is_handled_without_initializing_keeper(self) -> None:
        fixture = Path(__file__).with_name("fixtures") / "autonomous_relay_invalid_network.json"
        published: list[tuple[relay.RelaySource, str]] = []
        with tempfile.TemporaryDirectory() as directory:
            report_path = Path(directory) / "report.json"
            comment_path = Path(directory) / "comment.md"
            with (
                mock.patch.object(
                    relay,
                    "publish_comment",
                    side_effect=lambda source, comment, env: published.append((source, comment)),
                ),
                mock.patch.object(
                    relay,
                    "CastClient",
                    side_effect=AssertionError("keeper must not initialize for malformed input"),
                ),
            ):
                result = relay.main(
                    [
                        "--event",
                        str(fixture),
                        "--execute",
                        "--publish",
                        "--report",
                        str(report_path),
                        "--comment",
                        str(comment_path),
                    ]
                )
            report = json.loads(report_path.read_text(encoding="utf-8"))
            comment = comment_path.read_text(encoding="utf-8")
        self.assertEqual(result, 0)
        self.assertEqual(report["outcome"], "refused")
        self.assertEqual(report["error_code"], "request_invalid")
        self.assertEqual(report["workflow_disposition"], "handled_after_feedback")
        self.assertEqual(report["action"], "submit")
        self.assertEqual(len(published), 1)
        self.assertEqual(published[0][0].comment_id, 424424)
        self.assertIn('Set "network" to "base-mainnet"', comment)
        self.assertIn("No transaction was broadcast", comment)

    def test_operational_relay_failure_remains_red_after_feedback(self) -> None:
        payload = json.loads(
            (Path(__file__).with_name("fixtures") / "autonomous_relay_invalid_network.json").read_text(
                encoding="utf-8"
            )
        )
        body = str(payload["comment"]["body"]).replace('"network":"Base"', '"network":"base-mainnet"')
        payload["comment"]["body"] = body
        published: list[str] = []
        with tempfile.TemporaryDirectory() as directory:
            event_path = Path(directory) / "event.json"
            report_path = Path(directory) / "report.json"
            comment_path = Path(directory) / "comment.md"
            event_path.write_text(json.dumps(payload), encoding="utf-8")
            with (
                mock.patch.object(relay, "CastClient", return_value=object()),
                mock.patch.object(
                    relay,
                    "relay",
                    side_effect=relay.RelayError(
                        "keeper chain unavailable",
                        code="keeper_chain_unavailable",
                        retryable=True,
                    ),
                ),
                mock.patch.object(
                    relay,
                    "publish_comment",
                    side_effect=lambda source, comment, env: published.append(comment),
                ),
            ):
                result = relay.main(
                    [
                        "--event",
                        str(event_path),
                        "--execute",
                        "--publish",
                        "--report",
                        str(report_path),
                        "--comment",
                        str(comment_path),
                    ]
                )
            report = json.loads(report_path.read_text(encoding="utf-8"))
        self.assertEqual(result, 1)
        self.assertEqual(report["outcome"], "retryable")
        self.assertEqual(report["error_code"], "keeper_chain_unavailable")
        self.assertEqual(len(published), 2)

    def test_common_validation_rejects_noncanonical_and_large_bounties(self) -> None:
        with self.assertRaisesRegex(relay.RelayError, "canonical"):
            relay.validate_common(bounty_state(canonical=False))
        with self.assertRaisesRegex(relay.RelayError, "5 USDC"):
            relay.validate_common(
                bounty_state(solver_reward=5_000_000, target_amount=5_100_000)
            )
        with self.assertRaisesRegex(relay.RelayError, "not allowlisted"):
            relay.validate_common(
                bounty_state(verifier_module="0x9999999999999999999999999999999999999999")
            )

    def test_claim_builds_only_bounded_authorization_call(self) -> None:
        client = FakeClient()
        signature, args = relay.action_call(client, event(claim_envelope()), bounty_state())
        self.assertTrue(signature.startswith("claimWithAuthorization"))
        self.assertEqual(args[0], SOLVER.lower())
        self.assertEqual(args[1], "0")
        self.assertFalse(client.sent)

    def test_claim_rejects_creator_and_long_authorization(self) -> None:
        with self.assertRaisesRegex(relay.RelayError, "independent"):
            relay.action_call(
                FakeClient(), event(claim_envelope(solver=CREATOR)), bounty_state()
            )
        envelope = claim_envelope()
        assert isinstance(envelope["authorization"], dict)
        envelope["authorization"]["valid_before"] = NOW + 3_601
        with self.assertRaisesRegex(relay.RelayError, "one hour"):
            relay.action_call(FakeClient(), event(envelope), bounty_state())

    def test_submit_binds_solver_round_hashes_and_claim_deadline(self) -> None:
        state = bounty_state(
            status=relay.STATUS_CLAIMED,
            round=1,
            solver=SOLVER,
            claim_expires_at=NOW + 1_000,
            active_claim_bond=100_000,
        )
        signature, args = relay.action_call(
            FakeClient(), event(submit_envelope()), state
        )
        self.assertTrue(signature.startswith("submitWithSignature"))
        self.assertEqual(args[:2], [HASH_A, HASH_B])
        with self.assertRaisesRegex(relay.RelayError, "round"):
            relay.action_call(
                FakeClient(), event(submit_envelope(round=2)), state
            )

    def test_settlement_relays_only_a_passing_allowlisted_module_proof(self) -> None:
        state = bounty_state(
            status=relay.STATUS_SUBMITTED,
            round=1,
            solver=SOLVER,
            verification_expires_at=NOW + 1_000,
            active_claim_bond=100_000,
            submission_hash=HASH_A,
            evidence_hash=HASH_B,
        )
        signature, _ = relay.action_call(
            FakeClient(), event(settle_envelope()), state
        )
        self.assertEqual(signature, "verifyAndSettle(bytes)")
        with self.assertRaisesRegex(relay.RelayError, "refusing rejection"):
            relay.action_call(
                FakeClient(verifier_passed=False), event(settle_envelope()), state
            )

    def run_relay(
        self,
        before: relay.BountyState,
        after: relay.BountyState,
        envelope: dict[str, object],
    ) -> tuple[dict[str, object], FakeClient]:
        client = FakeClient()
        states = [before, after]
        original = relay.read_state
        relay.read_state = lambda ignored, contract, block=None: states.pop(0)  # type: ignore[assignment]
        try:
            report = relay.relay(
                client, event(envelope), execute=True, private_key=PRIVATE_KEY
            )
            return report, client
        finally:
            relay.read_state = original

    def test_keeper_preflight_rejects_malformed_secret_before_chain_state(self) -> None:
        original = relay.read_state
        relay.read_state = (  # type: ignore[assignment]
            lambda *args, **kwargs: self.fail("chain state read before signer preflight")
        )
        try:
            with self.assertRaises(relay.RelayError) as raised:
                relay.relay(
                    FakeClient(),
                    event(claim_envelope()),
                    execute=True,
                    private_key="not-a-key",
                )
        finally:
            relay.read_state = original
        self.assertEqual(raised.exception.code, "keeper_configuration_invalid")
        self.assertTrue(raised.exception.retryable)

    def test_keeper_health_checks_signer_chain_and_gas_reserve(self) -> None:
        report = relay.keeper_health(FakeClient(), PRIVATE_KEY)
        self.assertEqual(report["outcome"], "healthy")
        self.assertEqual(report["chain_id"], relay.CHAIN_ID)
        self.assertGreaterEqual(
            report["keeper_balance_wei"], report["minimum_balance_wei"]
        )

    def test_settlement_uses_latest_state_after_bounded_predecessor_wait(self) -> None:
        claimed = bounty_state(
            status=relay.STATUS_CLAIMED,
            round=1,
            solver=SOLVER,
            claim_expires_at=NOW + 1_000,
            active_claim_bond=100_000,
        )
        submitted = bounty_state(
            status=relay.STATUS_SUBMITTED,
            round=1,
            solver=SOLVER,
            verification_expires_at=NOW + 1_000,
            active_claim_bond=100_000,
            submission_hash=HASH_A,
            evidence_hash=HASH_B,
        )
        settled = bounty_state(
            status=relay.STATUS_SETTLED,
            round=1,
            solver=SOLVER,
            funded_amount=0,
            submission_hash=HASH_A,
            evidence_hash=HASH_B,
        )
        states = [claimed, submitted, settled]
        blocks: list[str | None] = []
        original = relay.read_state

        def read(ignored, contract, block=None):
            blocks.append(block)
            return states.pop(0)

        relay.read_state = read  # type: ignore[assignment]
        try:
            report = relay.relay(
                FakeClient(),
                event(settle_envelope()),
                execute=True,
                private_key=PRIVATE_KEY,
                state_wait_seconds=5,
                state_poll_seconds=0.1,
                sleep_fn=lambda ignored: None,
                clock=lambda: 0.0,
            )
        finally:
            relay.read_state = original
        self.assertEqual(report["outcome"], "relayed")
        self.assertEqual(report["lifecycle_block_tag"], "latest")
        self.assertEqual(report["state_attempts"], 2)
        self.assertEqual(blocks, ["latest", "latest", "0x1234"])

    def test_predecessor_wait_times_out_with_retryable_terminal_status(self) -> None:
        client = FakeClient()
        claimed = bounty_state(
            status=relay.STATUS_CLAIMED,
            round=1,
            solver=SOLVER,
            claim_expires_at=NOW + 1_000,
            active_claim_bond=100_000,
        )
        original = relay.read_state
        relay.read_state = lambda ignored, contract, block=None: claimed  # type: ignore[assignment]
        try:
            with self.assertRaises(relay.RelayError) as raised:
                relay.relay(
                    client,
                    event(settle_envelope()),
                    execute=True,
                    private_key=PRIVATE_KEY,
                    state_wait_seconds=0,
                )
        finally:
            relay.read_state = original
        self.assertEqual(raised.exception.code, "lifecycle_state_timeout")
        self.assertTrue(raised.exception.retryable)
        self.assertEqual(raised.exception.details["observed_status"], "claimed")
        self.assertFalse(client.sent)

    def test_executes_claim_once_and_validates_post_state(self) -> None:
        report, client = self.run_relay(
            bounty_state(),
            bounty_state(
                status=relay.STATUS_CLAIMED,
                round=1,
                solver=SOLVER,
                claim_expires_at=NOW + 10_000,
                active_claim_bond=100_000,
            ),
            claim_envelope(),
        )
        self.assertEqual(report["outcome"], "relayed")
        self.assertTrue(client.sent)

    def test_executes_submission_once_and_validates_commitments(self) -> None:
        before = bounty_state(
            status=relay.STATUS_CLAIMED,
            round=1,
            solver=SOLVER,
            claim_expires_at=NOW + 1_000,
            active_claim_bond=100_000,
        )
        after = bounty_state(
            status=relay.STATUS_SUBMITTED,
            round=1,
            solver=SOLVER,
            claim_expires_at=NOW + 1_000,
            verification_expires_at=NOW + 2_000,
            active_claim_bond=100_000,
            submission_hash=HASH_A,
            evidence_hash=HASH_B,
        )
        report, _ = self.run_relay(before, after, submit_envelope())
        self.assertEqual(report["outcome"], "relayed")

    def test_executes_passing_settlement_and_accepts_zero_funded_post_state(self) -> None:
        before = bounty_state(
            status=relay.STATUS_SUBMITTED,
            round=1,
            solver=SOLVER,
            verification_expires_at=NOW + 1_000,
            active_claim_bond=100_000,
            submission_hash=HASH_A,
            evidence_hash=HASH_B,
        )
        after = bounty_state(
            status=relay.STATUS_SETTLED,
            round=1,
            solver=SOLVER,
            funded_amount=0,
            active_claim_bond=0,
            submission_hash=HASH_A,
            evidence_hash=HASH_B,
        )
        report, _ = self.run_relay(before, after, settle_envelope())
        self.assertEqual(report["outcome"], "relayed")

    def test_already_applied_is_idempotent_and_never_sends(self) -> None:
        client = FakeClient()
        state = bounty_state(
            status=relay.STATUS_CLAIMED,
            round=1,
            solver=SOLVER,
            claim_expires_at=NOW + 1_000,
            active_claim_bond=100_000,
        )
        original = relay.read_state
        relay.read_state = lambda ignored, contract, block=None: state  # type: ignore[assignment]
        try:
            report = relay.relay(
                client, event(claim_envelope()), execute=True, private_key=PRIVATE_KEY
            )
        finally:
            relay.read_state = original
        self.assertEqual(report["outcome"], "already_applied")
        self.assertFalse(client.sent)

    def test_gas_and_balance_caps_fail_before_send(self) -> None:
        client = FakeClient()
        client.estimate = lambda *args: relay.GAS_CAPS["claim"] + 1  # type: ignore[method-assign]
        original = relay.read_state
        relay.read_state = lambda ignored, contract, block=None: bounty_state()  # type: ignore[assignment]
        try:
            with self.assertRaisesRegex(relay.RelayError, "gas limit"):
                relay.relay(
                    client, event(claim_envelope()), execute=True, private_key=PRIVATE_KEY
                )
        finally:
            relay.read_state = original
        self.assertFalse(client.sent)

    def test_receipt_target_and_comment_evidence_boundary(self) -> None:
        with self.assertRaisesRegex(relay.RelayError, "target"):
            relay.validate_receipt(
                {
                    "status": 1,
                    "to": CREATOR,
                    "transactionHash": "0x" + "aa" * 32,
                    "blockNumber": 1,
                },
                CONTRACT,
            )
        comment = relay.render_comment(
            {
                "outcome": "relayed",
                "action": "claim",
                "bounty_contract": CONTRACT,
                "transaction_hash": "0x" + "aa" * 32,
            },
            99,
        )
        self.assertIn("only `BountySettled` proves solver payment", comment)
        self.assertIn("Source comment id: `99`", comment)
        processing = relay.render_comment(
            {
                "outcome": "processing",
                "action": "settle",
                "bounty_contract": CONTRACT,
                "state_wait_seconds": 120,
            },
            100,
        )
        self.assertIn("Request received", processing)
        self.assertIn("bounded lifecycle wait of 120 seconds", processing)


if __name__ == "__main__":
    unittest.main()
