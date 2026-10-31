#!/usr/bin/env python3
"""Fixed native-USDC sponsored legacy lifecycle in the isolated Sepolia canary.

Only --execute signs or relays. The disposable solver identity creates a 2.1
test-USDC bounty; the disposable creator identity solves it. Exact envelopes
are journaled outside Git before submission, with canonical receipts and replay
checks. No native ETH is sent to either customer identity.
"""
import argparse
import hashlib
import re
import terms_publication_client
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import stat
import time
from datetime import datetime, timezone
from eth_account import Account
from eth_account.messages import encode_typed_data, encode_defunct

spec=importlib.util.spec_from_file_location("hosted",Path(__file__).with_name("check-posting-hosted-sepolia.py"))
h=importlib.util.module_from_spec(spec);spec.loader.exec_module(h)
ex=h.ex
CREATOR=h.funding.ADDRESSES["solver"]
SOLVER=h.funding.ADDRESSES["creator"]


def sign(key, primary, fields, message, contract, token=False):
    typed={"domain":{"name":"USDC" if token else "Agent Bounties","version":"2" if token else "1","chainId":84532,"verifyingContract":contract},
           "types":{primary:[{"name":n,"type":t} for n,t in fields]},"primaryType":primary,"message":message}
    return "0x"+Account.from_key(key).sign_message(encode_typed_data(full_message=typed)).signature.hex()


def transfer_signature(key, td, sender, to, amount):
    domain=td["domain"];m=td["message"]
    ex.require(h.number(domain.get("chainId",domain.get("chain_id")))==84532 and domain.get("verifyingContract",domain.get("verifying_contract")).lower()==h.funding.USDC
               and domain["name"]=="USDC" and domain["version"]=="2" and td.get("primaryType")=="TransferWithAuthorization","Wrong USDC signature domain")
    ex.require(m["from"].lower()==sender and m["to"].lower()==to and h.number(m["value"])==amount,"Wrong native-USDC intent")
    normalized={"from":sender,"to":to,"value":amount,"validAfter":h.number(m.get("validAfter",m.get("valid_after"))),"validBefore":h.number(m.get("validBefore",m.get("valid_before"))),"nonce":m["nonce"]}
    ex.require(normalized["validAfter"]<=int(time.time()) and int(time.time())<normalized["validBefore"]<=int(time.time())+7200,"Wrong authorization lifetime")
    return sign(key,"TransferWithAuthorization",[("from","address"),("to","address"),("value","uint256"),("validAfter","uint256"),("validBefore","uint256"),("nonce","bytes32")],normalized,h.funding.USDC,True)


def new_document():
    now=int(time.time());d=json.loads((ex.ROOT/"bounties/autonomous-v1/244.json").read_text());c=d["contract_terms"]
    c.update(creator_wallet=CREATOR,network="base-sepolia",settlement_token=h.funding.USDC,funding_deadline=now+86400,creation_nonce=h.keccak256(b"posting-hosted-legacy-20260930"))
    for k,n in (("solver_reward",2_000_000),("verifier_reward",100_000),("claim_bond",100_000),("initial_funding",2_100_000)):c[k]["amount"]=n
    d.update(title="Synthetic sponsored legacy lifecycle",goal="Check native test USDC and zero customer ETH in the isolated hosted canary.",acceptance_criteria=["Canonical native test USDC payout without customer ETH"],
             benchmark={"engine":"creator_review_v1","reviewer":"creator","acceptance":"all_published_criteria","delivery_deadline":now+86400},
             verification_policy={"engine":"creator_review_v1","mechanism":"signed_quorum","threshold":1,"verifiers":[CREATOR],"public_disclosure":"The synthetic test creator signs the verdict."},
             evidence_schema=h.SCHEMA,source_url="https://example.org/synthetic-posting-rehearsal",discovery_source="isolated synthetic testnet rehearsal")
    return d


def exchange_login(key, address=SOLVER):
    c,o,_=h.request("/v1/auth/wallet/challenge",{"address":address,"chain_id":84532})
    ex.require(c==200,"Wallet login challenge unavailable: "+str(c))
    ex.require(address.lower() in o["message"].lower() and "Chain ID: 84532" in o["message"],"Unexpected SIWE identity")
    signature="0x"+Account.from_key(key).sign_message(encode_defunct(text=o["message"])).signature.hex()
    c,session,_=h.request("/v1/auth/wallet/session",{"challenge_id":o["challenge_id"],"signature":signature,"chain_id":84532})
    ex.require(c==200,"Wallet login failed: "+str(c))
    return {"Authorization":"Bearer "+session["token"]}


