#!/usr/bin/env python3
"""One-time cleanup that converts the embedded-wallet draft into checked-in production source.

This script is run once by a self-deleting branch workflow. It removes the draft's
source-materialization machinery, tightens user-facing security language, and makes
Pages consume the exact lockfile instead of modifying the pull-request branch.
"""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_once(path: str, old: str, new: str, label: str) -> None:
    file_path = ROOT / path
    source = file_path.read_text(encoding="utf-8")
    if new in source and old not in source:
        return
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one old block in {path}, found {count}")
    file_path.write_text(source.replace(old, new, 1), encoding="utf-8")


def replace_all(path: str, old: str, new: str, expected: int, label: str) -> None:
    file_path = ROOT / path
    source = file_path.read_text(encoding="utf-8")
    count = source.count(old)
    if count == 0 and source.count(new) >= expected:
        return
    if count != expected:
        raise SystemExit(f"{label}: expected {expected} old blocks in {path}, found {count}")
    file_path.write_text(source.replace(old, new), encoding="utf-8")


def update_adapter_copy() -> None:
    path = "tools/coinbase-wallet-adapter/index.js"
    replace_once(
        path,
        'message: "Reviewing the exact Base calls. Agent Bounties is sponsoring gas, not the USDC amount.",',
        'message: "Reviewing the exact Base calls. The configured CDP paymaster sponsors this direct transaction. Gas sponsorship never authorizes Agent Bounties to move your USDC.",',
        "direct sponsorship attribution",
    )
    replace_once(
        path,
        '<p class="embedded-wallet-fine">SMS availability depends on Coinbase\'s supported countries and carriers. Email remains available when SMS is not.</p>',
        '<p class="embedded-wallet-fine">SMS availability depends on Coinbase\'s supported countries and carriers. SMS is also more exposed to SIM-swap attacks; prefer email or social login for meaningful balances.</p>',
        "SMS risk disclosure",
    )
    replace_once(
        path,
        '<p class="embedded-wallet-fine">Coinbase handles authentication and returns you to this exact Agent Bounties page. Your bounty action remains a separate approval.</p>',
        '<p class="embedded-wallet-fine">Coinbase handles authentication and returns you to this exact Agent Bounties page. Your bounty action remains a separate approval. Use the same sign-in method each time unless you later link methods; unlinked methods can create separate Coinbase identities and wallets.</p>',
        "identity-linking disclosure",
    )
    replace_once(
        path,
        '<p class="embedded-wallet-fine">Coinbase supplies the wallet infrastructure. Agent Bounties never receives a private key, seed phrase, email code, or SMS code. Creating a wallet does not post, fund, claim, or settle a bounty.</p>',
        '<p class="embedded-wallet-fine">Coinbase supplies the wallet infrastructure. Agent Bounties does not receive wallet keys or seed phrases and does not send authentication codes to its servers; this browser passes the code to Coinbase for verification. Creating a wallet does not post, fund, claim, or settle a bounty.</p>',
        "authentication-data disclosure",
    )


def update_public_copy() -> None:
    replace_once(
        "site/earn.html",
        "MoonPay can add Base USDC to your connected or embedded wallet. The embedded-wallet adapter sponsors gas; existing wallets keep their normal Base gas path. Buying USDC does not fund this bounty; return here and separately approve the exact canonical contribution.",
        "MoonPay can add Base USDC to your connected or embedded wallet. No ETH purchase is required for the embedded-wallet path; direct embedded-wallet calls use the configured CDP paymaster, while existing wallets keep their configured sponsor or normal Base gas path. Buying USDC does not fund this bounty; return here and separately approve the exact canonical contribution.",
        "funding gas copy",
    )
    replace_once(
        "site/earn.html",
        "Start work only after the claim is confirmed. The embedded-wallet adapter uses sponsored gas; existing wallets keep their configured Base gas path.",
        "Start work only after the claim is confirmed. No ETH purchase is required for the embedded-wallet path; existing wallets keep their configured sponsor or normal Base gas path.",
        "claim gas copy",
    )
    replace_once(
        "site/objective.html",
        "The embedded-wallet adapter sponsors gas; existing wallets keep their normal Base gas path.",
        "No ETH purchase is required for the embedded-wallet path; existing wallets keep their configured sponsor or normal Base gas path.",
        "composer gas copy",
    )
    replace_all(
        "site/objective.html",
        "Sponsored for embedded wallet",
        "Sponsored on embedded path",
        1,
        "composer gas label",
    )
    replace_once(
        "site/onramp.html",
        "The embedded-wallet adapter sponsors gas for supported Agent Bounties transactions. Existing wallets retain their normal Base gas behavior. Only the matching indexed canonical event changes protocol state.",
        "No ETH purchase is required for the embedded-wallet path. Direct embedded-wallet calls use the configured CDP paymaster; existing wallets retain their configured sponsor or normal Base gas path. Only the matching indexed canonical event changes protocol state.",
        "on-ramp gas boundary",
    )
    replace_once(
        "site/onramp.html",
        "The embedded-wallet adapter sponsors gas for its supported action path.",
        "No ETH purchase is required for the embedded-wallet path.",
        "on-ramp gas guidance",
    )
    replace_all(
        "site/onramp.html",
        "Sponsored for embedded wallet",
        "Sponsored on embedded path",
        1,
        "on-ramp gas label",
    )
    replace_once(
        "docs/moonpay-onramp.md",
        "The embedded-wallet adapter sponsors the supported Base transaction through its paymaster path. The MoonPay onboarding interface therefore does not ask an embedded-wallet user to buy ETH. Existing external wallets retain their own configured gas behavior.",
        "No ETH purchase is required for the embedded-wallet path. Direct embedded-wallet calls use the configured CDP paymaster, hosted Agent Bounties routes retain their existing gas sponsorship, and external wallets retain their configured sponsor or normal Base gas behavior.",
        "MoonPay gas documentation",
    )


