#!/usr/bin/env python3
"""Read-only Base-mainnet liquidity report for the configured bounded wallet.

Reports actual spendable USDC (liquid balance) separately from remaining
policy authority and from escrowed bounty inventory. The report is
read-only: it never signs, never broadcasts, and never moves funds.

Fail-closed contract: on wrong chain, wrong token, wrong bytecode, wrong
owner, policy drift, or RPC unavailability, no spendable quantity is ever
reported from unverified state -- the report raises ``LiquidityReportError``
instead of inventing balances.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

# Canonical Base mainnet anchors used for fail-closed validation.
BASE_CHAIN_ID = 8453
# Canonical USDC on Base mainnet.
BASE_USDC_ADDRESS = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"
# Canonical bounded-wallet bytecode hash (shared with bounty_inventory_guard.py).
BOUNDED_WALLET_BYTECODE_HASH = (
    "0x25c41d7d51e2c807754b901733de17cdb1778dbd353f86347ff33e10289fcb54"
)

REPORT_VERSION = "bounded-wallet-liquidity-v1"


class LiquidityReportError(RuntimeError):
    """Raised when a fail-closed condition makes spendable balances unknowable."""


@dataclass(frozen=True)
class WalletSnapshot:
    """Raw, unverified observation of the bounded wallet at one block.

    Every field mirrors one RPC or policy-source read; nothing here is
    inferred. ``rpc_available`` must be false whenever the observation
    could not be produced from a live canonical RPC response.
    """

    chain_id: int
    token_address: str
    wallet_address: str
    owner_address: str
    bytecode_hash: str
    usdc_balance: float
    lifetime_spent: float
    max_lifetime: float
    period_spent: float
    max_per_period: float
    period_started_at: str
    policy_version: str
    policy_hash: str
    observed_block: int
    rpc_available: bool


@dataclass(frozen=True)
class WalletPolicy:
    """Expected policy anchor used to detect policy drift."""

    policy_version: str
    policy_hash: str
    expected_owner_address: str


def _fail(reason: str) -> None:
    raise LiquidityReportError(reason)


def build_liquidity_report(
    snapshot: WalletSnapshot, policy: WalletPolicy | None = None
) -> dict[str, Any]:
    """Build the read-only liquidity report, failing closed on any drift.

    The three spendable-adjacent quantities are reported as *different*
    labeled quantities:

    - ``liquid_usdc_balance`` -- actual USDC held by the bounded wallet now;
    - ``remaining_lifetime_authority`` -- ``max_lifetime - lifetime_spent``;
    - ``remaining_period_authority`` -- ``max_per_period - period_spent``.

    Currently claimable escrow is a fourth, distinct quantity that this
    report deliberately does NOT estimate: it is only ever reported by the
    canonical inventory verifier after on-chain verification.
    """

    if not snapshot.rpc_available:
        _fail(
            "rpc_unavailable: refusing to report spendable balances from "
            "unverified RPC state"
        )
    if snapshot.chain_id != BASE_CHAIN_ID:
        _fail(f"wrong chain: expected {BASE_CHAIN_ID}, got {snapshot.chain_id}")
    if snapshot.token_address.lower() != BASE_USDC_ADDRESS.lower():
        _fail(f"wrong token: expected Base USDC, got {snapshot.token_address}")
    if snapshot.bytecode_hash.lower() != BOUNDED_WALLET_BYTECODE_HASH.lower():
        _fail("wrong bytecode: bounded-wallet bytecode hash mismatch")
    if policy is not None:
        if snapshot.owner_address.lower() != policy.expected_owner_address.lower():
            _fail(
                f"wrong owner: expected {policy.expected_owner_address}, "
                f"got {snapshot.owner_address}"
            )
        if (
            snapshot.policy_version != policy.policy_version
            or snapshot.policy_hash.lower() != policy.policy_hash.lower()
        ):
            _fail(
                "policy drift: observed policy version/hash does not match "
                "the expected policy anchor"
            )

    remaining_lifetime_authority = max(0.0, snapshot.max_lifetime - snapshot.lifetime_spent)
    remaining_period_authority = max(0.0, snapshot.max_per_period - snapshot.period_spent)

    return {
        "report_version": REPORT_VERSION,
        "status": "ok",
        "observed_block": snapshot.observed_block,
        "observed_at": datetime.now(timezone.utc).isoformat(),
        "wallet_address": snapshot.wallet_address,
        "quantities": {
            "liquid_usdc_balance": snapshot.usdc_balance,
            "remaining_lifetime_authority": remaining_lifetime_authority,
            "remaining_period_authority": remaining_period_authority,
            "claimable_escrow": None,
        },
        "authority": {
            "lifetime_spent": snapshot.lifetime_spent,
            "max_lifetime": snapshot.max_lifetime,
            "period_spent": snapshot.period_spent,
            "max_per_period": snapshot.max_per_period,
            "period_started_at": snapshot.period_started_at,
        },
        "policy": {
            "policy_version": snapshot.policy_version,
            "policy_hash": snapshot.policy_hash,
        },
        "notes": {
            "claimable_escrow": (
                "escrowed bounty inventory is a different quantity, reported "
                "only by the canonical inventory verifier"
            )
        },
    }


def snapshot_from_dict(raw: dict[str, Any]) -> WalletSnapshot:
    return WalletSnapshot(
        chain_id=int(raw["chain_id"]),
        token_address=str(raw["token_address"]),
        wallet_address=str(raw["wallet_address"]),
        owner_address=str(raw["owner_address"]),
        bytecode_hash=str(raw["bytecode_hash"]),
        usdc_balance=float(raw["usdc_balance"]),
        lifetime_spent=float(raw["lifetime_spent"]),
        max_lifetime=float(raw["max_lifetime"]),
        period_spent=float(raw["period_spent"]),
        max_per_period=float(raw["max_per_period"]),
        period_started_at=str(raw["period_started_at"]),
        policy_version=str(raw["policy_version"]),
        policy_hash=str(raw["policy_hash"]),
        observed_block=int(raw["observed_block"]),
        rpc_available=bool(raw["rpc_available"]),
    )


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--snapshot",
        required=True,
        help="path to a wallet snapshot JSON (see scripts/fixtures/bounded-wallet-liquidity/)",
    )
    parser.add_argument(
        "--policy",
        default=None,
        help="optional expected policy anchor JSON (policy_version, policy_hash, expected_owner_address)",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    raw = json.loads(Path(args.snapshot).read_text(encoding="utf-8"))
    snapshot = snapshot_from_dict(raw)
    policy: WalletPolicy | None = None
    if args.policy:
        p = json.loads(Path(args.policy).read_text(encoding="utf-8"))
        policy = WalletPolicy(
            policy_version=str(p["policy_version"]),
            policy_hash=str(p["policy_hash"]),
            expected_owner_address=str(p["expected_owner_address"]),
        )
    try:
        report = build_liquidity_report(snapshot, policy)
    except LiquidityReportError as exc:
        print(
            json.dumps(
                {
                    "report_version": REPORT_VERSION,
                    "status": "fail_closed",
                    "reason": str(exc),
                },
                indent=2,
            )
        )
        return 1
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
