import unittest
from unittest.mock import patch

from agent_bounties.smoke import local_fixture_client


class LocalSmokeFixtureTests(unittest.TestCase):
    def test_remote_and_ambiguous_urls_are_rejected_before_creating_client(self):
        with patch.dict("os.environ", {"OPERATOR_API_TOKEN": "local-fixture-only"}), patch(
            "agent_bounties.smoke.AgentBountiesClient"
        ) as client:
            for url in (
                "https://api.agentizable.ai",
                "http://remote.example",
                "http://localhost.remote.example",
                "http://localhost@remote.example",
                "http://token@127.0.0.1",
                "http://127.0.0.1/private",
                "http://127.0.0.1?forward=remote",
                "http://127.0.0.1#remote",
            ):
                with self.subTest(url=url), self.assertRaises(ValueError):
                    local_fixture_client(url)
            client.assert_not_called()

    def test_local_fixture_requires_and_explicitly_supplies_credentials(self):
        for token in ("", " "):
            with patch.dict("os.environ", {"OPERATOR_API_TOKEN": token}), self.assertRaises(ValueError):
                local_fixture_client("http://127.0.0.1:18280")
        with patch.dict("os.environ", {"OPERATOR_API_TOKEN": "local-fixture-only"}), patch(
            "agent_bounties.smoke.AgentBountiesClient"
        ) as client:
            local_fixture_client("http://localhost:18280/")
            client.assert_called_once_with(
                "http://localhost:18280/", operator_api_token="local-fixture-only"
            )

    def test_wallet_fixture_uses_only_the_synthetic_session(self):
        with patch.dict("os.environ", {
            "OPERATOR_API_TOKEN": "local-operator",
            "AGENT_BOUNTIES_SMOKE_WALLET_SESSION": "abws_local-wallet",
        }), patch("agent_bounties.smoke.AgentBountiesClient") as client:
            local_fixture_client("http://127.0.0.1:18280", wallet=True)
            client.assert_called_once_with("http://127.0.0.1:18280", session_token="abws_local-wallet")
