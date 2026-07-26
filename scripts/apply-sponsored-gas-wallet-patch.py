#!/usr/bin/env python3
"""Apply the sponsored-gas source update to the large static bounty composer.

The composer is intentionally a single browser bundle. This script performs narrow,
counted replacements so GitHub Actions can update it without reformatting unrelated
UX code. It is idempotent and fails closed if the expected source has drifted.
"""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PATH = ROOT / "site" / "bounty-composer-v2.js"

OLD_EMPTY = 'if(!providers.length){ui.walletMessage.textContent="No compatible browser wallet was detected. Install or open a Base-compatible wallet, then try again. Never enter a recovery phrase on this website.";return;}'
NEW_EMPTY = 'if(!providers.length){ui.walletMessage.textContent="The embedded wallet is not active and no compatible existing wallet was detected. Configure Coinbase CDP or open a Base-compatible wallet, then try again. Never enter a recovery phrase on this website.";return;}'

OLD_READINESS = 'async function refreshWalletReadiness(){if(!state.provider||!state.account)return;const protocol=await loadProtocol();const required=usdcBaseUnits(state.fundingUsdc);setPaymentStatus("Checking Base USDC and gas readiness…","pending");const[usdcRaw,ethRaw]=await Promise.all([state.provider.request({method:"eth_call",params:[{to:protocol.native_usdc,data:`0x70a08231${addressWord(state.account)}`},"latest"]}),state.provider.request({method:"eth_getBalance",params:[state.account,"latest"]})]);const usdc=BigInt(usdcRaw||"0x0");const eth=BigInt(ethRaw||"0x0");state.balances={usdc,eth,required};const usdcReady=usdc>=required;const ethReady=eth>0n;ui.account.textContent=`${state.account.slice(0,8)}…${state.account.slice(-6)}`;ui.usdcBalance.textContent=`${formatUsdc(Number(usdc)/1_000_000)} USDC`;ui.ethBalance.textContent=`${(Number(eth)/1e18).toFixed(6)} ETH`;ui.requiredUsdc.textContent=`${formatUsdc(state.fundingUsdc)} USDC`;ui.readiness.hidden=false;ui.fundingHelp.hidden=usdcReady&&ethReady;const missing=required>usdc?required-usdc:0n;ui.missingUsdc.textContent=`${formatUsdc(Number(missing)/1_000_000)} USDC`;ui.fundNow.disabled=!(usdcReady&&ethReady);if(usdcReady&&ethReady)setPaymentStatus("Wallet ready. Review the exact amount, legal terms, and wallet request before signing.","success");else if(!usdcReady&&!ethReady)setPaymentStatus("This wallet needs more Base USDC and a small amount of Base ETH for gas.","error");else if(!usdcReady)setPaymentStatus("This wallet does not yet hold enough USDC on Base.","error");else setPaymentStatus("The USDC is available, but the wallet needs a small amount of Base ETH for gas.","error");}'
NEW_READINESS = 'async function refreshWalletReadiness(){if(!state.provider||!state.account)return;const protocol=await loadProtocol();const required=usdcBaseUnits(state.fundingUsdc);setPaymentStatus("Checking Base USDC readiness. Agent Bounties sponsors the transaction gas…","pending");const usdcRaw=await state.provider.request({method:"eth_call",params:[{to:protocol.native_usdc,data:`0x70a08231${addressWord(state.account)}`},"latest"]});const usdc=BigInt(usdcRaw||"0x0");state.balances={usdc,eth:0n,required,gasSponsored:true};const usdcReady=usdc>=required;ui.account.textContent=`${state.account.slice(0,8)}…${state.account.slice(-6)}`;ui.usdcBalance.textContent=`${formatUsdc(Number(usdc)/1_000_000)} USDC`;ui.ethBalance.textContent="Sponsored";ui.requiredUsdc.textContent=`${formatUsdc(state.fundingUsdc)} USDC`;ui.readiness.hidden=false;ui.fundingHelp.hidden=usdcReady;const missing=required>usdc?required-usdc:0n;ui.missingUsdc.textContent=`${formatUsdc(Number(missing)/1_000_000)} USDC`;ui.fundNow.disabled=!usdcReady;if(usdcReady)setPaymentStatus("Wallet ready. Agent Bounties sponsors gas; review the exact USDC amount, legal terms, and wallet request before signing.","success");else setPaymentStatus("This wallet does not yet hold enough USDC on Base. No ETH purchase is required for the supported Agent Bounties action path.","error");}'

OLD_GATE = 'if(state.balances.usdc<state.balances.required||state.balances.eth===0n)throw new Error("The wallet is not ready to fund this bounty.");'
NEW_GATE = 'if(state.balances.usdc<state.balances.required)throw new Error("The wallet does not yet hold enough Base USDC to fund this bounty.");'


def replace_exact(source: str, old: str, new: str, label: str) -> str:
    old_count = source.count(old)
    new_count = source.count(new)
    if old_count == 1 and new_count == 0:
        return source.replace(old, new, 1)
    if old_count == 0 and new_count == 1:
        return source
    raise SystemExit(
        f"{label} replacement contract failed: old={old_count}, new={new_count}. "
        "Inspect composer drift before changing wallet behavior."
    )


def main() -> int:
    source = PATH.read_text(encoding="utf-8")
    updated = replace_exact(source, OLD_EMPTY, NEW_EMPTY, "empty-wallet")
    updated = replace_exact(updated, OLD_READINESS, NEW_READINESS, "readiness")
    updated = replace_exact(updated, OLD_GATE, NEW_GATE, "funding-gate")

    for forbidden in (
        "Checking Base USDC and gas readiness",
        "needs more Base USDC and a small amount of Base ETH",
        "wallet needs a small amount of Base ETH for gas",
        "state.balances.eth===0n",
    ):
        if forbidden in updated:
            raise SystemExit(f"sponsored-gas composer still contains forbidden gate: {forbidden}")

    for required in (
        "gasSponsored:true",
        'ui.ethBalance.textContent="Sponsored"',
        "No ETH purchase is required",
        "Agent Bounties sponsors the transaction gas",
    ):
        if required not in updated:
            raise SystemExit(f"sponsored-gas composer missing required marker: {required}")

    PATH.write_text(updated, encoding="utf-8")
    print("Sponsored-gas wallet readiness patch is applied")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
