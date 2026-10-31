#!/usr/bin/env python3
"""Bounded rehearsal against the isolated Sepolia canary; never mainnet.

Read-only by default. --execute signs only the fixed synthetic requests below,
using this task's dedicated test identities. Exact signed requests are persisted
privately before relay; resuming uses those same requests. No production secrets.
"""
import argparse
import copy
from datetime import datetime, timezone
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import stat
import time
import urllib.error
import urllib.request

from eth_account import Account
from eth_account.messages import encode_typed_data
from eth_abi import encode
from _shared.evm import keccak256, keccak_bytes, address_word

spec = importlib.util.spec_from_file_location("funding", Path(__file__).with_name("fund-posting-sepolia-rehearsal.py"))
funding = importlib.util.module_from_spec(spec)
spec.loader.exec_module(funding)
ex = funding.executor
API = "https://agent-bounties-posting-api-canary.onrender.com"
REVISION = "abacc3b4cbe2db960d969c85b21ec13bdbcdd3bb"
STATE = ex.PRIVATE / "hosted"
OUTPUT = ex.ROOT / "docs/evidence/posting-hosted-sepolia-20260930.json"
OPEN_RELEASE = json.loads((ex.ROOT / "docs/evidence/posting-creator-open-sepolia-release-20260930.json").read_text())
SETUP_RELEASE = json.loads((ex.ROOT / "docs/evidence/posting-setup-sepolia-release-20260930.json").read_text())
DISCLOSURE = "The creator judges the published criteria. The first passing creator verdict pays one solver. Rejected entries pay their bond to the creator; unreviewed entries recover only their bond after their review deadline or another winner. There is no exclusive claim or guarantee of payment for work."
SCHEMA = {"type":"object","required":["artifact_url","artifact_sha256"],"properties":{
    "artifact_url":{"type":"string","pattern":"^https://"},
    "artifact_sha256":{"type":"string","pattern":"^sha256:[0-9a-f]{64}$"}}}


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def hash_json(value):
    return keccak256(canonical(value))


def number(value):
    return int(value, 16 if value.startswith("0x") else 10) if isinstance(value, str) else value


