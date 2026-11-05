#!/usr/bin/env python3
"""Prepare one exact ERC-1271 test-account deployment; remote RPC is read-only.

Only an owned loopback Anvil instance deploys the fixture. Dedicated test keys
sign a non-transaction proof locally. No production keys, remote broadcasts or
ledger reservations are used. The resulting bundle requires separate approval.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
from pathlib import Path
import socket
import stat
import subprocess
import tempfile
import time
from eth_account import Account
from web3 import Web3
from _shared.evm import address_word, artifact_hex, create_address, keccak256, keccak_bytes

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("executor", Path(__file__).with_name("execute-posting-sepolia-release.py"))
ex = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ex)
buildspec = importlib.util.spec_from_file_location("builder", Path(__file__).with_name("build-posting-sepolia-release.py"))
builder = importlib.util.module_from_spec(buildspec)
buildspec.loader.exec_module(builder)
OWNER = "0x1119daeb9951f96f0637038c70b2247f46e12ad4"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--anvil", required=True)
    parser.add_argument("--solc-module", type=Path, required=True,
                        help="Official solc@0.8.26 module installed in the private tool directory")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    remote = ex.Rpc()
    head = ex.fresh_head(remote)
    nonce = int(remote("eth_getTransactionCount", [ex.DEPLOYER, "pending"]), 16)
    predicted = create_address(ex.DEPLOYER, nonce)
    ex.require(remote("eth_getCode", [predicted, "latest"]) == "0x", "Account address occupied")
    source_name = "test/PostingSepoliaAccountFixture.sol"
    compiled = subprocess.run(["node", "-e", r"""
const fs = require('fs');
const solc = require(process.argv[1]);
if (!solc.version().startsWith('0.8.26+commit.8a97fa7a')) throw Error('Wrong Solidity compiler');
const name = 'test/PostingSepoliaAccountFixture.sol';
const input = {language:'Solidity',sources:{[name]:{content:fs.readFileSync(process.argv[2],'utf8')}},
 settings:{optimizer:{enabled:true,runs:200},evmVersion:'cancun',
 outputSelection:{'*':{'*':['abi','evm.bytecode','evm.deployedBytecode','metadata']}}}};
const out = JSON.parse(solc.compile(JSON.stringify(input)));
if ((out.errors||[]).some(e=>e.severity==='error')) throw Error('Fixture compilation failed');
const a = out.contracts[name].PostingSepoliaAccountFixture;
process.stdout.write(JSON.stringify({abi:a.abi,bytecode:a.evm.bytecode,
 deployedBytecode:a.evm.deployedBytecode,metadata:JSON.parse(a.metadata)}));
