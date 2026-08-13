# #772 round-4 expiry recovery

This incident facade is pinned to Base mainnet bounty contract
`0x9baa8a4a2ad3096c6ebfb2c994a93afb7a299274`, bounty ID
`0x34e8d16cdbfff635e77ce703cc6efea8fc64a3adb1ee2ef293c604b85bb6a8cb`,
round 4, solver `0xc49e5374f0072abc0b4c134b2fd413d87aa6354a`, verification expiry
`1786586903`, active bond `10000`, and clone runtime hash
`0x6e7d6297e170d10e6484c9b72314bb0e2173cd967aa8e05231ee369dbde0c0a1`.

It has no target, chain, function, value, or calldata arguments. The only call
is zero-value `expireSubmission()` (`0xf9251ec7`). Any tuple or runtime mismatch
stops before execution.

Dry-run only:

```bash
python3 scripts/expire_772_round4.py \
  --output target/expiry-772-round4-dry-run.json
```

The checked-in dry-run evidence is
[`evidence/expiry-772-round4-dry-run-2026-08-13.json`](evidence/expiry-772-round4-dry-run-2026-08-13.json).
A successful simulation is not an expiry, refund, or payment event.

Settlement-desk may perform the separately authorized one-shot execution with:

```bash
python3 scripts/expire_772_round4.py \
  --execute \
  --acknowledge authorize-expire-772-round4-once \
  --output target/expiry-772-round4-receipt.json
```

The wrapper rechecks the tuple immediately before calling the existing bounded
hosted timeout relay. It then requires the exact successful receipt, one
canonical `SubmissionExpired` event for round 4 and the pinned solver, a 10000
atomic bond refund, status `Claimable`, zero active bond, the exact solver USDC
balance delta, and recorded gas usage. It contains and accepts no private key.
The receipt path is create-only to preserve earlier evidence.
