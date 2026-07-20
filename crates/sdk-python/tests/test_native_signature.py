import hashlib
import inspect
import json
from pathlib import Path
import unittest
from unittest.mock import patch

import agent_bounties
import httpx
from agent_bounties.client import AgentBountiesClient


class StubAgentBountiesClient(AgentBountiesClient):
    def __init__(self, responses):
        self.responses = list(responses)
        self.requests = []

    def _request(self, method, path, **kwargs):
        self.requests.append(kwargs["json"].copy())
        return self.responses.pop(0)


class NativeSignatureTests(unittest.TestCase):
    def test_public_api_matches_compatibility_fixture(self):
        fixture = json.loads(
            (Path(__file__).parents[1] / "fixtures/public-api.json").read_text(encoding="utf-8")
        )
        public_methods = {
            name: str(inspect.signature(member))
            for name, member in inspect.getmembers(AgentBountiesClient, inspect.isfunction)
            if not name.startswith("_")
        }
        canonical = json.dumps(
            {"exports": sorted(agent_bounties.__all__), "methods": public_methods},
            sort_keys=True,
            separators=(",", ":"),
        )
        self.assertEqual(len(agent_bounties.__all__), fixture["export_count"])
        self.assertEqual(len(public_methods), fixture["public_method_count"])
        self.assertEqual(hashlib.sha256(canonical.encode()).hexdigest(), fixture["canonical_sha256"])

    def test_agent_native_claim_replays_wallet_signature_unchanged(self):
        wallet_signature = f"0x{'11' * 64}1b"
        client = StubAgentBountiesClient(
            [
                {
                    "signing_payload": {"primaryType": "ReceiveWithAuthorization"},
                    "candidate": {"status": "authorization_ready"},
                },
                {
                    "signing_payload": None,
                    "candidate": {"status": "claimed"},
                    "canonical_event_id": "00000000-0000-0000-0000-000000000001",
                },
            ]
        )

        response = client.agent_native_claim(
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
            signer=lambda _payload: wallet_signature,
            idempotency_key="native-signature-test",
        )

        self.assertEqual(response["candidate"]["status"], "claimed")
        self.assertEqual(len(client.requests), 2)
        self.assertEqual(client.requests[1]["wallet_signature"], wallet_signature)
        self.assertNotIn("signature", client.requests[1])

    def test_canonical_child_plan_sends_task_acceptance_criteria(self):
        client = StubAgentBountiesClient([{"benchmark_hash": "0x1234"}])
        criteria = ["The committed regression test passes."]

        client.plan_autonomous_canonical_child_terms(
            f"0x{'11' * 32}",
            1,
            "0x2222222222222222222222222222222222222222",
            {"amount": 2_000_000, "currency": "usdc"},
            criteria,
            "0x3333333333333333333333333333333333333333",
        )

        self.assertEqual(client.requests[0]["child_acceptance_criteria"], criteria)

    def test_compile_objective_sends_bounded_graph_request(self):
        client = StubAgentBountiesClient(
            [{"schema_version": "agent-bounties/cloud-objective-plan-v1"}]
        )

        response = client.compile_objective(
            "Ship a replayable release",
            constraints=["Keep settlement deterministic."],
            max_tasks=4,
            solver_budget_usdc="8.00",
        )

        self.assertEqual(
            response["schema_version"],
            "agent-bounties/cloud-objective-plan-v1",
        )
        self.assertEqual(client.requests[0]["max_tasks"], 4)
        self.assertEqual(client.requests[0]["solver_budget_usdc"], "8.00")

    def test_shared_http_builder_preserves_false_zero_and_headers(self):
        client = AgentBountiesClient("https://example.test", "operator")
        response = httpx.Response(200, json={"ok": True}, request=httpx.Request("GET", "https://example.test"))
        with patch("agent_bounties.client.httpx.request", return_value=response) as request:
            client._http(
                "GET",
                "/v1/test",
                params={"false": False, "zero": 0, "none": None},
                headers={"x-extra": "value"},
            )
        self.assertEqual(request.call_args.kwargs["params"], {"false": False, "zero": 0})
        self.assertEqual(
            request.call_args.kwargs["headers"],
            {"x-operator-token": "operator", "x-extra": "value"},
        )

    def test_stripe_event_methods_share_identical_transport_contract(self):
        client = AgentBountiesClient("https://example.test", "operator")
        response = httpx.Response(200, json={"ok": True}, request=httpx.Request("POST", "https://example.test"))
        with patch("agent_bounties.client.httpx.request", return_value=response) as request:
            for method, path in (
                (client.reconcile_stripe_checkout_webhook, "/v1/stripe/checkout-webhooks"),
                (client.reconcile_stripe_transfer_event, "/v1/stripe/transfer-events"),
            ):
                with self.subTest(path=path):
                    self.assertEqual(method({"id": "evt"}, "signature"), {"ok": True})
                    self.assertEqual(request.call_args.args, ("POST", f"https://example.test{path}"))
                    self.assertEqual(
                        request.call_args.kwargs["headers"],
                        {"x-operator-token": "operator", "stripe-signature": "signature"},
                    )


if __name__ == "__main__":
    unittest.main()
