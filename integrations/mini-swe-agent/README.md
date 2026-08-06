# Mini-SWE-Agent Paid-Work Environment Integration

This integration provides a reproducible mini-SWE-agent environment that scans claimable coding bounties, selects positive-margin opportunities, and emits verification-ready evidence packaging.

## Features

- **Inventory Scan**: Direct argv discovery from `discovery_source`.
- **Claim Planning**: Only claims canonical opportunities respecting exclusive claimants.
- **Evidence Packaging**: Emits verification evidence including `source_snapshot_digest`.
- **Settlement Verification**: Monitors `BountySettled` events on Base mainnet for USDC payment.

## Usage

```bash
python3 select_bounty.py --input fixtures/multiple.json
```
