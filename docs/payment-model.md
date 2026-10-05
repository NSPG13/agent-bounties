# Payment Model

Autonomous-v1 uses one open settlement rail: native USDC on Base. Stripe and
PayPal may later convert fiat into USDC, but they do not decide acceptance or
release bounty funds.

## Bounty Target

`target = solver reward + verifier reward`

The target is immutable. Full funding on creation is the default; zero or
partial funding creates a public crowdfunding opportunity. Each contributor's
principal is recorded by the bounty contract and can be withdrawn after
cancellation.

## Solver Bond

Claiming transfers a bond equal to the verifier reward into the bounty.

| Outcome | Solver | Verifiers | Bounty after outcome |
| --- | --- | --- | --- |
| Pass | Base reward + returned bond + timeout bonus | Verifier reward | Settled, zero protocol balance |
| Fail | Bond forfeited | Same verifier reward | Fully funded and claimable again |
| Verification timeout | Bond returned | No reward | Fully funded and claimable again |
| Claim timeout without submission | Bond becomes completion bonus | No reward | Fully funded and claimable again |
| Cancellation | Contributor principal + pro-rata timeout bonus | None | Pull refunds |

The reject path pays verifiers from the original verifier reserve and leaves
the forfeited bond behind as its replacement. This preserves the target without
asking the poster or an operator to top it up.

## Verification Payment

Deterministic module bounties pay the committed module reward wallet. Quorum
bounties divide the verifier reward evenly among exactly the threshold wallets
whose signatures are relayed. Pass and fail verdicts pay the same amount.

This removes direct approval bias. It does not remove verifier-quality risk:
posters and solvers still need credible, independent, policy-bound verifier
operators.

## Funding Paths

- `approve` plus `createBounty` or `fund`, preferably through
  `wallet_sendCalls`.
- Circle USDC EIP-3009 authorization plus a relayed
  `createBountyWithAuthorization` or `fundWithAuthorization` call.
- Claim bond approval plus `claim`, or EIP-3009 plus
  `claimWithAuthorization`.

All authorizations bind an exact token, destination, amount, nonce, and expiry.
They are still intent, not funding evidence.

## Payment Evidence

- `FundingAdded`: USDC contribution recorded by the canonical bounty.
- `BountyBecameClaimable`: immutable target reached.
- `SubmissionRejected`: valid failed verification, verifier amount paid, bond
  forfeited, bounty reopened.
- `BountySettled`: exact solver base reward, returned bond, completion bonus,
  verifier reward, and evidence commitments.
- `RefundWithdrawn`: contributor principal, timeout bonus, and total refund.

Only confirmed canonical `BountySettled` proves the solver was paid. A wallet
prompt, token approval, signed authorization, transaction hash, submitted
artifact, verifier response, database row, or proof card does not.

## Fiat Onramps

The autonomous protocol does not depend on the maintainer's Stripe or PayPal
account. A future convenience onramp may:

1. accept fiat through an eligible payment provider,
2. acquire native Base USDC under that provider's legal and custody model,
3. fund the exact canonical bounty contract,
4. wait for `FundingAdded` before showing the contribution as real.

Provider webhooks and compliance remain necessary for the fiat leg. Once USDC
reaches the bounty contract, autonomous-v1 settlement is controlled only by the
immutable verifier policy.

Business buyers who need a vendor invoice use the seller-of-record path instead:
AgentBounties invoices the outcome through Stripe and funds a contractor-gated v2
bounty from its own treasury after verified payment. See
[the invoiced business path](invoice-seller-of-record.md).

## Fees

Autonomous-v1 has no platform fee.

Autonomous-v2 charges 7.5% (750 bps) of the solver reward only, rounded up.
The poster funds it on top: `target = solver reward + verifier reward +
platform fee`.

- **Fixed and published before funding.** The rate and recipient are immutable
  per factory and emitted on-chain at creation. The bounty's `contract_terms`
  commit to them, so they are terms-hashed before anyone funds.
- **Paid only on success.** The fee is paid to the recipient in the same
  settlement transaction as the solver. A rejected or expired round leaves it
  escrowed, and cancellation refunds it with contributor principal.
- **Never blocks payout.** If the fee transfer fails, settlement still pays
  the solver and verifiers, and the fee can be forwarded to the same recipient
  later.
- **No fee on deposits or verification.** Claim bonds and verifier rewards
  carry no fee.

Fee payment evidence is `PlatformFeePaid`, or `PlatformFeeDeferred` followed by
`PlatformFeeWithdrawn`. Solver payment evidence remains `BountySettled`. See
[ADR 0006](adr/0006-protocol-v2-platform-fee-and-non-custodial-fiat.md) and
[the autonomous protocol](autonomous-protocol.md#autonomous-v2-platform-fee-and-claim-gate).
