#!/usr/bin/env python3
"""Operate the exact owner-recoverable Open Competition V2 GMV reserve locally.

The delegate key is loaded only from the existing Windows-DPAPI protected
keystore. Every action is rebuilt from reviewed inputs, simulated on two Base
RPCs at one safe block, written to the private crash ledger before broadcast,
and reconciled canonically before another action can start.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import os
import re
import stat
import tempfile
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterator

from eth_abi import decode, encode
from eth_account import Account
from eth_utils import keccak, to_checksum_address

from build_open_competition_v2_gmv_activation import (
    ACTIVATION_SCHEMA,
    CHAIN_ID,
    DAILY_CAP,
    INITIAL_FUNDING,
    OWNER,
    PER_COMPETITION,
    build_activation,
)
from build_open_competition_v2_gmv_relay import build_relay
TARGET = 10
FLOOR = 5
CONFIRMATIONS = 2
RECEIPT_TIMEOUT = 240
LEDGER_SCHEMA = "agent-bounties/local-open-competition-v2-gmv-guard-ledger-v1"
STATE_SCHEMA = "agent-bounties/local-open-competition-v2-gmv-guard-state-v1"
LEDGER_FILE = "gmv-guard-ledger.json"
LOCK_FILE = "gmv-guard.lock"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
ZERO_HASH = "0x" + "00" * 32
HASH = re.compile(r"^0x[0-9a-f]{64}$")
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
KEYSTORE = "keystore.json"
DPAPI_BLOB = "credential.dpapi"
METADATA = "state.json"
STATE_KEY_SCHEMA = "agent-bounties/local-delegate-state-v1"
MAX_GAS_LIMIT = 2_000_000
MAX_FEE_PER_GAS_WEI = 2_000_000_000
MAX_TOTAL_GAS_WEI = 3_000_000_000_000_000
RPC_MIN_INTERVAL_SECONDS = 1.1
RPC_RETRY_DELAYS = (0.5, 1.0, 2.0, 4.0)
_LAST_RPC_REQUEST: dict[str, float] = {}


class GuardError(ValueError):
    pass


def rpc(url: str, method: str, params: list[object], request_id: int) -> object:
    for attempt in range(len(RPC_RETRY_DELAYS) + 1):
        now = time.monotonic()
        wait = _LAST_RPC_REQUEST.get(url, 0.0) + RPC_MIN_INTERVAL_SECONDS - now
        if wait > 0:
            time.sleep(wait)
        _LAST_RPC_REQUEST[url] = time.monotonic()
        request = urllib.request.Request(
            url,
            data=json.dumps(
                {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
            ).encode(),
            headers={"content-type": "application/json", "user-agent": "agent-bounties-gmv-guard/1"},
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:  # nosec B310
                payload = json.load(response)
        except urllib.error.HTTPError as error:
            retryable = error.code == 429 or 500 <= error.code < 600
            if retryable and attempt < len(RPC_RETRY_DELAYS):
                time.sleep(RPC_RETRY_DELAYS[attempt])
                continue
            raise GuardError(f"RPC {method} failed") from error
        except (OSError, urllib.error.URLError, json.JSONDecodeError) as error:
            if attempt < len(RPC_RETRY_DELAYS):
                time.sleep(RPC_RETRY_DELAYS[attempt])
                continue
            raise GuardError(f"RPC {method} failed") from error
        error = payload.get("error") if isinstance(payload, dict) else None
        error_message = str(error.get("message") or "").lower() if isinstance(error, dict) else ""
        if error is not None and "rate limit" in error_message and attempt < len(RPC_RETRY_DELAYS):
            time.sleep(RPC_RETRY_DELAYS[attempt])
            continue
        if not isinstance(payload, dict) or error is not None or "result" not in payload:
            raise GuardError(f"RPC {method} returned an error")
        return payload["result"]
    raise GuardError(f"RPC {method} failed")


def require_private_file(path: Path) -> bytes:
    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise GuardError(f"private state file {path.name} is unsafe")
    return path.read_bytes()


def read_json(path: Path) -> dict[str, Any]:
    value = json.loads(require_private_file(path).decode())
    if not isinstance(value, dict):
        raise GuardError(f"private state file {path.name} must contain an object")
    return value


def json_bytes(value: dict[str, Any]) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def write_atomic(path: Path, data: bytes) -> None:
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        view = memoryview(data)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise GuardError("private state write made no progress")
            view = view[written:]
        os.fsync(descriptor)
        os.close(descriptor)
        descriptor = -1
        os.replace(temporary, path)
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        if os.path.exists(temporary):
            os.unlink(temporary)


def public_address(state_dir: Path) -> str:
    metadata = read_json(state_dir / METADATA)
    delegate = normalized(metadata.get("delegate"))
    keystore = read_json(state_dir / KEYSTORE)
    stored = "0x" + normalized(keystore.get("address")).removeprefix("0x")
    if (
        metadata.get("schema") != STATE_KEY_SCHEMA
        or not ADDRESS.fullmatch(delegate)
        or stored != delegate
    ):
        raise GuardError("protected delegate metadata and keystore do not agree")
    return delegate


def rpc_hex(value: bytes) -> str:
    encoded = value.hex()
    return encoded if encoded.startswith("0x") else f"0x{encoded}"


def transaction_parameters(rpc_url: str, transaction: dict[str, Any]) -> dict[str, Any]:
    delegate = transaction["from"]
    simulation = rpc(rpc_url, "eth_call", [transaction, "latest"], 490)
    if not isinstance(simulation, str) or not simulation.startswith("0x"):
        raise GuardError("transaction simulation returned invalid data")
    estimated = int(str(rpc(rpc_url, "eth_estimateGas", [transaction], 491)), 16)
    gas = estimated + max(estimated // 5, 10_000)
    if gas > MAX_GAS_LIMIT:
        raise GuardError("estimated gas exceeds the local delegate cap")
    latest = rpc(rpc_url, "eth_getBlockByNumber", ["latest", False], 492)
    if not isinstance(latest, dict) or "baseFeePerGas" not in latest:
        raise GuardError("Base RPC omitted the latest base fee")
    base_fee = int(str(latest["baseFeePerGas"]), 16)
    try:
        priority_fee = int(str(rpc(rpc_url, "eth_maxPriorityFeePerGas", [], 493)), 16)
    except GuardError:
        gas_price = int(str(rpc(rpc_url, "eth_gasPrice", [], 494)), 16)
        priority_fee = max(gas_price - base_fee, 100_000)
    max_fee = base_fee * 2 + priority_fee
    if max_fee > MAX_FEE_PER_GAS_WEI or gas * max_fee > MAX_TOTAL_GAS_WEI:
        raise GuardError("estimated Base gas cost exceeds the local delegate cap")
    balance = int(str(rpc(rpc_url, "eth_getBalance", [delegate, "latest"], 495)), 16)
    if balance < gas * max_fee:
        raise GuardError("delegate needs more Base ETH for gas")
    nonce = int(str(rpc(rpc_url, "eth_getTransactionCount", [delegate, "pending"], 496)), 16)
    return {
        "chainId": CHAIN_ID,
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


def load_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8-sig"))
    if not isinstance(value, dict):
        raise GuardError(f"{path} must contain an object")
    return value


def normalized(value: object) -> str:
    return str(value or "").lower()


def call_data(signature: str, types: list[str] | None = None, values: list[object] | None = None) -> str:
    return "0x" + (
        keccak(text=signature)[:4] + encode(types or [], values or [])
    ).hex()


def contract_call(
    rpc_url: str,
    target: str,
    signature: str,
    output_types: list[str],
    block: int,
    input_types: list[str] | None = None,
    input_values: list[object] | None = None,
) -> tuple[Any, ...]:
    result = rpc(
        rpc_url,
        "eth_call",
        [
            {
                "to": target,
                "data": call_data(signature, input_types, input_values),
            },
            hex(block),
        ],
        501,
    )
    if not isinstance(result, str) or not result.startswith("0x"):
        raise GuardError(f"{signature} returned malformed RPC data")
    try:
        return decode(output_types, bytes.fromhex(result[2:]))
    except Exception as error:  # eth_abi exposes several decode exception types
        raise GuardError(f"{signature} returned undecodable RPC data") from error


def code_at(rpc_url: str, target: str, block: int) -> str:
    value = rpc(rpc_url, "eth_getCode", [target, hex(block)], 502)
    if not isinstance(value, str) or not value.startswith("0x"):
        raise GuardError("RPC returned malformed contract code")
    return value.lower()


def code_hash(code: str) -> str:
    return "0x" + keccak(bytes.fromhex(code.removeprefix("0x"))).hex()


def safe_block(primary: str, shadow: str, minimum: int = 1) -> dict[str, Any]:
    primary_head = int(str(rpc(primary, "eth_blockNumber", [], 503)), 16)
    shadow_head = int(str(rpc(shadow, "eth_blockNumber", [], 504)), 16)
    number = min(primary_head, shadow_head) - CONFIRMATIONS
    if number < minimum:
        raise GuardError("dual-RPC safe block is not yet available")
    blocks = [
        rpc(url, "eth_getBlockByNumber", [hex(number), False], 505)
        for url in (primary, shadow)
    ]
    if any(not isinstance(value, dict) for value in blocks):
        raise GuardError("dual-RPC safe block is unavailable")
    hashes = [normalized(value.get("hash")) for value in blocks]
    timestamps = [int(str(value.get("timestamp")), 16) for value in blocks]
    if (
        not HASH.fullmatch(hashes[0])
        or hashes[0] == ZERO_HASH
        or hashes[0] != hashes[1]
        or timestamps[0] != timestamps[1]
    ):
        raise GuardError("primary and shadow RPCs disagree at the safe block")
    return {"number": number, "hash": hashes[0], "timestamp": timestamps[0]}


def validate_reviewed_inputs(
    release: dict[str, Any],
    reserve_deployment: dict[str, Any],
    pool: dict[str, Any],
    bundle: dict[str, Any],
    delegate: str,
) -> None:
    if bundle.get("schema_version") != ACTIVATION_SCHEMA:
        raise GuardError("activation bundle schema is invalid")
    if normalized(bundle.get("delegate")) != delegate:
        raise GuardError("activation bundle is not bound to the protected delegate")
    if normalized(bundle.get("owner")) != OWNER:
        raise GuardError("activation bundle owner is invalid")
    valid_after = int(bundle.get("policy", {}).get("valid_after", -1))
    if valid_after < 0:
        raise GuardError("activation policy time is invalid")
    activation_time = datetime.fromtimestamp(valid_after + 60, tz=timezone.utc)
    rebuilt = build_activation(
        release,
        reserve_deployment,
        pool,
        activation_time,
        owner=OWNER,
        delegate=delegate,
    )
    if rebuilt != bundle:
        raise GuardError("activation bundle is not the exact deterministic reviewed build")


def expected_proxy_hash(implementation: str) -> str:
    runtime = (
        "363d3d373d3d3d363d73"
        + implementation.removeprefix("0x")
        + "5af43d82803e903d91602b57fd5bf3"
    )
    return "0x" + keccak(bytes.fromhex(runtime)).hex()


def dual_equal(primary: str, shadow: str, function) -> Any:
    left = function(primary)
    right = function(shadow)
    if left != right:
        raise GuardError("primary and shadow RPC evidence disagrees")
    return left


def inspect_state(
    primary: str,
    shadow: str,
    release: dict[str, Any],
    reserve_deployment: dict[str, Any],
    bundle: dict[str, Any],
    *,
    minimum_block: int = 1,
) -> dict[str, Any]:
    safe = safe_block(primary, shadow, minimum_block)
    block = safe["number"]
    if int(str(rpc(primary, "eth_chainId", [], 506)), 16) != CHAIN_ID or int(
        str(rpc(shadow, "eth_chainId", [], 507)), 16
    ) != CHAIN_ID:
        raise GuardError("both RPCs must be Base mainnet")

    factory = normalized(bundle["competition_factory"])
    reserve_factory = normalized(bundle["reserve_factory"])
    reserve = normalized(bundle["reserve_wallet"])
    manifest_factory = reserve_deployment["reserve_factory"]
    checks = (
        (factory, normalized(release["factory_runtime_code_hash"]), "competition factory"),
        (
            normalized(release["implementation_contract"]),
            normalized(release["implementation_runtime_code_hash"]),
            "competition implementation",
        ),
        (reserve_factory, normalized(manifest_factory["runtime_code_hash"]), "reserve factory"),
        (
            normalized(manifest_factory["implementation"]),
            normalized(manifest_factory["implementation_runtime_code_hash"]),
            "reserve implementation",
        ),
    )
    for target, expected, label in checks:
        observed = dual_equal(primary, shadow, lambda url, t=target: code_hash(code_at(url, t, block)))
        if observed != expected:
            raise GuardError(f"{label} runtime hash differs from the reviewed release")

    reserve_code = dual_equal(primary, shadow, lambda url: code_at(url, reserve, block))
    if reserve_code == "0x":
        return {
            "schema": STATE_SCHEMA,
            "safe_block": safe,
            "reserve_deployed": False,
            "active": 0,
            "floor": FLOOR,
            "target": TARGET,
            "severity": "critical",
        }
    if code_hash(reserve_code) != normalized(manifest_factory["clone_runtime_code_hash"]):
        raise GuardError("reserve runtime differs from the reviewed deterministic clone")

    def view(url: str, signature: str, outputs: list[str], inputs=None, values=None):
        return contract_call(url, reserve, signature, outputs, block, inputs, values)

    owner = normalized(dual_equal(primary, shadow, lambda url: view(url, "owner()", ["address"])[0]))
    live_delegate = normalized(
        dual_equal(
            primary,
            shadow,
            lambda url: view(
                url,
                "policy()",
                [
                    "address",
                    "uint64",
                    "uint64",
                    "uint64",
                    "uint256",
                    "uint256",
                    "uint256",
                    "uint256",
                    "uint256",
                    "bytes32",
                    "bytes32",
                    "bytes32",
                ],
            ),
        )[0]
    )
    policy_version = int(
        dual_equal(primary, shadow, lambda url: view(url, "policyVersion()", ["uint64"])[0])
    )
    policy_hash = "0x" + dual_equal(
        primary, shadow, lambda url: view(url, "activePolicyHash()", ["bytes32"])[0]
    ).hex()
    initial_policy_hash = "0x" + dual_equal(
        primary, shadow, lambda url: view(url, "initialPolicyHash()", ["bytes32"])[0]
    ).hex()
    revoked = bool(dual_equal(primary, shadow, lambda url: view(url, "revoked()", ["bool"])[0]))
    settlement_token = normalized(
        dual_equal(primary, shadow, lambda url: view(url, "settlementToken()", ["address"])[0])
    )
    competition_factory = normalized(
        dual_equal(primary, shadow, lambda url: view(url, "competitionFactory()", ["address"])[0])
    )
    factory_wallet = bool(
        dual_equal(
            primary,
            shadow,
            lambda url: contract_call(
                url,
                reserve_factory,
                "isFactoryWallet(address)",
                ["bool"],
                block,
                ["address"],
                [reserve],
            )[0],
        )
    )
    if (
        owner != normalized(bundle["owner"])
        or live_delegate != normalized(bundle["delegate"])
        or policy_version != 1
        or policy_hash != normalized(bundle["policy_hash"])
        or initial_policy_hash != policy_hash
        or revoked
        or settlement_token != USDC
        or competition_factory != factory
        or not factory_wallet
    ):
        raise GuardError("live reserve ownership or bounded policy differs from authorization")

    period_bucket = int(
        dual_equal(primary, shadow, lambda url: view(url, "periodBucket()", ["uint256"])[0])
    )
    period_spent = int(
        dual_equal(primary, shadow, lambda url: view(url, "periodSpent()", ["uint256"])[0])
    )
    lifetime_spent = int(
        dual_equal(primary, shadow, lambda url: view(url, "lifetimeSpent()", ["uint256"])[0])
    )
    balance = int(
        dual_equal(
            primary,
            shadow,
            lambda url: contract_call(
                url,
                USDC,
                "balanceOf(address)",
                ["uint256"],
                block,
                ["address"],
                [reserve],
            )[0],
        )
    )

    competition_hash = expected_proxy_hash(normalized(release["implementation_contract"]))
    creations = []
    active = 0
    for creation in bundle["creations"]:
        commitment = normalized(creation["creation_commitment"])
        competition = normalized(creation["predicted_competition"])
        approved = bool(
            dual_equal(
                primary,
                shadow,
                lambda url, c=commitment: view(
                    url,
                    "isApprovedCreation(bytes32)",
                    ["bool"],
                    ["bytes32"],
                    [bytes.fromhex(c[2:])],
                )[0],
            )
        )
        used = bool(
            dual_equal(
                primary,
                shadow,
                lambda url, c=commitment: view(
                    url,
                    "usedCreation(bytes32)",
                    ["bool"],
                    ["bytes32"],
                    [bytes.fromhex(c[2:])],
                )[0],
            )
        )
        deployed_code = dual_equal(primary, shadow, lambda url, c=competition: code_at(url, c, block))
        if not approved:
            raise GuardError("a reviewed creation is not approved by the live reserve")
        status = None
        if deployed_code != "0x":
            if code_hash(deployed_code) != competition_hash or not used:
                raise GuardError("a predicted competition has unexpected code or usage state")
            canonical = bool(
                dual_equal(
                    primary,
                    shadow,
                    lambda url, c=competition: contract_call(
                        url,
                        factory,
                        "isCanonicalCompetition(address)",
                        ["bool"],
                        block,
                        ["address"],
                        [c],
                    )[0],
                )
            )
            fields = dual_equal(
                primary,
                shadow,
                lambda url, c=competition: (
                    normalized(contract_call(url, c, "creator()", ["address"], block)[0]),
                    normalized(contract_call(url, c, "settlementToken()", ["address"], block)[0]),
                    int(contract_call(url, c, "targetAmount()", ["uint256"], block)[0]),
                    int(contract_call(url, c, "fundedAmount()", ["uint256"], block)[0]),
                    int(contract_call(url, c, "status()", ["uint8"], block)[0]),
                ),
            )
            creator, token, target_amount, funded_amount, status = fields
            if (
                not canonical
                or creator != reserve
                or token != USDC
                or target_amount != PER_COMPETITION
                or funded_amount != PER_COMPETITION
                or status not in (1, 2, 3)
            ):
                raise GuardError("predicted competition is not an exact canonical funded V2 competition")
            if status == 1:
                active += 1
        elif used:
            raise GuardError("reserve marks an absent competition creation as used")
        allowance = int(
            dual_equal(
                primary,
                shadow,
                lambda url, c=competition: contract_call(
                    url,
                    USDC,
                    "allowance(address,address)",
                    ["uint256"],
                    block,
                    ["address", "address"],
                    [reserve, c],
                )[0],
            )
        )
        if allowance != 0:
            raise GuardError("reserve has a nonzero competition allowance")
        creations.append(
            {
                "candidate_id": creation["candidate_id"],
                "commitment": commitment,
                "competition": competition,
                "approved": approved,
                "used": used,
                "status": status,
            }
        )

    current_bucket = safe["timestamp"] // int(bundle["policy"]["period_seconds"])
    effective_period_spent = period_spent if period_bucket == current_bucket else 0
    return {
        "schema": STATE_SCHEMA,
        "safe_block": safe,
        "reserve_deployed": True,
        "reserve_wallet": reserve,
        "reserve_balance_base_units": balance,
        "active": active,
        "floor": FLOOR,
        "target": TARGET,
        "severity": "critical" if active < FLOOR else ("warning" if active < TARGET else "none"),
        "period_spent_base_units": effective_period_spent,
        "lifetime_spent_base_units": lifetime_spent,
        "creations": creations,
    }


def choose_creations(state: dict[str, Any]) -> list[dict[str, Any]]:
    active = int(state["active"])
    if active >= TARGET:
        return []
    deficit = TARGET - active
    unused = [item for item in state["creations"] if item["approved"] and not item["used"]]
    period_capacity = (DAILY_CAP - int(state["period_spent_base_units"])) // PER_COMPETITION
    lifetime_capacity = (INITIAL_FUNDING - int(state["lifetime_spent_base_units"])) // PER_COMPETITION
    balance_capacity = int(state["reserve_balance_base_units"]) // PER_COMPETITION
    capacity = min(len(unused), period_capacity, lifetime_capacity, balance_capacity)
    if capacity < deficit:
        raise GuardError(
            f"cannot restore target: deficit {deficit}, exact bounded capacity {capacity}"
        )
    return unused[:deficit]


def load_ledger(state_dir: Path) -> dict[str, Any]:
    path = state_dir / LEDGER_FILE
    if not path.exists():
        return {"schema": LEDGER_SCHEMA, "transactions": []}
    value = read_json(path)
    if value.get("schema") != LEDGER_SCHEMA or not isinstance(value.get("transactions"), list):
        raise GuardError("private GMV guard ledger is invalid")
    return value


def save_ledger(state_dir: Path, ledger: dict[str, Any]) -> None:
    write_atomic(state_dir / LEDGER_FILE, json_bytes(ledger))


@contextlib.contextmanager
def exclusive_guard(state_dir: Path) -> Iterator[None]:
    path = state_dir / LOCK_FILE
    descriptor = os.open(path, os.O_RDWR | os.O_CREAT | getattr(os, "O_BINARY", 0), 0o600)
    try:
        if os.name == "nt":
            import msvcrt

            if os.path.getsize(path) == 0:
                os.write(descriptor, b"0")
                os.lseek(descriptor, 0, os.SEEK_SET)
            try:
                msvcrt.locking(descriptor, msvcrt.LK_NBLCK, 1)
            except OSError as error:
                raise GuardError("another GMV guard process is already running") from error
        else:
            import fcntl

            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError as error:
                raise GuardError("another GMV guard process is already running") from error
        yield
    finally:
        os.close(descriptor)


def sign_transaction(state_dir: Path, delegate: str, prepared: dict[str, Any]):
    # Import only at execution time: this helper owns the reviewed Windows-DPAPI
    # implementation, while pure policy tests remain platform independent.
    from local_delegate_wallet import unprotect_secret

    keystore = read_json(state_dir / KEYSTORE)
    password = bytearray(unprotect_secret(require_private_file(state_dir / DPAPI_BLOB)))
    private_key = bytearray()
    try:
        private_key.extend(Account.decrypt(keystore, bytes(password)))
        account = Account.from_key(bytes(private_key))
        if normalized(account.address) != delegate:
            raise GuardError("decrypted key does not match the configured delegate")
        return account.sign_transaction(prepared)
    finally:
        for secret in (password, private_key):
            for index in range(len(secret)):
                secret[index] = 0


def wait_canonical_receipt(primary: str, shadow: str, tx_hash: str, timeout: int) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    receipt_block = None
    while time.monotonic() < deadline:
        receipts = [rpc(url, "eth_getTransactionReceipt", [tx_hash], 508) for url in (primary, shadow)]
        available = [value for value in receipts if isinstance(value, dict)]
        if available:
            if any(value.get("status") != "0x1" for value in available):
                raise GuardError("guard transaction reverted")
            numbers = {int(str(value["blockNumber"]), 16) for value in available}
            hashes = {normalized(value["blockHash"]) for value in available}
            if len(numbers) != 1 or len(hashes) != 1:
                raise GuardError("RPCs disagree about the guard transaction receipt")
            receipt_block = numbers.pop()
            try:
                safe = safe_block(primary, shadow, receipt_block)
            except GuardError:
                time.sleep(2)
                continue
            if safe["number"] >= receipt_block:
                return {"transaction_hash": tx_hash, "receipt_block": receipt_block, "safe_block": safe}
        time.sleep(2)
    boundary = f" after receipt block {receipt_block}" if receipt_block is not None else ""
    raise GuardError(f"transaction canonical reconciliation timed out{boundary}; inspect by hash {tx_hash}")


def resume_pending(
    state_dir: Path,
    primary: str,
    shadow: str,
    receipt_primary: str,
    receipt_shadow: str,
) -> list[dict[str, Any]]:
    """Rebroadcast the exact persisted raw transaction before planning new work."""
    ledger = load_ledger(state_dir)
    resumed = []
    changed = False
    for record in ledger["transactions"]:
        if record.get("status") not in {"signed", "broadcast"}:
            continue
        raw_transaction = str(record.get("raw_transaction") or "")
        tx_hash = normalized(record.get("transaction_hash"))
        if not raw_transaction.startswith("0x") or not HASH.fullmatch(tx_hash):
            raise GuardError("pending crash-ledger transaction is malformed")
        delegate = public_address(state_dir)
        if normalized(Account.recover_transaction(raw_transaction)) != delegate:
            raise GuardError("pending crash-ledger transaction is not signed by the protected delegate")
        for url in (primary, shadow):
            try:
                returned = normalized(rpc(url, "eth_sendRawTransaction", [raw_transaction], 513))
                if returned != tx_hash:
                    raise GuardError("recovery RPC returned a different transaction hash")
            except (RuntimeError, GuardError):
                # Exact raw rebroadcast is idempotent. Canonical receipt evidence below
                # determines whether an already-known/nonce-used response is acceptable.
                pass
        evidence = wait_canonical_receipt(
            receipt_primary, receipt_shadow, tx_hash, RECEIPT_TIMEOUT
        )
        record.update({"status": "canonically_confirmed", **evidence})
        record.pop("raw_transaction", None)
        resumed.append(evidence)
        changed = True
    if changed:
        save_ledger(state_dir, ledger)
    return resumed


def execute_direct(
    state_dir: Path,
    primary: str,
    shadow: str,
    receipt_primary: str,
    receipt_shadow: str,
    direct: dict[str, Any],
    kind: str,
    candidate_id: str | None,
    *,
    broadcast: bool,
) -> dict[str, Any]:
    delegate = public_address(state_dir)
    intent = {
        "from": delegate,
        "to": normalized(direct["to"]),
        "data": direct["data"],
        "value": "0x0",
    }
    safe = safe_block(primary, shadow)
    for url in (primary, shadow):
        result = rpc(url, "eth_call", [intent, hex(safe["number"])], 509)
        if not isinstance(result, str) or not result.startswith("0x"):
            raise GuardError("dual-RPC transaction simulation failed")
    prepared = transaction_parameters(primary, intent)
    shadow_nonce = int(str(rpc(shadow, "eth_getTransactionCount", [delegate, "pending"], 510)), 16)
    if shadow_nonce != prepared["nonce"]:
        raise GuardError("primary and shadow pending nonces disagree")
    result = {
        "kind": kind,
        "candidate_id": candidate_id,
        "safe_block": safe,
        "gas_limit": prepared["gas"],
        "max_fee_per_gas_wei": prepared["maxFeePerGas"],
        "broadcast": broadcast,
    }
    if not broadcast:
        return result

    signed = sign_transaction(state_dir, delegate, prepared)
    tx_hash = rpc_hex(signed.hash).lower()
    raw_transaction = rpc_hex(signed.raw_transaction)
    ledger = load_ledger(state_dir)
    if any(item.get("transaction_hash") == tx_hash for item in ledger["transactions"]):
        raise GuardError("transaction is already recorded in the private crash ledger")
    record = {
        "kind": kind,
        "candidate_id": candidate_id,
        "transaction_hash": tx_hash,
        "nonce": prepared["nonce"],
        "raw_transaction": raw_transaction,
        "status": "signed",
        "recorded_at": int(time.time()),
    }
    ledger["transactions"].append(record)
    save_ledger(state_dir, ledger)

    accepted = False
    errors = []
    for url in (primary, shadow):
        try:
            returned = normalized(rpc(url, "eth_sendRawTransaction", [raw_transaction], 511))
            if returned != tx_hash:
                raise GuardError("RPC returned a transaction hash different from the signed hash")
            accepted = True
        except (RuntimeError, GuardError) as error:
            errors.append(str(error))
    if not accepted:
        # Both endpoints may report an already-known/nonce-used error after accepting
        # the exact raw transaction. Receipt reconciliation decides, never the text.
        if all(
            rpc(url, "eth_getTransactionReceipt", [tx_hash], 512) is None
            for url in (receipt_primary, receipt_shadow)
        ):
            raise GuardError(f"both RPC broadcasts failed before a receipt was visible: {errors}")
    record["status"] = "broadcast"
    save_ledger(state_dir, ledger)
    evidence = wait_canonical_receipt(
        receipt_primary, receipt_shadow, tx_hash, RECEIPT_TIMEOUT
    )
    record.update({"status": "canonically_confirmed", **evidence})
    record.pop("raw_transaction", None)
    save_ledger(state_dir, ledger)
    return {**result, **evidence}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state-dir", type=Path, required=True)
    parser.add_argument("--release", type=Path, required=True)
    parser.add_argument("--reserve-deployment", type=Path, required=True)
    parser.add_argument("--candidate-pool", type=Path, required=True)
    parser.add_argument("--activation-bundle", type=Path, required=True)
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--shadow-rpc-url", required=True)
    parser.add_argument("--receipt-rpc-url")
    parser.add_argument("--shadow-receipt-rpc-url")
    parser.add_argument("--json-out", type=Path)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("status")
    relay_parser = commands.add_parser("relay-funding")
    relay_parser.add_argument("--signature-file", type=Path, required=True)
    relay_parser.add_argument("--broadcast", action="store_true")
    replenish_parser = commands.add_parser("replenish")
    replenish_parser.add_argument("--broadcast", action="store_true")
    args = parser.parse_args(argv)

    try:
        state_dir = args.state_dir.resolve()
        delegate = public_address(state_dir)
        release = load_object(args.release)
        reserve_deployment = load_object(args.reserve_deployment)
        pool = load_object(args.candidate_pool)
        bundle = load_object(args.activation_bundle)
        receipt_rpc_url = args.receipt_rpc_url or args.rpc_url
        shadow_receipt_rpc_url = args.shadow_receipt_rpc_url or args.shadow_rpc_url
        validate_reviewed_inputs(release, reserve_deployment, pool, bundle, delegate)
        with exclusive_guard(state_dir):
            resumed = resume_pending(
                state_dir,
                args.rpc_url,
                args.shadow_rpc_url,
                receipt_rpc_url,
                shadow_receipt_rpc_url,
            )
            state = inspect_state(
                args.rpc_url,
                args.shadow_rpc_url,
                release,
                reserve_deployment,
                bundle,
            )
            result: dict[str, Any] = {"status": "inspected", "state": state, "resumed": resumed}
            if args.command == "relay-funding":
                if state["reserve_deployed"]:
                    raise GuardError("reserve already exists; owner authorization cannot be replayed")
                signature_value = load_object(args.signature_file)
                relay = build_relay(
                    bundle,
                    str(signature_value.get("signature") or ""),
                    now=state["safe_block"]["timestamp"],
                )
                direct = {"to": relay["to"], "data": relay["data"]}
                execution = execute_direct(
                    state_dir,
                    args.rpc_url,
                    args.shadow_rpc_url,
                    receipt_rpc_url,
                    shadow_receipt_rpc_url,
                    direct,
                    "reserve_funding",
                    None,
                    broadcast=args.broadcast,
                )
                result = {"status": "submitted" if args.broadcast else "ready", "execution": execution}
                if args.broadcast:
                    result["state"] = inspect_state(
                        args.rpc_url,
                        args.shadow_rpc_url,
                        release,
                        reserve_deployment,
                        bundle,
                        minimum_block=execution["receipt_block"],
                    )
                    if (
                        not result["state"]["reserve_deployed"]
                        or result["state"]["reserve_balance_base_units"] != INITIAL_FUNDING
                    ):
                        raise GuardError("funded reserve did not reconcile to the exact authorization")
                    result["status"] = "canonically_funded"
            elif args.command == "replenish":
                if not state["reserve_deployed"]:
                    raise GuardError("reserve is not funded and cannot replenish")
                selected = choose_creations(state)
                result = {
                    "status": "noop" if not selected else "ready",
                    "selected_candidate_ids": [item["candidate_id"] for item in selected],
                    "state": state,
                    "executions": [],
                }
                if args.broadcast:
                    for selected_item in selected:
                        latest = inspect_state(
                            args.rpc_url,
                            args.shadow_rpc_url,
                            release,
                            reserve_deployment,
                            bundle,
                        )
                        still_selected = choose_creations(latest)
                        if not still_selected or still_selected[0]["candidate_id"] != selected_item["candidate_id"]:
                            raise GuardError("canonical state changed after planning; refusing stale creation")
                        creation = next(
                            item for item in bundle["creations"]
                            if item["candidate_id"] == selected_item["candidate_id"]
                        )
                        execution = execute_direct(
                            state_dir,
                            args.rpc_url,
                            args.shadow_rpc_url,
                            receipt_rpc_url,
                            shadow_receipt_rpc_url,
                            creation["delegate_transaction"],
                            "create_competition",
                            creation["candidate_id"],
                            broadcast=True,
                        )
                        result["executions"].append(execution)
                    result["state"] = inspect_state(
                        args.rpc_url,
                        args.shadow_rpc_url,
                        release,
                        reserve_deployment,
                        bundle,
                    )
                    if result["state"]["active"] != TARGET:
                        raise GuardError("replenishment ended without the exact active target")
                    result["status"] = "canonically_replenished"
        if args.json_out:
            args.json_out.parent.mkdir(parents=True, exist_ok=True)
            args.json_out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        print(
            json.dumps(
                {
                    "status": result["status"],
                    "active": result.get("state", {}).get("active"),
                    "safe_block": result.get("state", {}).get("safe_block"),
                    "selected": result.get("selected_candidate_ids", []),
                },
                sort_keys=True,
            )
        )
        return 0
    except (OSError, json.JSONDecodeError, KeyError, GuardError, ValueError) as error:
        print(json.dumps({"status": "blocked", "error": str(error)}))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
