#!/usr/bin/env python3
"""Expose bounded-wallet liquidity and delegate authority separately on Base mainnet.

Proves and separates:
1. Native ERC20 USDC balance (actual liquid funds in wallet).
2. Lifetime authority cap, spent counter, and remaining lifetime authority.
3. Period authority cap, spent counter, and remaining period authority.
4. Effective spendable liquidity: min(usdc_balance, remaining_lifetime_authority, remaining_period_authority).
5. Distinct claimable escrow (held in external bounty contracts, not wallet balance).
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import urllib.error
import urllib.request
from decimal import Decimal
from pathlib import Path
from typing import Any, Callable

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "deployments" / "bounded-agent-wallet-v2-base-mainnet.json"
LEGACY_MANIFEST = ROOT / "deployments" / "bounded-agent-wallet-base-mainnet.json"
ZERO_ADDRESS = "0x" + "00" * 20
ZERO_HASH = "0x" + "00" * 32
BASE_CHAIN_ID = 8453
USDC_DECIMALS = 6

POLICY_FIELDS = (
    "delegate",
    "valid_after",
    "valid_until",
    "period_seconds",
    "max_per_action",
    "max_per_period",
    "max_lifetime_spend",
    "max_bounty_target",
    "allowed_actions",
    "allowed_verification_modes",
    "deterministic_verifier_module",
    "signed_quorum_verifier_set_hash",
    "ai_judge_verifier_set_hash",
)

# Known selectors
SEL_IMPLEMENTATION = "0x5c60da1b"
SEL_BOUNTY_FACTORY = "0x30cdfa52"
SEL_SETTLEMENT_TOKEN = "0x7b9e618d"
SEL_IS_FACTORY_WALLET = "0x240fa116"
SEL_POLICY = "0x0505c8c9"
SEL_OWNER = "0x8da5cb5b"
SEL_DELEGATE_NONCE = "0x9bbd99c2"
SEL_POLICY_VERSION = "0x58355ead"
SEL_PERIOD_BUCKET = "0xcb2e5d9c"
SEL_PERIOD_SPENT = "0xb80762dd"
SEL_LIFETIME_SPENT = "0x93e67715"
SEL_REVOKED = "0x63d256ce"
SEL_BALANCE_OF = "0x70a08231"


def keccak_256(data: bytes) -> bytes:
    """Pure Python Keccak-256 implementation matching Ethereum specifications."""
    rc = [
        0x0000000000000001,
        0x0000000000008082,
        0x800000000000808A,
        0x8000000080008000,
        0x000000000000808B,
        0x0000000080000001,
        0x8000000080008081,
        0x8000000000008009,
        0x000000000000008A,
        0x0000000000000088,
        0x0000000080008009,
        0x000000008000000A,
        0x000000008000808B,
        0x800000000000008B,
        0x8000000000008089,
        0x8000000000008003,
        0x8000000000008002,
        0x8000000000000080,
        0x000000000000800A,
        0x800000008000000A,
        0x8000000080008081,
        0x8000000000008080,
        0x0000000080000001,
        0x8000000080008008,
    ]
    rotc = [
        [0, 36, 3, 41, 18],
        [1, 44, 10, 45, 2],
        [62, 6, 43, 15, 61],
        [28, 55, 25, 21, 56],
        [27, 20, 39, 8, 14],
    ]
    state = [[0] * 5 for _ in range(5)]
    rate = 1088 // 8
    pad = bytearray(data)
    pad.append(0x01)
    while len(pad) % rate != rate - 1:
        pad.append(0x00)
    pad.append(0x80)

    for i in range(0, len(pad), rate):
        block = pad[i : i + rate]
        for j in range(len(block) // 8):
            x = j % 5
            y = j // 5
            val = int.from_bytes(block[j * 8 : (j + 1) * 8], "little")
            state[x][y] ^= val
        for round_idx in range(24):
            c = [
                state[x][0] ^ state[x][1] ^ state[x][2] ^ state[x][3] ^ state[x][4]
                for x in range(5)
            ]
            d = [
                c[(x - 1) % 5]
                ^ (((c[(x + 1) % 5] << 1) & 0xFFFFFFFFFFFFFFFF) | (c[(x + 1) % 5] >> 63))
                for x in range(5)
            ]
            for x in range(5):
                for y in range(5):
                    state[x][y] ^= d[x]
            b = [[0] * 5 for _ in range(5)]
            for x in range(5):
                for y in range(5):
                    shift = rotc[x][y]
                    val = state[x][y]
                    b[y][(2 * x + 3 * y) % 5] = (
                        ((val << shift) & 0xFFFFFFFFFFFFFFFF) | (val >> (64 - shift))
                        if shift
                        else val
                    )
            for x in range(5):
                for y in range(5):
                    state[x][y] = b[x][y] ^ ((~b[(x + 1) % 5][y]) & b[(x + 2) % 5][y])
            state[0][0] ^= rc[round_idx]

    out = bytearray()
    for j in range(4):
        x = j % 5
        y = j // 5
        out.extend(state[x][y].to_bytes(8, "little"))
    return bytes(out)


def keccak_hex(data: bytes | str) -> str:
    """Return 0x-prefixed 32-byte keccak-256 hash."""
    if isinstance(data, str):
        if data.startswith("0x"):
            raw = bytes.fromhex(data[2:])
        else:
            raw = data.encode("utf-8")
    else:
        raw = data
    return f"0x{keccak_256(raw).hex()}"


def code_hash(code: str) -> str | None:
    """Return code hash or None if empty bytecode."""
    normalized = code.lower().strip()
    if normalized in {"0x", "0x0", ""}:
        return None
    return keccak_hex(normalized)


def require_address(value: str, label: str) -> str:
    """Validate 20-byte EVM address format."""
    normalized = str(value).strip().lower()
    if len(normalized) != 42 or not normalized.startswith("0x"):
        raise SystemExit(f"{label} must be a 20-byte EVM address")
    try:
        bytes.fromhex(normalized[2:])
    except ValueError as error:
        raise SystemExit(f"{label} must be a 20-byte EVM address") from error
    if normalized == ZERO_ADDRESS:
        raise SystemExit(f"{label} cannot be zero")
    return normalized


def require_bytes32(value: str, label: str) -> str:
    """Validate 32-byte hex string."""
    normalized = str(value).strip().lower()
    if len(normalized) != 66 or not normalized.startswith("0x"):
        raise SystemExit(f"{label} must be 32 bytes")
    try:
        bytes.fromhex(normalized[2:])
    except ValueError as error:
        raise SystemExit(f"{label} must be 32 bytes") from error
    return normalized


def pad_address(address: str) -> str:
    """Pad address to 32-byte ABI argument."""
    clean = address.lower().removeprefix("0x")
    return clean.zfill(64)


def pad_uint(val: int) -> str:
    """Pad integer to 32-byte ABI argument."""
    return hex(val).removeprefix("0x").zfill(64)


def words(value: str) -> list[str]:
    """Break ABI hex output into 32-byte (64-hex character) words."""
    raw = value.lower().removeprefix("0x")
    if len(raw) == 0 or len(raw) % 64 != 0:
        raise RuntimeError(f"ABI result is not word aligned (length: {len(raw)})")
    return [raw[index : index + 64] for index in range(0, len(raw), 64)]


def word_address(word: str) -> str:
    """Extract 20-byte address from 32-byte ABI word."""
    if len(word) != 64 or any(c not in "0123456789abcdef" for c in word):
        raise RuntimeError("invalid ABI address word")
    return f"0x{word[-40:]}"


def word_uint(word: str) -> int:
    """Convert 32-byte hex word to int."""
    return int(word, 16)


def format_usdc(atomic_units: int) -> str:
    """Format 6-decimal atomic USDC units into a human-readable decimal string."""
    decimal_val = Decimal(atomic_units) / (Decimal(10) ** USDC_DECIMALS)
    return f"{decimal_val:.6f}"


def default_rpc(url: str, method: str, params: list, request_id: int = 1) -> Any:
    """Standard JSON-RPC call over HTTP with timeout and error classification."""
    request = urllib.request.Request(
        url,
        data=json.dumps(
            {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
        ).encode("utf-8"),
        headers={"content-type": "application/json", "user-agent": "agent-bounties/1.0"},
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            payload = json.loads(response.read().decode("utf-8"))
    except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, json.JSONDecodeError) as error:
        raise RuntimeError(f"Base RPC {method} failed: {error}") from error
    if not isinstance(payload, dict):
        raise RuntimeError(f"Base RPC {method} returned invalid payload")
    if payload.get("error"):
        raise RuntimeError(
            f"Base RPC {method} returned error: {payload['error'].get('message', 'an error')}"
        )
    if "result" not in payload:
        raise RuntimeError(f"Base RPC {method} omitted result")
    return payload["result"]


def inspect_bounded_wallet_liquidity(
    wallet: str,
    manifest: dict | Path | str | None = None,
    rpc_url: str | None = None,
    expected_owner: str | None = None,
    expected_delegate: str | None = None,
    expected_policy_hash: str | None = None,
    expected_policy_version: int | None = None,
    rpc_caller: Callable[[str, str, list, int], Any] | None = None,
) -> dict[str, Any]:
    """Inspect bounded-wallet liquidity, authority limits, and counters in a single safe block read.

    Validates:
    - Base mainnet chain ID (8453).
    - Factory, implementation, and clone bytecode hashes.
    - Token binding (settlement token is USDC 0x833589fcd6edb6e08f4c7c32d4f71b54bda02913).
    - Policy validity, timing, revocation, and active authority caps.
    """
    if manifest is None:
        if DEFAULT_MANIFEST.exists():
            manifest_dict = json.loads(DEFAULT_MANIFEST.read_text(encoding="utf-8"))
        elif LEGACY_MANIFEST.exists():
            manifest_dict = json.loads(LEGACY_MANIFEST.read_text(encoding="utf-8"))
        else:
            raise RuntimeError("Default deployment manifest not found")
    elif isinstance(manifest, (Path, str)):
        manifest_dict = json.loads(Path(manifest).read_text(encoding="utf-8"))
    else:
        manifest_dict = manifest

    target_wallet = require_address(wallet, "wallet")
    call_rpc = rpc_caller or default_rpc
    endpoint = rpc_url or manifest_dict.get("rpc_url") or "https://mainnet.base.org"

    # Step 1: Query safe block
    try:
        safe = call_rpc(endpoint, "eth_getBlockByNumber", ["safe", False], 1)
    except Exception as error:
        raise RuntimeError(f"RPC unavailable: safe block query failed: {error}") from error

    if not isinstance(safe, dict) or not safe.get("number") or not safe.get("hash"):
        raise RuntimeError("Base safe block is unavailable")

    block_hex = str(safe["number"])
    block_num = int(block_hex, 16) if block_hex.startswith("0x") else int(block_hex)
    block_hash = str(safe["hash"]).lower()
    timestamp = (
        int(str(safe["timestamp"]), 16)
        if str(safe.get("timestamp", "")).startswith("0x")
        else int(safe.get("timestamp", 0))
    )

    observed_block = {
        "number": block_num,
        "hash": block_hash,
        "timestamp": timestamp,
        "tag": "safe",
    }

    # Step 2: Validate chain ID
    try:
        chain_raw = call_rpc(endpoint, "eth_chainId", [], 2)
        chain_id = int(str(chain_raw), 16) if str(chain_raw).startswith("0x") else int(chain_raw)
    except Exception as error:
        raise RuntimeError(f"RPC unavailable: eth_chainId failed: {error}") from error

    expected_chain = int(manifest_dict.get("chain_id", BASE_CHAIN_ID))
    failures: list[str] = []

    if chain_id != expected_chain or chain_id != BASE_CHAIN_ID:
        failures.append(f"wrong chain (observed: {chain_id}, expected: {expected_chain})")

    # Step 3: Validate bytecode hashes
    factory = require_address(manifest_dict["wallet_factory"]["address"], "wallet factory")
    implementation = require_address(
        manifest_dict["wallet_factory"]["implementation"], "implementation"
    )
    bounty_factory = require_address(
        manifest_dict["canonical"]["bounty_factory"], "bounty factory"
    )
    settlement_token = require_address(
        manifest_dict["canonical"]["settlement_token"], "settlement token"
    )

    addresses = [factory, implementation, target_wallet]
    observed_code: dict[str, str] = {}
    for index, addr in enumerate(addresses):
        try:
            code_res = call_rpc(endpoint, "eth_getCode", [addr, block_hex], 10 + index)
            observed_code[addr] = str(code_res).lower()
        except Exception as error:
            raise RuntimeError(f"RPC unavailable: eth_getCode for {addr} failed: {error}") from error

    hashes = {addr: code_hash(code) for addr, code in observed_code.items()}

    if hashes.get(factory) != manifest_dict["wallet_factory"]["runtime_code_hash"].lower():
        failures.append("wallet_factory_code_mismatch")
    if (
        hashes.get(implementation)
        != manifest_dict["wallet_factory"]["implementation_runtime_code_hash"].lower()
    ):
        failures.append("wallet_implementation_code_mismatch")
    if hashes.get(target_wallet) != manifest_dict["wallet_factory"]["clone_runtime_code_hash"].lower():
        failures.append("wallet_clone_code_mismatch")

    state: dict[str, Any] = {}
    policy_dict: dict[str, Any] = {}
    usdc_balance = 0
    lifetime_spent = 0
    max_lifetime = 0
    period_spent = 0
    max_per_period = 0
    max_per_action = 0
    remaining_lifetime_authority = 0
    remaining_period_authority = 0
    spendable_liquidity = 0
    policy_hash = ZERO_HASH
    policy_version = 0
    owner = ZERO_ADDRESS
    delegate = ZERO_ADDRESS

    def eth_call(to: str, data: str, req_id: int) -> str:
        res = call_rpc(endpoint, "eth_call", [{"to": to, "data": data}, block_hex], req_id)
        if not isinstance(res, str) or not res.startswith("0x"):
            raise RuntimeError(f"invalid eth_call result for {to}: {res}")
        return res.lower()

    if not failures:
        # Check factory bindings
        factory_impl = word_address(words(eth_call(factory, SEL_IMPLEMENTATION, 20))[0])
        factory_bounty = word_address(words(eth_call(factory, SEL_BOUNTY_FACTORY, 21))[0])
        factory_token = word_address(words(eth_call(factory, SEL_SETTLEMENT_TOKEN, 22))[0])
        is_registered = bool(
            word_uint(
                words(
                    eth_call(
                        factory, f"{SEL_IS_FACTORY_WALLET}{pad_address(target_wallet)}", 23
                    )
                )[0]
            )
        )

        if factory_impl != implementation:
            failures.append("factory_implementation_mismatch")
        if factory_bounty != bounty_factory:
            failures.append("factory_bounty_binding_mismatch")
        if factory_token != settlement_token:
            failures.append("factory_token_binding_mismatch")
        if not is_registered:
            failures.append("wallet_not_registered")

        # Query wallet policy tuple
        policy_raw = eth_call(wallet, SEL_POLICY, 30)
        policy_words = words(policy_raw)
        if len(policy_words) != len(POLICY_FIELDS):
            raise RuntimeError(
                f"Unexpected policy word count: {len(policy_words)} vs {len(POLICY_FIELDS)}"
            )

        policy_dict = dict(zip(POLICY_FIELDS, policy_words, strict=True))
        for name in ("delegate", "deterministic_verifier_module"):
            policy_dict[name] = word_address(str(policy_dict[name]))
        for name in POLICY_FIELDS[1:10]:
            policy_dict[name] = word_uint(str(policy_dict[name]))
        policy_dict["signed_quorum_verifier_set_hash"] = f"0x{policy_dict['signed_quorum_verifier_set_hash']}"
        policy_dict["ai_judge_verifier_set_hash"] = f"0x{policy_dict['ai_judge_verifier_set_hash']}"
        policy_dict["max_lifetime"] = policy_dict["max_lifetime_spend"]

        policy_hash = keccak_hex(policy_raw)
        policy_dict["policy_hash"] = policy_hash

        # Query wallet state
        owner = word_address(words(eth_call(wallet, SEL_OWNER, 31))[0])
        delegate_nonce = word_uint(words(eth_call(wallet, SEL_DELEGATE_NONCE, 32))[0])
        policy_version = word_uint(words(eth_call(wallet, SEL_POLICY_VERSION, 33))[0])
        period_bucket = word_uint(words(eth_call(wallet, SEL_PERIOD_BUCKET, 34))[0])
        period_spent = word_uint(words(eth_call(wallet, SEL_PERIOD_SPENT, 35))[0])
        lifetime_spent = word_uint(words(eth_call(wallet, SEL_LIFETIME_SPENT, 36))[0])
        revoked = bool(word_uint(words(eth_call(wallet, SEL_REVOKED, 37))[0]))

        # Query settlement token balance
        balance_raw = eth_call(
            settlement_token, f"{SEL_BALANCE_OF}{pad_address(target_wallet)}", 38
        )
        usdc_balance = word_uint(words(balance_raw)[0])

        delegate = str(policy_dict["delegate"])
        max_lifetime = int(policy_dict["max_lifetime_spend"])
        max_per_period = int(policy_dict["max_per_period"])
        max_per_action = int(policy_dict["max_per_action"])

        # Policy checks
        if owner == ZERO_ADDRESS:
            failures.append("owner_zero")
        if delegate == ZERO_ADDRESS:
            failures.append("delegate_zero")
        if expected_owner is not None and owner != expected_owner.lower():
            failures.append(f"owner_mismatch (expected {expected_owner}, got {owner})")
        if expected_delegate is not None and delegate != expected_delegate.lower():
            failures.append(f"delegate_mismatch (expected {expected_delegate}, got {delegate})")
        if expected_policy_hash is not None and policy_hash != expected_policy_hash.lower():
            failures.append(
                f"policy_hash_mismatch (expected {expected_policy_hash}, got {policy_hash})"
            )
        if (
            expected_policy_version is not None
            and policy_version != expected_policy_version
        ):
            failures.append(
                f"policy_version_mismatch (expected {expected_policy_version}, got {policy_version})"
            )
        if revoked:
            failures.append("policy_revoked")
        if timestamp < policy_dict["valid_after"]:
            failures.append("policy_not_active")
        if timestamp > policy_dict["valid_until"]:
            failures.append("policy_expired")

        # Authority caps calculation
        remaining_lifetime_authority = max(0, max_lifetime - lifetime_spent)
        remaining_period_authority = max(0, max_per_period - period_spent)

        # Spendable liquidity is the strictest bottleneck among balance and remaining policy authorities
        spendable_liquidity = min(
            usdc_balance, remaining_lifetime_authority, remaining_period_authority
        )

        state = {
            "owner": owner,
            "delegate": delegate,
            "policy_version": policy_version,
            "delegate_nonce": delegate_nonce,
            "period_bucket": str(period_bucket),
            "period_spent": str(period_spent),
            "lifetime_spent": str(lifetime_spent),
            "revoked": revoked,
            "is_registered": is_registered,
        }

    currently_claimable_escrow = 0

    liquidity_summary = {
        "usdc_balance": usdc_balance,
        "usdc_balance_formatted": format_usdc(usdc_balance),
        "lifetime_spent": lifetime_spent,
        "lifetime_spent_formatted": format_usdc(lifetime_spent),
        "max_lifetime": max_lifetime,
        "max_lifetime_formatted": format_usdc(max_lifetime),
        "remaining_lifetime_authority": remaining_lifetime_authority,
        "remaining_lifetime_authority_formatted": format_usdc(remaining_lifetime_authority),
        "period_spent": period_spent,
        "period_spent_formatted": format_usdc(period_spent),
        "max_per_period": max_per_period,
        "max_per_period_formatted": format_usdc(max_per_period),
        "remaining_period_authority": remaining_period_authority,
        "remaining_period_authority_formatted": format_usdc(remaining_period_authority),
        "max_per_action": max_per_action,
        "max_per_action_formatted": format_usdc(max_per_action),
        "spendable_liquidity": spendable_liquidity,
        "spendable_liquidity_formatted": format_usdc(spendable_liquidity),
        "currently_claimable_escrow": currently_claimable_escrow,
        "currently_claimable_escrow_formatted": format_usdc(currently_claimable_escrow),
    }

    report = {
        "schema": "agent-bounties/bounded-wallet-liquidity-v1",
        "ready": len(failures) == 0,
        "failures": failures,
        "network": manifest_dict.get("network", "base-mainnet"),
        "chain_id": chain_id,
        "wallet": target_wallet,
        "owner": owner,
        "delegate": delegate,
        "settlement_token": settlement_token,
        "observed_block": observed_block,
        "usdc_balance": usdc_balance,
        "lifetime_spent": lifetime_spent,
        "max_lifetime": max_lifetime,
        "period_spent": period_spent,
        "max_per_period": max_per_period,
        "remaining_lifetime_authority": remaining_lifetime_authority,
        "remaining_period_authority": remaining_period_authority,
        "max_per_action": max_per_action,
        "spendable_liquidity": spendable_liquidity,
        "currently_claimable_escrow": currently_claimable_escrow,
        "policy_hash": policy_hash,
        "policy_version": policy_version,
        "policy": policy_dict,
        "state": state,
        "liquidity": liquidity_summary,
        "runtime_code_hashes": hashes,
        "semantic_distinction": (
            "Native ERC20 USDC balance (usdc_balance) represents liquid tokens in the wallet contract. "
            "Policy caps (max_lifetime, max_per_period, max_per_action) constrain delegate spending limits. "
            "Spendable liquidity is the effective available amount: min(usdc_balance, remaining_lifetime_authority, remaining_period_authority). "
            "Escrowed funds (currently_claimable_escrow) reside in external bounty contracts and are distinct from wallet balance."
        ),
    }

    return report


def render_markdown_summary(report: dict[str, Any]) -> str:
    """Generate a clean Markdown status table from the liquidity inspection report."""
    liq = report["liquidity"]
    obs = report["observed_block"]
    status_icon = "PASS" if report["ready"] else "FAIL"

    lines = [
        f"# Bounded Wallet Liquidity & Authority Report [{status_icon}]",
        "",
        f"- **Wallet:** `{report['wallet']}`",
        f"- **Owner:** `{report.get('owner', 'N/A')}`",
        f"- **Delegate:** `{report.get('delegate', 'N/A')}`",
        f"- **Network / Chain ID:** `{report['network']}` (`{report['chain_id']}`)",
        f"- **Observed Block:** `{obs['number']}` (`{obs['hash']}` @ `{obs['timestamp']}`)",
        f"- **Policy Hash:** `{report['policy_hash']}`",
        f"- **Policy Version:** `{report['policy_version']}`",
        "",
        "## Liquidity & Authority Breakdown",
        "",
        "| Metric | Atomic Units (6 Dec) | Formatted USDC | Description |",
        "| :--- | :--- | :--- | :--- |",
        f"| **Native USDC Balance** (`usdc_balance`) | `{liq['usdc_balance']}` | `${liq['usdc_balance_formatted']}` | Liquid ERC20 balance in wallet |",
        f"| **Spendable Liquidity** (`spendable_liquidity`) | `{liq['spendable_liquidity']}` | `${liq['spendable_liquidity_formatted']}` | Effective spendable amount |",
        f"| **Remaining Lifetime Authority** | `{liq['remaining_lifetime_authority']}` | `${liq['remaining_lifetime_authority_formatted']}` | Max lifetime spend remaining |",
        f"| **Lifetime Spent** (`lifetime_spent`) | `{liq['lifetime_spent']}` | `${liq['lifetime_spent_formatted']}` | Cumulative lifetime spent |",
        f"| **Max Lifetime Spend** (`max_lifetime`) | `{liq['max_lifetime']}` | `${liq['max_lifetime_formatted']}` | Lifetime budget envelope |",
        f"| **Remaining Period Authority** | `{liq['remaining_period_authority']}` | `${liq['remaining_period_authority_formatted']}` | Current period spend remaining |",
        f"| **Period Spent** (`period_spent`) | `{liq['period_spent']}` | `${liq['period_spent_formatted']}` | Current 24h period spent |",
        f"| **Max Per Period** (`max_per_period`) | `{liq['max_per_period']}` | `${liq['max_per_period_formatted']}` | 24h period budget envelope |",
        f"| **Max Per Action** (`max_per_action`) | `{liq['max_per_action']}` | `${liq['max_per_action_formatted']}` | Single action spend ceiling |",
        f"| **Claimable Bounty Escrow** | `{liq['currently_claimable_escrow']}` | `${liq['currently_claimable_escrow_formatted']}` | External bounty escrow (distinct) |",
        "",
    ]

    if report["failures"]:
        lines.append("## Failures & Warnings")
        lines.append("")
        for fail in report["failures"]:
            lines.append(f"- ❌ `{fail}`")
        lines.append("")

    lines.append("## Semantic Separation Note")
    lines.append(f"> {report['semantic_distinction']}")
    lines.append("")

    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Inspect bounded agent wallet liquidity and authority separately."
    )
    parser.add_argument("--wallet", required=True, help="Bounded agent wallet address")
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST, help="Deployment manifest")
    parser.add_argument("--rpc-url", help="Base JSON-RPC endpoint")
    parser.add_argument("--expect-owner", help="Expected owner address")
    parser.add_argument("--expect-delegate", help="Expected delegate address")
    parser.add_argument("--expect-policy-hash", help="Expected 32-byte policy hash")
    parser.add_argument("--expect-policy-version", type=int, help="Expected policy version")
    parser.add_argument("--output", type=Path, help="Write JSON report to file")
    parser.add_argument("--md-out", type=Path, help="Write Markdown report to file")
    parser.add_argument("--json", action="store_true", help="Print JSON report to stdout")
    args = parser.parse_args()

    report = inspect_bounded_wallet_liquidity(
        wallet=args.wallet,
        manifest=args.manifest,
        rpc_url=args.rpc_url,
        expected_owner=args.expect_owner,
        expected_delegate=args.expect_delegate,
        expected_policy_hash=args.expect_policy_hash,
        expected_policy_version=args.expect_policy_version,
    )

    json_str = json.dumps(report, indent=2) + "\n"

    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json_str, encoding="utf-8")
        print(f"JSON report written to: {args.output}")

    if args.md_out:
        args.md_out.parent.mkdir(parents=True, exist_ok=True)
        md_text = render_markdown_summary(report)
        args.md_out.write_text(md_text, encoding="utf-8")
        print(f"Markdown report written to: {args.md_out}")

    if args.json:
        print(json_str, end="")
    elif not args.output and not args.md_out:
        print(render_markdown_summary(report))

    sys.exit(0 if report["ready"] else 1)


if __name__ == "__main__":
    main()
