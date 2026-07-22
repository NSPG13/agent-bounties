# Standing Meta Low-Bond Migration

Tracking issue: [#527](https://github.com/NSPG13/agent-bounties/issues/527)

## Approved economics

Each replacement parent remains fully funded with 1 native Base USDC:

| Component | Old | Replacement |
| --- | ---: | ---: |
| Solver reward | 0.90 USDC | **0.99 USDC** |
| Verifier reward | 0.10 USDC | **0.01 USDC** |
| Refundable claim bond | 0.10 USDC | **0.01 USDC** |
| Initial funding | 1.00 USDC | **1.00 USDC** |

The claim bond must equal the verifier reward under `agent-bounties/autonomous-v1`.

## Contracts being replaced

- #333 CLI: `0xfffecb0fcd36477c5f6ecec808f6f0cf53819562`
- #334 API: `0xbe17ef2d154265ebe3142d7bda5e99610d571455`
- #335 MCP: `0x43d42cb227d76588ab16693f14efd6cff851fa7a`
- #336 wallet UX: `0xe8c1d3f046f3e4690bef59ba4abd5d02d2a6984b`

The old terms under `bounties/autonomous-v1/333.json` through `336.json` are immutable historical evidence and must not be edited.

## Prepare exact replacement terms

```bash
python scripts/prepare_low_bond_standing_meta_migration.py
python scripts/test_prepare_low_bond_standing_meta_migration.py -v
```

The generator fails closed if the old economics, creator wallet, source issue, or standing-meta engine changed. It writes four replacement terms and `manifest.json` under `target/standing-meta-low-bond-migration/`.

Generated files are plans only. They do not publish terms or move funds.

## Execution order

1. Re-read all four canonical contracts at one current Base `safe` block. Stop if any contract is no longer claimable, has a solver, has a submission, or differs from the immutable old terms.
2. Reserve the old contracts in hosted discovery during migration:

   ```text
   0xfffecb0fcd36477c5f6ecec808f6f0cf53819562,0xbe17ef2d154265ebe3142d7bda5e99610d571455,0x43d42cb227d76588ab16693f14efd6cff851fa7a,0xe8c1d3f046f3e4690bef59ba4abd5d02d2a6984b
   ```

   Append these addresses to `BASE_RECOVERY_RESERVED_BOUNTY_CONTRACTS`; do not remove existing reservations. Confirm the hosted earning feed excludes them or marks `verification_ready=false` with an explicit migration reason.
3. Publish each generated terms document through the hosted terms store and verify the returned hash against the generated manifest.
4. Build fresh creation plans against the current canonical factory and one exact safe block.
5. Use the active bounded wallet `0x1eaa1c68772cf76bc5f4e4174766076e33ace662` to create and fully fund each replacement. Its existing reviewed policy permits creation and funding for the standing-meta-v2 module, subject to live caps and expiry.
6. Require confirmed `CanonicalBountyCreated`, `CanonicalBountyTermsCommitted`, `CanonicalBountyEconomicsConfigured`, `CanonicalBountyVerificationConfigured`, `FundingAdded`, and `BountyBecameClaimable` events before updating #333–#336.
7. Independently verify each replacement reports 0.99 USDC solver reward, 0.01 USDC verifier reward, 0.01 USDC bond, valid terms, and `verification_ready=true`.
8. Update the source issues and labels only from reconciled canonical state.

## Old-fund recovery

The old contracts were created by the bounded wallet. Its current delegate policy permits create, fund, claim, and submit, but not cancel or refund withdrawal. Do not broaden that authority casually.

Recover the four old deposits only through either:

- an owner-reviewed replacement policy that adds exact canonical `cancel` and `withdrawRefund` actions with narrow targets and caps; or
- a separately reviewed one-shot recovery adapter bound to these four contracts and the owner destination.

Only confirmed `BountyCancelled` and `RefundWithdrawn` events prove recovery. Until then, the old funds remain locked in their canonical contracts.

## Evidence boundary

A source file, generated terms hash, API response, pull request, wallet plan, signature request, or transaction hash is not replacement funding. Canonical events prove lifecycle transitions; only `BountySettled` proves solver payment.
