#!/usr/bin/env python3
"""Operate one policy-bound agent wallet with a local encrypted delegate key."""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
import secrets
import stat
# Used only with absolute System32 executables, argument vectors, and no shell.
import subprocess  # nosec B404
import tempfile
import time
from ctypes import wintypes
from pathlib import Path
from typing import Any

from eth_account import Account
from eth_utils import to_checksum_address

from bounded_agent_create import decode_create_calldata, validate_creation_plan
from inspect_bounded_agent_wallet import inspect, rpc
from plan_bounded_agent_action import (
    ACTIONS,
    ZERO_ADDRESS,
    bounty_state,
    validate_bounty_policy,
    validate_spend,
)
from plan_bounded_agent_budget import (
    DEFAULT_MANIFEST,
    ROOT,
    calldata,
    encode,
    keccak_hex,
    require_address,
    require_bytes32,
    validate_manifest,
)


STATE_SCHEMA = "agent-bounties/local-delegate-state-v1"
BINDING_SCHEMA = "agent-bounties/local-delegate-binding-v1"
PLAN_SCHEMA = "agent-bounties/bounded-agent-action-plan-v1"
RELAY_SCHEMA = "agent-bounties/bounded-wallet-relay-v1"
EXPECTED_CHAIN_ID = 8453
EXPECTED_NETWORK = "base-mainnet"
MAX_PLAN_AGE_SECONDS = 300
MAX_GAS_LIMIT = 2_000_000
MAX_FEE_PER_GAS_WEI = 2_000_000_000
MAX_TOTAL_GAS_WEI = 3_000_000_000_000_000
DPAPI_ENTROPY = b"AgentBounties/local-delegate/v1"
KEYSTORE = "keystore.json"
DPAPI_BLOB = "credential.dpapi"
METADATA = "state.json"
BINDING = "binding.json"


def fail(message: str) -> None:
    raise SystemExit(message)


def default_state_dir() -> Path:
    local_app_data = os.environ.get("LOCALAPPDATA")
    if not local_app_data:
        fail("LOCALAPPDATA is unavailable; pass --state-dir outside the repository")
    return Path(local_app_data) / "AgentBounties" / "delegate"


def resolve_state_dir(value: str | None) -> Path:
    path = Path(value).expanduser().resolve() if value else default_state_dir().resolve()
    try:
        path.relative_to(ROOT.resolve())
    except ValueError:
        return path
    fail("delegate state must be stored outside the repository")


def require_private_file(path: Path) -> bytes:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        fail(f"delegate state is incomplete: {path.name} is missing")
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        fail(f"delegate state is unsafe: {path.name} must be a regular file")
    return path.read_bytes()


def write_all(descriptor: int, data: bytes) -> None:
    remaining = memoryview(data)
    while remaining:
        written = os.write(descriptor, remaining)
        if written <= 0:
            fail("secure delegate state write made no progress")
        remaining = remaining[written:]


def write_exclusive(path: Path, data: bytes) -> None:
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0)
    descriptor = os.open(path, flags, 0o600)
    try:
        write_all(descriptor, data)
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def write_atomic(path: Path, data: bytes) -> None:
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        write_all(descriptor, data)
        os.fsync(descriptor)
        os.close(descriptor)
        descriptor = -1
        os.replace(temporary, path)
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        if os.path.exists(temporary):
            os.unlink(temporary)