def publication_headers(creator, document, login_headers):
    token = login_headers.get("Authorization", "")
    ex.require(re.fullmatch(r"Bearer abws_[0-9a-f]{64}", token) is not None, "A verified creator session is required before fixture publication")
    c, session, _ = h.request("/v1/auth/session", headers=login_headers)
    principal = session.get("principal", {}) if isinstance(session, dict) else {}
    ex.require(c == 200 and principal.get("kind") == "wallet" and principal.get("chain_id") == 84532
               and str(principal.get("address", "")).lower() == creator.lower(), "Fixture session must prove the exact creator on Base Sepolia")
    ct = document.get("contract_terms", {})
    ex.require(str(ct.get("creator_wallet", "")).lower() == creator.lower() and ct.get("network") in ("BaseSepolia", "base-sepolia"), "Fixture publication scope differs")
    body = {"creator_wallet": creator, "document": document}
    key = "hosted-canary-terms-v1-" + hashlib.sha256(h.canonical(body)).hexdigest()
    return {"Authorization": token, "Idempotency-Key": key}


def send(name,path,body,journal,state_path,rpc,headers=None,complete=None):
    rec=journal["actions"].setdefault(name,{"path":path,"body":body})
    ex.require(rec["path"]==path and rec["body"]==body,"Saved legacy envelope changed")
    ex.save(state_path,journal)
    for attempt in range(20):
        c,out,_=h.request(path,body,headers)
        rec["last_http_status"]=c
        ex.save(state_path,journal)
        ex.require(c in (200,202),name+" HTTP "+str(c)+" "+str(out))
        if complete(out):break
        time.sleep(3)
    ex.require(complete(out),"Legacy outcome still pending: "+name)
    tx=out.get("transaction_hash") or out.get("operation",{}).get("transaction_hash") or out.get("relay",{}).get("transaction")
    if not tx:
        tx=out.get("candidate",{}).get("claim_transaction_hash")
    if not tx:
        # Claim candidates expose their relay through the returned relay object.
        tx=out.get("candidate",{}).get("relay",{}).get("transaction")
    ex.require(isinstance(tx,str) and len(tx)==66,"Missing canonical transaction for "+name+": "+str(out))
    rec["receipt"]=h.receipt(rpc,{"operation":{"transaction_hash":tx}});rec["outcome"]=out;ex.save(state_path,journal)
    before=h.get("/v1/base/gas-sponsorship")["daily_budget"]
    c,repeated,_=h.request(path,body,headers)
    after=h.get("/v1/base/gas-sponsorship")["daily_budget"]
    ex.require(c==200 and complete(repeated) and before["reserved_micro_usdc"]==after["reserved_micro_usdc"],"Legacy replay changed budget")
    rec["replay_verified"]=True;ex.save(state_path,journal)
    print(json.dumps({"action":name,"receipt":rec["receipt"],"replay_verified":True}),flush=True)
    return out


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--execute",action="store_true");args=parser.parse_args()
    rpc=ex.Rpc();ex.load_bundle();ex.fresh_head(rpc);initial=h.balances(rpc)
    if not args.execute:print(json.dumps({"network":"base-sepolia","balances":initial,"execute":False}));return
    wallet_path=ex.PRIVATE/"wallets.json";ex.require(stat.S_IMODE(wallet_path.stat().st_mode)==0o600,"Test wallet permissions")
    wallets=json.loads(wallet_path.read_text());creator_key=wallets["solver"]["private_key"];solver_key=wallets["creator"]["private_key"]
    with os.fdopen(os.open(h.STATE/"lock",os.O_CREAT|os.O_RDWR,0o600),"w") as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        state_path=h.STATE/"legacy-journal.json"
        j=json.loads(state_path.read_text()) if state_path.exists() else {"document":new_document(),"valid_before":int(time.time())+1800,"actions":{},"initial_balances":initial}
        ex.save(state_path,j)
        creator_headers = publication_headers(CREATOR, j["document"], exchange_login(creator_key, CREATOR))
        if "terms" not in j:
            c,terms,_=h.request("/v1/base/autonomous-bounties/terms",{"creator_wallet":CREATOR,"document":j["document"]}, creator_headers)
            ex.require(c==200,"Terms publication HTTP "+str(c)+" "+str(terms));j["terms"]=terms;ex.save(state_path,j)
        terms_publication_client.validate_record(j["terms"], CREATOR, j["document"])
        terms=j["terms"];d=j["document"];ct=d["contract_terms"]
        create={"creator":CREATOR,**{k:ct[k] for k in ("solver_reward","verifier_reward","funding_deadline","claim_window_seconds","verification_window_seconds","initial_funding","creation_nonce")},
                **{k:terms[k] for k in ("terms_hash","policy_hash","acceptance_criteria_hash","benchmark_hash","evidence_schema_hash")},
                "verification_mode":"signed_quorum","verifier_module":None,"verifier_reward_recipient":None,"verifiers":[CREATOR],"threshold":1}
        if "creation" not in j["actions"]:
            c,plan,_=h.request("/v1/base/autonomous-bounties/creation-plan",{"network":"base-sepolia","authorization_valid_before":j["valid_before"],"create":create})
            ex.require(c==200,"Legacy plan HTTP "+str(c)+" "+str(plan));j["plan"]=plan
            sig=transfer_signature(creator_key,plan["eip3009_authorization"],CREATOR,plan["predicted_bounty_contract"].lower(),2_100_000)
            body={"network":"base-sepolia","authorization_valid_before":j["valid_before"],"create":create,"signature":{"v":int(sig[-2:],16),"r":"0x"+sig[2:66],"s":"0x"+sig[66:130]}}
        else:body=j["actions"]["creation"]["body"]
        send("creation","/v1/base/autonomous-bounties/creation-relay",body,j,state_path,rpc,complete=lambda o:o.get("funding_confirmed") is True)
        plan=j["plan"];bounty=plan["predicted_bounty_contract"].lower();login=exchange_login(solver_key)
        claim_path="/v1/base/autonomous-bounties/claims"
        if "claim" not in j["actions"]:
            body={"idempotency_key":"hosted-sepolia:"+bounty+":1","network":"base-sepolia","bounty_contract":bounty,"solver_wallet":SOLVER,"request_bond_sponsorship":False}
            c,prepared,_=h.request(claim_path,body,login);ex.require(c==200,"Claim prepare HTTP "+str(c)+" "+str(prepared))
            body["wallet_signature"]=transfer_signature(solver_key,prepared["signing_payload"],SOLVER,bounty,100_000)
        else:body=j["actions"]["claim"]["body"]
        send("claim",claim_path,body,j,state_path,rpc,login,complete=lambda o:o.get("candidate",{}).get("status")=="claimed")
        submission={"bounty_contract":bounty,"bounty_id":plan["bounty_id"],"solver":SOLVER,"round":1,"submission_hash":h.keccak256(b"synthetic-legacy-submission"),"evidence_hash":h.keccak256(b"synthetic-legacy-evidence"),"policy_hash":terms["policy_hash"],"deadline":j["valid_before"]}
        message={"bounty":bounty,"bountyId":plan["bounty_id"],"solver":SOLVER,"round":1,"submissionHash":submission["submission_hash"],"evidenceHash":submission["evidence_hash"],"policyHash":terms["policy_hash"],"deadline":j["valid_before"]}
        fields=[("bounty","address"),("bountyId","bytes32"),("solver","address"),("round","uint64"),("submissionHash","bytes32"),("evidenceHash","bytes32"),("policyHash","bytes32"),("deadline","uint256")]
        body={"network":"base-sepolia","submission":submission,"signature":sign(solver_key,"Submit",fields,message,bounty)}
        send("submission","/v1/base/autonomous-bounties/submission-relay",body,j,state_path,rpc,complete=lambda o:o.get("confirmed") is True)
        j.setdefault("verdict_deadline",int(time.time())+540);ex.save(state_path,j)
        attestation={k:v for k,v in submission.items() if k!="solver"};attestation.update(verifier=CREATOR,passed=True,response_hash=h.keccak256(b"synthetic-legacy-passed"),deadline=j["verdict_deadline"])
        message={k:v for k,v in message.items() if k!="solver"};message.update(verifier=CREATOR,passed=True,responseHash=attestation["response_hash"],deadline=j["verdict_deadline"])
        fields=[("bounty","address"),("bountyId","bytes32"),("round","uint64"),("verifier","address"),("submissionHash","bytes32"),("evidenceHash","bytes32"),("policyHash","bytes32"),("passed","bool"),("responseHash","bytes32"),("deadline","uint256")]
        body={"network":"base-sepolia","attestation":attestation,"signature":sign(creator_key,"VerificationAttestation",fields,message,bounty)}
        send("verdict","/v1/base/autonomous-bounties/creator-verdict-relay",body,j,state_path,rpc,complete=lambda o:o.get("solver_paid") is True)
        nonce=h.keccak256(b"posting-hosted-unused-cancel-20260930");message={"authorizer":CREATOR,"nonce":nonce}
        body={"network":"base-sepolia",**message,"signature":sign(creator_key,"CancelAuthorization",[("authorizer","address"),("nonce","bytes32")],message,h.funding.USDC,True)}
        send("revocation","/v1/base/usdc/authorization-revocation-relay",body,j,state_path,rpc,complete=lambda o:o.get("authorization_cancelled") is True)
        evidence={"network":"base-sepolia","observed_at":datetime.now(timezone.utc).isoformat(),"initial_balances":j["initial_balances"],"final_balances":h.balances(rpc),"final_budget":h.get("/v1/base/gas-sponsorship")["daily_budget"],
                  "actions":[{"name":k,"receipt":v["receipt"],"replay_verified":v["replay_verified"]} for k,v in j["actions"].items()],"production_changed":False}
        (ex.ROOT/"docs/evidence/posting-hosted-legacy-20260930.json").write_text(json.dumps(evidence,indent=2)+"\n")


if __name__=="__main__":main()
