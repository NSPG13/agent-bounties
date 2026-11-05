#!/usr/bin/env python3
"""Dedicated Sepolia competition and expiry rehearsal, read-only by default.

The existing test sponsor posts two 0.6-test-USDC bounties. The two existing
zero-ETH customer identities compete, then recover an unreviewed bond. No new
contract implementation, wallet, ETH transfer or production action is allowed.
--execute starts/resumes exact journaled requests; --phase finish requires the
real chain deadline to have passed and never advances the clock.
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
import urllib.request
from eth_account import Account

spec = importlib.util.spec_from_file_location("hosted", Path(__file__).with_name("check-posting-hosted-sepolia.py"))
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)
ex = h.ex
REVISION = "579c4a672779b055c16cbad31823f096e5213d9b"
# Reviewed availability/quota-only upgrade; contract release bytes are unchanged.
REVIEWED_TRANSITIONS = {
    ("bdbf2f7805b4e33a576beeebdab2a38ca6c840ed", "8667e2af1db9c3ae98c71cc8de4a78b4c5ed3f7a"),
}
JOURNAL = h.STATE / "open-edges-journal.json"
OUTPUT = ex.ROOT / "docs/evidence/posting-hosted-open-edges-20261001.json"


def plans(now):
    digest = lambda label: h.keccak256(("posting-open-edges-20261001/" + label).encode())
    base = copy.deepcopy(h.new_plan()[0]["request"])
    base["original_terms"].update(
        creator=h.funding.ADDRESSES["relayer"], bounty_id=digest("competition"),
        solver_reward_micro_usdc=500_000, review_reward_micro_usdc=100_000,
        title="Synthetic two-entrant sponsored competition",
        goal="Verify one winner and the other entrant's fixed-beneficiary bond refund.",
        submission_deadline=now + 86400, review_window_seconds=3600)
    base.update(user_salt=digest("competition-salt"), expected_solver_reward_micro_usdc=500_000,
                valid_before=now + 1800, action={"kind": "create", "token_nonce": digest("competition-fund")})
    result = []

    def add(name, request, event, phase="start"):
        result.append({"name": name, "request": copy.deepcopy(request), "event": event, "phase": phase})

    add("competition-create", base, "CreatorOpenBountyCreated")
    entries = []
    for n, role in enumerate(("creator", "solver"), 1):
        base["action"] = {"kind": "submit", "solver": h.funding.ADDRESSES[role],
                          "submission_hash": digest("submission-" + role), "evidence_hash": digest("evidence-" + role),
                          "token_nonce": digest("bond-" + role)}
        entries.append(copy.deepcopy(base["action"]))
        add("competition-entry-" + str(n), base, "OpenSubmissionAdded")
    base["action"] = {k: v for k, v in entries[0].items() if k != "token_nonce"}
    base["action"].update(kind="review", entry=1, passed=True, response_hash=digest("winner"))
    add("competition-winner", base, "OpenBountySettled")
    base["action"] = {"kind": "refund_bond", "entry": 2}
    add("competition-other-bond", base, "OpenBondRefunded")

    base["original_terms"].update(bounty_id=digest("expiry"), title="Synthetic sponsored expiry and refund",
                                  goal="Verify real elapsed deadlines and exact bond and principal refunds.",
                                  submission_deadline=now + 300)
    base.update(user_salt=digest("expiry-salt"), action={"kind": "create", "token_nonce": digest("expiry-fund")})
    add("expiry-create", base, "CreatorOpenBountyCreated")
    base["action"] = {"kind": "submit", "solver": h.funding.ADDRESSES["solver"],
                      "submission_hash": digest("expiry-submission"), "evidence_hash": digest("expiry-evidence"),
                      "token_nonce": digest("expiry-bond")}
    add("expiry-entry", base, "OpenSubmissionAdded")
    for name, action, event in (
        ("expiry-bond", {"kind": "refund_bond", "entry": 1}, "OpenBondRefunded"),
        ("expiry-close", {"kind": "expire"}, "OpenBountyCancelled"),
        ("expiry-principal", {"kind": "refund_principal"}, "OpenPrincipalRefunded"),
    ):
        base["action"] = action
        add(name, base, event, "finish")
    return result


def principal_deltas(journal, rpc):
    """Measure only this rehearsal's canonical token logs, allowing other tests."""
    deltas = {role: 0 for role in ("relayer", "creator", "solver")}
    transfer = h.keccak256(b"Transfer(address,address,uint256)")
    for plan in journal["plans"]:
        saved = journal["records"][plan["name"]]["receipt"]
        confirmed = h.receipt(rpc, {"operation": {"transaction_hash": saved["transaction_hash"]}})
        ex.require(confirmed["block_hash"] == saved["block_hash"], "Saved receipt changed")
        receipt = rpc("eth_getTransactionReceipt", [saved["transaction_hash"]])
        for log in receipt["logs"]:
            if log["address"].lower() != h.funding.USDC or log["topics"][0].lower() != transfer:
                continue
            ex.require(len(log["topics"]) == 3 and len(log["data"]) == 66, "Malformed USDC transfer")
            sender, receiver = ("0x" + t[-40:].lower() for t in log["topics"][1:])
            amount = int(log["data"], 16)
            for role in deltas:
                address = h.funding.ADDRESSES[role]
                deltas[role] += amount * ((receiver == address) - (sender == address))
    ex.require(deltas == {"relayer": -500_000, "creator": 500_000, "solver": 0},
               "Canonical test principal conservation failed")
    for name in ("competition-create", "expiry-create"):
        bounty = journal["records"][name]["outcome"]["bounty_contract"]
        call = "0x" + (h.keccak_bytes(b"balanceOf(address)")[:4] + h.address_word(bounty)).hex()
        ex.require(int(rpc("eth_call", [{"to": h.funding.USDC, "data": call}, "latest"]), 16) == 0,
                   "Fixture escrow retains principal or bond")
    return deltas