def update_checker() -> None:
    path = "scripts/check-coinbase-embedded-wallet.py"
    replace_once(
        path,
        '''            "agentBountiesGasSponsored",
            "Creating a wallet does not post, fund, claim, or settle a bounty",
''',
        '''            "agentBountiesGasSponsored",
            "Gas sponsorship never authorizes Agent Bounties to move your USDC",
            "does not send authentication codes to its servers",
            "unlinked methods can create separate Coinbase identities and wallets",
            "SIM-swap",
            "Creating a wallet does not post, fund, claim, or settle a bounty",
''',
        "adapter disclosure checks",
    )
    replace_once(
        path,
        '''    if "enableSpendPermissions: true" in moonpay_backend:
        fail("MoonPay backend must not grant wallet spend permissions")
''',
        '''    if moonpay_backend.count("fn wallet_onboarding_checkout_does_not_require_an_existing_bounty()") != 1:
        fail("the pre-bounty MoonPay onboarding test must exist exactly once")
    if "enableSpendPermissions: true" in moonpay_backend:
        fail("MoonPay backend must not grant wallet spend permissions")
''',
        "single onboarding test check",
    )


def update_pages_workflow() -> None:
    path = ROOT / ".github/workflows/pages.yml"
    source = path.read_text(encoding="utf-8")

    for line in (
        '      - "scripts/apply-sponsored-gas-wallet-patch.py"\n',
        '      - "scripts/apply-generic-moonpay-wallet-patch.py"\n',
        '      - "scripts/apply-embedded-wallet-ui-patch.py"\n',
    ):
        source = source.replace(line, "")

    embedded_trigger = '      - "scripts/check-coinbase-embedded-wallet.py"\n'
    bundle_trigger = '      - "scripts/check-coinbase-browser-bundle.py"\n'
    if bundle_trigger not in source:
        source = source.replace(embedded_trigger, embedded_trigger + bundle_trigger)

    materialize = '''      - name: Materialize reviewed wallet source boundaries
        run: |
          python scripts/apply-sponsored-gas-wallet-patch.py
          python scripts/apply-generic-moonpay-wallet-patch.py
          python scripts/apply-embedded-wallet-ui-patch.py

'''
    count = source.count(materialize)
    if count not in (0, 2):
        raise SystemExit(f"Pages materialization blocks drifted: found {count}")
    source = source.replace(materialize, "")

    source = source.replace(
        '          node-version: "22"\n',
        '          node-version: "22"\n          cache: npm\n          cache-dependency-path: tools/coinbase-wallet-adapter/package-lock.json\n',
    )
    source = source.replace(
        "npm install --ignore-scripts --no-audit --no-fund",
        "npm ci --ignore-scripts --no-audit --no-fund",
    )

    validate_marker = '''          python scripts/check-coinbase-embedded-wallet.py --require-built-bundle
          python scripts/check-moonpay-onramp.py
'''
    validate_replacement = '''          python scripts/check-coinbase-embedded-wallet.py --require-built-bundle
          python scripts/check-coinbase-browser-bundle.py
          python scripts/check-moonpay-onramp.py
'''
    if validate_replacement not in source:
        if source.count(validate_marker) != 1:
            raise SystemExit("Pages built-wallet verification block drifted")
        source = source.replace(validate_marker, validate_replacement, 1)

    start = source.index("      - name: Verify the staged production wallet surface\n")
    end = source.index("      - uses: actions/upload-pages-artifact@v5\n", start)
    staged = '''      - name: Verify the staged production wallet surface
        run: |
          python scripts/check-coinbase-browser-bundle.py
          node --check site/wallet-runtime-config.js
          node --check site/coinbase-embedded-wallet.bundle.js
          grep -q 'agent-bounties/coinbase-embedded-wallet-v1' site/coinbase-embedded-wallet.bundle.js
          grep -q 'useCdpPaymaster' site/coinbase-embedded-wallet.bundle.js
          grep -q 'enableSpendPermissions' site/coinbase-embedded-wallet.bundle.js
          grep -q 'wallet-runtime-config.js?v=1' site/objective.html
          grep -q 'coinbase-embedded-wallet.bundle.js?v=1' site/objective.html

'''
    source = source[:start] + staged + source[end:]

    for forbidden in (
        "apply-sponsored-gas-wallet-patch.py",
        "apply-generic-moonpay-wallet-patch.py",
        "apply-embedded-wallet-ui-patch.py",
        "npm install --ignore-scripts",
        "Materialize reviewed wallet source boundaries",
    ):
        if forbidden in source:
            raise SystemExit(f"Pages workflow still contains temporary marker: {forbidden}")
    if source.count("scripts/check-coinbase-browser-bundle.py") < 3:
        raise SystemExit("Pages workflow does not verify the vanilla browser bundle in all required stages")
    if source.count("cache-dependency-path: tools/coinbase-wallet-adapter/package-lock.json") != 2:
        raise SystemExit("Pages workflow must cache the exact wallet lockfile in validate and deploy jobs")

    path.write_text(source, encoding="utf-8")


def main() -> int:
    update_adapter_copy()
    update_public_copy()
    update_checker()
    update_pages_workflow()
    print("Coinbase embedded-wallet draft cleanup applied")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
