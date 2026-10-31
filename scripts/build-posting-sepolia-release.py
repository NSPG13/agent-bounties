#!/usr/bin/env python3
"""Build exact unsigned posting deployment evidence on a local Base Sepolia fork.

The remote RPC is read-only. All deployment/simulation writes are sent to an
Anvil process started by this script on loopback. No signing key is accepted.
The resulting bundle is a proposal, not proof of a live deployment.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import time
from urllib.parse import urlsplit
import urllib.request

from _shared.evm import address_word, artifact_hex, create_address, keccak256

ROOT = Path(__file__).resolve().parents[1]
CONTRACTS = ROOT / "contracts/base-escrow"
USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


def rpc(url, method, params):
    if urlsplit(url).hostname != "127.0.0.1" and method not in {
        "eth_chainId", "eth_getBlockByNumber", "eth_getTransactionCount", "eth_getCode",
    }:
        raise ValueError("Remote writes are forbidden by this preparation tool")
    body = {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
    req = urllib.request.Request(url, data=json.dumps(body).encode(), headers={
        "Content-Type": "application/json", "User-Agent": "agent-bounties-release/1"})
    with urllib.request.build_opener(NoRedirect()).open(req, timeout=30) as response:
        value = json.load(response)
    if value.get("id") != 1 or value.get("jsonrpc") != "2.0" or "error" in value:
        raise ValueError(f"RPC {method} failed its response contract")
    return value["result"]


def checked_artifact(name):
    artifact = json.loads((CONTRACTS / "out" / (name + ".sol") / (name + ".json")).read_text())
    metadata = artifact["metadata"]
    if metadata["compiler"]["version"] != "0.8.26+commit.8a97fa7a":
        raise ValueError("Unexpected Solidity compiler; rebuild with the reviewed version")
    if metadata["settings"]["optimizer"] != {"enabled": True, "runs": 200} or metadata["settings"]["evmVersion"] != "cancun":
        raise ValueError("Unreviewed compilation settings")
    for relative, source in metadata["sources"].items():
        path = (CONTRACTS / relative).resolve()
        if CONTRACTS not in path.parents or keccak256(path.read_bytes()) != source["keccak256"]:
            raise ValueError("Source changed since compilation: " + relative)
    return artifact, keccak256(artifact_hex(artifact["bytecode"], name)), metadata


def check_remote(url):
    parsed = urlsplit(url)
    if (parsed.scheme != "https" or not parsed.hostname or parsed.username
            or parsed.password or parsed.query or parsed.fragment):
        raise ValueError("Use a credential-free HTTPS Base Sepolia read endpoint")
    if rpc(url, "eth_chainId", []) != "0x14a34":
        raise ValueError("The remote endpoint is not Base Sepolia")
    block = rpc(url, "eth_getBlockByNumber", ["latest", False])
    now = int(time.time())
    if not block or abs(now - int(block["timestamp"], 16)) > 180:
        raise ValueError("Remote chain head is stale")
    return block


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deployer", required=True)
    parser.add_argument("--rpc", default="https://sepolia.base.org")
    parser.add_argument("--anvil", default="anvil")
    parser.add_argument("--port", type=int, default=18552)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    deployer = args.deployer.lower()
    address_word(deployer)
    if int(deployer, 16) == 0:
        raise ValueError("Deployer cannot be zero")
    block = check_remote(args.rpc)
    tag = {"blockHash": block["hash"], "requireCanonical": True}
    nonce = int(rpc(args.rpc, "eth_getTransactionCount", [deployer, tag]), 16)
    if nonce != 0 or rpc(args.rpc, "eth_getCode", [deployer, tag]) != "0x":
        raise ValueError("This isolated rehearsal requires a fresh dedicated EOA")
    names = ["AgentBountyFactory", "BoundedAgentWalletV2Factory", "ParticipantEligibilityRegistry", "CreatorReviewBountyFactoryV1"]
    predicted = {name: create_address(deployer, i) for i, name in enumerate(names)}
    token_code = rpc(args.rpc, "eth_getCode", [USDC, tag])
    if token_code == "0x":
        raise ValueError("Native Base Sepolia USDC is unavailable")
    artifacts = {name: checked_artifact(name) for name in names}
    for address in predicted.values():
        if rpc(args.rpc, "eth_getCode", [address, tag]) != "0x":
            raise ValueError("A proposed deployment address is occupied")
    constructors = [USDC, predicted["AgentBountyFactory"], deployer, USDC]
    local = f"http://127.0.0.1:{args.port}"
    transactions = []
    implementations = {}
    with socket.socket() as guard:
        guard.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        guard.bind(("127.0.0.1", args.port))
    with tempfile.TemporaryDirectory(prefix="posting-release-fork-") as tmp:
        log = open(Path(tmp) / "anvil.log", "w")
        process = subprocess.Popen([args.anvil, "--silent", "--host", "127.0.0.1", "--port", str(args.port),
            "--fork-url", args.rpc, "--fork-header", "User-Agent: agent-bounties-release/1",
            "--fork-block-number", str(int(block["number"], 16)), "--chain-id", "84532"],
            stdout=log, stderr=log, stdin=subprocess.DEVNULL)
        try:
            for _ in range(60):
                if process.poll() is not None:
                    raise RuntimeError("Isolated Anvil failed to start; no remote write was attempted")
                try:
                    if rpc(local, "eth_chainId", []) == "0x14a34":
                        break
                except (OSError, ValueError):
                    time.sleep(.25)
            else:
                raise RuntimeError("Isolated Anvil startup timed out")
            if "anvil" not in rpc(local, "web3_clientVersion", []).lower():
                raise ValueError("Local endpoint is not the expected Anvil instance")
            if rpc(local, "eth_getBlockByNumber", [block["number"], False])["hash"] != block["hash"]:
                raise ValueError("Local fork block differs from the reviewed remote block")
            rpc(local, "anvil_setBalance", [deployer, hex(10**18)])
            rpc(local, "anvil_impersonateAccount", [deployer])
            for i, (name, argument) in enumerate(zip(names, constructors)):
                artifact, artifact_hash, metadata = artifacts[name]
                data = artifact_hex(artifact["bytecode"], name) + address_word(argument)
                tx = {"from": deployer, "data": "0x" + data.hex(), "value": "0x0", "nonce": hex(i)}
                estimate = int(rpc(local, "eth_estimateGas", [tx]), 16)
                gas_limit = (estimate * 13 + 9) // 10
                if gas_limit > 12_000_000:
                    raise ValueError("Deployment exceeds the reviewed 12M gas ceiling")
                tx["gas"] = hex(gas_limit)
                sent = rpc(local, "eth_sendTransaction", [tx])
                receipt = None
                for _ in range(100):
                    receipt = rpc(local, "eth_getTransactionReceipt", [sent])
                    if receipt is not None:
                        break
                    time.sleep(.05)
                if receipt is None:
                    raise ValueError("Local deployment receipt was not observed")
                if receipt["status"] != "0x1" or receipt["contractAddress"].lower() != predicted[name]:
                    raise ValueError("Local deployment differs from the proposed address")
                code = rpc(local, "eth_getCode", [predicted[name], receipt["blockNumber"]])
                transactions.append({"name": name, "nonce": i, "chain_id": 84532,
                    "from": deployer, "to": None, "value_wei": "0", "data": tx["data"],
                    "expected_contract": predicted[name], "constructor_address": argument,
                    "creation_code_hash": keccak256(data), "artifact_creation_hash": artifact_hash,
                    "expected_runtime_code": code, "runtime_code_hash": keccak256(bytes.fromhex(code[2:])),
                    "gas_limit": gas_limit, "local_fork_gas_used": int(receipt["gasUsed"], 16),
                    "compiler": metadata["compiler"], "source_hashes": {p: v["keccak256"] for p, v in metadata["sources"].items()}})
                if name != "ParticipantEligibilityRegistry":
                    value = rpc(local, "eth_call", [{"to": predicted[name], "data": "0x5c60da1b"}, receipt["blockNumber"]])
                    implementation = "0x" + value[-40:]
                    if implementation != create_address(predicted[name], 1):
                        raise ValueError("Unexpected implementation address")
                    runtime = rpc(local, "eth_getCode", [implementation, receipt["blockNumber"]])
                    implementations[name] = {"address": implementation, "runtime_code": runtime,
                        "runtime_code_hash": keccak256(bytes.fromhex(runtime[2:]))}
            # The local simulator never writes to or changes the remote chain.
            if rpc(args.rpc, "eth_getTransactionCount", [deployer, "latest"]) != "0x0":
                raise ValueError("Remote deployer nonce changed during preparation")
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            log.close()
    bundle = {"schema": "agent-bounties/posting-sepolia-release-v1", "network": "base-sepolia",
        "chain_id": 84532, "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "deployer": deployer, "native_usdc": USDC, "preflight_block": {k: block[k] for k in ("number", "hash", "timestamp")},
        "preflight_deployer_nonce": 0, "remote_transactions_submitted": False,
        "approval_required": "Exact-bytecode testnet contract deployment; action-time approval before signing.",
        "transactions": transactions, "implementations": implementations,
        "not_proven": ["live contract deployment", "hosted sponsored lifecycle", "production activation"]}
    encoded = (json.dumps(bundle, indent=2) + "\n").encode()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(encoded)
    print(json.dumps({"bundle": str(args.output), "sha256": hashlib.sha256(encoded).hexdigest(),
        "deployments": [{"name": t["name"], "address": t["expected_contract"], "gas_limit": t["gas_limit"]} for t in transactions],
        "remote_transactions_submitted": False}))


if __name__ == "__main__":
    main()
