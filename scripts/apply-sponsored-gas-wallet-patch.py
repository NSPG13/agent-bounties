#!/usr/bin/env python3
"""Apply embedded-wallet gas sponsorship to the static bounty composer.

The composer is intentionally a single browser bundle. This script performs narrow,
counted replacements so automation can update it without reformatting unrelated UX
code. It is idempotent and fails closed when the expected source has drifted.
"""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PATH = ROOT / "site" / "bounty-composer-v2.js"

OLD_EMPTY = 'if(!providers.length){ui.walletMessage.textContent="No compatible browser wallet was detected. Install or open a Base-compatible wallet, then try again. Never enter a recovery phrase on this website.";return;}'
NEW_EMPTY = 'if(!providers.length){ui.walletMessage.textContent="The embedded wallet is not active and no compatible existing wallet was detected. Configure Coinbase CDP or open a Base-compatible wallet, then try again. Never enter a recovery phrase on this website.";return;}'

ORIGINAL_READINESS = 'async function refreshWalletReadiness(){if(!state.provider||!state.account)return;const protocol=await loadProtocol();const required=usdcBaseUnits(state.fundingUsdc);setPaymentStatus("Checking Base USDC and gas readiness…","pending");const[usdcRaw,ethRaw]=await Promise.all([state.provider.request({method:"eth_call",params:[{to:protocol.native_usdc,data:`0x70a08231${addressWord(state.account)}`} ,"latest"]}),state.provider.request({method:"eth_getBalance",params:[state.account,"latest"]})]);const usdc=BigInt(usdcRaw||"0x0");const eth=BigInt(ethRaw||"0x0");state.balances={usdc,eth,required};const usdcReady=usdc>=required;const ethReady=eth>0n;ui.account.textContent=`${state.account.slice(0,8)}…${state.account.slice(-6)}`;ui.usdcBalance.textContent=`${formatUsdc(Number(usdc)/1_000_000)} USDC`;ui.ethBalance.textContent=`${(Number(eth)/1e18).toFixed(6)} ETH`;ui.requiredUsdc.textContent=`${formatUsdc(state.fundingUsdc)} USDC`;ui.readiness.hidden=false;ui.fundingHelp.hidden=usdcReady&&ethReady;const missing=required>usdc?required-usdc:0n;ui.missingUsdc.textContent=`${formatUsdc(Number(missing)/1_000_000)} USDC`;ui.fundNow.disabled=!(usdcReady&&ethReady);if(usdcReady&&ethReady)setPaymentStatus("Wallet ready. Review the exact amount, legal terms, and wallet request before signing.","success");else if(!usdcReady&&!ethReady)setPaymentStatus("This wallet needs more Base USDC and a small amount of Base ETH for gas.","error");else if(!usdcReady)setPaymentStatus("This wallet does not yet hold enough USDC on Base.","error");else setPaymentStatus("The USDC is available, but the wallet needs a small amount of Base ETH for gas.","error");}'
ORIGINAL_READINESS = ORIGINAL_READINESS.replace(']} ,"latest"]', ']},"latest"]')
UNCONDITIONAL_READINESS = 'async function refreshWalletReadiness(){if(!state.provider||!state.account)return;const protocol=await loadProtocol();const required=usdcBaseUnits(state.fundingUsdc);setPaymentStatus("Checking Base USDC readiness. Agent Bounties sponsors the transaction gas…","pending");const usdcRaw=await state.provider.request({method:"eth_call",params:[{to:protocol.native_usdc,data:`0x70a08231${addressWord(state.account)}`} ,"latest"]});const usdc=BigInt(usdcRaw||"0x0");state.balances={usdc,eth:0n,required,gasSponsored:true};const usdcReady=usdc>=required;ui.account.textContent=`${state.account.slice(0,8)}…${state.account.slice(-6)}`;ui.usdcBalance.textContent=`${formatUsdc(Number(usdc)/1_000_000)} USDC`;ui.ethBalance.textContent="Sponsored";ui.requiredUsdc.textContent=`${formatUsdc(state.fundingUsdc)} USDC`;ui.readiness.hidden=false;ui.fundingHelp.hidden=usdcReady;const missing=required>usdc?required-usdc:0n;ui.missingUsdc.textContent=`${formatUsdc(Number(missing)/1_000_000)} USDC`;ui.fundNow.disabled=!usdcReady;if(usdcReady)setPaymentStatus("Wallet ready. Agent Bounties sponsors gas; review the exact USDC amount, legal terms, and wallet request before signing.","success");else setPaymentStatus("This wallet does not yet hold enough USDC on Base. No ETH purchase is required for the supported Agent Bounties action path.","error");}'
UNCONDITIONAL_READINESS = UNCONDITIONAL_READINESS.replace(']} ,"latest"]', ']},"latest"]')
CONDITIONAL_READINESS = 'async function refreshWalletReadiness(){if(!state.provider||!state.account)return;const protocol=await loadProtocol();const required=usdcBaseUnits(state.fundingUsdc);const gasSponsored=state.provider?.agentBountiesGasSponsored===true;setPaymentStatus(gasSponsored?"Checking Base USDC readiness. The embedded-wallet paymaster sponsors gas…":"Checking Base USDC and wallet gas readiness…","pending");const usdcRequest=state.provider.request({method:"eth_call",params:[{to:protocol.native_usdc,data:`0x70a08231${addressWord(state.account)}`} ,"latest"]});const[usdcRaw,ethRaw]=gasSponsored?[await usdcRequest,"0x0"]:await Promise.all([usdcRequest,state.provider.request({method:"eth_getBalance",params:[state.account,"latest"]})]);const usdc=BigInt(usdcRaw||"0x0");const eth=BigInt(ethRaw||"0x0");state.balances={usdc,eth,required,gasSponsored};const usdcReady=usdc>=required;const gasReady=gasSponsored||eth>0n;ui.account.textContent=`${state.account.slice(0,8)}…${state.account.slice(-6)}`;ui.usdcBalance.textContent=`${formatUsdc(Number(usdc)/1_000_000)} USDC`;ui.ethBalance.textContent=gasSponsored?"Sponsored":`${(Number(eth)/1e18).toFixed(6)} ETH`;ui.requiredUsdc.textContent=`${formatUsdc(state.fundingUsdc)} USDC`;ui.readiness.hidden=false;ui.fundingHelp.hidden=usdcReady&&gasReady;const missing=required>usdc?required-usdc:0n;ui.missingUsdc.textContent=`${formatUsdc(Number(missing)/1_000_000)} USDC`;ui.fundNow.disabled=!(usdcReady&&gasReady);if(usdcReady&&gasReady)setPaymentStatus(gasSponsored?"Embedded wallet ready. The CDP paymaster sponsors gas; review the exact USDC amount, legal terms, and wallet request before signing.":"Wallet ready. This existing wallet will use its normal Base gas path; review the exact amount before signing.","success");else if(!usdcReady&&!gasReady)setPaymentStatus("This existing wallet needs more Base USDC and enough Base ETH for its own gas. The embedded wallet option removes the ETH requirement.","error");else if(!usdcReady)setPaymentStatus(gasSponsored?"This embedded wallet needs more USDC on Base. No ETH purchase is required.":"This wallet does not yet hold enough USDC on Base.","error");else setPaymentStatus("The USDC is available, but this existing wallet needs enough Base ETH for its own transaction gas.","error");}'
CONDITIONAL_READINESS = CONDITIONAL_READINESS.replace(']} ,"latest"]', ']},"latest"]')

