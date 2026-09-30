import json
import os
import unittest
from unittest.mock import patch

import httpx

from agent_bounties.client import AgentBountiesClient, AgentBountiesHttpError


class AuthenticationTests(unittest.TestCase):
    def test_wallet_and_cookie_clients_do_not_inherit_operator_authority(self):
        with patch.dict(os.environ, {"OPERATOR_API_TOKEN": "fixture-operator"}):
            wallet = AgentBountiesClient(session_token="abws_fixture")
            cookie = AgentBountiesClient(headers={"Cookie": "agent_bounties_session=fixture"})
            self.assertEqual(wallet._headers()["authorization"], "Bearer abws_fixture")
            self.assertNotIn("x-operator-token", wallet._headers())
            self.assertNotIn("x-operator-token", cookie._headers())
            with self.assertRaises(ValueError):
                AgentBountiesClient(operator_api_token="fixture", session_token="abws_fixture")
            with self.assertRaises(ValueError):
                AgentBountiesClient(session_token="abws_fixture", headers={"Authorization": "Bearer other"})

    def test_wallet_challenge_exchange_and_revocation_preserve_the_reviewed_identity(self):
        seen = []

        def handle(request):
            seen.append(request)
            if request.url.path.endswith("/challenge"):
                return httpx.Response(200, json={"challenge_id": "fixture-challenge", "message": "Exact reviewed message", "chain_id": 84532})
            if request.url.path.endswith("/wallet/session"):
                return httpx.Response(200, json={"token": "abws_fixture", "chain_id": 84532})
            if request.url.path.endswith("/revoke"):
                return httpx.Response(204)
            return httpx.Response(200, json={"principal": "wallet"})

        with httpx.Client(transport=httpx.MockTransport(handle)) as transport:
            with patch("agent_bounties.client.httpx.request", side_effect=transport.request), patch.dict(os.environ, {}, clear=True):
                public = AgentBountiesClient("https://example.test")
                challenge = public.create_wallet_challenge("0x" + "12" * 20, 84532)
                session = public.create_wallet_session(challenge["challenge_id"], "reviewed-signature", 84532)
                self.assertIsNone(public.session_token)
                wallet = AgentBountiesClient("https://example.test", session_token=session["token"])
                self.assertEqual(wallet.get_auth_session(), {"principal": "wallet"})
                self.assertTrue(wallet.revoke_auth_session()["success"])
                self.assertNotIn("authorization", wallet._headers())

        self.assertEqual(json.loads(seen[0].content), {"address": "0x" + "12" * 20, "chain_id": 84532})
        self.assertEqual(json.loads(seen[1].content), {"challenge_id": "fixture-challenge", "signature": "reviewed-signature", "chain_id": 84532})
        self.assertNotIn("authorization", seen[0].headers)
        self.assertNotIn("authorization", seen[1].headers)
        self.assertEqual(seen[2].headers["authorization"], "Bearer abws_fixture")
        self.assertEqual(seen[3].headers["authorization"], "Bearer abws_fixture")

    def test_publication_sends_only_prepared_content_once_and_keeps_idempotency(self):
        seen = []
        prepared = {
            "title": "A prepared request",
            "goal": "Use the exact content I reviewed.",
            "acceptance_criteria": ["The requested evidence is attached."],
            "idempotency_key": "fixture-prepared-v1",
            "source_url": "https://example.test/task",
        }

        def handle(request):
            seen.append(request)
            return httpx.Response(200, json={"bounty": prepared, "demo_agent_solution": None})

        with httpx.Client(transport=httpx.MockTransport(handle)) as transport:
            with patch("agent_bounties.client.httpx.request", side_effect=transport.request):
                client = AgentBountiesClient("https://example.test", session_token="abws_fixture")
                response = client.publish_unfunded_bounty(**prepared)
        self.assertEqual(len(seen), 1)
        self.assertEqual(seen[0].url.path, "/v1/unfunded-bounties")
        self.assertEqual(seen[0].method, "POST")
        self.assertEqual(json.loads(seen[0].content), prepared)
        self.assertEqual(seen[0].headers["authorization"], "Bearer abws_fixture")
        self.assertIsNone(response["demo_agent_solution"])

    def test_retired_generation_errors_are_permanent_and_never_follow_redirects(self):
        seen = []

        def handle(request):
            seen.append(request)
            return httpx.Response(410, json={"error_code": "hosted_generation_removed", "retryable": False})

        with httpx.Client(transport=httpx.MockTransport(handle)) as transport:
            with patch("agent_bounties.client.httpx.request", side_effect=transport.request) as request:
                client = AgentBountiesClient("https://example.test", session_token="abws_fixture")
                with self.assertRaises(AgentBountiesHttpError) as caught:
                    client.compile_objective("Prepare this in my own AI")
                self.assertFalse(request.call_args.kwargs["follow_redirects"])
        self.assertEqual(len(seen), 1)
        self.assertEqual(caught.exception.status_code, 410)
        self.assertEqual(caught.exception.error_code, "hosted_generation_removed")
        self.assertFalse(caught.exception.retryable)
        self.assertIn("your own AI account", caught.exception.next_action)
        self.assertIn("post.html", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
