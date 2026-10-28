# Invoiced Business Path (Seller of Record)

Business buyers often cannot hold crypto and need a vendor invoice. On this
path:
- AgentBounties, the US entity, sells the verified outcome and invoices it
  through Stripe.
- AgentBounties funds an autonomous-v2 bounty from its own treasury.
- Only attested contractors can claim that bounty.

The design and its rejected alternatives are in
[ADR 0006](adr/0006-protocol-v2-platform-fee-and-non-custodial-fiat.md).

This page describes the deterministic core and the hosted operator
endpoints. The core covers quote math, Stripe request plans, webhook evidence,
the order state machine, contractor eligibility, registry attestations and
1099 totals. Everything can run in Stripe test mode. Live activation waits
for:
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
| Cancelled → refund pending | Canonical treasury `RefundWithdrawn` (when funding was planned), then a signature-verified `credit_note.created` for the full paid amount | `OrderEvent::RefundObserved`, `OrderEvent::CreditNoteIssued` |
| Refund pending → refunded | Authenticated Stripe reads bind the full credit-note allocation to this invoice and order, and every linked refund reports `succeeded` to a US bank-transfer destination | `OrderEvent::BuyerRefundReconciled` |
| Refunded → repayment required again | A fresh read shows a bank return: `requires_action`, `pending`, `failed` or `canceled` | `OrderEvent::BuyerRefundReconciled` |

`invoicing::project_order` replays the append-only log.

**Fail-closed rules.** The whole projection fails on any of these:
- an out-of-order event;
- a duplicate Stripe event or log key;
- a wrong amount, currency, mode, invoice, order, treasury or contract.

**Other rules.**
- An unpaid invoice is voided, not cancelled.
- A settlement that lands before a cancellation takes effect wins.
- **No buyer refund while escrow may exist.** Once treasury funding is
  planned, the creation may already be signed or mined but not yet indexed, so
  a missing `FundingAdded` proves nothing. The buyer's credit note then waits
  for the treasury's canonical `RefundWithdrawn` from the cancelled bounty.
  The operator completes the planned funding if it never ran, then cancels
  and withdraws. Without this rule the treasury could refund the buyer and
  still pay a contractor.
- An invoice marked paid outside Stripe (`paid_out_of_band`) is never payment.
- Each state names its `next_action`.
- An issued credit note records an accounting adjustment, not returned money.
  `refund_due_cents` is the amount eligible for a credit note; the separate
  `refund_outstanding_cents` remains positive until successful bank repayment.
- Refund reads are bound to the order log sequence observed before the Stripe
  requests. A concurrent append invalidates an older observation; retry by
  reading Stripe again. Unchanged snapshots do not grow the log. Genuine bank
  return/retry cycles can record the same status again without being mistaken
  for a duplicate from an earlier cycle.

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
- **Fee rate.** It must equal the v2 factory's immutable fee when the invoice
  is issued and when funding is planned. Replay checks each quote against its
  own fee terms, so a later factory fee change never erases an order's
  history or its 1099 totals.
