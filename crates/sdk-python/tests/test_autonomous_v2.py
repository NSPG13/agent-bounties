import json
import unittest
from unittest.mock import patch

import httpx

from agent_bounties.client import AgentBountiesClient


class AutonomousV2Tests(unittest.TestCase):
    def test_quote_and_creation_plans_post_the_exact_routes_and_bodies(self):
        calls = []
        quote = {
            "protocol_version": "agent-bounties/autonomous-v2",
            "solver_reward": "1000000",
            "verifier_reward": "100000",
            "claim_bond": "100000",
            "platform_fee_bps": 750,
            "platform_fee": "75000",
            "platform_fee_recipient": "0x884834e884d6e93462655a2820140ad03e6747bc",
            "target_amount": "1175000",
            "fee_boundary": "fee is paid by the poster",
        }

        def handle(request):
            calls.append(request)
            return httpx.Response(200, json=quote)

        reward = {"amount": 1_000_000, "currency": "USDC"}
        verifier = {"amount": 100_000, "currency": "USDC"}
        create = {"creator": "0x" + "11" * 20, "terms_hash": "0x" + "22" * 32}
        signature = {"v": 27, "r": "0x" + "33" * 32, "s": "0x" + "44" * 32}
        relayer = "0x" + "55" * 20
        with httpx.Client(transport=httpx.MockTransport(handle)) as transport, patch(
            "agent_bounties.client.httpx.request", side_effect=transport.request
        ):
            client = AgentBountiesClient("https://example.test")
            result = client.quote_autonomous_v2_bounty(reward, verifier, network="base-sepolia")
            client.plan_autonomous_v2_bounty_creation(create)
            client.plan_autonomous_v2_bounty_authorized_creation(
                create, signature, network="base-sepolia", relayer=relayer
            )

        self.assertEqual(
            int(result["target_amount"]),
            int(result["solver_reward"]) + int(result["verifier_reward"]) + int(result["platform_fee"]),
        )
        self.assertEqual(
            [request.url.path for request in calls],
            [
                "/v1/base/autonomous-bounties/v2/quote",
                "/v1/base/autonomous-bounties/v2/creation-plan",
                "/v1/base/autonomous-bounties/v2/authorized-creation-plan",
            ],
        )
        self.assertTrue(all(request.method == "POST" for request in calls))
        self.assertEqual(
            json.loads(calls[0].content),
            {"network": "base-sepolia", "solver_reward": reward, "verifier_reward": verifier},
        )
        self.assertEqual(json.loads(calls[1].content), {"network": None, "create": create})
        self.assertEqual(
            json.loads(calls[2].content),
            {"network": "base-sepolia", "create": create, "signature": signature, "relayer": relayer},
        )


if __name__ == "__main__":
    unittest.main()
