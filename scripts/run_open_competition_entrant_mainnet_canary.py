#!/usr/bin/env python3
"""Prepare and run the bounded hosted entrant-relay canary on Base mainnet."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import secrets
import time
from typing import Any
import urllib.error
import urllib.request

from eth_abi import encode
from eth_account import Account
from web3 import Web3

import local_delegate_wallet as delegate_store


SCHEMA = "agent-bounties/open-competition-entrant-mainnet-canary-plan-v1"
EVIDENCE_SCHEMA = "agent-bounties/open-competition-entrant-mainnet-canary-evidence-v1"
COMMITMENT_SCHEMA = "agent-bounties/open-competition-v1-commitment-v1"
CHAIN_ID = 8_453
NETWORK = "base-mainnet"
RPC_URL = "https://base.drpc.org"
API_URL = "https://api.agentbounties.app"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
COMPETITION_FACTORY = "0x9e9382beb8b1a45b737d484b5eafa7b8779d4ca5"
COMPETITION_IMPLEMENTATION = "0xa504454ac8cdd043c11c8c6af81a072472aa1651"
ENTRANT_FACTORY = "0x9b92a65a42de770157f30dd75f44a3136f2cda79"
ENTRANT_IMPLEMENTATION = "0xd7890aa6c4d4c981c246a05576a6fc689255923c"
VERIFIER = "0xcc6059ceeda5bc4ba8a97ecfbffa7488c8fd579e"
VERIFIER_RUNTIME_HASH = "0xbaa3a8305c4b65d0dc20131d0ef207fdaf4763f345393a831370cd04077df9b3"
BENCHMARK_HASH = "0x8f5dc601eaff77e6102aab44f16a9b176df7ce0a998078782fb5d4b9e0c0ebf2"
EVIDENCE_SCHEMA_HASH = "0xea961c63fb67f86823003426b04a928406e44e9c8acc3dcb298189e9558083da"
FACTORY_RUNTIME_HASH = "0xa0596a53e2f4685d104c2f24176307edfcb4fe8f0fd86162378347996c8f3c40"
IMPLEMENTATION_RUNTIME_HASH = "0xd1789de47b6c956b090f4fcf693361ef93ad4aeeec74ddc359bdcf73cb1ea998"
CLONE_RUNTIME_HASH = "0xe94f67382a2692b2ebe7f71ab4163ae0c9c16bded92d45695454deed927b01d4"
COMPETITION_FACTORY_RUNTIME_HASH = "0x963ac715bb99d8eb669b0a2a77acc752a660474e8da5d0c3a5025878b0557712"
COMPETITION_IMPLEMENTATION_RUNTIME_HASH = "0xd06d8b654362da834eeec8f93fdc68481f93ff314c692ccc44f2e14327cd52cd"
SOLVER_REWARD = 80_000
VERIFIER_REWARD = 10_000
ENTRY_BOND = VERIFIER_REWARD
TARGET = SOLVER_REWARD + VERIFIER_REWARD
WALLET_FUNDING = ENTRY_BOND
TOTAL_CREATOR_BUDGET = TARGET + WALLET_FUNDING
COMPETITION_WINDOW = 24 * 60 * 60
REVEAL_WINDOW = 60 * 60
MAX_ENTRIES = 4
RECOVERY_FILE = "open-competition-mainnet-canary-commitment.dpapi"


ERC20_ABI = [
    {"type": "function", "name": "balanceOf", "stateMutability": "view", "inputs": [{"name": "account", "type": "address"}], "outputs": [{"type": "uint256"}]},
    {"type": "function", "name": "approve", "stateMutability": "nonpayable", "inputs": [{"name": "spender", "type": "address"}, {"name": "amount", "type": "uint256"}], "outputs": [{"type": "bool"}]},
    {"type": "function", "name": "transfer", "stateMutability": "nonpayable", "inputs": [{"name": "to", "type": "address"}, {"name": "amount", "type": "uint256"}], "outputs": [{"type": "bool"}]},
]

CREATE_COMPONENTS = [
    {"name": "solverReward", "type": "uint256"},
    {"name": "verifierReward", "type": "uint256"},
    {"name": "termsHash", "type": "bytes32"},
    {"name": "policyHash", "type": "bytes32"},
    {"name": "acceptanceCriteriaHash", "type": "bytes32"},
    {"name": "benchmarkHash", "type": "bytes32"},
    {"name": "evidenceSchemaHash", "type": "bytes32"},
    {"name": "fundingDeadline", "type": "uint64"},
    {"name": "competitionWindowSeconds", "type": "uint64"},
    {"name": "revealWindowSeconds", "type": "uint64"},
    {"name": "maxEntries", "type": "uint8"},
    {"name": "verifierModule", "type": "address"},
    {"name": "verifierRewardRecipient", "type": "address"},
]

COMPETITION_FACTORY_ABI = [
    {"type": "function", "name": "createCompetition", "stateMutability": "nonpayable", "inputs": [{"name": "params", "type": "tuple", "components": CREATE_COMPONENTS}, {"name": "initialFunding", "type": "uint256"}, {"name": "creationNonce", "type": "bytes32"}], "outputs": [{"name": "bountyAddress", "type": "address"}, {"name": "bountyId", "type": "bytes32"}]},
    {"type": "function", "name": "predictCompetitionAddress", "stateMutability": "view", "inputs": [{"name": "creator", "type": "address"}, {"name": "params", "type": "tuple", "components": CREATE_COMPONENTS}, {"name": "creationNonce", "type": "bytes32"}], "outputs": [{"type": "address"}]},
    {"type": "function", "name": "isCanonicalCompetition", "stateMutability": "view", "inputs": [{"name": "bounty", "type": "address"}], "outputs": [{"type": "bool"}]},
    {"type": "function", "name": "implementation", "stateMutability": "view", "inputs": [], "outputs": [{"type": "address"}]},
]

POLICY_COMPONENTS = [
    {"name": "delegate", "type": "address"},
    {"name": "validAfter", "type": "uint64"},
    {"name": "validUntil", "type": "uint64"},
    {"name": "periodSeconds", "type": "uint64"},
    {"name": "maxPerAction", "type": "uint256"},
    {"name": "maxPerPeriod", "type": "uint256"},
    {"name": "maxLifetimeSpend", "type": "uint256"},
    {"name": "maxBountyTarget", "type": "uint256"},
    {"name": "allowedActions", "type": "uint8"},
    {"name": "verifierModule", "type": "address"},
    {"name": "verifierRuntimeCodeHash", "type": "bytes32"},
    {"name": "verifierPolicyHash", "type": "bytes32"},
    {"name": "acceptanceCriteriaHash", "type": "bytes32"},
    {"name": "benchmarkHash", "type": "bytes32"},
    {"name": "evidenceSchemaHash", "type": "bytes32"},
]

ENTRANT_FACTORY_ABI = [
    {"type": "function", "name": "createWallet", "stateMutability": "nonpayable", "inputs": [{"name": "owner", "type": "address"}, {"name": "policy", "type": "tuple", "components": POLICY_COMPONENTS}, {"name": "userSalt", "type": "bytes32"}], "outputs": [{"name": "wallet", "type": "address"}]},
    {"type": "function", "name": "predictWallet", "stateMutability": "view", "inputs": [{"name": "owner", "type": "address"}, {"name": "policy", "type": "tuple", "components": POLICY_COMPONENTS}, {"name": "userSalt", "type": "bytes32"}], "outputs": [{"type": "address"}]},
    {"type": "function", "name": "implementation", "stateMutability": "view", "inputs": [], "outputs": [{"type": "address"}]},
    {"type": "function", "name": "isFactoryWallet", "stateMutability": "view", "inputs": [{"name": "wallet", "type": "address"}], "outputs": [{"type": "bool"}]},
]

ENTRANT_WALLET_ABI = [
    {"type": "function", "name": "owner", "stateMutability": "view", "inputs": [], "outputs": [{"type": "address"}]},
    {"type": "function", "name": "policyHash", "stateMutability": "view", "inputs": [], "outputs": [{"type": "bytes32"}]},
    {"type": "function", "name": "policyVersion", "stateMutability": "view", "inputs": [], "outputs": [{"type": "uint64"}]},
    {"type": "function", "name": "delegateNonce", "stateMutability": "view", "inputs": [], "outputs": [{"type": "uint256"}]},
    {"type": "function", "name": "policy", "stateMutability": "view", "inputs": [], "outputs": [{"name": "value", "type": "tuple", "components": POLICY_COMPONENTS}]},
]

BOUNTY_ABI = [
    {"type": "function", "name": "bountyId", "stateMutability": "view", "inputs": [], "outputs": [{"type": "bytes32"}]},
    {"type": "function", "name": "creator", "stateMutability": "view", "inputs": [], "outputs": [{"type": "address"}]},
    {"type": "function", "name": "status", "stateMutability": "view", "inputs": [], "outputs": [{"type": "uint8"}]},
    {"type": "function", "name": "winner", "stateMutability": "view", "inputs": [], "outputs": [{"type": "address"}]},
    {"type": "function", "name": "policyHash", "stateMutability": "view", "inputs": [], "outputs": [{"type": "bytes32"}]},
    {"type": "function", "name": "entries", "stateMutability": "view", "inputs": [{"name": "solver", "type": "address"}], "outputs": [{"name": "commitment", "type": "bytes32"}, {"name": "committedBlock", "type": "uint64"}, {"name": "revealDeadline", "type": "uint64"}, {"name": "bond", "type": "uint256"}, {"name": "state", "type": "uint8"}]},
]


def fail(message: str) -> None:
    raise SystemExit(message)


def checksum(value: str) -> str:
    try:
        return Web3.to_checksum_address(value)
    except ValueError:
        fail("invalid EVM address")


def bytes32(value: str) -> bytes:
    raw = Web3.to_bytes(hexstr=value)
    if len(raw) != 32:
        fail("bytes32 value has the wrong length")
    return raw


def code_hash(w3: Web3, address: str, block: int | str = "safe") -> str:
    return Web3.to_hex(
        Web3.keccak(w3.eth.get_code(checksum(address), block_identifier=block))
    ).lower()


def minimal_proxy_runtime_hash(implementation: str) -> str:
    runtime = bytes.fromhex(
        "363d3d373d3d3d363d73"
        + checksum(implementation)[2:].lower()
        + "5af43d82803e903d91602b57fd5bf3"
    )
    return Web3.to_hex(Web3.keccak(runtime)).lower()


def safe_block(w3: Web3) -> dict[str, Any]:
    block = w3.eth.get_block("safe")
    if w3.eth.chain_id != CHAIN_ID or block.number is None:
        fail("Base mainnet safe block is unavailable")
    return {
        "number": block.number,
        "hash": Web3.to_hex(block.hash).lower(),
        "timestamp": block.timestamp,
    }


def label_hash(delegate: str, block_number: int, label: str) -> bytes:
    return Web3.keccak(text=f"agent-bounties/open-competition-v1/entrant-mainnet-canary/{delegate.lower()}/{block_number}/{label}")


def commitment_for(bounty: str, solver: str, submission: bytes, evidence: bytes, salt: bytes) -> bytes:
    domain = Web3.keccak(text="agent-bounties/open-competition-v1-solution")
    return Web3.keccak(
        encode(
            ["bytes32", "uint256", "address", "address", "bytes32", "bytes32", "bytes32"],
            [domain, CHAIN_ID, checksum(bounty), checksum(solver), submission, evidence, salt],
        )
    )


def mine_proof(
    bounty_id: bytes,
    solver: str,
    submission: bytes,
    evidence: bytes,
    policy_hash: bytes,
    *,
    difficulty_bits: int = 16,
    cap: int = 2_000_000,
) -> tuple[int, bytes, bytes]:
    prefix = encode(
        ["bytes32", "uint256", "address", "bytes32", "bytes32", "bytes32"],
        [bounty_id, 1, checksum(solver), submission, evidence, policy_hash],
    )
    for nonce in range(cap):
        proof = nonce.to_bytes(32, "big")
        work_hash = Web3.keccak(prefix + proof)
        if int.from_bytes(work_hash, "big") >> (256 - difficulty_bits) == 0:
            return nonce, proof, work_hash
    fail("leading-zero canary proof search cap reached")


def encode_call(contract: Any, name: str, arguments: list[Any]) -> str:
    return contract.encode_abi(name, args=arguments).lower()


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    state_dir = delegate_store.resolve_state_dir(args.state_dir)
    delegate = checksum(delegate_store.public_address(state_dir))
    creator = checksum(args.creator)
    if creator.lower() == delegate.lower():
        fail("creator must be separate from the entrant owner and delegate")
    w3 = Web3(Web3.HTTPProvider(args.rpc_url, request_kwargs={"timeout": 30}))
    safe = safe_block(w3)
    if code_hash(w3, ENTRANT_FACTORY, safe["number"]) != FACTORY_RUNTIME_HASH:
        fail("entrant factory runtime mismatch")
    if code_hash(w3, ENTRANT_IMPLEMENTATION, safe["number"]) != IMPLEMENTATION_RUNTIME_HASH:
        fail("entrant implementation runtime mismatch")
    if code_hash(w3, COMPETITION_FACTORY, safe["number"]) != COMPETITION_FACTORY_RUNTIME_HASH:
        fail("competition factory runtime mismatch")
    if code_hash(w3, COMPETITION_IMPLEMENTATION, safe["number"]) != COMPETITION_IMPLEMENTATION_RUNTIME_HASH:
        fail("competition implementation runtime mismatch")
    if code_hash(w3, VERIFIER, safe["number"]) != VERIFIER_RUNTIME_HASH:
        fail("canary verifier runtime mismatch")

    token = w3.eth.contract(address=checksum(USDC), abi=ERC20_ABI)
    competition_factory = w3.eth.contract(address=checksum(COMPETITION_FACTORY), abi=COMPETITION_FACTORY_ABI)
    entrant_factory = w3.eth.contract(address=checksum(ENTRANT_FACTORY), abi=ENTRANT_FACTORY_ABI)
    creator_usdc = token.functions.balanceOf(creator).call(block_identifier=safe["number"])
    creator_eth = w3.eth.get_balance(creator, block_identifier=safe["number"])
    if creator_usdc != TOTAL_CREATOR_BUDGET:
        fail("creator must hold exactly 0.10 USDC before the bounded canary")
    if creator_eth <= 0:
        fail("creator needs Base ETH for the four setup transactions")

    terms_hash = label_hash(delegate, safe["number"], "terms-v1")
    policy_hash = label_hash(delegate, safe["number"], "policy-v1")
    acceptance_hash = label_hash(delegate, safe["number"], "acceptance-v1")
    creation_nonce = label_hash(delegate, safe["number"], "creation-nonce-v1")
    user_salt = label_hash(delegate, safe["number"], "entrant-wallet-salt-v1")
    funding_deadline = safe["timestamp"] + 24 * 60 * 60
    params = (
        SOLVER_REWARD,
        VERIFIER_REWARD,
        terms_hash,
        policy_hash,
        acceptance_hash,
        bytes32(BENCHMARK_HASH),
        bytes32(EVIDENCE_SCHEMA_HASH),
        funding_deadline,
        COMPETITION_WINDOW,
        REVEAL_WINDOW,
        MAX_ENTRIES,
        checksum(VERIFIER),
        creator,
    )
    bounty = checksum(
        competition_factory.functions.predictCompetitionAddress(
            creator, params, creation_nonce
        ).call(block_identifier=safe["number"])
    )
    policy = (
        delegate,
        max(0, safe["timestamp"] - 30),
        safe["timestamp"] + COMPETITION_WINDOW + REVEAL_WINDOW + 3_600,
        24 * 60 * 60,
        ENTRY_BOND,
        ENTRY_BOND,
        ENTRY_BOND,
        TARGET,
        7,
        checksum(VERIFIER),
        bytes32(VERIFIER_RUNTIME_HASH),
        policy_hash,
        acceptance_hash,
        bytes32(BENCHMARK_HASH),
        bytes32(EVIDENCE_SCHEMA_HASH),
    )
    wallet_policy_hash = Web3.to_hex(
        Web3.keccak(
            encode(
                [
                    "address",
                    "uint64",
                    "uint64",
                    "uint64",
                    "uint256",
                    "uint256",
                    "uint256",
                    "uint256",
                    "uint8",
                    "address",
                    "bytes32",
                    "bytes32",
                    "bytes32",
                    "bytes32",
                    "bytes32",
                ],
                list(policy),
            )
        )
    )
    wallet = checksum(
        entrant_factory.functions.predictWallet(delegate, policy, user_salt).call(
            block_identifier=safe["number"]
        )
    )
    if w3.eth.get_code(bounty, block_identifier=safe["number"]):
        fail("predicted canary bounty address is already occupied")
    if w3.eth.get_code(wallet, block_identifier=safe["number"]):
        fail("predicted entrant wallet address is already occupied")

    transactions = [
        {
            "name": "approve_competition_funding",
            "from": creator.lower(),
            "to": USDC,
            "value": "0x0",
            "data": encode_call(token, "approve", [checksum(COMPETITION_FACTORY), TARGET]),
            "summary": "Approve exactly 0.09 USDC to the frozen Open Competition factory.",
        },
        {
            "name": "create_hidden_competition",
            "from": creator.lower(),
            "to": COMPETITION_FACTORY,
            "value": "0x0",
            "data": encode_call(competition_factory, "createCompetition", [params, TARGET, creation_nonce]),
            "summary": "Create and fully fund one hidden 0.08/0.01 USDC canary with four-entry capacity.",
        },
        {
            "name": "create_policy_bound_entrant_wallet",
            "from": creator.lower(),
            "to": ENTRANT_FACTORY,
            "value": "0x0",
            "data": encode_call(entrant_factory, "createWallet", [delegate, policy, user_salt]),
            "summary": "Create the separate DPAPI-backed policy-bound entrant wallet without funding it.",
        },
        {
            "name": "fund_entrant_bond",
            "from": creator.lower(),
            "to": USDC,
            "value": "0x0",
            "data": encode_call(token, "transfer", [wallet, WALLET_FUNDING]),
            "summary": "Transfer exactly 0.01 USDC to the entrant wallet for its one allowed bond.",
        },
    ]
    plan = {
        "schema_version": SCHEMA,
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "safe_block": safe,
        "creator": creator.lower(),
        "delegate": delegate.lower(),
        "owner": delegate.lower(),
        "bounty": bounty.lower(),
        "wallet": wallet.lower(),
        "wallet_policy_hash": wallet_policy_hash,
        "economics": {
            "solver_reward": SOLVER_REWARD,
            "verifier_reward": VERIFIER_REWARD,
            "entry_bond": ENTRY_BOND,
            "creator_setup_budget": TOTAL_CREATOR_BUDGET,
            "max_entries": MAX_ENTRIES,
            "competition_window_seconds": COMPETITION_WINDOW,
            "reveal_window_seconds": REVEAL_WINDOW,
        },
        "profile": {
            "verifier": VERIFIER,
            "verifier_runtime_code_hash": VERIFIER_RUNTIME_HASH,
            "policy_hash": Web3.to_hex(policy_hash),
            "acceptance_criteria_hash": Web3.to_hex(acceptance_hash),
            "benchmark_hash": BENCHMARK_HASH,
            "evidence_schema_hash": EVIDENCE_SCHEMA_HASH,
        },
        "transactions": transactions,
        "preflight_balances": {"creator_usdc": creator_usdc, "creator_eth_wei": creator_eth},
        "public_activation": False,
        "evidence_boundary": (
            "This is an unsigned four-transaction hidden-canary setup plan. It does not prove entry, "
            "settlement, payment, public readiness, or representative digital-work verification."
        ),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(plan, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return {"output": str(args.output), "bounty": plan["bounty"], "wallet": plan["wallet"], "transactions": transactions}


def api_json(method: str, url: str, *, token: str | None = None, body: dict | None = None) -> dict:
    headers = {"accept": "application/json", "user-agent": "agent-bounties-mainnet-entrant-canary/1"}
    data = None
    if token is not None:
        headers["x-operator-token"] = token
    if body is not None:
        headers["content-type"] = "application/json"
        data = json.dumps(body, separators=(",", ":")).encode()
    request = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=95) as response:
            value = json.load(response)
    except urllib.error.HTTPError as error:
        fail(f"hosted entrant relay returned HTTP {error.code}")
    except (OSError, TimeoutError):
        fail("hosted entrant relay transport failed")
    if not isinstance(value, dict):
        fail("hosted entrant relay returned an invalid response")
    return value


def read_plan(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict) or value.get("schema_version") != SCHEMA:
        fail("canary setup plan is invalid")
    return value


def wait_for_safe_receipts(w3: Web3, transaction_hashes: list[str], timeout: int) -> tuple[list[Any], dict]:
    deadline = time.monotonic() + timeout
    receipts = []
    for transaction_hash in transaction_hashes:
        receipt = w3.eth.get_transaction_receipt(transaction_hash)
        if receipt.status != 1 or receipt.blockNumber is None:
            fail("one canary setup transaction reverted or lacks a canonical block")
        receipts.append(receipt)
    target = max(receipt.blockNumber for receipt in receipts)
    while time.monotonic() < deadline:
        safe = safe_block(w3)
        if safe["number"] >= target:
            for receipt in receipts:
                canonical = w3.eth.get_block(receipt.blockNumber)
                if canonical.hash != receipt.blockHash:
                    fail("canary setup transaction was reorganized")
            return receipts, safe
        time.sleep(3)
    fail("canary setup receipts did not become safe before timeout")


def sign_plan(state_dir: Path, plan: dict) -> str:
    delegate = delegate_store.public_address(state_dir).lower()
    typed = plan.get("signing_payload")
    if not isinstance(typed, dict) or plan.get("delegate", "").lower() != delegate:
        fail("hosted plan delegate or signing payload is invalid")
    protected = delegate_store.require_private_file(state_dir / delegate_store.DPAPI_BLOB)
    password = bytearray(delegate_store.unprotect_secret(protected))
    private_key = bytearray()
    try:
        private_key = bytearray(
            Account.decrypt(delegate_store.read_json(state_dir / delegate_store.KEYSTORE), bytes(password))
        )
        account = Account.from_key(bytes(private_key))
        if account.address.lower() != delegate:
            fail("decrypted canary delegate does not match its public address")
        return Web3.to_hex(Account.sign_typed_data(bytes(private_key), full_message=typed).signature).lower()
    finally:
        for secret in (password, private_key):
            for index in range(len(secret)):
                secret[index] = 0


def expected_payload(action: str, bounty: str, envelope: dict, proof: bytes | None) -> bytes:
    if action == "commit":
        return encode(["address", "bytes32"], [checksum(bounty), bytes32(envelope["commitment"])])
    if proof is None:
        fail("reveal proof is missing")
    return encode(
        ["address", "bytes32", "bytes32", "bytes32", "bytes"],
        [
            checksum(bounty),
            bytes32(envelope["submission_hash"]),
            bytes32(envelope["evidence_hash"]),
            bytes32(envelope["salt"]),
            proof,
        ],
    )


def validate_hosted_plan(plan: dict, setup: dict, action: str, envelope: dict, proof: bytes | None) -> None:
    action_code = 0 if action == "commit" else 1
    payload = expected_payload(action, setup["bounty"], envelope, proof)
    typed = plan.get("signing_payload") or {}
    message = typed.get("message") or {}
    expected_domain = {
        "name": "Agent Bounties Open Competition Entrant Wallet",
        "version": "1",
        "chainId": CHAIN_ID,
        "verifyingContract": setup["wallet"],
    }
    expected_types = {
        "EIP712Domain": [
            {"name": "name", "type": "string"},
            {"name": "version", "type": "string"},
            {"name": "chainId", "type": "uint256"},
            {"name": "verifyingContract", "type": "address"},
        ],
        "OpenCompetitionEntrantAction": [
            {"name": "wallet", "type": "address"},
            {"name": "action", "type": "uint8"},
            {"name": "payloadHash", "type": "bytes32"},
            {"name": "nonce", "type": "uint256"},
            {"name": "deadline", "type": "uint256"},
            {"name": "policyVersion", "type": "uint64"},
        ],
    }
    if (
        plan.get("schema_version") != "agent-bounties/open-competition-entrant-wallet-action-v1"
        or plan.get("network") != NETWORK
        or plan.get("chain_id") != CHAIN_ID
        or plan.get("wallet", "").lower() != setup["wallet"]
        or plan.get("delegate", "").lower() != setup["delegate"]
        or plan.get("policy_hash", "").lower() != setup["wallet_policy_hash"].lower()
        or plan.get("action") != action
        or plan.get("action_code") != action_code
        or plan.get("payload", "").lower() != Web3.to_hex(payload).lower()
        or plan.get("payload_hash", "").lower() != Web3.to_hex(Web3.keccak(payload)).lower()
        or typed.get("domain") != expected_domain
        or typed.get("types") != expected_types
        or typed.get("primaryType") != "OpenCompetitionEntrantAction"
        or str(message.get("wallet", "")).lower() != setup["wallet"]
        or message.get("action") != action_code
        or str(message.get("payloadHash", "")).lower() != Web3.to_hex(Web3.keccak(payload)).lower()
        or int(str(message.get("nonce", -1))) != plan.get("nonce")
        or int(str(message.get("deadline", -1))) != plan.get("deadline")
        or int(str(message.get("policyVersion", -1))) != plan.get("policy_version")
        or plan.get("relay_call")
        != {
            "to": setup["wallet"],
            "function": "executeWithSignature(uint8,bytes,uint256,uint256,bytes)",
            "arguments_before_signature": [
                action_code,
                Web3.to_hex(payload).lower(),
                plan.get("nonce"),
                plan.get("deadline"),
            ],
            "signature_tail": ["delegate_signature"],
        }
    ):
        fail("hosted entrant action plan does not match the exact local action")


def persist_recovery(state_dir: Path, envelope: dict) -> None:
    plaintext = bytearray((json.dumps(envelope, sort_keys=True) + "\n").encode())
    try:
        protected = delegate_store.protect_secret(bytes(plaintext))
        delegate_store.write_atomic(state_dir / RECOVERY_FILE, protected)
        recovered = delegate_store.unprotect_secret(
            delegate_store.require_private_file(state_dir / RECOVERY_FILE)
        )
        if recovered != bytes(plaintext):
            fail("DPAPI commitment recovery verification failed")
    finally:
        for index in range(len(plaintext)):
            plaintext[index] = 0


def relay_action(
    *,
    setup: dict,
    state_dir: Path,
    token: str,
    action: str,
    envelope: dict,
    proof: bytes | None,
) -> dict:
    body = {
        "network": NETWORK,
        "wallet": setup["wallet"],
        "bounty_contract": setup["bounty"],
        "action": action,
        "deadline_seconds": 600,
    }
    if action == "commit":
        body["commitment"] = envelope["commitment"]
    else:
        body["commitment_envelope"] = envelope
        body["proof"] = Web3.to_hex(proof)
    plan = api_json(
        "POST",
        f"{API_URL}/v1/base/open-competition-v1/entrant-action-preparation",
        token=token,
        body=body,
    )
    validate_hosted_plan(plan, setup, action, envelope, proof)
    signature = sign_plan(state_dir, plan)
    relay = api_json(
        "POST",
        f"{API_URL}/v1/base/open-competition-v1/entrant-action-relays",
        token=token,
        body={
            "idempotency_key": f"mainnet-canary-{action}-{plan['nonce']}-{secrets.token_hex(8)}",
            "plan": plan,
            "signature": signature,
        },
    )
    relay_id = relay.get("id")
    if not isinstance(relay_id, str):
        fail("hosted entrant relay omitted its durable id")
    for _ in range(90):
        if relay.get("status") in {"confirmed", "failed"}:
            break
        time.sleep(3)
        relay = api_json(
            "GET", f"{API_URL}/v1/base/open-competition-v1/entrant-action-relays/{relay_id}"
        )
    if relay.get("status") != "confirmed":
        fail("hosted entrant relay did not confirm at a Base safe block")
    expected_event = "SolutionCommitted" if action == "commit" else "BountySettled"
    if relay.get("canonical_event") != expected_event:
        fail("hosted entrant relay confirmed an unexpected canonical event")
    if bool(relay.get("payment_proven")) != (action == "reveal"):
        fail("hosted entrant relay payment evidence is inconsistent")
    return relay


def relay(args: argparse.Namespace) -> dict[str, Any]:
    setup = read_plan(args.plan)
    state_dir = delegate_store.resolve_state_dir(args.state_dir)
    if delegate_store.public_address(state_dir).lower() != setup["delegate"]:
        fail("DPAPI delegate does not match the canary setup plan")
    token = os.environ.get(args.operator_token_env, "")
    if len(token) < 32 or any(character.isspace() for character in token):
        fail("operator token environment variable is missing or malformed")
    w3 = Web3(Web3.HTTPProvider(args.rpc_url, request_kwargs={"timeout": 30}))
    transaction_hashes = [args.approval_tx, args.creation_tx, args.wallet_creation_tx, args.wallet_funding_tx]
    receipts, setup_safe = wait_for_safe_receipts(w3, transaction_hashes, args.safe_timeout)
    transactions = [w3.eth.get_transaction(transaction_hash) for transaction_hash in transaction_hashes]
    expected_transactions = setup.get("transactions")
    if not isinstance(expected_transactions, list) or len(expected_transactions) != 4:
        fail("canary setup plan transaction list is invalid")
    for transaction, expected in zip(transactions, expected_transactions, strict=True):
        if (
            transaction["from"].lower() != expected["from"]
            or transaction["to"].lower() != expected["to"]
            or Web3.to_hex(transaction["input"]).lower() != expected["data"]
            or int(transaction["value"]) != 0
        ):
            fail("canary setup transaction does not match the frozen plan")
    if code_hash(w3, setup["wallet"], setup_safe["number"]) != CLONE_RUNTIME_HASH:
        fail("deployed entrant wallet runtime mismatch")
    entrant_factory = w3.eth.contract(address=checksum(ENTRANT_FACTORY), abi=ENTRANT_FACTORY_ABI)
    competition_factory = w3.eth.contract(address=checksum(COMPETITION_FACTORY), abi=COMPETITION_FACTORY_ABI)
    wallet_contract = w3.eth.contract(address=checksum(setup["wallet"]), abi=ENTRANT_WALLET_ABI)
    bounty_contract = w3.eth.contract(address=checksum(setup["bounty"]), abi=BOUNTY_ABI)
    token_contract = w3.eth.contract(address=checksum(USDC), abi=ERC20_ABI)
    block = setup_safe["number"]
    live_policy = wallet_contract.functions.policy().call(block_identifier=block)
    expected_profile = setup["profile"]
    if (
        not entrant_factory.functions.isFactoryWallet(checksum(setup["wallet"])).call(block_identifier=block)
        or not competition_factory.functions.isCanonicalCompetition(checksum(setup["bounty"])).call(block_identifier=block)
        or wallet_contract.functions.owner().call(block_identifier=block).lower() != setup["owner"]
        or bounty_contract.functions.creator().call(block_identifier=block).lower() != setup["creator"]
        or entrant_factory.functions.implementation().call(block_identifier=block).lower() != ENTRANT_IMPLEMENTATION
        or competition_factory.functions.implementation().call(block_identifier=block).lower() != COMPETITION_IMPLEMENTATION
        or Web3.to_hex(wallet_contract.functions.policyHash().call(block_identifier=block)).lower()
        != setup["wallet_policy_hash"].lower()
        or live_policy[0].lower() != setup["delegate"]
        or int(live_policy[4]) != ENTRY_BOND
        or int(live_policy[5]) != ENTRY_BOND
        or int(live_policy[6]) != ENTRY_BOND
        or int(live_policy[7]) != TARGET
        or int(live_policy[8]) != 7
        or live_policy[9].lower() != VERIFIER
        or Web3.to_hex(live_policy[10]).lower() != VERIFIER_RUNTIME_HASH
        or Web3.to_hex(live_policy[11]).lower() != expected_profile["policy_hash"]
        or Web3.to_hex(live_policy[12]).lower() != expected_profile["acceptance_criteria_hash"]
        or Web3.to_hex(live_policy[13]).lower() != BENCHMARK_HASH
        or Web3.to_hex(live_policy[14]).lower() != EVIDENCE_SCHEMA_HASH
        or Web3.to_hex(bounty_contract.functions.policyHash().call(block_identifier=block)).lower()
        != expected_profile["policy_hash"]
        or code_hash(w3, setup["bounty"], block) != minimal_proxy_runtime_hash(COMPETITION_IMPLEMENTATION)
        or token_contract.functions.balanceOf(checksum(setup["wallet"])).call(block_identifier=block) != WALLET_FUNDING
        or token_contract.functions.balanceOf(checksum(setup["bounty"])).call(block_identifier=block) != TARGET
        or token_contract.functions.balanceOf(checksum(setup["creator"])).call(block_identifier=block) != 0
    ):
        fail("canary setup state or exact USDC allocation mismatch")

    submission = Web3.keccak(text=f"agent-bounties/open-competition-v1/entrant-mainnet-canary/{setup['bounty']}/submission-v1")
    evidence = Web3.keccak(text=f"agent-bounties/open-competition-v1/entrant-mainnet-canary/{setup['bounty']}/evidence-v1")
    salt = secrets.token_bytes(32)
    commitment = commitment_for(setup["bounty"], setup["wallet"], submission, evidence, salt)
    envelope = {
        "schema_version": COMMITMENT_SCHEMA,
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "bounty": setup["bounty"],
        "solver": setup["wallet"],
        "submission_hash": Web3.to_hex(submission),
        "evidence_hash": Web3.to_hex(evidence),
        "salt": Web3.to_hex(salt),
        "commitment": Web3.to_hex(commitment),
        "committed_block": None,
        "reveal_deadline": None,
        "evidence_boundary": "DPAPI-protected local recovery material; never persist on the API",
    }
    persist_recovery(state_dir, envelope)
    commit_relay = relay_action(
        setup=setup,
        state_dir=state_dir,
        token=token,
        action="commit",
        envelope=envelope,
        proof=None,
    )
    commit_safe = int(commit_relay["canonical_safe_block"])
    entry = bounty_contract.functions.entries(checksum(setup["wallet"])).call(block_identifier=commit_safe)
    envelope["committed_block"] = int(entry[1])
    envelope["reveal_deadline"] = int(entry[2])
    if Web3.to_hex(entry[0]).lower() != envelope["commitment"] or int(entry[4]) != 1:
        fail("canonical committed entry does not match the recovery envelope")
    persist_recovery(state_dir, envelope)

    bounty_id = bounty_contract.functions.bountyId().call(block_identifier=commit_safe)
    policy_hash = bounty_contract.functions.policyHash().call(block_identifier=commit_safe)
    nonce, proof, work_hash = mine_proof(
        bounty_id, setup["wallet"], submission, evidence, policy_hash
    )
    reveal_relay = relay_action(
        setup=setup,
        state_dir=state_dir,
        token=token,
        action="reveal",
        envelope=envelope,
        proof=proof,
    )
    final_safe = int(reveal_relay["canonical_safe_block"])
    creator_final = token_contract.functions.balanceOf(checksum(setup["creator"])).call(block_identifier=final_safe)
    wallet_final = token_contract.functions.balanceOf(checksum(setup["wallet"])).call(block_identifier=final_safe)
    bounty_final = token_contract.functions.balanceOf(checksum(setup["bounty"])).call(block_identifier=final_safe)
    winner = bounty_contract.functions.winner().call(block_identifier=final_safe).lower()
    status = bounty_contract.functions.status().call(block_identifier=final_safe)
    assertions = {
        "creator_is_not_solver": setup["creator"] != setup["wallet"],
        "separate_dpapi_delegate": setup["delegate"] == setup["owner"] and setup["delegate"] != setup["creator"],
        "hosted_commit_confirmed": commit_relay["canonical_event"] == "SolutionCommitted",
        "hosted_reveal_confirmed": reveal_relay["canonical_event"] == "BountySettled",
        "canonical_payment_proven": reveal_relay["payment_proven"] is True,
        "settled_status": status == 2,
        "winner_is_entrant_wallet": winner == setup["wallet"],
        "creator_final_usdc": creator_final == VERIFIER_REWARD,
        "wallet_final_usdc": wallet_final == SOLVER_REWARD + ENTRY_BOND,
        "bounty_final_usdc_zero": bounty_final == 0,
        "escrow_conservation": creator_final + wallet_final + bounty_final == TOTAL_CREATOR_BUDGET,
        "public_activation_remains_disabled": setup.get("public_activation") is False,
    }
    if not all(assertions.values()):
        fail("hidden entrant-relay canary failed final reconciliation")
    evidence_result = {
        "schema_version": EVIDENCE_SCHEMA,
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "setup_safe_block": setup_safe,
        "creator": setup["creator"],
        "owner": setup["owner"],
        "delegate": setup["delegate"],
        "wallet": setup["wallet"],
        "bounty": setup["bounty"],
        "setup_transactions": transaction_hashes,
        "commit_relay": {key: commit_relay.get(key) for key in ("id", "transaction_hash", "receipt_block", "receipt_block_hash", "canonical_safe_block", "canonical_safe_block_hash", "canonical_event", "payment_proven")},
        "reveal_relay": {key: reveal_relay.get(key) for key in ("id", "transaction_hash", "receipt_block", "receipt_block_hash", "canonical_safe_block", "canonical_safe_block_hash", "canonical_event", "payment_proven")},
        "proof": {"nonce": nonce, "work_hash": Web3.to_hex(work_hash), "difficulty_bits": 16, "proof_persisted": False},
        "balances": {"creator_final": creator_final, "entrant_wallet_final": wallet_final, "bounty_final": bounty_final},
        "assertions": assertions,
        "passed": True,
        "secrets_published": False,
        "evidence_boundary": (
            "This proves one bounded operator-authenticated hosted relay canary using the canary-only "
            "LeadingZeroWork(16) profile. It does not approve that profile for public inventory or enable public earning."
        ),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(evidence_result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return evidence_result


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    prepare_parser = commands.add_parser("prepare")
    prepare_parser.add_argument("--state-dir", required=True)
    prepare_parser.add_argument("--creator", required=True)
    prepare_parser.add_argument("--rpc-url", default=RPC_URL)
    prepare_parser.add_argument(
        "--output",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-mainnet-live-canary-plan.json"),
    )
    relay_parser = commands.add_parser("relay")
    relay_parser.add_argument("--state-dir", required=True)
    relay_parser.add_argument("--plan", type=Path, required=True)
    relay_parser.add_argument("--approval-tx", required=True)
    relay_parser.add_argument("--creation-tx", required=True)
    relay_parser.add_argument("--wallet-creation-tx", required=True)
    relay_parser.add_argument("--wallet-funding-tx", required=True)
    relay_parser.add_argument("--operator-token-env", default="OPEN_COMPETITION_CANARY_OPERATOR_TOKEN")
    relay_parser.add_argument("--rpc-url", default=RPC_URL)
    relay_parser.add_argument("--safe-timeout", type=int, default=300)
    relay_parser.add_argument(
        "--output",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-mainnet-live-canary-evidence.json"),
    )
    return root


def main() -> None:
    args = parser().parse_args()
    result = prepare(args) if args.command == "prepare" else relay(args)
    if args.command == "prepare":
        print(json.dumps(result, indent=2, sort_keys=True))
    else:
        print(json.dumps({"passed": result["passed"], "output": str(args.output)}, sort_keys=True))


if __name__ == "__main__":
    main()
