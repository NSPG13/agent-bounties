"""Signing-boundary tests use public fixture keys, never hosted services."""
import copy
import importlib.util
from pathlib import Path
import unittest
from eth_account import Account
from eth_account.messages import encode_typed_data

spec=importlib.util.spec_from_file_location("hosted",Path(__file__).with_name("check-posting-hosted-sepolia.py"))
h=importlib.util.module_from_spec(spec);spec.loader.exec_module(h)


class HostedSigningTests(unittest.TestCase):
    def setUp(self):
        self.key="11"*32
        self.address=Account.from_key(self.key).address.lower()
        self.wallets={"fixture":{"private_key":self.key,"address":self.address}}
        self.message={"authorizer":self.address,"nonce":"0x"+"22"*32}
        self.typed={"domain":{"chainId":84532,"name":"USDC","version":"2","verifyingContract":h.funding.USDC},
                    "types":{"CancelAuthorization":[{"name":"authorizer","type":"address"},{"name":"nonce","type":"bytes32"}]},
                    "primaryType":"CancelAuthorization","message":self.message.copy()}
        encoded=encode_typed_data(full_message=self.typed)
        self.requirement={"signer":self.address,"typed_data":self.typed,"digest":h.keccak256(b"\x19"+encoded.version+encoded.header+encoded.body)}

    def signing(self):
        return h.sign_requirement(self.requirement,self.wallets,self.address,h.funding.USDC,"CancelAuthorization",self.message,"USDC","2")

    def test_exact_signature_recovers_fixture(self):
        signature=self.signing()
        self.assertEqual(Account.recover_message(encode_typed_data(full_message=self.typed),signature=signature).lower(),self.address)

    def test_domain_mutations_fail_before_signing(self):
        for field,value in (("chainId",8453),("name","USD Coin"),("version","1"),("verifyingContract",self.address)):
            with self.subTest(field=field):
                old=self.typed["domain"][field];self.typed["domain"][field]=value
                with self.assertRaises(ValueError):self.signing()
                self.typed["domain"][field]=old

    def test_message_and_digest_mutations_fail(self):
        self.typed["message"]["nonce"]="0x"+"33"*32
        with self.assertRaises(ValueError):self.signing()
        self.typed["message"]=self.message.copy()
        self.requirement["digest"]="0x"+"00"*32
        with self.assertRaises(ValueError):self.signing()

    def test_original_address_stable_through_increase_and_review(self):
        plans=h.new_plan()
        self.assertEqual(len(plans),9)
        self.assertEqual(len({h.predict(p["request"]) for p in plans[:6]}),1)
        self.assertEqual(len({h.predict(p["request"]) for p in plans[6:]}),1)
        self.assertNotEqual(h.predict(plans[0]["request"]),h.predict(plans[6]["request"]))
        changed=copy.deepcopy(plans[0]["request"]);changed["original_terms"]["solver_reward_micro_usdc"]+=1
        self.assertNotEqual(h.predict(changed),h.predict(plans[0]["request"]))


if __name__=="__main__":unittest.main()
