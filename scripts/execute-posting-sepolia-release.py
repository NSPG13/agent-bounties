#!/usr/bin/env python3
"""Execute only the user's exact approved four-contract Base Sepolia bundle.

Default mode is read-only. --broadcast uses the dedicated test key and a durable,
locked outbox outside Git. Unknown broadcasts preserve the same bytes and nonce;
--retry-saved permits at most three attempts of those bytes. Never uses mainnet.
Install requirements-wallet.txt and requirements-attest.txt in a private venv.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile
import time
import urllib.error
import urllib.request

from eth_account import Account
from eth_account.typed_transactions import TypedTransaction
from hexbytes import HexBytes
from _shared.evm import keccak256, keccak_bytes, uint_word, create_address

ROOT = Path(__file__).resolve().parents[1]
BUNDLE = ROOT / "docs/evidence/posting-sepolia-unsigned-deployment-20260930.json"
APPROVED = "de4fea7a4a7f8b5a838ada005f044cd206deca89a380b3fd8f9f9e2a73c6e158"
DEPLOYER = "0x7425b6bee34972592d251cd53760845593ec90ee"
CAP = 10**15  # 0.001 free test ETH for all four deployments together.
ORACLE = "0x420000000000000000000000000000000000000f"
PRIVATE = Path.home() / ".local/share/agent-bounties/posting-sepolia-20260930"


def require(ok, message):
    if not ok:
        raise ValueError(message)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


class Rpc:
    def __init__(self, broadcast=False):
        self.broadcast = broadcast
        self.http = urllib.request.build_opener(NoRedirect())

    def __call__(self, method, params):
        reads = {"eth_chainId", "eth_getBlockByNumber", "eth_getCode", "eth_call",
                 "eth_getTransactionCount", "eth_getBalance", "eth_estimateGas",
                 "eth_maxPriorityFeePerGas", "eth_getTransactionReceipt"}
        require(method in reads or (self.broadcast and method == "eth_sendRawTransaction"),
                "RPC operation is outside the approved deployment")
        request = urllib.request.Request("https://sepolia.base.org", data=json.dumps({
            "jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
            headers={"Content-Type": "application/json", "User-Agent": "agent-bounties-release/1"})
        try:
            with self.http.open(request, timeout=25) as response:
                raw = response.read(2_000_001)
            require(len(raw) <= 2_000_000, "Oversized RPC response")
            value = json.loads(raw)
            require(value.get("id") == 1 and value.get("jsonrpc") == "2.0"
                    and "error" not in value, "RPC failed: " + method)
            return value["result"]
        except (OSError, urllib.error.URLError) as error:
            raise RuntimeError("RPC transport failed: " + method) from None


def load_bundle(path=BUNDLE):
    raw = path.read_bytes()
    require(hashlib.sha256(raw).hexdigest() == APPROVED, "Bundle differs from the approved hash")
    value = json.loads(raw)
    require(value["chain_id"] == 84532 and value["deployer"] == DEPLOYER,
            "Wrong approved chain or deployer")
    require(len(value["transactions"]) == 4, "Wrong deployment count")
    for i, tx in enumerate(value["transactions"]):
        require(tx["nonce"] == i and tx["chain_id"] == 84532 and tx["from"] == DEPLOYER
                and tx["to"] is None and tx["value_wei"] == "0"
                and tx["expected_contract"] == create_address(DEPLOYER, i), "Invalid deployment intent")
        require(keccak256(bytes.fromhex(tx["data"][2:])) == tx["creation_code_hash"], "Invalid init code")
        for name, digest in tx["source_hashes"].items():
            source = (ROOT / "contracts/base-escrow" / name).resolve()
            require((ROOT / "contracts/base-escrow") in source.parents
                    and keccak256(source.read_bytes()) == digest, "Reviewed source changed: " + name)
    return value


def save(path, value):
    """Atomic file replacement plus file/directory fsync before any broadcast."""
    fd, name = tempfile.mkstemp(prefix=".journal-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as file:
            json.dump(value, file, indent=2)
            file.write("\n")
            file.flush()
            os.fsync(file.fileno())
        os.replace(name, path)
        parent = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def fresh_head(rpc):
    require(rpc("eth_chainId", []) == "0x14a34", "RPC is not Base Sepolia")
    head = rpc("eth_getBlockByNumber", ["latest", False])
    require(head is not None and abs(int(time.time()) - int(head["timestamp"], 16)) < 180,
            "Stale Base Sepolia head")
    return head


def oracle_fee(rpc, function, amount, tag="latest"):
    data = "0x" + (keccak_bytes(function.encode())[:4] + uint_word(amount)).hex()
    value = rpc("eth_call", [{"to": ORACLE, "data": data}, tag])
    require(isinstance(value, str) and len(value) == 66, "Invalid fee oracle response")
    return int(value, 16)


def quote(rpc, intent, max_fee):
    # The reviewed padded envelope bounds size before signing. Variable rollup
    # fees receive a 2x margin and are re-quoted immediately before broadcast.
    length = len(bytes.fromhex(intent["data"][2:])) + 256
    extra = oracle_fee(rpc, "getL1FeeUpperBound(uint256)", length)
    extra += oracle_fee(rpc, "getOperatorFee(uint256)", intent["gas_limit"])
    return intent["gas_limit"] * max_fee + 2 * extra


def transaction(intent, maximum, priority):
    return {"chainId": 84532, "nonce": intent["nonce"], "value": 0,
            "data": intent["data"], "gas": intent["gas_limit"], "type": 2,
            "maxFeePerGas": maximum, "maxPriorityFeePerGas": priority, "accessList": []}


def reserve_remaining(rpc, bundle, records, start, maximum):
    costs = [int(r["receipt"]["fee_wei"]) for r in records[:start]]
    for i in range(start, 4):
        intent = bundle["transactions"][i]
        record = records[i] if i < len(records) else None
        fee = validate_saved(record, intent) if record else maximum
        costs.append(max(quote(rpc, intent, fee), record["reserved_wei"] if record else 0))
    require(sum(costs) <= CAP, "Fresh total fee quote exceeds approved 0.001 test ETH cap")
    require(int(rpc("eth_getBalance", [DEPLOYER, "latest"]), 16) >= sum(costs[start:]),
            "Insufficient free test ETH for remaining approved deployments")
    return costs


def validate_saved(record, intent):
    raw = HexBytes(record["raw"])
    require(record["hash"] == keccak256(raw), "Saved transaction hash mismatch")
    require(Account.recover_transaction(raw).lower() == DEPLOYER, "Saved signer mismatch")
    decoded = TypedTransaction.from_bytes(raw).as_dict()
    maximum, priority = decoded["maxFeePerGas"], decoded["maxPriorityFeePerGas"]
    require(0 <= priority <= maximum and maximum > 0, "Invalid saved fee caps")
    expected = transaction(intent, maximum, priority)
    for key in ("chainId", "nonce", "value", "gas", "type", "maxFeePerGas", "maxPriorityFeePerGas"):
        require(decoded[key] == expected[key], "Saved transaction scope changed: " + key)
    require(bytes(decoded["to"]) == b"" and bytes(decoded["data"]) == bytes.fromhex(intent["data"][2:])
            and not decoded["accessList"], "Saved transaction destination or data changed")
    require(record["transaction"] == expected, "Saved transaction metadata differs from signed bytes")
    require(0 <= record["attempts"] <= 3, "Invalid saved broadcast count")
    return maximum


def reconcile(rpc, intent, implementation, record):
    receipt = rpc("eth_getTransactionReceipt", [record["hash"]])
    if receipt is None:
        return None
    block = rpc("eth_getBlockByNumber", [receipt["blockNumber"], False])
    require(block and block["hash"] == receipt["blockHash"], "Receipt is not canonical")
    require(receipt["transactionHash"].lower() == record["hash"]
            and receipt["from"].lower() == DEPLOYER, "Receipt sender/hash mismatch")
    require(receipt["status"] == "0x1", "Deployment reverted; preserve journal and stop")
    require(receipt["contractAddress"].lower() == intent["expected_contract"], "Wrong deployed address")
    tag = {"blockHash": block["hash"], "requireCanonical": True}
    require(rpc("eth_getCode", [intent["expected_contract"], tag]).lower()
            == intent["expected_runtime_code"].lower(), "Deployed runtime differs from approved bytes")
    if implementation:
        require(rpc("eth_getCode", [implementation["address"], tag]).lower()
                == implementation["runtime_code"].lower(), "Implementation runtime differs from approved bytes")
    require("l1Fee" in receipt, "Missing rollup fee receipt")
    gas = int(receipt["gasUsed"], 16)
    operator = oracle_fee(rpc, "getOperatorFee(uint256)", gas, tag)
    spent = gas * int(receipt["effectiveGasPrice"], 16) + int(receipt["l1Fee"], 16) + operator
    require(spent <= record["reserved_wei"], "Actual fee exceeded its conservative reservation")
    head = fresh_head(rpc)
    if int(head["number"], 16) - int(block["number"], 16) < 3:
        return None
    return {"transaction_hash": record["hash"], "contract": intent["expected_contract"],
            "block_number": int(block["number"], 16), "block_hash": block["hash"],
            "gas_used": gas, "fee_wei": str(spent), "runtime_code_hash": intent["runtime_code_hash"]}


def run(rpc, bundle, journal, path, signer=None, retry_saved=False, wait_seconds=180):
    head = fresh_head(rpc)
    require(journal["bundle_sha256"] == APPROVED and journal["cap_wei"] == CAP,
            "Journal approval differs")
    records = journal["records"]
    require(len(records) <= 4, "Too many saved deployments")
    require(rpc("eth_getCode", [DEPLOYER, "latest"]) == "0x", "Deployer is no longer the dedicated EOA")
    require(rpc("eth_getCode", [bundle["native_usdc"], "latest"]) != "0x", "Test USDC is missing")
    for i, record in enumerate(records):
        intent = bundle["transactions"][i]
        validate_saved(record, intent)
        receipt = reconcile(rpc, intent, bundle["implementations"].get(intent["name"]), record)
        require(receipt is not None or i == len(records) - 1,
                "Earlier saved nonce has no canonical receipt")
        require(receipt is not None or record.get("receipt") is None,
                "Previously reconciled receipt disappeared; operator reconciliation required")
        record["receipt"] = receipt
    pending = int(rpc("eth_getTransactionCount", [DEPLOYER, "pending"]), 16)
    mined = int(rpc("eth_getTransactionCount", [DEPLOYER, "latest"]), 16)
    confirmed = sum(r.get("receipt") is not None for r in records)
    # After all four canonical receipts, later infrastructure funding is outside
    # this executor. Re-audit the completed bundle without re-deploying anything.
    require((4 <= mined <= pending) if confirmed == 4 else (confirmed <= mined <= pending <= len(records)),
            "Unexpected deployer nonce; do not replace transactions")
    priority = max(1, int(rpc("eth_maxPriorityFeePerGas", []), 16))
    maximum = int(head["baseFeePerGas"], 16) * 2 + priority
    costs = []
    for i, intent in enumerate(bundle["transactions"]):
        record = records[i] if i < len(records) else None
        if record and record.get("receipt"):
            costs.append(int(record["receipt"]["fee_wei"]))
        else:
            fee = validate_saved(record, intent) if record else maximum
            costs.append(max(quote(rpc, intent, fee), record["reserved_wei"] if record else 0))
            require(rpc("eth_getCode", [intent["expected_contract"], "latest"]) == "0x"
                    or record is not None, "Untracked deployed contract occupies an approved address")
    require(sum(costs) <= CAP, "Fresh total fee quote exceeds approved 0.001 test ETH cap")
    remaining = sum(costs[confirmed:])
    balance = int(rpc("eth_getBalance", [DEPLOYER, "latest"]), 16)
    report = {"bundle_sha256": APPROVED, "chain_id": 84532, "deployer": DEPLOYER,
              "balance_wei": str(balance), "maximum_total_fee_wei": str(sum(costs)),
              "remaining_fee_bound_wei": str(remaining), "funded": balance >= remaining,
              "confirmed_contracts": confirmed, "broadcast_enabled": signer is not None}
    print(json.dumps(report), flush=True)
    if signer is None:
        return report
    require(signer.address.lower() == DEPLOYER, "Only the dedicated test deployer is approved")
    require(balance >= remaining, "Insufficient free test ETH; do not buy tokens or use production keys")
    save(path, journal)
    for i in range(confirmed, 4):
        intent = bundle["transactions"][i]
        fresh_head(rpc)
        costs = reserve_remaining(rpc, bundle, records, i, maximum)
        if i >= len(records):
            require(int(rpc("eth_getTransactionCount", [DEPLOYER, "pending"]), 16) == i
                    and int(rpc("eth_getTransactionCount", [DEPLOYER, "latest"]), 16) == i,
                    "Deployer nonce moved before signing")
            require(rpc("eth_getCode", [intent["expected_contract"], "latest"]) == "0x",
                    "Approved address occupied before signing")
            tx = transaction(intent, maximum, priority)
            call = {"from": DEPLOYER, "data": intent["data"], "value": "0x0", "nonce": hex(i),
                    "maxFeePerGas": hex(maximum), "maxPriorityFeePerGas": hex(priority)}
            estimate = int(rpc("eth_estimateGas", [call]), 16)
            require(estimate <= intent["gas_limit"], "Live estimate exceeds approved gas limit")
            signed = signer.sign_transaction(tx)
            record = {"transaction": tx, "raw": "0x" + bytes(signed.raw_transaction).hex(),
                      "hash": keccak256(signed.raw_transaction), "attempts": 0,
                      "reserved_wei": costs[i], "receipt": None}
            validate_saved(record, intent)
            records.append(record)
            save(path, journal)
        record = records[i]
        receipt = reconcile(rpc, intent, bundle["implementations"].get(intent["name"]), record)
        if receipt is None:
            fee = validate_saved(record, intent)
            costs = reserve_remaining(rpc, bundle, records, i, maximum)
            record["reserved_wei"] = costs[i]
            require(record["attempts"] == 0 or retry_saved,
                    "Saved transaction outcome is pending/unknown; reconcile or explicitly retry identical bytes")
            require(record["attempts"] < 3, "Saved transaction reached the three-attempt limit")
            record["attempts"] += 1
            save(path, journal)  # Record intent to broadcast even if the response is lost.
            try:
                observed = rpc("eth_sendRawTransaction", [record["raw"]])
                require(observed.lower() == record["hash"], "Broadcast hash differs from saved bytes")
            except Exception:
                raise RuntimeError("Broadcast outcome unknown; preserve saved bytes and reconcile before retry") from None
            print(json.dumps({"name": intent["name"], "broadcast_hash": record["hash"]}), flush=True)
            deadline = time.monotonic() + wait_seconds
            while receipt is None and time.monotonic() < deadline:
                time.sleep(2)
                receipt = reconcile(rpc, intent, bundle["implementations"].get(intent["name"]), record)
            require(receipt is not None, "Receipt not yet reconciled; signed bytes remain saved")
        record["receipt"] = receipt
        save(path, journal)
        print(json.dumps({"name": intent["name"], "canonical_receipt": receipt}), flush=True)
    safe = rpc("eth_getBlockByNumber", ["safe", False])
    journal["safe_at_observation"] = bool(safe and int(safe["number"], 16) >= records[-1]["receipt"]["block_number"])
    journal["complete"] = True
    journal["observed_at"] = datetime.now(timezone.utc).isoformat()
    save(path, journal)
    return {**report, "confirmed_contracts": 4, "safe_at_observation": journal["safe_at_observation"],
            "balance_wei": str(int(rpc("eth_getBalance", [DEPLOYER, "latest"]), 16)),
            "maximum_total_fee_wei": str(sum(int(r["receipt"]["fee_wei"]) for r in records)),
            "remaining_fee_bound_wei": "0"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--broadcast", action="store_true")
    parser.add_argument("--retry-saved", action="store_true")
    parser.add_argument("--state-directory", type=Path, default=PRIVATE / "deployment")
    parser.add_argument("--wallets", type=Path, default=PRIVATE / "wallets.json")
    args = parser.parse_args()
    bundle = load_bundle()
    directory = args.state_directory.resolve()
    require(ROOT not in directory.parents and directory != ROOT, "Keep signed bytes outside Git")
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    require(stat.S_IMODE(directory.stat().st_mode) == 0o700, "State directory must have mode 0700")
    path = directory / "journal.json"
    lock_fd = os.open(directory / "lock", os.O_CREAT | os.O_RDWR, 0o600)
    with os.fdopen(lock_fd, "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        journal = json.loads(path.read_text()) if path.exists() else {
            "bundle_sha256": APPROVED, "cap_wei": CAP, "records": [], "complete": False}
        signer = None
        if args.broadcast:
            require(stat.S_IMODE(args.wallets.stat().st_mode) == 0o600, "Test wallet file must have mode 0600")
            wallets = json.loads(args.wallets.read_text())
            require(wallets["deployer"]["address"].lower() == DEPLOYER, "Wrong test wallet file")
            signer = Account.from_key(wallets["deployer"]["private_key"])
        result = run(Rpc(args.broadcast), bundle, journal, path, signer, args.retry_saved)
        print(json.dumps(result))


if __name__ == "__main__":
    main()
