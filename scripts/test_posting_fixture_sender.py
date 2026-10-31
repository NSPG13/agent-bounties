"""Fail-closed recovery checks for the isolated rehearsal's durable sender."""
import copy
import types
import unittest
from eth_account import Account
from hexbytes import HexBytes
import posting_sepolia_fixture_sender as f

class FixtureSenderRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.signer=Account.from_key('11'*32)
        self.scope={'chainId':84532,'nonce':7,'value':0,'gas':120000,'type':2,'maxFeePerGas':1000000,'maxPriorityFeePerGas':1000000,'accessList':[],
                    'to':'0x036cbd53842c5426634e7929541ec2318f3dcf7e','data':'0xdeadbeef'}
        signed=self.signer.sign_transaction({**self.scope,'to':HexBytes(self.scope['to'])})
        self.record={'raw':'0x'+signed.raw_transaction.hex(),'hash':f.keccak256(signed.raw_transaction),'transaction':copy.deepcopy(self.scope),'reserved_wei':2000}
        def require(ok,message):
            if not ok:raise ValueError(message)
        self.h=types.SimpleNamespace(ex=types.SimpleNamespace(require=require,oracle_fee=lambda *a:2),
            funding=types.SimpleNamespace(ADDRESSES={'relayer':self.signer.address.lower()}),
            receipt=lambda *a:{'block_hash':'0x'+'22'*32,'gas_used':100})
        self.reads=[]
    def rpc(self,method,params):
        self.reads.append(method);self.assertEqual(method,'eth_getTransactionReceipt')
        return {'transactionHash':self.record['hash'],'blockHash':'0x'+'22'*32,'to':self.scope['to'],'effectiveGasPrice':'0xa','l1Fee':'0x64'}
    def test_lowercase_address_signed_as_bytes_recovers_with_full_fees(self):
        result=f.reconcile(self.h,self.record,self.rpc)
        self.assertEqual(result['fee_wei'],'1102')
        self.assertEqual(self.reads,['eth_getTransactionReceipt'])
    def test_saved_nonce_and_fee_mutations_stop_before_rpc(self):
        for field in ('nonce','gas','chainId','maxFeePerGas','maxPriorityFeePerGas'):
            with self.subTest(field=field):
                record=copy.deepcopy(self.record);record['transaction'][field]+=1
                with self.assertRaisesRegex(ValueError,'metadata differs'):f.reconcile(self.h,record,self.rpc)
        self.assertEqual(self.reads,[])
    def test_unknown_receipt_never_broadcasts_or_replaces(self):
        methods=[]
        def unknown(method,params):methods.append(method);return None
        with self.assertRaisesRegex(ValueError,'unknown'):f.reconcile(self.h,self.record,unknown)
        self.assertEqual(methods,['eth_getTransactionReceipt'])
    def test_fee_over_reservation_is_rejected(self):
        self.record['reserved_wei']=1101
        with self.assertRaisesRegex(ValueError,'exceeded reservation'):f.reconcile(self.h,self.record,self.rpc)

    def test_receipt_fees_cannot_be_combined_across_forks(self):
        def changed(method,params):
            return {**self.rpc(method,params),'blockHash':'0x'+'33'*32}
        with self.assertRaisesRegex(ValueError,'receipt scope differs'):f.reconcile(self.h,self.record,changed)

if __name__=='__main__':unittest.main()
