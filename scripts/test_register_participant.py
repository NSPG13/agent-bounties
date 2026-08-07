from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("register_participant.py")
SPEC = importlib.util.spec_from_file_location("register_participant", SCRIPT)
assert SPEC and SPEC.loader
registration = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = registration
SPEC.loader.exec_module(registration)


def event(body: str = "/agent-bounty register 0x" + "a" * 40):
    return {
        "action": "created",
        "repository": {"full_name": "NSPG13/agent-bounties"},
        "comment": {"body": body},
        "issue": {"number": 321},
        "sender": {"id": 12345, "login": "solver-agent"},
    }


class RegisterParticipantTests(unittest.TestCase):
    def test_exact_command_binds_numeric_github_identity(self) -> None:
        request = registration.parse_event(event(), "NSPG13/agent-bounties")
        self.assertEqual(request.github_user_id, 12345)
        self.assertEqual(request.wallet, "0x" + "a" * 40)

    def test_repository_pr_and_command_confusion_fail_closed(self) -> None:
        wrong_repo = event()
        wrong_repo["repository"]["full_name"] = "attacker/fork"
        pull_request = event()
        pull_request["issue"]["pull_request"] = {"url": "https://example.test"}
        malformed = event("/agent-bounty register 0x" + "a" * 40 + " --extra")
        for value in (wrong_repo, pull_request, malformed):
            with self.subTest(value=value), self.assertRaises(registration.RegistrationError):
                registration.parse_event(value, "NSPG13/agent-bounties")

    def test_same_timestamp_registration_uses_strict_next_cutoff(self) -> None:
        participant_id = "0x" + "1" * 64
        source_hash = "0x" + "2" * 64
        registered_at = 1_784_264_209
        valid_until = registered_at + 30 * 24 * 60 * 60
        cutoff = registration.registration_cutoff(
            [participant_id, source_hash, registered_at, valid_until],
            participant_id,
            source_hash,
            valid_until,
        )
        self.assertEqual(cutoff, registered_at + 1)
        registration.validate_eligibility(
            [participant_id, source_hash, True], participant_id, source_hash
        )

    def test_registration_record_and_eligibility_mismatches_fail_closed(self) -> None:
        participant_id = "0x" + "1" * 64
        source_hash = "0x" + "2" * 64
        with self.assertRaises(registration.RegistrationError):
            registration.registration_cutoff(
                ["0x" + "3" * 64, source_hash, 100, 200],
                participant_id,
                source_hash,
                200,
            )
        with self.assertRaises(registration.RegistrationError):
            registration.validate_eligibility(
                [participant_id, source_hash, False], participant_id, source_hash
            )

    def test_rpc_configuration_accepts_ordered_fallbacks_and_skips_bad_entries(self) -> None:
        configured = (
            "https://first.example/rpc, https//malformed.example/rpc, "
            "https://second.example/rpc, https://first.example/rpc"
        )
        self.assertEqual(
            registration.parse_rpc_urls(configured),
            ["https://first.example/rpc", "https://second.example/rpc"],
        )

    def test_rpc_configuration_requires_a_credential_free_https_endpoint(self) -> None:
        for configured in ("", "http://base.example/rpc", "https://user:pass@base.example/rpc"):
            with self.subTest(configured=configured), self.assertRaises(registration.RegistrationError):
                registration.parse_rpc_urls(configured)

    def test_rpc_selection_uses_first_endpoint_that_reports_base_mainnet(self) -> None:
        calls: list[list[str]] = []
        registry = "0x" + "9" * 40

        def fake_run(command: list[str]) -> str:
            calls.append(command)
            if command[1] == "chain-id":
                endpoint = command[-1]
                if endpoint == "https://down.example/rpc":
                    raise registration.RegistrationError("endpoint unavailable")
                if endpoint == "https://wrong-chain.example/rpc":
                    return "1"
                return registration.BASE_CHAIN_ID
            self.assertEqual(command[1], "call")
            self.assertEqual(command[4], registry)
            return "0x" + "8" * 40

        with patch.object(registration, "run", side_effect=fake_run):
            selected = registration.select_base_rpc(
                "cast",
                "https://down.example/rpc,https://wrong-chain.example/rpc,https://base.example/rpc",
                registry,
            )

        self.assertEqual(selected, "https://base.example/rpc")
        self.assertEqual(len(calls), 4)
        self.assertTrue(all(call[1:3] == ["chain-id", "--rpc-url"] for call in calls[:3]))
        self.assertEqual(calls[3][1:3], ["call", "--rpc-url"])

    def test_rpc_selection_skips_chain_only_endpoint_without_registry_read(self) -> None:
        calls: list[list[str]] = []
        registry = "0x" + "9" * 40

        def fake_run(command: list[str]) -> str:
            calls.append(command)
            if command[1] == "chain-id":
                return registration.BASE_CHAIN_ID
            endpoint = command[3]
            if endpoint == "https://chain-only.example/rpc":
                raise registration.RegistrationError("archive read refused")
            return "0x" + "8" * 40

        with patch.object(registration, "run", side_effect=fake_run):
            selected = registration.select_base_rpc(
                "cast",
                "https://chain-only.example/rpc,https://healthy.example/rpc",
                registry,
            )

        self.assertEqual(selected, "https://healthy.example/rpc")
        self.assertEqual(
            [call[3] for call in calls if call[1] == "call"],
            ["https://chain-only.example/rpc", "https://healthy.example/rpc"],
        )

    def test_register_uses_selected_rpc_for_every_chain_operation(self) -> None:
        registry = "0x" + "9" * 40
        wallet = "0x" + "a" * 40
        attester = "0x" + "8" * 40
        participant_id = "0x" + "1" * 64
        source_hash = "0x" + "2" * 64
        digest = "0x" + "3" * 64
        transaction_hash = "0x" + "4" * 64
        selected_rpc = "https://healthy.example/rpc"
        raw_rpcs = "https://chain-only.example/rpc,https://healthy.example/rpc"
        now = 1_786_000_000
        valid_until = now + 30 * 24 * 60 * 60
        chain_commands: list[list[str]] = []

        def fake_run(command: list[str]) -> str:
            if "--rpc-url" in command:
                chain_commands.append(command)
                self.assertEqual(command[command.index("--rpc-url") + 1], selected_rpc)
            if command[1:3] == ["wallet", "address"]:
                return attester
            if command[1] == "keccak":
                return participant_id if "github-user-v1" in command[2] else source_hash
            if command[1] == "send":
                return '{"transactionHash":"' + transaction_hash + '","status":"0x1"}'
            if command[1] == "wallet" and command[2] == "sign":
                return "0x" + "5" * 130
            if command[1] == "call" and "attester()(address)" in command:
                return attester
            if command[1] == "call" and "nonces(address)(uint256)" in command:
                return "0"
            if command[1] == "call" and any(
                value.startswith("attestationDigest(") for value in command
            ):
                return digest
            if command[1] == "call" and any(
                value.startswith("participants(address)") for value in command
            ):
                return (
                    '["'
                    + participant_id
                    + '","'
                    + source_hash
                    + '",'
                    + str(now)
                    + ','
                    + str(valid_until)
                    + "]"
                )
            if command[1] == "call" and any(
                value.startswith("eligibleAt(address,uint64)") for value in command
            ):
                return '["' + participant_id + '","' + source_hash + '",true]'
            raise AssertionError(f"unexpected command: {command}")

        args = SimpleNamespace(
            registry=registry,
            cast="cast",
            rpc_url=raw_rpcs,
        )
        request = registration.RegistrationRequest(
            repository="NSPG13/agent-bounties",
            issue_number=333,
            github_login="solver-agent",
            github_user_id=12345,
            wallet=wallet,
        )
        with (
            patch.dict(
                registration.os.environ,
                {
                    "PARTICIPANT_ATTESTER_PRIVATE_KEY": "attester-key",
                    "BASE_KEEPER_PRIVATE_KEY": "keeper-key",
                },
            ),
            patch.object(registration, "select_base_rpc", return_value=selected_rpc),
            patch.object(registration, "run", side_effect=fake_run),
            patch.object(registration.time, "time", return_value=now),
        ):
            result = registration.register(args, request)

        self.assertEqual(result["transaction_hash"], transaction_hash)
        self.assertGreaterEqual(len(chain_commands), 5)

    def test_post_receipt_error_preserves_transaction_evidence(self) -> None:
        error = registration.RegistrationError(
            "confirmation failed", {"transaction_hash": "0x" + "4" * 64}
        )
        self.assertEqual(error.evidence["transaction_hash"], "0x" + "4" * 64)


if __name__ == "__main__":
    unittest.main()
