#!/usr/bin/env python3
"""Static and behavioral checks for the vendor-neutral Coinbase wallet adapter."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SITE = ROOT / "site"
TOOL = ROOT / "tools/coinbase-embedded-wallet"


def fail(message: str) -> None:
    raise SystemExit(message)


def require(path: Path, markers: tuple[str, ...]) -> str:
    if not path.exists():
        fail(f"missing Coinbase wallet integration file: {path.relative_to(ROOT)}")
    text = path.read_text(encoding="utf-8")
    missing = [marker for marker in markers if marker not in text]
    if missing:
        fail(f"{path.relative_to(ROOT)} is missing required markers: {missing}")
    return text


def assert_order(text: str, first: str, second: str, path: Path) -> None:
    if text.index(first) >= text.index(second):
        fail(f"{path.relative_to(ROOT)} must load {first} before {second}")


def main() -> int:
    registry = require(
        SITE / "wallet-adapter-registry.js",
        (
            "AgentBountiesWalletAdapters",
            "eip6963:announceProvider",
            "eip6963:requestProvider",
            "capabilitiesFor",
            "Wallet adapter provider must implement EIP-1193",
        ),
    )
    if "coinbase" in registry.lower():
        fail("the wallet adapter registry must remain vendor-neutral")

    config = require(
        SITE / "wallet-config.js",
        (
            "agent-bounties/wallet-providers-v1",
            'accountType: "eoa"',
            'disableAnalytics: true',
            'secureIframeBasePath: "https://secure-wallet.cdp.coinbase.com"',
            'transactionPolicy: "agent-bounties-relay-required"',
            '"email"',
            '"sms"',
            '"oauth:google"',
            '"oauth:apple"',
            '"oauth:x"',
            '"oauth:telegram"',
        ),
    )
    placeholder_count = config.count("__COINBASE_CDP_PROJECT_ID__")
    if placeholder_count not in (0, 1):
        fail("wallet-config.js must be either the one-placeholder source template or a configured build")
    if placeholder_count == 0 and 'projectId = "__COINBASE_' in config:
        fail("wallet-config.js contains a malformed deployment placeholder")

    source = require(
        TOOL / "src/index.js",
        (
            'createOnLogin: "eoa"',
            "createCDPEmbeddedWallet",
            "CDPReactProvider",
            "AuthButton",
            "LinkAuth",
            "LinkAuthError",
            "LinkAuthFlow",
            "LinkAuthFlowBackButton",
            "LinkAuthTitle",
            "useCurrentUser",
            "useIsSignedIn",
            "eip3009: true",
            "arbitraryTransactionsGasSponsored: false",
            "directTransactions: false",
            "UNSPONSORED_TRANSACTION_METHODS",
            "SDK_READY_TIMEOUT_MS",
            "secureIframeBasePath",
            "maintained sign-in interface",
            "Use the same sign-in method",
            "Link another sign-in method",
            "Coinbase is verifying",
            "linkedAuthMethods",
            "manageAccess",
            "authMethodLinking: true",
            "Linking does not merge two existing wallet identities",
        ),
    )
    forbidden_source = (
        "CDP_API_KEY_SECRET",
        "CDP_WALLET_SECRET",
        "exportEvmAccount",
        "privateKey",
        "seedPhrase",
        "localStorage",
        "window.opener",
        "signInWithEmail",
        "verifyEmailOTP",
        "signInWithSms",
        "verifySmsOTP",
        "signInWithOAuth",
    )
    for term in forbidden_source:
        if term in source:
            fail(f"Coinbase browser adapter contains forbidden term: {term}")

    package_path = TOOL / "package.json"
    lock_path = TOOL / "package-lock.json"
    package = json.loads(package_path.read_text(encoding="utf-8"))
    if not lock_path.exists():
        fail("tools/coinbase-embedded-wallet/package-lock.json must be committed")
    lock = json.loads(lock_path.read_text(encoding="utf-8"))
    if lock.get("lockfileVersion") != 3:
        fail("Coinbase wallet package lock must use npm lockfileVersion 3")
    root_lock = lock.get("packages", {}).get("", {})
    if root_lock.get("dependencies") != package.get("dependencies"):
        fail("package-lock root dependencies differ from package.json")
    if root_lock.get("devDependencies") != package.get("devDependencies"):
        fail("package-lock root devDependencies differ from package.json")
    dependencies = package.get("dependencies", {})
    for dependency in ("@coinbase/cdp-core", "@coinbase/cdp-hooks", "@coinbase/cdp-react"):
        if dependencies.get(dependency) != "0.0.118":
            fail(f"{dependency} must be pinned exactly")
    if dependencies.get("react") != "19.2.7" or dependencies.get("react-dom") != "19.2.7":
        fail("React and React DOM must be pinned to the same reviewed version")
    if dependencies.get("viem") != "2.55.8":
        fail("viem must be pinned exactly")
    if package.get("devDependencies", {}).get("esbuild") != "0.28.1":
        fail("esbuild must be pinned exactly")

    x402 = require(
        SITE / "x402-browser.js",
        (
            "agent-bounty-fund",
            "eth_signTypedData_v4",
            "TransferWithAuthorization",
            "payment-required",
            "payment-signature",
            "payment-response",
            "FundingAdded",
            "pollRelay",
            "x-agent-bounties-legal-acceptance",
            "AgentBountiesX402",
        ),
    )
    if "eth_sendTransaction" in x402 or "wallet_sendCalls" in x402:
        fail("browser x402 funding must not ask the user wallet to broadcast or pay gas")

    for relative, consumer in (
        ("earn.html", "autonomous.js"),
        ("onramp.html", "moonpay-onramp.js"),
    ):
        path = SITE / relative
        page = require(
            path,
            (
                "wallet-adapters.css",
                "wallet-config.js",
                "wallet-adapter-registry.js",
                "coinbase-embedded-wallet.bundle.js",
                "coinbase-embedded-wallet.bundle.css",
            ),
        )
        assert_order(page, "wallet-config.js", "wallet-adapter-registry.js", path)
        assert_order(page, "wallet-adapter-registry.js", "coinbase-embedded-wallet.bundle.js", path)
        assert_order(page, "coinbase-embedded-wallet.bundle.js", consumer, path)
    funding_alias = require(
        SITE / "funding.html",
        (
            'data-destination="earn.html#fund"',
            'src="route-alias.js"',
        ),
    )
    route_alias = require(
        SITE / "route-alias.js",
        (
            "if (window.location.search) target.search = window.location.search",
            "window.location.replace(target.href)",
        ),
    )
    if "target.search = window.location.search" not in route_alias:
        fail("ChatGPT funding handoffs must preserve their intent query through funding.html")

    onramp = require(
        SITE / "onramp.html",
        (
            "frame-src https://secure-wallet.cdp.coinbase.com",
            "connect-src 'self' https://mcp.agentbounties.app https://api.cdp.coinbase.com https://mainnet.base.org",
            "Buy only Base USDC for these supported actions; no ETH purchase is required.",
        ),
    )
    if '<option value="eth">' in onramp:
        fail("the sponsored onboarding flow must not ask users to purchase ETH")

    earn = require(
        SITE / "earn.html",
        (
            "x402-browser.js",
            "Gas is sponsored for this funding action",
            'data-wallet-requires="direct-transactions"',
        ),
    )
    assert_order(earn, "x402-browser.js", "autonomous.js", SITE / "earn.html")

    autonomous = require(
        SITE / "autonomous.js",
        (
            "AgentBountiesX402",
            "Gas-sponsored authorization required",
            "x402Result.transactionHash",
            "capabilitiesFor",
            "directTransactions",
            "No compatible transaction wallet available",
        ),
    )
    fund_start = autonomous.index("async function fundBounty")
    fund_end = autonomous.index("async function submitBounty", fund_start)
    fund_body = autonomous[fund_start:fund_end]
    if "authorized-contribution-plan" in fund_body or "sendTransaction(authorized.relay_transaction" in fund_body:
        fail("EOA funding still broadcasts the relayer transaction from the user's wallet")
    if 'required !== "direct-transactions"' not in autonomous or 'capabilities?.directTransactions !== false' not in autonomous:
        fail("wallet selectors do not hide relay-only embedded wallets from direct-transaction actions")


    workflow = require(
        ROOT / ".github/workflows/pages.yml",
        (
            "COINBASE_CDP_PROJECT_ID",
            "tools/coinbase-embedded-wallet",
            "npm ci",
            "npm rebuild",
            "npm run build",
            "configure-wallet-providers.py",
            "check-coinbase-embedded-wallet.py",
        ),
    )
    if "secrets.COINBASE_CDP_PROJECT_ID" in workflow:
        fail("the public Coinbase project ID should use a GitHub variable, not a secret")
    if "vars.COINBASE_CDP_PROJECT_ID" not in workflow:
        fail("Pages deployment must inject vars.COINBASE_CDP_PROJECT_ID")
    if "configure-wallet-providers.py --allow-disabled" in workflow:
        fail("production Pages deployment must fail closed when the Coinbase project ID is missing")
    if "npm install --prefix tools/coinbase-embedded-wallet" in workflow:
        fail("Pages must use the committed dependency lock through npm ci")
    if "! grep -q '__COINBASE_CDP_PROJECT_ID__' /tmp/live-wallet-config.js" not in workflow:
        fail("the live canary must reject an unconfigured Coinbase wallet deployment")

    require(
        ROOT / "docs/coinbase-embedded-wallet.md",
        (
            "EIP-1193",
            "EIP-6963",
            "createOnLogin: \"eoa\"",
            "COINBASE_CDP_PROJECT_ID",
            "Auth method linking",
            "LinkAuth",
            "ACCOUNT_EXISTS",
            "METHOD_ALREADY_LINKED",
            "FundingAdded",
            "gas-only relayer",
            "arbitraryTransactionsGasSponsored: false",
            "directTransactions: false",
        ),
    )

    for script in (
        SITE / "wallet-adapter-registry.js",
        SITE / "wallet-config.js",
        SITE / "x402-browser.js",
        TOOL / "src/index.js",
        TOOL / "build.mjs",
        ROOT / "scripts/test-wallet-adapter-registry.js",
        ROOT / "scripts/test-x402-browser.js",
    ):
        subprocess.run(["node", "--check", str(script)], cwd=ROOT, check=True)
    subprocess.run(["node", "scripts/test-wallet-adapter-registry.js"], cwd=ROOT, check=True)
    subprocess.run(["node", "scripts/test-x402-browser.js"], cwd=ROOT, check=True)

    print("Coinbase embedded-wallet adapter and gas-sponsored x402 funding checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
