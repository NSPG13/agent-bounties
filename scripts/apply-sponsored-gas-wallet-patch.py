#!/usr/bin/env python3
"""Scope gas sponsorship to the embedded-wallet adapter in the static composer.

The composer is a single browser bundle. This script replaces only the wallet
readiness function and the final funding gate, using stable function boundaries
rather than formatting-sensitive full-file strings. It is idempotent and fails
closed when the surrounding source contract changes.
"""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PATH = ROOT / "site" / "bounty-composer-v2.js"

OLD_EMPTY = 'if(!providers.length){ui.walletMessage.textContent="No compatible browser wallet was detected. Install or open a Base-compatible wallet, then try again. Never enter a recovery phrase on this website.";return;}'
NEW_EMPTY = 'if(!providers.length){ui.walletMessage.textContent="The embedded wallet is not active and no compatible existing wallet was detected. Configure Coinbase CDP or open a Base-compatible wallet, then try again. Never enter a recovery phrase on this website.";return;}'

CONDITIONAL_READINESS = '''async function refreshWalletReadiness(){if(!state.provider||!state.account)return;const protocol=await loadProtocol();const required=usdcBaseUnits(state.fundingUsdc);const gasSponsored=state.provider?.agentBountiesGasSponsored===true;setPaymentStatus(gasSponsored?"Checking Base USDC readiness. The embedded-wallet paymaster sponsors gas…":"Checking Base USDC and wallet gas readiness…","pending");const usdcRequest=state.provider.request({method:"eth_call",params:[{to:protocol.native_usdc,data:`0x70a08231${addressWord(state.account)}`},"latest"]});const[usdcRaw,ethRaw]=gasSponsored?[await usdcRequest,"0x0"]:await Promise.all([usdcRequest,state.provider.request({method:"eth_getBalance",params:[state.account,"latest"]})]);const usdc=BigInt(usdcRaw||"0x0");const eth=BigInt(ethRaw||"0x0");state.balances={usdc,eth,required,gasSponsored};const usdcReady=usdc>=required;const gasReady=gasSponsored||eth>0n;ui.account.textContent=`${state.account.slice(0,8)}…${state.account.slice(-6)}`;ui.usdcBalance.textContent=`${formatUsdc(Number(usdc)/1_000_000)} USDC`;ui.ethBalance.textContent=gasSponsored?"Sponsored":`${(Number(eth)/1e18).toFixed(6)} ETH`;ui.requiredUsdc.textContent=`${formatUsdc(state.fundingUsdc)} USDC`;ui.readiness.hidden=false;ui.fundingHelp.hidden=usdcReady&&gasReady;const missing=required>usdc?required-usdc:0n;ui.missingUsdc.textContent=`${formatUsdc(Number(missing)/1_000_000)} USDC`;ui.fundNow.disabled=!(usdcReady&&gasReady);if(usdcReady&&gasReady)setPaymentStatus(gasSponsored?"Embedded wallet ready. The CDP paymaster sponsors gas; review the exact USDC amount, legal terms, and wallet request before signing.":"Wallet ready. This existing wallet will use its normal Base gas path; review the exact amount before signing.","success");else if(!usdcReady&&!gasReady)setPaymentStatus("This existing wallet needs more Base USDC and enough Base ETH for its own gas. The embedded wallet option removes the ETH requirement.","error");else if(!usdcReady)setPaymentStatus(gasSponsored?"This embedded wallet needs more USDC on Base. No ETH purchase is required.":"This wallet does not yet hold enough USDC on Base.","error");else setPaymentStatus("The USDC is available, but this existing wallet needs enough Base ETH for its own transaction gas.","error");}'''

USDC_GATE = 'if(state.balances.usdc<state.balances.required)throw new Error("The wallet does not yet hold enough Base USDC to fund this bounty.");'
ORIGINAL_GATE = 'if(state.balances.usdc<state.balances.required||state.balances.eth===0n)throw new Error("The wallet is not ready to fund this bounty.");'
CONDITIONAL_GATE = USDC_GATE + 'if(!state.balances.gasSponsored&&state.balances.eth===0n)throw new Error("This existing wallet needs enough Base ETH for its own gas. Choose the embedded wallet for sponsored gas.");'


def patch_readiness(source: str) -> str:
    start_marker = "async function refreshWalletReadiness()"
    end_marker = "\n\n  async function watchUsdcAsset"
    if source.count(start_marker) != 1 or source.count(end_marker) != 1:
        raise SystemExit("wallet readiness function boundaries changed; inspect composer source")
    start = source.index(start_marker)
    end = source.index(end_marker, start)
    current = source[start:end]
    if current == CONDITIONAL_READINESS:
        return source
    recognized = (
        "Checking Base USDC and gas readiness" in current
        or "state.balances={usdc,eth:0n,required,gasSponsored:true}" in current
        or "Agent Bounties sponsors the transaction gas" in current
    )
    if not recognized:
        raise SystemExit("wallet readiness function is not a recognized pre-adapter variant")
    return source[:start] + CONDITIONAL_READINESS + source[end:]


def patch_gate(source: str) -> str:
    if source.count(CONDITIONAL_GATE) == 1:
        return source
    matches = [(candidate, source.count(candidate)) for candidate in (ORIGINAL_GATE, USDC_GATE)]
    matches = [(candidate, count) for candidate, count in matches if count]
    if len(matches) != 1 or matches[0][1] != 1:
        raise SystemExit(f"funding gate changed unexpectedly: {[(count) for _, count in matches]}")
    return source.replace(matches[0][0], CONDITIONAL_GATE, 1)


def main() -> int:
    source = PATH.read_text(encoding="utf-8")
    if OLD_EMPTY in source:
        if source.count(OLD_EMPTY) != 1:
            raise SystemExit("legacy empty-wallet copy appears more than once")
        source = source.replace(OLD_EMPTY, NEW_EMPTY, 1)
    elif source.count(NEW_EMPTY) != 1:
        raise SystemExit("empty-wallet discovery copy changed unexpectedly")

    source = patch_readiness(source)
    source = patch_gate(source)

    for required in (
        "agentBountiesGasSponsored===true",
        'gasSponsored?"Sponsored"',
        "CDP paymaster sponsors gas",
        "Choose the embedded wallet for sponsored gas",
    ):
        if required not in source:
            raise SystemExit(f"conditional sponsored-gas composer missing required marker: {required}")
    if "state.balances={usdc,eth:0n,required,gasSponsored:true}" in source:
        raise SystemExit("composer still marks every wallet as sponsored")

    PATH.write_text(source, encoding="utf-8")
    print("Conditional embedded-wallet gas sponsorship patch is applied")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
