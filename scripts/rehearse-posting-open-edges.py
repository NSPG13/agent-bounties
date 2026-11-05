#!/usr/bin/env python3
"""Replay the exact hosted edge-case plan on an owned local Sepolia fork only."""
import argparse
from datetime import datetime, timezone
import importlib.util
import json
from pathlib import Path
import socket
import stat
import subprocess
import tempfile
import time
from eth_account import Account
from eth_abi import encode
from web3 import Web3
from _shared.evm import keccak_bytes

spec = importlib.util.spec_from_file_location("edges", Path(__file__).with_name("check-posting-hosted-open-edges.py"))
e = importlib.util.module_from_spec(spec)
spec.loader.exec_module(e)
h, ex = e.h, e.ex


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--anvil", required=True)
    p.add_argument("--output", type=Path, required=True)
    args = p.parse_args()
    ex.load_bundle()
    remote = ex.Rpc()
    head = ex.fresh_head(remote)
    with socket.socket() as guard:
        guard.bind(("127.0.0.1", 0))
        port = guard.getsockname()[1]
    local = "http://127.0.0.1:" + str(port)
    with tempfile.TemporaryDirectory(prefix="posting-edges-fork-") as tmp:
        with open(Path(tmp) / "anvil.log", "w") as log:
            proc = subprocess.Popen([args.anvil, "--silent", "--host", "127.0.0.1", "--port", str(port),
                "--fork-url", "https://sepolia.base.org", "--fork-header", "User-Agent: agent-bounties-release/1",
                "--fork-block-number", str(int(head["number"], 16)), "--chain-id", "84532"], stdout=log, stderr=log)
            try:
                w3 = Web3(Web3.HTTPProvider(local, request_kwargs={"timeout": 30}))
                for _ in range(100):
                    ex.require(proc.poll() is None, "Owned Anvil exited")
                    if w3.is_connected():
                        break
                    time.sleep(.2)
                ex.require(w3.is_connected() and w3.eth.chain_id == 84532, "Wrong local chain")
                ex.require(w3.eth.get_block(int(head["number"], 16)).hash.hex() == head["hash"][2:], "Wrong fork hash")
                def local_rpc(method, params):
                    reply = w3.provider.make_request(method, params)
                    ex.require("error" not in reply, "Local RPC failed: " + method)
                    return reply["result"]
                sender = Web3.to_checksum_address(h.funding.ADDRESSES["relayer"])
                local_rpc("anvil_impersonateAccount", [sender])
                # Every actual value transfer below uses the copied native test-USDC state.
                key_path = ex.PRIVATE / "wallets.json"
                ex.require(stat.S_IMODE(key_path.stat().st_mode) == 0o600, "Private test keys")
                wallets = json.loads(key_path.read_text())
                accounts = {w["address"].lower(): Account.from_key(w["private_key"]) for w in wallets.values()}
                abi = lambda name: json.loads((ex.ROOT / "contracts/base-escrow/out" / (name + ".sol") / (name + ".json")).read_text())["abi"]
                factory = w3.eth.contract(address=Web3.to_checksum_address(h.OPEN_RELEASE["factory"]), abi=abi("CreatorReviewBountyFactoryV1"))
                token = Web3.to_checksum_address(h.funding.USDC)
                domain = w3.eth.call({"to": token, "data": "0x" + keccak_bytes(b"DOMAIN_SEPARATOR()")[:4].hex()})
                def balance(address):
                    data = keccak_bytes(b"balanceOf(address)")[:4] + bytes.fromhex(address[2:]).rjust(32, b"\0")
                    return int.from_bytes(w3.eth.call({"to": token, "data": data}), "big")
                initial = {r: balance(a) for r, a in h.funding.ADDRESSES.items()}
                plan = e.plans(w3.eth.get_block("latest").timestamp)
                receipts, bounties = [], {}
                premature_rejections = []
                for item in plan:
                    req, name = item["request"], item["name"]
                    t, a = req["original_terms"], req["action"]
                    kind = a["kind"]
                    address = Web3.to_checksum_address(h.predict(req))
                    bounty = w3.eth.contract(address=address, abi=abi("CreatorReviewBountyV1"))
                    def authorize(owner, amount, nonce):
                        struct = keccak_bytes(encode(["bytes32", "address", "address", "uint256", "uint256", "uint256", "bytes32"],
                            [keccak_bytes(b"ReceiveWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)"),
                             owner, address, amount, 0, req["valid_before"], bytes.fromhex(nonce[2:])]))
                        signature = bytes(accounts[owner.lower()].unsafe_sign_hash(keccak_bytes(b"\x19\x01" + domain + struct)).signature)
                        return (0, req["valid_before"], bytes.fromhex(nonce[2:]), signature)
                    if item["phase"] == "finish" and not premature_rejections:
                        for fn in (bounty.functions.refundBond(1), bounty.functions.expire()):
                            rejected = False
                            try:
                                fn.call({"from": sender})
                            except Exception:
                                rejected = True
                            ex.require(rejected, "Unelapsed deadline accepted")
                            premature_rejections.append(True)
                        local_rpc("evm_setNextBlockTimestamp", [t["submission_deadline"] + 3601])
                        local_rpc("evm_mine", [])
                    if kind == "create":
                        config = (bytes.fromhex(t["bounty_id"][2:]), Web3.to_checksum_address(t["creator"]),
                            t["solver_reward_micro_usdc"], t["review_reward_micro_usdc"], bytes.fromhex(h.hash_json(t)[2:]),
                            bytes.fromhex(h.hash_json(t["acceptance_criteria"])[2:]), bytes.fromhex(h.hash_json(h.SCHEMA)[2:]),
                            t["submission_deadline"], t["review_window_seconds"], t["max_pending_entries"])
                        salt = bytes.fromhex(req["user_salt"][2:])
                        ex.require(factory.functions.predict(config, salt).call() == address, "Plan prediction differs")
                        fn = factory.functions.createWithAuthorization(config, salt, authorize(t["creator"], 600_000, a["token_nonce"]))
                        bounties[name] = address
                    elif kind == "submit":
                        submission = (Web3.to_checksum_address(a["solver"]), bytes.fromhex(a["submission_hash"][2:]),
                            bytes.fromhex(a["evidence_hash"][2:]), bytes.fromhex(h.hash_json(t)[2:]), 1, req["valid_before"])
                        auth = authorize(a["solver"], 100_000, a["token_nonce"])
                        digest = bounty.functions.submissionDigest(submission, auth[2]).call()
                        fn = bounty.functions.submitAuthorized(submission, auth, bytes(accounts[a["solver"].lower()].unsafe_sign_hash(digest).signature))
                    elif kind == "review":
                        response = bytes.fromhex(a["response_hash"][2:])
                        digest = bounty.functions.reviewDigest(a["entry"], True, response, req["valid_before"]).call()
                        fn = bounty.functions.reviewAuthorized(a["entry"], True, response, req["valid_before"], bytes(accounts[t["creator"].lower()].unsafe_sign_hash(digest).signature))
                    elif kind == "refund_bond":
                        fn = bounty.functions.refundBond(a["entry"])
                    elif kind == "expire":
                        fn = bounty.functions.expire()
                    elif kind == "refund_principal":
                        fn = bounty.functions.refundPrincipal()
                    else:
                        raise ValueError("Unexpected planned action")
                    tx = fn.transact({"from": sender, "gas": 1_500_000,
                                      "maxFeePerGas": 100_000_000, "maxPriorityFeePerGas": 1_000_000})
                    receipt = w3.eth.wait_for_transaction_receipt(tx)
                    ex.require(receipt.status == 1, "Local edge reverted: " + name)
                    receipts.append({"action": name, "local_gas_used": receipt.gasUsed})
                final = {r: balance(a) for r, a in h.funding.ADDRESSES.items()}
                for role, delta in (("relayer", -500_000), ("creator", 500_000), ("solver", 0)):
                    ex.require(final[role] == initial[role] + delta, "Principal conservation failed")
                ex.require(all(w3.eth.get_balance(Web3.to_checksum_address(h.funding.ADDRESSES[r])) == 0 for r in ("creator", "solver")), "Entrant ETH changed")
                ex.require(all(balance(a) == 0 for a in bounties.values()), "Escrow not fully disbursed")
                output = {"observed_at": datetime.now(timezone.utc).isoformat(), "network": "local-base-sepolia-fork",
                    "remote_source_block": head["number"], "actions": receipts, "premature_refund_and_expiry_rejected": all(premature_rejections),
                    "initial_test_usdc_micro": initial, "final_test_usdc_micro": final, "both_entrants_eth_wei": "0",
                    "both_escrows_empty": True, "remote_transactions_sent": 0, "local_clock_advanced_only": True}
                args.output.write_text(json.dumps(output, indent=2) + "\n")
                print(json.dumps({"local_actions": len(receipts), "both_escrows_empty": True, "remote_transactions_sent": 0}))
            finally:
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()


if __name__ == "__main__":
    main()
