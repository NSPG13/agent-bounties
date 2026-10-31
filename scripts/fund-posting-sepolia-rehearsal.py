#!/usr/bin/env python3
"""Fund only this task's dedicated Sepolia relayer and synthetic test identities.

Moves 0.0005 free test ETH to infrastructure, and 12/5 faucet USDC to the creator
and solver. Customer identities receive zero ETH. --broadcast is explicit and
the three exact transactions have a combined 0.00001 test ETH gas ceiling.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import stat
import time
import fcntl
import os

from eth_account import Account
from eth_account.typed_transactions import TypedTransaction
from hexbytes import HexBytes
from eth_utils import to_checksum_address
from _shared.evm import address_word, uint_word, keccak_bytes, keccak256

spec = importlib.util.spec_from_file_location("executor", Path(__file__).with_name("execute-posting-sepolia-release.py"))
executor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(executor)
ADDRESSES = {"deployer": executor.DEPLOYER,
    "relayer": "0x8d76f56f9897ba7cec25c79057522b6ff88cf4d2",
    "creator": "0xf8fed04219cb7efe19620243613189b1ed158cfb",
    "solver": "0x1119daeb9951f96f0637038c70b2247f46e12ad4"}
USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e"
FEE_CAP = 10**13


def intents():
    plans = [{"role": "deployer", "nonce": 4, "to": ADDRESSES["relayer"], "value": 5*10**14,
              "data": "0x", "gas": 21000}]
    for i, (role, amount) in enumerate((("creator", 12_000_000), ("solver", 5_000_000))):
        data = "0x" + (keccak_bytes(b"transfer(address,uint256)")[:4] + address_word(ADDRESSES[role]) + uint_word(amount)).hex()
        plans.append({"role": "relayer", "nonce": i, "to": USDC, "value": 0, "data": data, "gas": 100000})
    return plans


def validate_saved(record, plan):
    raw = HexBytes(record["raw"])
    executor.require(keccak256(raw) == record["hash"], "Funding hash mismatch")
    executor.require(Account.recover_transaction(raw).lower() == ADDRESSES[plan["role"]], "Funding signer mismatch")
    tx = TypedTransaction.from_bytes(raw).as_dict()
    for key, expected in {"chainId": 84532, "type": 2, **{k: plan[k] for k in ("nonce", "value", "gas")}}.items():
        executor.require(tx[key] == expected, "Funding intent changed: " + key)
    executor.require(bytes(tx["to"]).hex() == plan["to"][2:] and bytes(tx["data"]).hex() == plan["data"][2:]
                     and not tx["accessList"], "Funding destination or calldata changed")
    executor.require(0 <= tx["maxPriorityFeePerGas"] <= tx["maxFeePerGas"] and tx["maxFeePerGas"] > 0,
                     "Funding fee caps invalid")
    return tx["maxFeePerGas"]


def reserve(rpc, plans, records, start, maximum):
    costs = [int(r["receipt"]["fee_wei"]) for r in records[:start]]
    for i in range(start, len(plans)):
        plan = plans[i]
        fee = validate_saved(records[i], plan) if i < len(records) else maximum
        costs.append(executor.quote(rpc, {"data": plan["data"], "gas_limit": plan["gas"]}, fee))
    executor.require(sum(costs) <= FEE_CAP, "Funding fees exceed the fixed test-only ceiling")
    return costs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--broadcast", action="store_true")
    args = parser.parse_args()
    rpc = executor.Rpc(args.broadcast)
    executor.load_bundle()
    deployment = json.loads((executor.PRIVATE / "deployment/journal.json").read_text())
    executor.require(deployment["complete"] and len(deployment["records"]) == 4, "Deployments must be reconciled first")
    head = executor.fresh_head(rpc)
    maximum = 2 * int(head["baseFeePerGas"],16) + int(rpc("eth_maxPriorityFeePerGas", []),16)
    plans = intents()
    bound = sum(reserve(rpc, plans, [], 0, maximum))
    print(json.dumps({"chain_id":84532,"maximum_total_fee_wei":str(bound),"infrastructure_test_eth_wei":str(5*10**14),
                      "creator_test_usdc":12,"solver_test_usdc":5,"broadcast":args.broadcast}),flush=True)
    if not args.broadcast:
        return
    key_path = executor.PRIVATE / "wallets.json"
    executor.require(stat.S_IMODE(key_path.stat().st_mode)==0o600,"Private test wallets require mode 0600")
    wallets = json.loads(key_path.read_text())
    for role, address in ADDRESSES.items():
        executor.require(wallets[role]["address"].lower()==address,"Unexpected test identity")
    path = executor.PRIVATE / "deployment/funding-journal.json"
    lock = os.open(executor.PRIVATE / "deployment/funding-lock",os.O_CREAT|os.O_RDWR,0o600)
    with os.fdopen(lock,"w") as locked:
        fcntl.flock(locked,fcntl.LOCK_EX|fcntl.LOCK_NB)
        journal = json.loads(path.read_text()) if path.exists() else {"plans":plans,"records":[]}
        executor.require(journal["plans"]==plans,"Funding plan changed")
        executor.require(len(journal["records"]) <= len(plans), "Unexpected funding records")
        for i,p in enumerate(plans):
            executor.fresh_head(rpc)
            costs = reserve(rpc, plans, journal["records"], i, maximum)
            signer = Account.from_key(wallets[p["role"]]["private_key"])
            executor.require(signer.address.lower()==ADDRESSES[p["role"]],"Wrong test signer")
            if i == len(journal["records"]):
                executor.require(int(rpc("eth_getTransactionCount",[signer.address,"pending"]),16)==p["nonce"],"Unexpected funding nonce")
                call = {"from":signer.address,"to":p["to"],"value":hex(p["value"]),"data":p["data"]}
                executor.require(int(rpc("eth_estimateGas",[call]),16)<=p["gas"],"Funding estimate exceeds fixed gas limit")
                executor.require(int(rpc("eth_getBalance",[signer.address,"latest"]),16)>=p["value"]+bound,"Test funding balance insufficient")
                tx = {k:v for k,v in p.items() if k!="role"}
                tx.update(to=to_checksum_address(p["to"]),chainId=84532,type=2,maxFeePerGas=maximum,
                          maxPriorityFeePerGas=int(rpc("eth_maxPriorityFeePerGas", []),16))
                signed=signer.sign_transaction(tx)
                journal["records"].append({"raw":"0x"+bytes(signed.raw_transaction).hex(),"hash":keccak256(signed.raw_transaction),"attempted":False,"receipt":None})
                executor.save(path,journal)
            record=journal["records"][i]
            validate_saved(record, p)
            receipt=rpc("eth_getTransactionReceipt",[record["hash"]])
            if receipt is None and not record["attempted"]:
                costs = reserve(rpc, plans, journal["records"], i, maximum)
                executor.require(int(rpc("eth_getTransactionCount",[signer.address,"pending"]),16)==p["nonce"], "Funding nonce moved")
                record["attempted"]=True
                record["reserved_wei"]=costs[i]
                executor.save(path,journal)
                try:
                    result=rpc("eth_sendRawTransaction",[record["raw"]])
                    executor.require(result.lower()==record["hash"],"Funding broadcast hash mismatch")
                except Exception:
                    raise RuntimeError("Funding broadcast uncertain; preserve journal and reconcile") from None
            deadline=time.monotonic()+120
            while receipt is None and time.monotonic()<deadline:
                time.sleep(2)
                receipt=rpc("eth_getTransactionReceipt",[record["hash"]])
            executor.require(receipt is not None and receipt["status"]=="0x1","Funding receipt pending or reverted; do not send new bytes")
            time.sleep(6)
            receipt=rpc("eth_getTransactionReceipt",[record["hash"]])
            executor.require(receipt is not None and receipt["status"]=="0x1", "Funding receipt disappeared")
            block=rpc("eth_getBlockByNumber",[receipt["blockNumber"],False])
            executor.require(block and block["hash"]==receipt["blockHash"],"Funding receipt is not canonical")
            executor.require(int(executor.fresh_head(rpc)["number"],16)-int(block["number"],16)>=3, "Funding needs three confirmations")
            executor.require(receipt["from"].lower()==signer.address.lower() and receipt["to"].lower()==p["to"],"Funding receipt identity differs")
            record["receipt"]={k:receipt[k] for k in ["transactionHash","blockNumber","blockHash","gasUsed","effectiveGasPrice","l1Fee"]}
            fee = int(receipt["gasUsed"],16)*int(receipt["effectiveGasPrice"],16)+int(receipt["l1Fee"],16)
            fee += executor.oracle_fee(rpc, "getOperatorFee(uint256)", int(receipt["gasUsed"],16),
                                       {"blockHash":block["hash"], "requireCanonical":True})
            executor.require(fee<=record["reserved_wei"], "Funding fee exceeded reservation")
            record["receipt"]["fee_wei"]=str(fee)
            executor.save(path,journal)
            print(json.dumps({"funding_step":i,"receipt":record["receipt"]}),flush=True)
        for role in ("creator","solver"):
            executor.require(rpc("eth_getBalance",[ADDRESSES[role],"latest"])=="0x0","Test customer ETH must stay zero")


if __name__ == "__main__":
    main()
