import json
import time
import unittest
from unittest import mock
import gas_sponsorship_budget as budget
import relay_autonomous_action as relay

class BudgetTests(unittest.TestCase):
    def client(self):
        client=relay.CastClient("cast","https://fixture.invalid")
        client.chain_id=mock.Mock(return_value=8453)
        client.keeper_address=mock.Mock(return_value="0x"+"11"*20)
        client.gas_price=mock.Mock(return_value=1000)
        client.call=mock.Mock(side_effect=["100", "10"])
        client.run=mock.Mock(side_effect=["0x12345678",'{"transactionHash":"0xfixture"}'])
        return client
    def opener(self, alter=lambda value:value):
        def opened(request, timeout):
            self.assertEqual(request.full_url,budget.ENDPOINT)
            self.assertEqual(request.get_header("Authorization"),"Bearer "+"fixture-token-"*4)
            body=json.loads(request.data)
            self.assertNotIn("private_key",body)
            reply={**body,"schema":"agent-bounties/operator-gas-reservation-v1","reserved":True,"daily_limit_micro_usdc":1000000,"carries_forward":False,"transaction_submitted":False,"valid_until":int(time.time())+30,"signer_lease_expires_at":int(time.time())+45,"shared_signer_lease":True,"sender_nonce":7}
            response=mock.MagicMock();response.__enter__.return_value.read.return_value=json.dumps(alter(reply)).encode()
            return response
        opener=mock.Mock();opener.open.side_effect=opened
        return opener
    def test_exact_total_reserved_before_send_and_fee_cap_is_enforced(self):
        client=self.client();opener=self.opener()
        with mock.patch.dict("os.environ",{"GAS_SPONSOR_BUDGET_TOKEN":"fixture-token-"*4}), mock.patch.object(budget.urllib.request,"build_opener",return_value=opener):
            previous_timeout=client.command_timeout_seconds
            original_run=client.run
            def bounded_run(*args,**kwargs):
                if args[0]=="send": self.assertLessEqual(client.remaining_command_timeout(),10)
                return original_run(*args,**kwargs)
            client.run=mock.Mock(side_effect=bounded_run)
            result=client.send("key-fixture",100000,"0x"+"22"*20,"submit()")
            self.assertEqual(client.command_timeout_seconds,previous_timeout)
        body=json.loads(opener.open.call_args.args[0].data)
        self.assertEqual(body["maximum_fee_wei"],str(100000*2000+2*(100+10)))
        sent=client.run.call_args.args
        self.assertEqual(sent[0],"send");self.assertEqual(sent[sent.index("--gas-price")+1],"2000")
        self.assertEqual(sent[sent.index("--nonce")+1],"7")
        self.assertEqual(body["relayer_address"],"0x"+"11"*20)
        self.assertEqual(result["transactionHash"],"0xfixture")
    def test_missing_budget_exhaustion_and_mismatched_reply_never_send(self):
        for mode in ["missing","exhausted","wrong","expired","unlocked"]:
            client=self.client();opener=self.opener(lambda value:{**value,"transaction_digest":"wrong"} if mode=="wrong" else {**value,"valid_until":0} if mode=="expired" else {**value,"shared_signer_lease":False} if mode=="unlocked" else value)
            if mode=="exhausted":opener.open.side_effect=RuntimeError("provider private detail")
            with mock.patch.dict("os.environ",{"GAS_SPONSOR_BUDGET_TOKEN":"" if mode=="missing" else "fixture-token-"*4}),mock.patch.object(budget.urllib.request,"build_opener",return_value=opener):
                with self.assertRaises(relay.RelayError) as error:client.send("key-fixture",100000,"0x"+"22"*20,"submit()")
            self.assertNotIn("private detail",str(error.exception))
            self.assertFalse(any(call.args[0]=="send" for call in client.run.call_args_list))
    def test_missing_boolean_negative_or_oversized_nonce_never_sends(self):
        for nonce in [None, True, -1, 2**63, "7"]:
            client=self.client();opener=self.opener(lambda value:{**value,"sender_nonce":nonce})
            with mock.patch.dict("os.environ",{"GAS_SPONSOR_BUDGET_TOKEN":"fixture-token-"*4}), mock.patch.object(budget.urllib.request,"build_opener",return_value=opener):
                with self.assertRaises(relay.RelayError):client.send("key-fixture",100000,"0x"+"22"*20,"submit()")
            self.assertFalse(any(call.args[0]=="send" for call in client.run.call_args_list))
    def test_oracle_failure_and_redirect_fail_closed(self):
        client=self.client();client.call.side_effect=ValueError("bad quote")
        with mock.patch.dict("os.environ",{"GAS_SPONSOR_BUDGET_TOKEN":"fixture-token-"*4}):
            with self.assertRaises(relay.RelayError):client.send("key-fixture",100000,"0x"+"22"*20,"submit()")
        with self.assertRaises(budget.BudgetUnavailable):budget.NoRedirect().redirect_request(None,None,302,"redirect",{},"https://evil.example")
        self.assertFalse(any(call.args[0]=="send" for call in client.run.call_args_list))

    def test_standalone_workflows_share_exact_budget_and_reserved_nonce(self):
        commands=[]
        def run(command, *, timeout):
            commands.append(command)
            self.assertLessEqual(timeout,10)
            if command[1]=="chain-id":return "8453"
            if command[1:3]==["wallet","address"]:return "0x"+"11"*20
            if command[1]=="calldata":return "0x12345678"
            if command[1]=="gas-price":return "1000"
            if command[1]=="estimate":return "200000"
            if command[1]=="call":return "100" if command[2]==budget.ORACLE else "0x"
            if command[1]=="send":
                self.assertEqual(timeout,10)
                self.assertEqual(command[command.index("--nonce")+1],"7")
                self.assertEqual(command[command.index("--gas-price")+1],"2000")
                self.assertEqual(command[command.index("--priority-gas-price")+1],"2000")
                return '{"transactionHash":"fixture"}'
            self.fail("Unexpected cast command")
        with mock.patch.dict("os.environ",{"GAS_SPONSOR_BUDGET_TOKEN":"fixture-token-"*4}),mock.patch.object(budget.urllib.request,"build_opener",return_value=self.opener()):
            result=budget.send_cast(run,"cast","https://fixture.invalid","fixture-key",300000,"0x"+"22"*20,"register()",expected_nonce=7)
        self.assertEqual(json.loads(result)["transactionHash"],"fixture")
        self.assertEqual(sum(c[1]=="send" for c in commands),1)
        self.assertLess(next(i for i,c in enumerate(commands) if c[1]=="estimate"), next(i for i,c in enumerate(commands) if c[1]=="send"))

    def test_standalone_budget_failures_and_unknown_sends_never_retry(self):
        for failure in ("missing","exhausted","nonce","simulation","gas","unknown"):
            commands=[]
            def run(command, *, timeout):
                commands.append(command[1])
                if command[1]=="chain-id":return "8453"
                if command[1:3]==["wallet","address"]:return "0x"+"11"*20
                if command[1]=="call":
                    if failure=="simulation":raise RuntimeError("fixture-key")
                    return "0x"
                if command[1]=="estimate":return "400000" if failure=="gas" else "200000"
                if command[1]=="send":raise RuntimeError("fixture-key")
                self.fail("Unexpected cast command")
            reservation=mock.Mock(return_value=(2000,7))
            if failure=="exhausted":reservation.side_effect=budget.BudgetUnavailable("exhausted")
            with mock.patch.dict("os.environ",{"GAS_SPONSOR_BUDGET_TOKEN":"" if failure=="missing" else "fixture-token-"*4}),mock.patch.object(budget,"reserve",reservation):
                with self.assertRaises(budget.BudgetUnavailable) as error:
                    budget.send_cast(run,"cast","https://fixture.invalid","fixture-key",300000,"0x"+"22"*20,"register()",expected_nonce=8 if failure=="nonce" else 7)
            self.assertNotIn("fixture-key",str(error.exception))
            self.assertEqual(commands.count("send"),1 if failure=="unknown" else 0)
            if failure in ("missing","simulation","gas"):reservation.assert_not_called()

    def test_reviewed_raw_calldata_is_reserved_without_reencoding(self):
        client=self.client();opener=self.opener()
        with mock.patch.dict("os.environ",{"GAS_SPONSOR_BUDGET_TOKEN":"fixture-token-"*4}),mock.patch.object(budget.urllib.request,"build_opener",return_value=opener):
            budget.reserve(client,"fixture-key",100000,"0x"+"22"*20,None,(),calldata="0xaabbccdd")
        client.run.assert_not_called()
        scope={"chain_id":8453,"from":"0x"+"11"*20,"to":"0x"+"22"*20,"data":"0xaabbccdd","value":"0","gas_limit":100000,"max_fee_per_gas":"2000"}
        digest=budget.hashlib.sha256(json.dumps(scope,sort_keys=True,separators=(",",":")).encode()).hexdigest()
        self.assertEqual(json.loads(opener.open.call_args.args[0].data)["transaction_digest"],digest)

if __name__=="__main__":unittest.main()
