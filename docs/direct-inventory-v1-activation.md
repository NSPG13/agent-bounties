# Direct inventory V1 activation

Issues #869-#873 are five ordinary coding bounties created to restore earning
inventory after the verified ready-to-earn count reached zero. They do not
require a child bounty.

Each task precommits 1.00 USDC for the solver, 0.10 USDC divided between the
two automated `sandboxed_regression_v1` signers, a refundable 0.10 USDC claim
bond, one immutable benchmark, and a seven-day claim window. The five
contracts require exactly 5.50 USDC, leaving 1.29 USDC in the bounded wallet
at the balance observed before activation.

The shared activation harness validates the exact version-5 bounded-wallet
policy, V1 wallet bytecode manifest, signer set, task economics, benchmark
digests, and canonical factory plan. GitHub receives `funded-live` and
`claimable-live` only after `CanonicalBountyCreated`, `FundingAdded`, and
`BountyBecameClaimable` reconcile with valid terms and executable
verification.

Run the non-financial checks with:

```powershell
python -m unittest scripts.test_activate_direct_growth_v2 scripts.test_activate_direct_inventory_v1 -v
python -m py_compile scripts/activate_direct_growth_v2.py
Get-ChildItem benchmarks/direct-inventory-v1/*/check.py | ForEach-Object { python -m py_compile $_.FullName }
```

Activation runs only from merged `main` through
`activate-direct-inventory-v1.yml`. A transaction hash is not claimability,
and a verifier result is not payment. Only a confirmed canonical
`BountySettled` event proves solver payment.
