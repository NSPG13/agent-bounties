import json
import unittest
from unittest.mock import patch
import httpx
from agent_bounties.client import AgentBountiesClient


class BroadVerificationTests(unittest.TestCase):
    def test_exact_files_plan_identity_and_caller_credentials_survive_transport(self):
        calls = []
        def handle(request):
            calls.append(request)
            return httpx.Response(200, json={"payment_authorized": False})
        artifact = '00000000-0000-4000-8000-000000000001'
        plan = {"schema_version": "broad-verification/v1", "checks": [{"exact_integer": 9223372036854775807}]}
        with httpx.Client(transport=httpx.MockTransport(handle)) as transport, patch('agent_bounties.client.httpx.request', side_effect=transport.request):
            client = AgentBountiesClient('https://example.test', session_token='abws_fixture')
            client.upload_verification_artifact([{"path":"data.json", "base64":"e30="}], 'same-input')
            client.run_verification_checks(artifact, plan)
            client.assign_review_task(artifact, 'wallet:8453:0x' + '12' * 20)
            client.get_verification_run(artifact)
            client.open_artifact_dispute(artifact, 'Please keep these files.')
            client.resolve_artifact_dispute(artifact, 'My review is done.')
        self.assertEqual(calls[0].headers['idempotency-key'], 'same-input')
        self.assertEqual(json.loads(calls[1].content)['plan'], plan)
        self.assertEqual(json.loads(calls[2].content), {'assignee_principal': 'wallet:8453:0x'+'12'*20})
        self.assertEqual(calls[3].method, 'GET')
        self.assertEqual(calls[4].url.path, f'/v1/verification/artifacts/{artifact}/disputes')
        self.assertEqual(json.loads(calls[4].content), {'reason':'Please keep these files.'})
        self.assertEqual(calls[5].url.path, f'/v1/verification/disputes/{artifact}/resolve')
        for request in calls:
            self.assertEqual(request.headers['authorization'], 'Bearer abws_fixture')
            self.assertNotIn('x-operator-token', request.headers)
