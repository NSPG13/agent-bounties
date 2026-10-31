#!/usr/bin/env python3
"""Bounded-wallet liquidity and policy authority reporter (Issue #871).

Reports actual spendable native Base USDC held in a BoundedAgentWallet
separately from remaining delegate policy authority and separately from
claimable AgentBounty escrow inventory.

All on-chain reads are pinned to one single `observed_block` (resolved via
`eth_getBlockByNumber("safe", False)` by default). Any chain, bytecode,
settlement-token, owner, revocation, policy-hash, policy-version, validity
window, or RPC failure fails closed and withholds financial figures (`None`
rather than substituting `0`).
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
CANONICAL_BOUNTY_FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
CANONICAL_WALLET_FACTORIES: Mapping[str, str] = {
    "0x3840936351049aed639780a16845e6094c1f17f6": "0x40d3e16082cf71ece0129ca3044e1b8233e29db8",
    "0xe3d4f7b203c5e8576e0225d3e64a8532429d3876": "0x00c250bda8fa3c49d80a11d9b6ebd961736b7202",
}

# Exact Solidity 4-byte selectors matching contracts/base-escrow/src/BoundedAgentWallet.sol
BOUNDED_AGENT_WALLET_ABI_SELECTORS: Mapping[str, str] = {
    "owner()": "0x8da5cb5b",
    "settlementToken()": "0x7b9e618d",
    "factory()": "0xc45a0155",
    "deploymentFactory()": "0x8ea261cc",
    "policy()": "0x0505c8c9",
    "policyVersion()": "0x58355ead",
    "policyHash()": "0x098fb624",
    "periodBucket()": "0xcb2e5d9c",
    "periodSpent()": "0x81497000",
    "lifetimeSpent()": "0xb80762dd",
    "revoked()": "0x63d256ce",
    "delegateNonce()": "0x93e67715",
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
    if not ADDRESS_RE.match(lower) or lower == "0x" + "00" * 20:
        raise LiquidityReportError(f"invalid_{label}", f"{label} is not a non-zero 20-byte hex address")
    return lower


def _norm_bytes32(value: Any, label: str) -> str:
    if not isinstance(value, str):
        raise LiquidityReportError(f"invalid_{label}", f"{label} must be a 32-byte hex string")
    lower = value.strip().lower()
    if not HEX32_RE.match(lower):
        raise LiquidityReportError(f"invalid_{label}", f"{label} is not a 32-byte hex value")
    return lower


def _decode_uint256(hex_data: Any, label: str) -> int:
    if not isinstance(hex_data, str) or not hex_data.startswith("0x") or len(hex_data) != 66:
        raise LiquidityReportError("invalid_abi_word", f"{label} did not return a single 32-byte ABI word")
    try:
        return int(hex_data[2:66], 16)
    except ValueError as exc:
        raise LiquidityReportError("invalid_abi_word", f"{label} returned non-hex ABI data") from exc


def _decode_address(hex_data: Any, label: str) -> str:
    if not isinstance(hex_data, str) or not hex_data.startswith("0x") or len(hex_data) != 66:
        raise LiquidityReportError("invalid_abi_address", f"{label} did not return a single 32-byte ABI word")
    if hex_data[2:26] != "0" * 24:
        raise LiquidityReportError("invalid_abi_address", f"{label} has non-zero upper 12-byte padding")
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
    if len(body) != 64 * 13:
        raise LiquidityReportError(
            "invalid_policy_tuple",
            f"policy() returned {len(body) // 64} words, expected exact 13 words",
        )
    words = [body[i * 64 : (i + 1) * 64] for i in range(13)]
    if words[0][:24] != "0" * 24:
        raise LiquidityReportError("invalid_policy_tuple", "delegate address word has non-zero padding")
    if words[10][:24] != "0" * 24:
        raise LiquidityReportError("invalid_policy_tuple", "deterministic_verifier_module word has non-zero padding")
    det_addr = "0x" + words[10][24:].lower()
    if not ADDRESS_RE.match(det_addr):
        raise LiquidityReportError("invalid_policy_tuple", "invalid deterministic_verifier_module address")
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
        "deterministic_verifier_module": det_addr,
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


def _extract_eip1167_implementation(runtime_code: str) -> str | None:
    """Return the 20-byte implementation address if runtime_code is an exact EIP-1167 minimal proxy."""
    lower = runtime_code.strip().lower()
    prefix = "0x363d3d373d3d3d363d73"
    suffix = "5af43d82803e903d91602b57fd5bf3"
    if len(lower) == len(prefix) + 40 + len(suffix) and lower.startswith(prefix) and lower.endswith(suffix):
        impl = "0x" + lower[len(prefix) : len(prefix) + 40]
        if ADDRESS_RE.match(impl):
            return impl
    return None


def _withheld_report(
    *,
    wallet: str,
    observed_block: int | None,
    status: str,
    blockers: Sequence[str],
    observed_block_hash: str | None = None,
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
        "observed_block_hash": observed_block_hash,
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
    """Read BoundedAgentWallet state and native Base USDC balance at one pinned safe block."""
    wallet = str(config.get("wallet", "")).strip().lower()
    if not ADDRESS_RE.match(wallet) or wallet == "0x" + "00" * 20:
        return _withheld_report(
            wallet=wallet,
            observed_block=None,
            status="blocked",
            blockers=["invalid_wallet_address"],
        )

    try:
        expected_owner = _norm_address(config.get("owner", ""), "expected_owner")
        expected_delegate = (
            _norm_address(config["delegate"], "expected_delegate")
            if config.get("delegate")
            else None
        )
        expected_factory = (
            _norm_address(config["canonical_factory"], "expected_factory")
            if config.get("canonical_factory")
            else CANONICAL_BOUNTY_FACTORY
        )
        expected_deployment_factory = (
            _norm_address(config["wallet_factory"], "expected_wallet_factory")
            if config.get("wallet_factory")
            else None
        )
        configured_token = _norm_address(
            config.get("settlement_token", NATIVE_BASE_USDC), "settlement_token"
        )
        expected_policy_hash = _norm_bytes32(
            config.get("wallet_policy_hash", ""), "wallet_policy_hash"
        )
        raw_pv = config.get("wallet_policy_version")
        if isinstance(raw_pv, bool) or not isinstance(raw_pv, int) or raw_pv <= 0:
            raise LiquidityReportError("invalid_policy_version", "wallet_policy_version must be a positive int")
        expected_policy_version = raw_pv
        expected_chain_id = int(config.get("chain_id", BASE_MAINNET_CHAIN_ID))
    except LiquidityReportError as cfg_err:
        return _withheld_report(
            wallet=wallet,
            observed_block=None,
            status="blocked",
            blockers=[cfg_err.code],
        )

    # 1. Verify configured token is canonical Base Mainnet native USDC before any read
    if configured_token != NATIVE_BASE_USDC:
        return _withheld_report(
            wallet=wallet,
            observed_block=None,
            status="blocked",
            blockers=["wrong_token_configuration"],
        )

    observed_block: int | None = None
    observed_block_hash: str | None = None
    observed_timestamp: int | None = None
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

        # 3. Resolve single safe observed_block and pin ALL subsequent reads to block_tag
        if pinned_block is not None:
            if isinstance(pinned_block, bool) or int(pinned_block) <= 0:
                raise LiquidityReportError("invalid_observed_block", "pinned_block must be > 0")
            observed_block = int(pinned_block)
            block_tag = hex(observed_block)
            try:
                blk = rpc_call("eth_getBlockByNumber", [block_tag, False])
                if isinstance(blk, Mapping):
                    if isinstance(blk.get("hash"), str) and HEX32_RE.match(blk["hash"].lower()):
                        observed_block_hash = blk["hash"].lower()
                    if blk.get("timestamp") is not None:
                        ts_raw = str(blk["timestamp"])
                        observed_timestamp = int(ts_raw, 16) if ts_raw.startswith("0x") else int(ts_raw)
            except AssertionError:
                pass
        else:
            safe_blk: Any = None
            try:
                safe_blk = rpc_call("eth_getBlockByNumber", ["safe", False])
            except AssertionError:
                safe_blk = None
            if isinstance(safe_blk, Mapping) and safe_blk.get("number") is not None:
                num_raw = str(safe_blk["number"])
                observed_block = int(num_raw, 16) if num_raw.startswith("0x") else int(num_raw)
                if isinstance(safe_blk.get("hash"), str) and HEX32_RE.match(safe_blk["hash"].lower()):
                    observed_block_hash = safe_blk["hash"].lower()
                if safe_blk.get("timestamp") is not None:
                    ts_raw = str(safe_blk["timestamp"])
                    observed_timestamp = int(ts_raw, 16) if ts_raw.startswith("0x") else int(ts_raw)
            else:
                raw_block = rpc_call("eth_blockNumber", [])
                observed_block = (
                    int(str(raw_block), 16) if str(raw_block).startswith("0x") else int(raw_block)
                )

        if observed_block is None or observed_block <= 0:
            raise LiquidityReportError("invalid_observed_block", "observed_block must be > 0")
        block_tag = hex(observed_block)

        def call_contract(target: str, data: str, label: str) -> str:
            res = rpc_call("eth_call", [{"to": target, "data": data}, block_tag])
            if not isinstance(res, str) or not res.startswith("0x") or res == "0x":
                raise LiquidityReportError(f"empty_call_{label}", f"{label} returned empty data")
            return res

        # 4. Verify non-empty runtime bytecode on both wallet and native USDC at block_tag
        wallet_code = rpc_call("eth_getCode", [wallet, block_tag])
        if not isinstance(wallet_code, str) or wallet_code.lower() in ("0x", "0x0", ""):
            return _withheld_report(
                wallet=wallet,
                observed_block=observed_block,
                observed_block_hash=observed_block_hash,
                status="blocked",
                blockers=["missing_wallet_bytecode"],
            )
        expected_code_prefix = str(config.get("expected_wallet_code_prefix", "")).lower()
        if expected_code_prefix and not wallet_code.lower().startswith(expected_code_prefix):
            return _withheld_report(
                wallet=wallet,
                observed_block=observed_block,
                observed_block_hash=observed_block_hash,
                status="blocked",
                blockers=["wallet_bytecode_drift"],
            )

        # Verify EIP-1167 clone implementation matches canonical bounded-wallet implementation if configured or recognized
        clone_impl = _extract_eip1167_implementation(wallet_code)
        expected_impl = (
            _norm_address(config["expected_implementation"], "expected_implementation")
            if config.get("expected_implementation")
            else None
        )
        if expected_impl and clone_impl != expected_impl:
            return _withheld_report(
                wallet=wallet,
                observed_block=observed_block,
                observed_block_hash=observed_block_hash,
                status="blocked",
                blockers=["wallet_bytecode_drift"],
            )

        token_code = rpc_call("eth_getCode", [NATIVE_BASE_USDC, block_tag])
        if not isinstance(token_code, str) or token_code.lower() in ("0x", "0x0", ""):
            return _withheld_report(
                wallet=wallet,
                observed_block=observed_block,
                observed_block_hash=observed_block_hash,
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
                observed_block_hash=observed_block_hash,
                status="blocked",
                blockers=["wrong_token_binding"],
            )

        # 6. Verify owner(), factory(), and optional deploymentFactory()
        onchain_owner = _decode_address(
            call_contract(wallet, BOUNDED_AGENT_WALLET_ABI_SELECTORS["owner()"], "owner"),
            "owner",
        )
        if onchain_owner != expected_owner:
            return _withheld_report(
                wallet=wallet,
                observed_block=observed_block,
                observed_block_hash=observed_block_hash,
                status="blocked",
                blockers=["owner_drift"],
            )

        onchain_factory = _decode_address(
            call_contract(wallet, BOUNDED_AGENT_WALLET_ABI_SELECTORS["factory()"], "factory"),
            "factory",
        )
        if onchain_factory != expected_factory:
            return _withheld_report(
                wallet=wallet,
                observed_block=observed_block,
                observed_block_hash=observed_block_hash,
                status="blocked",
                blockers=["factory_drift"],
            )

        if expected_deployment_factory is not None:
            onchain_dep_factory = _decode_address(
                call_contract(
                    wallet,
                    BOUNDED_AGENT_WALLET_ABI_SELECTORS["deploymentFactory()"],
                    "deploymentFactory",
                ),
                "deploymentFactory",
            )
            if onchain_dep_factory != expected_deployment_factory:
                return _withheld_report(
                    wallet=wallet,
                    observed_block=observed_block,
                    observed_block_hash=observed_block_hash,
                    status="blocked",
                    blockers=["wallet_factory_drift"],
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
        if decoded_policy["period_seconds"] <= 0:
            drift_blockers.append("invalid_policy_period")
        if decoded_policy["max_per_period"] <= 0 or decoded_policy["max_lifetime_spend"] <= 0:
            drift_blockers.append("invalid_policy_caps")
        if observed_timestamp is not None:
            if observed_timestamp < decoded_policy["valid_after"]:
                drift_blockers.append("policy_not_yet_active")
            if observed_timestamp > decoded_policy["valid_until"]:
                drift_blockers.append("policy_expired")

        if drift_blockers:
            return _withheld_report(
                wallet=wallet,
                observed_block=observed_block,
                observed_block_hash=observed_block_hash,
                status="blocked",
                blockers=drift_blockers,
                policy_hash=onchain_policy_hash,
                policy_version=onchain_policy_version,
            )

        # 8. Read spend counters & native USDC balance at the exact same block_tag
        period_spent_raw = _decode_uint256(
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
            observed_block_hash=observed_block_hash,
            status="blocked",
            blockers=[err.code],
        )
    except Exception:
        return _withheld_report(
            wallet=wallet,
            observed_block=observed_block,
            observed_block_hash=observed_block_hash,
            status="unavailable",
            blockers=["rpc_unavailable"],
        )

    # Account for period rollover when safe block timestamp is available
    period_seconds = decoded_policy["period_seconds"]
    if observed_timestamp is not None and period_seconds > 0:
        current_bucket = observed_timestamp // period_seconds
        period_spent = period_spent_raw if current_bucket == period_bucket else 0
    else:
        period_spent = period_spent_raw

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
        "observed_block_hash": observed_block_hash,
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
            "period_seconds": period_seconds,
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
    parser.add_argument("--rpc-url", default=None, help="Optional preferred Base HTTPS RPC endpoint")
    parser.add_argument("--pinned-block", type=int, default=None, help="Optional block number to pin reads to")
    args = parser.parse_args(argv)
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    try:
        from scripts._shared.rpc import rpc as shared_rpc, select_working_base_rpc
    except ImportError:
        from _shared.rpc import rpc as shared_rpc, select_working_base_rpc  # type: ignore

    preferred_url = args.rpc_url or str(manifest.get("rpc_url") or "").strip() or None
    try:
        endpoint = select_working_base_rpc(preferred=preferred_url)
    except Exception:
        report = _withheld_report(
            wallet=str(manifest.get("wallet", "")).strip().lower(),
            observed_block=None,
            status="unavailable",
            blockers=["rpc_unavailable"],
        )
        print(json.dumps(report, indent=2, sort_keys=True))
        return 1

    request_seq = 0

    def _bound_rpc(method: str, params: list[Any]) -> Any:
        nonlocal request_seq
        request_seq += 1
        return shared_rpc(endpoint, method, params, request_id=request_seq)

    report = generate_bounded_wallet_liquidity_report(
        manifest,
        _bound_rpc,
        pinned_block=args.pinned_block,
    )
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
