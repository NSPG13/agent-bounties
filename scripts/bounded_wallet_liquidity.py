#!/usr/bin/env python3
"""Bounded-wallet liquidity and policy authority reporter (Issue #871).

Reports actual spendable native Base USDC held in a BoundedAgentWallet
separately from remaining delegate policy authority and separately from
claimable AgentBounty escrow inventory.

All on-chain reads are pinned to one single `observed_block`. Any chain,
bytecode, settlement-token, owner, revocation, policy-hash, policy-version, or
RPC failure fails closed and withholds financial figures (`None` rather than
substituting `0`).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any, Callable, Mapping, Sequence


REPORT_SCHEMA = "agent-bounties/bounded-wallet-liquidity-report-v1"
BASE_MAINNET_CHAIN_ID = 8453
NATIVE_BASE_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"

# Exact Solidity 4-byte selectors matching contracts/base-escrow/src/BoundedAgentWallet.sol
BOUNDED_AGENT_WALLET_ABI_SELECTORS: Mapping[str, str] = {
    "owner()": "0x8da5cb5b",
    "settlementToken()": "0x7b9e618d",
    "factory()": "0xc45a0155",
    "policy()": "0x050037b4",
    "policyVersion()": "0x896f2d6b",
    "policyHash()": "0x39b327f0",
    "periodBucket()": "0x6739f400",
    "periodSpent()": "0x9190b7d3",
    "lifetimeSpent()": "0x9d16c2c3",
    "revoked()": "0x75c1bb14",
}
ERC20_BALANCE_OF_SELECTOR = "0x70a08231"

ADDRESS_RE = re.compile(r"^0x[0-9a-f]{40}$")
HEX32_RE = re.compile(r"^0x[0-9a-f]{64}$")

RpcCallable = Callable[[str, list[Any]], Any]


class LiquidityReportError(RuntimeError):
    """Internal fail-closed check failure with deterministic blocker code."""

    def __init__(self, code: str, detail: str) -> None:
        super().__init__(detail)
        self.code = code
        self.detail = detail


def _norm_address(value: Any, label: str) -> str:
    if not isinstance(value, str):
        raise LiquidityReportError(f"invalid_{label}", f"{label} must be a hex address string")
    lower = value.strip().lower()
    if not ADDRESS_RE.match(lower):
        raise LiquidityReportError(f"invalid_{label}", f"{label} is not a 20-byte hex address")
    return lower


def _norm_bytes32(value: Any, label: str) -> str:
    if not isinstance(value, str):
        raise LiquidityReportError(f"invalid_{label}", f"{label} must be a 32-byte hex string")
    lower = value.strip().lower()
    if not HEX32_RE.match(lower):
        raise LiquidityReportError(f"invalid_{label}", f"{label} is not a 32-byte hex value")
    return lower


def _decode_uint256(hex_data: Any, label: str) -> int:
    if not isinstance(hex_data, str) or not hex_data.startswith("0x") or len(hex_data) < 66:
        raise LiquidityReportError("invalid_abi_word", f"{label} did not return a 32-byte ABI word")
    try:
        return int(hex_data[2:66], 16)
    except ValueError as exc:
        raise LiquidityReportError("invalid_abi_word", f"{label} returned non-hex ABI data") from exc


def _decode_address(hex_data: Any, label: str) -> str:
    if not isinstance(hex_data, str) or not hex_data.startswith("0x") or len(hex_data) < 66:
        raise LiquidityReportError("invalid_abi_address", f"{label} did not return a 32-byte ABI word")
    raw = "0x" + hex_data[26:66].lower()
    return _norm_address(raw, label)


def _decode_bool(hex_data: Any, label: str) -> bool:
    val = _decode_uint256(hex_data, label)
    if val not in (0, 1):
        raise LiquidityReportError("invalid_abi_bool", f"{label} returned non-boolean value {val}")
    return bool(val)


def _decode_policy_struct(hex_data: Any) -> dict[str, Any]:
    """Decode BoundedAgentWallet.policy() 13-word tuple return."""
    if not isinstance(hex_data, str) or not hex_data.startswith("0x"):
        raise LiquidityReportError("invalid_policy_tuple", "policy() did not return hex")
    body = hex_data[2:]
    if len(body) < 64 * 13:
        raise LiquidityReportError(
            "invalid_policy_tuple",
            f"policy() returned {len(body) // 64} words, expected 13 words",
        )
    words = [body[i * 64 : (i + 1) * 64] for i in range(13)]
    return {
        "delegate": _norm_address("0x" + words[0][24:], "delegate"),
        "valid_after": int(words[1], 16),
        "valid_until": int(words[2], 16),
        "period_seconds": int(words[3], 16),
        "max_per_action": int(words[4], 16),
        "max_per_period": int(words[5], 16),
        "max_lifetime_spend": int(words[6], 16),
        "max_bounty_target": int(words[7], 16),
        "allowed_actions": int(words[8], 16),
        "allowed_verification_modes": int(words[9], 16),
        "deterministic_verifier_module": _norm_address(
            "0x" + words[10][24:], "deterministic_verifier_module"
        ),
        "signed_quorum_verifier_set_hash": _norm_bytes32(
            "0x" + words[11], "signed_quorum_verifier_set_hash"
        ),
        "ai_judge_verifier_set_hash": _norm_bytes32(
            "0x" + words[12], "ai_judge_verifier_set_hash"
        ),
    }


def _encode_balance_of(wallet_address: str) -> str:
    clean = wallet_address.lower().removeprefix("0x")
    return ERC20_BALANCE_OF_SELECTOR + ("0" * 24) + clean


def _withheld_report(
    *,
    wallet: str,
    observed_block: int | None,
    status: str,
    blockers: Sequence[str],
    policy_hash: str | None = None,
    policy_version: int | None = None,
) -> dict[str, Any]:
    """Fail-closed report that withholds all financial numbers (None, never 0)."""
    return {
        "schema": REPORT_SCHEMA,
        "ok": False,
        "status": status,
        "wallet": wallet,
        "observed_block": observed_block,
        "policy_hash": policy_hash,
        "policy_version": policy_version,
        "usdc_balance": None,
        "lifetime_spent": None,
        "max_lifetime": None,
        "max_lifetime_spend": None,
        "period_spent": None,
        "max_per_period": None,
        "wallet_liquidity": None,
        "policy_authority": None,
        "claimable_escrow_separation": {
            "is_claimable_bounty_inventory": False,
            "claimable_escrow_usdc": None,
            "notice": (
                "Wallet liquidity and policy spend authority are NOT funded or claimable "
                "AgentBounty escrow inventory. Financial figures are withheld because "
                "verification failed closed."
            ),
        },
        "blockers": list(blockers),
    }


def generate_bounded_wallet_liquidity_report(
    config: Mapping[str, Any],
    rpc_call: RpcCallable,
    *,
    pinned_block: int | None = None,
) -> dict[str, Any]:
    """Read BoundedAgentWallet state and native Base USDC balance at one pinned block."""
    wallet = str(config.get("wallet", "")).strip().lower()
    if not ADDRESS_RE.match(wallet):
        return _withheld_report(
            wallet=wallet,
            observed_block=None,
            status="blocked",
            blockers=["invalid_wallet_address"],
        )

    expected_owner = _norm_address(config.get("owner", ""), "expected_owner")
    expected_delegate = (
        _norm_address(config["delegate"], "expected_delegate")
        if config.get("delegate")
        else None
    )
    expected_factory = (
        _norm_address(config["canonical_factory"], "expected_factory")
        if config.get("canonical_factory")
        else None
    )
    configured_token = _norm_address(
        config.get("settlement_token", NATIVE_BASE_USDC), "settlement_token"
    )
    expected_policy_hash = _norm_bytes32(
        config.get("wallet_policy_hash", ""), "wallet_policy_hash"
    )
    expected_policy_version = int(config.get("wallet_policy_version", -1))
    expected_chain_id = int(config.get("chain_id", BASE_MAINNET_CHAIN_ID))

    # 1. Verify configured token is canonical Base Mainnet native USDC before any read
    if configured_token != NATIVE_BASE_USDC:
        return _withheld_report(
            wallet=wallet,
            observed_block=None,
            status="blocked",
            blockers=["wrong_token_configuration"],
        )

    observed_block: int | None = None
    try:
        # 2. Verify chain ID == 8453 (Base Mainnet)
        raw_chain = rpc_call("eth_chainId", [])
        chain_id = int(str(raw_chain), 16) if str(raw_chain).startswith("0x") else int(raw_chain)
        if chain_id != BASE_MAINNET_CHAIN_ID or chain_id != expected_chain_id:
            return _withheld_report(
                wallet=wallet,
                observed_block=None,
                status="blocked",
                blockers=["wrong_chain"],
            )

        # 3. Determine single safe observed_block and pin ALL subsequent reads to block_tag
        if pinned_block is not None:
            observed_block = int(pinned_block)
        else:
            raw_block = rpc_call("eth_blockNumber", [])
            observed_block = (
                int(str(raw_block), 16) if str(raw_block).startswith("0x") else int(raw_block)
            )
        if observed_block <= 0:
            raise LiquidityReportError("invalid_observed_block", "observed_block must be > 0")
        block_tag = hex(observed_block)

        def call_contract(target: str, data: str, label: str) -> str:
            res = rpc_call("eth_call", [{"to": target, "data": data}, block_tag])
            if not isinstance(res, str) or not res.startswith("0x") or res == "0x":
                raise LiquidityReportError(f"empty_call_{label}", f"{label} returned empty data")
            return res

        # 4. Verify non-empty runtime bytecode on both wallet and native USDC at block_tag
        wallet_code = rpc_call("eth_getCode", [wallet, block_tag])
        if not isinstance(wallet_code, str) or wallet_code in ("0x", "0x0", ""):
            return _withheld_report(
                wallet=wallet,
                observed_block=observed_block,
                status="blocked",
                blockers=["missing_wallet_bytecode"],
            )
        expected_code_prefix = str(config.get("expected_wallet_code_prefix", "")).lower()
        if expected_code_prefix and not wallet_code.lower().startswith(expected_code_prefix):
            return _withheld_report(
                wallet=wallet,
                observed_block=observed_block,
                status="blocked",
                blockers=["wallet_bytecode_drift"],
            )

        token_code = rpc_call("eth_getCode", [NATIVE_BASE_USDC, block_tag])
        if not isinstance(token_code, str) or token_code in ("0x", "0x0", ""):
            return _withheld_report(
                wallet=wallet,
                observed_block=observed_block,
                status="blocked",
                blockers=["missing_token_bytecode"],
            )

        # 5. Prove on-chain settlementToken() binding to canonical Base Mainnet native USDC
        onchain_token = _decode_address(
            call_contract(
                wallet,
                BOUNDED_AGENT_WALLET_ABI_SELECTORS["settlementToken()"],
                "settlementToken",
            ),
            "settlementToken",
        )
        if onchain_token != NATIVE_BASE_USDC:
            return _withheld_report(
                wallet=wallet,
                observed_block=observed_block,
                status="blocked",
                blockers=["wrong_token_binding"],
            )

        # 6. Verify owner() and optional factory()
        onchain_owner = _decode_address(
            call_contract(wallet, BOUNDED_AGENT_WALLET_ABI_SELECTORS["owner()"], "owner"),
            "owner",
        )
        if onchain_owner != expected_owner:
            return _withheld_report(
                wallet=wallet,
                observed_block=observed_block,
                status="blocked",
                blockers=["owner_drift"],
            )

        if expected_factory is not None:
            onchain_factory = _decode_address(
                call_contract(wallet, BOUNDED_AGENT_WALLET_ABI_SELECTORS["factory()"], "factory"),
                "factory",
            )
            if onchain_factory != expected_factory:
                return _withheld_report(
                    wallet=wallet,
                    observed_block=observed_block,
                    status="blocked",
                    blockers=["factory_drift"],
                )

        # 7. Verify policyVersion(), policyHash(), revoked(), and policy() struct at block_tag
        onchain_policy_version = _decode_uint256(
            call_contract(
                wallet,
                BOUNDED_AGENT_WALLET_ABI_SELECTORS["policyVersion()"],
                "policyVersion",
            ),
            "policyVersion",
        )
        onchain_policy_hash = _norm_bytes32(
            call_contract(
                wallet,
                BOUNDED_AGENT_WALLET_ABI_SELECTORS["policyHash()"],
                "policyHash",
            )[:66],
            "policyHash",
        )
        onchain_revoked = _decode_bool(
            call_contract(wallet, BOUNDED_AGENT_WALLET_ABI_SELECTORS["revoked()"], "revoked"),
            "revoked",
        )
        decoded_policy = _decode_policy_struct(
            call_contract(wallet, BOUNDED_AGENT_WALLET_ABI_SELECTORS["policy()"], "policy")
        )

        drift_blockers: list[str] = []
        if onchain_revoked:
            drift_blockers.append("policy_revoked")
        if onchain_policy_version != expected_policy_version:
            drift_blockers.append("policy_version_drift")
        if onchain_policy_hash != expected_policy_hash:
            drift_blockers.append("policy_hash_drift")
        if expected_delegate and decoded_policy["delegate"] != expected_delegate:
            drift_blockers.append("delegate_drift")

        if drift_blockers:
            return _withheld_report(
                wallet=wallet,
                observed_block=observed_block,
                status="blocked",
                blockers=drift_blockers,
                policy_hash=onchain_policy_hash,
                policy_version=onchain_policy_version,
            )

        # 8. Read spend counters & native USDC balance at the exact same block_tag
        period_spent = _decode_uint256(
            call_contract(
                wallet, BOUNDED_AGENT_WALLET_ABI_SELECTORS["periodSpent()"], "periodSpent"
            ),
            "periodSpent",
        )
        lifetime_spent = _decode_uint256(
            call_contract(
                wallet, BOUNDED_AGENT_WALLET_ABI_SELECTORS["lifetimeSpent()"], "lifetimeSpent"
            ),
            "lifetimeSpent",
        )
        period_bucket = _decode_uint256(
            call_contract(
                wallet, BOUNDED_AGENT_WALLET_ABI_SELECTORS["periodBucket()"], "periodBucket"
            ),
            "periodBucket",
        )
        usdc_balance = _decode_uint256(
            call_contract(
                NATIVE_BASE_USDC,
                _encode_balance_of(wallet),
                "usdc_balanceOf",
            ),
            "usdc_balanceOf",
        )

    except LiquidityReportError as err:
        return _withheld_report(
            wallet=wallet,
            observed_block=observed_block,
            status="blocked",
            blockers=[err.code],
        )
    except Exception:
        return _withheld_report(
            wallet=wallet,
            observed_block=observed_block,
            status="unavailable",
            blockers=["rpc_unavailable"],
        )

    max_per_period = decoded_policy["max_per_period"]
    max_lifetime = decoded_policy["max_lifetime_spend"]
    remaining_period_authority = max(0, max_per_period - period_spent)
    remaining_lifetime_authority = max(0, max_lifetime - lifetime_spent)
    remaining_authority = min(remaining_period_authority, remaining_lifetime_authority)
    effective_spendable_usdc = min(usdc_balance, remaining_authority)

    state_flags: list[str] = []
    if usdc_balance == 0:
        state_flags.append("empty_wallet_balance")
    if remaining_period_authority == 0:
        state_flags.append("period_cap_exhausted")
    if remaining_lifetime_authority == 0:
        state_flags.append("lifetime_cap_exhausted")

    if state_flags:
        status = "exhausted" if "empty_wallet_balance" not in state_flags else (
            "empty" if len(state_flags) == 1 else "empty_and_cap_exhausted"
        )
    else:
        status = "healthy"

    return {
        "schema": REPORT_SCHEMA,
        "ok": True,
        "status": status,
        "wallet": wallet,
        "observed_block": observed_block,
        "settlement_token": NATIVE_BASE_USDC,
        "owner": onchain_owner,
        "delegate": decoded_policy["delegate"],
        "policy_hash": onchain_policy_hash,
        "policy_version": onchain_policy_version,
        # Top-level canonical keys required by benchmark and consumers
        "usdc_balance": usdc_balance,
        "lifetime_spent": lifetime_spent,
        "max_lifetime": max_lifetime,
        "max_lifetime_spend": max_lifetime,
        "period_spent": period_spent,
        "max_per_period": max_per_period,
        # Explicit separation between liquid balance, policy authority, and claimable escrow
        "wallet_liquidity": {
            "settlement_token": NATIVE_BASE_USDC,
            "usdc_balance": usdc_balance,
            "effective_spendable_usdc": effective_spendable_usdc,
            "is_bounty_escrow": False,
        },
        "policy_authority": {
            "policy_hash": onchain_policy_hash,
            "policy_version": onchain_policy_version,
            "revoked": False,
            "period_bucket": period_bucket,
            "period_seconds": decoded_policy["period_seconds"],
            "period_spent": period_spent,
            "max_per_period": max_per_period,
            "remaining_period_authority": remaining_period_authority,
            "lifetime_spent": lifetime_spent,
            "max_lifetime_spend": max_lifetime,
            "remaining_lifetime_authority": remaining_lifetime_authority,
            "remaining_authority": remaining_authority,
            "max_per_action": decoded_policy["max_per_action"],
            "max_bounty_target": decoded_policy["max_bounty_target"],
        },
        "claimable_escrow_separation": {
            "is_claimable_bounty_inventory": False,
            "claimable_escrow_usdc": None,
            "notice": (
                "Liquid wallet USDC and delegate policy authority represent unallocated "
                "wallet capacity only; they are NOT funded or claimable AgentBounty escrow."
            ),
        },
        "blockers": state_flags,
    }


class BoundedWalletLiquidityReporter:
    """Object wrapper around generate_bounded_wallet_liquidity_report."""

    def __init__(self, config: Mapping[str, Any], rpc_call: RpcCallable) -> None:
        self._config = dict(config)
        self._rpc_call = rpc_call

    def generate_report(self, *, pinned_block: int | None = None) -> dict[str, Any]:
        return generate_bounded_wallet_liquidity_report(
            self._config, self._rpc_call, pinned_block=pinned_block
        )

    generateReport = generate_report


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path, help="Path to direct inventory manifest JSON")
    args = parser.parse_args(argv)
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    try:
        from scripts._shared.rpc import rpc as shared_rpc
    except ImportError:
        from _shared.rpc import rpc as shared_rpc  # type: ignore

    report = generate_bounded_wallet_liquidity_report(
        manifest,
        lambda method, params: shared_rpc(method, params),
    )
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
