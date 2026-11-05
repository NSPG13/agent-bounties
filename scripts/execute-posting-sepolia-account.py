#!/usr/bin/env python3
"""Deploy only the separately approved Sepolia signature fixture, within the shared cap.

Read-only by default. --broadcast requires the exact approved bundle hash and a
live budget-only staging credential. Signed bytes are stored in a private outbox
before one broadcast. Unknown results are reconciled, never replaced or retried.
"""
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import time
from eth_account import Account
from _shared.evm import keccak256

spec = importlib.util.spec_from_file_location("hosted", Path(__file__).with_name("check-posting-hosted-sepolia.py"))
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)
ex = h.ex
BUNDLE = ex.ROOT / "docs/evidence/posting-sepolia-account-unsigned-review-20260930.json"
EXPECTED = "be47bca54484386f5b03bcf6e41631d92507e8f8646a54d5f8579ddd783cf657"
PATH = h.STATE / "account-deployment-journal.json"
OUTPUT = ex.ROOT / "docs/evidence/posting-sepolia-account-deployment-20261001.json"


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--broadcast", action="store_true")
    p.add_argument("--approved-sha256")
    p.add_argument("--revision", default="579c4a672779b055c16cbad31823f096e5213d9b")
    args = p.parse_args()
    raw = BUNDLE.read_bytes()
    ex.require(hashlib.sha256(raw).hexdigest() == EXPECTED, "Reviewed fixture bundle changed")
    bundle = json.loads(raw)
    intent = bundle["transaction"]
    ex.require(bundle["chain_id"] == 84532 and intent["from"] == ex.DEPLOYER and intent["to"] is None
               and intent["value_wei"] == "0" and intent["nonce"] == 5, "Unexpected fixture scope")
    for source, digest in bundle["source_hashes"].items():
        path = (ex.ROOT / "contracts/base-escrow" / source).resolve()
        ex.require((ex.ROOT / "contracts/base-escrow") in path.parents and keccak256(path.read_bytes()) == digest,
                   "Reviewed fixture source changed")
    ex.require(keccak256(bytes.fromhex(intent["data"][2:])) == intent["creation_code_hash"], "Init code changed")
    rpc = ex.Rpc(broadcast=args.broadcast)
    ex.fresh_head(rpc)
    with os.fdopen(os.open(h.STATE / "lock", os.O_CREAT | os.O_RDWR, 0o600), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if PATH.exists():
            ex.require(stat.S_IMODE(PATH.stat().st_mode) == 0o600, "Private outbox permissions")
            journal = json.loads(PATH.read_text())
            ex.require(journal["bundle_sha256"] == EXPECTED, "Saved deployment scope changed")
        else:
            journal = {"bundle_sha256": EXPECTED, "record": None}
        record = journal["record"]
        if record:
            ex.validate_saved(record, intent)
            result = ex.reconcile(rpc, intent, None, record)
            if result:
                record["receipt"] = result
                ex.save(PATH, journal)
                output = {"network": "base-sepolia", "observed_at": datetime.now(timezone.utc).isoformat(),
                    "bundle_sha256": EXPECTED, "receipt": result, "shared_budget_reservation": journal["reservation"],
                    "production_changed": False}
                OUTPUT.write_text(json.dumps(output, indent=2) + "\n")
                print(json.dumps(output))
                return
            ex.require(not record["attempts"], "Prior send remains unknown; read-only reconciliation required")
        budget = h.get("/v1/base/gas-sponsorship")["daily_budget"]
        ex.require(budget["limit_micro_usdc"] == 1_000_000 and budget["carries_forward"] is False, "Daily cap changed")
        print(json.dumps({"expected_contract": intent["expected_contract"], "daily_budget": budget,
                          "broadcast": args.broadcast}), flush=True)
        if not args.broadcast:
            return
        ex.require(args.approved_sha256 == EXPECTED, "Exact separate fixture approval required")
        ex.require(record is None and not journal.get("reservation_attempted"), "Previous reservation must be reconciled")
        ex.require(budget["remaining_micro_usdc"] > 0, "Shared allowance exhausted")
        code, health, headers = h.request("/health")
        ex.require(len(args.revision) == 40 and all(c in "0123456789abcdef" for c in args.revision), "Exact revision required")
        ex.require(code == 200 and health == "ok" and headers.get("x-agent-bounties-revision") == args.revision,
                   "Unexpected staging revision")
        caps = h.get("/v1/base/creator-open/capabilities")
        ex.require(not next(n for n in caps["networks"] if n["network"] == "base-mainnet")["available"], "Mainnet must remain unavailable")
        ex.require(int(rpc("eth_getTransactionCount", [ex.DEPLOYER, "pending"]), 16) == intent["nonce"], "Deployer nonce changed")
        ex.require(rpc("eth_getCode", [intent["expected_contract"], "latest"]) == "0x", "Deployment already exists; reconcile")
        head = ex.fresh_head(rpc)
        priority = min(int(rpc("eth_maxPriorityFeePerGas", []), 16), 1_000_000)
        maximum = max(1_000_000, int(head["baseFeePerGas"], 16) * 2 + priority)
        ex.require(maximum <= 100_000_000, "Fee ceiling exceeded")
        cost = ex.quote(rpc, intent, maximum)
        ex.require(cost <= int(intent["maximum_total_fee_wei"]), "Fee exceeds separately reviewed cap")
        ex.require(int(rpc("eth_getBalance", [ex.DEPLOYER, "latest"]), 16) >= cost, "Insufficient dedicated free test ETH")
        estimate = int(rpc("eth_estimateGas", [{"from": ex.DEPLOYER, "data": intent["data"], "value": "0x0"}]), 16)
        ex.require(estimate <= intent["gas_limit"], "Deployment gas changed")
        token_path = h.STATE / "budget-token"
        ex.require(stat.S_IMODE(token_path.stat().st_mode) == 0o600, "Budget credential permissions")
        scope = ex.transaction(intent, maximum, priority)
        reservation = {"network": "base-sepolia", "relayer_address": ex.DEPLOYER,
                       "transaction_digest": hashlib.sha256(json.dumps(scope, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
                       "maximum_fee_wei": str(cost)}
        journal["reservation_attempted"] = True
        journal["reservation_request"] = reservation
        ex.save(PATH, journal)
        code, approval, _ = h.request("/v1/operator/gas-sponsorship/reservations", reservation,
                                     {"Authorization": "Bearer " + token_path.read_text()})
        ex.require(code == 200 and approval.get("reserved") is True and
                   all(approval.get(k) == v for k, v in reservation.items()) and
                   approval.get("sender_nonce") == intent["nonce"] and approval.get("daily_limit_micro_usdc") == 1_000_000 and
                   approval.get("carries_forward") is False and approval.get("transaction_submitted") is False and
                   approval.get("shared_signer_lease") is True, "Reservation unavailable or different; preserve pending journal")
        journal["reservation"] = approval
        ex.save(PATH, journal)
        ex.require(time.time() + 2 < approval["valid_until"] <= time.time() + 35 and
                   approval["signer_lease_expires_at"] == approval["valid_until"] + 15, "Reservation window invalid")
        key_path = ex.PRIVATE / "wallets.json"
        ex.require(stat.S_IMODE(key_path.stat().st_mode) == 0o600, "Dedicated wallet permissions")
        wallets = json.loads(key_path.read_text())
        signer = next((Account.from_key(w["private_key"]) for w in wallets.values() if w["address"].lower() == ex.DEPLOYER), None)
        ex.require(signer is not None and signer.address.lower() == ex.DEPLOYER, "Dedicated deployer unavailable")
        signed = signer.sign_transaction(scope)
        record = {"raw": "0x" + signed.raw_transaction.hex(), "hash": keccak256(signed.raw_transaction),
                  "transaction": scope, "reserved_wei": cost, "attempts": 0}
        journal["record"] = record
        ex.validate_saved(record, intent)
        ex.save(PATH, journal)
        ex.require(time.time() + 1 < approval["valid_until"], "Reserved send window elapsed; do not broadcast")
        record["attempts"] = 1
        ex.save(PATH, journal)
        sent = rpc("eth_sendRawTransaction", [record["raw"]])
        ex.require(sent.lower() == record["hash"], "Unknown broadcast result; preserve exact outbox")
        print(json.dumps({"broadcast_transaction": sent, "next_step": "Run default read-only mode to reconcile exact runtime and receipt."}))


if __name__ == "__main__":
    main()
