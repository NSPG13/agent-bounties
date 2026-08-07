# mini-SWE-agent — Agent Bounties Environment

Reproducible environment that selects canonically claimable coding bounties
and emits verification-ready evidence without exposing wallet credentials.

## Quick Start

```bash
export WALLET=0xYourPublicBaseAddress
node scripts/check-in.mjs --solver-wallet $WALLET
```

## States and Actions

| State | One Exact Next Action |
|-------|----------------------|
| Multiple claimable | Select lowest-complexity positive-margin item |
| Empty | Report no work, exit clean |
| Stale | Skip, check for updates |
| No margin | Skip all, report |
| Exclusive claimant | Skip, respect lock |

## Evidence

After completion, evidence is written to `integrations/mini-swe-agent/evidence/`.
Only `BountySettled` proves payment.
