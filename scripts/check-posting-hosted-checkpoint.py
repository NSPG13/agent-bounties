#!/usr/bin/env python3
"""Reconcile completed staging fixtures after a deployment, with signing off.

No keys are loaded and no new requests are prepared or signed. Legacy routes
replay only already-confirmed envelopes while every signing gate is disabled.
Saved signed journals stay private; output contains canonical public evidence.
"""
import argparse
from datetime import datetime, timezone
import importlib.util
import json
from pathlib import Path
import stat
import time

spec = importlib.util.spec_from_file_location("hosted", Path(__file__).with_name("check-posting-hosted-sepolia.py"))
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)
require = h.ex.require


def load(name):
    path = h.STATE / name
    require(stat.S_IMODE(path.stat().st_mode) == 0o600, "Private journal permissions")
    return json.loads(path.read_text())


def transaction(value):
    return (value.get("transaction_hash") or value.get("operation", {}).get("transaction_hash")
            or value.get("relay", {}).get("transaction"))


def disabled(revision):
    code, body, headers = h.request("/health")
    require(code == 200 and body == "ok" and headers.get("x-agent-bounties-revision") == revision,
            "Unexpected deployed revision")
    require(headers.get("x-agent-bounties-x402-relay") == "disabled", "Hosted signing must be off")
    gas = h.get("/v1/base/gas-sponsorship")
    for action in ("creation", "creator_verdict", "submission", "contribution", "claim", "authorization_revocation"):
        require(gas[action]["available"] is False, "Signing route is enabled: " + action)
    for protocol in ("creator-open", "sponsored-setup"):
        capabilities = h.get("/v1/base/" + protocol + "/capabilities")
        require(all(not item["available"] for item in capabilities["networks"]), "Protocol signing enabled")
    return gas["daily_budget"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    require(len(args.revision) == 40 and all(c in "0123456789abcdef" for c in args.revision), "Exact revision required")
    rpc = h.ex.Rpc()
    require(int(rpc("eth_chainId", []), 16) == 84532, "Sepolia only")
    before = disabled(args.revision)
    sponsor = h.funding.ADDRESSES["relayer"]
    nonce = rpc("eth_getTransactionCount", [sponsor, "pending"])
    balances = h.balances(rpc)
    results = []
    read_retries = []

    def read(path, body=None):
        # Retry only these read-only routes on temporary unavailability. Record
        # every failure; never retry a send or treat unavailable state as proof.
        require(body is None or path.endswith("/status"), "Read-only retry required")
        for attempt in range(3):
            code, outcome, headers = h.request(path, body)
            if code != 503 or attempt == 2:
                return code, outcome, headers
            read_retries.append({"path": path, "http": code, "attempt": attempt + 1})
            time.sleep(5)

    def record(name, path, body, expected, confirmed):
        # Refuse new or unknown operations even if a journal is accidentally edited.
        tx = expected["receipt"]["transaction_hash"]
        chain = h.receipt(rpc, {"operation": {"transaction_hash": tx}})
        require(chain["block_hash"] == expected["receipt"]["block_hash"], "Saved receipt changed")
        code, outcome, _ = (read(path, body) if path.endswith("/status")
                            else h.request(path, body))
        require(code == 200 and confirmed(outcome) and transaction(outcome) == tx,
                "Completed outcome not preserved: " + name + " HTTP " + str(code))
        results.append({"action": name, "http": code, "transaction_hash": tx,
                        "confirmed_with_signing_disabled": True})

    opened = load("journal.json")
    require(opened["api"] == h.API and opened["release"] == h.OPEN_RELEASE, "Wrong creator-open journal")
    for plan in opened["plans"]:
        item = opened["records"][plan["name"]]
        record(plan["name"], "/v1/base/creator-open/status",
               {"network": "base-sepolia", "approval_hash": item["signed_body"]["approval_hash"]}, item,
               lambda value: value.get("confirmed") is True)
    setup = load("setup-journal.json")
    require(setup["release"] == h.SETUP_RELEASE, "Wrong setup journal")
    for plan in setup["plans"]:
        record(plan["name"], "/v1/base/sponsored-setup/status", plan["request"],
               setup["records"][plan["name"]], lambda value: value.get("setup_complete") is True)
    legacy = load("legacy-journal.json")
    for name, route, field in (
        ("creation", "/v1/base/autonomous-bounties/creation-relay", "funding_confirmed"),
        ("submission", "/v1/base/autonomous-bounties/submission-relay", "confirmed"),
        ("verdict", "/v1/base/autonomous-bounties/creator-verdict-relay", "solver_paid"),
        ("revocation", "/v1/base/usdc/authorization-revocation-relay", "authorization_cancelled"),
    ):
        item = legacy["actions"][name]
        require(item["path"] == route and item["body"]["network"] == "base-sepolia", "Wrong saved route")
        record("legacy-" + name, route, item["body"], item, lambda value: value.get(field) is True)

    if (h.STATE / "open-edges-journal.json").exists():
        edges = load("open-edges-journal.json")
        require(edges["api"] == h.API and edges["release"] == h.OPEN_RELEASE, "Wrong edge-case journal")
        for plan in edges["plans"]:
            item = edges["records"][plan["name"]]
            require(item.get("replay_verified") and item.get("receipt"), "Edge-case rehearsal incomplete")
            record(plan["name"], "/v1/base/creator-open/status",
                   {"network": "base-sepolia", "approval_hash": item["signed_body"]["approval_hash"]}, item,
                   lambda value: value.get("confirmed") is True)
    if (h.STATE / "contract-wallet-journal.json").exists():
        contract = load("contract-wallet-journal.json")
        require(contract.get("fixture_calls", {}).get("account-principal-return", {}).get("receipt"),
                "Test account principal has not been recovered")
        routes = {
            "/v1/base/autonomous-bounties/creation-relay": "funding_confirmed",
            "/v1/base/autonomous-bounties/contribution-relay": "funding_confirmed",
            "/v1/base/autonomous-bounties/submission-relay": "confirmed",
            "/v1/base/autonomous-bounties/creator-verdict-relay": "solver_paid",
            "/v1/base/usdc/authorization-revocation-relay": "authorization_cancelled",
        }
        for name, item in contract["actions"].items():
            if item["path"] == "/v1/base/autonomous-bounties/claims":
                continue  # Authenticated claims were checked within their original sessions.
            require(item["path"] in routes and item["body"]["network"] == "base-sepolia"
                    and item.get("replay_verified"), "Unreviewed contract-wallet replay")
            field = routes[item["path"]]
            record(name, item["path"], item["body"], item, lambda value: value.get(field) is True)

    projections = []
    for name, status, paid in (("create", "settled", True), ("cancel-create", "cancelled", False)):
        contract = opened["records"][name]["outcome"]["bounty_contract"]
        code, snapshot, _ = read("/v1/base/creator-open/bounties?network=base-sepolia&bounty_contract=" + contract)
        require(code == 200, "Indexed bounty unavailable: HTTP " + str(code))
        require(snapshot["unavailable_count"] == 0 and len(snapshot["items"]) == 1, "Missing indexed bounty")
        item = snapshot["items"][0]
        require(item["status"] == status and item["solver_paid"] is paid
                and item["principal_micro_usdc"] == "0" and item["outstanding_bonds_micro_usdc"] == "0",
                "Indexed lifecycle changed")
        if paid:
            require(item["settlement_transaction"] == opened["records"]["review-2"]["receipt"]["transaction_hash"],
                    "Indexed payout changed")
        projections.append(item)

    after = disabled(args.revision)
    require(before == after, "Budget changed during reconciliation")
    require(rpc("eth_getTransactionCount", [sponsor, "pending"]) == nonce, "Sponsor nonce advanced")
    require(h.balances(rpc) == balances, "Fixture balances changed")
    result = {"observed_at": datetime.now(timezone.utc).isoformat(), "revision": args.revision,
              "network": "base-sepolia", "results": results, "indexer_projections": projections,
              "sender_nonce": int(nonce, 16), "sender_nonce_unchanged": True,
              "budget_unchanged": after, "customer_balances": balances, "keys_loaded": False,
              "temporary_read_failures": read_retries,
              "claim_status_not_requeried": "Authenticated claim was verified during the original session.",
              "production_changed": False}
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"completed_actions": len(results), "canonical_projections": len(projections),
                      "budget_unchanged": after, "sender_nonce_unchanged": True}))


if __name__ == "__main__":
    main()
