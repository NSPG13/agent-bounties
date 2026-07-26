#!/usr/bin/env python3
"""Verify the Coinbase embedded-wallet adapter, sponsorship, and onboarding boundaries."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SITE = ROOT / "site"
TOOL = ROOT / "tools" / "coinbase-wallet-adapter"


def fail(message: str) -> None:
    raise SystemExit(message)


def require(path: Path, phrases: tuple[str, ...]) -> str:
    if not path.exists():
        fail(f"missing embedded-wallet file: {path.relative_to(ROOT)}")
    text = path.read_text(encoding="utf-8")
    for phrase in phrases:
        if phrase not in text:
            fail(f"{path.relative_to(ROOT)} missing required marker: {phrase}")
    return text


def check_script_order(page: str, names: tuple[str, ...], path: Path) -> None:
    positions = [page.find(f'src="{name}') for name in names]
    if any(position < 0 for position in positions):
        fail(f"{path.relative_to(ROOT)} is missing wallet scripts: {names}")
    if positions != sorted(positions):
        fail(f"{path.relative_to(ROOT)} loads wallet scripts in an unsafe order: {names}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--require-built-bundle", action="store_true")
    args = parser.parse_args()

    runtime = require(
        SITE / "wallet-runtime-config.js",
        (
            'schemaVersion: "agent-bounties/wallet-runtime-v1"',
            "chainId: 8453",
            'chainIdHex: "0x2105"',
            "gasSponsored: true",
            "enabled: false",
            'projectId: ""',
            'accountType: "eoa"',
            'gasSponsorshipMode: "eip7702-cdp-paymaster"',
            "disableAnalytics: true",
        ),
    )
    if "pk_" in runtime or "sk_" in runtime or "apiKey" in runtime:
        fail("wallet runtime configuration must not contain provider credentials")

    registry = require(
        SITE / "wallet-adapters.js",
        (
            "agent-bounties/wallet-adapter-registry-v1",
            "eip6963:announceProvider",
            "eip6963:requestProvider",
            "agentbounties:wallet-adapter-announced",
        ),
    )
    if "coinbase" in registry.lower():
        fail("the generic wallet registry must not hard-code Coinbase")

    adapter = require(
        TOOL / "index.js",
        (
            "createCDPEmbeddedWallet",
            "signInWithEmail",
            "signInWithSms",
            "signInWithOAuth",
            "verifyEmailOTP",
            "verifySmsOTP",
            "createEvmEip7702Delegation",
            "waitForEvmEip7702Delegation",
            "sendUserOperation",
            "getUserOperation",
            "enableSpendPermissions: false",
            "useCdpPaymaster: true",
            'gasSponsorshipMode: "eip7702-cdp-paymaster"',
            "agentBountiesGasSponsored",
            "Creating a wallet does not post, fund, claim, or settle a bounty",
        ),
    )
    for forbidden in (
        "privateKey",
        "seedPhrase",
        "recoveryPhrase",
        "localStorage",
        "enableSpendPermissions: true",
        "paymasterUrl:",
    ):
        if forbidden in adapter:
            fail(f"Coinbase adapter contains forbidden custody or authorization marker: {forbidden}")

    sponsorship = require(
        TOOL / "sponsorship.js",
        (
            "transactionToCalls",
            "walletRequestToCalls",
            "waitForUserOperationTransaction",
            'network: "base"',
            "Transaction sender does not match",
            "completed without a valid transaction hash",
        ),
    )
    if "eval(" in sponsorship or "new Function" in sponsorship:
        fail("sponsorship adapter must not execute dynamic code")

    package = json.loads((TOOL / "package.json").read_text(encoding="utf-8"))
    if package.get("dependencies", {}).get("@coinbase/cdp-core") != "0.0.118":
        fail("@coinbase/cdp-core must remain exactly pinned")
    if package.get("dependencies", {}).get("viem") != "2.55.8":
        fail("viem must remain exactly pinned")
    if package.get("devDependencies", {}).get("esbuild") != "0.28.1":
        fail("esbuild must remain exactly pinned")

    pages = {
        name: require(SITE / name, ("wallet-runtime-config.js", "wallet-adapters.js", "coinbase-embedded-wallet.bundle.js"))
        for name in ("earn.html", "objective.html", "onramp.html")
    }
    for name, page in pages.items():
        check_script_order(
            page,
            ("wallet-runtime-config.js", "wallet-adapters.js", "coinbase-embedded-wallet.bundle.js"),
            SITE / name,
        )
        if "wallet-adapters.css" not in page:
            fail(f"{name} does not load the embedded-wallet dialog styles")

    objective = pages["objective.html"]
    for marker in (
        "Create or access your wallet",
        "data-wallet-required",
        "Buy Base USDC with MoonPay",
        "objective-onramp-link.js",
    ):
        if marker not in objective and marker != "Create or access your wallet":
            fail(f"objective.html missing onboarding marker: {marker}")
    require(
        SITE / "objective-onramp-link.js",
        ("data-wallet-required", "amount", "return", "intent", "MutationObserver"),
    )

    onramp = pages["onramp.html"]
    for marker in (
        'data-onramp-asset type="hidden" value="usdc"',
        "Buying USDC does not fund, claim, or post a bounty",
        "Not created yet",
        "Base USDC",
    ):
        if marker not in onramp and marker != "Not created yet":
            fail(f"onramp.html missing USDC-only marker: {marker}")
    if '<option value="eth">' in onramp or "Buy Base ETH" in onramp:
        fail("the public embedded-wallet onboarding flow must not require an ETH purchase")

    onramp_js = require(
        SITE / "moonpay-onramp.js",
        (
            'asset: "usdc"',
            "protocol_action_completed !== false",
            "canonical_event !== null",
            "bounty_contract: state.bountyContract",
            '"/objective.html"',
            "agentBountiesGasSponsored",
        ),
    )
    if 'method: "eth_getBalance"' in onramp_js:
        fail("the USDC onboarding page must not gate embedded users on an ETH balance")

    moonpay_backend = require(
        ROOT / "crates" / "mcp-server" / "src" / "moonpay.rs",
        (
            "bounty_contract: Option<String>",
            "protocol_action_completed: bool",
            "canonical_event: Option<String>",
            "wallet_onboarding_checkout_does_not_require_an_existing_bounty",
            "do not complete any Agent Bounties action",
        ),
    )
    if "enableSpendPermissions: true" in moonpay_backend:
        fail("MoonPay backend must not grant wallet spend permissions")

    composer = require(
        SITE / "bounty-composer-v2.js",
        (
            "agentBountiesGasSponsored===true",
            "CDP paymaster sponsors gas",
            "Choose the embedded wallet for sponsored gas",
        ),
    )
    if "state.balances={usdc,eth:0n,required,gasSponsored:true}" in composer:
        fail("the composer must not falsely describe every external wallet as gas-sponsored")

    bundle = require(
        SITE / "coinbase-embedded-wallet.bundle.js",
        ("agent-bounties/coinbase-embedded-wallet-v1",),
    )
    if args.require_built_bundle:
        if len(bundle.encode("utf-8")) < 20_000:
            fail("the Coinbase production bundle was not generated")
        for marker in ("eip7702-cdp-paymaster", "useCdpPaymaster", "wallet_sendCalls", "signInWithEmail"):
            if marker not in bundle:
                fail(f"generated Coinbase bundle missing marker: {marker}")
        if "generated during CI and GitHub Pages deployment" in bundle:
            fail("the fail-closed source stub was not replaced by the generated bundle")
    elif "generated during CI and GitHub Pages deployment" not in bundle:
        fail("the repository should keep a small fail-closed bundle stub; deployment builds the vendor code")

    for script in (
        SITE / "wallet-adapters.js",
        SITE / "wallet-runtime-config.js",
        SITE / "objective-onramp-link.js",
        SITE / "moonpay-onramp.js",
        SITE / "moonpay-direct-fallback.js",
        TOOL / "provider.js",
        TOOL / "sponsorship.js",
        TOOL / "index.js",
        TOOL / "test-provider.js",
        TOOL / "test-sponsorship.js",
    ):
        subprocess.run(["node", "--check", str(script)], cwd=ROOT, check=True)

    print("Coinbase embedded-wallet adapter checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
