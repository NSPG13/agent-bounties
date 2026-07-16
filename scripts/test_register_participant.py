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


if __name__ == "__main__":
    unittest.main()
