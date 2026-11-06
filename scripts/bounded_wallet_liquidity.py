#!/usr/bin/env python3
"""Bounded-wallet liquidity report (#871).

Reports actual spendable USDC separately from remaining policy authority.
Never exposes signer secrets, and never implies either amount is bounty
inventory. Fails closed on wrong chain, bytecode, token, owner or policy drift.
"""
from __future__ import annotations

SCHEMA_VERSION = "bounded-wallet-liquidity/v1"

CHAIN_ID_BASE_MAINNET = 8453
USDC_BASE_MAINNET = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"

# Fail-closed reasons. Every one of these makes ok False.
REASON_CHAIN = "wrong_chain"
REASON_BYTECODE = "bytecode_mismatch"
REASON_TOKEN = "token_mismatch"
REASON_OWNER = "owner_mismatch"
REASON_POLICY = "policy_drift"
REASON_RPC = "rpc_unavailable"


def _fail(reason: str, observed_block=None) -> dict:
    return {"ok": False, "schema": SCHEMA_VERSION, "reason": reason,
            "usdc_balance": None, "lifetime_spent": None, "max_lifetime": None,
            "period_spent": None, "max_per_period": None,
            "policy_hash": None, "policy_version": None,
            "observed_block": observed_block,
            "labels": {"liquid_balance": None, "remaining_lifetime_authority": None,
                       "remaining_period_authority": None, "claimable_escrow": None}}


def liquidity_report(observed: dict, expected: dict) -> dict:
    """Build a read-only liquidity report from an observed on-chain snapshot."""
    if observed.get("rpc_available") is False or observed.get("chain_id") is None:
        return _fail(REASON_RPC, observed.get("observed_block"))
    if int(observed["chain_id"]) != int(expected.get("chain_id", CHAIN_ID_BASE_MAINNET)):
        return _fail(REASON_CHAIN, observed.get("observed_block"))
    if observed.get("bytecode_hash") != expected.get("bytecode_hash"):
        return _fail(REASON_BYTECODE, observed.get("observed_block"))
    if observed.get("token_address", "").lower() != expected.get("token_address", USDC_BASE_MAINNET).lower():
        return _fail(REASON_TOKEN, observed.get("observed_block"))
    if observed.get("owner", "").lower() != expected.get("owner", "").lower():
        return _fail(REASON_OWNER, observed.get("observed_block"))
    if observed.get("policy_hash") != expected.get("policy_hash"):
        return _fail(REASON_POLICY, observed.get("observed_block"))

    usdc_balance = observed.get("usdc_balance", 0)
    lifetime_spent = observed.get("lifetime_spent", 0)
    max_lifetime = observed.get("max_lifetime")
    period_spent = observed.get("period_spent", 0)
    max_per_period = observed.get("max_per_period")

    return {
        "ok": True, "schema": SCHEMA_VERSION, "reason": None,
        "chain_id": int(observed["chain_id"]),
        "usdc_balance": usdc_balance,
        "lifetime_spent": lifetime_spent, "max_lifetime": max_lifetime,
        "period_spent": period_spent, "max_per_period": max_per_period,
        "policy_hash": observed.get("policy_hash"),
        "policy_version": observed.get("policy_version"),
        "observed_block": observed.get("observed_block"),
        # Three different quantities, kept explicitly distinct:
        "labels": {
            "liquid_balance": usdc_balance,
            "remaining_lifetime_authority": None if max_lifetime is None
            else max_lifetime - lifetime_spent,
            "remaining_period_authority": None if max_per_period is None
            else max_per_period - period_spent,
            "claimable_escrow": observed.get("claimable_escrow"),
        },
    }
