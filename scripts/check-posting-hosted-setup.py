#!/usr/bin/env python3
"""Two exact sponsored setup actions for the dedicated zero-ETH Sepolia solver.

Default read-only. --execute registers a synthetic participant attestation and
deploys an empty bounded wallet on the reviewed testnet factories. All authority
is limited to disposable test identities; signed envelopes stay outside Git.
"""
import argparse
import importlib.util
import json
import os
import stat
import time
import fcntl
from datetime import datetime, timezone
from pathlib import Path
from eth_account import Account
from eth_account.messages import encode_typed_data
from web3 import Web3

spec=importlib.util.spec_from_file_location("hosted",Path(__file__).with_name("check-posting-hosted-sepolia.py"))
h=importlib.util.module_from_spec(spec);spec.loader.exec_module(h)
ex=h.ex
OWNER=h.funding.ADDRESSES["solver"]


def contract(name):
    abi=json.loads((ex.ROOT/f"contracts/base-escrow/out/{name}.sol/{name}.json").read_text())["abi"]
    return Web3().eth.contract(abi=abi)


def plans(wallets):
    now=int(time.time())
    digest=lambda x:h.keccak256(("posting-hosted-setup-20260930/"+x).encode())
    policy={"delegate":h.funding.ADDRESSES["relayer"],"valid_after":now-60,"valid_until":now+7200,"period_seconds":3600,
            "max_per_action_micro_usdc":100_000,"max_per_period_micro_usdc":100_000,"max_lifetime_micro_usdc":100_000,
            "max_bounty_target_micro_usdc":100_000,"allowed_actions":8,"allowed_verification_modes":2,
            "deterministic_verifier_module":"0x"+"00"*20,"signed_quorum_verifier_set_hash":digest("verifiers"),"ai_judge_verifier_set_hash":"0x"+"00"*32}
    wallet={"network":"base-sepolia","owner":OWNER,"valid_before":now+1800,"action":{"kind":"wallet_deploy","policy":policy,"user_salt":digest("salt")}}
    message={"wallet":OWNER,"participantId":digest("synthetic-participant"),"sourceHash":digest("synthetic-fixture-not-real-identity"),"validUntil":now+86400,"nonce":0}
    domain={"name":"Agent Bounties Participant Registry","version":"1","chainId":84532,"verifyingContract":h.SETUP_RELEASE["participant_registry"]}
    fields=[{"name":n,"type":t} for n,t in (("wallet","address"),("participantId","bytes32"),("sourceHash","bytes32"),("validUntil","uint64"),("nonce","uint256"))]
    typed={"types":{"ParticipantAttestation":fields},"primaryType":"ParticipantAttestation","domain":domain,"message":message}
    signature="0x"+Account.from_key(wallets["deployer"]["private_key"]).sign_message(encode_typed_data(full_message=typed)).signature.hex()
    registry={"network":"base-sepolia","owner":OWNER,"valid_before":now+1800,"action":{"kind":"participant_register","participant_id":message["participantId"],"source_hash":message["sourceHash"],"valid_until":message["validUntil"],"registry_nonce":0,"attester_signature":signature}}
    return [{"name":"wallet-deploy","request":wallet},{"name":"participant-register","request":registry}]


def exact_call(req):
    a=req["action"]
    if a["kind"]=="wallet_deploy":
        fields=("delegate","valid_after","valid_until","period_seconds","max_per_action_micro_usdc","max_per_period_micro_usdc",
                "max_lifetime_micro_usdc","max_bounty_target_micro_usdc","allowed_actions","allowed_verification_modes",
                "deterministic_verifier_module","signed_quorum_verifier_set_hash","ai_judge_verifier_set_hash")
        p=[a["policy"][key] for key in fields]
        p[0]=Web3.to_checksum_address(p[0]);p[10]=Web3.to_checksum_address(p[10])
        data=contract("BoundedAgentWalletV2Factory").functions.createWallet(Web3.to_checksum_address(OWNER),tuple(p),a["user_salt"])._encode_transaction_data()
        return h.SETUP_RELEASE["wallet_factory"],data
    data=contract("ParticipantEligibilityRegistry").functions.register(Web3.to_checksum_address(OWNER),a["participant_id"],a["source_hash"],a["valid_until"],a["attester_signature"])._encode_transaction_data()
    return h.SETUP_RELEASE["participant_registry"],data


