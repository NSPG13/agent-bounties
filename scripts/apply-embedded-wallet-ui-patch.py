#!/usr/bin/env python3
"""Apply small, counted embedded-wallet copy and loader updates to static pages."""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

REPLACEMENTS = {
    ROOT / "site" / "objective.html": (
        (
            '<div><span>Transaction gas</span><strong data-wallet-eth>Sponsored</strong></div>',
            '<div><span>Transaction gas</span><strong data-wallet-eth>Sponsored for embedded wallet</strong></div>',
        ),
        (
            '<p>This wallet needs <strong data-missing-usdc>—</strong>. Agent Bounties sponsors the transaction gas.</p>',
            '<p>This wallet needs <strong data-missing-usdc>—</strong>. The embedded-wallet adapter sponsors gas; existing wallets keep their normal Base gas path.</p>',
        ),
        (
            '<a class="button secondary" href="onramp.html?return=objective.html%23post">Buy Base USDC with MoonPay</a>',
            '<a class="button secondary" href="onramp.html?return=objective.html%23post" data-objective-onramp-link>Buy Base USDC with MoonPay</a>',
        ),
        (
            '<script src="bounty-composer-v2.js?v=3"></script>\n    <script src="bounty-chat-ui.js?v=4"></script>',
            '<script src="bounty-composer-v2.js?v=3"></script>\n    <script src="objective-onramp-link.js?v=1"></script>\n    <script src="bounty-chat-ui.js?v=4"></script>',
        ),
    ),
    ROOT / "site" / "earn.html": (
        (
            'MoonPay can add Base USDC to your connected or embedded wallet. Agent Bounties sponsors the transaction gas. Buying USDC does not fund this bounty; return here and separately approve the exact canonical contribution.',
            'MoonPay can add Base USDC to your connected or embedded wallet. The embedded-wallet adapter sponsors gas; existing wallets keep their normal Base gas path. Buying USDC does not fund this bounty; return here and separately approve the exact canonical contribution.',
        ),
        (
            '<p class="fine">Start work only after the claim is confirmed. Agent Bounties sponsors the transaction gas.</p>',
            '<p class="fine">Start work only after the claim is confirmed. The embedded-wallet adapter uses sponsored gas; existing wallets keep their configured Base gas path.</p>',
        ),
    ),
    ROOT / "site" / "onramp.html": (
        (
            '<p>Agent Bounties sponsors the transaction gas. Only the matching indexed canonical event changes protocol state.</p>',
            '<p>The embedded-wallet adapter sponsors gas for supported Agent Bounties transactions. Existing wallets retain their normal Base gas behavior. Only the matching indexed canonical event changes protocol state.</p>',
        ),
        (
            '<div><dt>Transaction gas</dt><dd><strong>Sponsored by Agent Bounties</strong></dd></div>',
            '<div><dt>Transaction gas</dt><dd><strong>Sponsored for embedded wallet</strong></dd></div>',
        ),
        (
            'Buy USDC into your wallet, then return to approve the original action. You do not need to buy ETH for Agent Bounties gas.',
            'Buy USDC into your wallet, then return to approve the original action. The embedded-wallet path does not require an ETH purchase.',
        ),
        (
            'Only Base USDC is needed for the bounty amount. Agent Bounties sponsors the transaction gas for the supported action path.',
            'Only Base USDC is needed for the bounty amount. The embedded-wallet adapter sponsors gas for its supported action path.',
        ),
    ),
}


def replace_once(source: str, old: str, new: str, path: Path) -> str:
    old_count = source.count(old)
    new_count = source.count(new)
    if old_count == 1 and new_count == 0:
        return source.replace(old, new, 1)
    if old_count == 0 and new_count == 1:
        return source
    raise SystemExit(
        f"{path.relative_to(ROOT)} replacement mismatch: old={old_count}, new={new_count}: {old[:80]}"
    )


def main() -> int:
    for path, pairs in REPLACEMENTS.items():
        source = path.read_text(encoding="utf-8")
        for old, new in pairs:
            source = replace_once(source, old, new, path)
        path.write_text(source, encoding="utf-8")
    print("Embedded-wallet static page patch is applied")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
