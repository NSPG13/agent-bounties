# Invoiced Business Path (Seller of Record)

Business buyers often cannot hold crypto and need a vendor invoice. On this
path:
- AgentBounties, the US entity, sells the verified outcome and invoices it
  through Stripe.
- AgentBounties funds an autonomous-v2 bounty from its own treasury.
- Only attested contractors can claim that bounty.

The design and its rejected alternatives are in
[ADR 0006](adr/0006-protocol-v2-platform-fee-and-non-custodial-fiat.md).

This page describes the deterministic core: quote math, Stripe request plans,
webhook evidence, the order state machine, contractor eligibility, registry
attestations and 1099 totals. Hosted endpoints, storage and reconciliation
build on it in a follow-up slice. Live activation waits for:
- the US entity's Stripe account;
- counsel's review of the business terms, the contractor agreement and the
  sales-tax position;
- a tax-form provider.

## Order flow

| Step | Evidence that advances it | Code |
| --- | --- | --- |
| Quote | Operator input, checked against policy | `invoicing::quote_invoice_order` |
| Invoice issued | Stripe finalized invoice whose total equals the quote | `payments_stripe::invoicing` planners |
| Paid | Signature-verified `invoice.paid`: status `paid`, `amount_paid` equal to the quote, nothing remaining, matching mode and order | `parse_invoice_webhook`, `OrderEvent::InvoicePaid` |
| Funding planned | Operator plans `AgentBountyFactoryV2.createBounty` from the treasury for exactly the quoted target | `plan_v2_creation` |
| Funded | Canonical `FundingAdded` from the planned contract: the treasury contributes exactly the target | `OrderEvent::FundingObserved` |
| Settled | Canonical `BountySettled` | `OrderEvent::SettlementObserved` |
| Cancelled → refunded | Canonical treasury `RefundWithdrawn` (when funded), then a signature-verified `credit_note.created` for the full paid amount | `OrderEvent::RefundObserved`, `OrderEvent::CreditNoteIssued` |

`invoicing::project_order` replays the append-only log.

**Fail-closed rules.** The whole projection fails on any of these:
- an out-of-order event;
- a duplicate Stripe event or log key;
- a wrong amount, currency, mode, invoice, order, treasury or contract.

**Other rules.**
- An unpaid invoice is voided, not cancelled.
- A settlement that lands before a cancellation takes effect wins.
- Each state names its `next_action`.

## Money

- **Escrow target.** The treasury escrows the exact v2 target:
  `solver + verifier + ceil(solver × fee bps / 10,000)`.
- **Invoice lines.** All amounts are in whole cents:
  - the outcome: solver plus verifier reward;
  - the platform fee, rounded up;
  - an optional processing pass-through.
- **Coverage invariant.** The invoice always covers the escrow, and it never
  over-collects a full cent for escrowed amounts. A property test checks this
  over random rewards and fee rates.
- **Fee rate.** It must equal the v2 factory's immutable fee, or the
  projection rejects the order.
- **Payment methods.** Invoices accept only US bank transfers
  (`payment_settings.payment_method_types = [customer_balance]`). These
  payments cannot be charged back, so `invoice.paid` is final. Card and ACH
  debit stay disabled until dispute reconciliation exists.

## Contractors

`invoicing::contractor_attestation` admits a wallet only when three things hold:
- the contractor accepted the current contractor agreement (an exact SHA-256
  match);
- a tax-form provider holds a W-9 with a matched TIN, or a W-8BEN or
  W-8BEN-E;
- the validity is at most 365 days.

This keeps automatic on-chain payouts away from US payees without a TIN.

**Registry identity.** The contractor's registry `participantId` is
`keccak256("agent-bounties/invoice-contractor-v1:<contractor_id>")`. Its
`sourceHash` is `keccak256("agent-bounties/invoice-contractor-v1")`, the
source every invoiced bounty commits as its claim gate.

**Separate registry.** The contractor gate uses a dedicated
`ParticipantEligibilityRegistry`. A wallet's source is immutable, so this
registry must not be the GitHub participant registry.

**Signing and registration.** `chain_base::plan_participant_attestation`
returns the EIP-712 payload for the registry's attester. A unit test checks the
digest byte for byte against a deployed registry's `attestationDigest`.
`plan_participant_registration` recovers the signer, refuses anything the
configured attester did not sign, and plans the relayed `register` call. A
signed attestation is not registration evidence; only `ParticipantAttested`
is.

## 1099 reporting

`invoicing::summarize_contractor_payments` totals each contractor's reportable
amount per calendar year (UTC). The reportable amount is the solver reward plus
the completion bonus; the returned claim bond is the solver's own money.

**Thresholds** (P.L. 119-21):
- $600 through 2025;
- $2,000 for 2026 and 2027;
- an explicit confirmed amount after 2027, once the threshold is indexed.

**Foreign payees.** W-8 payees are listed separately for the tax provider's
1042-S review. A settlement recorded twice fails the summary.

## Stripe request plans

Every plan uses a deterministic idempotency key per order, and IDs are
validated before they are placed in a URL path:
- `POST /v1/customers`
- `POST /v1/invoices`: `send_invoice`, bank transfer only, a seller-of-record
  footer, and metadata `{order_id, purpose: invoiced_outcome, terms_sha256}`
- `POST /v1/invoiceitems`, one per quote line
- `POST /v1/invoices/{id}/finalize`, `/send` and `/void`
- `POST /v1/credit_notes`: the full amount, refunded once, either through
  Stripe (`refund_amount`) or recorded outside it (`out_of_band_amount`)

**Webhooks.** `parse_invoice_webhook` accepts native Stripe events after
signature verification. It ignores objects not tagged
`purpose: invoiced_outcome`. It rejects malformed events, mode mismatches and
unsupported event types.

## Never evidence

None of these changes an order's money state:
- a quote;
- a hosted invoice link;
- a redirect;
- a payment intent;
- a transaction plan;
- a signature;
- a transaction hash.
