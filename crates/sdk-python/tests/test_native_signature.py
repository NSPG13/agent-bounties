import unittest

from agent_bounties.client import AgentBountiesClient


class StubAgentBountiesClient(AgentBountiesClient):
    def __init__(self, responses):
        self.responses = list(responses)
        self.requests = []

    def _request(self, method, path, **kwargs):
        self.requests.append(kwargs["json"].copy())
        return self.responses.pop(0)


class NativeSignatureTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
