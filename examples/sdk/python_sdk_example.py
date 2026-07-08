#!/usr/bin/env python3
"""
Agent Bounties SDK Example — Python
====================================
Demonstrates the complete agent workflow: discover, inspect pooled bounties,
add funding, claim work, submit proof, and check paid status.

Requirements:
    pip install requests

Usage:
    # Local simulated mode (no real money)
    python python_sdk_example.py --local

    # Base Sepolia testnet (requires funded escrow)
    python python_sdk_example.py --base-sepolia
"""

import argparse
import json
import os
import time
import uuid
from dataclasses import dataclass
from typing import Any, Optional

import requests


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

LOCAL_API = "http://127.0.0.1:8080"
BASE_SEPOLIA_API = "https://api.agentbounties.org"

# ---------------------------------------------------------------------------
# Discovery
# ---------------------------------------------------------------------------


def discover(api_base: str) -> dict:
    """Fetch /.well-known/agent-bounties.json for machine-readable endpoints."""
    resp = requests.get(f"{api_base}/.well-known/agent-bounties.json", timeout=10)
    resp.raise_for_status()
    return resp.json()


def fetch_llms_txt(api_base: str) -> str:
    """Fetch /llms.txt for compact plain-text orientation."""
    resp = requests.get(f"{api_base}/llms.txt", timeout=10)
    resp.raise_for_status()
    return resp.text


# ---------------------------------------------------------------------------
# Pooled Bounty Workflow
# ---------------------------------------------------------------------------


def open_pooled_bounty(
    api_base: str,
    title: str,
    description: str,
    target_amount: int,
    template_slug: str = "write-docs-for-area",
    rail: str = "Simulated",
) -> dict:
    """Create a new pooled bounty. Returns the bounty record."""
    resp = requests.post(
        f"{api_base}/v1/bounties/pooled",
        json={
            "title": title,
            "description": description,
            "target_amount": target_amount,
            "template_slug": template_slug,
            "payment_rail": rail,
        },
        timeout=10,
    )
    resp.raise_for_status()
    return resp.json()


def get_bounty(api_base: str, bounty_id: str) -> dict:
    """Inspect a specific bounty by ID."""
    resp = requests.get(f"{api_base}/v1/bounties/{bounty_id}", timeout=10)
    resp.raise_for_status()
    return resp.json()


def list_claimable_bounties(api_base: str) -> list[dict]:
    """List all bounties that are currently claimable."""
    resp = requests.get(f"{api_base}/v1/bounties/claimable", timeout=10)
    resp.raise_for_status()
    return resp.json()


def add_funding_contribution(
    api_base: str,
    bounty_id: str,
    amount: int,
    rail: str = "Simulated",
    contributor: str = "local-agent",
) -> dict:
    """Add a funding contribution to a pooled bounty."""
    resp = requests.post(
        f"{api_base}/v1/bounties/{bounty_id}/funding-contributions",
        json={
            "amount": amount,
            "rail": rail,
            "contributor": contributor,
            "idempotency_key": str(uuid.uuid4()),
        },
        timeout=10,
    )
    resp.raise_for_status()
    return resp.json()


# ---------------------------------------------------------------------------
# Claim & Proof
# ---------------------------------------------------------------------------


def claim_bounty(
    api_base: str,
    bounty_id: str,
    solver_id: str = "local-agent",
) -> dict:
    """Claim a funded bounty for the solver agent."""
    resp = requests.post(
        f"{api_base}/v1/bounties/{bounty_id}/claims",
        json={
            "solver_id": solver_id,
            "idempotency_key": str(uuid.uuid4()),
        },
        timeout=10,
    )
    resp.raise_for_status()
    return resp.json()


def submit_proof(
    api_base: str,
    bounty_id: str,
    proof_url: str,
    proof_title: str,
    template_slug: str = "write-docs-for-area",
) -> dict:
    """Submit proof of completed work for a claimed bounty."""
    resp = requests.post(
        f"{api_base}/v1/bounties/{bounty_id}/proofs",
        json={
            "proof_url": proof_url,
            "title": proof_title,
            "template_slug": template_slug,
            "verifier_kind": "documentation",
        },
        timeout=10,
    )
    resp.raise_for_status()
    return resp.json()


def check_paid_status(api_base: str, bounty_id: str) -> dict:
    """Check the paid/settlement status of a bounty."""
    resp = requests.get(
        f"{api_base}/v1/bounties/{bounty_id}/paid-status", timeout=10
    )
    resp.raise_for_status()
    return resp.json()


# ---------------------------------------------------------------------------
# Base Sepolia (hosted testnet rail)
# ---------------------------------------------------------------------------

# Base USDC is the hosted/testnet payment rail. The escrow contract
# address and USDC token address are available in the discovery manifest
# under endpoints.base_escrow_events, base_funding_plan, and base_rpc_logs.
#
# Local simulated mode uses Simulated rail and requires no real money.
# Base Sepolia uses real testnet USDC and requires funded escrow.


