#!/usr/bin/env python3
"""Execute and reconcile one autonomous bounded-wallet bounty loop on Base."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import time
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

from Crypto.Hash import keccak
from eth_abi import encode as abi_encode


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BUNDLE = ROOT / "deployments" / "bounded-wallet-base-activation.json"
KEY_DIR = ROOT / "target" / "bounded-wallet-activation" / "keys"
EVENT_SIGNATURE = (
    "BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,"
    "bytes32,bytes32,bytes32,bytes32)"
)


def executable(name: str) -> str:
    found = shutil.which(name)
    if found:
        return found
    candidate = ROOT / ".tools" / "foundry" / f"{name}.exe"
    if candidate.exists():
        return str(candidate)
    raise SystemExit(f"{name} is required; install Foundry or use .tools/foundry")


CAST = executable("cast")


def run(command: list[str], input_text: str | None = None) -> str:
    result = subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        input=input_text,
    )
    return result.stdout.strip()


def cast(*args: str, input_text: str | None = None) -> str:
    return run([CAST, *args], input_text=input_text)


def rpc(url: str, method: str, params: list, request_id: int = 1):
    payload = json.dumps(
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
    ).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=payload,
        headers={
            "content-type": "application/json",
            "user-agent": "agent-bounties-bounded-wallet-rehearsal/1",
        },
    )
    with urllib.request.urlopen(request, timeout=45) as response:
        body = json.loads(response.read().decode("utf-8"))
    if "error" in body:
        raise RuntimeError(f"RPC {method} failed: {body['error']}")
    return body["result"]


def rpc_int(url: str, method: str, params: list) -> int:
    return int(rpc(url, method, params), 16)


def wait_receipt(url: str, transaction_hash: str, timeout: int = 240) -> dict:
    deadline = time.time() + timeout
    while time.time() < deadline:
        receipt = rpc(url, "eth_getTransactionReceipt", [transaction_hash])
        if receipt is not None:
            if int(receipt["status"], 16) != 1:
                raise RuntimeError(f"transaction reverted: {transaction_hash}")
            return receipt
        time.sleep(1.5)
    raise TimeoutError(f"transaction receipt timed out: {transaction_hash}")


def keccak_hex(value: bytes) -> str:
    digest = keccak.new(digest_bits=256)
    digest.update(value)
    return f"0x{digest.hexdigest()}"


def text_hash(value: str) -> str:
    return keccak_hex(value.encode("utf-8"))


def call(rpc_url: str, contract: str, signature: str, *args: str) -> str:
    return cast("call", contract, signature, *args, "--rpc-url", rpc_url).splitlines()[0]


def call_int(rpc_url: str, contract: str, signature: str, *args: str) -> int:
    return int(call(rpc_url, contract, signature, *args).split()[0], 0)


def token_balance(rpc_url: str, token: str, address: str) -> int:
    return call_int(rpc_url, token, "balanceOf(address)(uint256)", address)


def wallet_address(role: str) -> str:
    return cast(
        "wallet",
        "address",
        "--keystore",
        str(KEY_DIR / f"{role}.keystore"),
        "--password-file",
        str(KEY_DIR / f"{role}.password"),
    ).lower()


def send(
    rpc_url: str,
    role: str,
    contract: str,
    signature: str,
    *args: str,
) -> tuple[str, dict]:
    transaction_hash = cast(
        "send",
        contract,
        signature,
        *args,
        "--rpc-url",
        rpc_url,
        "--keystore",
        str(KEY_DIR / f"{role}.keystore"),
        "--password-file",
        str(KEY_DIR / f"{role}.password"),
        "--async",
    ).splitlines()[-1]
    return transaction_hash, wait_receipt(rpc_url, transaction_hash)


def sign_digest(role: str, digest: str) -> str:
    return cast(
        "wallet",
        "sign",
        "--no-hash",
        digest,
        "--keystore",
        str(KEY_DIR / f"{role}.keystore"),
        "--password-file",
        str(KEY_DIR / f"{role}.password"),
    ).splitlines()[-1]


def encoded_payload(signature: str, *args: str) -> str:
    return cast("abi-encode", signature, *args)


def relay_action(
    rpc_url: str,
    wallet: str,
    delegate_role: str,
    action: int,
    payload: str,
) -> tuple[str, dict, dict]:
    nonce = call_int(rpc_url, wallet, "delegateNonce()(uint256)")
    policy_version = call_int(rpc_url, wallet, "policyVersion()(uint64)")
    deadline = int(time.time()) + 900
    payload_hash = keccak_hex(bytes.fromhex(payload[2:]))
    digest = call(
        rpc_url,
        wallet,
        "actionDigest(uint8,bytes32,uint256,uint256)(bytes32)",
        str(action),
        payload_hash,
        str(nonce),
        str(deadline),
    )
    signature = sign_digest(delegate_role, digest)
    transaction_hash, receipt = send(
        rpc_url,
        "relayer",
        wallet,
        "executeWithSignature(uint8,bytes,uint256,uint256,bytes)",
        str(action),
        payload,
        str(nonce),
        str(deadline),
        signature,
    )
    return transaction_hash, receipt, {
        "action": action,
        "delegate": wallet_address(delegate_role),
        "nonce": nonce,
        "policy_version": policy_version,
        "deadline": deadline,
        "payload_hash": payload_hash,
        "digest": digest,
    }


def mine_proof(
    difficulty: int,
    bounty_id: str,
    round_number: int,
    solver: str,
    submission_hash: str,
    evidence_hash: str,
    policy_hash: str,
) -> tuple[int, str, str]:
    values = [
        bytes.fromhex(bounty_id[2:]),
        round_number,
        solver,
        bytes.fromhex(submission_hash[2:]),
        bytes.fromhex(evidence_hash[2:]),
        bytes.fromhex(policy_hash[2:]),
    ]
    types = ["bytes32", "uint64", "address", "bytes32", "bytes32", "bytes32", "uint256"]
    for nonce in range(0, 2**32):
        response_hash = keccak_hex(abi_encode(types, [*values, nonce]))
        if int(response_hash, 16) >> (256 - difficulty) == 0:
            return nonce, f"0x{nonce:064x}", response_hash
    raise RuntimeError("proof nonce was not found in uint32 range")


def event_logs(
    rpc_url: str, address: str, topic: str, from_block: int
) -> list[dict]:
    return rpc(
        rpc_url,
        "eth_getLogs",
        [
            {
                "address": address,
                "fromBlock": hex(from_block),
                "toBlock": "latest",
                "topics": [topic],
            }
        ],
    )


def one_action_log(
    rpc_url: str, wallet: str, action: int, from_block: int
) -> dict:
    topic = text_hash("AgentActionExecuted(uint8,address,address,uint256,bytes32)")
    action_topic = f"0x{action:064x}"
    matches = [
        item
        for item in event_logs(rpc_url, wallet, topic, from_block)
        if item["topics"][1].lower() == action_topic
    ]
    if len(matches) != 1:
        raise RuntimeError(f"expected one action {action} log, observed {len(matches)}")
    return matches[0]


def require_code(rpc_url: str, address: str, label: str, timeout: int = 30) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if rpc(rpc_url, "eth_getCode", [address, "latest"]) != "0x":
            return
        time.sleep(1)
    raise RuntimeError(f"{label} has no code: {address}")


def wait_call_int(
    rpc_url: str,
    contract: str,
    signature: str,
    expected: int,
    *args: str,
    timeout: int = 30,
) -> int:
    deadline = time.time() + timeout
    observed = call_int(rpc_url, contract, signature, *args)
    while observed != expected and time.time() < deadline:
        time.sleep(1)
        observed = call_int(rpc_url, contract, signature, *args)
    return observed


def wait_token_balance(
    rpc_url: str, token: str, address: str, expected: int, timeout: int = 30
) -> int:
    return wait_call_int(
        rpc_url, token, "balanceOf(address)(uint256)", expected, address, timeout=timeout
    )


def safe_block_attestation(
    rpc_url: str,
    minimum_block: int,
    addresses: dict[str, str],
    timeout: int = 600,
) -> dict:
    deadline = time.time() + timeout
    while time.time() < deadline:
        block = rpc(rpc_url, "eth_getBlockByNumber", ["safe", False])
        if block and int(block["number"], 16) >= minimum_block:
            exact = block["number"]
            code_hashes = {
                label: keccak_hex(
                    bytes.fromhex(rpc(rpc_url, "eth_getCode", [address, exact])[2:])
                )
                for label, address in addresses.items()
            }
            return {
                "number": int(exact, 16),
                "hash": block["hash"].lower(),
                "timestamp": int(block["timestamp"], 16),
                "code_hashes": code_hashes,
                "code_observation_method": "eth_getCode at the exact safe block, locally keccak256 hashed",
            }
        time.sleep(5)
    raise TimeoutError("Base safe block did not include the settled transaction")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("network", choices=("base-sepolia", "base-mainnet"))
    parser.add_argument("--bundle", type=Path, default=DEFAULT_BUNDLE)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--confirm-mainnet-canary", action="store_true")
    parser.add_argument("--resume-bounty", help="Resume one already-created claimable rehearsal bounty.")
    parser.add_argument("--creation-transaction", help="Confirmed creation transaction for resumed evidence.")
    args = parser.parse_args()
    bundle = json.loads(args.bundle.read_text(encoding="utf-8"))
    network = bundle["networks"][args.network]
    if args.network == "base-mainnet" and not args.confirm_mainnet_canary:
        raise SystemExit("mainnet requires --confirm-mainnet-canary")
    if int(network["pilot"]["total_usdc_funding"]) != 400_000:
        raise SystemExit("pilot total must remain exactly 0.40 USDC")
    for role in ("delegate-a", "delegate-b", "relayer"):
        if not (KEY_DIR / f"{role}.keystore").exists() or not (KEY_DIR / f"{role}.password").exists():
            raise SystemExit(f"missing encrypted local signer: {role}")
    if wallet_address("delegate-a") != network["pilot"]["wallets"][0]["delegate"]:
        raise SystemExit("creator delegate keystore does not match the activation bundle")
    if wallet_address("delegate-b") != network["pilot"]["wallets"][1]["delegate"]:
        raise SystemExit("solver delegate keystore does not match the activation bundle")
    if wallet_address("relayer") != network["pilot"]["relayer"]:
        raise SystemExit("relayer keystore does not match the activation bundle")

    rpc_url = network["rpc_url"]
    creator_wallet = network["pilot"]["wallets"][0]["expected_contract"]
    solver_wallet = network["pilot"]["wallets"][1]["expected_contract"]
    relayer = network["pilot"]["relayer"]
    usdc = network["native_usdc"]
    for label, address in {
        "bounty factory": network["bounty_factory"],
        "verifier module": network["verifier_module"],
        "wallet factory": network["wallet_factory"],
        "creator wallet": creator_wallet,
        "solver wallet": solver_wallet,
    }.items():
        require_code(rpc_url, address, label)
    if rpc_int(rpc_url, "eth_getBalance", [relayer, "latest"]) == 0:
        raise SystemExit("relayer has no ETH for gas")

    now = int(time.time())
    terms_document = {
        "schema": "agent-bounties/autonomous-terms-v1",
        "network": args.network,
        "goal": "Complete the deterministic bounded-wallet funded-loop rehearsal.",
        "acceptance": "Submit the precommitted artifact and evidence hashes, then satisfy the exact leading-zero verifier.",
        "solver_reward": "200000",
        "verifier_reward": "100000",
    }
    terms_hash = text_hash(json.dumps(terms_document, sort_keys=True, separators=(",", ":")))
    policy_hash = text_hash("bounded-wallet-rehearsal:deterministic-leading-zero:v1")
    criteria_hash = text_hash(terms_document["acceptance"])
    benchmark_hash = text_hash(f"leading-zero-bits:{network['verifier_difficulty_bits']}")
    evidence_schema_hash = text_hash("artifact_hash:bytes32,evidence_hash:bytes32")
    transactions = {}
    already_settled = False
    if args.resume_bounty:
        predicted_bounty = args.resume_bounty.lower()
        require_code(rpc_url, predicted_bounty, "resumed bounty")
        expected_calls = {
            "factory": ("factory()(address)", network["bounty_factory"]),
            "creator": ("creator()(address)", creator_wallet),
            "terms hash": ("termsHash()(bytes32)", terms_hash),
            "policy hash": ("policyHash()(bytes32)", policy_hash),
            "criteria hash": ("acceptanceCriteriaHash()(bytes32)", criteria_hash),
            "benchmark hash": ("benchmarkHash()(bytes32)", benchmark_hash),
            "evidence schema hash": ("evidenceSchemaHash()(bytes32)", evidence_schema_hash),
        }
        for label, (signature, expected) in expected_calls.items():
            observed = call(rpc_url, predicted_bounty, signature).split()[0].lower()
            if observed != expected.lower():
                raise SystemExit(f"resumed bounty {label} does not match the rehearsal")
        observed_status = call_int(rpc_url, predicted_bounty, "bountyStatus()(uint8)")
        if observed_status not in (1, 4):
            raise SystemExit("resumed bounty must be claimable or already settled")
        already_settled = observed_status == 4
        creation_receipt = (
            wait_receipt(rpc_url, args.creation_transaction)
            if args.creation_transaction
            else None
        )
        transactions["create"] = {
            "hash": args.creation_transaction,
            "receipt": creation_receipt,
            "resumed": True,
        }
        bounty_id = call(rpc_url, predicted_bounty, "bountyId()(bytes32)")
        if already_settled:
            submission_hash = call(rpc_url, predicted_bounty, "submissionHash()(bytes32)")
            evidence_hash = call(rpc_url, predicted_bounty, "evidenceHash()(bytes32)")
        else:
            submission_hash = text_hash(f"bounded-wallet-rehearsal-artifact:{bounty_id}")
            evidence_hash = text_hash(f"bounded-wallet-rehearsal-evidence:{bounty_id}")
    else:
        creation_nonce = text_hash(f"{args.network}:{bundle['source_revision']}:{now}")
        submission_hash = text_hash(f"bounded-wallet-rehearsal-artifact:{creation_nonce}")
        evidence_hash = text_hash(f"bounded-wallet-rehearsal-evidence:{creation_nonce}")
        params = (
            f"({network['pilot']['solver_reward']},{network['pilot']['verifier_reward']},"
            f"{terms_hash},{policy_hash},{criteria_hash},{benchmark_hash},{evidence_schema_hash},"
            f"{now + 604800},86400,86400,0,{network['verifier_module']},{relayer},1)"
        )
        tuple_type = (
            "(uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,"
            "uint64,uint8,address,address,uint8)"
        )
        predicted_bounty = call(
            rpc_url,
            network["bounty_factory"],
            f"predictBountyAddress(address,{tuple_type},address[],bytes32)(address)",
            creator_wallet,
            params,
            "[]",
            creation_nonce,
        ).lower()
        create_payload = encoded_payload(
            f"f({tuple_type},address[],uint256,bytes32)",
            params,
            "[]",
            "300000",
            creation_nonce,
        )
        balances = {
            "creator_wallet": token_balance(rpc_url, usdc, creator_wallet),
            "solver_wallet": token_balance(rpc_url, usdc, solver_wallet),
        }
        if balances["creator_wallet"] < 300_000 or balances["solver_wallet"] < 100_000:
            raise SystemExit("pilot wallets are not funded with 0.30 and 0.10 USDC")
        transaction_hash, receipt, authorization = relay_action(
            rpc_url, creator_wallet, "delegate-a", 0, create_payload
        )
        transactions["create"] = {
            "hash": transaction_hash,
            "receipt": receipt,
            "authorization": authorization,
        }
        require_code(rpc_url, predicted_bounty, "created bounty")
        if wait_call_int(rpc_url, predicted_bounty, "bountyStatus()(uint8)", 1) != 1:
            raise RuntimeError("created bounty is not fully funded and claimable")

    balances_before = {
        "creator_wallet": token_balance(rpc_url, usdc, creator_wallet),
        "solver_wallet": token_balance(rpc_url, usdc, solver_wallet),
        "relayer": token_balance(rpc_url, usdc, relayer),
        "bounty": token_balance(rpc_url, usdc, predicted_bounty),
    }

    settled_topic = text_hash(EVENT_SIGNATURE)
    bounty_id = call(rpc_url, predicted_bounty, "bountyId()(bytes32)")
    round_number = call_int(rpc_url, predicted_bounty, "round()(uint64)")
    if already_settled:
        creation_receipt = transactions["create"]["receipt"]
        latest = rpc_int(rpc_url, "eth_blockNumber", [])
        from_block = (
            int(creation_receipt["blockNumber"], 16)
            if creation_receipt
            else max(0, latest - 5_000)
        )
        claim_log = one_action_log(rpc_url, solver_wallet, 2, from_block)
        submit_log = one_action_log(rpc_url, solver_wallet, 3, from_block)
        settled_logs = event_logs(rpc_url, predicted_bounty, settled_topic, from_block)
        if len(settled_logs) != 1:
            raise RuntimeError(f"expected one settlement log, observed {len(settled_logs)}")
        transaction_hash = settled_logs[0]["transactionHash"]
        settle_receipt = wait_receipt(rpc_url, transaction_hash)
        transactions["claim"] = {
            "hash": claim_log["transactionHash"],
            "receipt": wait_receipt(rpc_url, claim_log["transactionHash"]),
            "recovered": True,
        }
        transactions["submit"] = {
            "hash": submit_log["transactionHash"],
            "receipt": wait_receipt(rpc_url, submit_log["transactionHash"]),
            "recovered": True,
        }
        transactions["settle"] = {
            "hash": transaction_hash,
            "receipt": settle_receipt,
            "recovered": True,
        }
        settled_transaction = rpc(rpc_url, "eth_getTransactionByHash", [transaction_hash])
        proof = cast(
            "calldata-decode", "verifyAndSettle(bytes)", settled_transaction["input"]
        ).splitlines()[0]
        nonce = int(proof, 16)
        response_hash = keccak_hex(
            abi_encode(
                ["bytes32", "uint64", "address", "bytes32", "bytes32", "bytes32", "uint256"],
                [
                    bytes.fromhex(bounty_id[2:]),
                    round_number,
                    solver_wallet,
                    bytes.fromhex(submission_hash[2:]),
                    bytes.fromhex(evidence_hash[2:]),
                    bytes.fromhex(policy_hash[2:]),
                    nonce,
                ],
            )
        )
        expected_verification_hash = keccak_hex(
            abi_encode(
                ["address", "bytes32", "bytes32"],
                [
                    network["verifier_module"],
                    bytes.fromhex(response_hash[2:]),
                    bytes.fromhex(keccak_hex(bytes.fromhex(proof[2:]))[2:]),
                ],
            )
        )
        event_verification_hash = f"0x{settled_logs[0]['data'][-64:]}".lower()
        if event_verification_hash != expected_verification_hash:
            raise RuntimeError("recovered proof does not match the canonical settlement event")
        balances_after = {
            "creator_wallet": token_balance(rpc_url, usdc, creator_wallet),
            "solver_wallet": token_balance(rpc_url, usdc, solver_wallet),
            "relayer": token_balance(rpc_url, usdc, relayer),
            "bounty": token_balance(rpc_url, usdc, predicted_bounty),
        }
        balances_before = {
            "creator_wallet": balances_after["creator_wallet"],
            "solver_wallet": balances_after["solver_wallet"] - 200_000,
            "relayer": balances_after["relayer"] - 100_000,
            "bounty": 300_000,
        }
    else:
        claim_payload = encoded_payload("f(address)", predicted_bounty)
        transaction_hash, receipt, authorization = relay_action(
            rpc_url, solver_wallet, "delegate-b", 2, claim_payload
        )
        transactions["claim"] = {
            "hash": transaction_hash,
            "receipt": receipt,
            "authorization": authorization,
        }
        if wait_call_int(rpc_url, predicted_bounty, "bountyStatus()(uint8)", 2) != 2:
            raise RuntimeError("bounty claim was not activated")

        submit_payload = encoded_payload(
            "f(address,bytes32,bytes32)", predicted_bounty, submission_hash, evidence_hash
        )
        transaction_hash, receipt, authorization = relay_action(
            rpc_url, solver_wallet, "delegate-b", 3, submit_payload
        )
        transactions["submit"] = {
            "hash": transaction_hash,
            "receipt": receipt,
            "authorization": authorization,
        }
        if wait_call_int(rpc_url, predicted_bounty, "bountyStatus()(uint8)", 3) != 3:
            raise RuntimeError("bounty submission was not recorded")

        nonce, proof, response_hash = mine_proof(
            int(network["verifier_difficulty_bits"]),
            bounty_id,
            round_number,
            solver_wallet,
            submission_hash,
            evidence_hash,
            policy_hash,
        )
        transaction_hash, settle_receipt = send(
            rpc_url, "relayer", predicted_bounty, "verifyAndSettle(bytes)", proof
        )
        transactions["settle"] = {"hash": transaction_hash, "receipt": settle_receipt}
        if wait_call_int(rpc_url, predicted_bounty, "bountyStatus()(uint8)", 4) != 4:
            raise RuntimeError("bounty did not settle")
        balances_after = {
            "creator_wallet": wait_token_balance(
                rpc_url, usdc, creator_wallet, balances_before["creator_wallet"]
            ),
            "solver_wallet": wait_token_balance(
                rpc_url, usdc, solver_wallet, balances_before["solver_wallet"] + 200_000
            ),
            "relayer": wait_token_balance(
                rpc_url, usdc, relayer, balances_before["relayer"] + 100_000
            ),
            "bounty": wait_token_balance(rpc_url, usdc, predicted_bounty, 0),
        }

    if not any(
        log["address"].lower() == predicted_bounty
        and log["topics"][0].lower() == settled_topic
        for log in settle_receipt["logs"]
    ):
        raise RuntimeError("settlement receipt has no canonical BountySettled event")
    before_total = sum(balances_before.values())
    after_total = sum(balances_after.values())
    if after_total != before_total:
        raise RuntimeError(f"USDC conservation failed: before {before_total}, after {after_total}")
    if balances_after["solver_wallet"] - balances_before["solver_wallet"] != 200_000:
        raise RuntimeError("solver wallet did not net the exact 0.20 USDC reward")
    if balances_after["relayer"] - balances_before["relayer"] != 100_000:
        raise RuntimeError("relayer did not receive the exact 0.10 USDC verifier reward")

    settled_block = int(settle_receipt["blockNumber"], 16)
    safe = safe_block_attestation(
        rpc_url,
        settled_block,
        {
            "wallet_factory": network["wallet_factory"],
            "creator_wallet": creator_wallet,
            "solver_wallet": solver_wallet,
            "bounty": predicted_bounty,
        },
    )
    report = {
        "schema_version": "agent-bounties/bounded-wallet-rehearsal-v1",
        "protocol_version": bundle["protocol_version"],
        "network": args.network,
        "chain_id": network["chain_id"],
        "source_revision": bundle["source_revision"],
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "contracts": {
            "bounty_factory": network["bounty_factory"],
            "wallet_factory": network["wallet_factory"],
            "verifier_module": network["verifier_module"],
            "creator_wallet": creator_wallet,
            "solver_wallet": solver_wallet,
            "bounty": predicted_bounty,
        },
        "terms": terms_document,
        "commitments": {
            "terms_hash": terms_hash,
            "policy_hash": policy_hash,
            "acceptance_criteria_hash": criteria_hash,
            "benchmark_hash": benchmark_hash,
            "evidence_schema_hash": evidence_schema_hash,
            "submission_hash": submission_hash,
            "evidence_hash": evidence_hash,
        },
        "proof": {
            "difficulty_bits": network["verifier_difficulty_bits"],
            "nonce": nonce,
            "response_hash": response_hash,
        },
        "balances_before": balances_before,
        "balances_after": balances_after,
        "usdc_conserved": True,
        "transactions": transactions,
        "canonical_settlement_transaction": transaction_hash,
        "safe_block_observation": safe,
        "evidence_boundary": "Only the confirmed canonical BountySettled event in the settlement receipt proves this solver payment. The report also reconciles exact USDC balance deltas and code hashes at a Base safe block.",
    }
    output = args.output or (
        ROOT
        / "target"
        / "bounded-wallet-activation"
        / f"{args.network}-rehearsal-{now}.json"
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(output)


if __name__ == "__main__":
    main()
