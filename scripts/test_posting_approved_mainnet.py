import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from eth_account import Account
from hexbytes import HexBytes
import posting_approved_mainnet as mainnet


class ApprovedMainnetTests(unittest.TestCase):
    def setUp(self):
        self.bundle, self.action = mainnet.reviewed_call()
        self.fixture = Account.from_key('0x' + '01' * 32)

    def record(self, **changes):
        tx = {'chainId': 8453, 'nonce': 153, 'value': 0, 'gas': 4233944,
              'type': 2, 'accessList': [], 'maxFeePerGas': 2_000_000,
              'maxPriorityFeePerGas': 1_000_000,
              'to': HexBytes(mainnet.TARGET), 'data': self.action['data']}
        tx.update(changes)
        signed = self.fixture.sign_transaction(tx)
        return {'raw': '0x' + signed.raw_transaction.hex(), 'hash': mainnet.digest(signed.raw_transaction),
                'fee_cap': 2_000_000, 'priority': 1_000_000, 'reserved_wei': 10**14}

    def test_exact_approved_projection(self):
        self.assertEqual(self.bundle['approved_review_sha256'], mainnet.REVIEW_SHA)
        self.assertEqual(self.action['gas_limit'], 4233944)
        self.assertEqual(self.action['value_wei'], '0')

    def test_wrong_signer_never_passes_scope_validation(self):
        with self.assertRaisesRegex(RuntimeError, 'signature'):
            mainnet.validate_record(self.record(), self.action)

    def test_signed_scope_changes_fail_closed(self):
        with patch.object(mainnet.Account, 'recover_transaction', return_value=mainnet.SENDER):
            mainnet.validate_record(self.record(), self.action)
            for mutation in ({'chainId': 84532}, {'nonce': 154}, {'value': 1}, {'gas': 4233945},
                             {'data': '0xabcd'}, {'to': HexBytes('0x' + '12' * 20)},
                             {'maxFeePerGas': 3_000_000}, {'maxPriorityFeePerGas': 2_000_000}):
                with self.subTest(mutation=mutation), self.assertRaisesRegex(RuntimeError, 'scope'):
                    mainnet.validate_record(self.record(**mutation), self.action)

    def test_readonly_rpc_cannot_broadcast(self):
        with patch.object(mainnet, 'request') as request:
            with self.assertRaisesRegex(RuntimeError, 'forbidden'):
                mainnet.rpc('eth_sendRawTransaction', ['0x00'])
            request.assert_not_called()

    def test_wrong_network_or_stale_head_blocks(self):
        with patch.object(mainnet, 'rpc', return_value='0x14a34'):
            with self.assertRaisesRegex(RuntimeError, 'Wrong chain'):
                mainnet.fresh_head()
        with patch.object(mainnet, 'rpc', side_effect=['0x2105', {'timestamp': '0x1'}]):
            with self.assertRaisesRegex(RuntimeError, 'Stale'):
                mainnet.fresh_head()

    def test_fee_quote_freshness_and_rounding(self):
        with patch.object(mainnet, 'request', return_value=({'data': {'base': 'ETH', 'currency': 'USDC', 'amount': '3000'}}, {})):
            self.assertEqual(mainnet.fresh_price_bound(1), 1)
            self.assertEqual(mainnet.fresh_price_bound(10**13), 37500)
        with patch.object(mainnet, 'request', return_value=({'data': {'base': 'ETH', 'currency': 'USDC', 'amount': '3000'}}, {'Age': '61'})):
            with self.assertRaisesRegex(RuntimeError, 'Fresh'):
                mainnet.fresh_price_bound(1)

    def test_old_outbox_and_rerun_stop_before_key_access(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'outbox.json'
            path.write_text('{}')
            with patch.object(mainnet, 'JOURNAL', path), patch.object(mainnet.Account, 'from_key') as key:
                with self.assertRaisesRegex(RuntimeError, 'Existing outbox'):
                    mainnet.prepare()
                key.assert_not_called()
            path.unlink()
            with patch.object(mainnet, 'JOURNAL', path), patch.dict(os.environ, {
                    'POSTING_APPROVED_REVIEW_SHA256': mainnet.REVIEW_SHA, 'GITHUB_RUN_ATTEMPT': '2'}), \
                    patch.object(mainnet.Account, 'from_key') as key:
                with self.assertRaisesRegex(RuntimeError, 'no reruns'):
                    mainnet.prepare()
                key.assert_not_called()

    def test_receipt_waits_for_its_block_without_resending(self):
        journal = {'record': self.record()}
        receipt = {'blockNumber': '0x123', 'blockHash': '0x' + 'ab' * 32}
        with patch.object(mainnet.Account, 'recover_transaction', return_value=mainnet.SENDER), \
                patch.object(mainnet, 'rpc', side_effect=[receipt, None]) as rpc:
            self.assertFalse(mainnet.reconcile(journal, self.action))
            self.assertEqual([call.args[0] for call in rpc.call_args_list],
                             ['eth_getTransactionReceipt', 'eth_getBlockByNumber'])

    def test_unknown_send_never_retries(self):
        journal = {'review_sha256': mainnet.REVIEW_SHA, 'run_id': 'fixture-run',
                   'record': self.record(), 'broadcast_attempted': True}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'outbox.json'; path.write_text(json.dumps(journal))
            with patch.object(mainnet, 'JOURNAL', path), patch.object(mainnet.Account, 'recover_transaction', return_value=mainnet.SENDER), \
                    patch.dict(os.environ, {'GITHUB_RUN_ID': 'fixture-run', 'POSTING_DURABLE_ARTIFACT_ID': '123'}), \
                    patch.object(mainnet, 'reconcile', return_value=False), patch.object(mainnet, 'rpc') as rpc:
                with self.assertRaisesRegex(RuntimeError, 'Unknown prior send'):
                    mainnet.broadcast()
                rpc.assert_not_called()


if __name__ == '__main__':
    unittest.main()