ORIGINAL_GATE = 'if(state.balances.usdc<state.balances.required||state.balances.eth===0n)throw new Error("The wallet is not ready to fund this bounty.");'
UNCONDITIONAL_GATE = 'if(state.balances.usdc<state.balances.required)throw new Error("The wallet does not yet hold enough Base USDC to fund this bounty.");'
CONDITIONAL_GATE = 'if(state.balances.usdc<state.balances.required)throw new Error("The wallet does not yet hold enough Base USDC to fund this bounty.");if(!state.balances.gasSponsored&&state.balances.eth===0n)throw new Error("This existing wallet needs enough Base ETH for its own gas. Choose the embedded wallet for sponsored gas.");'


def replace_one_of(source: str, variants: tuple[str, ...], new: str, label: str) -> str:
    if source.count(new) == 1 and sum(source.count(value) for value in variants) == 0:
        return source
    matches = [(value, source.count(value)) for value in variants if source.count(value)]
    if len(matches) == 1 and matches[0][1] == 1 and source.count(new) == 0:
        return source.replace(matches[0][0], new, 1)
    raise SystemExit(
        f"{label} replacement contract failed: variants={[count for _, count in matches]}, "
        f"new={source.count(new)}. Inspect composer drift before changing wallet behavior."
    )


def main() -> int:
    source = PATH.read_text(encoding="utf-8")
    updated = replace_one_of(source, (OLD_EMPTY,), NEW_EMPTY, "empty-wallet")
    updated = replace_one_of(
        updated,
        (ORIGINAL_READINESS, UNCONDITIONAL_READINESS),
        CONDITIONAL_READINESS,
        "readiness",
    )
    updated = replace_one_of(
        updated,
        (ORIGINAL_GATE, UNCONDITIONAL_GATE),
        CONDITIONAL_GATE,
        "funding-gate",
    )

    for required in (
        "agentBountiesGasSponsored===true",
        'gasSponsored?"Sponsored"',
        "CDP paymaster sponsors gas",
        "Choose the embedded wallet for sponsored gas",
    ):
        if required not in updated:
            raise SystemExit(f"conditional sponsored-gas composer missing required marker: {required}")

    PATH.write_text(updated, encoding="utf-8")
    print("Conditional embedded-wallet gas sponsorship patch is applied")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