def request(path, body=None, headers=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(API + path, data=data, headers={"Content-Type":"application/json", "User-Agent":"agent-bounties-hosted-rehearsal/1", **(headers or {})})
    try:
        response = urllib.request.build_opener(ex.NoRedirect()).open(req, timeout=55)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read(2_000_001)
        ex.require(len(raw) <= 2_000_000, "Oversized hosted response")
        try:
            value = json.loads(raw)
        except ValueError:
            value = raw.decode(errors="replace")
        return response.status, value, response.headers


def get(path):
    code, body, _ = request(path)
    ex.require(code == 200, "Read failed: " + path + " HTTP " + str(code))
    return body


def balances(rpc):
    result = {}
    for role, address in funding.ADDRESSES.items():
        call = "0x" + (keccak_bytes(b"balanceOf(address)")[:4] + address_word(address)).hex()
        result[role] = {"address":address, "eth_wei":str(int(rpc("eth_getBalance", [address, "latest"]),16)),
                        "test_usdc_micro":str(int(rpc("eth_call", [{"to":funding.USDC,"data":call},"latest"]),16))}
    ex.require(all(result[r]["eth_wei"] == "0" for r in ("creator", "solver")), "Customer ETH must remain zero")
    return result


def predict(req):
    t = req["original_terms"]
    config = (bytes.fromhex(t["bounty_id"][2:]), t["creator"], t["solver_reward_micro_usdc"], t["review_reward_micro_usdc"],
              bytes.fromhex(hash_json(t)[2:]), bytes.fromhex(hash_json(t["acceptance_criteria"])[2:]),
              bytes.fromhex(hash_json(SCHEMA)[2:]), t["submission_deadline"], t["review_window_seconds"], t["max_pending_entries"])
    salt = keccak_bytes(encode(["(bytes32,address,uint128,uint128,bytes32,bytes32,bytes32,uint64,uint64,uint16)","bytes32"],
                              [config, bytes.fromhex(req["user_salt"][2:])]))
    code = bytes.fromhex("3d602d80600a3d3981f3363d3d373d3d3d363d73" + OPEN_RELEASE["implementation"][2:] + "5af43d82803e903d91602b57fd5bf3")
    return "0x" + keccak_bytes(b"\xff" + bytes.fromhex(OPEN_RELEASE["factory"][2:]) + salt + keccak_bytes(code))[-20:].hex()


def sign_requirement(requirement, wallets, expected_signer, contract, primary, expected_message, domain_name, version):
    td = requirement["typed_data"]
    domain = td["domain"]
    ex.require(requirement["signer"].lower() == expected_signer.lower(), "Unexpected hosted signer")
    ex.require(number(domain["chainId"]) == 84532 and domain["verifyingContract"].lower() == contract.lower()
               and domain["name"] == domain_name and domain["version"] == version and td["primaryType"] == primary,
               "Unexpected hosted signature domain")
    ex.require(set(td["message"]) == set(expected_message), "Unexpected signed message fields")
    for key, expected in expected_message.items():
        actual = td["message"][key]
        ex.require((number(actual) == expected) if isinstance(expected, int) else str(actual).lower() == str(expected).lower(),
                   "Hosted signing intent differs: " + key)
    message = encode_typed_data(full_message=td)
    ex.require(keccak256(b"\x19" + message.version + message.header + message.body) == requirement["digest"], "Typed digest mismatch")
    signer = next((Account.from_key(w["private_key"]) for w in wallets.values() if w["address"].lower() == expected_signer.lower()), None)
    ex.require(signer is not None, "No dedicated test signer")
    return "0x" + signer.sign_message(message).signature.hex()


def open_body(req, prepared, wallets):
    ex.require(prepared["request"] == req and prepared["bounty_contract"] == predict(req), "Prepared request or predicted address differs")
    ex.require(prepared["customer_gas_wei"] == "0", "Customer gas requested")
    t, a = req["original_terms"], req["action"]
    kind, bounty = a["kind"], prepared["bounty_contract"]
    signer = a["solver"] if kind == "submit" else t["creator"]
    principal = {"create": t["solver_reward_micro_usdc"] + t["review_reward_micro_usdc"],
                 "submit": t["review_reward_micro_usdc"],
                 "increase": a.get("new_reward_micro_usdc", req["expected_solver_reward_micro_usdc"]) - req["expected_solver_reward_micro_usdc"]}.get(kind, 0)
    ex.require(0 <= principal <= 3_000_000 and number(prepared["customer_principal_micro_usdc"]) == principal, "Test principal exceeded")
    sigs = {"token":"0x", "action":"0x"}
    if principal:
        sigs["token"] = sign_requirement(prepared["token_authorization"], wallets, signer, funding.USDC,
            "ReceiveWithAuthorization", {"from":signer,"to":bounty,"value":principal,"validAfter":0,"validBefore":req["valid_before"],"nonce":a["token_nonce"]}, "USDC", "2")
    else:
        ex.require(prepared["token_authorization"] is None, "Unexpected token authorization")
    current = {**t,"solver_reward_micro_usdc":req["expected_solver_reward_micro_usdc"]}
    message = {"revision":req["expected_revision"], "termsHash":hash_json(current), "deadline":req["valid_before"]}
    primary = None
    if kind == "increase":
        primary = "IncreaseReward"
        message.update(newTermsHash=hash_json({**current,"solver_reward_micro_usdc":a["new_reward_micro_usdc"]}),newReward=a["new_reward_micro_usdc"],tokenNonce=a["token_nonce"])
    elif kind == "submit":
        primary = "OpenSubmission"
        message.update(solver=a["solver"],submissionHash=a["submission_hash"],evidenceHash=a["evidence_hash"],tokenNonce=a["token_nonce"])
    elif kind == "review":
        primary = "CreatorReview"
        message.update(entry=a["entry"],solver=a["solver"],submissionHash=a["submission_hash"],evidenceHash=a["evidence_hash"],passed=a["passed"],responseHash=a["response_hash"])
    elif kind == "cancel":
        primary = "CancelBounty"
    if primary:
        sigs["action"] = sign_requirement(prepared["action_authorization"], wallets, signer, bounty, primary, message, "Agent Bounties Creator Open", "1")
    else:
        ex.require(prepared["action_authorization"] is None, "Unexpected action authorization")
    return {"request":req,"approval_hash":prepared["approval_hash"],"signatures":sigs}


def new_plan():
    now = int(time.time())
    h = lambda label: keccak256(("posting-hosted-sepolia-20260930/" + label).encode())
    terms = {"schema":"agent-bounties/creator-open-v1","network":"base-sepolia","bounty_id":h("bounty"),
             "creator":funding.ADDRESSES["creator"],"title":"Synthetic zero-ETH hosted lifecycle test", "goal":"Verify sponsored testnet settlement using dedicated test identities only.",
             "acceptance_criteria":["Canonical test USDC payout and no customer ETH"],"solver_reward_micro_usdc":2_000_000,
             "review_reward_micro_usdc":100_000,"submission_deadline":now+86400,"review_window_seconds":3600,"max_pending_entries":4,"disclosure":DISCLOSURE}
    req = {"original_terms":terms,"user_salt":h("salt"),"expected_revision":1,"expected_solver_reward_micro_usdc":2_000_000,
           "valid_before":now+1800,"action":{"kind":"create","token_nonce":h("create")}}
    plans = [{"name":"create","request":copy.deepcopy(req),"event":"CreatorOpenBountyCreated"}]
    req["action"]={"kind":"increase","new_reward_micro_usdc":2_500_000,"token_nonce":h("increase")}
    plans.append({"name":"increase","request":copy.deepcopy(req),"event":"OpenRewardIncreased"})
    req.update(expected_revision=2,expected_solver_reward_micro_usdc=2_500_000)
    for entry, passed in ((1,False),(2,True)):
        req["action"]={"kind":"submit","solver":funding.ADDRESSES["solver"],"submission_hash":h("submission"+str(entry)),"evidence_hash":h("evidence"+str(entry)),"token_nonce":h("bond"+str(entry))}
        plans.append({"name":"submit-"+str(entry),"request":copy.deepcopy(req),"event":"OpenSubmissionAdded"})
        req["action"]={k:v for k,v in req["action"].items() if k!="token_nonce"}
        req["action"].update(kind="review",entry=entry,passed=passed,response_hash=h("review"+str(entry)))
        plans.append({"name":"review-"+str(entry),"request":copy.deepcopy(req),"event":"OpenBountySettled" if passed else "OpenSubmissionRejected"})
    req = copy.deepcopy(plans[0]["request"])
    req["original_terms"].update(bounty_id=h("cancellation-bounty"),solver_reward_micro_usdc=1_000_000,title="Synthetic cancellation and refund test")
    req.update(user_salt=h("cancel-salt"),expected_solver_reward_micro_usdc=1_000_000,action={"kind":"create","token_nonce":h("cancel-create")})
    plans.append({"name":"cancel-create","request":copy.deepcopy(req),"event":"CreatorOpenBountyCreated"})
    req["action"]={"kind":"cancel"}
    plans.append({"name":"cancel","request":copy.deepcopy(req),"event":"OpenBountyCancelled"})
    req["action"]={"kind":"refund_principal"}
    plans.append({"name":"refund-principal","request":copy.deepcopy(req),"event":"OpenPrincipalRefunded"})
    return plans


def receipt(rpc, out):
    tx = out["operation"]["transaction_hash"]
    for attempt in range(6):
        r = rpc("eth_getTransactionReceipt", [tx])
        ex.require(r and r["status"] == "0x1" and r["from"].lower() == funding.ADDRESSES["relayer"], "Unexpected sponsor receipt")
        block = rpc("eth_getBlockByNumber", [r["blockNumber"],False])
        ex.require(block and block["hash"] == r["blockHash"], "Receipt is not canonical")
        if int(ex.fresh_head(rpc)["number"],16)-int(block["number"],16)>=3:
            break
        ex.require(attempt<5,"Receipt needs three subsequent blocks")
        time.sleep(2)
    return {"transaction_hash":tx,"block_number":int(r["blockNumber"],16),"block_hash":r["blockHash"],"sponsor":r["from"],"gas_used":int(r["gasUsed"],16)}


def run_open(plan, record, journal, path, wallets, rpc):
    req = plan["request"]
    if "signed_body" not in record:
        code, out, _ = request("/v1/base/creator-open/prepare", req)
        ex.require(code == 200, "Prepare " + plan["name"] + " HTTP " + str(code) + " " + str(out))
        record["signed_body"] = open_body(req, out["prepared"], wallets)
        ex.save(path, journal)
    body = record["signed_body"]
    status_body = {"network":"base-sepolia","approval_hash":body["approval_hash"]}
    # Reconcile every saved action before resubmitting its exact signed envelope.
    if record.get("attempted"):
        code, out, _ = request("/v1/base/creator-open/status", status_body)
    else:
        code, out = 404, None
    if not (code == 200 and out.get("confirmed")):
        record["attempted"] = True
        ex.save(path, journal)
        code, out, _ = request("/v1/base/creator-open/relay", body)
        ex.require(code in (200,202), "Relay " + plan["name"] + " HTTP " + str(code) + " " + str(out))
    for _ in range(15):
        if code == 200 and out.get("confirmed"):
            break
        time.sleep(3)
        code, out, _ = request("/v1/base/creator-open/status", status_body)
    ex.require(code == 200 and out.get("confirmed") and out["evidence"]["event"] == plan["event"], "Canonical outcome missing: " + plan["name"] + " " + str(out))
    ex.require(out["solver_paid"] == (plan["event"] == "OpenBountySettled"), "Payment evidence boundary differs")
    record["outcome"] = out
    record["receipt"] = receipt(rpc, out)
    ex.save(path, journal)
    # A duplicate must reuse its canonical transaction and consume no additional gas budget.
    before = get("/v1/base/gas-sponsorship")["daily_budget"]
    code, replay, _ = request("/v1/base/creator-open/relay", body)
    after = get("/v1/base/gas-sponsorship")["daily_budget"]
    ex.require(code == 200 and replay["operation"]["transaction_hash"] == out["operation"]["transaction_hash"]
               and after["reserved_micro_usdc"] == before["reserved_micro_usdc"], "Replay sent again or reserved more gas")
    record["replay_verified"] = True
    ex.save(path, journal)
    print(json.dumps({"action":plan["name"],"event":plan["event"],"receipt":record["receipt"],"replay_same_transaction":True}),flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    rpc = ex.Rpc()
    ex.load_bundle()
    ex.fresh_head(rpc)
    code, health, headers = request("/health")
    ex.require(code == 200 and health == "ok" and headers.get("x-agent-bounties-revision") == REVISION, "Canary revision differs")
    caps = get("/v1/base/creator-open/capabilities")
    ex.require(next(n for n in caps["networks"] if n["network"]=="base-sepolia")["release"]==OPEN_RELEASE, "Canary release differs")
    ex.require(not next(n for n in caps["networks"] if n["network"]=="base-mainnet")["available"], "Mainnet must stay unavailable")
    budget = get("/v1/base/gas-sponsorship")["daily_budget"]
    ex.require(budget["limit_micro_usdc"] == 1_000_000 and budget["carries_forward"] is False, "Shared daily cap differs")
    observed_balances = balances(rpc)
    print(json.dumps({"revision":REVISION,"daily_budget":budget,"balances":observed_balances,"execute":args.execute}),flush=True)
    if not args.execute:
        return
    STATE.mkdir(mode=0o700,exist_ok=True)
    ex.require(stat.S_IMODE(STATE.stat().st_mode)==0o700, "Private state directory required")
    with os.fdopen(os.open(STATE/"lock",os.O_CREAT|os.O_RDWR,0o600),"w") as locked:
        fcntl.flock(locked,fcntl.LOCK_EX|fcntl.LOCK_NB)
        path = STATE / "journal.json"
        journal = json.loads(path.read_text()) if path.exists() else {"api":API,"release":OPEN_RELEASE,"plans":new_plan(),"records":{},"initial_balances":observed_balances,"initial_budget":budget}
        ex.require(journal["api"] == API and journal["release"] == OPEN_RELEASE, "Saved rehearsal scope differs")
        ex.save(path,journal)
        wallet_path=ex.PRIVATE/"wallets.json"
        ex.require(stat.S_IMODE(wallet_path.stat().st_mode)==0o600, "Private test wallet permissions")
        wallets=json.loads(wallet_path.read_text())
        for role,address in funding.ADDRESSES.items():
            ex.require(Account.from_key(wallets[role]["private_key"]).address.lower()==address,"Unexpected dedicated signer")
        for plan in journal["plans"]:
            record = journal["records"].setdefault(plan["name"], {})
            run_open(plan,record,journal,path,wallets,rpc)
        evidence = {"schema":"agent-bounties/hosted-sepolia-rehearsal-v1","observed_at":datetime.now(timezone.utc).isoformat(),
                    "api":API,"revision":REVISION,"network":"base-sepolia","initial_balances":journal["initial_balances"],"final_balances":balances(rpc),
                    "initial_budget":journal["initial_budget"],"final_budget":get("/v1/base/gas-sponsorship")["daily_budget"],
                    "actions":[{"name":p["name"],"event":p["event"],"receipt":journal["records"][p["name"]]["receipt"],"replay_verified":True} for p in journal["plans"]],
                    "production_changed":False,"remaining_gates":["sponsored setup", "legacy hosted lifecycle", "creator-open indexer", "budget exhaustion", "flag-off reconciliation", "consented notifications", "authenticated GitHub inbox"]}
        OUTPUT.write_text(json.dumps(evidence,indent=2)+"\n")
        print(json.dumps({"evidence":str(OUTPUT),"completed_actions":len(evidence["actions"])}))


if __name__ == "__main__":
    main()
