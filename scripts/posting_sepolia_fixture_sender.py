"""Exact zero-value fixture calls from the existing isolated Sepolia sponsor.

Not a production sender. The caller validates its fixed native-test-USDC or
legacy-factory call. Every call uses the hosted shared budget, a short nonce
lease and a private outbox. One broadcast only; unknown outcomes retain bytes.
"""
import hashlib
import json
import stat
import time
from eth_account import Account
from eth_account.typed_transactions import TypedTransaction
from hexbytes import HexBytes
from _shared.evm import keccak256


def reconcile(h, record, rpc):
    raw = HexBytes(record["raw"])
    h.ex.require(keccak256(raw) == record["hash"] and
                 Account.recover_transaction(raw).lower() == h.funding.ADDRESSES["relayer"],
                 "Saved fixture signature changed")
    decoded = TypedTransaction.from_bytes(raw).as_dict()
    scope = record["transaction"]
    h.ex.require(all(decoded[k] == scope[k] for k in
        ("chainId", "nonce", "value", "gas", "type", "maxFeePerGas", "maxPriorityFeePerGas"))
        and not decoded["accessList"] and not scope["accessList"]
        and bytes(decoded["to"]).hex() == scope["to"][2:]
        and bytes(decoded["data"]).hex() == scope["data"][2:], "Saved transaction metadata differs")
    receipt = rpc("eth_getTransactionReceipt", [record["hash"]])
    h.ex.require(receipt is not None, "Prior fixture send unknown; reconcile without another broadcast")
    result = h.receipt(rpc, {"operation": {"transaction_hash": record["hash"]}})
    h.ex.require(receipt["transactionHash"].lower() == record["hash"] and
                 receipt["blockHash"].lower() == result["block_hash"].lower() and
                 receipt["to"].lower() == scope["to"] and "l1Fee" in receipt,
                 "Fixture receipt scope differs")
    tag = {"blockHash": result["block_hash"], "requireCanonical": True}
    fee = result["gas_used"] * int(receipt["effectiveGasPrice"], 16) + int(receipt["l1Fee"], 16)
    fee += h.ex.oracle_fee(rpc, "getOperatorFee(uint256)", result["gas_used"], tag)
    h.ex.require(fee <= record["reserved_wei"], "Actual fixture fee exceeded reservation")
    return {**result, "fee_wei": str(fee)}