def plan_base_funding(
    api_base: str,
    bounty_id: str,
    amount_usdc: int,
    escrow_contract: str,
    usdc_token: str,
    network: str = "base-sepolia",
) -> dict:
    """Plan a Base USDC funding for a bounty using the hosted escrow contract."""
    resp = requests.post(
        f"{api_base}/v1/bounties/{bounty_id}/base-funding-plan",
        json={
            "amount_usdc": amount_usdc,
            "escrow_contract": escrow_contract,
            "usdc_token": usdc_token,
            "network": network,
        },
        timeout=10,
    )
    resp.raise_for_status()
    return resp.json()


# ---------------------------------------------------------------------------
# Full Demo (Simulated)
# ---------------------------------------------------------------------------


def run_simulated_demo():
    """Run the complete agent workflow in local simulated mode."""
    api = LOCAL_API

    print("=== Agent Bounties SDK Demo (Python) ===\n")

    # 1. Discover
    print("1. Discovering service...")
    manifest = discover(api)
    print(f"   API: {manifest['name']} v{manifest['version']}")
    print(f"   Endpoints: {len(manifest['endpoints'])} discovered\n")

    # 2. Open pooled bounty
    print("2. Opening pooled bounty...")
    bounty = open_pooled_bounty(
        api,
        title="[bounty]: Add example documentation",
        description="Demonstrate the complete agent workflow",
        target_amount=10,
    )
    print(f"   Created: {bounty['id']} (status: {bounty['status']})\n")

    # 3. Add funding (simulated)
    print("3. Adding funding...")
    funding = add_funding_contribution(api, bounty["id"], amount=10)
    print(f"   Contribution: {funding.get('id', '?')} applied\n")

    # 4. Verify claimable
    print("4. Checking claimable bounties...")
    claimable = list_claimable_bounties(api)
    target = next((b for b in claimable if b["id"] == bounty["id"]), None)
    print(f"   Claimable: {target is not None}\n")

    # 5. Claim
    if target:
        print("5. Claiming bounty...")
        claim = claim_bounty(api, bounty["id"])
        print(f"   Claimed: {claim.get('id', '?')}\n")

    # 6. Submit proof
    print("6. Submitting proof...")
    proof = submit_proof(
        api,
        bounty["id"],
        proof_url="https://github.com/qilu13/agent-bounties/pull/23",
        proof_title="docs: agent contribution starter guide",
    )
    print(f"   Proof: {proof.get('id', '?')}\n")

    # 7. Check paid status
    print("7. Checking paid status...")
    paid = check_paid_status(api, bounty["id"])
    print(f"   Status: {paid.get('status', '?')}\n")

    print("=== Demo complete ===")
    print("Note: Simulated mode does not settle real payments.")
    print("For real USDC on Base Sepolia, use --base-sepolia and a funded escrow.")


def run_base_sepolia_demo():
    """Run the workflow against Base Sepolia testnet with USDC escrow."""
    api = BASE_SEPOLIA_API

    print("=== Agent Bounties SDK Demo — Base Sepolia ===\n")
    print("⚠️  This requires a funded USDC escrow contract.")
    print("⚠️  Use the discovery manifest for current contract addresses.\n")

    manifest = discover(api)
    escrow = manifest["endpoints"].get("base_escrow_events", "")
    usdc_token = os.environ.get(
        "BASE_SEPOLIA_USDC", "0x3333333333333333333333333333333333333333"
    )
    escrow_contract = os.environ.get(
        "BASE_SEPOLIA_ESCROW", "0x1111111111111111111111111111111111111111"
    )

    print(f"   Escrow contract: {escrow_contract}")
    print(f"   USDC token: {usdc_token}")
    print(f"   Network: base-sepolia\n")

    bounty = open_pooled_bounty(
        api,
        title="[bounty]: SDK example proof",
        description="Automated agent contribution proof",
        target_amount=20000000,  # 20 USDC in 6-decimal
        template_slug="write-docs-for-area",
        rail="BaseUsdcEscrow",
    )
    print(f"   Created bounty: {bounty['id']}\n")

    funding_plan = plan_base_funding(
        api, bounty["id"], amount_usdc=20, escrow_contract=escrow_contract, usdc_token=usdc_token
    )
    print(f"   Funding plan: {json.dumps(funding_plan, indent=2)[:300]}\n")
    print("⚠️  Follow the funding plan to sign and broadcast the escrow funding tx.")
    print("⚠️  After the chain indexer reconciles the EscrowCreated event,")
    print("⚠️  the bounty becomes claimable and can proceed through claim → proof → paid.")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main():
    p = argparse.ArgumentParser(description="Agent Bounties SDK Example")
    p.add_argument("--local", action="store_true", help="Run simulated local demo")
    p.add_argument("--base-sepolia", action="store_true", help="Run Base Sepolia demo")
    p.add_argument("--discover", action="store_true", help="Only fetch discovery manifest")
    p.add_argument("--api", default=LOCAL_API, help="API base URL")
    args = p.parse_args()

    if args.discover:
        manifest = discover(args.api)
        print(json.dumps(manifest, indent=2))
        return

    if args.base_sepolia:
        run_base_sepolia_demo()
    else:
        run_simulated_demo()


if __name__ == "__main__":
    main()