def harden_acl(path: Path) -> None:
    if os.name != "nt":
        fail("the local delegate currently requires Windows DPAPI")
    system32 = Path(os.environ.get("SystemRoot", r"C:\Windows")) / "System32"
    icacls_executable = system32 / "icacls.exe"
    if not icacls_executable.is_file():
        fail("Windows ACL tools are unavailable")
    secur32 = ctypes.WinDLL("secur32", use_last_error=True)
    secur32.GetUserNameExW.argtypes = [ctypes.c_int, wintypes.LPWSTR, ctypes.POINTER(wintypes.ULONG)]
    secur32.GetUserNameExW.restype = wintypes.BOOL
    size = wintypes.ULONG(1024)
    principal_buffer = ctypes.create_unicode_buffer(size.value)
    if not secur32.GetUserNameExW(2, principal_buffer, ctypes.byref(size)):
        fail(f"could not identify the current Windows principal ({ctypes.get_last_error()})")
    principal = principal_buffer.value
    # This call uses a verified absolute System32 path and never invokes a shell.
    result = subprocess.run(  # nosec B603
        [str(icacls_executable), str(path), "/inheritance:r", "/grant:r", f"{principal}:(OI)(CI)F"],
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    if result.returncode != 0:
        fail("failed to restrict the delegate directory to the current Windows user")


class DataBlob(ctypes.Structure):
    _fields_ = [("cbData", wintypes.DWORD), ("pbData", ctypes.POINTER(ctypes.c_ubyte))]


def _blob(data: bytes) -> tuple[DataBlob, Any]:
    buffer = (ctypes.c_ubyte * len(data)).from_buffer_copy(data)
    return DataBlob(len(data), ctypes.cast(buffer, ctypes.POINTER(ctypes.c_ubyte))), buffer


def _crypt32() -> tuple[Any, Any]:
    if os.name != "nt":
        fail("the local delegate currently requires Windows DPAPI")
    crypt32 = ctypes.WinDLL("crypt32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    crypt32.CryptProtectData.argtypes = [
        ctypes.POINTER(DataBlob),
        wintypes.LPCWSTR,
        ctypes.POINTER(DataBlob),
        ctypes.c_void_p,
        ctypes.c_void_p,
        wintypes.DWORD,
        ctypes.POINTER(DataBlob),
    ]
    crypt32.CryptProtectData.restype = wintypes.BOOL
    crypt32.CryptUnprotectData.argtypes = [
        ctypes.POINTER(DataBlob),
        ctypes.c_void_p,
        ctypes.POINTER(DataBlob),
        ctypes.c_void_p,
        ctypes.c_void_p,
        wintypes.DWORD,
        ctypes.POINTER(DataBlob),
    ]
    crypt32.CryptUnprotectData.restype = wintypes.BOOL
    kernel32.LocalFree.argtypes = [ctypes.c_void_p]
    kernel32.LocalFree.restype = ctypes.c_void_p
    return crypt32, kernel32


def protect_secret(secret: bytes) -> bytes:
    crypt32, kernel32 = _crypt32()
    source, source_buffer = _blob(secret)
    entropy, entropy_buffer = _blob(DPAPI_ENTROPY)
    output = DataBlob()
    try:
        succeeded = crypt32.CryptProtectData(
            ctypes.byref(source),
            "Agent Bounties local delegate",
            ctypes.byref(entropy),
            None,
            None,
            0x1,
            ctypes.byref(output),
        )
        if not succeeded:
            fail(f"Windows DPAPI protection failed ({ctypes.get_last_error()})")
        return ctypes.string_at(output.pbData, output.cbData)
    finally:
        ctypes.memset(source_buffer, 0, len(secret))
        ctypes.memset(entropy_buffer, 0, len(DPAPI_ENTROPY))
        if output.pbData:
            kernel32.LocalFree(ctypes.cast(output.pbData, ctypes.c_void_p))


def unprotect_secret(protected: bytes) -> bytes:
    crypt32, kernel32 = _crypt32()
    source, source_buffer = _blob(protected)
    entropy, entropy_buffer = _blob(DPAPI_ENTROPY)
    output = DataBlob()
    try:
        succeeded = crypt32.CryptUnprotectData(
            ctypes.byref(source),
            None,
            ctypes.byref(entropy),
            None,
            None,
            0x1,
            ctypes.byref(output),
        )
        if not succeeded:
            fail(f"Windows DPAPI unprotection failed ({ctypes.get_last_error()})")
        return ctypes.string_at(output.pbData, output.cbData)
    finally:
        ctypes.memset(source_buffer, 0, len(protected))
        ctypes.memset(entropy_buffer, 0, len(DPAPI_ENTROPY))
        if output.pbData:
            kernel32.LocalFree(ctypes.cast(output.pbData, ctypes.c_void_p))


def json_bytes(value: dict) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def read_json(path: Path) -> dict:
    try:
        value = json.loads(require_private_file(path).decode())
    except (UnicodeDecodeError, json.JSONDecodeError):
        fail(f"delegate state is invalid: {path.name} is not valid JSON")
    if not isinstance(value, dict):
        fail(f"delegate state is invalid: {path.name} must contain an object")
    return value


def manifest_data(path: Path) -> tuple[dict, str]:
    raw = path.read_bytes()
    try:
        manifest = json.loads(raw)
    except json.JSONDecodeError:
        fail("bounded-wallet manifest is invalid JSON")
    validate_manifest(manifest)
    return manifest, hashlib.sha256(raw).hexdigest()


def public_address(state_dir: Path) -> str:
    metadata = read_json(state_dir / METADATA)
    if metadata.get("schema") != STATE_SCHEMA:
        fail("local delegate state schema is unsupported")
    address = require_address(str(metadata.get("delegate", "")), "delegate")
    keystore = read_json(state_dir / KEYSTORE)
    stored_address = require_address(f"0x{keystore.get('address', '')}", "keystore address")
    if stored_address != address:
        fail("delegate metadata does not match the encrypted keystore")
    return address


def initialize(state_dir: Path) -> dict:
    if state_dir.exists():
        fail("delegate state already exists; rotate by creating a new directory and owner policy")
    state_dir.parent.mkdir(parents=True, exist_ok=True)
    state_dir.mkdir()
    try:
        harden_acl(state_dir)
        account = Account.create(extra_entropy=secrets.token_bytes(32))
        password = bytearray(secrets.token_urlsafe(48).encode())
        recovered_password = bytearray()
        recovered_key = bytearray()
        try:
            encrypted = Account.encrypt(account.key, bytes(password))
            protected = protect_secret(bytes(password))
            address = require_address(account.address, "generated delegate")
            metadata = {
                "schema": STATE_SCHEMA,
                "delegate": address,
                "created_at": int(time.time()),
                "protection": "web3-secret-storage-v3+scrypt+windows-dpapi-current-user",
                "bound": False,
            }
            write_exclusive(state_dir / KEYSTORE, json_bytes(encrypted))
            write_exclusive(state_dir / DPAPI_BLOB, protected)
            write_exclusive(state_dir / METADATA, json_bytes(metadata))

            recovered_password.extend(unprotect_secret(require_private_file(state_dir / DPAPI_BLOB)))
            if not secrets.compare_digest(password, recovered_password):
                fail("delegate credential failed its post-write verification")
            recovered_key.extend(Account.decrypt(read_json(state_dir / KEYSTORE), bytes(recovered_password)))
            recovered_address = require_address(Account.from_key(bytes(recovered_key)).address, "recovered delegate")
            if recovered_address != address:
                fail("delegate keystore failed its post-write address verification")
            return {"initialized": True, "delegate": address, "bound": False}
        finally:
            for index in range(len(password)):
                password[index] = 0
            for index in range(len(recovered_password)):
                recovered_password[index] = 0
            for index in range(len(recovered_key)):
                recovered_key[index] = 0
    except BaseException:
        for name in (KEYSTORE, DPAPI_BLOB, METADATA):
            path = state_dir / name
            if path.is_file() and not path.is_symlink():
                path.unlink(missing_ok=True)
        state_dir.rmdir()
        raise


def status(state_dir: Path) -> dict:
    if not state_dir.exists():
        return {"initialized": False, "bound": False}
    if state_dir.is_symlink() or not state_dir.is_dir():
        fail("delegate state directory is unsafe")
    address = public_address(state_dir)
    binding_path = state_dir / BINDING
    result = {"initialized": True, "delegate": address, "bound": binding_path.exists()}
    if binding_path.exists():
        binding = read_json(binding_path)
        if binding.get("schema") != BINDING_SCHEMA or binding.get("delegate") != address:
            fail("delegate binding is invalid")
        result.update(
            {
                "network": binding.get("network"),
                "wallet": binding.get("wallet"),
                "owner": binding.get("owner"),
                "policy_hash": binding.get("policy_hash"),
                "bound_at_block": binding.get("bound_at_block"),
            }
        )
    return result


def bind(
    state_dir: Path,
    manifest_path: Path,
    rpc_url: str | None,
    wallet_value: str,
    owner_value: str,
    policy_hash_value: str,
) -> dict:
    delegate = public_address(state_dir)
    manifest, digest = manifest_data(manifest_path)
    wallet = require_address(wallet_value, "wallet")
    owner = require_address(owner_value, "owner")
    policy_hash = require_bytes32(policy_hash_value, "policy hash")
    report = inspect(rpc_url or manifest["rpc_url"], wallet, manifest, owner, delegate, policy_hash)
    if not report["ready"]:
        fail(f"bounded wallet inspection failed: {', '.join(report['failures'])}")
    binding = {
        "schema": BINDING_SCHEMA,
        "network": EXPECTED_NETWORK,
        "chain_id": EXPECTED_CHAIN_ID,
        "delegate": delegate,
        "wallet": wallet,
        "owner": owner,
        "policy_hash": policy_hash,
        "policy_version": report["state"]["policy_version"],
        "wallet_factory": report["wallet_factory"],
        "settlement_token": manifest["canonical"]["settlement_token"],
        "manifest_sha256": digest,
        "contract_source_revision": manifest["contract_source_revision"],
        "bound_at_block": report["safe_block"],
    }
    path = state_dir / BINDING
    if path.exists() and read_json(path) != binding:
        fail("delegate is already bound; use a new delegate and owner policy to rotate")
    if not path.exists():
        write_atomic(path, json_bytes(binding))
    metadata = read_json(state_dir / METADATA)
    metadata["bound"] = True
    write_atomic(state_dir / METADATA, json_bytes(metadata))
    return binding


def require_uint(value: object, label: str) -> int:
    if isinstance(value, bool) or not str(value).isdigit():
        fail(f"{label} must be an unsigned integer")
    return int(str(value))


def validate_plan(
    plan: dict,
    binding: dict,
    manifest: dict,
    report: dict,
    observed: dict,
    plan_block: dict,
) -> dict:
    if plan.get("schema") != PLAN_SCHEMA:
        fail("action plan schema is unsupported")
    if plan.get("network") != EXPECTED_NETWORK or manifest["chain_id"] != EXPECTED_CHAIN_ID:
        fail("action plan must target Base mainnet")
    wallet = require_address(str(plan.get("wallet", "")), "plan wallet")
    delegate = require_address(str(plan.get("delegate", "")), "plan delegate")
    bounty = require_address(str(plan.get("bounty", "")), "plan bounty")
    policy_hash = require_bytes32(str(plan.get("policy_hash", "")), "plan policy hash")
    if wallet != binding["wallet"] or delegate != binding["delegate"]:
        fail("action plan does not match the bound wallet and delegate")
    if policy_hash != binding["policy_hash"]:
        fail("action plan policy hash does not match the binding")
    if not report["ready"]:
        fail(f"bounded wallet inspection failed: {', '.join(report['failures'])}")
    if report["state"]["owner"] != binding["owner"]:
        fail("live wallet owner does not match the binding")
    if report["state"]["policy_hash"] != binding["policy_hash"]:
        fail("live wallet policy does not match the binding")
    if report["state"]["policy_version"] != binding["policy_version"]:
        fail("live wallet policy version changed; rotate and rebind")

    planned_block = plan.get("safe_block") or {}
    if not isinstance(planned_block, dict) or plan_block.get("hash") != planned_block.get("hash"):
        fail("action plan safe block is no longer canonical")
    planned_timestamp = require_uint(planned_block.get("timestamp"), "plan timestamp")
    current_timestamp = require_uint(report["safe_block"].get("timestamp"), "safe timestamp")
    if current_timestamp < planned_timestamp or current_timestamp - planned_timestamp > MAX_PLAN_AGE_SECONDS:
        fail("action plan is stale; generate a fresh same-state plan")
    action = str(plan.get("action", ""))
    if action not in ACTIONS or plan.get("action_code") != ACTIONS[action]:
        fail("action plan has an invalid action binding")
    if plan.get("bounty_state") != observed:
        fail("action state changed after planning; generate a fresh plan")
    policy = report["state"]["policy"]
    if not policy["allowed_actions"] & (1 << ACTIONS[action]):
        fail("action is not enabled by the live wallet policy")

    summary = plan.get("action_summary")
    if not isinstance(summary, dict):
        fail("action plan summary is missing")
    if action == "create":
        creation_plan = plan.get("creation_plan")
        if not isinstance(creation_plan, dict):
            fail("create action plan is missing its canonical creation plan")
        intent = creation_plan.get("create_bounty")
        if not isinstance(intent, dict):
            fail("create action plan is missing its transaction intent")
        decoded = decode_create_calldata(intent.get("data"))
        spend = int(decoded["initial_funding"])
        payload = str(decoded["payload"])
        direct_data = str(decoded["direct_data"])
        expected_summary = {
            "bounty_id": observed["bounty_id"],
            "predicted_bounty_contract": bounty,
            "target_amount": str(observed["target_amount"]),
            "initial_funding": str(spend),
            "terms_hash": observed["terms_hash"],
            "creation_nonce": observed["creation_nonce"],
        }
        if summary != expected_summary:
            fail("create action summary does not match canonical calldata")
    else:
        validate_bounty_policy(
            observed,
            policy,
            require_address(manifest["canonical"]["bounty_factory"], "bounty factory"),
            require_address(manifest["canonical"]["settlement_token"], "settlement token"),
        )

    if action == "fund":
        requested = require_uint(summary.get("requested_amount"), "requested amount")
        if observed["status"] != 0 or observed["funded_amount"] >= observed["target_amount"]:
            fail("bounty is not open for funding")
        if current_timestamp > observed["funding_deadline"]:
            fail("bounty funding deadline has passed")
        spend = min(requested, observed["target_amount"] - observed["funded_amount"])
        payload = encode("f(address,uint256)", bounty, str(requested))
        direct_data = calldata("fundBounty(address,uint256)", bounty, str(requested))
        if require_uint(summary.get("maximum_accepted_amount"), "maximum accepted amount") != spend:
            fail("planned accepted funding amount changed")
    elif action == "claim":
        if observed["status"] != 1 or observed["solver"] != ZERO_ADDRESS:
            fail("bounty is not claimable")
        if observed["creator"] == wallet:
            fail("creator wallet cannot claim its own bounty")
        spend = observed["verifier_reward"]
        payload = encode("f(address)", bounty)
        direct_data = calldata("claimBounty(address)", bounty)
        if require_uint(summary.get("claim_bond"), "claim bond") != spend:
            fail("planned claim bond changed")
    elif action == "submit":
        submission_hash = require_bytes32(str(summary.get("submission_hash", "")), "submission hash")
        evidence_hash = require_bytes32(str(summary.get("evidence_hash", "")), "evidence hash")
        if observed["status"] != 2 or observed["solver"] != wallet:
            fail("wallet does not own the active claim")
        if current_timestamp > observed["claim_expires_at"]:
            fail("claim has expired")
        spend = 0
        payload = encode("f(address,bytes32,bytes32)", bounty, submission_hash, evidence_hash)
        direct_data = calldata("submitBounty(address,bytes32,bytes32)", bounty, submission_hash, evidence_hash)

    validate_spend(report, spend)
    if plan.get("payload") != payload or plan.get("payload_hash") != keccak_hex(payload):
        fail("action plan payload does not match its declared action")
    expected_transaction = {"from": delegate, "to": wallet, "data": direct_data, "value": "0x0"}
    if plan.get("direct_transaction") != expected_transaction:
        fail("action plan contains an unexpected target, sender, value, or calldata")
    if require_uint(plan.get("maximum_gross_spend"), "maximum gross spend") != spend:
        fail("action plan spend does not match the live state")
    validate_relay_authorization(plan, binding, report, payload)
    return expected_transaction


def validate_relay_authorization(
    plan: dict,
    binding: dict,
    report: dict,
    payload: str,
) -> dict:
    action = str(plan["action"])
    action_code = ACTIONS[action]
    nonce = int(report["state"]["delegate_nonce"])
    policy_version = int(report["state"]["policy_version"])
    typed = plan.get("relay_authorization_typed_data")
    if not isinstance(typed, dict):
        fail("action plan relay authorization is missing")
    message = typed.get("message")
    if not isinstance(message, dict):
        fail("action plan relay authorization message is missing")
    deadline = require_uint(message.get("deadline"), "relay deadline")
    now = require_uint(report["safe_block"].get("timestamp"), "safe timestamp")
    if deadline < now or deadline > now + 900:
        fail("relay authorization deadline is expired or exceeds fifteen minutes")
    expected = {
        "types": {
            "EIP712Domain": [
                {"name": "name", "type": "string"},
                {"name": "version", "type": "string"},
                {"name": "chainId", "type": "uint256"},
                {"name": "verifyingContract", "type": "address"},
            ],
            "AgentAction": [
                {"name": "wallet", "type": "address"},
                {"name": "action", "type": "uint8"},
                {"name": "payloadHash", "type": "bytes32"},
                {"name": "nonce", "type": "uint256"},
                {"name": "deadline", "type": "uint256"},
                {"name": "policyVersion", "type": "uint64"},
            ],
        },
        "primaryType": "AgentAction",
        "domain": {
            "name": "Agent Bounties Bounded Wallet",
            "version": "1",
            "chainId": EXPECTED_CHAIN_ID,
            "verifyingContract": binding["wallet"],
        },
        "message": {
            "wallet": binding["wallet"],
            "action": action_code,
            "payloadHash": keccak_hex(payload),
            "nonce": str(nonce),
            "deadline": str(deadline),
            "policyVersion": policy_version,
        },
    }
    if typed != expected:
        fail("action plan relay authorization does not match live policy and nonce")
    expected_call = {
        "to": binding["wallet"],
        "function": "executeWithSignature(uint8,bytes,uint256,uint256,bytes)",
        "arguments_before_signature": [action_code, payload, nonce, deadline],
        "signature_tail": ["delegate_signature"],
    }
    if plan.get("relay_call") != expected_call:
        fail("action plan relay call does not match its authorization")
    return typed


def load_bound_context(state_dir: Path, manifest_path: Path) -> tuple[str, dict, dict, dict]:
    delegate = public_address(state_dir)
    binding = read_json(state_dir / BINDING)
    if binding.get("schema") != BINDING_SCHEMA or binding.get("delegate") != delegate:
        fail("delegate binding is invalid")
    manifest, digest = manifest_data(manifest_path)
    if digest != binding.get("manifest_sha256"):
        fail("bounded-wallet manifest changed after binding")
    return delegate, binding, manifest, read_json(state_dir / KEYSTORE)


def load_validated_action(
    state_dir: Path,
    manifest_path: Path,
    plan_path: Path,
    rpc_url_override: str | None,
) -> tuple[str, dict, dict, dict, dict, str, dict, dict]:
    delegate, binding, manifest, keystore = load_bound_context(state_dir, manifest_path)
    plan = json.loads(plan_path.read_text(encoding="utf-8"))
    if not isinstance(plan, dict):
        fail("action plan must contain a JSON object")
    rpc_url = rpc_url_override or manifest["rpc_url"]
    report = inspect(
        rpc_url,
        binding["wallet"],
        manifest,
        binding["owner"],
        delegate,
        binding["policy_hash"],
    )
    bounty = require_address(str(plan.get("bounty", "")), "plan bounty")
    if plan.get("action") == "create":
        created = validate_creation_plan(
            plan.get("creation_plan"),
            binding["wallet"],
            manifest,
            report,
            rpc_url,
            hex(report["safe_block"]["number"]),
        )
        if created["bounty"] != bounty:
            fail("create action bounty does not match the canonical prediction")
        observed = created["summary"]
    else:
        observed = bounty_state(
            rpc_url,
            bounty,
            require_address(manifest["canonical"]["bounty_factory"], "bounty factory"),
            hex(report["safe_block"]["number"]),
        )
    planned_number = require_uint((plan.get("safe_block") or {}).get("number"), "plan block number")
    plan_block = rpc(rpc_url, "eth_getBlockByNumber", [hex(planned_number), False], 100)
    if not isinstance(plan_block, dict):
        fail("action plan safe block is unavailable")
    direct = validate_plan(plan, binding, manifest, report, observed, plan_block)
    return delegate, binding, manifest, keystore, plan, rpc_url, report, direct


def transaction_parameters(rpc_url: str, transaction: dict) -> dict:
    delegate = transaction["from"]
    simulation = rpc(rpc_url, "eth_call", [transaction, "latest"], 101)
    if not isinstance(simulation, str) or not simulation.startswith("0x"):
        fail("transaction simulation returned an invalid result")
    estimated = require_uint(int(str(rpc(rpc_url, "eth_estimateGas", [transaction], 102)), 16), "gas estimate")
    gas = estimated + max(estimated // 5, 10_000)
    if gas > MAX_GAS_LIMIT:
        fail("estimated gas exceeds the local delegate cap")
    latest = rpc(rpc_url, "eth_getBlockByNumber", ["latest", False], 103)
    if not isinstance(latest, dict) or "baseFeePerGas" not in latest:
        fail("Base RPC omitted the latest base fee")
    base_fee = int(str(latest["baseFeePerGas"]), 16)
    try:
        priority_fee = int(str(rpc(rpc_url, "eth_maxPriorityFeePerGas", [], 104)), 16)
    except RuntimeError:
        gas_price = int(str(rpc(rpc_url, "eth_gasPrice", [], 104)), 16)
        priority_fee = max(gas_price - base_fee, 100_000)
    max_fee = base_fee * 2 + priority_fee
    if max_fee > MAX_FEE_PER_GAS_WEI or gas * max_fee > MAX_TOTAL_GAS_WEI:
        fail("estimated Base gas cost exceeds the local delegate cap")
    balance = int(str(rpc(rpc_url, "eth_getBalance", [delegate, "latest"], 105)), 16)
    if balance < gas * max_fee:
        fail("delegate needs more Base ETH for gas")
    nonce = int(str(rpc(rpc_url, "eth_getTransactionCount", [delegate, "pending"], 106)), 16)
    return {
        "chainId": EXPECTED_CHAIN_ID,
        "from": to_checksum_address(delegate),
        "to": to_checksum_address(transaction["to"]),
        "data": transaction["data"],
        "value": 0,
        "nonce": nonce,
        "gas": gas,
        "maxFeePerGas": max_fee,
        "maxPriorityFeePerGas": priority_fee,
        "type": 2,
    }


def wait_for_receipt(rpc_url: str, transaction_hash: str, timeout: int) -> dict:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        receipt = rpc(rpc_url, "eth_getTransactionReceipt", [transaction_hash], 110)
        if receipt is not None:
            if not isinstance(receipt, dict) or receipt.get("status") != "0x1":
                fail("delegate transaction reverted")
            return receipt
        time.sleep(2)
    fail("delegate transaction was broadcast but its receipt timed out; reconcile by transaction hash")


def rpc_hex(value: bytes) -> str:
    encoded = value.hex()
    return encoded if encoded.startswith("0x") else f"0x{encoded}"


def execute(
    state_dir: Path,
    manifest_path: Path,
    plan_path: Path,
    rpc_url_override: str | None,
    broadcast: bool,
    receipt_timeout: int,
) -> dict:
    delegate, binding, _, keystore, plan, rpc_url, _, direct = load_validated_action(
        state_dir, manifest_path, plan_path, rpc_url_override
    )
    bounty = require_address(str(plan.get("bounty", "")), "plan bounty")
    prepared = transaction_parameters(rpc_url, direct)
    result = {
        "schema": "agent-bounties/local-delegate-execution-v1",
        "network": EXPECTED_NETWORK,
        "delegate": delegate,
        "wallet": binding["wallet"],
        "action": plan["action"],
        "bounty": bounty,
        "maximum_gross_spend": plan["maximum_gross_spend"],
        "gas_limit": prepared["gas"],
        "max_fee_per_gas_wei": prepared["maxFeePerGas"],
        "broadcast": broadcast,
        "evidence_boundary": (
            "A successful transaction applies one bounded wallet action. Only reconciled canonical bounty "
            "events prove funding, claim, submission, or payout state."
        ),
    }
    if not broadcast:
        return result

    protected = require_private_file(state_dir / DPAPI_BLOB)
    password = bytearray(unprotect_secret(protected))
    private_key = bytearray()
    try:
        private_key = bytearray(Account.decrypt(keystore, bytes(password)))
        account = Account.from_key(bytes(private_key))
        if require_address(account.address, "decrypted delegate") != delegate:
            fail("decrypted key does not match the bound delegate")
        signed = account.sign_transaction(prepared)
        expected_hash = rpc_hex(signed.hash).lower()
        transaction_hash = str(
            rpc(rpc_url, "eth_sendRawTransaction", [rpc_hex(signed.raw_transaction)], 107)
        ).lower()
        if transaction_hash != expected_hash:
            fail("RPC returned a transaction hash that does not match the signed transaction")
    finally:
        for secret in (password, private_key):
            for index in range(len(secret)):
                secret[index] = 0
    receipt = wait_for_receipt(rpc_url, transaction_hash, receipt_timeout)
    result.update(
        {
            "transaction_hash": transaction_hash,
            "block_number": int(str(receipt["blockNumber"]), 16),
            "receipt_status": "confirmed-success",
        }
    )
    return result


def sign_plan(
    state_dir: Path,
    manifest_path: Path,
    plan_path: Path,
    rpc_url_override: str | None,
    issue_number: int,
) -> dict:
    delegate, binding, _, keystore, plan, _, report, _ = load_validated_action(
        state_dir, manifest_path, plan_path, rpc_url_override
    )
    if plan.get("action") != "create":
        fail("the public bounded-wallet relay currently accepts create plans only")
    if issue_number <= 0:
        fail("issue number must be positive")
    typed_data = plan["relay_authorization_typed_data"]
    protected = require_private_file(state_dir / DPAPI_BLOB)
    password = bytearray(unprotect_secret(protected))
    private_key = bytearray()
    try:
        private_key = bytearray(Account.decrypt(keystore, bytes(password)))
        account = Account.from_key(bytes(private_key))
        if require_address(account.address, "decrypted delegate") != delegate:
            fail("decrypted key does not match the bound delegate")
        signed = Account.sign_typed_data(bytes(private_key), full_message=typed_data)
        signature = rpc_hex(signed.signature).lower()
    finally:
        for secret in (password, private_key):
            for index in range(len(secret)):
                secret[index] = 0
    message = typed_data["message"]
    summary = plan["action_summary"]
    return {
        "schema": RELAY_SCHEMA,
        "network": EXPECTED_NETWORK,
        "action": "create",
        "issue_number": issue_number,
        "wallet": binding["wallet"],
        "policy_hash": binding["policy_hash"],
        "policy_version": int(report["state"]["policy_version"]),
        "nonce": require_uint(message["nonce"], "relay nonce"),
        "deadline": require_uint(message["deadline"], "relay deadline"),
        "payload": plan["payload"],
        "payload_hash": plan["payload_hash"],
        "signature": signature,
        "bounty_id": summary["bounty_id"],
        "predicted_bounty_contract": summary["predicted_bounty_contract"],
    }


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(
        description="Operate one bounded Base wallet without exposing the owner wallet or arbitrary signing."
    )
    root.add_argument("--state-dir", help="Private state outside the repository")
    commands = root.add_subparsers(dest="command", required=True)
    commands.add_parser("init", help="Create a DPAPI-protected delegate and print only its address")
    commands.add_parser("status", help="Print public initialization and binding state")

    bind_parser = commands.add_parser("bind", help="Pin the delegate to one inspected wallet policy")
    bind_parser.add_argument("--wallet", required=True)
    bind_parser.add_argument("--expect-owner", required=True)
    bind_parser.add_argument("--expect-policy-hash", required=True)
    bind_parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    bind_parser.add_argument("--rpc-url")

    execute_parser = commands.add_parser("execute-plan", help="Validate and optionally broadcast one plan")
    execute_parser.add_argument("--plan", type=Path, required=True)
    execute_parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    execute_parser.add_argument("--rpc-url")
    execute_parser.add_argument("--broadcast", action="store_true")
    execute_parser.add_argument("--receipt-timeout", type=int, default=120)

    sign_parser = commands.add_parser(
        "sign-plan", help="Sign one exact plan for the capped public gas sponsor"
    )
    sign_parser.add_argument("--plan", type=Path, required=True)
    sign_parser.add_argument("--issue-number", type=int, required=True)
    sign_parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    sign_parser.add_argument("--rpc-url")
    sign_parser.add_argument(
        "--output", type=Path, default=ROOT / "target" / "bounded-wallet-relay-envelope.json"
    )
    return root


def main() -> None:
    args = parser().parse_args()
    state_dir = resolve_state_dir(args.state_dir)
    if args.command == "init":
        result = initialize(state_dir)
    elif args.command == "status":
        result = status(state_dir)
    elif args.command == "bind":
        result = bind(
            state_dir,
            args.manifest,
            args.rpc_url,
            args.wallet,
            args.expect_owner,
            args.expect_policy_hash,
        )
    elif args.command == "execute-plan":
        if args.receipt_timeout < 15 or args.receipt_timeout > 600:
            fail("receipt-timeout must be between 15 and 600 seconds")
        result = execute(
            state_dir,
            args.manifest,
            args.plan,
            args.rpc_url,
            args.broadcast,
            args.receipt_timeout,
        )
    else:
        result = sign_plan(
            state_dir,
            args.manifest,
            args.plan,
            args.rpc_url,
            args.issue_number,
        )
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
