#!/usr/bin/env python3
"""Verify the bounded MoonPay wallet-top-up integration and its evidence boundary."""

from __future__ import annotations

import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SITE = ROOT / "site"


def fail(message: str) -> None:
    raise SystemExit(message)


def require(path: Path, phrases: list[str]) -> str:
    if not path.exists():
        fail(f"missing MoonPay integration file: {path.relative_to(ROOT)}")
    text = path.read_text(encoding="utf-8")
    for phrase in phrases:
        if phrase not in text:
            fail(f"{path.relative_to(ROOT)} missing required phrase: {phrase}")
    return text


def main() -> int:
    backend = require(
        ROOT / "crates/mcp-server/src/moonpay.rs",
        [
            "agent-bounties/moonpay-onramp-checkout-v1",
            "allowedIpAddress",
            "signature",
            "bounty_funded: false",
            "Only the matching indexed canonical FundingAdded event",
            "MOONPAY_SECRET_KEY",
            "MOONPAY_ALLOWED_ORIGINS",
            "MOONPAY_CLIENT_IP_HEADER",
            "url_signing_matches_moonpay_documentation_vector",
        ],
    )
    main_rs = require(
        ROOT / "crates/mcp-server/src/main.rs",
        [
            "mod moonpay;",
            '"/v1/onramps/moonpay/checkout"',
            "post(moonpay::prepare_checkout)",
        ],
    )
    if main_rs.count('"/v1/onramps/moonpay/checkout"') != 1:
        fail("MoonPay checkout route must be registered exactly once")

    onramp = require(
        SITE / "onramp.html",
        [
            '<meta name="robots" content="noindex">',
            '<meta name="referrer" content="no-referrer">',
            "Buying crypto does not fund the bounty.",
            "Only the matching indexed <code>FundingAdded</code> event",
            "Direct MoonPay fallback",
            "cannot prefill or cryptographically bind your wallet",
            'data-start-moonpay',
            'data-direct-moonpay',
            'data-copy-direct-wallet',
            'rel="noopener noreferrer"',
            'data-return-link',
            'src="moonpay-onramp.js?v=1"',
            'src="moonpay-direct-fallback.js?v=1"',
        ],
    )
    if any(term in onramp.lower() for term in ('name="card', 'name="cvv', 'name="cvc')):
        fail("Agent Bounties must not collect card data on the MoonPay handoff page")

    earn = require(
        SITE / "earn.html",
        [
            "Need Base USDC or gas?",
            'data-moonpay-onramp-link',
            'src="moonpay-link.js?v=1"',
            "Buying crypto does not fund this bounty",
        ],
    )
    if earn.index('src="moonpay-link.js?v=1"') > earn.index('src="autonomous.js"'):
        fail("moonpay-link.js must load before autonomous.js initializes the funding form")

    browser = require(
        SITE / "moonpay-onramp.js",
        [
            "buy.moonpay.com",
            "buy-sandbox.moonpay.com",
            "/v1/onramps/moonpay/checkout",
            "bounty_funded !== false",
            "canonical_funding_event !== null",
            "eth_getBalance",
            "eth_call",
            "wallet_switchEthereumChain",
        ],
    )
    fallback = require(
        SITE / "moonpay-direct-fallback.js",
        [
            "https://www.moonpay.com/buy/usdc",
            "https://www.moonpay.com/buy/eth",
            "USDC on Base (USDC_BASE)",
            "ETH on Base (ETH_BASE)",
            "navigator.clipboard.writeText",
            'setAttribute("aria-disabled"',
            "stop if the final screen shows another network or address",
        ],
    )
    if any(term in fallback for term in ("apiKey=", "walletAddress=", "signature=")):
        fail("The direct MoonPay fallback must not imitate a signed or wallet-prefilled partner URL")
    if 'target="_blank"' not in onramp or 'rel="noopener noreferrer"' not in onramp:
        fail("The direct MoonPay fallback must open with noopener and noreferrer")

    require(SITE / "moonpay-link.js", ["onramp.html", "bountyContract", "amount", "intent"])
    require(SITE / "onramp.css", [".onramp-page", ".onramp-action", "@media"])
    require(
        ROOT / "docs/moonpay-onramp.md",
        [
            "MOONPAY_PUBLISHABLE_KEY",
            "MOONPAY_SECRET_KEY",
            "MOONPAY_ENVIRONMENT",
            "FundingAdded",
            "Base ETH",
            "MoonPay sandbox",
            "Direct consumer fallback",
            "www.moonpay.com/buy/usdc",
        ],
    )
    render = require(
        ROOT / "render.yaml",
        [
            "MOONPAY_PUBLISHABLE_KEY",
            "MOONPAY_SECRET_KEY",
            "MOONPAY_ENVIRONMENT",
            "MOONPAY_ALLOWED_ORIGINS",
            "MOONPAY_CLIENT_IP_HEADER",
        ],
    )
    if "sync: false" not in render:
        fail("Render must keep MoonPay credentials out of the repository")

    for path in SITE.rglob("*"):
        if path.is_file() and path.suffix in {".html", ".js", ".json", ".css"}:
            text = path.read_text(encoding="utf-8")
            if "MOONPAY_SECRET_KEY" in text or "sk_live_" in text or "sk_test_" in text:
                fail(f"MoonPay secret material must not appear in browser assets: {path.relative_to(ROOT)}")

    chatgpt_app = (ROOT / "crates/mcp-server/src/chatgpt_app.rs").read_text(encoding="utf-8")
    if "moonpay" in chatgpt_app.lower():
        fail("MoonPay must remain a first-party web handoff, not expand the public ChatGPT tool surface")

    if "localStorage" in browser or "localStorage" in fallback:
        fail("The MoonPay browser handoff must not persist checkout URLs or provider credentials")

    for script in (
        SITE / "moonpay-onramp.js",
        SITE / "moonpay-direct-fallback.js",
        SITE / "moonpay-link.js",
        ROOT / "scripts/test-moonpay-direct-fallback.js",
    ):
        subprocess.run(["node", "--check", str(script)], cwd=ROOT, check=True)
    subprocess.run(["node", "scripts/test-moonpay-direct-fallback.js"], cwd=ROOT, check=True)

    # The official MoonPay documentation vector is intentionally committed as a Rust unit test.
    if "oIJxSghyzll/BLhUFdQZhkxf7DAS8REFaWr/ibO+K8Q=" not in backend:
        fail("MoonPay URL signing must retain the official documentation test vector")

    print("MoonPay on-ramp integration checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