def signed(req,prepared,wallets):
    ex.require(prepared["request"]==req and prepared["release"]==h.SETUP_RELEASE,"Setup request or release changed")
    target,data=exact_call(req)
    tx=prepared["transaction"]
    ex.require(tx["to"]==target and tx["data"]==data and tx["value_wei"]==0,"Setup calldata differs from independent ABI encoding")
    release_hash=h.keccak256(json.dumps(h.SETUP_RELEASE,separators=(",",":")).encode())
    message={"owner":OWNER,"target":target,"callHash":h.keccak256(bytes.fromhex(data[2:])),"releaseHash":release_hash,"validBefore":req["valid_before"]}
    requirement={"signer":OWNER,"typed_data":prepared["typed_data"],"digest":prepared["digest"]}
    signature=h.sign_requirement(requirement,wallets,OWNER,target,"SponsoredSetup",message,"Agent Bounties Sponsored Setup","1")
    return {"request":req,"owner_signature":signature}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--execute",action="store_true");args=parser.parse_args()
    rpc=ex.Rpc();ex.load_bundle();ex.fresh_head(rpc)
    caps=h.get("/v1/base/sponsored-setup/capabilities")
    ex.require(next(n for n in caps["networks"] if n["network"]=="base-sepolia")["release"]==h.SETUP_RELEASE,"Setup release differs")
    ex.require(not next(n for n in caps["networks"] if n["network"]=="base-mainnet")["available"],"Unexpected mainnet setup")
    initial=h.balances(rpc)
    if not args.execute:
        print(json.dumps({"capabilities":caps,"customer_eth_wei":"0","execute":False}));return
    h.STATE.mkdir(mode=0o700,exist_ok=True)
    wallet_path=ex.PRIVATE/"wallets.json";ex.require(stat.S_IMODE(wallet_path.stat().st_mode)==0o600,"Wallet permissions")
    wallets=json.loads(wallet_path.read_text())
    with os.fdopen(os.open(h.STATE/"lock",os.O_CREAT|os.O_RDWR,0o600),"w") as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        path=h.STATE/"setup-journal.json"
        journal=json.loads(path.read_text()) if path.exists() else {"release":h.SETUP_RELEASE,"plans":plans(wallets),"records":{},"initial_balances":initial}
        ex.require(journal["release"]==h.SETUP_RELEASE,"Saved setup release changed")
        ex.save(path,journal)
        for plan in journal["plans"]:
            req=plan["request"];rec=journal["records"].setdefault(plan["name"],{})
            if "signed_body" not in rec:
                code,out,_=h.request("/v1/base/sponsored-setup/prepare",req)
                ex.require(code==200,"Setup prepare HTTP "+str(code)+" "+str(out))
                rec["signed_body"]=signed(req,out["prepared"],wallets);ex.save(path,journal)
            code,out,_=h.request("/v1/base/sponsored-setup/status",req)
            if code!=200 or not out.get("setup_complete"):
                rec["attempted"]=True;ex.save(path,journal)
                code,out,_=h.request("/v1/base/sponsored-setup/relay",rec["signed_body"])
                ex.require(code in (200,202),"Setup relay HTTP "+str(code)+" "+str(out))
            for _ in range(15):
                if code==200 and out.get("setup_complete") and out.get("operation",{}).get("status")=="confirmed":break
                time.sleep(3);code,out,_=h.request("/v1/base/sponsored-setup/status",req)
            ex.require(code==200 and out.get("setup_complete") and out["operation"]["status"]=="confirmed","Setup lacks confirmed receipt")
            rec["receipt"]=h.receipt(rpc,out);rec["outcome"]=out;ex.save(path,journal)
            before=h.get("/v1/base/gas-sponsorship")["daily_budget"]
            code,repeated,_=h.request("/v1/base/sponsored-setup/relay",rec["signed_body"])
            after=h.get("/v1/base/gas-sponsorship")["daily_budget"]
            ex.require(code==200 and repeated["operation"]["transaction_hash"]==out["operation"]["transaction_hash"] and before["reserved_micro_usdc"]==after["reserved_micro_usdc"],"Setup replay spent again")
            rec["replay_verified"]=True;ex.save(path,journal)
            print(json.dumps({"action":plan["name"],"receipt":rec["receipt"],"replay_verified":True}),flush=True)
        evidence={"network":"base-sepolia","observed_at":datetime.now(timezone.utc).isoformat(),"release":h.SETUP_RELEASE,
                  "initial_balances":journal["initial_balances"],"final_balances":h.balances(rpc),"final_budget":h.get("/v1/base/gas-sponsorship")["daily_budget"],
                  "actions":[{"name":p["name"],"receipt":journal["records"][p["name"]]["receipt"],"replay_verified":True} for p in journal["plans"]],
                  "empty_wallet":journal["records"]["wallet-deploy"]["outcome"]["predicted_wallet"],"synthetic_participant_only":True,"production_changed":False}
        (ex.ROOT/"docs/evidence/posting-hosted-setup-20260930.json").write_text(json.dumps(evidence,indent=2)+"\n")


if __name__=="__main__":main()