def send(h, name, target, data, gas_limit, journal, path, rpc, wallets):
    ex = h.ex
    sender = h.funding.ADDRESSES["relayer"]
    allowed = {h.funding.USDC, "0xf699c4261709e67ad748c731ac0f7d014677e6e7"}
    ex.require(target.lower() in allowed and 0 < gas_limit <= 1_500_000 and data.startswith("0x"), "Fixture intent outside scope")
    ex.require(name in ("account-funding", "pooled-creation", "account-principal-return"), "Unknown fixture action")
    ex.fresh_head(rpc)
    call = {"to": target.lower(), "data": data.lower(), "value": 0, "gas": gas_limit, "chainId": 84532}
    manual = journal.setdefault("fixture_calls", {})
    record = manual.setdefault(name, {"call": call})
    ex.require(record["call"] == call, "Saved fixture call changed")
    ex.save(path, journal)
    if record.get("raw"):
        signed = HexBytes(record["raw"])
        ex.require(keccak256(signed) == record["hash"] and Account.recover_transaction(signed).lower() == sender,
                   "Saved fixture signature changed")
        tx = TypedTransaction.from_bytes(signed).as_dict()
        ex.require(all(tx[k] == call[k] for k in ("value", "gas", "chainId")) and
                   bytes(tx["to"]).hex() == target[2:].lower() and bytes(tx["data"]).hex() == data[2:].lower(),
                   "Saved fixture transaction differs")
        result = reconcile(h, record, rpc)
        record["receipt"] = result
        ex.save(path, journal)
        return result
    ex.require(not record.get("reservation_attempted"), "Unknown previous gas reservation; inspect ledger before resuming")
    head = ex.fresh_head(rpc)
    priority = min(int(rpc("eth_maxPriorityFeePerGas", []), 16), 1_000_000)
    fee = max(1_000_000, int(head["baseFeePerGas"], 16) * 2 + priority)
    ex.require(fee <= 100_000_000, "Fixture gas fee exceeds ceiling")
    rpc("eth_call", [{"from": sender, "to": target, "data": data}, "latest"])
    estimate = int(rpc("eth_estimateGas", [{"from": sender, "to": target, "data": data}]), 16)
    ex.require(estimate <= gas_limit, "Fixture call exceeds gas limit")
    maximum = ex.quote(rpc, {"data": data, "gas_limit": gas_limit}, fee)
    ex.require(maximum <= 100_000_000_000_000, "Fixture fee ceiling exceeded")
    ex.require(int(rpc("eth_getBalance", [sender, "latest"]), 16) >= maximum, "Missing sponsor funds")
    token = h.STATE / "budget-token"
    ex.require(stat.S_IMODE(token.stat().st_mode) == 0o600, "Budget credential permissions")
    nonce = int(rpc("eth_getTransactionCount", [sender, "pending"]), 16)
    scope = {**call, "nonce": nonce, "type": 2, "accessList": [], "maxFeePerGas": fee, "maxPriorityFeePerGas": priority}
    # eth-account requires a checksummed string or bytes for addresses. Validate
    # its exact representation before reserving scarce shared capacity.
    signing_scope = {**scope, "to": HexBytes(scope["to"])}
    TypedTransaction.from_dict(signing_scope)
    req = {"network": "base-sepolia", "relayer_address": sender,
           "transaction_digest": hashlib.sha256(json.dumps(scope, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
           "maximum_fee_wei": str(maximum)}
    record["reservation_attempted"] = True
    record["reservation_request"] = req
    ex.save(path, journal)
    code, reserved, _ = h.request("/v1/operator/gas-sponsorship/reservations", req, {"Authorization": "Bearer " + token.read_text()})
    ex.require(code == 200 and reserved.get("reserved") is True and reserved.get("sender_nonce") == nonce and
               all(reserved.get(k) == v for k, v in req.items()) and reserved.get("daily_limit_micro_usdc") == 1_000_000 and
               reserved.get("carries_forward") is False and reserved.get("transaction_submitted") is False and
               reserved.get("shared_signer_lease") is True and time.time() + 2 < reserved["valid_until"] <= time.time() + 35 and
               reserved["signer_lease_expires_at"] == reserved["valid_until"] + 15, "Fixture gas reservation unavailable; preserve journal")
    record["reservation"] = reserved
    ex.save(path, journal)
    signer = Account.from_key(wallets["relayer"]["private_key"])
    ex.require(signer.address.lower() == sender, "Wrong test sponsor")
    signed = signer.sign_transaction(signing_scope)
    record.update(raw="0x" + signed.raw_transaction.hex(), hash=keccak256(signed.raw_transaction), transaction=scope, reserved_wei=maximum)
    ex.save(path, journal)
    ex.require(time.time() + 1 < reserved["valid_until"], "Fixture reservation expired; do not send")
    record["broadcast_attempted"] = True
    ex.save(path, journal)
    sent = rpc("eth_sendRawTransaction", [record["raw"]])
    ex.require(sent.lower() == record["hash"], "Unknown fixture send response; preserve outbox")
    for _ in range(12):
        receipt = rpc("eth_getTransactionReceipt", [record["hash"]])
        if receipt:
            result = reconcile(h, record, rpc)
            record["receipt"] = result
            ex.save(path, journal)
            print(json.dumps({"fixture_action": name, "receipt": result}), flush=True)
            return result
        time.sleep(3)
    raise ValueError("Fixture send remains unknown; use read-only reconciliation")