- **Payment methods.** Invoices accept only US bank transfers
  (`payment_settings.payment_method_types = [customer_balance]`). These
  payments can be reversed when sent over US ACH rails. `invoice.paid`
  establishes invoice payment, not bank cash availability or irreversible
  settlement. This operator path still needs reversal handling and explicit
  settled-cash/conversion evidence before production funding. Card and ACH
  debit remain disabled. See [Stripe bank-transfer disputes](https://docs.stripe.com/payments/bank-transfers#disputes).

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
- `POST /v1/credit_notes`: request the full Stripe refund (`refund_amount`).
  The low-level planner also supports accounting for an external repayment,
  but the hosted refund endpoint rejects `out_of_band` until verified external
  repayment evidence is implemented.
- Stripe's [retrieve a credit note](https://docs.stripe.com/api/credit_notes/retrieve)
  and [retrieve a refund](https://docs.stripe.com/api/refunds/retrieve) GET requests
  read the complete refund allocation and current repayment status.
  Reconciliation never issues another refund.

An interrupted run is resumed by reading the stored invoice back before
anything is added to it.

**Webhooks.** `parse_invoice_webhook` accepts native Stripe events after
signature verification. It ignores objects not tagged
`purpose: invoiced_outcome`, and tagged lifecycle events it does not apply
(such as `invoice.finalized`), so Stripe does not retry them. It rejects
malformed events, connected-account/organization event scopes and mode
mismatches. The native credit-note currency must match the USD order.

## Never evidence

None of these changes an order's money state:
- a quote;
- a hosted invoice link;
- a redirect;
- a payment intent;
- a transaction plan;
- a signature;
- a transaction hash.

## Hosted endpoints

Every route below is operator-only and requires `OPERATOR_API_TOKEN`. An
unset token fails closed here, unlike other operator routes in local
development. Order state is replayed from the append-only
`invoice_order_events` log (migration `0038_invoice_orders.sql`). A new event
is stored only when that replay accepts it.

| Route | Purpose |
| --- | --- |
| `POST /v1/invoicing/orders` | Quote and create an order |
| `GET /v1/invoicing/orders`, `GET /v1/invoicing/orders/{id}` | Order states, events and `next_action` |
| `POST /v1/invoicing/orders/{id}/invoice` | Create the customer, draft and items, finalize, record the invoice, then send. Each Stripe id is stored before the next step. A retry reads a stored invoice back instead of re-adding items, and an already recorded invoice is only resent. The finalized total must equal the quote |
| `POST /v1/invoicing/orders/{id}/void` | Void an unpaid invoice. `invoice.voided` records it |
| `POST /v1/invoicing/orders/{id}/funding-plan` | Publish the v2 terms and return the treasury's unsigned creation plan. The terms commit the quote economics, the treasury as creator, the contractor gate and the verifier quorum |
| `POST /v1/invoicing/orders/{id}/reconcile` | Apply indexed canonical treasury events when funding was planned; when a credit note exists, also read its linked Stripe refunds. Works for cancellation before funding too |
| `POST /v1/invoicing/orders/{id}/cancel` | Cancel a paid order after applying the latest indexed events. When funded and not yet refunded, it returns the treasury's `cancel()` and `withdrawRefund()` calls. Repeating it returns the current calls |
| `POST /v1/invoicing/orders/{id}/refund` | Apply the latest indexed events, then issue the credit note once no escrow is at risk. `credit_note.created` leaves `refund_pending`; reconciliation must confirm repayment |
| `PUT /v1/invoicing/contractors/{id}` | Record a contractor: wallet, agreement acceptance, and a tax-form reference only. A wallet change keeps the old wallet in `previous_wallets`, and a wallet never moves to another contractor |
| `POST /v1/invoicing/contractors/{id}/attestation-plan` | Return the attester's EIP-712 payload for an eligible contractor |
| `POST /v1/invoicing/contractors/{id}/registration-plan` | Plan the relayed `register` call for an attestation that the configured attester signed |
| `GET /v1/invoicing/contractor-payments?year=` | Annual 1099-NEC totals and foreign-payee totals over every order. Settlements to a contractor's earlier wallets count. `unmatched_settlements` and `replay_errors` list what needs review, and `complete` is true only when both are empty |
| `POST /v1/stripe/invoice-webhooks` | Signature-verified Stripe events, using a separate endpoint secret |

**Webhook handling.** Each Stripe event applies at most once; a retry is
reported as a duplicate only when its financial facts match. Reusing an
evidence ID with different facts or a different order returns a conflict.
A transition the projection rejects is acknowledged
with its reason, so Stripe does not retry evidence that can never apply. The
exception is an `invoice.paid` that arrives before its order records the
invoice: it gets a retryable 409, so Stripe redelivers it.

**Refund operation and compatibility.** Stripe can return a bank refund from
`succeeded` to `requires_action`, then process corrected bank details through
`pending` and `succeeded` again. `failed` and `canceled` leave the amount owed
visible as `refund_failed`; the operator must arrange a separately verified
recovery. Never issue another repayment just because a read failed. See
[Stripe's bank refund lifecycle](https://docs.stripe.com/payments/customer-balance/refunding).

Only Stripe refunds fully allocated to the matching credit note are supported.
Customer credit balances, cash-balance refund destinations and external payment
annotations do not prove a return to the customer's bank. Unknown successful
refund destinations fail closed. International wire refunds require a separate
external payment path and are not completed by this implementation.

Old `CreditNoteIssued` events still replay, now as `refund_pending` until linked
refunds are verified. No stored events are rewritten. New
`BuyerRefundReconciled` events include the observed log sequence and require this
reader: once they are written, rolling back to an older binary would make those
order logs unreadable. Disable intake/invoicing if necessary and deploy a
compatible correction; retain the event reader when reverting other behavior.

**Known limits.**
- Refund status is refreshed by the operator's reconciliation endpoint. There
  is no automatic refund-event worker yet; late bank returns need continued
  reconciliation, including for previously successful refunds.
- A failed or canceled refund identity cannot be replaced silently. Recovery
  through a new payment provider/refund is not implemented here.
- Live funding still requires the cash-availability, conversion, reversal and
  account-country work described above. These refund changes do not enable
  invoicing or approve any live money movement.
- `ParticipantEligibilityRegistry` cannot revoke an attestation. A
  contractor's earlier wallet stays eligible until its attestation expires, so
  keep validity short (the plan accepts at most 365 days).
- Registration proves the attester's approval, not the contractor's control of
  the wallet. Confirm the wallet with the contractor before attesting it.

## Configuration

Invoicing stays off unless `INVOICING_ENABLED=true` and all of the following
are set:
- **Network and v2 factory:** `INVOICING_NETWORK` (default `base-sepolia`),
  which needs a configured v2 factory (`BASE_SEPOLIA_BOUNTY_V2_*`).
- **Treasury:** `INVOICING_TREASURY_WALLET`, the creator that signs funding.
- **Contractor registry:** `INVOICING_CONTRACTOR_REGISTRY` and
  `INVOICING_CONTRACTOR_ATTESTER`.
- **Agreements and terms:** `INVOICING_CONTRACTOR_AGREEMENT_SHA256`,
  `INVOICING_BUSINESS_TERMS_URL` (HTTPS) and `INVOICING_BUSINESS_TERMS_SHA256`.
- **Verification:** `INVOICING_VERIFIERS` (a comma list),
  `INVOICING_VERIFIER_THRESHOLD` (default 2), and
  `INVOICING_VERIFICATION_MODE` (`signed_quorum` or `ai_judge_quorum`).
- **Stripe:** `STRIPE_SECRET_KEY` and `STRIPE_INVOICE_WEBHOOK_SECRET`. A live
  key is refused unless `INVOICING_LIVEMODE=true`, and a test key is refused
  when it is set. The restricted key must permit the existing invoice writes
  plus credit-note and refund reads for reconciliation. Keep it scoped to the
  actual seller account and configured mode.

**Optional settings:**
- `INVOICING_PROCESSING_FEE_BPS` and `INVOICING_PROCESSING_FEE_FIXED_CENTS`
  (default 0);
- `INVOICING_MAX_INVOICE_CENTS` (default $10,000);
- `INVOICING_DAYS_UNTIL_DUE` (default 14);
- `INVOICING_FUNDING_WINDOW_SECONDS`, `INVOICING_CLAIM_WINDOW_SECONDS` and
  `INVOICING_VERIFICATION_WINDOW_SECONDS`.

**End-to-end test.** `invoiced_seller_of_record_path_runs_end_to_end_postgres`
runs the whole path against Postgres and a local Stripe stand-in:
quote, bank-transfer invoice (including a resent and a resumed issue), signed
`invoice.paid` (including a replay and an early delivery), treasury funding
plan, reconciliation from indexed events, contractor attestation and
registration, a contractor wallet change, the 1099 summary, and a cancelled
order whose planned funding is completed, cancelled and withdrawn before the
credit note leaves customer repayment pending. It then exercises independent
runtime retries, concurrent reconciliation, invalid allocations, successful
repayment, bank returns, a subsequent retry cycle, and terminal failure. All
refund reconciliation requests are reads.
