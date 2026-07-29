# Direct Recovery 689

Issue [#689](https://github.com/NSPG13/agent-bounties/issues/689) is the
public accounting record for five Base-mainnet direct bounties that became
unusable because their committed verifier path was not executable.

This recovery does not bypass the bounties' acceptance criteria. It implements
the five tasks, runs the repository gate and each task's published check,
submits the exact resulting commit as evidence, and requires two independent
signatures from the immutable verifier set before settlement.

## Exact Scope

The machine-readable allowlist is
`ops/recovery/direct-mainnet-689.json`. The recovery script rejects any
contract, issue, chain, token, creator, verifier set, terms hash, policy hash,
acceptance hash, benchmark hash, evidence schema hash, or amount outside that
manifest.

The five contracts hold 10 USDC in total. The operator solver needs 0.05 USDC
for five refundable 0.01 USDC claim bonds. Successful settlement returns
exactly 10 USDC to the disclosed owner wallet and pays 0.025 USDC to each
immutable verifier wallet.

The external claim on issue #639 and every standing-meta contract are outside
this recovery. They must not be claimed, signed, settled, or refunded through
this workflow.

## Evidence Boundary

Recovery settlements are operational accounting, not organic marketplace
activity. They are excluded from:

- funded-loop and external-solver counts
- adoption, retention, and conversion metrics
- revenue and marketplace-volume metrics
- solver reputation and leaderboards

Only canonical `BountySettled` events prove settlement. A passing command,
pull-request merge, workflow run, transaction broadcast, or verifier signature
is not payment evidence.

## Procedure

1. Merge the reviewed implementation PR into `main`.
2. Transfer exactly 0.05 USDC on Base mainnet to the disclosed operator solver.
3. Dispatch `Direct recovery 689` from the exact current `main` revision with
   confirmation `RECOVER ISSUE 689 EXACTLY 10 USDC`.
4. The workflow comments on #689 before any value-bearing transaction.
5. The prepare job reruns all checks, claims, and submits the five contracts.
6. Two isolated signer jobs rerun the checks and sign exact EIP-712
   attestations.
7. The relay job reruns the checks, verifies both signer artifacts, settles the
   exact contracts, returns exactly 10 USDC, and posts transaction evidence.
8. Reconcile the five canonical events and confirm recovery-excluded metrics.

Use the read-only audit at any time:

```bash
python scripts/direct_recovery_689.py \
  --rpc-url https://mainnet.base.org \
  audit \
  --output target/direct-recovery-audit.json
```