def run_expiry_with_discarded_reply(plan, record, journal, wallets, rpc):
    """Discard one actual HTTP response body, then reconcile through status only."""
    original = h.request
    if record.get("discarded_reply_attempted") and not record.get("receipt"):
        # A previous interrupted test cannot turn into a second uncertain send.
        code, outcome, _ = original("/v1/base/creator-open/status", {
            "network": "base-sepolia", "approval_hash": record["signed_body"]["approval_hash"]})
        ex.require(code == 200 and outcome.get("confirmed"),
                   "Discarded-reply action remains uncertain; use status-only reconciliation")

    def request(path, body=None, headers=None):
        if path != "/v1/base/creator-open/relay" or record.get("discarded_reply_attempted"):
            return original(path, body, headers)
        ex.require(plan["name"] == "expiry-close" and body == record["signed_body"]
                   and body["request"]["action"] == {"kind": "expire"}
                   and body["signatures"] == {"token": "0x", "action": "0x"},
                   "Lost-reply probe must be the exact permissionless test expiry")
        record["discarded_reply_attempted"] = True
        ex.save(JOURNAL, journal)
        req = urllib.request.Request(h.API + path, data=json.dumps(body).encode(),
                                     headers={"Content-Type": "application/json"})
        with urllib.request.build_opener(ex.NoRedirect()).open(req, timeout=55) as response:
            # Deliberately do not read or decode the outcome/transaction hash.
            record["discarded_reply_http_status"] = response.status
            ex.require(response.status in (200, 202), "Unexpected discarded response status")
        ex.save(JOURNAL, journal)
        return 202, {"confirmed": False}, {}

    h.request = request
    try:
        h.run_open(plan, record, journal, JOURNAL, wallets, rpc)
    finally:
        h.request = original
    record["discarded_reply_recovered_via_status"] = True
    ex.save(JOURNAL, journal)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--revision", default=REVISION)
    parser.add_argument("--phase", choices=("start", "finish"), default="start")
    args = parser.parse_args()
    rpc = ex.Rpc()
    ex.load_bundle()
    head = ex.fresh_head(rpc)
    code, value, headers = h.request("/health")
    ex.require(len(args.revision) == 40 and all(c in "0123456789abcdef" for c in args.revision), "Exact revision required")
    ex.require(code == 200 and value == "ok" and headers.get("x-agent-bounties-revision") == args.revision,
               "Exact staging revision required")
    caps = h.get("/v1/base/creator-open/capabilities")
    testnet = next(n for n in caps["networks"] if n["network"] == "base-sepolia")
    ex.require(testnet["release"] == h.OPEN_RELEASE and
               not next(n for n in caps["networks"] if n["network"] == "base-mainnet")["available"],
               "Dedicated Sepolia release required")
    budget = h.get("/v1/base/gas-sponsorship")["daily_budget"]
    ex.require(budget["limit_micro_usdc"] == 1_000_000 and not budget["carries_forward"], "Daily cap changed")
    initial = h.balances(rpc)
    print(json.dumps({"phase": args.phase, "execute": args.execute, "daily_budget": budget,
                      "customer_eth_wei": {r: initial[r]["eth_wei"] for r in ("creator", "solver")}}), flush=True)
    if not args.execute:
        return
    ex.require(testnet["available"] and budget["remaining_micro_usdc"] >= 100_000, "Insufficient shared allowance")
    ex.require(stat.S_IMODE(h.STATE.stat().st_mode) == 0o700, "Private state permissions")
    with os.fdopen(os.open(h.STATE / "lock", os.O_CREAT | os.O_RDWR, 0o600), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if JOURNAL.exists():
            ex.require(stat.S_IMODE(JOURNAL.stat().st_mode) == 0o600, "Private journal permissions")
            journal = json.loads(JOURNAL.read_text())
        else:
            ex.require(args.phase == "start", "Start phase has not run")
            journal = {"api": h.API, "release": h.OPEN_RELEASE, "revision": args.revision,
                       "plans": plans(int(time.time())), "records": {},
                       "initial_balances": initial, "initial_budget": budget}
        ex.require(journal["api"] == h.API and journal["release"] == h.OPEN_RELEASE, "Saved scope changed")
        if journal["revision"] != args.revision:
            ex.require((journal["revision"], args.revision) in REVIEWED_TRANSITIONS,
                       "Unreviewed runtime transition")
            transitions = journal.setdefault("runtime_transitions", [])
            if not any(item["to"] == args.revision for item in transitions):
                transitions.append({"from": journal["revision"], "to": args.revision,
                                    "observed_at": datetime.now(timezone.utc).isoformat(),
                                    "original_requests_preserved": True})
        finish_after = journal["plans"][5]["request"]["original_terms"]["submission_deadline"] + 3600
        if args.phase == "finish":
            ex.require(all(p["name"] in journal["records"] and journal["records"][p["name"]].get("replay_verified")
                           for p in journal["plans"] if p["phase"] == "start"), "Start phase incomplete")
            ex.require(int(head["timestamp"], 16) > finish_after, "Real on-chain expiry has not passed")
        ex.save(JOURNAL, journal)
        key_path = ex.PRIVATE / "wallets.json"
        ex.require(stat.S_IMODE(key_path.stat().st_mode) == 0o600, "Dedicated wallet permissions")
        wallets = json.loads(key_path.read_text())
        for role, address in h.funding.ADDRESSES.items():
            ex.require(Account.from_key(wallets[role]["private_key"]).address.lower() == address, "Wrong test identity")
        for plan in journal["plans"]:
            if plan["phase"] != args.phase:
                continue
            record = journal["records"].setdefault(plan["name"], {})
            if plan["name"] == "expiry-create" and "signed_body" not in record:
                ex.require(not any(journal["records"].get(p["name"], {}).get("signed_body")
                                   for p in journal["plans"] if p["name"].startswith("expiry-")),
                           "Cannot change committed expiry terms")
                # This independent fixture begins only after the competition.
                # Never alter any request that already carries a signature.
                for pending in journal["plans"]:
                    if pending["name"].startswith("expiry-"):
                        pending["request"]["original_terms"]["submission_deadline"] = int(time.time()) + 300
                        pending["request"]["valid_before"] = int(time.time()) + 1800
                finish_after = plan["request"]["original_terms"]["submission_deadline"] + 3600
                ex.save(JOURNAL, journal)
            if args.phase == "finish" and "signed_body" not in record:
                plan["request"]["valid_before"] = int(time.time()) + 1800
                ex.save(JOURNAL, journal)
            # Preparation is read-only. Preserve intermittent failures as evidence,
            # and retry it only before an envelope exists; sends stay single-shot.
            for attempt in range(3):
                try:
                    if plan["name"] == "expiry-close":
                        run_expiry_with_discarded_reply(plan, record, journal, wallets, rpc)
                    else:
                        h.run_open(plan, record, journal, JOURNAL, wallets, rpc)
                    break
                except ValueError as error:
                    if "signed_body" in record or not str(error).startswith("Prepare ") or attempt == 2:
                        raise
                    journal.setdefault("preparation_failures", []).append({"action": plan["name"], "error": str(error)})
                    ex.save(JOURNAL, journal)
                    time.sleep(5)
        final = h.balances(rpc)
        complete = all(journal["records"].get(p["name"], {}).get("replay_verified") for p in journal["plans"])
        deltas = principal_deltas(journal, rpc) if complete else None
        evidence = {"network": "base-sepolia", "observed_at": datetime.now(timezone.utc).isoformat(),
                    "revision": args.revision, "complete": complete, "finish_after_unix": finish_after,
                    "runtime_transitions": journal.get("runtime_transitions", []),
                    "lost_http_reply": {key: journal["records"].get("expiry-close", {}).get(key)
                                        for key in ("discarded_reply_attempted", "discarded_reply_http_status",
                                                    "discarded_reply_recovered_via_status")},
                    "fixture_poster": "Existing funded test sponsor; both entrants retain zero ETH",
                    "initial_balances": journal["initial_balances"], "final_balances": final,
                    "canonical_test_usdc_deltas": deltas,
                    "preparation_failures": journal.get("preparation_failures", []),
                    "initial_budget": journal["initial_budget"], "final_budget": h.get("/v1/base/gas-sponsorship")["daily_budget"],
                    "actions": [{"name": p["name"], "event": p["event"], "receipt": journal["records"][p["name"]]["receipt"],
                                 "replay_verified": True} for p in journal["plans"] if journal["records"].get(p["name"], {}).get("replay_verified")],
                    "production_changed": False}
        OUTPUT.write_text(json.dumps(evidence, indent=2) + "\n")
        print(json.dumps({"complete": complete, "completed_actions": len(evidence["actions"]), "finish_after_unix": finish_after}))


if __name__ == "__main__":
    main()