""", str(args.solc_module.resolve()), str(ROOT / "contracts/base-escrow" / source_name)],
        capture_output=True, text=True, check=True, timeout=30)
    artifact = json.loads(compiled.stdout)
    meta = artifact["metadata"]
    ex.require(meta["compiler"]["version"] == "0.8.26+commit.8a97fa7a" and
               meta["settings"]["optimizer"] == {"enabled": True, "runs": 200} and
               meta["settings"]["evmVersion"] == "cancun", "Unreviewed compiler settings")
    sources = {}
    for relative, source in meta["sources"].items():
        path = (ROOT / "contracts/base-escrow" / relative).resolve()
        ex.require((ROOT / "contracts/base-escrow") in path.parents and keccak256(path.read_bytes()) == source["keccak256"], "Source differs from artifact")
        sources[relative] = source["keccak256"]
    data = artifact_hex(artifact["bytecode"], "PostingSepoliaAccountFixture") + address_word(OWNER)
    with socket.socket() as guard:
        guard.bind(("127.0.0.1", 0))
        port = guard.getsockname()[1]
    local = "http://127.0.0.1:" + str(port)
    with tempfile.TemporaryDirectory(prefix="posting-account-review-") as tmp:
        with open(Path(tmp) / "anvil.log", "w") as log:
            process = subprocess.Popen([args.anvil, "--silent", "--host", "127.0.0.1", "--port", str(port),
                "--fork-url", "https://sepolia.base.org", "--fork-header", "User-Agent: agent-bounties-release/1",
                "--fork-block-number", str(int(head["number"], 16)), "--chain-id", "84532"], stdout=log, stderr=log)
            try:
                w3 = Web3(Web3.HTTPProvider(local, request_kwargs={"timeout": 30}))
                for _ in range(100):
                    ex.require(process.poll() is None, "Owned Anvil exited")
                    if w3.is_connected():
                        break
                    time.sleep(.2)
                ex.require(w3.is_connected() and w3.eth.chain_id == 84532, "Wrong local fork")
                ex.require(w3.eth.get_block(int(head["number"], 16)).hash.hex() == head["hash"][2:], "Fork hash mismatch")
                builder.rpc(local, "anvil_impersonateAccount", [ex.DEPLOYER])
                sender = Web3.to_checksum_address(ex.DEPLOYER)
                tx = {"from": sender, "nonce": nonce, "value": 0, "data": "0x" + data.hex()}
                gas = (w3.eth.estimate_gas(tx) * 13 + 9) // 10
                ex.require(gas <= 500_000, "Fixture gas ceiling exceeded")
                tx["gas"] = gas
                receipt = w3.eth.wait_for_transaction_receipt(w3.eth.send_transaction(tx))
                ex.require(receipt.status == 1 and receipt.contractAddress.lower() == predicted, "Local deployment mismatch")
                code = bytes(w3.eth.get_code(receipt.contractAddress))
                expected = bytearray(artifact_hex(artifact["deployedBytecode"], "PostingSepoliaAccountFixture"))
                for refs in artifact["deployedBytecode"]["immutableReferences"].values():
                    for ref in refs:
                        ex.require(ref["length"] == 32, "Unexpected immutable width")
                        expected[ref["start"]:ref["start"] + 32] = address_word(OWNER)
                ex.require(code == bytes(expected), "Local runtime differs from exact reviewed bytes")
                contract = w3.eth.contract(address=receipt.contractAddress, abi=artifact["abi"])
                ex.require(contract.functions.owner().call().lower() == OWNER, "Wrong immutable owner")
                keys = ex.PRIVATE / "wallets.json"
                ex.require(stat.S_IMODE(keys.stat().st_mode) == 0o600, "Dedicated key permissions")
                owner = Account.from_key(json.loads(keys.read_text())["solver"]["private_key"])
                ex.require(owner.address.lower() == OWNER, "Wrong dedicated test owner")
                digest = keccak_bytes(b"local-only Sepolia test account proof; not transaction authority")
                signature = bytes(owner.unsafe_sign_hash(digest).signature)
                for encoding in (signature, b"\x7f" + signature):
                    ex.require(contract.functions.isValidSignature(digest, encoding).call() == bytes.fromhex("1626ba7e"), "Compatible signature rejected")
                ex.require(contract.functions.isValidSignature(keccak_bytes(b"altered proof"), signature).call() != bytes.fromhex("1626ba7e"), "Altered digest accepted")
                ex.require(w3.eth.get_balance(receipt.contractAddress) == 0, "Unexpected account ETH")
                result = {"schema": "agent-bounties/test-account-deployment-review-v1",
                    "prepared_at": datetime.now(timezone.utc).isoformat(), "chain_id": 84532, "network": "base-sepolia",
                    "source_block": {"number": int(head["number"], 16), "hash": head["hash"]},
                    "transaction": {"from": ex.DEPLOYER, "to": None, "nonce": nonce, "value_wei": "0",
                        "data": "0x" + data.hex(), "gas_limit": gas, "expected_contract": predicted,
                        "immutable_owner": OWNER, "creation_code_hash": keccak256(data),
                        "expected_runtime_code": "0x" + code.hex(), "runtime_code_hash": keccak256(code),
                        "maximum_total_fee_wei": "10000000000000"},
                    "local_only_evidence": {"gas_used": receipt.gasUsed, "compatible_65_byte_signature": True,
                        "compatible_66_byte_signature": True, "altered_digest_rejected": True, "account_eth_wei": "0"},
                    "compiler": meta["compiler"], "source_hashes": sources,
                    "budget": "Reserve inside the existing 1-USDC UTC-day staging allowance before any remote signing; no rollover.",
                    "approval_required": True, "remote_transactions_sent": 0, "production_changed": False}
                args.output.write_text(json.dumps(result, indent=2) + "\n")
                print(json.dumps({"bundle": str(args.output), "sha256": hashlib.sha256(args.output.read_bytes()).hexdigest(),
                                  "expected_contract": predicted, "gas_limit": gas, "remote_transactions_sent": 0}))
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


if __name__ == "__main__":
    main()
