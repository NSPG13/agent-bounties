import json
import os
import unittest
from unittest.mock import patch

import httpx
from agent_bounties.client import AgentBountiesClient, AgentBountiesHttpError


class ManagedServiceTests(unittest.TestCase):
    def test_account_calls_preserve_exact_acceptance_and_do_not_inherit_operator(self):
        seen = []

        def handle(request):
            seen.append(request)
            return httpx.Response(
                409 if request.url.path.endswith('/checkout') else 200,
                json={'status': 'awaiting_payment'},
            )

        with httpx.Client(transport=httpx.MockTransport(handle)) as transport:
            with (
                patch('agent_bounties.client.httpx.request', side_effect=transport.request),
                patch.dict(os.environ, {'OPERATOR_API_TOKEN': 'must-not-inherit'}),
            ):
                client = AgentBountiesClient('https://api.example', headers={
                    'Cookie': 'agent_bounties_session=fixture',
                    'Origin': 'https://agentizable.ai',
                })
                identifier = '00000000-0000-4000-8000-000000000001'
                client.read_managed_service_quote(identifier)
                client.accept_managed_service_quote(identifier, 'a' * 64, 'accepted-key')
                client.get_managed_service_order(identifier)
                client.request_managed_service_refund(identifier)
                with self.assertRaises(AgentBountiesHttpError):
                    client.open_managed_service_checkout(identifier)
                with self.assertRaises(ValueError):
                    client.get_managed_service_order('../stripe/connect-transfers')

        self.assertEqual(len(seen), 5)
        self.assertEqual(json.loads(seen[1].content), {
            'quote_digest': 'a' * 64, 'idempotency_key': 'accepted-key',
        })
        self.assertEqual(json.loads(seen[3].content), {})
        self.assertEqual(seen[3].url.path, '/v1/managed-service/orders/' + identifier + '/refund')
        self.assertEqual(json.loads(seen[4].content), {})
        for request in seen:
            self.assertNotIn('x-operator-token', request.headers)
            self.assertEqual(request.headers['origin'], 'https://agentizable.ai')

    def test_card_bounty_quote_sends_draft_and_rejects_non_integer_rewards(self):
        seen = []

        def handle(request):
            seen.append(request)
            return httpx.Response(200, json={})

        with httpx.Client(transport=httpx.MockTransport(handle)) as transport:
            with patch('agent_bounties.client.httpx.request', side_effect=transport.request):
                client = AgentBountiesClient('https://api.example', headers={
                    'Cookie': 'agent_bounties_session=fixture',
                    'Origin': 'https://agentizable.ai',
                })
                document = {'title': 'Fixture'}
                client.issue_managed_bounty_quote(document, 10_000_000)
                for bad in (0, -1, 1.5, True, '10'):
                    with self.assertRaises(ValueError):
                        client.issue_managed_bounty_quote(document, bad)

        self.assertEqual(len(seen), 1)
        self.assertEqual(seen[0].url.path, '/v1/managed-service/bounty-quotes')
        self.assertEqual(json.loads(seen[0].content), {
            'document': {'title': 'Fixture'}, 'solver_reward_micro_usdc': 10_000_000,
        })


    def test_bounty_review_sends_only_an_exact_reasoned_verdict(self):
        seen = []

        def handle(request):
            seen.append(request)
            return httpx.Response(200, json={})

        with httpx.Client(transport=httpx.MockTransport(handle)) as transport:
            with patch('agent_bounties.client.httpx.request', side_effect=transport.request):
                client = AgentBountiesClient('https://api.example', headers={
                    'Cookie': 'agent_bounties_session=fixture',
                    'Origin': 'https://agentizable.ai',
                })
                identifier = '00000000-0000-4000-8000-000000000001'
                hashes = ('0x' + '11' * 32, '0x' + '22' * 32)
                client.get_managed_bounty_review(identifier)
                client.record_managed_bounty_review(
                    identifier, 1, *hashes, False, 'The artifact omits the requested report.')
                client.record_managed_bounty_review(identifier, 2, *hashes, True)
                for bad in (
                    (0, *hashes, True, ''),
                    (True, *hashes, True, ''),
                    (1, '0x' + 'AA' * 32, hashes[1], True, ''),
                    (1, *hashes, 'yes', ''),
                    (1, *hashes, False, 'Too short.'),
                    (1, *hashes, True, ' padded'),
                ):
                    with self.assertRaises(ValueError):
                        client.record_managed_bounty_review(identifier, *bad)

        self.assertEqual(len(seen), 3)
        self.assertEqual(seen[0].url.path, '/v1/managed-service/orders/' + identifier + '/review')
        self.assertEqual(json.loads(seen[1].content), {
            'round': 1, 'submission_hash': '0x' + '11' * 32, 'evidence_hash': '0x' + '22' * 32,
            'passed': False, 'reason': 'The artifact omits the requested report.',
        })
        self.assertEqual(json.loads(seen[2].content)['reason'], '')

    def test_card_bounty_appeal_sends_only_the_signed_appeal_and_operator_ruling(self):
        seen = []

        def handle(request):
            seen.append(request)
            return httpx.Response(200, json={})

        with httpx.Client(transport=httpx.MockTransport(handle)) as transport:
            with patch('agent_bounties.client.httpx.request', side_effect=transport.request):
                client = AgentBountiesClient('https://api.example')
                bounty = '0x' + 'AB' * 20
                solver = '0x' + '5E' * 20
                reason = 'The report is in section 2 of the artifact.'
                signature = '0x' + '12' * 65
                client.get_card_bounty_review('base-sepolia', bounty)
                client.appeal_card_bounty_review('base-sepolia', bounty, 2, solver, reason, signature)
                client.list_card_bounty_appeals()
                client.rule_card_bounty_appeal('base-sepolia', bounty, 2, True,
                                               'Section 2 contains the requested report.')
                for bad in (
                    (0, solver, reason, signature),
                    (True, solver, reason, signature),
                    (2, 'alice', reason, signature),
                    (2, solver, 'Disagree.', signature),
                    (2, solver, reason, '0x1'),
                ):
                    with self.assertRaises(ValueError):
                        client.appeal_card_bounty_review('base-sepolia', bounty, *bad)
                for network, contract in (('ethereum', bounty), ('base-sepolia', '../appeals')):
                    with self.assertRaises(ValueError):
                        client.get_card_bounty_review(network, contract)
                with self.assertRaises(ValueError):
                    client.rule_card_bounty_appeal('base-sepolia', bounty, 2, 'yes', 'x' * 30)

        self.assertEqual(len(seen), 4)
        lower = '0x' + 'ab' * 20
        self.assertEqual([r.url.path for r in seen], [
            f'/v1/managed-service/bounties/base-sepolia/{lower}/review',
            f'/v1/managed-service/bounties/base-sepolia/{lower}/appeal',
            '/v1/managed-service/appeals',
            f'/v1/managed-service/appeals/base-sepolia/{lower}/2/ruling',
        ])
        self.assertEqual(json.loads(seen[1].content), {
            'round': 2, 'solver': solver.lower(), 'reason': reason, 'signature': signature,
        })
        self.assertEqual(json.loads(seen[3].content), {
            'reversed': True, 'reason': 'Section 2 contains the requested report.',
        })

    def test_deposit_payouts_record_only_an_exact_transaction_hash(self):
        seen = []

        def handle(request):
            seen.append(request)
            return httpx.Response(200, json={})

        with httpx.Client(transport=httpx.MockTransport(handle)) as transport:
            with patch('agent_bounties.client.httpx.request', side_effect=transport.request):
                client = AgentBountiesClient('https://api.example')
                bounty = '0x' + 'ab' * 20
                tx_hash = '0x' + 'A1' * 32
                client.list_card_bounty_deposit_payouts()
                client.record_card_bounty_deposit_payout('base-mainnet', bounty, tx_hash)
                for bad in (('base-mainnet', bounty, '0x12'), ('base-mainnet', '../appeals', tx_hash)):
                    with self.assertRaises(ValueError):
                        client.record_card_bounty_deposit_payout(*bad)

        self.assertEqual([r.url.path for r in seen], [
            '/v1/managed-service/deposit-payouts',
            f'/v1/managed-service/deposit-payouts/base-mainnet/{bounty}',
        ])
        self.assertEqual(json.loads(seen[1].content), {'tx_hash': tx_hash.lower()})
