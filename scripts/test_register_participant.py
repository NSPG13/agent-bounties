from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


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

    def test_post_receipt_error_preserves_transaction_evidence(self) -> None:
        error = registration.RegistrationError(
            "confirmation failed", {"transaction_hash": "0x" + "4" * 64}
        )
        self.assertEqual(error.evidence["transaction_hash"], "0x" + "4" * 64)


if __name__ == "__main__":
    unittest.main()
