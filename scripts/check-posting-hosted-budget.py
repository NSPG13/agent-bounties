#!/usr/bin/env python3
"""Exhaust only the isolated canary's daily gas ledger without broadcasting.

Requires --reserve-test-capacity and this task's private budget-only credential.
The API never signs or sends through the reservation endpoint. The consumed
test allowance is deliberately retained until the normal UTC reset, no rollover.
"""
import argparse
from datetime import datetime, timezone
from decimal import Decimal, ROUND_CEILING
import importlib.util
import json
from pathlib import Path
import stat
import time
import urllib.request

spec=importlib.util.spec_from_file_location("hosted",Path(__file__).with_name("check-posting-hosted-sepolia.py"))
h=importlib.util.module_from_spec(spec);spec.loader.exec_module(h)
ex=h.ex


def price():
    req=urllib.request.Request("https://api.coinbase.com/v2/prices/ETH-USDC/spot",headers={"Cache-Control":"no-cache","User-Agent":"agent-bounties-canary-budget/1"})
    with urllib.request.build_opener(ex.NoRedirect()).open(req,timeout=10) as response:
        ex.require(int(response.headers.get("age","0"))<=60,"Stale test quote")
        data=json.load(response)["data"]
    ex.require(data["base"]=="ETH" and data["currency"]=="USDC","Unexpected quote currency")
    return int((Decimal(data["amount"])*1_000_000).to_integral_value(rounding=ROUND_CEILING))


def nonce(rpc):
    return rpc("eth_getTransactionCount",[h.funding.ADDRESSES["relayer"],"pending"])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--reserve-test-capacity",action="store_true");args=parser.parse_args()
    rpc=ex.Rpc();ex.fresh_head(rpc)
    before=h.get("/v1/base/gas-sponsorship")["daily_budget"]
    ex.require(before["limit_micro_usdc"]==1_000_000 and before["carries_forward"] is False,"Wrong daily budget")
    if not args.reserve_test_capacity:print(json.dumps(before));return
    token=h.STATE/"budget-token";ex.require(stat.S_IMODE(token.stat().st_mode)==0o600,"Budget credential permissions")
    headers={"Authorization":"Bearer "+token.read_text()}
    path=h.STATE/"budget-journal.json"
    journal=json.loads(path.read_text()) if path.exists() else {"api":h.API,"initial_budget":before,"initial_nonce":nonce(rpc),"reservations":[]}
    ex.require(journal["api"]==h.API and journal["initial_budget"]["resets_at"]==before["resets_at"],"Test window changed; inspect original evidence")
    # The latest reservation retains a 45-second sender lease. Let it expire;
    # waiting never releases or rewrites the database's lease/nonce fences.
    for _ in range(8):
        if journal["reservations"]:
            until=journal["reservations"][-1].get("lease_until",0)
            if time.time()<until:time.sleep(min(47,until-time.time()+1))
        budget=h.get("/v1/base/gas-sponsorship")["daily_budget"]
        left=budget["remaining_micro_usdc"]
        if left==0:break
        quote=price()
        # Leave small room for independent API FX rounding, then reserve the
        # final one-micro unit with the endpoint's minimum nonzero test ticket.
        target=max(1,left-max(1,left//1000))
        maximum=1 if left<=2 else max(1,min(100_000_000_000_000,target*4_000_000_000_000_000_000//(quote*5)))
        intent={"network":"base-sepolia","relayer_address":h.funding.ADDRESSES["relayer"],"transaction_digest":h.keccak256(("canary-budget-only-"+str(len(journal["reservations"]))).encode())[2:],"maximum_fee_wei":str(maximum)}
        ex.save(path,journal)
        c,out,_=h.request("/v1/operator/gas-sponsorship/reservations",intent,headers)
        ex.require(c==200 and out["reserved"] and out["transaction_submitted"] is False,"Reservation failed; do not retry unknown capacity consumption")
        after=h.get("/v1/base/gas-sponsorship")["daily_budget"]
        ex.require(after["reserved_micro_usdc"]<=1_000_000 and after["reserved_micro_usdc"]>budget["reserved_micro_usdc"],"Invalid budget increment")
        journal["reservations"].append({"maximum_fee_wei":str(maximum),"budget_after":after,"lease_until":out["signer_lease_expires_at"],"transaction_submitted":False})
        ex.save(path,journal)
        print(json.dumps({"remaining_test_allowance_micro_usdc":after["remaining_micro_usdc"],"transaction_submitted":False}),flush=True)
    final=h.get("/v1/base/gas-sponsorship")["daily_budget"]
    ex.require(final["remaining_micro_usdc"]==0 and final["reserved_micro_usdc"]==1_000_000,"Test allowance not yet exhausted")
    intent={"network":"base-sepolia","relayer_address":h.funding.ADDRESSES["relayer"],"transaction_digest":"aa"*32,"maximum_fee_wei":"1"}
    c,_,_=h.request("/v1/operator/gas-sponsorship/reservations",intent,headers)
    ex.require(c==503,"Exhausted budget must reject another reservation")
    ex.require(nonce(rpc)==journal["initial_nonce"],"Budget-only test unexpectedly changed sender nonce")
    caps=h.get("/v1/base/creator-open/capabilities")
    ex.require(all(not n["available"] for n in caps["networks"]),"Exhausted creator-open capacity should be unavailable")
    evidence={"observed_at":datetime.now(timezone.utc).isoformat(),"api":h.API,"initial_budget":journal["initial_budget"],"final_budget":final,
              "reservations":journal["reservations"],"exhausted_reservation_http":c,"sender_nonce_unchanged":True,
              "customer_balances":h.balances(rpc),"real_eth_spent_by_this_test":"0","production_changed":False}
    (ex.ROOT/"docs/evidence/posting-hosted-budget-20260930.json").write_text(json.dumps(evidence,indent=2)+"\n")


if __name__=="__main__":main()
